//! Controller-backed immutable document generation for fallible consumers.
//! A generation is built by the incremental pass over its slots ([`update`];
//! a cold build is that pass over the empty generation) or captured from an
//! existing eager one ([`Catalog::capture`]). Measured metadata/staging floors
//! and daemon publication remain the host's work.

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
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

thread_local! {
    /// Leases this thread took from a [`Catalog`] and has not released.
    static HELD: Arc<AtomicUsize> = Arc::default();
}

/// One leased body, counted against the thread that took it until it ends,
/// wherever it is dropped.
#[derive(Debug)]
struct Lease {
    pin: Pin,
    held: Arc<AtomicUsize>,
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.held.fetch_sub(1, Ordering::Relaxed);
    }
}

impl DocumentLease for Lease {
    fn document(&self) -> io::Result<&model::Document> {
        DocumentLease::document(&self.pin)
    }
}

/// Occupied allowance met by a caller that already holds a lease
/// (merkle-spec §6.4): its own lease is part of what occupies the allowance,
/// so the same request cannot be admitted by retrying.
#[derive(Debug)]
struct SelfBlocked;

impl std::fmt::Display for SelfBlocked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            "residency allowance is occupied while this request holds another lease on it; \
             a retry cannot admit both bodies",
        )
    }
}
impl std::error::Error for SelfBlocked {}

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
/// this very caller holds. All three leave as [`model::catalog::transient`],
/// except a `Busy` met while the caller is `holding` a lease of its own,
/// which no retry cures ([`SelfBlocked`]); a load, identity or peak failure
/// is the member's own fault. Neither is retried nor transient.
fn admit<T>(
    mut attempt: impl FnMut() -> Result<T, Error>,
    holding: bool,
    pauses: &[Duration],
    mut pause: impl FnMut(Duration),
) -> io::Result<T> {
    let mut pauses = pauses.iter();
    loop {
        let refusal = match attempt() {
            Ok(leased) => return Ok(leased),
            // Control prototype: the caller's own lease is not told apart.
            Err(Error::Admission(AdmissionError::Busy)) if holding && false => {
                return Err(io::Error::other(SelfBlocked));
            }
            Err(Error::Admission(AdmissionError::Busy)) => AdmissionError::Busy,
            Err(Error::Admission(
                refusal @ (AdmissionError::Retiring | AdmissionError::Settling),
            )) => match pauses.next() {
                Some(wait) => {
                    pause(*wait);
                    continue;
                }
                None => refusal,
            },
            Err(error) => return Err(io::Error::other(error)),
        };
        return Err(model::catalog::transient(refusal));
    }
}

fn pause(wait: Duration) {
    if wait.is_zero() {
        std::thread::yield_now();
    } else {
        std::thread::sleep(wait);
    }
}

/// One member version in a generation: the facts its consumers read from
/// metadata and the controller member that owns its sheddable body.
#[derive(Debug)]
pub struct Slot {
    revision: NodeRev,
    member: Member,
    /// The mount names this version's wikilink and embed targets address.
    link_roots: Box<[addr::MountName]>,
    /// The mount names this version's lock addresses name; `None` when it
    /// carries no lock row.
    lock_roots: Option<Box<[addr::MountName]>>,
    /// The aliases this version was entered into the name index under, so a
    /// later generation removes its rows without loading it.
    aliases: Box<[String]>,
}

impl Slot {
    /// Publish `doc` as a cold slot: its exact bytes retained as backing, a
    /// controller member over them, and its descriptor facts. The parsed
    /// tree is not kept; the first pin re-reads the backing.
    ///
    /// # Errors
    /// The backing cannot be retained.
    pub fn publish(
        doc: &model::Document,
        controller: &Controller,
        store: &RawStore,
    ) -> io::Result<Self> {
        let source = store.retain(doc.raw.as_bytes())?;
        Ok(Self {
            revision: doc.root.node_rev.clone(),
            member: controller.retain(Arc::new(source)),
            link_roots: model::catalog::link_roots(doc).into_iter().collect(),
            lock_roots: view::walk::lock_fact(doc).map(|roots| roots.into_iter().collect()),
            aliases: model::doc_aliases(doc).into(),
        })
    }
}

/// A generation's slots by workspace-relative path. A clone shares every
/// slot, so generations built one from another hold one controller member
/// per unchanged version.
#[derive(Debug, Clone, Default)]
pub struct Slots(BTreeMap<String, Arc<Slot>>);

impl std::ops::Deref for Slots {
    type Target = BTreeMap<String, Arc<Slot>>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl fs::SlotMap for Slots {
    type Slot = Arc<Slot>;
    fn len(&self) -> usize {
        self.0.len()
    }
    fn contains_key(&self, rel: &str) -> bool {
        self.0.contains_key(rel)
    }
    fn remove(&mut self, rel: &str) -> Option<Self::Slot> {
        self.0.remove(rel)
    }
    fn insert(&mut self, rel: String, slot: Self::Slot) {
        self.0.insert(rel, slot);
    }
    fn indexed_aliases(slot: &Self::Slot) -> Vec<String> {
        slot.aliases.to_vec()
    }
    fn index(&self) -> io::Result<CorpusIndex> {
        Err(io::Error::other(
            "a residency generation carries its own name index; rebuilding one would load \
             every body",
        ))
    }
}

/// The incremental pass ([`fs::update_corpus_with`]) over a residency
/// generation: unmoved members keep their slots, each parsed mover is
/// published ([`Slot::publish`]) before the next is read, and no prior
/// version is loaded. Over the empty generation (an empty, present name
/// index) with every leaf a mover, this is the cold build.
///
/// # Errors
/// What [`fs::update_corpus_with`] refuses, a mover's backing that cannot be
/// retained included. Nothing is returned to publish on error.
pub fn update(
    root: &fs::WorkspaceRoot,
    prior: &fs::CorpusPrior<'_, Slots>,
    moves: &[fs::LeafMove],
    fresh_root: Option<&model::MerkleRoot>,
    controller: &Controller,
    store: &RawStore,
) -> io::Result<fs::CorpusUpdate<Slots>> {
    fs::update_corpus_with(root, prior, moves, fresh_root, |_, doc| {
        Slot::publish(&doc, controller, store).map(Arc::new)
    })
}

/// Complete generation membership with sheddable bodies and exact backing.
/// Holding the catalog keeps descriptors alive, never every parsed document.
/// `pin` protects a scan without promoting LRU recency; `pin_read` promotes
/// the selected member for interactive serving.
#[derive(Debug)]
pub struct Catalog {
    entries: Slots,
    index: Arc<CorpusIndex>,
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
        let mut entries = Slots::default();
        let mut index = CorpusIndex::new();
        for (path, doc) in docs {
            let slot = Slot::publish(doc, controller, store)?;
            entries.0.insert(path.clone(), Arc::new(slot));
            index.insert(path, doc);
        }
        Ok(Self::new(entries, Arc::new(index), load_peak))
    }

    /// The generation `entries` and `index` describe ([`update`]'s product),
    /// with the host's per-load peak.
    #[must_use]
    pub fn new(entries: Slots, index: Arc<CorpusIndex>, load_peak: usize) -> Self {
        Self {
            entries,
            index,
            load_peak,
        }
    }

    /// The generation's slots: the prior of the next incremental pass
    /// ([`update`]).
    #[must_use]
    pub fn slots(&self) -> &Slots {
        &self.entries
    }
}
impl Catalog {
    fn lease(&self, path: &str, access: Access) -> io::Result<Option<Box<dyn DocumentLease + '_>>> {
        self.entries
            .get(path)
            .map(|entry| {
                let held = HELD.with(Arc::clone);
                let pin = admit(
                    || entry.member.pin(access, self.load_peak),
                    held.load(Ordering::Relaxed) > 0,
                    &RETRY_PAUSES,
                    pause,
                )?;
                held.fetch_add(1, Ordering::Relaxed);
                Ok(Box::new(Lease { pin, held }) as Box<dyn DocumentLease>)
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
mod generation_tests;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod read_tests;

#[cfg(test)]
mod admission_tests;
