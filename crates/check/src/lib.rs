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

use std::collections::BTreeMap;

use fs::WorkspaceRoot;
use model::Document;

pub use layer0::{
    BaselineMismatch, ClaimFinding, GREY_CANNOT_ASSESS, JournalTrace, NO_BASELINE_DETAIL,
    OrphanedBlob, PinPlane, PinRow, claims_realised, journal_page, journal_trace, journal_trace_of,
    pin_plane,
};
pub use layer1::{ArmedConvention, ArmedFault, ArmedFinding, ArmedReport, evaluate};

/// The layer-0 (convention-free core) verdict over a workspace: the journal
/// TRACE (the baseline check, then chain continuity), the claims-realised
/// findings, and the PIN PLANE. Green ⇔ the journal dated the tree, the chain is
/// continuous, every claim converged, every pin holds and every pinned blob is
/// anchored. With no baseline, or a baseline the journal cannot show is current,
/// the TRACE is grey ([`CoreReport::cannot_assess`]) — never green, because
/// nothing was assessed.
///
/// # The two planes fail INDEPENDENTLY (U14)
/// Before U14 this report was the journal plane alone, so a green meant *"baseline
/// provable AND nothing the journal plane can see"* — a true statement about the
/// journal plane that stayed **silent about the pin plane**. The silence was the
/// debt: a lock that arrives by clone or pull while its source has moved leaves the
/// journal plane with nothing whatever to report, and a pinned blob no ref reaches
/// is a fact **no journal row will ever carry**. The fence reads this verb's triad,
/// so the verb has to hold both planes or the fence is a false green by
/// construction.
#[derive(Debug)]
pub struct CoreReport {
    /// The journal TRACE: the baseline check, then the chain recompute.
    pub trace: JournalTrace,
    /// The claims whose observation drifted (not realised) — empty ⇔ all realised.
    pub drifted_claims: Vec<ClaimFinding>,
    /// The pin plane: red/grey pins, and the `objects:` blobs no ref reaches.
    pub pins: PinPlane,
}

impl CoreReport {
    /// The core found a finding: a broken chain read against a current baseline, a
    /// drifted claim, a red pin, or a blob reachable from nothing.
    #[must_use]
    pub fn is_red(&self) -> bool {
        self.trace.is_red() || !self.drifted_claims.is_empty() || self.pins.is_red()
    }

    /// The core could not assess something: the journal detectors (no baseline, or
    /// one it cannot show is current — S3-R5, S3-R8), a pin outside sight, or an
    /// object store it could not ask. Grey sits above green and below red in the
    /// worst-of order — a report can be grey and red at once (a drifted pin on an
    /// undatable journal), and red is what it is called then.
    #[must_use]
    pub fn cannot_assess(&self) -> bool {
        self.trace.cannot_assess() || self.pins.cannot_assess()
    }

    /// The grey render — everything that could not be assessed and why — or `None`
    /// when the core had the evidence to answer every question it put.
    ///
    /// One line per unassessable plane, because they are unassessable for
    /// DIFFERENT reasons with different fixes: a journal that cannot date the tree
    /// is not an object store that cannot be reached, and collapsing the two would
    /// teach a reader to look in the wrong place (S3-R43/S3-R50).
    #[must_use]
    pub fn grey_summary(&self) -> Option<String> {
        let mut lines = Vec::new();
        if let Some(trace) = self.trace.grey_summary() {
            lines.push(format!("{GREY_CANNOT_ASSESS}: {trace}"));
        }
        // A grey PIN carries its own word (`unmounted`, `path-unseeable`, …) in
        // its label, so prefixing it with `cannot-assess` would collapse two
        // distinct causes into one tone — S3-R43 read backwards, which cost a
        // round once already. One vocabulary, distinct words.
        for pin in &self.pins.grey {
            lines.push(format!("pin: {}", render_pin(pin)));
        }
        if let Some(detail) = &self.pins.cannot_ask {
            lines.push(format!("{GREY_CANNOT_ASSESS}: {detail}"));
        }
        (!lines.is_empty()).then(|| lines.join("\n"))
    }

    /// A red render naming every core finding, or `None` when the core is green.
    /// Composes the journal TRACE render with one line per drifted claim, per red
    /// pin, and per blob no ref reaches.
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
        for pin in &self.pins.red {
            lines.push(format!("pin: {}", render_pin(pin)));
        }
        for orphan in &self.pins.orphaned {
            lines.push(format!(
                "{}: {} objects.{} ({}) is reachable from no ref, and the file hashes to {} now \
                 — no commit will anchor it",
                orphan.state.word(),
                orphan.src_path,
                orphan.key,
                orphan.blob_sha,
                orphan.live
            ));
        }
        Some(lines.join("\n"))
    }
}

/// One pin row as a line: the page, the ref it declares, and the colour its ONE
/// computer rendered. The reason word is never re-spelled here — [`PinRow::label`]
/// carries `view`'s own render, so this composes and never speaks.
fn render_pin(pin: &PinRow) -> String {
    if pin.declared_ref.is_empty() {
        format!("{} — {}", pin.src_path, pin.label)
    } else {
        format!("{} → {} — {}", pin.src_path, pin.declared_ref, pin.label)
    }
}

/// Run the convention-free core (layer 0) over a workspace: recompute the journal
/// TRACE (the baseline check, then the chain), check every claim realised, and
/// read the PIN PLANE. Reads the reserved journal page, folds the live tree
/// merkle, and asks git about the pinned blobs — no write, no cap.
///
/// `docs` is the corpus the caller already built, and `pins` are the colours it
/// read off THAT build through the one pin computer. Both are passed in rather
/// than rebuilt so the two planes describe one corpus: a second build would let
/// them describe two.
///
/// # Errors
/// [`io::Error`](std::io::Error) if the journal page or the tree snapshot cannot
/// be read; [`realise::CheckError`] if a claim's observation itself faults
/// (distinct from a clean drift). A caller with no claims passes `&[]`. The pin
/// plane never errors — an unanswerable question there is a REPORTED grey, not a
/// fault, because refusing to run is a worse answer than saying what could not be
/// asked.
pub fn core(
    root: &WorkspaceRoot,
    docs: &BTreeMap<String, Document>,
    claims: &[realise::Claim],
    pins: &[PinRow],
) -> Result<CoreReport, CoreError> {
    let trace = journal_trace(root).map_err(CoreError::Io)?;
    let drifted_claims = claims_realised(root, claims).map_err(CoreError::Claim)?;
    Ok(CoreReport {
        trace,
        drifted_claims,
        pins: pin_plane(root, docs, pins),
    })
}

/// [`core`] over an INTERVAL the caller holds the bytes of — the same verdict, on
/// a snapshot that is not the worktree.
///
/// `journal_page` and `live_root` are that interval's own journal bytes and its
/// own folded tree root ([`fs::overlay_snapshot`]); `docs` and `pins` are the
/// corpus built from the same bytes. `root` is used for the OBJECT STORE alone —
/// blob reachability is a property of the repository, not of an interval, and the
/// same store answers for both.
///
/// # Why an interval-bearing entry point exists at all (F1)
/// The pre-commit fence's whole job is to speak about what is being committed,
/// and what is being committed is the INDEX. A verdict computer reachable only
/// through a worktree read can be given the right question and still answer about
/// the wrong bytes — a false green with every gate intact. So the interval is a
/// PARAMETER here, and [`core`] is this function with the worktree supplied.
///
/// **Claims are not evaluated over a foreign interval.** A claim's observation is
/// a live read against the tree ([`claims_realised`] takes the root), so it cannot
/// be re-pointed at bytes on no disk; the worktree pass owns that plane and this
/// one reports it empty rather than pretending to have asked.
#[must_use]
pub fn core_of(
    root: &WorkspaceRoot,
    journal_page: &str,
    live_root: &str,
    docs: &BTreeMap<String, Document>,
    pins: &[PinRow],
) -> CoreReport {
    CoreReport {
        trace: journal_trace_of(journal_page, live_root),
        drifted_claims: Vec::new(),
        pins: pin_plane(root, docs, pins),
    }
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
