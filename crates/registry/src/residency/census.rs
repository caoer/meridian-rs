//! Quiescent heap census of the non-body substrate a residency host's
//! registry holds (merkle-spec §6.4): generations' slot maps, slot facts and
//! name indexes, leaf records, memos, rings and watch states. The host
//! charges the sum as its floor.
//!
//! Owners share allocations — an unchanged slot across generations, the
//! name index while no mover touched it — so every `Arc` an owner may share
//! is offered to one [`Seen`] and counted the first time only. The census's
//! own scratch (the `Seen` set) is temporary work outside what it reports.

use std::collections::HashSet;
use std::sync::Arc;

/// The shared allocations a census has already counted, by address.
#[derive(Debug, Default)]
pub struct Seen(HashSet<usize>);

impl Seen {
    /// `true` the first time `shared`'s allocation is offered.
    pub fn first<T: ?Sized>(&mut self, shared: &Arc<T>) -> bool {
        self.0.insert(Arc::as_ptr(shared).cast::<()>() as usize)
    }
}

/// An `Arc<CorpusIndex>` counted once: its allocation and the index's heap.
#[must_use]
pub fn index_bytes(index: &Arc<model::CorpusIndex>, seen: &mut Seen) -> usize {
    if seen.first(index) {
        model::allocation::arc_bytes::<model::CorpusIndex>().saturating_add(index.heap_bytes())
    } else {
        0
    }
}
