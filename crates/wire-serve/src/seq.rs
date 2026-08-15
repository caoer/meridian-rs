//! Where a Delta's `seq` comes from.
//!
//! `seq` is a monotone per-workspace batch counter (§4.7), allocated inside the
//! write flock at the instant the frame is assembled. The flock serializes both
//! producers — the write path and the registry's external-change detector —
//! so one lock means one allocator and one chain; allocating outside it can
//! duplicate a `seq` or break the `root_before`/`root_after` chain, forcing
//! subscribers into a full resync.
//!
//! `None` is the in-process caller ([`crate::guard::Origin::InProcess`]): no
//! ring, no subscribers, `seq` stays `0`.

use wire::{DeltaFile, DeltaFrame, Root};

/// Allocates the `seq` for one Delta, called under the workspace write flock.
///
/// An implementor that holds a ring records the committed frame under the same
/// lock ([`Self::committed`]), atomically with respect to the other producer.
/// The roots and files are passed so it can record the frame it is numbering,
/// not merely count.
pub trait SeqSink {
    /// The `seq` this Delta carries. Monotone within one epoch.
    fn allocate(&self, root_before: &Root, root_after: &Root, files: &[DeltaFile]) -> u64;

    /// The committed frame, offered while the caller's write flock is still
    /// held — after every fallible post-commit step, so an offer means the
    /// write path answers this frame to its caller.
    ///
    /// An implementor that holds a ring advances it HERE, not after the
    /// choke-point returns: by the time a detect cycle can take the flock the
    /// tip already carries the moved root, and reconcile syncs silently (the
    /// internal-commit arm — the `delta_sink` pattern). Recording after the
    /// flock drops re-opens the detector double-emission window: a cycle in
    /// the gap re-tells the same change as an actor-absent external frame.
    fn committed(&self, frame: &DeltaFrame);
}

/// The allocation call, with the in-process default folded in (`None` ⇒ `0`).
///
/// `_flock` is never read: a `&fs::WriteLock` exists only while the flock is
/// held, so allocation drifting outside the critical section is a compile
/// error. It witnesses that a lock is held, not that it is this workspace's.
pub(crate) fn allocate(
    sink: Option<&dyn SeqSink>,
    _flock: &fs::WriteLock,
    root_before: &Root,
    root_after: &Root,
    files: &[DeltaFile],
) -> u64 {
    sink.map_or(0, |s| s.allocate(root_before, root_after, files))
}

/// The committed-frame offer, with the in-process default folded in (`None` ⇒
/// nothing to record).
///
/// `_flock` is never read: it witnesses that the caller still holds the write
/// flock, so the offer drifting outside the critical section is a compile
/// error — that drift IS the detector window.
pub(crate) fn committed(sink: Option<&dyn SeqSink>, _flock: &fs::WriteLock, frame: &DeltaFrame) {
    if let Some(s) = sink {
        s.committed(frame);
    }
}
