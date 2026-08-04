//! U20b — **where a Delta's `seq` comes from.**
//!
//! `seq` is a monotone per-workspace batch counter (§4.7). Before this seam the
//! write path took it as a `u64` PARAMETER, read by the caller BEFORE the write
//! flock was acquired: the sidecar passed `epoch.seq()` and advanced its ring
//! afterwards, and the resident daemon passed the literal `0` because it had no
//! ring at all.
//!
//! That is safe with exactly one producer and unsafe with two. The interval
//! between "read the counter" and "advance the ring" spans the whole write, and
//! a second producer — the registry's external-change detector — can advance
//! inside it. The result is a duplicated `seq` or a `root_before` that does not
//! meet the previous `root_after`: a chain break whose only client-side repair
//! is the full resync a subscription exists to avoid.
//!
//! # The rule
//! **`seq` is allocated INSIDE the write flock, at the instant the frame is
//! assembled.** The flock is the workspace's serialization point for both
//! producers — the write path holds it across its critical section, and the
//! registry's detector reconciles under the same lock. One lock, one allocator,
//! one chain.
//!
//! # Why a sink rather than a number
//! A `u64` can only be read early; a sink can be CALLED late. Passing the
//! capability instead of the value is what moves allocation to the moment the
//! frame exists — the type makes the old mistake unwriteable rather than
//! forbidden by comment.
//!
//! `None` is the in-process caller (`mrd`, the run plane, tests —
//! [`crate::guard::Origin::InProcess`]): no ring, no subscribers, `seq` stays
//! `0` and behaviour is exactly what it was.

use wire::{DeltaFile, Root};

/// Allocates the `seq` for one Delta, called under the workspace write flock.
///
/// Implementors own a ring: the sidecar's per-epoch [`crate::ring::RootRing`]
/// and the registry's per-workspace ring. The call is the ONE moment at which a
/// frame's number is decided, so an implementor that also advances its ring does
/// both under the caller's lock, atomically with respect to the other producer.
///
/// The roots and files are passed because an implementor may want to record the
/// frame it is numbering, not merely count. They are the frame's own facts,
/// already computed at the call site.
pub trait SeqSink {
    /// The `seq` this Delta carries. Monotone within one epoch.
    fn allocate(&self, root_before: &Root, root_after: &Root, files: &[DeltaFile]) -> u64;
}

/// The allocation call, with the in-process default folded in.
///
/// `None` ⇒ `0`: an in-process write emits a frame nobody is numbering, exactly
/// as it did before this seam existed.
///
/// # The flock witness
/// `_flock` is never read. It is the type-level statement of the rule at the top
/// of this module — **allocation happens INSIDE the write flock** — and it is
/// here because a correct allocator called at the wrong moment satisfies every
/// other check and still collides. The original defect was a LOCK-SCOPE error,
/// not a counter error: a high-water mark is worthless if the call drifts back
/// outside the critical section, and nothing about the counter can detect that.
///
/// A `&fs::WriteLock` exists only where one was acquired and not yet dropped, so
/// a caller that has left the flock has nothing to pass. The drift is a compile
/// error rather than a test that someone must remember to keep pointed at the
/// right line.
///
/// It witnesses that A lock is held, not that it is THIS workspace's — the guard
/// carries its fd, not its root. That residual is stated, not hidden.
pub(crate) fn allocate(
    sink: Option<&dyn SeqSink>,
    _flock: &fs::WriteLock,
    root_before: &Root,
    root_after: &Root,
    files: &[DeltaFile],
) -> u64 {
    sink.map_or(0, |s| s.allocate(root_before, root_after, files))
}
