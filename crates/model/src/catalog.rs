//! Immutable generation metadata and fallible, pinned body access (merkle §6.8).
//! The host supplies storage and admission; model owns neither I/O nor a cache.

use crate::{
    AmbientDisk, CorpusIndex, Docs, Document, HashDomain, Node, NodeKind, NodeRev, RootedCorpus,
};
use addr::MountName;
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    fmt::Debug,
    io,
    sync::OnceLock,
};

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
    /// Every mount root the members' wikilink and embed targets address: the
    /// union of each member's [`link_roots`], a descriptor fact fixed when
    /// that member was parsed. Answering pins no body, so a cold or
    /// unreadable member still contributes its roots.
    fn link_roots(&self) -> BTreeSet<MountName>;
    /// Pin one exact member version for a scan. `None` means absent membership
    /// only; pinning need not promote the host's read recency.
    ///
    /// # Errors
    /// Required backing, identity, parsing or admission failed. Never retry
    /// against a newer generation or silently omit a known member.
    fn pin(&self, path: &str) -> io::Result<Option<Box<dyn DocumentLease + '_>>>;

    /// Pin one exact member for an interactive read. A host with read-recency
    /// accounting overrides this to promote the selected member; eager
    /// adapters can use the default borrowing pin.
    ///
    /// # Errors
    /// As [`Self::pin`]; this never selects a newer generation.
    fn pin_read(&self, path: &str) -> io::Result<Option<Box<dyn DocumentLease + '_>>> {
        self.pin(path)
    }
}

/// A known member whose selected version could not be served.
#[derive(Debug)]
pub struct MemberFault {
    /// The member, as the catalog spells it.
    pub path: String,
    /// Why its retained version is unavailable.
    pub cause: io::Error,
}
impl std::fmt::Display for MemberFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "cannot read retained member {}: {}",
            self.path, self.cause
        )
    }
}
impl std::error::Error for MemberFault {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

/// Compute over one member under a scan pin; the lease ends before this
/// returns, so a caller streaming a corpus holds one body at a time.
/// `Ok(None)` means `path` is not a member, and nothing else.
///
/// # Errors
/// A known member has no lease, cannot be pinned or borrowed, or parses to a
/// revision other than its descriptor's.
pub fn scan<T>(
    catalog: &dyn DocumentCatalog,
    path: &str,
    read: impl FnOnce(&Document) -> T,
) -> Result<Option<T>, MemberFault> {
    let Some(revision) = catalog.revision(path) else {
        return Ok(None);
    };
    let fault = |cause| MemberFault {
        path: path.to_owned(),
        cause,
    };
    let lease = catalog
        .pin(path)
        .map_err(fault)?
        .ok_or_else(|| fault(io::Error::other("known member has no body lease")))?;
    let doc = lease.document().map_err(fault)?;
    if doc.root.node_rev != *revision {
        return Err(fault(io::Error::new(
            io::ErrorKind::InvalidData,
            "body does not match the selected revision",
        )));
    }
    Ok(Some(read(doc)))
}

/// Every mount root one document's wikilink and embed targets address. The
/// root name is read from [`addr::Addr::parse`] of each target, the grammar
/// the resolver peels; a target outside the grammar contributes no root.
#[must_use]
pub fn link_roots(doc: &Document) -> BTreeSet<MountName> {
    fn collect(node: &Node, roots: &mut BTreeSet<MountName>) {
        if let NodeKind::Wikilink { target, .. } | NodeKind::Embed { target, .. } = &node.kind
            && let Ok(addr) = addr::Addr::parse(target)
            && let Some(root) = addr.root()
        {
            roots.insert(root.clone());
        }
        for child in &node.children {
            collect(child, roots);
        }
    }
    let mut roots = BTreeSet::new();
    collect(&doc.root, &mut roots);
    roots
}

/// An eager corpus, borrowed or owned, with a lazily built name index. Merely
/// testing the SQL pin does not walk/clone the corpus or build its index.
#[derive(Debug)]
pub struct EagerCatalog<'a> {
    docs: Cow<'a, Docs>,
    index: OnceLock<CorpusIndex>,
}
impl<'a> EagerCatalog<'a> {
    #[must_use]
    pub fn new(docs: &'a Docs) -> Self {
        Self {
            docs: Cow::Borrowed(docs),
            index: OnceLock::new(),
        }
    }
    /// The documents this catalog serves.
    #[must_use]
    pub fn docs(&self) -> &Docs {
        &self.docs
    }
}
impl EagerCatalog<'static> {
    /// A catalog that owns its corpus, for a host with no longer-lived owner.
    #[must_use]
    pub fn owned(docs: Docs) -> Self {
        Self {
            docs: Cow::Owned(docs),
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
/// A lease on a document its owner already holds in memory: it cannot fault
/// and releases nothing.
#[must_use]
pub fn resident_lease(doc: &Document) -> Box<dyn DocumentLease + '_> {
    Box::new(EagerLease(doc))
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
            for (path, doc) in self.docs() {
                index.insert(path, doc);
            }
            index
        })
    }
    fn link_roots(&self) -> BTreeSet<MountName> {
        self.docs.values().flat_map(|doc| link_roots(doc)).collect()
    }
    fn pin(&self, path: &str) -> io::Result<Option<Box<dyn DocumentLease + '_>>> {
        Ok(self.docs.get(path).map(|doc| resident_lease(doc)))
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
/// The ambient root's hash domain and disk travel with it, as they do with
/// [`RootedCorpus`]: a face that colours pins needs both questions answered
/// by whoever built the generation.
#[derive(Debug)]
pub struct RootedCatalog<'a> {
    ambient: &'a dyn DocumentCatalog,
    mounted: BTreeMap<MountName, &'a dyn DocumentCatalog>,
    ambient_domain: Option<&'a dyn HashDomain>,
    ambient_disk: Option<&'a dyn AmbientDisk>,
}
impl<'a> RootedCatalog<'a> {
    #[must_use]
    pub fn ambient(catalog: &'a dyn DocumentCatalog) -> Self {
        Self {
            ambient: catalog,
            mounted: BTreeMap::new(),
            ambient_domain: None,
            ambient_disk: None,
        }
    }
    #[must_use]
    pub fn with_root(mut self, name: MountName, catalog: &'a dyn DocumentCatalog) -> Self {
        self.mounted.insert(name, catalog);
        self
    }
    /// Record the hash domain the ambient generation was built under.
    #[must_use]
    pub fn with_hash_domain(mut self, domain: &'a dyn HashDomain) -> Self {
        self.ambient_domain = Some(domain);
        self
    }
    /// Record the disk the ambient generation was built from.
    #[must_use]
    pub fn with_ambient_disk(mut self, disk: &'a dyn AmbientDisk) -> Self {
        self.ambient_disk = Some(disk);
        self
    }
    /// The ambient root's generation.
    #[must_use]
    pub fn ambient_catalog(&self) -> &'a dyn DocumentCatalog {
        self.ambient
    }
    /// One mounted root's generation, by canonical name.
    #[must_use]
    pub fn root(&self, name: &MountName) -> Option<&'a dyn DocumentCatalog> {
        self.mounted.get(name).copied()
    }
    /// [`RootedCorpus::in_hash_domain`], with the same `None` = cannot say.
    #[must_use]
    pub fn in_hash_domain(&self, root: Option<&MountName>, path: &str) -> Option<bool> {
        match root {
            None => self.ambient_domain.map(|d| d.contains(path)),
            Some(_) => None,
        }
    }
    /// [`RootedCorpus::on_ambient_disk`], with the same `None` = cannot say.
    #[must_use]
    pub fn on_ambient_disk(&self, root: Option<&MountName>, path: &str) -> Option<bool> {
        match root {
            None => self.ambient_disk.and_then(|d| d.exists(path)),
            Some(_) => None,
        }
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
