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

use wire::{DeltaFile, Root};

/// Allocates the `seq` for one Delta, called under the workspace write flock.
///
/// An implementor that also advances its ring does both under the caller's
/// lock, atomically with respect to the other producer. The roots and files are
/// passed so it can record the frame it is numbering, not merely count.
pub trait SeqSink {
    /// The `seq` this Delta carries. Monotone within one epoch.
    fn allocate(&self, root_before: &Root, root_after: &Root, files: &[DeltaFile]) -> u64;
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
