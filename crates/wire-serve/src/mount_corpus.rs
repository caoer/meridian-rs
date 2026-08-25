//! The mount-table corpus assembly the pin planes share (walk/check/status
//! and the § A.10 wire `walk`): one loader binds `MERIDIAN.md`'s table and
//! builds a corpus per NEEDED root, one assembler produces the
//! [`model::RootedCorpus`] every colour door reads. One owner — the CLI verbs
//! and the wire serve path must hand the colour computer the same corpus, so
//! the table bind, the refusal carrying, and the set assembly cannot drift
//! between hosts (the same one-owner law `Mounts::rooted` was born under).
//!
//! Voicing is the CALLER's: this module returns each mounted root's unserved
//! members instead of printing them — the CLI voices to stderr, the wire host
//! has no stderr to speak on.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// One mounted root's corpus, owned — the backing store the
/// [`model::RootedCorpus`] borrows — plus the members it could not serve.
pub struct MountedCorpus {
    pub name: addr::MountName,
    /// The root's canonical bound path (canonicalize-at-bind) — the handle
    /// the per-root durability read (`check`) opens that root's git through.
    pub root: std::path::PathBuf,
    pub docs: model::Docs,
    /// Hash-domain members under this root that serve no spans/nodes
    /// (per-file UTF-8 degradation), path → condition. The caller voices.
    pub unserved: BTreeMap<String, String>,
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
            corpus = corpus.with_root(mount.name.clone(), &mount.docs);
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
    load_mounts_where(&|name| needed.contains(name))
}

/// Load `MERIDIAN.md`'s mount table into both halves resolution needs: one
/// corpus per usable wanted root, and the projection that says what the file
/// declares. A declared root that cannot be used is carried with its state,
/// never dropped.
#[must_use]
pub fn load_mounts_where(wanted: &dyn Fn(&addr::MountName) -> bool) -> MountCorpora {
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
        match build_docs_at(path) {
            Ok((docs, unserved)) => {
                bound.push(name.clone());
                corpora.push(MountedCorpus {
                    name,
                    root: path.to_path_buf(),
                    docs,
                    unserved,
                });
            }
            // Bound per the table, but its corpus will not build — unreadable
            // from here, and just as much a declared root as any other.
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
/// so re-resolving it would ask a second owner an answered question. Returns
/// the docs plus the unserved members for the caller to voice.
#[allow(clippy::type_complexity)] // the two corpus maps, verbatim from fs::build_corpus
fn build_docs_at(root: &Path) -> Result<(model::Docs, BTreeMap<String, String>), String> {
    let root = fs::WorkspaceRoot(root.to_path_buf());
    let (files, _fingerprint) =
        fs::domain_snapshot(&root).map_err(|e| format!("cannot read the mounted corpus: {e}"))?;
    let (_index, docs, unserved) = fs::build_corpus(files);
    Ok((docs, unserved))
}
