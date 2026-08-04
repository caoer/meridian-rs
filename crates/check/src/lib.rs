//! The `check` engine (U2.10) — the pure READ verb of the reconciliation loop
//! (d2 §3 check: "what lies? (validity)").
//!
//! # What check is
//! `status = freshness, check = validity` (d2 §3). `check` reads a workspace and
//! answers whether it LIES — without writing a byte, minting a receipt, or
//! spending a cap. Two layers, split by whether the workspace has armed anything:
//!
//! - **Layer 0 — rule-free core** ([`layer0`]). Two pack-free reads:
//!   1. **claims realised** — observe each claim against the current tree and
//!      report the drifted ones (the realise engine's pure detection, run here
//!      read-only — no apply, no cap).
//!   2. **the pin plane** — the CLAIM plane (did the pinned content drift?) and the
//!      RETRIEVAL plane (is the pinned blob durably anchored?).
//!
//! # Why check holds no write-history plane — the LAW (ZT, 2026-08-03)
//! Verbatim: *"Engine does not have memory. It should not have. History is pin to
//! git when we lock. Anything between locks is not history."*
//!
//! `check` therefore answers **at-rest / at-touch truth only: does the world still
//! match the pins.** The receipt journal is deleted, and chain continuity, baseline
//! dating and interval accounting are **never rebuilt as check memory** — they were
//! never this engine's to carry. Archaeology lives in git; attribution lives in
//! transcript JSONL.
//!
//! An out-of-band edit followed by a governed write is not detected here. That is
//! **outside the engine's domain by design**, not a tolerated defect.
//!
//! One consequence is disclosed rather than absorbed: a corpus that once read grey
//! (*"I cannot date your write history"*) now reads green (*"the world still matches
//! the pins"*). The claim is SMALLER, not stronger, so both faces carry
//! [`WRITE_HISTORY_NOT_ASSESSED`] together with the reason — a reader must never
//! carry the old, wider green forward.
//!
//! - **Layer 1 — armed rules read-only** ([`layer1`]). Each armed rule's
//!   `check_change` runs over the change through the page loader — the SAME
//!   surface the door mounts (U4.2), so a refusal here is byte-for-byte the
//!   refusal the door would mint. Its input is the law
//!   [`policy::resolve_armed_law`] already resolved at the write's own path, so
//!   this layer performs no I/O and cannot be handed an armed set the door would
//!   not have honoured.
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
pub mod orphan;

use std::collections::BTreeMap;

use fs::WorkspaceRoot;
use model::Document;

pub use layer0::{
    ClaimFinding, GREY_CANNOT_ASSESS, OrphanedBlob, PinPlane, PinRow, WRITE_HISTORY_NOT_ASSESSED,
    claims_realised, pin_plane,
};
// Layer 1 exports no armed-input type and no fault type of its own: the input is
// `policy::ArmedRule` (sealed to `policy::resolve_armed_law`) and an evaluation
// fault is `policy::ArmedFault::Unevaluable`, the one armed-law fault vocabulary.
// See `layer1`'s module doc for why both belong to `policy` and not here.
pub use layer1::{ArmedFinding, ArmedReport, evaluate};

/// The layer-0 (rule-free core) verdict over a workspace: the claims-realised
/// findings and the PIN PLANE. Green ⇔ every claim converged, every pin holds and
/// every pinned blob is anchored.
///
/// **A green here says nothing about write history**, because this report holds no
/// write-history plane at all — not a grey one, none. The engine keeps no memory by
/// design (ZT 2026-08-03), so that is the law rather than a gap. It is narrower than
/// the green this struct once returned, and the renderers disclose it
/// ([`WRITE_HISTORY_NOT_ASSESSED`]) rather than letting the old meaning ride.
///
/// # The planes fail INDEPENDENTLY (U14)
/// A lock that arrives by clone or pull while its source has moved, and a pinned
/// blob no ref reaches, are facts no journal row ever carried — which is why the pin
/// plane outlived the journal plane. The fence reads this verb, so the verb has to
/// hold every plane it claims or the fence is a false green by construction.
#[derive(Debug)]
pub struct CoreReport {
    /// The claims whose observation drifted (not realised) — empty ⇔ all realised.
    pub drifted_claims: Vec<ClaimFinding>,
    /// The pin plane: red/grey pins, and the pinned blobs no ref reaches.
    pub pins: PinPlane,
    /// The run plane: pre-exec receipts with no completion (G3).
    ///
    /// **REPORTED, never gated on** — it does not move [`is_red`](Self::is_red)
    /// or the exit code, following the fence line's precedent. The reason is the
    /// T1 lesson rather than timidity: receipts written before the completion
    /// marker are unauditable by construction and can NEVER clear, so gating on
    /// them would install a permanent red, and a permanent red is how readers
    /// learn to bump past a plane without reading it. Gating on LIVE orphans
    /// once the historic era has aged out is a separate decision that changes
    /// exit codes, so it needs its own ruling.
    pub orphans: Vec<orphan::OrphanedRun>,
}

impl CoreReport {
    /// The core found a finding: a broken chain read against a current baseline, a
    /// drifted claim, a red pin, or a blob reachable from nothing.
    #[must_use]
    pub fn is_red(&self) -> bool {
        !self.drifted_claims.is_empty() || self.pins.is_red()
    }

    /// The core could not assess something: the journal detectors (no baseline, or
    /// one it cannot show is current — S3-R5, S3-R8), a pin outside sight, or an
    /// object store it could not ask. Grey sits above green and below red in the
    /// worst-of order — a report can be grey and red at once (a drifted pin on an
    /// undatable journal), and red is what it is called then.
    #[must_use]
    pub fn cannot_assess(&self) -> bool {
        self.pins.cannot_assess()
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

/// Run the rule-free core (layer 0) over a workspace: check every claim realised
/// and read the PIN PLANE. Folds nothing and reads no history — asks git about the
/// pinned blobs, and that is all. No write, no cap.
///
/// `docs` is the corpus the caller already built, and `pins` are the colours it
/// read off THAT build through the one pin computer. Both are passed in rather
/// than rebuilt so the two planes describe one corpus: a second build would let
/// them describe two.
///
/// # Errors
/// [`realise::CheckError`] if a claim's observation itself faults (distinct from a
/// clean drift). A caller with no claims passes `&[]`. The pin plane never errors —
/// an unanswerable question there is a REPORTED grey, not a fault, because refusing
/// to run is a worse answer than saying what could not be asked.
pub fn core(
    root: &WorkspaceRoot,
    docs: &BTreeMap<String, Document>,
    claims: &[realise::Claim],
    pins: &[PinRow],
) -> Result<CoreReport, CoreError> {
    let drifted_claims = claims_realised(root, claims).map_err(CoreError::Claim)?;
    Ok(CoreReport {
        drifted_claims,
        pins: pin_plane(root, docs, pins),
        orphans: orphan::orphaned_runs(docs),
    })
}

/// [`core`] over an INTERVAL the caller holds the bytes of — the same verdict, on
/// a snapshot that is not the worktree.
///
/// `docs` and `pins` are the corpus built from that interval's bytes. `root` is
/// used for the OBJECT STORE alone — blob reachability is a property of the
/// repository, not of an interval, and the same store answers for both.
///
/// # Why an interval-bearing entry point still exists (F1)
/// The pre-commit fence's whole job is to speak about what is being committed, and
/// what is being committed is the INDEX. A verdict computer reachable only through
/// a worktree read can be given the right question and still answer about the wrong
/// bytes — a false green with every gate intact. So the interval stays a PARAMETER,
/// and [`core`] is this function with the worktree supplied.
///
/// **There is no longer a separate staged entry point.** `core_of_staged` existed
/// solely to date a staged interval against the RECORD instead of its own last row,
/// because a legitimately staged intermediate state is not the current one and
/// refusing it was a false red. With the journal deleted there is no record and no
/// dating, so the two entry points collapsed into this one — the staged and
/// worktree intervals now differ only in which bytes built the corpus.
///
/// **Claims are not evaluated over a foreign interval.** A claim's observation is a
/// live read against the tree ([`claims_realised`] takes the root), so it cannot be
/// re-pointed at bytes on no disk; the worktree pass owns that plane and this one
/// reports it empty rather than pretending to have asked.
#[must_use]
pub fn core_of(
    root: &WorkspaceRoot,
    docs: &BTreeMap<String, Document>,
    pins: &[PinRow],
) -> CoreReport {
    CoreReport {
        drifted_claims: Vec::new(),
        pins: pin_plane(root, docs, pins),
        orphans: orphan::orphaned_runs(docs),
    }
}

/// Why the layer-0 core could not complete its read. A fault stops the read; it
/// is never a false green.
#[derive(Debug)]
pub enum CoreError {
    /// A claim's observation faulted (page load / I/O) — not a clean drift.
    Claim(realise::CheckError),
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CoreError::Claim(e) => write!(f, "check core claim observation failed: {e}"),
        }
    }
}

impl std::error::Error for CoreError {}

#[cfg(test)]
mod honesty {
    //! **The honesty law, proven structurally rather than asserted.**
    //!
    //! Coherence finding C1 caught the original spec asserting honesty while the
    //! risk map denied it, so it is a thing this crate PROVES: `check` must hold no
    //! member — type or word — that can report a property it no longer observes.
    //! These arms fail the moment someone reintroduces one.

    /// The whole point, in one arm: **no verdict member can say "clean" or
    /// "verified" about a plane `check` does not read.**
    ///
    /// `CoreReport`'s fields ARE the planes it observes. A journal/chain/baseline
    /// field reappearing here means the write-history plane came back without the
    /// evidence to support it — which is the false green the unit exists to close.
    /// The source text is the structural surface, because a field can be added
    /// without any existing arm noticing.
    #[test]
    fn no_core_report_field_claims_a_property_check_no_longer_observes() {
        let src = include_str!("lib.rs");
        let decl = src
            .split_once("pub struct CoreReport {")
            .expect("CoreReport is declared here")
            .1
            .split_once("\n}")
            .expect("and it closes")
            .0;
        for banned in [
            "trace",
            "chain",
            "journal",
            "baseline",
            "accounted",
            "vouches",
            "verified",
            "clean",
        ] {
            assert!(
                !decl.contains(banned),
                "CoreReport gained a `{banned}` member. check does not observe write history — a \
                 field that reports one is memory the engine is ruled not to keep (ZT 2026-08-03)."
            );
        }
    }

    /// **`is_red` and `cannot_assess` may only consult planes that were READ.**
    /// A verdict that consulted a journal field would be reporting an unobserved
    /// property even if the field itself were honestly named.
    #[test]
    fn the_verdict_consults_only_the_planes_that_were_read() {
        let src = include_str!("lib.rs");
        for f in ["pub fn is_red", "pub fn cannot_assess"] {
            let body = src
                .split_once(f)
                .expect("the verdict reader is declared")
                .1
                .split_once("\n    }")
                .expect("and it closes")
                .0;
            assert!(
                !body.contains("trace") && !body.contains("chain"),
                "{f} consults a write-history plane that check does not read"
            );
        }
    }

    /// **The narrowing is DISCLOSED, not silent** (advisor gate 1 §2, mandatory).
    /// A corpus that once read grey now reads green, and the claim behind that green
    /// is smaller. The constant is what stops a reader carrying the old, wider green
    /// forward — deleting it would make the narrowing invisible, which is the whole
    /// failure this condition guards.
    #[test]
    fn the_narrowed_green_carries_its_disclosure() {
        assert_eq!(
            crate::WRITE_HISTORY_NOT_ASSESSED,
            "not-assessed",
            "one spelling, shared by both faces (S3-R6)"
        );
    }

    /// A green `CoreReport` is green on the planes it actually holds — and that is
    /// the ONLY thing its green means. Load-bearing in both directions: a red pin
    /// still reddens, so this is not a verdict that always passes.
    #[test]
    fn a_green_report_means_only_the_planes_it_holds() {
        use crate::{CoreReport, PinPlane};
        let clean = || PinPlane {
            red: Vec::new(),
            grey: Vec::new(),
            orphaned: Vec::new(),
            anchored: 0,
            pending: 0,
            never: 0,
            cannot_ask: None,
            declared: 0,
            out_of_jurisdiction: Vec::new(),
        };
        let green = CoreReport {
            drifted_claims: Vec::new(),
            pins: clean(),
            orphans: Vec::new(),
        };
        assert!(!green.is_red(), "nothing was found");
        assert!(!green.cannot_assess(), "and nothing was unreadable");
        assert_eq!(green.red_summary(), None);
        assert_eq!(green.grey_summary(), None);

        let drifted = CoreReport {
            pins: PinPlane {
                red: vec![crate::PinRow {
                    src_path: "claim.md".to_string(),
                    declared_ref: "source.md#S".to_string(),
                    color: model::selector::Color::Red(model::selector::RedReason::Drifted),
                    label: "red content-drifted".to_string(),
                }],
                ..clean()
            },
            drifted_claims: Vec::new(),
            orphans: Vec::new(),
        };
        assert!(
            drifted.is_red(),
            "the surviving plane still reddens — the green above is earned, not automatic"
        );
    }
}
