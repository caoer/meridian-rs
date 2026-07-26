//! The `check` engine (U2.10) — the pure READ verb of the reconciliation loop
//! (d2 §3 check: "what lies? (validity)").
//!
//! # What check is
//! `status = freshness, check = validity` (d2 §3). `check` reads a workspace and
//! answers whether it LIES: is the receipt chain intact, are the claims realised,
//! and (armed) do the conventions hold — all without writing a byte, minting a
//! receipt, or spending a cap. Two layers, split by whether a rules pack is armed:
//!
//! - **Layer 0 — convention-free core** ([`layer0`]). Three pack-free reads:
//!   1. **the baseline check** — last-receipt-vs-live: the live tree root must
//!      equal the last receipt's recorded `root_after`. When there is no row, or
//!      the roots differ, the journal cannot date the tree and the TRACE is grey
//!      (`cannot-assess`) — it claims nothing rather than claiming wrongly.
//!   2. **chain recompute** — past that gate, parse the reserved receipt journal
//!      and recompute chain continuity (the U2.1
//!      [`receipt::journal::check_chain`] primitive, mounted here end-to-end); a
//!      spliced/forged row reddens with a row cite.
//!   3. **claims realised** — observe each claim against the current tree and
//!      report the drifted ones (the realise engine's pure detection, run here
//!      read-only — no apply, no cap).
//!
//!   The historical `foreign_edit` RED — "an out-of-writer edit landed with no
//!   receipt" — is withdrawn to the grey above (S3-R8): a governed splice advances
//!   the tree root and writes no journal row, so it leaves evidence identical to
//!   the edit that finding was accusing. The interim reports what it can prove.
//!
//! - **Layer 1 — armed conventions read-only** ([`layer1`]). Each armed
//!   convention's `check_change` runs over the change through the U1.3 loader —
//!   the SAME surface the door mounts (U4.2), so a refusal here is byte-for-byte
//!   the refusal the door would mint. This layer performs no I/O.
//!
//! Session-property integrity is exactly this verb run over a session tree as a
//! workspace (d2 §3).
//!
//! # It never writes
//! Every read here is a pure function of the tree bytes and the pinned evidence.
//! The engine holds no write path, mints no receipt, and takes no cap — the
//! whole surface is `&WorkspaceRoot` / `&Change` in, a report out.

pub mod layer0;
pub mod layer1;

use fs::WorkspaceRoot;

pub use layer0::{
    BaselineMismatch, ClaimFinding, GREY_CANNOT_ASSESS, JournalTrace, NO_BASELINE_DETAIL,
    claims_realised, journal_trace,
};
pub use layer1::{ArmedConvention, ArmedFault, ArmedFinding, ArmedReport, evaluate};

/// The layer-0 (convention-free core) verdict over a workspace: the journal
/// TRACE (the baseline check, then chain continuity) and the claims-realised
/// findings. Green ⇔ the journal dated the tree, the chain is continuous, and
/// every claim converged. With no baseline, or a baseline the journal cannot show
/// is current, the TRACE is grey ([`CoreReport::cannot_assess`]) — never green,
/// because nothing was assessed.
#[derive(Debug)]
pub struct CoreReport {
    /// The journal TRACE: the baseline check, then the chain recompute.
    pub trace: JournalTrace,
    /// The claims whose observation drifted (not realised) — empty ⇔ all realised.
    pub drifted_claims: Vec<ClaimFinding>,
}

impl CoreReport {
    /// The core found a lie: a broken chain read against a current baseline, or a
    /// drifted claim.
    #[must_use]
    pub fn is_red(&self) -> bool {
        self.trace.is_red() || !self.drifted_claims.is_empty()
    }

    /// The core could not assess the journal detectors: no baseline, or one it
    /// cannot show is current, so their silence carries no evidence (S3-R5,
    /// S3-R8). Grey sits above green and below red in the worst-of order — a
    /// report can be grey and red at once (a drifted claim on an undatable
    /// journal), and red is what it is called then.
    #[must_use]
    pub fn cannot_assess(&self) -> bool {
        self.trace.cannot_assess()
    }

    /// The grey render — what could not be assessed and why — or `None` when the
    /// core had the evidence to answer.
    #[must_use]
    pub fn grey_summary(&self) -> Option<String> {
        self.trace.grey_summary()
    }

    /// A red render naming every core finding, or `None` when the core is green.
    /// Composes the journal TRACE render with one line per drifted claim.
    #[must_use]
    pub fn red_summary(&self) -> Option<String> {
        if !self.is_red() {
            return None;
        }
        let mut lines = Vec::new();
        if let Some(trace) = self.trace.red_summary() {
            lines.push(trace);
        }
        for claim in &self.drifted_claims {
            lines.push(format!(
                "claim not realised: {} — {}",
                claim.selector, claim.detail
            ));
        }
        Some(lines.join("\n"))
    }
}

/// Run the convention-free core (layer 0) over a workspace: recompute the journal
/// TRACE (the baseline check, then the chain) and check every claim realised.
/// Reads the reserved journal page and folds the live tree merkle — no write, no
/// cap.
///
/// # Errors
/// [`io::Error`](std::io::Error) if the journal page or the tree snapshot cannot
/// be read; [`realise::CheckError`] if a claim's observation itself faults
/// (distinct from a clean drift). A caller with no claims passes `&[]`.
pub fn core(root: &WorkspaceRoot, claims: &[realise::Claim]) -> Result<CoreReport, CoreError> {
    let trace = journal_trace(root).map_err(CoreError::Io)?;
    let drifted_claims = claims_realised(root, claims).map_err(CoreError::Claim)?;
    Ok(CoreReport {
        trace,
        drifted_claims,
    })
}

/// Why the layer-0 core could not complete its read. A fault stops the read; it
/// is never a false green.
#[derive(Debug)]
pub enum CoreError {
    /// The journal page or the tree snapshot could not be read.
    Io(std::io::Error),
    /// A claim's observation faulted (page load / I/O) — not a clean drift.
    Claim(realise::CheckError),
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CoreError::Io(e) => write!(f, "check core read failed: {e}"),
            CoreError::Claim(e) => write!(f, "check core claim observation failed: {e}"),
        }
    }
}

impl std::error::Error for CoreError {}
