//! Quiescent heap census of the non-body substrate a residency host's
//! registry holds (merkle-spec §6.4): generations' slot maps, slot facts and
//! name indexes, leaf records, memos, rings and watch states. The host
//! charges the sum as its floor.
//!
//! Owners share allocations — an unchanged slot across generations, the
//! name index while no mover touched it — so every `Arc` an owner may share
//! is offered to one [`Seen`] and counted the first time only. The census's
//! own scratch (the `Seen` set and its clones) is temporary work outside what
//! it reports.

use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

/// The shared allocations a census has already counted, by address. It holds
/// a clone of each until the census ends: an allocation freed mid-census
/// would free its address for a new one, which an address key would then
/// skip. So a census may walk owners one at a time — upgrade, count, drop —
/// and every allocation it met stays put until the `Seen` drops.
#[derive(Default)]
pub struct Seen(HashMap<usize, Arc<dyn Any>>);

impl std::fmt::Debug for Seen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Seen").field(&self.0.len()).finish()
    }
}

impl Seen {
    /// `true` the first time `shared`'s allocation is offered; the census
    /// then holds it until this `Seen` drops.
    pub fn first<T: Any>(&mut self, shared: &Arc<T>) -> bool {
        let address = Arc::as_ptr(shared).cast::<()>() as usize;
        match self.0.entry(address) {
            std::collections::hash_map::Entry::Occupied(_) => false,
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(Arc::clone(shared) as Arc<dyn Any>);
                true
            }
        }
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
