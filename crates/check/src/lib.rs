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
//!   1. **chain recompute** — parse the reserved receipt journal and recompute
//!      chain continuity (the U2.1 [`receipt::journal::check_chain`] primitive,
//!      mounted here end-to-end); a spliced/forged row reddens with a row cite.
//!   2. **claims realised** — observe each claim against the current tree and
//!      report the drifted ones (the realise engine's pure detection, run here
//!      read-only — no apply, no cap).
//!   3. **journal TRACE (`foreign_edit`)** — last-receipt-vs-live: the live tree
//!      root must continue the last receipt's recorded `root_after`; when it does
//!      not, an out-of-writer edit landed with no receipt, and check reddens
//!      `foreign_edit` (a finding, never a door refusal — refusal-amendment row).
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

pub use layer0::{ClaimFinding, ForeignEdit, JournalTrace, claims_realised, journal_trace};
pub use layer1::{ArmedConvention, ArmedFault, ArmedFinding, ArmedReport, evaluate};

/// The layer-0 (convention-free core) verdict over a workspace: the journal
/// TRACE (chain continuity + the `foreign_edit` detector) and the claims-realised
/// findings. Green ⇔ the chain is continuous, no foreign edit, and every claim
/// converged.
#[derive(Debug)]
pub struct CoreReport {
    /// The journal TRACE: chain recompute + the `foreign_edit` detector.
    pub trace: JournalTrace,
    /// The claims whose observation drifted (not realised) — empty ⇔ all realised.
    pub drifted_claims: Vec<ClaimFinding>,
}

impl CoreReport {
    /// The core found a lie: a broken chain, a foreign edit, or a drifted claim.
    #[must_use]
    pub fn is_red(&self) -> bool {
        self.trace.is_red() || !self.drifted_claims.is_empty()
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
/// TRACE (chain + `foreign_edit`) and check every claim realised. Reads the
/// reserved journal page and folds the live tree merkle — no write, no cap.
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
