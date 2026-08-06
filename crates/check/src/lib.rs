//! The `check` engine — the pure READ verb of the reconciliation loop:
//! `status = freshness, check = validity`.
//!
//! `check` reads a workspace and answers whether it LIES — without writing a byte,
//! minting a receipt, or spending a cap. Two layers:
//!
//! - **Layer 0 — rule-free core** ([`layer0`]): claims realised (observe each
//!   claim against the current tree, report the drifted ones) and the pin plane
//!   (did the pinned content drift? is the pinned blob durably anchored?).
//! - **Layer 1 — armed rules read-only** ([`layer1`]): each armed rule's
//!   `check_change` runs through the SAME surface the door mounts, so a refusal
//!   here is byte-for-byte the refusal the door would mint.
//!
//! `check` holds no write-history plane by law: the engine keeps no memory —
//! history is pinned to git at lock. It answers at-rest truth only: does the world
//! still match the pins. The narrowed green is disclosed, not silent: both faces
//! carry [`WRITE_HISTORY_NOT_ASSESSED`] with the reason.
//!
//! Every read is a pure function of the tree bytes and the pinned evidence:
//! `&WorkspaceRoot` / `&Change` in, a report out.

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
// Layer 1 exports no armed-input or fault type of its own: the input is
// `policy::ArmedRule` and a fault is `policy::ArmedFault::Unevaluable`.
pub use layer1::{ArmedFinding, ArmedReport, evaluate};

/// The layer-0 (rule-free core) verdict over a workspace: the claims-realised
/// findings and the PIN PLANE. Green ⇔ every claim converged, every pin holds and
/// every pinned blob is anchored.
///
/// A green here says nothing about write history — this report holds no
/// write-history plane at all; the renderers disclose it
/// ([`WRITE_HISTORY_NOT_ASSESSED`]) rather than letting the old meaning ride.
///
/// The planes fail independently: a lock arriving by clone or pull while its
/// source moved, and a pinned blob no ref reaches, are facts no journal row
/// ever carried.
#[derive(Debug)]
pub struct CoreReport {
    /// The claims whose observation drifted (not realised) — empty ⇔ all realised.
    pub drifted_claims: Vec<ClaimFinding>,
    /// The pin plane: red/grey pins, and the pinned blobs no ref reaches.
    pub pins: PinPlane,
    /// The run plane: pre-exec receipts with no completion (G3).
    ///
    /// REPORTED, never gated on — it does not move [`is_red`](Self::is_red) or
    /// the exit code: such receipts are unauditable by construction and can
    /// never clear, so gating would install a permanent red. Gating on LIVE
    /// orphans is a separate decision that needs its own ruling.
    pub orphans: Vec<orphan::OrphanedRun>,
}

impl CoreReport {
    /// The core found a finding: a broken chain read against a current baseline, a
    /// drifted claim, a red pin, or a blob reachable from nothing.
    #[must_use]
    pub fn is_red(&self) -> bool {
        !self.drifted_claims.is_empty() || self.pins.is_red()
    }

    /// The core could not assess something: a pin outside sight, or an object
    /// store it could not ask. Grey sits above green and below red in the
    /// worst-of order — a report can be grey and red at once, and red is what
    /// it is called then.
    #[must_use]
    pub fn cannot_assess(&self) -> bool {
        self.pins.cannot_assess()
    }

    /// The grey render — everything that could not be assessed and why — or `None`
    /// when the core had the evidence to answer every question it put.
    ///
    /// One line per unassessable plane: they are unassessable for different
    /// reasons with different fixes.
    #[must_use]
    pub fn grey_summary(&self) -> Option<String> {
        let mut lines = Vec::new();
        // A grey PIN carries its own reason word in its label; prefixing it
        // with `cannot-assess` would collapse two distinct causes into one.
        for pin in &self.pins.grey {
            lines.push(format!("pin: {}", render_pin(pin)));
        }
        if let Some(detail) = &self.pins.cannot_ask {
            lines.push(format!("{GREY_CANNOT_ASSESS}: {detail}"));
        }
        (!lines.is_empty()).then(|| lines.join("\n"))
    }

    /// A red render naming every core finding, or `None` when the core is
    /// green — one line per drifted claim, red pin, and orphaned blob.
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
/// repository, not of an interval.
///
/// The interval stays a parameter because the pre-commit fence speaks about the
/// INDEX: a verdict computer reachable only through a worktree read could be
/// asked the right question and still answer about the wrong bytes. [`core`] is
/// this function with the worktree supplied.
///
/// Claims are not evaluated over a foreign interval: a claim's observation is a
/// live read against the tree, so that plane is reported empty rather than
/// pretending to have asked.
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
    //! The honesty law, proven structurally: `check` must hold no member —
    //! type or word — that can report a property it no longer observes.

    /// No verdict member can say "clean" or "verified" about a plane `check`
    /// does not read. `CoreReport`'s fields ARE the planes it observes; the
    /// source text is the structural surface because a field can be added
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

    /// `is_red` and `cannot_assess` may only consult planes that were READ.
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

    /// The narrowing is DISCLOSED, not silent: the constant is what stops a
    /// reader carrying the old, wider green forward.
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
