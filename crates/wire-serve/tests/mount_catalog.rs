//! A mounted root bound into the link plane's rooted catalog is held as its
//! generation's descriptors (merkle-spec §6.8): rooted name questions are
//! answered from that root's membership and name index, and no document of
//! it is leased or read.
//!
//! One test fn, alone in this binary: `MERIDIAN_CONFIG` is process-global.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use model::catalog::{DocumentCatalog, DocumentLease, EagerCatalog, NameCorpus};
use wire_serve::mount_corpus::{ServedCorpus, load_mounts_for_from};

/// A mounted root that can answer as descriptors only: every lease is
/// counted, and it has no document map to hand out.
#[derive(Debug)]
struct Descriptors {
    eager: EagerCatalog<'static>,
    leases: AtomicUsize,
    unserved: BTreeMap<String, String>,
}
impl DocumentCatalog for Descriptors {
    fn revisions(&self) -> Box<dyn Iterator<Item = (&str, &model::NodeRev)> + '_> {
        self.eager.revisions()
    }
    fn revision(&self, path: &str) -> Option<&model::NodeRev> {
        self.eager.revision(path)
    }
    fn index(&self) -> &model::CorpusIndex {
        self.eager.index()
    }
    fn link_roots(&self) -> BTreeSet<addr::MountName> {
        self.eager.link_roots()
    }
    fn pin(&self, path: &str) -> io::Result<Option<Box<dyn DocumentLease + '_>>> {
        self.leases.fetch_add(1, Ordering::Relaxed);
        self.eager.pin(path)
    }
}
impl ServedCorpus for Descriptors {
    fn catalog(&self) -> &dyn DocumentCatalog {
        self
    }
    fn docs(&self) -> &model::Docs {
        panic!("the link plane reads no document map of a mounted root");
    }
    fn unserved(&self) -> &BTreeMap<String, String> {
        &self.unserved
    }
}

fn docs(files: &[(&str, &str)]) -> model::Docs {
    files
        .iter()
        .map(|(path, raw)| {
            (
                (*path).to_owned(),
                Arc::new(model::build((*raw).to_owned(), syntax::parse(raw))),
            )
        })
        .collect()
}

#[test]
fn a_mounted_root_is_named_through_its_catalog_and_no_body_is_leased() {
    let tmp = tempfile::tempdir().unwrap();
    let other = tmp.path().join("other");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(
        other.join("MERIDIAN.md"),
        "---\ntype: meridian-root\nversion: 1\nname: other\n---\n",
    )
    .unwrap();
    let config = tmp.path().join("conf").join("MERIDIAN.md");
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(
        &config,
        format!(
            "---\ntype: meridian-config\nversion: 1\n---\n\n# Roots\n\n```meridian-mount\nname: other\npath: {}\nvault: other\n```\n",
            other.display()
        ),
    )
    .unwrap();
    // SAFETY: the only test in this binary, set before the table is read.
    unsafe { std::env::set_var("MERIDIAN_CONFIG", &config) };

    let name = addr::MountName::parse("other").unwrap();
    let mounted = Arc::new(Descriptors {
        eager: EagerCatalog::owned(docs(&[("deep/c.md", "---\naliases: [sea]\n---\n# C\n")])),
        leases: AtomicUsize::new(0),
        unserved: BTreeMap::new(),
    });
    let asked = AtomicUsize::new(0);
    let mounts = load_mounts_for_from(&BTreeSet::from([name.clone()]), &|_root| {
        asked.fetch_add(1, Ordering::Relaxed);
        Ok(Arc::clone(&mounted) as Arc<dyn ServedCorpus>)
    });
    assert_eq!(
        asked.load(Ordering::Relaxed),
        1,
        "the table binds `other`, so its corpus is asked for once"
    );

    let ambient_docs = docs(&[("a.md", "# A\n\n[[other:sea]]\n")]);
    let ambient = EagerCatalog::new(&ambient_docs);
    let corpus = mounts.rooted_catalog(&ambient);

    assert!(
        corpus.contains(Some(&name), "deep/c.md"),
        "a member of the mounted generation is a member of the rooted catalog"
    );
    assert!(!corpus.contains(Some(&name), "a.md"));
    assert!(corpus.contains(None, "a.md"));
    assert_eq!(
        corpus
            .root_index(&name)
            .expect("a bound root carries its name index")
            .resolve_linkpath("sea", "a.md"),
        Some("deep/c.md".to_owned()),
        "an alias resolves through the mounted generation's own name index"
    );
    assert_eq!(
        mounted.leases.load(Ordering::Relaxed),
        0,
        "naming a mounted root's members leases none of its bodies"
    );
}
