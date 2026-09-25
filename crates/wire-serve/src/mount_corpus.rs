//! The mount-table corpus assembly the pin planes share (walk/check/status
//! and the § A.10 wire `walk`): one loader binds `MERIDIAN.md`'s table and
//! obtains a corpus per NEEDED root, one assembler produces the
//! [`model::RootedCorpus`] every colour door reads. One owner — the CLI verbs
//! and the wire serve path must hand the colour computer the same corpus, so
//! the table bind, the refusal carrying, and the set assembly cannot drift
//! between hosts (the same one-owner law `Mounts::rooted` was born under).
//!
//! WHERE a wanted root's corpus comes from is the caller's ([`CorpusSource`]):
//! the in-process lanes build it from disk ([`load_mounts_for`]); the daemon
//! hands in its resident engine for that root ([`load_mounts_for_from`]) —
//! parsed once, refreshed O(dirty) by the same currency pass every served
//! workspace gets — so a mounted root is never rebuilt from disk per call. A
//! per-call disk build inside the daemon costs the whole root's walk, hash
//! and parse, and overlapping calls each hold a copy (measured on a 61k-file,
//! 8.9 GB sessions root: two concurrent `sql` calls at 17.8 GB resident).
//!
//! Voicing is the CALLER's: this module returns each mounted root's unserved
//! members instead of printing them — the CLI voices to stderr, the wire host
//! has no stderr to speak on.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A mounted root's served projection, whoever produced it: the documents
/// the colour computer reads and the hash-domain members that serve no
/// spans (per-file UTF-8 degradation), path → condition.
pub trait ServedCorpus: Send + Sync {
    /// The parsed documents, keyed by root-relative path.
    fn docs(&self) -> &model::Docs;
    /// Hash-domain members under this root that serve no spans/nodes.
    fn unserved(&self) -> &BTreeMap<String, String>;
}

/// How a wanted root's corpus is obtained, by its canonical bound path. `Err`
/// is the reason the root cannot be read from here — carried into the mount
/// set as unreachable, never dropped.
pub type CorpusSource<'a> = &'a dyn Fn(&Path) -> Result<Arc<dyn ServedCorpus>, String>;

/// The in-process build: snapshot + parse, exactly what the workspace's own
/// ambient corpus is built through.
struct BuiltCorpus {
    docs: model::Docs,
    unserved: BTreeMap<String, String>,
}

impl ServedCorpus for BuiltCorpus {
    fn docs(&self) -> &model::Docs {
        &self.docs
    }

    fn unserved(&self) -> &BTreeMap<String, String> {
        &self.unserved
    }
}

/// One mounted root's corpus — the backing store the [`model::RootedCorpus`]
/// borrows — plus the members it could not serve.
pub struct MountedCorpus {
    pub name: addr::MountName,
    /// The root's canonical bound path (canonicalize-at-bind) — the handle
    /// the per-root durability read (`check`) opens that root's git through.
    pub root: PathBuf,
    corpus: Arc<dyn ServedCorpus>,
}

impl MountedCorpus {
    /// The root's parsed documents.
    #[must_use]
    pub fn docs(&self) -> &model::Docs {
        self.corpus.docs()
    }

    /// Hash-domain members under this root that serve no spans/nodes,
    /// path → condition. The caller voices.
    #[must_use]
    pub fn unserved(&self) -> &BTreeMap<String, String> {
        self.corpus.unserved()
    }
}

/// The mount table as the pin planes consume it: the loaded corpora, and the
/// projection naming what the file declares and which of it is usable.
#[derive(Default)]
pub struct MountCorpora {
    /// The per-root corpora, in table order.
    pub corpora: Vec<MountedCorpus>,
    set: addr::MountSet,
}

impl MountCorpora {
    /// The root-keyed corpus over `docs`: the ambient workspace plus one root
    /// per bound mount. One owner for this assembly — every colour door reads
    /// pins through the same computer, so they must hand it the same corpus.
    /// `domain` is the ambient workspace's hash domain, REQUIRED rather than
    /// optional: a colour door that cannot say which paths its corpus was
    /// filtered by reds every out-of-domain target (decision 0034). `root` is
    /// REQUIRED for the same reason one level down (decision 0049): a colour
    /// door that knows a target is out of domain but cannot READ that target
    /// greys an absence, and the corpus map cannot tell the two apart.
    #[must_use]
    pub fn rooted<'a>(
        &'a self,
        docs: &'a model::Docs,
        domain: &'a fs::domain::Domain,
        root: &'a fs::WorkspaceRoot,
    ) -> model::RootedCorpus<'a> {
        let mut corpus = model::RootedCorpus::ambient(docs)
            .with_hash_domain(domain)
            .with_ambient_disk(root);
        for mount in &self.corpora {
            corpus = corpus.with_root(mount.name.clone(), mount.docs());
        }
        corpus
    }

    /// The projection naming what `MERIDIAN.md` declares and which of it is
    /// usable — the mount-table resolution is a lookup in.
    #[must_use]
    pub fn set(&self) -> &addr::MountSet {
        &self.set
    }
}

/// [`load_mounts_where`], building a corpus only for the roots in `needed` —
/// a corpus build costs the root's size, and nothing about the workspace
/// bounds it. Which roots are needed is the caller's question: the pin planes
/// read them off lock items, the link planes off wikilink/embed targets.
/// `needed` narrows the corpora, never the [`addr::MountSet`].
#[must_use]
pub fn load_mounts_for(needed: &BTreeSet<addr::MountName>) -> MountCorpora {
    load_mounts_for_from(needed, &build_in_process)
}

/// [`load_mounts_for`] with the corpora obtained through `source` instead of
/// built from disk — the daemon's lane, handing in its resident engines.
#[must_use]
pub fn load_mounts_for_from(
    needed: &BTreeSet<addr::MountName>,
    source: CorpusSource<'_>,
) -> MountCorpora {
    load_mounts_where_from(&|name| needed.contains(name), source)
}

/// Load `MERIDIAN.md`'s mount table into both halves resolution needs: one
/// corpus per usable wanted root, and the projection that says what the file
/// declares. A declared root that cannot be used is carried with its state,
/// never dropped.
#[must_use]
pub fn load_mounts_where(wanted: &dyn Fn(&addr::MountName) -> bool) -> MountCorpora {
    load_mounts_where_from(wanted, &build_in_process)
}

/// [`load_mounts_where`] with the corpora obtained through `source`.
#[must_use]
pub fn load_mounts_where_from(
    wanted: &dyn Fn(&addr::MountName) -> bool,
    source: CorpusSource<'_>,
) -> MountCorpora {
    let env = config::Env::from_process();
    let Ok(resolution) = config::resolve(&env) else {
        return MountCorpora::default();
    };
    // Through `Resolution::bind`, not the declared-only `mount::bind`: the
    // table walk and sql resolve against must be the SAME table every other
    // door binds — implicit default `sessions` mount included (schema §5.1c),
    // and state A (no config) still serves the default when it binds.
    let Ok(table) = resolution.bind(&env) else {
        return MountCorpora::default();
    };

    let mut corpora: Vec<MountedCorpus> = Vec::new();
    let mut bound: Vec<addr::MountName> = Vec::new();
    let mut unreachable: Vec<(addr::MountName, String, String)> = Vec::new();

    for mount in table.mounts() {
        let Ok(name) = addr::MountName::parse(mount.name()) else {
            continue; // not a canonical name — no address can reach it anyway
        };
        // Declared, but the mount plane refuses it. Carried, with the path it
        // declares and that plane's own reason verbatim — never dropped.
        if mount.state().refuses() {
            // The raw filesystem reason where the mount plane has one;
            // otherwise that plane's teaching is the most specific available.
            let detail = match mount.state() {
                config::mount::MountState::PathUnseeable { detail } => detail.clone(),
                other => other.detail(),
            };
            unreachable.push((name, mount.declared_path().to_owned(), detail));
            continue;
        }
        let Some(path) = mount.canonical_path() else {
            continue;
        };
        // Bound per the table, corpus not asked for. It joins `bound` exactly
        // as a built root does: the table's answer does not depend on whether
        // this process read its pages.
        if !wanted(&name) {
            bound.push(name);
            continue;
        }
        match source(path) {
            Ok(corpus) => {
                bound.push(name.clone());
                corpora.push(MountedCorpus {
                    name,
                    root: path.to_path_buf(),
                    corpus,
                });
            }
            // Bound per the table, but its corpus cannot be read from here —
            // and just as much a declared root as any other.
            Err(e) => unreachable.push((name, path.display().to_string(), e)),
        }
    }

    let mut set = addr::MountSet::new(bound);
    for (name, path, detail) in unreachable {
        set = set.with_unreachable(name, path, detail);
    }
    MountCorpora { corpora, set }
}

/// The corpus build at an already-canonical mount path (canonicalize-at-bind),
/// so re-resolving it would ask a second owner an answered question.
fn build_in_process(root: &Path) -> Result<Arc<dyn ServedCorpus>, String> {
    let root = fs::WorkspaceRoot(root.to_path_buf());
    let (files, _fingerprint) =
        fs::domain_snapshot(&root).map_err(|e| format!("cannot read the mounted corpus: {e}"))?;
    let (_index, docs, unserved) = fs::build_corpus(files);
    Ok(Arc::new(BuiltCorpus { docs, unserved }))
}
