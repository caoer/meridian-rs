//! Layer 0 — the convention-free core of `check` (d2 §3 check: "Layer 0 (`check
//! --core`): chain integrity by full recompute, claims realised, and the
//! mechanical journal TRACE … pack-free").
//!
//! Three pack-free reads over the run plane, none of which writes a byte:
//! - [`journal_trace`] — chain recompute (the U2.1 [`receipt::journal::check_chain`]
//!   primitive) + the `foreign_edit` detector (last-receipt-vs-live);
//! - [`claims_realised`] — observe each claim against the current tree and report
//!   the drifted ones (the realise engine's pure detection, run read-only here).

use std::io;

use fs::WorkspaceRoot;
use realise::{CheckOutcome, Claim};
use receipt::journal::{ChainReport, ParsedRow, check_chain, parse_rows};

/// The journal TRACE over a workspace: chain continuity by full recompute plus
/// the `foreign_edit` detector. Both are mechanical, source-3 facts of the
/// reserved receipt journal and the live tree — no convention, no cap.
#[derive(Debug)]
pub struct JournalTrace {
    /// The chain-continuity report from the U2.1 primitive — red cites the
    /// spliced/forged row that fails to continue the chain.
    pub chain: ChainReport,
    /// The `foreign_edit` finding, present iff the live tree root no longer
    /// continues the last receipt's recorded `root_after` — an out-of-writer
    /// edit with no receipt.
    pub foreign_edit: Option<ForeignEdit>,
}

/// A `foreign_edit`: the tree moved with no journal row to explain it (d2 §3;
/// refusal-amendment: "an out-of-writer edit renders red convention-free; a check
/// finding, not a door refusal"). Cites the last governed receipt and both roots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignEdit {
    /// The last receipt row's anchor (`r-NNNNNN`) — the last governed write.
    pub last_receipt: String,
    /// The tree root the last receipt recorded (`root_after`).
    pub recorded_root: String,
    /// The live tree root now — it differs, so an out-of-writer edit landed.
    pub live_root: String,
}

impl JournalTrace {
    /// The TRACE found a lie: a broken chain or a foreign edit.
    #[must_use]
    pub fn is_red(&self) -> bool {
        self.chain.is_red() || self.foreign_edit.is_some()
    }

    /// A red render citing the broken row and/or the foreign edit, or `None` when
    /// the TRACE is green. This is the string the `check --core` verb renders.
    #[must_use]
    pub fn red_summary(&self) -> Option<String> {
        if !self.is_red() {
            return None;
        }
        let mut lines = Vec::new();
        if let Some(chain) = self.chain.red_summary() {
            lines.push(chain);
        }
        if let Some(fe) = &self.foreign_edit {
            lines.push(format!(
                "foreign_edit: the tree root {} does not continue the last receipt ^{} \
                 (recorded root_after={}) — an out-of-writer edit landed with no receipt",
                fe.live_root, fe.last_receipt, fe.recorded_root
            ));
        }
        Some(lines.join("\n"))
    }
}

/// Recompute the journal TRACE over `root`: read the reserved receipt journal
/// page, recompute chain continuity ([`check_chain`]), and compare the last
/// receipt's recorded `root_after` against the live tree root — a mismatch is a
/// [`ForeignEdit`]. Reads two things (the journal bytes and the tree fold); writes
/// nothing.
///
/// The journal is root-EXCLUDED, so its own bytes never enter the live tree fold
/// — splicing a forged row changes the chain (caught by [`check_chain`]) but not
/// the tree root, keeping the two detectors independent.
///
/// # Errors
/// [`io::Error`] when the journal page or the domain snapshot cannot be read.
pub fn journal_trace(root: &WorkspaceRoot) -> io::Result<JournalTrace> {
    let page = read_journal(root)?;
    let rows = parse_rows(&page);
    let chain = check_chain(&rows);
    let live_root = fs::domain_snapshot(root)?.1.0;
    let foreign_edit = detect_foreign_edit(&rows, &live_root);
    Ok(JournalTrace {
        chain,
        foreign_edit,
    })
}

/// **The writer-attribution check** (d2 §3 journal TRACE, "last-receipt-vs-live
/// catching `foreign_edit`"): the live tree root must equal the last receipt's
/// recorded `root_after`, so every byte of the tree is attributable to a governed
/// write. When they differ, an out-of-writer edit landed with no receipt — a
/// [`ForeignEdit`]. An empty journal has no last receipt, so no baseline and no
/// finding (nothing was ever governed to attribute against).
fn detect_foreign_edit(rows: &[ParsedRow], live_root: &str) -> Option<ForeignEdit> {
    let last = rows.last()?;
    if last.root_after == live_root {
        return None;
    }
    Some(ForeignEdit {
        last_receipt: last.anchor.clone(),
        recorded_root: last.root_after.clone(),
        live_root: live_root.to_string(),
    })
}

/// Read the reserved receipt journal page bytes, or the empty string when the
/// page does not exist yet (a genesis workspace has no journal). Reads the raw
/// bytes directly — the row grammar is line-oriented, so no parse is needed.
fn read_journal(root: &WorkspaceRoot) -> io::Result<String> {
    let page = root.0.join(fs::domain::RESERVED_JOURNAL_PATH);
    match std::fs::read_to_string(&page) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e),
    }
}

/// One claim that is NOT realised: its selector and why the observation drifted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimFinding {
    /// The claim selector (the board-card key; d2 §5.4).
    pub selector: String,
    /// Why the check failed — the observed-vs-expected detail from the drift.
    pub detail: String,
}

/// Claims-realised (d2 §3): observe each claim against the current tree and
/// report the ones that did not converge. A pure read — no apply, no cap (the
/// `check` verb never converges; the realise loop does). Reuses the realise
/// engine's [`Check`] detection rather than forking it, so `check` reads exactly
/// what `realise` would converge.
///
/// # Errors
/// [`realise::CheckError`] when a claim's observation itself faults (page load /
/// I/O) — distinct from a clean [`CheckOutcome::Drifted`], which is a finding, not
/// an error.
pub fn claims_realised(
    root: &WorkspaceRoot,
    claims: &[Claim],
) -> Result<Vec<ClaimFinding>, realise::CheckError> {
    let mut drifted = Vec::new();
    for claim in claims {
        match claim.check.observe(root)? {
            CheckOutcome::Converged => {}
            CheckOutcome::Drifted { detail } => drifted.push(ClaimFinding {
                selector: claim.selector.clone(),
                detail,
            }),
        }
    }
    Ok(drifted)
}

#[cfg(test)]
mod tests {
    use realise::Check;

    use super::*;

    /// Build a [`ParsedRow`] with the two roots and an anchor — the columns the
    /// `foreign_edit` detector reads (line number is irrelevant to detection).
    fn row(anchor: &str, root_before: &str, root_after: &str) -> ParsedRow {
        ParsedRow {
            anchor: anchor.to_string(),
            op: "splice".to_string(),
            path: "a.md".to_string(),
            actor: None,
            now: None,
            root_before: root_before.to_string(),
            root_after: root_after.to_string(),
            edits: 0,
            line_no: 1,
        }
    }

    /// A read-only claim (no apply) over a fixed [`CheckOutcome`] — the pure-read
    /// half of a realise [`Claim`], for the claims-realised tests.
    fn fixed_claim(selector: &str, outcome: CheckOutcome) -> Claim {
        struct Fixed(CheckOutcome);
        impl Check for Fixed {
            fn observe(&self, _root: &WorkspaceRoot) -> Result<CheckOutcome, realise::CheckError> {
                Ok(self.0.clone())
            }
        }
        Claim {
            selector: selector.to_string(),
            check: Box::new(Fixed(outcome)),
            apply: None,
            retry_budget: 0,
        }
    }

    /// The writer-attribution check fires: when the live root differs from the
    /// last receipt's recorded `root_after`, a `foreign_edit` is detected citing
    /// the last receipt and both roots. Dropping the `root_after == live_root`
    /// comparison makes [`detect_foreign_edit`] return `None`, and this test FAILS
    /// (the gate's falsification).
    #[test]
    fn foreign_edit_fires_when_live_root_drifts_from_last_receipt() {
        let rows = [
            row("r-000001", "b3:0", "b3:1"),
            row("r-000002", "b3:1", "b3:2"),
        ];
        let fe = detect_foreign_edit(&rows, "b3:LIVE_DRIFTED")
            .expect("live root != last receipt root_after must fire foreign_edit");
        assert_eq!(fe.last_receipt, "r-000002", "cites the last receipt");
        assert_eq!(fe.recorded_root, "b3:2", "the recorded root_after");
        assert_eq!(fe.live_root, "b3:LIVE_DRIFTED", "the live root");
    }

    /// The writer-attribution check is load-bearing in BOTH directions: when the
    /// live root EQUALS the last receipt's recorded root, the tree is attributed
    /// and there is NO `foreign_edit`. A detector that always fired fails here; one
    /// that never fired fails the test above. Together they pin the comparison.
    #[test]
    fn no_foreign_edit_when_live_root_matches_last_receipt() {
        let rows = [
            row("r-000001", "b3:0", "b3:1"),
            row("r-000002", "b3:1", "b3:2"),
        ];
        assert_eq!(
            detect_foreign_edit(&rows, "b3:2"),
            None,
            "live root == last receipt root_after ⇒ attributed, no foreign_edit"
        );
    }

    /// An empty journal has no last receipt — no baseline to attribute against, so
    /// no `foreign_edit` finding (a genesis tree is not a lie).
    #[test]
    fn empty_journal_has_no_foreign_edit() {
        assert_eq!(detect_foreign_edit(&[], "b3:anything"), None);
    }

    /// A drifted claim surfaces as a [`ClaimFinding`]; a converged one does not.
    #[test]
    fn claims_realised_reports_only_drifted_claims() {
        let root = WorkspaceRoot(std::path::PathBuf::from("/nonexistent"));
        let claims = [
            fixed_claim("green", CheckOutcome::Converged),
            fixed_claim(
                "red",
                CheckOutcome::Drifted {
                    detail: "status: 'open' is 'closed'".to_string(),
                },
            ),
        ];
        let drifted = claims_realised(&root, &claims).expect("clean observations");
        assert_eq!(drifted.len(), 1, "only the drifted claim surfaces");
        assert_eq!(drifted[0].selector, "red");
        assert_eq!(drifted[0].detail, "status: 'open' is 'closed'");
    }

    /// A claim whose observation faults propagates the error (fail-loud), never a
    /// false green.
    #[test]
    fn claims_realised_propagates_an_observation_fault() {
        struct Faults;
        impl Check for Faults {
            fn observe(&self, _root: &WorkspaceRoot) -> Result<CheckOutcome, realise::CheckError> {
                Err(realise::CheckError {
                    selector: "faulty".to_string(),
                    reason: "observation faulted".to_string(),
                })
            }
        }
        let root = WorkspaceRoot(std::path::PathBuf::from("/nonexistent"));
        let claim = Claim {
            selector: "faulty".to_string(),
            check: Box::new(Faults),
            apply: None,
            retry_budget: 0,
        };
        let err = claims_realised(&root, std::slice::from_ref(&claim))
            .expect_err("a faulting observation is an error, not a drift");
        assert_eq!(err.reason, "observation faulted");
    }
}
