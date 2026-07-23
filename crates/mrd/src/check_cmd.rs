//! `mrd check` — the pure READ validity verb (U2.10; d2 §3 check).
//!
//! ```text
//! mrd check [--core] [--json]
//! ```
//!
//! Runs the convention-free CORE (layer 0) over the resolved workspace: recompute the
//! receipt journal's chain continuity and the `foreign_edit` trace
//! (last-receipt-vs-live). `status = freshness, check = validity` — this answers
//! "what lies?", writing nothing and minting no receipt.
//!
//! `--core` names layer 0 explicitly (the default today). The armed layer-1
//! evaluation is the `check` engine surface the door mounts (U4.2) — its
//! change-framing over a whole tree lands with that door, not this verb.
//!
//! Read-only. Exit triad (§4 preamble):
//! - **0** — green: the chain is continuous and no foreign edit.
//! - **1** — a finding: a broken journal chain (cites the row) or a `foreign_edit`
//!   (an out-of-writer edit). A check finding, never a door refusal
//!   (refusal-amendment).
//! - **2** — bad invocation, or an unreadable workspace / journal.

use std::path::Path;

use check::CoreReport;
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
fn render_human(workspace: &Path, report: &CoreReport) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "check core {}", workspace.display());

    if let Some(summary) = report.trace.chain.red_summary() {
        let _ = writeln!(out, "  chain: RED — {summary}");
    } else {
        let _ = writeln!(out, "  chain: green");
    }

    match &report.trace.foreign_edit {
        Some(fe) => {
            let _ = writeln!(
                out,
                "  foreign_edit: RED — tree root {} does not continue the last receipt ^{} \
                 (recorded root_after={})",
                fe.live_root, fe.last_receipt, fe.recorded_root
            );
        }
        None => {
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
fn to_json(workspace: &Path, report: &CoreReport) -> Value {
    let breaks: Vec<Value> = report
        .trace
        .chain
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
    let foreign_edit = report.trace.foreign_edit.as_ref().map(|fe| {
        json!({
            "last_receipt": fe.last_receipt,
            "recorded_root": fe.recorded_root,
            "live_root": fe.live_root,
        })
    });
    let claims: Vec<Value> = report
        .drifted_claims
        .iter()
        .map(|c| json!({ "selector": c.selector, "detail": c.detail }))
        .collect();
    json!({
        "workspace": workspace.display().to_string(),
        "red": report.is_red(),
        "core": {
            "chain": { "green": report.trace.chain.is_green(), "breaks": breaks },
            "foreign_edit": foreign_edit,
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
