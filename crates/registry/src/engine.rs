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

/// A generation's bodies (merkle-spec §6.8): an eager document map with its
/// members' link-addressed roots, or the slots of a generation built under a
/// residency host.
#[derive(Debug)]
enum Bodies {
    Eager(model::Docs, RootCounts),
    Resident(Catalog),
}

/// How many members of an eager generation address each mount root through a
/// wikilink or embed target — [`DocumentCatalog::link_roots`] is its keys
/// (merkle-spec §6.8). Counted, not just collected, so a pass that replaces or
/// drops a member can take that member's roots back out.
type RootCounts = BTreeMap<addr::MountName, usize>;

/// Count every member of `docs`: the one whole-corpus walk, at a build.
fn count_roots(docs: &model::Docs) -> RootCounts {
    let mut counts = RootCounts::new();
    for doc in docs.values() {
        add_roots(&mut counts, doc);
    }
    counts
}

/// `prior`'s counts carried to `docs`, the map an incremental pass built from
/// `prior_docs`: an unmoved member is the prior's own document (merkle-spec
/// §6.8), so only a member whose document differs, appeared or went is
/// walked. One ordered pass over both maps, which share their key order.
fn carry_roots(prior_docs: &model::Docs, prior: &RootCounts, docs: &model::Docs) -> RootCounts {
    use std::cmp::Ordering;
    let mut counts = prior.clone();
    let mut was = prior_docs.iter().peekable();
    let mut now = docs.iter().peekable();
    loop {
        match (was.peek(), now.peek()) {
            (None, None) => break,
            (Some(&(_, old)), None) => {
                drop_roots(&mut counts, old);
                was.next();
            }
            (None, Some(&(_, new))) => {
                add_roots(&mut counts, new);
                now.next();
            }
            (Some(&(old_path, old)), Some(&(new_path, new))) => match old_path.cmp(new_path) {
                Ordering::Less => {
                    drop_roots(&mut counts, old);
                    was.next();
                }
                Ordering::Greater => {
                    add_roots(&mut counts, new);
                    now.next();
                }
                Ordering::Equal => {
                    if !Arc::ptr_eq(old, new) {
                        drop_roots(&mut counts, old);
                        add_roots(&mut counts, new);
                    }
                    was.next();
                    now.next();
                }
            },
        }
    }
    counts
}

fn add_roots(counts: &mut RootCounts, doc: &model::Document) {
    for root in model::catalog::link_roots(doc) {
        *counts.entry(root).or_default() += 1;
    }
}

fn drop_roots(counts: &mut RootCounts, doc: &model::Document) {
    for root in model::catalog::link_roots(doc) {
        if let std::collections::btree_map::Entry::Occupied(mut held) = counts.entry(root) {
            *held.get_mut() -= 1;
            if *held.get() == 0 {
                held.remove();
            }
        }
    }
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
    /// A generation that owns its parsed documents, its link-addressed roots
    /// counted over every member.
    #[must_use]
    pub fn eager(
        index: Arc<model::CorpusIndex>,
        docs: model::Docs,
        unserved: BTreeMap<String, String>,
        at_fingerprint: model::MerkleRoot,
        leaves: BTreeMap<PathBuf, [u8; 32]>,
    ) -> Self {
        let roots = count_roots(&docs);
        Self {
            index,
            bodies: Bodies::Eager(docs, roots),
            unserved,
            at_fingerprint,
            leaves,
        }
    }

    /// The generation an incremental pass built from `prior`
    /// ([`fs::update_corpus`]): its roots are `prior`'s, recounted for the
    /// members whose document the pass replaced, added or dropped. A prior
    /// that owns no documents counts every member.
    #[must_use]
    pub fn eager_after(prior: &Self, update: fs::CorpusUpdate) -> Self {
        let roots = match &prior.bodies {
            Bodies::Eager(prior_docs, counts) => carry_roots(prior_docs, counts, &update.docs),
            Bodies::Resident(_) => count_roots(&update.docs),
        };
        Self {
            index: update.index,
            bodies: Bodies::Eager(update.docs, roots),
            unserved: update.unserved,
            at_fingerprint: update.root,
            leaves: update.leaves,
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
            Bodies::Eager(docs, roots) => docs
                .keys()
                .fold(
                    model::allocation::btree_bytes::<String, Arc<model::Document>>(docs.len()),
                    |bytes, path| bytes.saturating_add(path.capacity()),
                )
                .saturating_add(roots.keys().fold(
                    model::allocation::btree_bytes::<addr::MountName, usize>(roots.len()),
                    |bytes, root| bytes.saturating_add(root.as_str().len()),
                )),
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
    /// load-peak rule ([`crate::residency::load_peak`]).
    #[must_use]
    pub fn resident(update: fs::CorpusUpdate<Slots>) -> Self {
        Self {
            bodies: Bodies::Resident(Catalog::new(update.docs, Arc::clone(&update.index))),
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
            Bodies::Eager(docs, _) => Some(docs),
            Bodies::Resident(_) => None,
        }
    }

    /// Take the parsed documents out of a generation that owns them, leaving
    /// it with none: an ownership census frees each part of a generation it
    /// solely owns separately. `None` for a generation built under a
    /// residency host.
    pub fn take_eager_docs(&mut self) -> Option<model::Docs> {
        match &mut self.bodies {
            Bodies::Eager(docs, _) => Some(std::mem::take(docs)),
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
            Bodies::Eager(..) => None,
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
            Bodies::Eager(docs, _) => Box::new(
                docs.iter()
                    .map(|(path, doc)| (path.as_str(), &doc.root.node_rev)),
            ),
            Bodies::Resident(catalog) => catalog.revisions(),
        }
    }
    fn revision(&self, path: &str) -> Option<&model::NodeRev> {
        match &self.bodies {
            Bodies::Eager(docs, _) => docs.get(path).map(|doc| &doc.root.node_rev),
            Bodies::Resident(catalog) => catalog.revision(path),
        }
    }
    fn index(&self) -> &model::CorpusIndex {
        &self.index
    }
    fn link_roots(&self) -> BTreeSet<addr::MountName> {
        match &self.bodies {
            Bodies::Eager(_, roots) => roots.keys().cloned().collect(),
            Bodies::Resident(catalog) => catalog.link_roots(),
        }
    }
    fn pin(&self, path: &str) -> io::Result<Option<Box<dyn DocumentLease + '_>>> {
        match &self.bodies {
            Bodies::Eager(docs, _) => Ok(docs
                .get(path)
                .map(|doc| model::catalog::resident_lease(doc))),
            Bodies::Resident(catalog) => catalog.pin(path),
        }
    }
    fn pin_read(&self, path: &str) -> io::Result<Option<Box<dyn DocumentLease + '_>>> {
        match &self.bodies {
            Bodies::Eager(..) => self.pin(path),
            Bodies::Resident(catalog) => catalog.pin_read(path),
        }
    }
}

/// The engine's lock facts (merkle-spec §6.4): read off the documents of a
/// generation that owns them, off the slot descriptors otherwise.
impl LockFacts for WorkspaceEngine {
    fn lock_roots(&self) -> BTreeSet<addr::MountName> {
        match &self.bodies {
            Bodies::Eager(docs, _) => view::walk::lock_addressed_roots(docs),
            Bodies::Resident(catalog) => catalog.lock_roots(),
        }
    }
    fn lock_holders(&self) -> Box<dyn Iterator<Item = &str> + '_> {
        match &self.bodies {
            Bodies::Eager(docs, _) => Box::new(
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

#[cfg(test)]
mod tests {
    use super::{RootCounts, carry_roots, count_roots};
    use std::sync::Arc;

    fn doc(raw: &str) -> Arc<model::Document> {
        Arc::new(model::build(raw.to_owned(), syntax::parse(raw)))
    }

    /// Carried counts equal a recount of the new generation, whether a member
    /// was kept by reference, replaced, dropped or added — and a root the
    /// pass leaves no member addressing is gone (merkle-spec §6.8).
    #[test]
    fn carried_link_roots_equal_a_recount() {
        let kept = doc("# Kept\n[[a:x]]\n");
        let prior: model::Docs = [
            ("kept.md", Arc::clone(&kept)),
            ("edited.md", doc("# Edited\n[[b:y]] and [[a:z]]\n")),
            ("gone.md", doc("# Gone\n![[c:w]]\n")),
        ]
        .into_iter()
        .map(|(path, doc)| (path.to_owned(), doc))
        .collect();
        let counts = count_roots(&prior);
        let next: model::Docs = [
            ("kept.md", kept),
            ("edited.md", doc("# Edited\nno links now\n")),
            ("born.md", doc("# Born\n[[d:v]] [[a:u]]\n")),
        ]
        .into_iter()
        .map(|(path, doc)| (path.to_owned(), doc))
        .collect();

        let carried = carry_roots(&prior, &counts, &next);
        assert_eq!(carried, count_roots(&next));
        let names = |counts: &RootCounts| -> Vec<(String, usize)> {
            counts
                .iter()
                .map(|(root, n)| (root.as_str().to_owned(), *n))
                .collect()
        };
        assert_eq!(
            names(&carried),
            [("a".to_owned(), 2), ("d".to_owned(), 1)],
            "b and c have no member left"
        );
    }
}
