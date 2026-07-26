//! Layer 0 — the convention-free core of `check` (d2 §3 check: "Layer 0 (`check
//! --core`): chain integrity by full recompute, claims realised, and the
//! mechanical journal TRACE … pack-free").
//!
//! Three pack-free reads over the run plane, none of which writes a byte:
//! - [`journal_trace`] — the baseline check (last-receipt-vs-live) and, only when
//!   that baseline is current, the chain recompute (the U2.1
//!   [`receipt::journal::check_chain`] primitive);
//! - [`claims_realised`] — observe each claim against the current tree and report
//!   the drifted ones (the realise engine's pure detection, run read-only here).

use std::io;

use fs::WorkspaceRoot;
use realise::{CheckOutcome, Claim};
use receipt::journal::{ChainReport, ParsedRow, check_chain, parse_rows};

/// The journal TRACE over a workspace: what the reserved receipt journal can
/// still prove about the live tree. Mechanical, source-3, no convention, no cap.
///
/// **A verdict requires a CURRENT baseline** (S3-R8). Both journal detectors rest
/// on one assumption — that the last receipt's recorded `root_after` still
/// accounts for the live tree — and that assumption fails in two measured ways:
///
/// - the journal carries **no row** ([`JournalTrace::NoBaseline`]): a workspace
///   with no guarded write behind it has nothing to compare against, so a silent
///   detector proves nothing;
/// - the last receipt **no longer matches** the live tree
///   ([`JournalTrace::StaleBaseline`]): something advanced the tree that the
///   journal does not account for.
///
/// So the trace spells both as states of its own instead of letting the detectors
/// report a green they never earned (the false green) or a red they cannot support
/// (the false red). *Outside sight is never verified* (R26; S5 honest degradation).
///
/// **U32 closed the first regime and narrowed the second.** A governed `splice`
/// now journals its row, so an ordinary pin/put workspace HAS a baseline and reads
/// a real verdict. What remains grey rather than red is the attribution: a write
/// door that lands bytes without journaling leaves the identical trace as an
/// out-of-writer edit, and one such door is still open by charter (`mrd realise
/// --truth file`'s bare `std::fs::write`, U31/U12). Naming the cause is a claim
/// this crate cannot yet support; stating the evidence is one it can.
#[derive(Debug)]
pub enum JournalTrace {
    /// **Grey — cannot assess.** The reserved journal carries no row: no pair of
    /// rows to recompute a chain over, no last receipt to attribute the live tree
    /// against.
    NoBaseline,
    /// **Grey — cannot assess.** The journal carries rows, but its last receipt
    /// does not account for the live tree. The cause is NOT determinable from
    /// here: an out-of-writer edit and any write door that lands bytes without
    /// journaling leave the identical evidence.
    StaleBaseline(BaselineMismatch),
    /// The last receipt accounts for the live tree: the baseline is current, so the
    /// journal's own rows are a verdict.
    Assessed {
        /// The chain-continuity report from the U2.1 primitive — red cites the
        /// spliced/forged row that fails to continue the chain.
        chain: ChainReport,
    },
}

/// The evidence of a stale baseline: the last governed receipt, the root it
/// recorded, and the root the tree folds to now.
///
/// This was `ForeignEdit`, and the rename is the correction: the same three facts
/// were rendered as the CLAIM *"an out-of-writer edit landed with no receipt"*,
/// which was false on every governed splice — measured on the deployed binary
/// against a fully governed corpus (finding-01). U32 gave the splice its row, so
/// that corpus no longer reaches here at all; the accusation stays withdrawn
/// because a non-journaling write door still leaves this same evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineMismatch {
    /// The last receipt row's anchor (`r-NNNNNN`) — the last journaled write.
    pub last_receipt: String,
    /// The tree root that receipt recorded (`root_after`).
    pub recorded_root: String,
    /// The live tree root now — it differs, so the baseline is out of date.
    pub live_root: String,
}

impl JournalTrace {
    /// The TRACE found a lie: the journal's own rows fail to chain, read against a
    /// baseline that is current. Grey is not red — a trace that could not look at
    /// anything, or could not date what it looked at, is a different answer from
    /// "the tree is honest".
    #[must_use]
    pub fn is_red(&self) -> bool {
        match self {
            JournalTrace::NoBaseline | JournalTrace::StaleBaseline(_) => false,
            JournalTrace::Assessed { chain } => chain.is_red(),
        }
    }

    /// The TRACE could not assess: it has no baseline, or cannot show the baseline
    /// it has is current. Never green, never red — grey.
    #[must_use]
    pub fn cannot_assess(&self) -> bool {
        matches!(
            self,
            JournalTrace::NoBaseline | JournalTrace::StaleBaseline(_)
        )
    }

    /// A red render citing the broken row, or `None` when the TRACE is not red.
    /// This is the string the `check --core` verb renders.
    #[must_use]
    pub fn red_summary(&self) -> Option<String> {
        let JournalTrace::Assessed { chain } = self else {
            return None;
        };
        chain.red_summary()
    }

    /// The grey render: what could not be assessed and why, or `None` when the
    /// TRACE had a current baseline to read.
    #[must_use]
    pub fn grey_summary(&self) -> Option<String> {
        match self {
            JournalTrace::NoBaseline => Some(NO_BASELINE_DETAIL.to_string()),
            JournalTrace::StaleBaseline(m) => Some(format!(
                "the live tree root {} does not continue the last receipt ^{} (recorded \
                 root_after={}) — something advanced the tree that the journal does not account \
                 for, so the journal cannot date the tree and neither detector can be read",
                m.live_root, m.last_receipt, m.recorded_root
            )),
            JournalTrace::Assessed { .. } => None,
        }
    }
}

/// The one sentence that explains the empty-journal grey — the engine states the
/// missing evidence, never a colour alone.
pub const NO_BASELINE_DETAIL: &str = "the receipt journal carries no row, so the chain recompute and the \
     foreign_edit trace have nothing to compare against";

/// The reason word this grey carries, verbatim on BOTH faces — the human render
/// and `--json` — per S3-R6. The colour alone is not the answer: `grey(unmounted)`
/// and `grey(cannot-assess)` are different refusals that share exit 1, so the word
/// is what tells them apart. One spelling, ruled centrally, because U11/U14 render
/// the same grammar.
pub const GREY_CANNOT_ASSESS: &str = "grey(cannot-assess)";

/// Recompute the journal TRACE over `root`: read the reserved receipt journal
/// page, date it against the live tree, and recompute chain continuity
/// ([`check_chain`]) only when the dating holds. Reads two things (the journal
/// bytes and the tree fold); writes nothing.
///
/// The journal is root-EXCLUDED, so its own bytes never enter the live tree fold
/// — splicing a forged row changes the chain (caught by [`check_chain`]) but not
/// the tree root.
///
/// **The baseline is checked before anything is claimed** (S3-R8): zero rows is
/// [`JournalTrace::NoBaseline`]; a last receipt that no longer accounts for the
/// live tree is [`JournalTrace::StaleBaseline`]. Both are grey. Only past that
/// gate does the chain recompute become a verdict a reader can act on.
///
/// # Errors
/// [`io::Error`] when the journal page or the domain snapshot cannot be read.
pub fn journal_trace(root: &WorkspaceRoot) -> io::Result<JournalTrace> {
    let page = read_journal(root)?;
    let rows = parse_rows(&page);
    let live_root = fs::domain_snapshot(root)?.1.0;
    match detect_baseline(&rows, &live_root) {
        None => Ok(JournalTrace::NoBaseline),
        Some(Err(mismatch)) => Ok(JournalTrace::StaleBaseline(mismatch)),
        Some(Ok(())) => Ok(JournalTrace::Assessed {
            chain: check_chain(&rows),
        }),
    }
}

/// **The baseline check** (d2 §3 journal TRACE, "last-receipt-vs-live"): can the
/// journal date the tree it is being read against? `None` = no rows at all;
/// `Some(Err)` = the last receipt's recorded `root_after` is not the live root;
/// `Some(Ok)` = the last receipt accounts for the live tree.
///
/// A mismatch is deliberately NOT called a `foreign_edit` any more. The same
/// evidence is produced by an out-of-writer edit and by any write door that lands
/// bytes without journaling, so naming one of the two is a claim wider than the
/// evidence — measured as a false red on a fully governed corpus (finding-01,
/// whose splice door U32 closed).
fn detect_baseline(rows: &[ParsedRow], live_root: &str) -> Option<Result<(), BaselineMismatch>> {
    let last = rows.last()?;
    if last.root_after == live_root {
        return Some(Ok(()));
    }
    Some(Err(BaselineMismatch {
        last_receipt: last.anchor.clone(),
        recorded_root: last.root_after.clone(),
        live_root: live_root.to_string(),
    }))
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
    /// baseline check reads (line number is irrelevant to detection).
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

    /// The baseline check fires: when the live root differs from the last
    /// receipt's recorded `root_after`, the baseline is stale and the mismatch
    /// cites the last receipt and both roots. Dropping the comparison makes
    /// [`detect_baseline`] answer `Some(Ok)`, and this test FAILS (the gate's
    /// falsification).
    #[test]
    fn baseline_is_stale_when_live_root_drifts_from_last_receipt() {
        let rows = [
            row("r-000001", "b3:0", "b3:1"),
            row("r-000002", "b3:1", "b3:2"),
        ];
        let m = detect_baseline(&rows, "b3:LIVE_DRIFTED")
            .expect("rows are present, so there IS a baseline to date")
            .expect_err("live root != last receipt root_after ⇒ the baseline is stale");
        assert_eq!(m.last_receipt, "r-000002", "cites the last receipt");
        assert_eq!(m.recorded_root, "b3:2", "the recorded root_after");
        assert_eq!(m.live_root, "b3:LIVE_DRIFTED", "the live root");
    }

    /// The baseline check is load-bearing in BOTH directions: when the live root
    /// EQUALS the last receipt's recorded root, the journal dates the tree and the
    /// rows may be read. A check that always faulted fails here; one that never
    /// faulted fails the test above. Together they pin the comparison.
    #[test]
    fn baseline_is_current_when_live_root_matches_last_receipt() {
        let rows = [
            row("r-000001", "b3:0", "b3:1"),
            row("r-000002", "b3:1", "b3:2"),
        ];
        assert_eq!(
            detect_baseline(&rows, "b3:2"),
            Some(Ok(())),
            "live root == last receipt root_after ⇒ the baseline is current"
        );
    }

    /// An empty journal has no last receipt — there is no baseline to date at all,
    /// which [`journal_trace`] answers as [`JournalTrace::NoBaseline`] (grey).
    #[test]
    fn empty_journal_has_no_baseline_to_attribute_against() {
        assert_eq!(detect_baseline(&[], "b3:anything"), None);
    }

    /// The grey is a state of the TRACE, not a colour a renderer chooses: it is
    /// neither red nor summarisable as red, and it says why in words.
    #[test]
    fn no_baseline_is_grey_and_never_red() {
        let trace = JournalTrace::NoBaseline;
        assert!(!trace.is_red(), "grey is not red");
        assert!(trace.cannot_assess(), "grey names itself");
        assert_eq!(trace.red_summary(), None, "nothing to report as a lie");
        assert_eq!(
            trace.grey_summary().as_deref(),
            Some(NO_BASELINE_DETAIL),
            "the grey carries its reason"
        );
        assert_eq!(
            GREY_CANNOT_ASSESS, "grey(cannot-assess)",
            "S3-R6 rules ONE spelling for this reason word, shared with U11/U14 — \
             a local respelling fractures three units"
        );
    }

    /// **The false red, withdrawn** (S3-R8). A stale baseline is grey, not red:
    /// the same mismatch is left by an out-of-writer edit and by any write door
    /// that lands bytes without journaling, so check states the mismatch as
    /// evidence and declines to name a cause.
    ///
    /// U32 removed the splice from that list of doors — the render may no longer
    /// cite it, because a splice now journals its row and a governed corpus never
    /// reaches this state.
    #[test]
    fn a_stale_baseline_is_grey_and_states_evidence_without_naming_a_cause() {
        let trace = JournalTrace::StaleBaseline(BaselineMismatch {
            last_receipt: "r-000001".to_string(),
            recorded_root: "b3:RECORDED".to_string(),
            live_root: "b3:LIVE".to_string(),
        });
        assert!(
            !trace.is_red(),
            "a non-journaling write door produces this state — accusing is the false red"
        );
        assert!(trace.cannot_assess(), "grey names itself");
        assert_eq!(trace.red_summary(), None, "no lie may be claimed from it");
        let grey = trace.grey_summary().expect("the grey carries its reason");
        assert!(
            grey.contains("r-000001") && grey.contains("b3:RECORDED") && grey.contains("b3:LIVE"),
            "the EVIDENCE survives the withdrawn claim: {grey}"
        );
        assert!(
            grey.contains("does not account for"),
            "and it says what it could not do, rather than who did it: {grey}"
        );
        assert!(
            !grey.contains("splice"),
            "U32: a governed splice journals its row, so it is no longer a cause \
             this state can have — citing it teaches the reader a defect that is \
             fixed: {grey}"
        );
    }

    /// A TRACE read against a current baseline is assessed: it can be green (and
    /// `cannot_assess` is false), so the grey cannot leak into that path.
    #[test]
    fn an_assessed_trace_is_never_grey() {
        let trace = JournalTrace::Assessed {
            chain: check_chain(&[row("r-000001", "b3:0", "b3:1")]),
        };
        assert!(!trace.cannot_assess(), "rows were read — this is a verdict");
        assert!(!trace.is_red(), "one honest row, no break");
        assert_eq!(trace.grey_summary(), None);
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
