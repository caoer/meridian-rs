//! `mrd check` — the pure READ validity verb (U2.10; d2 §3 check).
//!
//! ```text
//! mrd check [--core] [--json]
//! ```
//!
//! Runs the convention-free CORE (layer 0) over the resolved workspace: date the
//! receipt journal against the live tree (last-receipt-vs-live) and, when that
//! holds, recompute the journal's chain continuity; then read the PIN PLANE — the
//! pin verdicts and the anchoring state of every pinned blob. `status = freshness,
//! check = validity` — this answers "what lies?", writing nothing and minting no
//! receipt.
//!
//! **U14 — the two planes fail independently.** Until this verb could see the pin
//! plane, a green here meant *"baseline provable AND nothing the JOURNAL plane can
//! see"*, and the fence built on it passed a corpus whose lock arrived by clone or
//! pull while its source moved (`check` green / `walk` `red content-drifted` /
//! `status` `lock red content-drifted`, one corpus, one run) and a corpus holding
//! a blob no ref reaches — a fact no journal row will ever carry. The pin colours
//! come from `view::walk::lock_pin_colors`, the SAME call `mrd status`'s lock axis
//! makes over the SAME corpus build, so the three planes agree by construction and
//! not by coincidence.
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

use check::{CoreReport, GREY_CANNOT_ASSESS, JournalTrace, PinRow};
use receipt::anchor::{ObjectAnchor, PENDING_ANCHOR_TTL};
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

    // ONE corpus build feeds BOTH planes. `mrd status` states the reason for its
    // own two axes and it is the same one here: a second build would let the pin
    // plane and the anchoring read describe two different corpora.
    let docs = crate::walk_cmd::build_docs(&canonical)?;
    let pins = pin_rows(&docs);

    let report = check::core(&root, &docs, &[], &pins)
        .map_err(|e| Fail::tool(format!("check core failed: {e}")))?;

    match parsed.format {
        Format::Json => {
            let value = to_json(&canonical, &report);
            println!("{}", serde_json::to_string_pretty(&value).expect("json"));
        }
        Format::Human => print!("{}", render_human(&canonical, &report)),
    }

    // Worst-of: red is reported first, grey next, green last. Both refuse on the
    // SAME leg (S3-R6: the exit code answers only "may this proceed?"; no fourth
    // code), so the prefix is the same verb and the REASON WORD in each line is
    // what tells a finding from an absence of evidence. Saying "found a lie" over
    // a pending-anchor blob would be a claim wider than the evidence — nothing
    // lied, a blob is simply held by nothing durable.
    if report.is_red() {
        let summary = report.red_summary().unwrap_or_default();
        return Err(Fail {
            code: EXIT_FINDING,
            message: format!("check refuses: {}", summary.replace('\n', "; ")),
        });
    }
    if let Some(grey) = report.grey_summary() {
        return Err(Fail {
            code: EXIT_FINDING,
            message: format!("check refuses: {}", grey.replace('\n', "; ")),
        });
    }
    Ok(())
}

/// Colour every `meridian-lock` pin in the corpus through **the one pin
/// computer** — `view::walk::lock_pin_colors`, which is exactly what
/// `mrd status`'s lock axis reads and what colours a `mrd walk` listing.
///
/// This is the seam that makes the three planes agree BY CONSTRUCTION. A `check`
/// that re-derived pin colours would be a second implementation of corpus index →
/// ref resolution → selector → fingerprint compare, and a second copy of that
/// chain is how the pin plane and the decoration plane once came to hash two
/// different documents for one ref. There is one computer here, not three that
/// happen to match today.
///
/// The label rides along from `color_label` for the same reason: the reason words
/// (`content-drifted`, `unmounted`, `path-unseeable`, …) are spelled once, in
/// `view`, and are never re-spelled by this verb (S3-R6/S3-R59).
fn pin_rows(docs: &std::collections::BTreeMap<String, model::Document>) -> Vec<PinRow> {
    view::walk::lock_pin_colors(docs)
        .into_iter()
        .map(|pin| PinRow {
            src_path: pin.src_path,
            declared_ref: pin.declared_ref,
            label: view::walk::color_label(&pin.color),
            color: pin.color,
        })
        .collect()
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
                 receipt ^{} (recorded root_after={}); something advanced the tree that the \
                 journal does not account for, and an out-of-writer edit is not the only door \
                 that leaves this trace",
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

    // ── the PIN PLANE (U14) ─────────────────────────────────────────────────
    // Two lines, always both present, because their silences mean different
    // things: `pins:` reads the CLAIM plane (did the content drift) and
    // `anchoring:` reads the RETRIEVAL plane (is the blob durably held). A verb
    // that printed only the failing one would leave a reader unable to tell
    // "assessed and clean" from "never looked".
    let pins = &report.pins;
    if pins.red.is_empty() && pins.grey.is_empty() {
        let _ = writeln!(out, "  pins: green");
    } else {
        for pin in pins.red.iter().chain(&pins.grey) {
            let _ = writeln!(out, "  pins: {}", pin_line(pin));
        }
    }
    // The anchoring THREE-STATE as a reading (GAP A), with its POPULATION beside
    // it (S3-R23(5)): the same empty orphan list means one thing over fifty pinned
    // blobs and something else entirely over none, and a reading that cannot tell
    // them apart is how coverage disappears with nothing failing.
    if let Some(detail) = &pins.cannot_ask {
        let _ = writeln!(out, "  anchoring: {GREY_CANNOT_ASSESS} — {detail}");
    } else if pins.asked() == 0 {
        let _ = writeln!(out, "  anchoring: no pinned objects");
    } else {
        let _ = writeln!(
            out,
            "  anchoring: {} {} · {} {} · {} {}",
            pins.anchored,
            ObjectAnchor::Anchored.word(),
            pins.pending,
            ObjectAnchor::PendingAnchor.word(),
            pins.never,
            ObjectAnchor::NeverAnchored.word()
        );
        if pins.pending > 0 {
            let _ = writeln!(out, "  {PENDING_ANCHOR_TTL}");
        }
        for orphan in &pins.orphaned {
            let _ = writeln!(
                out,
                "  anchoring: {} ORPHANED — {} objects.{} ({}) is reachable from no ref and the \
                 file hashes to {} now, so no commit will anchor it",
                orphan.state.word(),
                orphan.src_path,
                orphan.key,
                orphan.blob_sha,
                orphan.live
            );
        }
    }
    out
}

/// One pin row as a render line: the page, the ref it declares, and the colour
/// label its ONE computer produced. Never re-spells a reason word.
fn pin_line(pin: &PinRow) -> String {
    if pin.declared_ref.is_empty() {
        format!("{} — {}", pin.label, pin.src_path)
    } else {
        format!("{} — {} → {}", pin.label, pin.src_path, pin.declared_ref)
    }
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
    let pins = pins_json(report);

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
                "detail": report.trace.grey_summary().unwrap_or_default(),
                "baseline": baseline,
            },
            "core": {
                "chain": Value::Null,
                "foreign_edit": Value::Null,
                "drifted_claims": claims,
            },
            "pins": pins,
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
        },
        "pins": pins,
    })
}

/// The `pins` block: the CLAIM plane's findings and the RETRIEVAL plane's
/// anchoring reading, each carrying its own reason word verbatim (S3-R6 — distinct
/// on the `--json` face as well as the human one).
///
/// `anchoring` is `null` when the object store could not be asked, and the reason
/// is stated in `anchoring_cannot_assess` — *not assessed*, never an empty array a
/// reader could bank as clean. The `pending_anchor` array is a reading of a plane
/// that WAS asked, so its emptiness means something; a `null` says nothing was.
fn pins_json(report: &CoreReport) -> Value {
    let pins = &report.pins;
    let row = |p: &PinRow| json!({ "src_path": p.src_path, "declared_ref": p.declared_ref, "color": p.label });
    let orphaned: Vec<Value> = pins
        .orphaned
        .iter()
        .map(|o| {
            json!({
                "src_path": o.src_path,
                "key": o.key,
                "blob_sha": o.blob_sha,
                "state": o.state.word(),
                "live": o.live,
                "nudge": PENDING_ANCHOR_TTL,
            })
        })
        .collect();
    json!({
        "red": pins.red.iter().map(row).collect::<Vec<_>>(),
        "grey": pins.grey.iter().map(row).collect::<Vec<_>>(),
        "anchoring": match &pins.cannot_ask {
            Some(_) => Value::Null,
            // The three-state READING plus its POPULATION (S3-R23(5)): an empty
            // `orphaned` over `asked: 0` is a reading of nothing, not a clean bill.
            None => json!({
                "asked": pins.asked(),
                "anchored": pins.anchored,
                "pending_anchor": pins.pending,
                "never_anchored": pins.never,
                "orphaned": orphaned,
            }),
        },
        "anchoring_cannot_assess": match &pins.cannot_ask {
            Some(detail) => json!({ "reason": GREY_CANNOT_ASSESS, "detail": detail }),
            None => Value::Null,
        },
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
