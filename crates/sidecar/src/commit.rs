//! The commit-orchestration seam (frozen §7.3, D4-DELTAS-LIVE): validate →
//! `fs::apply_batch` → change-fact computation (`model::delta`, wire-owned) →
//! `ring.advance` — ONE `wire::Delta` constructed per committed batch, stored
//! in the ring, and the SAME stored object serves `diff` now and `sub` at T5.
//! Byte-identity between replay and live is a property of this single
//! construction site, never of two implementations agreeing (§7.3:
//! "byte-identical … (or would have been)").
//!
//! Emission is UNCONDITIONAL at commit: the Delta exists whether or not any
//! subscriber does — this seam is the one point that knows "a batch committed
//! and the root advanced". `fs` stays a Delta-free byte writer (the F4
//! `apply_batch` signature untouched); `model/src/lib.rs` stays
//! wire-noun-free; `model/src/delta.rs` computes the facts this seam
//! assembles into the envelope (`seq` from the epoch ring, roots from the
//! real fold, `actor`/`now` recorded exactly as given — never invented, §9).
//!
//! The wire `splice` arm wires onto [`commit_batch`] at D4-SPLICE (where the
//! F4 caller obligations — receipt pairing as a production concern, receipt
//! parent-dir existence — land with the production caller). This unit lands
//! the seam and the named replay ≡ live test (`tests/replay_live.rs`,
//! retiring A6 fix-at-freeze flag 4).

use std::path::Path as FsPath;

use wire::{Delta, DeltaFrame, ErrorBody, Root};

use crate::ring::RootRing;

/// One commit's inputs: the model-side batch plus the envelope facts the
/// engine records but never invents (§9). `receipt` carries the receipt
/// file's path and the pre-rendered append (rendered by the `receipt` crate,
/// folded in BEFORE validation so it rides the sealed batch and the single
/// root advance — §6.1, D-C3); its presence must pair with the batch's —
/// `fs` enforces the §6.5 seam contract fail-loud before any byte lands.
#[derive(Debug, Clone)]
pub struct CommitRequest {
    pub content_path: String,
    pub batch: model::SpliceRequest,
    pub receipt: Option<(String, model::ReceiptAppend)>,
    pub actor: Option<String>,
    pub now: Option<String>,
}

/// A commit that did not emit: no byte reached disk, no Delta exists, the
/// ring did not advance. D4-SPLICE maps each variant to its wire frame.
#[derive(Debug)]
pub enum CommitError {
    /// A typed validation refusal (§5.2 failure split) — the batch never
    /// reached `fs`.
    Refused(model::SpliceVerdict),
    /// Ambient-root/domain failure, already in the wire envelope shape.
    Env(Box<ErrorBody>),
    /// The atomic write failed, or the §6.5 seam contract refused
    /// (`InvalidInput` before any byte).
    Io(std::io::Error),
}

/// Commit one batch and emit its Delta (§7.1: one Delta = one batch = one
/// root advance). On success the returned frame is a clone of the object
/// just stored in the ring — the object `diff` will replay byte-identically.
///
/// # Errors
/// [`CommitError`] — validation refusal, environment failure, or I/O; in
/// every error case nothing was emitted and the ring is unchanged (a Delta
/// exists only for a batch that actually committed).
pub fn commit_batch(
    root: &fs::WorkspaceRoot,
    ring: &mut RootRing,
    req: &CommitRequest,
) -> Result<DeltaFrame, CommitError> {
    // Pre-state: the documents the batch validates against + the world root.
    let before_content = fs::load(root, FsPath::new(&req.content_path)).map_err(CommitError::Io)?;
    let before_receipt = match &req.receipt {
        Some((rp, _)) => load_optional(root, rp)?,
        None => None,
    };
    let root_before = ambient(root)?;

    // Validate (§5.1 order) — mints the sealed batch, the only path to fs.
    let sealed = match model::validate_batch(
        &before_content,
        Some(&model::MerkleRoot(root_before.0.clone())),
        &req.batch,
        req.receipt.as_ref().map(|(_, append)| append.clone()),
    ) {
        model::SpliceVerdict::Validated(batch) => batch,
        refused => return Err(CommitError::Refused(refused)),
    };

    // Commit: the two-file atomic write (§6.5). fs enforces the pairing
    // contract fail-loud; a refusal here means no byte landed.
    fs::apply_batch(
        root,
        FsPath::new(&req.content_path),
        req.receipt.as_ref().map(|(rp, _)| FsPath::new(rp.as_str())),
        &sealed,
    )
    .map_err(CommitError::Io)?;

    // Post-state + the advanced root.
    let after_content = fs::load(root, FsPath::new(&req.content_path)).map_err(CommitError::Io)?;
    let after_receipt = match &req.receipt {
        Some((rp, _)) => load_optional(root, rp)?,
        None => None,
    };
    let root_after = ambient(root)?;

    // Change facts (wire-owned delta.rs) → wire projection, worked-frame file
    // order: content file first, then the receipt file (§7.1 E3/E4 print
    // order).
    let mut files = Vec::new();
    if let Some(fd) = model::delta::file_delta(Some(&before_content), Some(&after_content)) {
        files.push(wire_map::project_file_delta(&req.content_path, &fd));
    }
    if let Some((rp, _)) = &req.receipt
        && let Some(fd) = model::delta::file_delta(before_receipt.as_ref(), after_receipt.as_ref())
    {
        files.push(wire_map::project_file_delta(rp, &fd));
    }

    // The ONE construction site: seq from this epoch's ring counter (§7.1
    // late law — per-daemon-epoch, nothing persisted), envelope facts as
    // given. Stored, then the clone returned — replay serves the store.
    let frame = DeltaFrame {
        delta: Delta {
            seq: ring.seq() + 1,
            root_before,
            root_after,
            actor: req.actor.clone(),
            now: req.now.clone(),
            files,
        },
    };
    ring.advance(frame.clone());
    Ok(frame)
}

/// A pre/post receipt-file read where absence is a legal state (the first
/// receipt append creates the file — `fs::read_or_empty` twin at the
/// document grain).
fn load_optional(
    root: &fs::WorkspaceRoot,
    rel: &str,
) -> Result<Option<model::Document>, CommitError> {
    match fs::load(root, FsPath::new(rel)) {
        Ok(doc) => Ok(Some(doc)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(CommitError::Io(e)),
    }
}

fn ambient(root: &fs::WorkspaceRoot) -> Result<Root, CommitError> {
    crate::arms::ambient_root(root).map_err(CommitError::Env)
}
