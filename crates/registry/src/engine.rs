//! Resident per-workspace query engine (U1).
//!
//! Warm [`WorkspaceEngine`] per workspace: parsed corpus + content hash it was
//! built at. [`Registry::warm_or_build`](crate::Registry::warm_or_build)
//! rebuilds only when the hash changes; else reuses and parses nothing.
//!
//! Disposable projection of disk. The name index stays RAM-only; immutable
//! documents may restore through the validated §6.9 cache after a cold gap.
//! Reuse key is the corpus CONTENT hash (R5), not workspace-identity Merkle.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use model::catalog::{DocumentCatalog, DocumentLease};
use view::walk::LockFacts;

use crate::residency::catalog::{Catalog, Slots};

/// A generation's bodies (merkle-spec §6.8): an eager document map, or the
/// slots of a generation built under a residency host.
#[derive(Debug)]
enum Bodies {
    Eager(model::Docs),
    Resident(Catalog),
}

/// The warm per-workspace engine state: the parsed corpus that `query` reads
/// (U2), plus the corpus content hash it was built at (the reuse key).
///
/// Model state — derived, disposable (decision 0002; `model` law 2). Never
/// persisted as a live world: a restart verifies disk truth and reconciles
/// any reusable document objects through `fs::update_corpus`.
#[derive(Debug)]
pub struct WorkspaceEngine {
    /// The vault name/alias index the `query` read ops borrow (U2). Shared
    /// with the prior engine while no rebuild mover changed its rows
    /// (merkle-spec §6.8).
    pub index: Arc<model::CorpusIndex>,
    /// The documents, keyed by workspace-relative path. Readers go through
    /// this engine's [`DocumentCatalog`] and [`view::walk::LockFacts`].
    bodies: Bodies,
    /// Members in the hash domain that serve no spans/nodes (per-file UTF-8
    /// degradation, node-rev-merkle-spec §3): workspace-relative path → the
    /// condition. Their bytes are under `at_fingerprint` — integrity coverage
    /// and span service are independent properties — and a read addressed to
    /// one mints the per-file `invalid_utf8` naming it.
    pub unserved: BTreeMap<String, String>,
    /// The corpus content hash the index + docs were built at — the reuse key.
    pub at_fingerprint: model::MerkleRoot,
    /// The §12.2 leaf set `at_fingerprint` folds — member → leaf digest at
    /// build time. The incremental pass's delta baseline
    /// ([`fs::update_corpus`]): the next rebuild re-parses exactly the
    /// members whose leaf moved against this record.
    pub leaves: BTreeMap<PathBuf, [u8; 32]>,
}

impl WorkspaceEngine {
    /// A generation that owns its parsed documents.
    #[must_use]
    pub fn eager(
        index: Arc<model::CorpusIndex>,
        docs: model::Docs,
        unserved: BTreeMap<String, String>,
        at_fingerprint: model::MerkleRoot,
        leaves: BTreeMap<PathBuf, [u8; 32]>,
    ) -> Self {
        Self {
            index,
            bodies: Bodies::Eager(docs),
            unserved,
            at_fingerprint,
            leaves,
        }
    }

    /// Requested heap this generation's metadata owns, each shared
    /// allocation counted once through `seen`: the name index, the bodies'
    /// map (a resident generation's slots and their facts; an eager one's map
    /// alone — its documents are its bodies, not metadata), the unserved
    /// and leaf records and the fingerprint. The inline struct is its
    /// owner's.
    #[must_use]
    pub fn heap_bytes(&self, seen: &mut crate::residency::census::Seen) -> usize {
        let Self {
            index,
            bodies,
            unserved,
            at_fingerprint,
            leaves,
        } = self;
        let bodies = match bodies {
            Bodies::Eager(docs) => docs.keys().fold(
                model::allocation::btree_bytes::<String, Arc<model::Document>>(docs.len()),
                |bytes, path| bytes.saturating_add(path.capacity()),
            ),
            Bodies::Resident(catalog) => catalog.heap_bytes(seen),
        };
        let unserved = unserved.iter().fold(
            model::allocation::btree_bytes::<String, String>(unserved.len()),
            |bytes, (path, condition)| {
                bytes
                    .saturating_add(path.capacity())
                    .saturating_add(condition.capacity())
            },
        );
        let leaves = leaves.keys().fold(
            model::allocation::btree_bytes::<PathBuf, [u8; 32]>(leaves.len()),
            |bytes, path| bytes.saturating_add(path.capacity()),
        );
        crate::residency::census::index_bytes(index, seen)
            .saturating_add(bodies)
            .saturating_add(unserved)
            .saturating_add(leaves)
            .saturating_add(at_fingerprint.0.capacity())
    }

    /// The generation a residency pass built
    /// ([`crate::residency::catalog::update`]), its bodies leased under the
    /// host's per-load peak.
    #[must_use]
    pub fn resident(update: fs::CorpusUpdate<Slots>, load_peak: usize) -> Self {
        Self {
            bodies: Bodies::Resident(Catalog::new(
                update.docs,
                Arc::clone(&update.index),
                load_peak,
            )),
            index: update.index,
            unserved: update.unserved,
            at_fingerprint: update.root,
            leaves: update.leaves,
        }
    }

    /// The parsed documents of a generation that owns them; `None` for one
    /// built under a residency host, whose bodies are leased.
    #[must_use]
    pub fn eager_docs(&self) -> Option<&model::Docs> {
        match &self.bodies {
            Bodies::Eager(docs) => Some(docs),
            Bodies::Resident(_) => None,
        }
    }

    /// Take the parsed documents out of a generation that owns them, leaving
    /// it with none: an ownership census frees each part of a generation it
    /// solely owns separately. `None` for a generation built under a
    /// residency host.
    pub fn take_eager_docs(&mut self) -> Option<model::Docs> {
        match &mut self.bodies {
            Bodies::Eager(docs) => Some(std::mem::take(docs)),
            Bodies::Resident(_) => None,
        }
    }

    /// This engine as the prior of the next incremental pass
    /// ([`fs::update_corpus`]) — its name index included, so the pass can
    /// share it. `None` for a generation built under a residency host.
    #[must_use]
    pub fn as_prior(&self) -> Option<fs::CorpusPrior<'_>> {
        Some(fs::CorpusPrior {
            docs: self.eager_docs()?,
            unserved: &self.unserved,
            leaves: &self.leaves,
            index: Some(&self.index),
        })
    }

    /// This engine as the prior of the next residency pass
    /// ([`crate::residency::catalog::update`]). `None` for a generation that
    /// owns its documents.
    #[must_use]
    pub fn resident_prior(&self) -> Option<fs::CorpusPrior<'_, Slots>> {
        match &self.bodies {
            Bodies::Eager(_) => None,
            Bodies::Resident(catalog) => Some(fs::CorpusPrior {
                docs: catalog.slots(),
                unserved: &self.unserved,
                leaves: &self.leaves,
                index: Some(&self.index),
            }),
        }
    }
}

/// A §6.9 restored snapshot as an incremental pass's prior: no name index,
/// so the pass builds one over the final docs (merkle-spec §6.9).
#[must_use]
pub(crate) fn restored_prior(restored: &parse_cache::snapshot::Restored) -> fs::CorpusPrior<'_> {
    fs::CorpusPrior {
        docs: &restored.docs,
        unserved: &restored.unserved,
        leaves: &restored.leaves,
        index: None,
    }
}

/// The engine is its generation's catalog (merkle-spec §6.8): membership,
/// revisions, the name index it already holds, and a lease per body. A
/// generation that owns its documents lends one and cannot fault; one built
/// under a residency host leases through its controller.
impl DocumentCatalog for WorkspaceEngine {
    fn revisions(&self) -> Box<dyn Iterator<Item = (&str, &model::NodeRev)> + '_> {
        match &self.bodies {
            Bodies::Eager(docs) => Box::new(
                docs.iter()
                    .map(|(path, doc)| (path.as_str(), &doc.root.node_rev)),
            ),
            Bodies::Resident(catalog) => catalog.revisions(),
        }
    }
    fn revision(&self, path: &str) -> Option<&model::NodeRev> {
        match &self.bodies {
            Bodies::Eager(docs) => docs.get(path).map(|doc| &doc.root.node_rev),
            Bodies::Resident(catalog) => catalog.revision(path),
        }
    }
    fn index(&self) -> &model::CorpusIndex {
        &self.index
    }
    fn link_roots(&self) -> BTreeSet<addr::MountName> {
        match &self.bodies {
            Bodies::Eager(docs) => docs
                .values()
                .flat_map(|doc| model::catalog::link_roots(doc))
                .collect(),
            Bodies::Resident(catalog) => catalog.link_roots(),
        }
    }
    fn pin(&self, path: &str) -> io::Result<Option<Box<dyn DocumentLease + '_>>> {
        match &self.bodies {
            Bodies::Eager(docs) => Ok(docs
                .get(path)
                .map(|doc| model::catalog::resident_lease(doc))),
            Bodies::Resident(catalog) => catalog.pin(path),
        }
    }
    fn pin_read(&self, path: &str) -> io::Result<Option<Box<dyn DocumentLease + '_>>> {
        match &self.bodies {
            Bodies::Eager(_) => self.pin(path),
            Bodies::Resident(catalog) => catalog.pin_read(path),
        }
    }
}

/// The engine's lock facts (merkle-spec §6.4): read off the documents of a
/// generation that owns them, off the slot descriptors otherwise.
impl LockFacts for WorkspaceEngine {
    fn lock_roots(&self) -> BTreeSet<addr::MountName> {
        match &self.bodies {
            Bodies::Eager(docs) => view::walk::lock_addressed_roots(docs),
            Bodies::Resident(catalog) => catalog.lock_roots(),
        }
    }
    fn lock_holders(&self) -> Box<dyn Iterator<Item = &str> + '_> {
        match &self.bodies {
            Bodies::Eager(docs) => Box::new(
                docs.iter()
                    .filter(|(_, doc)| view::walk::lock_fact(doc).is_some())
                    .map(|(path, _)| path.as_str()),
            ),
            Bodies::Resident(catalog) => catalog.lock_holders(),
        }
    }
}

/// The resident engine IS a mounted root's served projection when a pin or
/// link plane names that root (`wire_serve::mount_corpus`): the daemon hands
/// its own warm corpus to the colour computer instead of rebuilding the root
/// from disk per call.
impl wire_serve::mount_corpus::ServedCorpus for WorkspaceEngine {
    fn catalog(&self) -> &dyn DocumentCatalog {
        self
    }

    fn docs(&self) -> Option<&model::Docs> {
        self.eager_docs()
    }

    fn unserved(&self) -> &BTreeMap<String, String> {
        &self.unserved
    }
}

/// Whether `warm_or_build` reused or rebuilt. `Reused` proves zero parses
/// (`build_corpus` is rebuild-only). In-process evidence; not on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WarmOutcome {
    /// Content hash unchanged; warm engine reused, nothing parsed.
    Reused,
    /// Hash changed or cold; rebuilt. `docs` = documents parsed.
    Built { docs: usize },
}
