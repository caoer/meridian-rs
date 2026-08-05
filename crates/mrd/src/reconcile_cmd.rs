//! `mrd reconcile <preset> [--prune] [--dry] [--actor A] [--now T]` (U3.5b): the fs-frontier
//! reconcile-toward-scaffold . A local CLIENT of `preset::reconcile` (docs/laws.md charter):
//! mrd resolves the workspace and dials the op; the fold + the guarded writes live in `preset`.
//! The law, verbatim (plan §4 Block 3 U3.
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!

use preset::{BirthOptions, FileOutcome, PruneOutcome, ReconcileReport};
use serde_json::json;

use crate::{Fail, Format};

/// Exit 1 — a finding (an undeclared content file, or an occupied materialize
/// path). Distinct from `Fail::tool` (exit 2).
fn findings(message: String) -> Fail {
    Fail::findings(message)
}

/// Run `mrd reconcile <preset> [--prune] [--dry] [--actor A] [--now T] [--json]`. Errors A tool
/// failure (exit 2) — bad usage, an unreadable workspace, a preset that is not a def, or a
/// faulting write — or a findings exit (1) when an undeclared content file remains or a
/// declared path was occupied.
///
///
pub(crate) fn run(args: &[String]) -> Result<(), Fail> {
    let parsed = Parsed::parse(args)?;
    let root = crate::preset_cmd::resolve_root()?;
    let preset_path = crate::preset_cmd::def_path(&parsed.preset);

    let opts = BirthOptions {
        actor: parsed.actor,
        now: parsed.now,
        dry: parsed.dry,
    };
    let report = preset::reconcile(&root, &preset_path, parsed.prune, &opts)
        .map_err(|e| Fail::tool(e.to_string()))?;

    match parsed.format {
        Format::Json => println!(
            "{}",
            serde_json::to_string_pretty(&json!({ "reconcile": report })).expect("json")
        ),
        Format::Human => print_human(&report, parsed.dry, parsed.prune),
    }

    if report.is_clean() {
        Ok(())
    } else {
        let occupied = report
            .materialized
            .iter()
            .filter(|f| matches!(f, FileOutcome::Occupied { .. }))
            .count();
        Err(findings(format!(
            "reconcile {}: {} finding(s), {occupied} occupied materialize path(s)",
            report.preset,
            report.findings.len()
        )))
    }
}

/// Print the reconcile report as a human table: the additive materialize half, the subtractive
/// prune half (files + empty dirs), then the findings the check plane renders (undeclared
/// content, NEVER pruned).
fn print_human(report: &ReconcileReport, dry: bool, prune: bool) {
    let mode = if dry { " [dry]" } else { "" };
    println!("reconcile {}{mode}", report.preset);
    for file in &report.materialized {
        match file {
            FileOutcome::Born { path } => {
                if dry {
                    println!("  would-materialize {path}");
                } else {
                    println!("  materialize {path}");
                }
            }
            FileOutcome::Occupied { path, reason } => {
                println!("  present {path} — {} [{}]", reason.message, reason.code);
            }
        }
    }
    if prune {
        for p in &report.pruned {
            match p {
                PruneOutcome::Removed { path } => {
                    if dry {
                        println!("  would-prune {path}");
                    } else {
                        println!("  prune {path}");
                    }
                }
                PruneOutcome::Refused { path, reason } => {
                    println!("  prune-refused {path} — {reason}");
                }
            }
        }
        for dir in &report.pruned_dirs {
            println!("  prune-dir {dir}/ (empty, undeclared)");
        }
    }
    for finding in &report.findings {
        println!("  finding {finding} (undeclared content — reported, never pruned)");
    }
}

/// The parsed `reconcile` tail: `<preset>`, `--prune`, `--dry`, `--actor`,
/// `--now`, `--json`.
struct Parsed {
    preset: String,
    prune: bool,
    dry: bool,
    actor: Option<String>,
    now: Option<String>,
    format: Format,
}

impl Parsed {
    fn parse(args: &[String]) -> Result<Self, Fail> {
        let mut preset: Option<String> = None;
        let mut prune = false;
        let mut dry = false;
        let mut json = false;
        let mut actor = None;
        let mut now = None;
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--prune" => prune = true,
                "--dry" => dry = true,
                "--json" => json = true,
                "--actor" => {
                    actor = Some(
                        it.next()
                            .ok_or_else(|| Fail::tool("--actor needs a value".to_owned()))?
                            .clone(),
                    );
                }
                "--now" => {
                    now = Some(
                        it.next()
                            .ok_or_else(|| Fail::tool("--now needs a value".to_owned()))?
                            .clone(),
                    );
                }
                flag if flag.starts_with('-') => {
                    return Err(Fail::tool(format!("unknown flag: {flag}")));
                }
                value if preset.is_none() => preset = Some(value.to_owned()),
                value => return Err(Fail::tool(format!("unexpected argument: {value}"))),
            }
        }
        let preset =
            preset.ok_or_else(|| Fail::tool("reconcile needs a PRESET argument".to_owned()))?;
        Ok(Parsed {
            preset,
            prune,
            dry,
            actor,
            now,
            format: if json { Format::Json } else { Format::Human },
        })
    }
}
