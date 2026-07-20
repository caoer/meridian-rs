//! Run reports: one canonical schema-versioned JSON, three renderings —
//! `RESULTS.md` (human), `latest.json` (agent), and an append-only history file
//! `results/<date>-<time>-<host>-<sha>.json`. Every report carries machine
//! fingerprint, git sha, and toolchain, so numbers are never orphaned from
//! their context; the time+sha in the history filename keep same-day reruns
//! from clobbering one another.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::claims::{Claim, Direction, ThresholdSource, Verdict, VerdictKind};
use crate::corpus::workspace_target;
use crate::measure::{self, Measurement};

pub const REPORT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunReport {
    pub schema_version: u32,
    pub created_utc: String,
    pub git_sha: String,
    pub rustc: String,
    pub machine: Machine,
    pub verdicts: Vec<Verdict>,
    pub measurements: Vec<Measurement>,
    pub criterion: Vec<CriterionEstimate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Machine {
    pub hostname: String,
    pub os: String,
    pub arch: String,
    pub cpu: String,
    pub cores: usize,
}

/// Mean/median pulled from criterion's `estimates.json` (the same surface
/// critcmp reads — de-facto stable across 0.5.x).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CriterionEstimate {
    pub id: String,
    pub mean_ns: f64,
    pub median_ns: f64,
}

#[must_use]
pub fn machine() -> Machine {
    Machine {
        hostname: cmd_line("hostname", &[]).unwrap_or_else(|| "unknown".into()),
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        cpu: cpu_brand().unwrap_or_else(|| "unknown".into()),
        cores: std::thread::available_parallelism().map_or(0, std::num::NonZero::get),
    }
}

fn cpu_brand() -> Option<String> {
    if cfg!(target_os = "macos") {
        cmd_line("sysctl", &["-n", "machdep.cpu.brand_string"])
    } else if cfg!(target_os = "linux") {
        let text = fs::read_to_string("/proc/cpuinfo").ok()?;
        text.lines()
            .find(|l| l.starts_with("model name"))
            .and_then(|l| l.split(':').nth(1))
            .map(|s| s.trim().to_owned())
    } else {
        None
    }
}

fn cmd_line(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    let line = String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()?
        .trim()
        .to_owned();
    (!line.is_empty()).then_some(line)
}

/// `YYYY-MM-DD HH:MM UTC` without a chrono dependency (Howard Hinnant's
/// civil-from-days algorithm).
#[must_use]
pub fn utc_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe as i64 + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02} UTC",
        rem / 3600,
        (rem % 3600) / 60
    )
}

/// Walk `target/criterion/**/new/estimates.json`; the id is the path between
/// `criterion/` and `/new`, slash-joined (criterion's group/function layout).
#[must_use]
pub fn collect_criterion() -> Vec<CriterionEstimate> {
    let root = workspace_target().join("criterion");
    let mut out = Vec::new();
    collect_criterion_walk(&root, &root, &mut out);
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

fn collect_criterion_walk(root: &Path, dir: &Path, out: &mut Vec<CriterionEstimate>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let estimates = path.join("new").join("estimates.json");
        if estimates.exists() {
            if let Some(est) = parse_estimates(root, &path, &estimates) {
                out.push(est);
            }
        } else {
            collect_criterion_walk(root, &path, out);
        }
    }
}

fn parse_estimates(root: &Path, bench_dir: &Path, estimates: &Path) -> Option<CriterionEstimate> {
    let value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(estimates).ok()?).ok()?;
    let point = |k: &str| value.get(k)?.get("point_estimate")?.as_f64();
    Some(CriterionEstimate {
        id: bench_dir
            .strip_prefix(root)
            .ok()?
            .to_string_lossy()
            .replace('\\', "/"),
        mean_ns: point("mean")?,
        median_ns: point("median")?,
    })
}

/// Assemble the full report: staged measurements joined against claims,
/// criterion estimates collected, context stamped.
pub fn build(claims: &[Claim]) -> io::Result<RunReport> {
    let measurements = measure::read_all()?;
    // Join by id, but only when the staged unit matches the claim's metric —
    // a µs value staged for an `ms` claim would otherwise be gated against the
    // threshold raw. A mismatch drops to UNTESTED (never a false PASS/FAIL).
    let metric_by_id: BTreeMap<&str, &str> = claims
        .iter()
        .map(|c| (c.id.as_str(), c.metric.as_str()))
        .collect();
    let mut measured: BTreeMap<String, f64> = BTreeMap::new();
    for m in &measurements {
        if let Some(&metric) = metric_by_id.get(m.id.as_str())
            && metric != m.unit
        {
            eprintln!(
                "bench-report: dropping measurement '{}' — unit '{}' ≠ claim metric '{}'",
                m.id, m.unit, metric
            );
            continue;
        }
        measured.insert(m.id.clone(), m.value);
    }
    Ok(RunReport {
        schema_version: REPORT_SCHEMA_VERSION,
        created_utc: utc_now(),
        git_sha: cmd_line("git", &["rev-parse", "--short", "HEAD"])
            .unwrap_or_else(|| "unknown".into()),
        rustc: cmd_line("rustc", &["--version"]).unwrap_or_else(|| "unknown".into()),
        machine: machine(),
        verdicts: crate::claims::join(claims, &measured),
        measurements,
        criterion: collect_criterion(),
    })
}

fn fmt_opt(v: Option<f64>) -> String {
    v.map_or_else(|| "—".into(), |x| format!("{x:.3}"))
}

fn gate_cell(claim: &Claim) -> String {
    match (claim.threshold_source, claim.effective_threshold()) {
        (ThresholdSource::Ruleset, _) => "ruleset".into(),
        (_, Some(t)) => match claim.direction {
            Direction::Higher => format!("≥ {t}"),
            Direction::Lower => format!("≤ {t}"),
        },
        (_, None) => "—".into(),
    }
}

/// The human rendering.
#[must_use]
pub fn render_markdown(report: &RunReport, claims: &[Claim]) -> String {
    let mut md = String::new();
    let m = &report.machine;
    let _ = writeln!(md, "# meridian-rs bench results\n");
    let _ = writeln!(
        md,
        "Run {} · `{}` ({} {}, {}, {} cores) · git `{}` · {} · report schema v{}\n",
        report.created_utc,
        m.hostname,
        m.arch,
        m.os,
        m.cpu,
        m.cores,
        report.git_sha,
        report.rustc,
        report.schema_version
    );
    let _ = writeln!(md, "## Claim verdicts\n");
    let _ = writeln!(
        md,
        "| claim | metric | baseline | gate | measured | verdict | note |"
    );
    let _ = writeln!(md, "|---|---|---|---|---|---|---|");
    for claim in claims {
        let verdict = report.verdicts.iter().find(|v| v.claim_id == claim.id);
        let _ = writeln!(
            md,
            "| `{}` | {} | {} | {} | {} | {} | {} |",
            claim.id,
            claim.metric,
            fmt_opt(claim.baseline),
            gate_cell(claim),
            fmt_opt(verdict.and_then(|v| v.measured)),
            verdict.map_or("UNTESTED", |v| verdict_str(v.verdict)),
            claim.note.as_deref().unwrap_or(""),
        );
    }
    if !report.measurements.is_empty() {
        let _ = writeln!(md, "\n## Latency distributions (hdrhistogram path, µs)\n");
        let _ = writeln!(
            md,
            "| id | headline | unit | samples | p50 | p90 | p99 | p99.9 | max |"
        );
        let _ = writeln!(md, "|---|---|---|---|---|---|---|---|---|");
        for meas in &report.measurements {
            if let Some(d) = &meas.dist {
                let _ = writeln!(
                    md,
                    "| `{}` | {:.3} | {} | {} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} |",
                    meas.id,
                    meas.value,
                    meas.unit,
                    meas.samples,
                    d.p50_us,
                    d.p90_us,
                    d.p99_us,
                    d.p999_us,
                    d.max_us
                );
            }
        }
    }
    if !report.criterion.is_empty() {
        let _ = writeln!(md, "\n## Criterion estimates\n");
        let _ = writeln!(md, "| bench | mean | median |");
        let _ = writeln!(md, "|---|---|---|");
        for est in &report.criterion {
            let _ = writeln!(
                md,
                "| `{}` | {} | {} |",
                est.id,
                fmt_ns(est.mean_ns),
                fmt_ns(est.median_ns)
            );
        }
    }
    let _ = writeln!(
        md,
        "\nBaseline provenance: a prior benchmark run (frozen 2026-07-17). \
         Corpora are recipe-generated (`corpusgen`), never committed; claims are data in \
         `claims.toml`; UNTESTED is the perf mirror of a `todo!()` body."
    );
    md
}

fn verdict_str(v: VerdictKind) -> &'static str {
    match v {
        VerdictKind::Pass => "PASS",
        VerdictKind::Fail => "**FAIL**",
        VerdictKind::Measured => "MEASURED",
        VerdictKind::Untested => "UNTESTED",
    }
}

fn fmt_ns(ns: f64) -> String {
    if ns >= 1e9 {
        format!("{:.2} s", ns / 1e9)
    } else if ns >= 1e6 {
        format!("{:.2} ms", ns / 1e6)
    } else if ns >= 1e3 {
        format!("{:.2} µs", ns / 1e3)
    } else {
        format!("{ns:.0} ns")
    }
}

/// Write the three renderings into `results_dir`. Returns written paths.
pub fn write(results_dir: &Path, report: &RunReport, claims: &[Claim]) -> io::Result<Vec<PathBuf>> {
    fs::create_dir_all(results_dir)?;
    // created_utc is `YYYY-MM-DD HH:MM UTC`.
    let mut parts = report.created_utc.split(' ');
    let date = parts.next().unwrap_or("undated");
    let hhmm = parts.next().unwrap_or("0000").replace(':', "");
    // Historical file carries date+time+host+sha so a second run on the same
    // host/day (or a rebuild) never silently clobbers earlier history.
    let dated = results_dir.join(format!(
        "{date}-{hhmm}-{}-{}.json",
        report.machine.hostname, report.git_sha
    ));
    let latest = results_dir.join("latest.json");
    let md_path = results_dir.join("RESULTS.md");
    let json = serde_json::to_string_pretty(report)?;
    fs::write(&dated, &json)?;
    fs::write(&latest, &json)?;
    fs::write(&md_path, render_markdown(report, claims))?;
    Ok(vec![dated, latest, md_path])
}
