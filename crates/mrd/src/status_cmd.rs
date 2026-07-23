//! `mrd status` — the bare, pure-local drift + freshness summary (U3.6, the LAST
//! leg of the bridge; cutover order law: status cuts last).
//!
//! ```text
//! mrd status [--json] [--cwd PATH]
//! ```
//!
//! # What bare `status` is (plan §4 Block 3 U3.6; d2 §6.2)
//! `status = freshness` (d2 §3; distinct from `check = validity`). Bare `status`
//! answers "what is armed, what drifted, what was forced, and how fresh is my
//! origin knowledge" — from FROZEN facts only. It is:
//!
//! - **pure-local** — no daemon, no network, no fetch (cap-free);
//! - **O(armed)** — it reads ONE index file (`conventions/INDEX.md`) for the
//!   armed set, re-hashes each armed convention's `CHECK.md` (O(armed) small
//!   reads), scans the bounded receipt journal, and reads the git refs. It NEVER
//!   walks the whole corpus, so the 3k-corpus wall-time stays sub-second;
//! - **fetch-less** — the anchor axis is therefore NEVER `verified` and never
//!   renders a bare `at-tip` (W-C1, U2.7; the colors amendment § anchor axis);
//! - **predicate-free** — it never evaluates a `check:` (the <1s budget holds;
//!   passenger-registry amendment). Drift here is a mechanical rev compare, never
//!   a starlark run.
//!
//! # The composed legend — three axes on one surface (U6.2)
//! `status` renders the three orthogonal axes side by side, never merged, each
//! rolled up worst-of INDEPENDENTLY (colors amendment § composed legend):
//!
//! - **pin color** — the armed set's evidence drift: `green` (every armed
//!   convention's live `CHECK.md` rev still equals its pinned `armed_rev`) or
//!   `red content-drifted` (some armed evidence drifted). The four named greys of
//!   the full color law are the render's capability (fixtures pin each), and are
//!   reached only by pinned `^inputs` edges, not the armed set.
//! - **anchor state** — the origin-freshness qualifier (U2.7): `as-known` /
//!   `unverified`, NEVER `verified` (status cannot fetch).
//! - **convention severity** — the worst armed severity (`off` / `warn` /
//!   `block`), and one violation row per `--force`-escaped skip.
//!
//! # The INDEX summary line
//! `<A> armed · <D> drifted · <F> forced-since-realise (receipts boundary)`:
//! - `armed` — the count of `[x]` rows in the attested INDEX (U1.4);
//! - `drifted` — armed conventions whose live `CHECK.md` rev ≠ the pinned
//!   `armed_rev` (the arming drift gate, read-only);
//! - `forced-since-realise` — the count of `op=force` journal rows (U4.3) newer
//!   than the last realise APPLY. The boundary is the last `now` in
//!   `receipts/realise.md` (the realise receipt ledger — realise applies leave NO
//!   reserved-journal marker, so this is a receipts boundary, not a
//!   journal-anchored guarantee). It resolves toward VISIBILITY: no realise
//!   receipt ⇒ genesis (count all forced writes), and a tied/unordered `now`
//!   COUNTS — the counter over-reports violations, never under-reports.
//!
//! # Exit triad (§4 preamble)
//! - **0** — clean: nothing armed drifted and nothing was forced.
//! - **1** — a finding: an armed convention drifted, a forced write is live, or
//!   the INDEX is a convention-fault. Field-equivalent to `md status`'s red
//!   (drifted) exit at the semantic class.
//! - **2** — bad invocation, or an unresolvable / unreadable workspace.

use std::path::{Path, PathBuf};
use std::process::Command;

use model::selector::{Color, RedReason};
use policy::{Enforcement, evidence_rev, parse_index_strict};
use receipt::anchor::{AnchorFacts, AnchorState, Observed, TipPosition};
use receipt::journal::parse_rows;
use serde_json::{Value, json};
use view::walk::{color_label, color_tone};

use crate::{Fail, Format, current_dir};

/// The finding leg of the triad: the invocation was well-formed, but the summary
/// carries a live drift, a forced write, or a faulted INDEX.
const EXIT_FINDING: u8 = 1;

/// The attested INDEX page — the ONE file the armed-set read opens (byte-equal to
/// `policy::binding::RESERVED_INDEX_PATH` / `fs::domain`).
const CONVENTIONS_INDEX: &str = "conventions/INDEX.md";

/// The realise receipt ledger — the `forced-since-realise` boundary source. NOT
/// the reserved journal: realise applies append `- run {json} ^r-NNNNNN` lines
/// here, never an `op=force`-carrying journal row.
const REALISE_RECEIPTS: &str = "receipts/realise.md";

/// Run `mrd status [--json] [--cwd PATH]`.
///
/// # Errors
/// [`Fail`] exit 2 on a bad invocation or an unresolvable / unreadable workspace;
/// exit 1 when the summary carries a finding (drift, a forced write, or a faulted
/// INDEX).
pub(crate) fn run(tail: &[String]) -> Result<(), Fail> {
    let (format, cwd_arg) = parse(tail)?;
    let cwd = match cwd_arg {
        Some(p) => p,
        None => current_dir()?,
    };
    let resolved = crate::resolve::resolve_runtime(&cwd).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace for {}: {e}",
            cwd.display()
        ))
    })?;
    let workspace = workspace::canonicalize(&resolved.workspace).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace {} ({e})",
            resolved.workspace.display()
        ))
    })?;

    let report = gather(&workspace);

    match format {
        Format::Json => println!("{}", report.json()),
        Format::Human => print!("{}", report.render_human()),
    }

    if report.has_findings() {
        return Err(Fail {
            code: EXIT_FINDING,
            message: report.finding_summary(),
        });
    }
    Ok(())
}

/// Parse `[--json] [--cwd PATH]` — the bare form only (no page positional; a page
/// query is not this verb, U3.6).
fn parse(tail: &[String]) -> Result<(Format, Option<PathBuf>), Fail> {
    let mut json = false;
    let mut cwd: Option<PathBuf> = None;
    let mut i = 0;
    while i < tail.len() {
        let arg = tail[i].as_str();
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) => (f, Some(v.to_owned())),
            None => (arg, None),
        };
        match flag {
            "--json" => json = true,
            "--cwd" => {
                let v = if let Some(v) = inline {
                    v
                } else {
                    i += 1;
                    tail.get(i)
                        .cloned()
                        .ok_or_else(|| Fail::tool("--cwd needs a value".to_owned()))?
                };
                cwd = Some(PathBuf::from(v));
            }
            other => return Err(Fail::tool(format!("unknown flag or argument: {other}"))),
        }
        i += 1;
    }
    let format = if json { Format::Json } else { Format::Human };
    Ok((format, cwd))
}

/// One `--force`-escaped skip surfaced as a violation row (U4.3 §11.1): the
/// bypassed rule, the journal anchor, and the recorded `now`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Violation {
    /// The bypassed convention / binding-break (the `forced_rule=` token).
    rule: String,
    /// The journal row anchor (`r-NNNNNN`) — the permanent record of the skip.
    anchor: String,
    /// The recorded timestamp of the forced write, verbatim (never invented).
    now: Option<String>,
}

/// Where the `forced-since-realise` count is anchored — named so the render can
/// state precisely that this is a receipts boundary, not a journal guarantee.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Boundary {
    /// No realise receipt exists — count ALL forced writes (visibility fallback).
    Genesis,
    /// The last realise-apply `now` from `receipts/realise.md`.
    LastApply(String),
}

/// The gathered, render-ready status summary — the three composed axes, the INDEX
/// counts, and the forced-write violation rows.
struct StatusReport {
    workspace: String,
    /// Count of `[x]` rows in the attested INDEX (the armed set).
    armed: usize,
    /// Count of armed conventions whose live `CHECK.md` rev ≠ pinned `armed_rev`.
    drifted: usize,
    /// Count of `op=force` journal rows newer than the realise boundary.
    forced: usize,
    /// Where the forced count is anchored.
    boundary: Boundary,
    /// The INDEX convention-fault detail, when the page is present but corrupt.
    index_fault: Option<String>,
    /// The pin-color axis roll-up — `Green` all-fresh, `Red(Drifted)` any-drift.
    pin_rollup: Color,
    /// The convention-severity axis roll-up — the worst armed severity.
    severity_rollup: Enforcement,
    /// The rendered anchor-qualified tip axis (U2.7) — never a bare `at-tip`.
    anchor_axis: String,
    /// The as-known-ageless nudge hint, when present.
    nudge: Option<&'static str>,
    /// The `--force`-escaped skips since the boundary.
    violations: Vec<Violation>,
}

impl StatusReport {
    /// A finding is live when armed evidence drifted, a forced write is unresolved,
    /// or the INDEX faulted — the exit-1 predicate.
    fn has_findings(&self) -> bool {
        self.drifted > 0 || self.forced > 0 || self.index_fault.is_some()
    }

    /// The one-line stderr summary that rides the exit-1 `Fail`.
    fn finding_summary(&self) -> String {
        if let Some(detail) = &self.index_fault {
            return format!("INDEX convention-fault: {detail}");
        }
        format!(
            "{} drifted, {} forced-since-realise",
            self.drifted, self.forced
        )
    }

    /// The composed multi-axis line — pin color · anchor state · convention
    /// severity, side by side, never merged (U6.2 composed legend).
    fn composed_line(&self) -> String {
        format!(
            "pin {} · anchor {} · convention {}",
            color_label(&self.pin_rollup),
            self.anchor_axis,
            self.severity_rollup.as_str(),
        )
    }

    /// Render the human summary block.
    fn render_human(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        let _ = writeln!(out, "status  {}", self.workspace);
        let boundary = match &self.boundary {
            Boundary::Genesis => "genesis".to_owned(),
            Boundary::LastApply(now) => format!("since {now}"),
        };
        let _ = writeln!(
            out,
            "  INDEX: {} armed · {} drifted · {} forced-since-realise (receipts boundary: {boundary})",
            self.armed, self.drifted, self.forced,
        );
        let _ = writeln!(out, "  {}", self.composed_line());
        if let Some(nudge) = self.nudge {
            let _ = writeln!(out, "  hint: {nudge}");
        }
        if let Some(detail) = &self.index_fault {
            let _ = writeln!(out, "  INDEX fault: {detail}");
        }
        for v in &self.violations {
            let _ = writeln!(out, "  {}", render_violation_row(v));
        }
        out
    }

    /// The `--json` shape: the three axes as fields, the counts, the boundary, and
    /// the violation rows.
    fn json(&self) -> String {
        let violations: Vec<Value> = self
            .violations
            .iter()
            .map(|v| {
                json!({
                    "rule": v.rule,
                    "anchor": v.anchor,
                    "now": v.now,
                })
            })
            .collect();
        let boundary = match &self.boundary {
            Boundary::Genesis => json!({ "kind": "genesis" }),
            Boundary::LastApply(now) => json!({ "kind": "receipts", "since": now }),
        };
        let doc = json!({
            "workspace": self.workspace,
            "index": {
                "armed": self.armed,
                "drifted": self.drifted,
                "forced_since_realise": self.forced,
                "boundary": boundary,
                "fault": self.index_fault,
            },
            "composed": {
                "pin_color": color_tone(&self.pin_rollup),
                "pin_label": color_label(&self.pin_rollup),
                "anchor": self.anchor_axis,
                "convention_severity": self.severity_rollup.as_str(),
            },
            "nudge": self.nudge,
            "violations": violations,
            "findings": self.has_findings(),
        });
        serde_json::to_string_pretty(&doc).unwrap_or_else(|_| doc.to_string())
    }
}

/// Render one forced-write violation row (U4.3 §11.1) — the visible, permanent
/// record of a `--force`-escaped armed refusal.
fn render_violation_row(v: &Violation) -> String {
    let now = v.now.as_deref().unwrap_or("(no timestamp)");
    format!(
        "violation: forced past `{}` · convention block · ^{} ({now})",
        v.rule, v.anchor,
    )
}

/// Gather the status summary over a canonical workspace path. The impure edge: it
/// reads the INDEX, the armed `CHECK.md` files, the journal, the realise receipts,
/// and the git refs — every read frozen, none re-evaluated. It never fails: every
/// absent / unreadable frozen fact degrades to its honest empty case (genesis,
/// unverified), so status always renders a summary.
fn gather(workspace: &Path) -> StatusReport {
    // 1. The armed set — ONE index-file read (O(armed)).
    let (armed, index_fault) = read_armed(workspace);

    // 2. Per-armed drift — re-hash each CHECK.md (O(armed) small reads).
    let mut drifted = 0usize;
    for a in &armed {
        if convention_drifted(workspace, &a.slug, &a.armed_rev) {
            drifted += 1;
        }
    }
    let pin_rollup = if drifted > 0 {
        Color::Red(RedReason::Drifted)
    } else {
        Color::Green
    };
    let severity_rollup = armed
        .iter()
        .map(|a| a.enforcement)
        .max()
        .unwrap_or(Enforcement::Off);

    // 3. The journal + the realise boundary — the forced-since-realise count.
    let journal_text = read_optional(workspace, receipt_journal_rel());
    let rows = parse_rows(&journal_text);
    let boundary = realise_boundary(workspace);
    let (forced, violations) = forced_since(&journal_text, &rows, &boundary);

    // 4. The anchor axis — git refs, fetch-less, never verified (U2.7).
    let (anchor_axis, nudge) = anchor_axis(workspace, &rows);

    StatusReport {
        workspace: workspace.display().to_string(),
        armed: armed.len(),
        drifted,
        forced,
        boundary,
        index_fault,
        pin_rollup,
        severity_rollup,
        anchor_axis,
        nudge,
        violations,
    }
}

/// One armed convention read from the INDEX: the pinned `armed_rev` per slug, plus
/// its severity.
struct ArmedConv {
    slug: String,
    enforcement: Enforcement,
    armed_rev: String,
}

/// Read the attested INDEX into the armed set. An absent page is a genesis
/// workspace (nothing armed, no fault); a present-but-corrupt page fails CLOSED to
/// a named fault (never read as an empty, gate-disabling armed set).
fn read_armed(workspace: &Path) -> (Vec<ArmedConv>, Option<String>) {
    let text = read_optional(workspace, CONVENTIONS_INDEX);
    if text.is_empty() {
        return (Vec::new(), None);
    }
    match parse_index_strict(&text) {
        Ok(refs) => {
            let armed = refs
                .into_iter()
                .map(|r| ArmedConv {
                    slug: r.slug,
                    enforcement: r.enforcement,
                    armed_rev: r.armed_rev,
                })
                .collect();
            (armed, None)
        }
        Err(corrupt) => (Vec::new(), Some(corrupt.detail)),
    }
}

/// Whether an armed convention's live `CHECK.md` rev differs from its pinned
/// `armed_rev` (the arming drift gate, read-only). A missing `CHECK.md` — the
/// pinned evidence vanished — counts as drift (fail-closed: the armed law can no
/// longer be verified at its rev).
fn convention_drifted(workspace: &Path, slug: &str, armed_rev: &str) -> bool {
    let rel = format!("conventions/{slug}/CHECK.md");
    match std::fs::read_to_string(workspace.join(&rel)) {
        Ok(text) => evidence_rev(&text) != armed_rev,
        Err(_) => true,
    }
}

/// The reserved receipt-journal path, workspace-relative.
fn receipt_journal_rel() -> &'static str {
    fs::domain::RESERVED_JOURNAL_PATH
}

/// Read a workspace-relative page, treating any read error (absent, unreadable) as
/// the empty string — the genesis / tolerated case for the frozen-fact reads.
fn read_optional(workspace: &Path, rel: &str) -> String {
    std::fs::read_to_string(workspace.join(rel)).unwrap_or_default()
}

/// Resolve the `forced-since-realise` boundary from the realise receipt ledger:
/// the LAST realise-apply `now` (`- run {json} ^r-NNNNNN` lines), or [`Boundary::Genesis`]
/// when the ledger is absent / has no dated apply.
fn realise_boundary(workspace: &Path) -> Boundary {
    let text = read_optional(workspace, REALISE_RECEIPTS);
    match last_receipt_now(&text) {
        Some(now) => Boundary::LastApply(now),
        None => Boundary::Genesis,
    }
}

/// The `now` field of the LAST `- run {json} ^r-NNNNNN` receipt line, or `None`
/// when no dated apply is present. Parses only the JSON envelope — never a
/// predicate.
fn last_receipt_now(receipts: &str) -> Option<String> {
    receipts
        .lines()
        .rev()
        .filter_map(|line| line.trim().strip_prefix("- run "))
        .find_map(|rest| {
            // Strip the trailing ` ^r-NNNNNN` anchor, parse the JSON envelope.
            let json = rest.rsplit_once(" ^").map_or(rest, |(head, _)| head);
            serde_json::from_str::<Value>(json.trim())
                .ok()
                .and_then(|v| v.get("now").and_then(Value::as_str).map(str::to_owned))
        })
}

/// Count the `op=force` journal rows newer than the realise boundary, and collect
/// them as violation rows. Resolves toward VISIBILITY (leader ruling): a forced
/// write with no `now`, or a `now` that ties / is unordered relative to the
/// boundary, COUNTS — only a row strictly older than the boundary is excluded.
fn forced_since(
    journal_text: &str,
    rows: &[receipt::journal::ParsedRow],
    boundary: &Boundary,
) -> (usize, Vec<Violation>) {
    let lines: Vec<&str> = journal_text.lines().collect();
    let mut violations = Vec::new();
    for row in rows.iter().filter(|r| r.op == "force") {
        if !counts_since(row.now.as_deref(), boundary) {
            continue;
        }
        let rule = lines
            .get(row.line_no.saturating_sub(1))
            .and_then(|line| forced_rule(line))
            .unwrap_or_else(|| "(unnamed)".to_owned());
        violations.push(Violation {
            rule,
            anchor: row.anchor.clone(),
            now: row.now.clone(),
        });
    }
    (violations.len(), violations)
}

/// Whether a forced row's `now` counts against the boundary. Over-reports: only a
/// dated row STRICTLY older than a dated boundary is excluded; genesis, an undated
/// row, and a tie all count.
fn counts_since(row_now: Option<&str>, boundary: &Boundary) -> bool {
    // Exclude ONLY a dated row strictly older than a dated boundary; genesis, an
    // undated row, and a tie all fall through to `true` (over-report).
    match (boundary, row_now) {
        (Boundary::LastApply(b), Some(n)) => n >= b.as_str(),
        _ => true,
    }
}

/// Extract the `forced_rule=<rule>` token from a raw `op=force` journal line. The
/// token is whitespace-collapsed at write time (one token, no split), so a plain
/// whitespace tokenize recovers it. `ParsedRow` does not model it.
fn forced_rule(line: &str) -> Option<String> {
    line.split_whitespace()
        .find_map(|tok| tok.strip_prefix("forced_rule="))
        .map(str::to_owned)
}

/// Render the anchor-qualified tip axis (U2.7) for the workspace, plus any nudge
/// hint. Fetch-less: `run_observed` is ALWAYS false, so the state is never
/// `verified` and never renders a bare `at-tip` (the W-C1 invariant). Returns a
/// custom "no origin ref" render when the remote-tracking ref is absent (no tip to
/// compare) rather than a misleading bare position.
fn anchor_axis(
    workspace: &Path,
    rows: &[receipt::journal::ParsedRow],
) -> (String, Option<&'static str>) {
    let branch = git(workspace, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default();
    let head = git(workspace, &["rev-parse", "HEAD"]);
    let origin_ref = format!("refs/remotes/origin/{branch}");
    let origin = git(
        workspace,
        &["rev-parse", "--verify", "--quiet", &origin_ref],
    );

    let now_unix = now_unix();
    let last_observation = receipt::anchor::last_observation(rows).and_then(|now| {
        parse_rfc3339(now).map(|unix| Observed {
            now: now.to_owned(),
            unix,
        })
    });
    let facts = AnchorFacts {
        run_observed: false,
        last_observation,
        origin_ref_present: origin.is_some(),
    };
    let state = AnchorState::classify(&facts, now_unix);
    let nudge = receipt::anchor::nudge_hint(&state);

    if let (Some(h), Some(o)) = (head, origin) {
        let tip = if h == o {
            TipPosition::AtTip
        } else {
            TipPosition::Behind
        };
        (receipt::anchor::render_tip_axis(tip, &state), nudge)
    } else {
        // No remote-tracking ref (or no HEAD): there is no tip to compare. Render
        // the anchor state alone, never a fabricated position.
        let word = if branch.is_empty() {
            "no origin ref".to_owned()
        } else {
            format!("no origin/{branch} ref")
        };
        (
            format!("{word} (anchor {})", anchor_state_word(&state)),
            nudge,
        )
    }
}

/// The bare state word for the no-origin-ref render (the qualifier without a tip
/// position, which does not exist when there is no ref to compare).
fn anchor_state_word(state: &AnchorState) -> &'static str {
    match state {
        AnchorState::Verified => "verified",
        AnchorState::AsKnownAged { .. } | AnchorState::AsKnownAgeless => "as-known",
        AnchorState::Unverified => "unverified",
    }
}

/// Current wall-clock as epoch seconds — the run's one anchor moment.
fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// Run `git -C workspace <args>` and return trimmed stdout on success, or `None`
/// on any failure (missing ref, no repo, non-zero exit). The tip axis degrades
/// honestly — a git failure renders `unverified`, never a fabricated position.
fn git(workspace: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(workspace)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

/// Parse a canonical RFC3339 UTC token (`YYYY-MM-DDTHH:MM:SSZ`) to epoch seconds,
/// or `None` when it is not that exact shape. The anchor's journaled observations
/// are minted in this form; a non-canonical token is undatable, so the anchor
/// degrades to as-known-AGELESS (never a wrong age). No date crate — the civil
/// arithmetic is exact and dependency-free.
fn parse_rfc3339(token: &str) -> Option<i64> {
    let bytes = token.as_bytes();
    // YYYY-MM-DDTHH:MM:SSZ is exactly 20 bytes with fixed separators.
    if bytes.len() != 20 || bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b'T' {
        return None;
    }
    if bytes[13] != b':' || bytes[16] != b':' || bytes[19] != b'Z' {
        return None;
    }
    let num = |lo: usize, hi: usize| token.get(lo..hi)?.parse::<i64>().ok();
    let year = num(0, 4)?;
    let month = num(5, 7)?;
    let day = num(8, 10)?;
    let hour = num(11, 13)?;
    let min = num(14, 16)?;
    let sec = num(17, 19)?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if hour > 23 || min > 59 || sec > 60 {
        return None;
    }
    let days = days_from_civil(year, month, day);
    Some(((days * 24 + hour) * 60 + min) * 60 + sec)
}

/// Days since the Unix epoch (1970-01-01) for a civil date, by Howard Hinnant's
/// `days_from_civil` algorithm — exact for the proleptic Gregorian calendar, no
/// lookup tables, no dependency.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::selector::GreyReason;

    // ── the four-grey render fixtures (U6.2 full color law) ──────────────────

    /// Each of the four named greys renders through the composed line's pin-color
    /// axis with its contract label (colors amendment § Colors / D2-F4). The
    /// render function carries the FULL color law, not just the armed set's
    /// green/red.
    #[test]
    fn four_greys_render_each_with_its_contract_label() {
        let cases = [
            (GreyReason::DeclaredUnpinned, "grey declared-unpinned"),
            (GreyReason::SupersededAlgo, "grey superseded-algo"),
            (GreyReason::ImmutableRoot, "grey immutable-root"),
            // `Ambiguous` is the code variant for the selector-ambiguity grey; the
            // contract's fourth named grey `unmanaged` shares the DeclaredUnpinned
            // variant (labeled `unmanaged` only on the pin surface). The composed
            // line renders whichever variant it is handed.
            (GreyReason::Ambiguous, "grey ambiguous"),
        ];
        for (reason, expected) in cases {
            let line = compose(
                Color::Grey(reason),
                "at-tip (anchor unverified)",
                Enforcement::Block,
            );
            assert!(
                line.contains(expected),
                "grey variant renders {expected}: {line}"
            );
        }
    }

    /// The composed multi-axis line renders all three axes side by side, never
    /// merged — pin color, anchor state, convention severity (U6.2).
    #[test]
    fn composed_line_shows_three_axes_side_by_side() {
        let line = compose(
            Color::Red(RedReason::Drifted),
            "behind (anchor as-known, observed 2026-07-20T00:00:00Z, ~2d)",
            Enforcement::Warn,
        );
        assert_eq!(
            line,
            "pin red content-drifted · anchor behind (anchor as-known, observed 2026-07-20T00:00:00Z, ~2d) · convention warn"
        );
    }

    /// The green, off, at-tip clean composition.
    #[test]
    fn composed_line_green_clean() {
        let line = compose(Color::Green, "at-tip (anchor as-known)", Enforcement::Off);
        assert_eq!(
            line,
            "pin green · anchor at-tip (anchor as-known) · convention off"
        );
    }

    /// Compose the three axes without an IO edge — the pure render under test by
    /// the fixtures.
    fn compose(pin: Color, anchor_axis: &str, severity: Enforcement) -> String {
        StatusReport {
            workspace: "/ws".to_owned(),
            armed: 0,
            drifted: 0,
            forced: 0,
            boundary: Boundary::Genesis,
            index_fault: None,
            pin_rollup: pin,
            severity_rollup: severity,
            anchor_axis: anchor_axis.to_owned(),
            nudge: None,
            violations: Vec::new(),
        }
        .composed_line()
    }

    // ── the forced-write violation row fixture (U4.3 §11.1) ──────────────────

    /// The forced-write violation row renders the bypassed rule, the block
    /// severity, and the permanent journal anchor.
    #[test]
    fn forced_write_renders_the_violation_row() {
        let v = Violation {
            rule: "reviewer-not-owner".to_owned(),
            anchor: "r-000042".to_owned(),
            now: Some("2026-07-23T10:00:00Z".to_owned()),
        };
        assert_eq!(
            render_violation_row(&v),
            "violation: forced past `reviewer-not-owner` · convention block · ^r-000042 (2026-07-23T10:00:00Z)"
        );
    }

    /// `forced_rule=` is recovered from a raw `op=force` journal line (it is not a
    /// `ParsedRow` field).
    #[test]
    fn forced_rule_reads_the_token() {
        let line = "- op=force path=tasks/fix.md actor=agent:a root_before=b3:1 root_after=b3:2 edits=0 forced_rule=reviewer-not-owner ^r-000042";
        assert_eq!(forced_rule(line).as_deref(), Some("reviewer-not-owner"));
        assert_eq!(forced_rule("- op=splice ^r-000001"), None);
    }

    // ── the forced-since-realise boundary (leader ruling: over-report) ───────

    /// Genesis (no realise receipt) counts every forced write.
    #[test]
    fn genesis_boundary_counts_all_forced() {
        assert!(counts_since(
            Some("2020-01-01T00:00:00Z"),
            &Boundary::Genesis
        ));
        assert!(counts_since(None, &Boundary::Genesis));
    }

    /// A dated boundary excludes ONLY a strictly-older dated row; a newer row, a
    /// tie, and an undated row all count (visibility).
    #[test]
    fn dated_boundary_over_reports() {
        let b = Boundary::LastApply("2026-07-23T00:00:00Z".to_owned());
        assert!(
            !counts_since(Some("2026-07-22T23:59:59Z"), &b),
            "strictly older is excluded"
        );
        assert!(
            counts_since(Some("2026-07-23T00:00:00Z"), &b),
            "a tie counts"
        );
        assert!(
            counts_since(Some("2026-07-23T00:00:01Z"), &b),
            "newer counts"
        );
        assert!(counts_since(None, &b), "an undated row counts");
    }

    /// The last realise-apply `now` is read from the receipt ledger's last
    /// `- run {json}` line; an absent / undated ledger is genesis.
    #[test]
    fn last_receipt_now_reads_the_latest_apply() {
        let ledger = "# realise receipts\n\
            - run {\"page\":\"a.md\",\"now\":\"2026-07-23T09:00:00Z\"} ^r-000001\n\
            - run {\"page\":\"b.md\",\"now\":\"2026-07-23T10:00:00Z\"} ^r-000002\n";
        assert_eq!(
            last_receipt_now(ledger).as_deref(),
            Some("2026-07-23T10:00:00Z")
        );
        assert_eq!(last_receipt_now(""), None);
        assert_eq!(last_receipt_now("# empty\n"), None);
    }

    /// End-to-end forced count over a journal: two force rows after the boundary,
    /// one before — the boundary excludes only the strictly-older one.
    #[test]
    fn forced_since_counts_rows_after_boundary() {
        let journal = "# journal\n\
            - op=splice path=a.md root_before=b3:0 root_after=b3:1 edits=1 ^r-000001\n\
            - op=force path=x.md actor=agent:a root_before=b3:1 root_after=b3:2 edits=0 forced_rule=rule-old ^r-000002\n\
            - op=force path=y.md actor=agent:a root_before=b3:2 root_after=b3:3 edits=0 forced_rule=rule-new ^r-000003\n";
        // Manually stamp `now` onto the force rows by re-parsing after injecting.
        let journal = journal
            .replace(
                "forced_rule=rule-old ^r-000002",
                "now=2026-07-22T00:00:00Z forced_rule=rule-old ^r-000002",
            )
            .replace(
                "forced_rule=rule-new ^r-000003",
                "now=2026-07-24T00:00:00Z forced_rule=rule-new ^r-000003",
            );
        let rows = parse_rows(&journal);
        let boundary = Boundary::LastApply("2026-07-23T00:00:00Z".to_owned());
        let (count, violations) = forced_since(&journal, &rows, &boundary);
        assert_eq!(count, 1, "only the post-boundary force row counts");
        assert_eq!(violations[0].rule, "rule-new");
        assert_eq!(violations[0].anchor, "r-000003");

        // Genesis counts both.
        let (all, _) = forced_since(&journal, &rows, &Boundary::Genesis);
        assert_eq!(all, 2, "genesis counts every forced write");
    }

    // ── the RFC3339 → epoch parser (anchor age arithmetic) ───────────────────

    /// Known epoch anchors round-trip; a non-canonical token is undatable
    /// (`None`), so the anchor degrades to ageless rather than mis-dating.
    #[test]
    fn rfc3339_parses_canonical_utc_only() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339("2000-01-01T00:00:00Z"), Some(946_684_800));
        assert_eq!(parse_rfc3339("2026-07-23T00:00:00Z"), Some(1_784_764_800));
        // Non-canonical shapes are undatable.
        assert_eq!(parse_rfc3339("2026-07-23"), None);
        assert_eq!(parse_rfc3339("2026-07-23T00:00:00+00:00"), None);
        assert_eq!(parse_rfc3339("not-a-date"), None);
    }
}
