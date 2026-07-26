//! `mrd check` — the pure READ validity verb (U2.10; d2 §3 check).
//!
//! ```text
//! mrd check [--core] [--json]
//! ```
//!
//! Runs the convention-free CORE (layer 0) over the resolved workspace: date the
//! receipt journal against the live tree (last-receipt-vs-live) and, when that
//! holds, recompute the journal's chain continuity. `status = freshness, check =
//! validity` — this answers "what lies?", writing nothing and minting no receipt.
//!
//! `--core` names layer 0 explicitly (the default today). The armed layer-1
//! evaluation is the `check` engine surface the door mounts (U4.2) — its
//! change-framing over a whole tree lands with that door, not this verb.
//!
//! Read-only. Exit triad (§4 preamble):
//! - **0** — green: the journal dates the live tree and its chain is continuous.
//! - **1** — a finding: a broken journal chain (cites the row). A check finding,
//!   never a door refusal (refusal-amendment). **Grey rides this leg too** (S3-R5
//!   and S3-R8, spelled by S3-R6): when the journal cannot date the tree — no rows,
//!   or a last receipt the live tree no longer matches — the verb refuses
//!   `grey(cannot-assess)`. Unknown is not clean, and a hook that rejects on
//!   non-zero must reject what nobody could vouch for. The triad stays CLOSED: no
//!   fourth code. The exit answers "may this proceed?" (red and grey both say no);
//!   the reason word, distinct on both faces, says why.
//! - **2** — bad invocation, or an unreadable workspace / journal.

use std::path::Path;

use check::{CoreReport, GREY_CANNOT_ASSESS, JournalTrace};
use serde_json::{Value, json};

use crate::{Fail, Format, current_dir};

/// The finding leg of the triad: the invocation was well-formed, the core found a
/// lie (a chain break or a foreign edit).
const EXIT_FINDING: u8 = 1;

/// Run `mrd check [--core] [--json]`: resolve the workspace and run the layer-0
/// core, printing the verdict.
///
/// # Errors
/// [`Fail`] exit 2 on a bad invocation or an unreadable workspace/journal; exit 1
/// when the core reddens (chain break or foreign edit).
pub(crate) fn dispatch(args: &[String]) -> Result<(), Fail> {
    let parsed = Check::parse(args)?;
    let cwd = current_dir()?;
    let resolved = crate::resolve::resolve_runtime(&cwd).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace for {}: {e}",
            cwd.display()
        ))
    })?;
    let canonical = workspace::canonicalize(&resolved.workspace).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace {} ({e})",
            resolved.workspace.display()
        ))
    })?;
    let root = fs::WorkspaceRoot(canonical.clone());

    let report =
        check::core(&root, &[]).map_err(|e| Fail::tool(format!("check core failed: {e}")))?;

    match parsed.format {
        Format::Json => {
            let value = to_json(&canonical, &report);
            println!("{}", serde_json::to_string_pretty(&value).expect("json"));
        }
        Format::Human => print!("{}", render_human(&canonical, &report)),
    }

    if report.is_red() {
        let summary = report.red_summary().unwrap_or_default();
        return Err(Fail {
            code: EXIT_FINDING,
            message: format!("check found a lie: {}", summary.replace('\n', "; ")),
        });
    }
    // Worst-of: red is reported first, grey next, green last. Grey is not a lie —
    // it is the absence of the evidence a green would have to rest on. It refuses
    // on the SAME leg as a finding (S3-R6: the exit code answers only "may this
    // proceed?"), and the reason word is what tells the two apart.
    if let Some(grey) = report.grey_summary() {
        return Err(Fail {
            code: EXIT_FINDING,
            message: format!("check refuses {GREY_CANNOT_ASSESS}: {grey}"),
        });
    }
    Ok(())
}

/// The parsed `check` invocation: the output format (the `--core` flag names
/// layer 0 explicitly, the default today, so it carries no extra state).
#[derive(Debug)]
struct Check {
    format: Format,
}

impl Check {
    fn parse(args: &[String]) -> Result<Self, Fail> {
        let mut json = false;
        for arg in args {
            match arg.as_str() {
                "--json" => json = true,
                // `--core` names layer 0 explicitly; it is the default today, so it
                // is accepted and needs no separate branch.
                "--core" => {}
                flag if flag.starts_with('-') => {
                    return Err(Fail::tool(format!("unknown flag: {flag}")));
                }
                value => {
                    return Err(Fail::tool(format!("unexpected argument: {value}")));
                }
            }
        }
        Ok(Check {
            format: if json { Format::Json } else { Format::Human },
        })
    }
}

/// Render the core verdict as a human block: the header, the chain line, the
/// `foreign_edit` line, and one line per drifted claim (none at the CLI today).
///
/// Both detector lines render `grey(cannot-assess)` when the journal cannot date
/// the tree — with no row, or with a last receipt the live root no longer
/// continues. Neither may borrow the word the assessed path earns, and neither may
/// accuse: the mismatch is rendered as the evidence it is.
fn render_human(workspace: &Path, report: &CoreReport) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "check core {}", workspace.display());

    match &report.trace {
        JournalTrace::NoBaseline => {
            let _ = writeln!(
                out,
                "  chain: {GREY_CANNOT_ASSESS} — the receipt journal carries no row, so there \
                 is no chain to recompute"
            );
            let _ = writeln!(
                out,
                "  foreign_edit: {GREY_CANNOT_ASSESS} — the receipt journal carries no last \
                 receipt to attribute the live tree against"
            );
        }
        JournalTrace::StaleBaseline(m) => {
            let _ = writeln!(
                out,
                "  chain: {GREY_CANNOT_ASSESS} — the journal's last receipt ^{} does not account \
                 for the live tree, so its rows cannot be read against it",
                m.last_receipt
            );
            let _ = writeln!(
                out,
                "  foreign_edit: {GREY_CANNOT_ASSESS} — tree root {} does not continue the last \
                 receipt ^{} (recorded root_after={}); a governed splice moves the root and \
                 journals no row, so this is NOT distinguishable from an out-of-writer edit",
                m.live_root, m.last_receipt, m.recorded_root
            );
        }
        JournalTrace::Assessed { chain } => {
            if let Some(summary) = chain.red_summary() {
                let _ = writeln!(out, "  chain: RED — {summary}");
            } else {
                let _ = writeln!(out, "  chain: green");
            }
            let _ = writeln!(out, "  foreign_edit: none");
        }
    }

    for claim in &report.drifted_claims {
        let _ = writeln!(
            out,
            "  claim not realised: {} — {}",
            claim.selector, claim.detail
        );
    }
    out
}

/// The `--json` shape: the workspace plus the core object (chain breaks, the
/// `foreign_edit`, the drifted claims) and the top-level `red` verdict.
///
/// When the journal cannot date the tree, both journal detectors are `null` —
/// *not assessed*, never a `{"green": true}` a reader could bank on — and a
/// `cannot_assess` block carries the reason word, the detectors it covers, the
/// detail, and the `baseline` evidence (`null` when there is no row at all).
/// `red` stays honest: grey is not red. The assessed shape is untouched.
fn to_json(workspace: &Path, report: &CoreReport) -> Value {
    let claims: Vec<Value> = report
        .drifted_claims
        .iter()
        .map(|c| json!({ "selector": c.selector, "detail": c.detail }))
        .collect();

    let JournalTrace::Assessed { chain } = &report.trace else {
        let baseline = match &report.trace {
            JournalTrace::StaleBaseline(m) => json!({
                "last_receipt": m.last_receipt,
                "recorded_root": m.recorded_root,
                "live_root": m.live_root,
            }),
            _ => Value::Null,
        };
        return json!({
            "workspace": workspace.display().to_string(),
            "red": report.is_red(),
            "cannot_assess": {
                "reason": GREY_CANNOT_ASSESS,
                "detectors": ["chain", "foreign_edit"],
                "detail": report.grey_summary().unwrap_or_default(),
                "baseline": baseline,
            },
            "core": {
                "chain": Value::Null,
                "foreign_edit": Value::Null,
                "drifted_claims": claims,
            }
        });
    };

    let breaks: Vec<Value> = chain
        .breaks
        .iter()
        .map(|b| {
            json!({
                "row_anchor": b.row_anchor,
                "line_no": b.line_no,
                "expected_root_before": b.expected_root_before,
                "found_root_before": b.found_root_before,
            })
        })
        .collect();
    json!({
        "workspace": workspace.display().to_string(),
        "red": report.is_red(),
        "core": {
            "chain": { "green": chain.is_green(), "breaks": breaks },
            // Assessed ⇔ the last receipt accounts for the live tree, so this key
            // is null by construction here. It stays in the shape: an absent field
            // reads as "not checked", and this one WAS checked (S3-R8 moved its
            // only non-null case into `cannot_assess`).
            "foreign_edit": Value::Null,
            "drifted_claims": claims,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_core_and_json() {
        let c = Check::parse(&["--core".to_string(), "--json".to_string()]).expect("parse");
        assert!(matches!(c.format, Format::Json));
    }

    #[test]
    fn parse_rejects_unknown_flag() {
        assert_eq!(Check::parse(&["--nope".to_string()]).unwrap_err().code, 2);
    }

    #[test]
    fn parse_rejects_stray_positional() {
        assert_eq!(Check::parse(&["extra".to_string()]).unwrap_err().code, 2);
    }
}
