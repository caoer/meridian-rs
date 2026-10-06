//! Controller-backed immutable document generation for fallible consumers.
//! Capture bridges an existing eager generation. Streaming builders, measured
//! metadata/staging floors and daemon publication remain the host's work.

use super::{
    Access,
    controller::{AdmissionError, Controller, Error, Member, Pin},
};
use crate::raw_store::RawStore;
use model::{
    CorpusIndex, Docs, NodeRev,
    catalog::{DocumentCatalog, DocumentLease},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
    time::Duration,
};

/// Pauses between attempts on one member version whose retirement or failed
/// loading wave is completing (merkle-spec §6.4): both end without this
/// caller's help, so a short wait clears them. About 30 ms at most.
const RETRY_PAUSES: [Duration; 6] = [
    Duration::ZERO,
    Duration::from_millis(1),
    Duration::from_millis(2),
    Duration::from_millis(4),
    Duration::from_millis(8),
    Duration::from_millis(16),
];

/// Lease one member version through the controller's admission. `Retiring`
/// and `Settling` retry the same version once per pause in `pauses`;
/// `Busy` is returned at once, because the allowance may be occupied by a pin
/// this very caller holds. All three leave as [`model::catalog::transient`];
/// a load, identity or peak failure is the member's own fault and is neither
/// retried nor transient.
fn admit<T>(
    mut attempt: impl FnMut() -> Result<T, Error>,
    pauses: &[Duration],
    mut pause: impl FnMut(Duration),
) -> io::Result<T> {
    // Control prototype: one attempt, and every refusal is a plain I/O fault.
    let _ = (pauses, &mut pause, AdmissionError::Busy);
    attempt().map_err(io::Error::other)
}

fn pause(wait: Duration) {
    if wait.is_zero() {
        std::thread::yield_now();
    } else {
        std::thread::sleep(wait);
    }
}

#[derive(Debug)]
struct Entry {
    revision: NodeRev,
    member: Member,
    /// The mount names this version's wikilink and embed targets address.
    link_roots: Box<[addr::MountName]>,
    /// The mount names this version's lock addresses name; `None` when it
    /// carries no lock row.
    lock_roots: Option<Box<[addr::MountName]>>,
}

/// Complete generation membership with sheddable bodies and exact backing.
/// Holding the catalog keeps descriptors alive, never every parsed document.
/// `pin` protects a scan without promoting LRU recency; `pin_read` promotes
/// the selected member for interactive serving.
#[derive(Debug)]
pub struct Catalog {
    entries: BTreeMap<String, Entry>,
    index: CorpusIndex,
    load_peak: usize,
}
impl Catalog {
    /// Capture an immutable eager generation, with the host's conservative
    /// per-load peak (source scratch, parsing and flight storage included).
    /// The host accounts for capture staging, index and backing metadata.
    ///
    /// # Errors
    /// Required backing cannot be retained. No partial catalog is published;
    /// the caller's original generation remains untouched.
    pub fn capture(
        docs: &Docs,
        controller: &Controller,
        store: &RawStore,
        load_peak: usize,
    ) -> io::Result<Self> {
        let mut entries = BTreeMap::new();
        let mut index = CorpusIndex::new();
        for (path, doc) in docs {
            let source = store.retain(doc.raw.as_bytes())?;
            entries.insert(
                path.clone(),
                Entry {
                    revision: doc.root.node_rev.clone(),
                    member: controller.retain(Arc::new(source)),
                    link_roots: model::catalog::link_roots(doc).into_iter().collect(),
                    lock_roots: view::walk::lock_fact(doc).map(|roots| roots.into_iter().collect()),
                },
            );
            index.insert(path, doc);
        }
        Ok(Self {
            entries,
            index,
            load_peak,
        })
    }
}
impl Catalog {
    fn lease(&self, path: &str, access: Access) -> io::Result<Option<Box<dyn DocumentLease + '_>>> {
        self.entries
            .get(path)
            .map(|entry| {
                admit(
                    || entry.member.pin(access, self.load_peak),
                    &RETRY_PAUSES,
                    pause,
                )
                .map(|pin| Box::new(pin) as Box<dyn DocumentLease>)
            })
            .transpose()
    }
}
impl DocumentCatalog for Catalog {
    fn revisions(&self) -> Box<dyn Iterator<Item = (&str, &NodeRev)> + '_> {
        Box::new(
            self.entries
                .iter()
                .map(|(path, entry)| (path.as_str(), &entry.revision)),
        )
    }
    fn revision(&self, path: &str) -> Option<&NodeRev> {
        self.entries.get(path).map(|entry| &entry.revision)
    }
    fn index(&self) -> &CorpusIndex {
        &self.index
    }
    fn link_roots(&self) -> BTreeSet<addr::MountName> {
        self.entries
            .values()
            .flat_map(|entry| entry.link_roots.iter().cloned())
            .collect()
    }
    fn pin(&self, path: &str) -> io::Result<Option<Box<dyn DocumentLease + '_>>> {
        self.lease(path, Access::Scan)
    }
    fn pin_read(&self, path: &str) -> io::Result<Option<Box<dyn DocumentLease + '_>>> {
        self.lease(path, Access::Read)
    }
}
impl view::walk::LockFacts for Catalog {
    fn lock_roots(&self) -> BTreeSet<addr::MountName> {
        self.entries
            .values()
            .filter_map(|entry| entry.lock_roots.as_deref())
            .flatten()
            .cloned()
            .collect()
    }
    fn lock_holders(&self) -> Box<dyn Iterator<Item = &str> + '_> {
        Box::new(
            self.entries
                .iter()
                .filter(|(_, entry)| entry.lock_roots.is_some())
                .map(|(path, _)| path.as_str()),
        )
    }
}
impl DocumentLease for Pin {
    fn document(&self) -> io::Result<&model::Document> {
        Pin::document(self).map_err(io::Error::other)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod read_tests;

#[cfg(test)]
mod admission_tests;
