//! Immutable generation metadata and fallible, pinned body access (merkle §6.8).
//! The host supplies storage and admission; model owns neither I/O nor a cache.

use crate::{CorpusIndex, Docs, Document, NodeRev, RootedCorpus};
use addr::MountName;
use std::{collections::BTreeMap, fmt::Debug, io, sync::OnceLock};

/// A body remains valid while this lease is alive. Consumers borrow the
/// document; escaping clones require the host's own accounting lease.
pub trait DocumentLease: Debug {
    /// Borrow the exact retained document.
    ///
    /// # Errors
    /// The retained bytes cannot be represented as a parsed document.
    fn document(&self) -> io::Result<&Document>;
}

/// One complete, immutable parsed generation. Cold members remain present in
/// metadata. Revisions, names and pin results must describe the same version
/// for the entire catalog lifetime; a failed load never changes membership.
pub trait DocumentCatalog: Debug {
    /// Every member exactly once, without hydrating bodies.
    fn revisions(&self) -> Box<dyn Iterator<Item = (&str, &NodeRev)> + '_>;
    /// A known member's revision, without hydrating it.
    fn revision(&self, path: &str) -> Option<&NodeRev>;
    /// The complete name/alias index, independent of body residency.
    fn index(&self) -> &CorpusIndex;
    /// Pin one exact member version. `None` means absent membership only.
    ///
    /// # Errors
    /// Required backing, identity, parsing or admission failed. Never retry
    /// against a newer generation or silently omit a known member.
    fn pin(&self, path: &str) -> io::Result<Option<Box<dyn DocumentLease + '_>>>;
}

/// Borrow the existing eager corpus, with a lazily built name index. Merely
/// testing the SQL pin does not walk/clone the corpus or build its index.
#[derive(Debug)]
pub struct EagerCatalog<'a> {
    docs: &'a Docs,
    index: OnceLock<CorpusIndex>,
}
impl<'a> EagerCatalog<'a> {
    #[must_use]
    pub fn new(docs: &'a Docs) -> Self {
        Self {
            docs,
            index: OnceLock::new(),
        }
    }
}
#[derive(Debug)]
struct EagerLease<'a>(&'a Document);
impl DocumentLease for EagerLease<'_> {
    fn document(&self) -> io::Result<&Document> {
        Ok(self.0)
    }
}
impl DocumentCatalog for EagerCatalog<'_> {
    fn revisions(&self) -> Box<dyn Iterator<Item = (&str, &NodeRev)> + '_> {
        Box::new(
            self.docs
                .iter()
                .map(|(path, doc)| (path.as_str(), &doc.root.node_rev)),
        )
    }
    fn revision(&self, path: &str) -> Option<&NodeRev> {
        self.docs.get(path).map(|doc| &doc.root.node_rev)
    }
    fn index(&self) -> &CorpusIndex {
        self.index.get_or_init(|| {
            let mut index = CorpusIndex::new();
            for (path, doc) in self.docs {
                index.insert(path, doc);
            }
            index
        })
    }
    fn pin(&self, path: &str) -> io::Result<Option<Box<dyn DocumentLease + '_>>> {
        Ok(self
            .docs
            .get(path)
            .map(|doc| Box::new(EagerLease(doc)) as Box<dyn DocumentLease>))
    }
}

/// Metadata-only view used by the sole reference resolver. Missing mounted
/// metadata means the root was not loaded, distinct from a missing member.
pub trait NameCorpus: Debug {
    fn contains(&self, root: Option<&MountName>, path: &str) -> bool;
    fn root_index(&self, root: &MountName) -> Option<&CorpusIndex>;
}
impl NameCorpus for RootedCorpus<'_> {
    fn contains(&self, root: Option<&MountName>, path: &str) -> bool {
        match root {
            None => self.ambient.contains_key(path),
            Some(root) => self
                .mounted
                .get(root)
                .is_some_and(|r| r.docs.contains_key(path)),
        }
    }
    fn root_index(&self, root: &MountName) -> Option<&CorpusIndex> {
        self.mounted.get(root).map(|r| &r.index)
    }
}

/// Borrow catalogs for name resolution without owning or pinning their bodies.
#[derive(Debug)]
pub struct RootedCatalog<'a> {
    ambient: &'a dyn DocumentCatalog,
    mounted: BTreeMap<MountName, &'a dyn DocumentCatalog>,
}
impl<'a> RootedCatalog<'a> {
    #[must_use]
    pub fn ambient(catalog: &'a dyn DocumentCatalog) -> Self {
        Self {
            ambient: catalog,
            mounted: BTreeMap::new(),
        }
    }
    #[must_use]
    pub fn with_root(mut self, name: MountName, catalog: &'a dyn DocumentCatalog) -> Self {
        self.mounted.insert(name, catalog);
        self
    }
}
impl NameCorpus for RootedCatalog<'_> {
    fn contains(&self, root: Option<&MountName>, path: &str) -> bool {
        match root {
            None => self.ambient.revision(path).is_some(),
            Some(root) => self
                .mounted
                .get(root)
                .is_some_and(|c| c.revision(path).is_some()),
        }
    }
    fn root_index(&self, root: &MountName) -> Option<&CorpusIndex> {
        self.mounted.get(root).map(|c| c.index())
    }
}
