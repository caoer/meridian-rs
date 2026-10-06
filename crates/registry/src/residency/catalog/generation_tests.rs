//! The incremental pass over a residency generation (merkle-spec §6.8):
//! a cold build is that pass over the empty generation, and a rebuild shares
//! every unmoved slot without loading a prior version.

use super::*;
use crate::residency::Phase;
use std::path::{Path, PathBuf};

const PEAK: usize = 131_072;

type Leaves = BTreeMap<PathBuf, [u8; 32]>;

fn workspace(files: &[(&str, &[u8])]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (path, bytes) in files {
        write(dir.path(), path, bytes);
    }
    dir
}

fn write(root: &Path, path: &str, bytes: &[u8]) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// The eager build of the workspace as it is now — the oracle every
/// generation is compared with.
struct Eager {
    index: CorpusIndex,
    docs: Docs,
    unserved: BTreeMap<String, String>,
    leaves: Leaves,
    root: model::MerkleRoot,
}

fn eager(root: &fs::WorkspaceRoot) -> Eager {
    let (files, leaves, fingerprint) = fs::domain_snapshot_with_leaves(root).unwrap();
    let (index, docs, unserved) = fs::build_corpus(files);
    Eager {
        index,
        docs,
        unserved,
        leaves,
        root: fingerprint,
    }
}

/// One pass over `prior` toward the workspace as it is now, planned the way
/// the registry plans it: the movers between the prior's leaves and the
/// current ones, and the current fold as the stamp.
fn pass(
    root: &fs::WorkspaceRoot,
    prior: &fs::CorpusPrior<'_, Slots>,
    controller: &Controller,
    store: &RawStore,
) -> fs::CorpusUpdate<Slots> {
    let now = eager(root);
    let moves = fs::leaf_delta(prior.leaves, now.leaves.iter().map(|(rel, d)| (rel, *d)));
    update(root, prior, &moves, Some(&now.root), controller, store).unwrap()
}

fn cold(
    root: &fs::WorkspaceRoot,
    controller: &Controller,
    store: &RawStore,
) -> fs::CorpusUpdate<Slots> {
    let index = Arc::new(CorpusIndex::new());
    pass(
        root,
        &fs::CorpusPrior {
            docs: &Slots::default(),
            unserved: &BTreeMap::new(),
            leaves: &BTreeMap::new(),
            index: Some(&index),
        },
        controller,
        store,
    )
}

fn prior(update: &fs::CorpusUpdate<Slots>) -> fs::CorpusPrior<'_, Slots> {
    fs::CorpusPrior {
        docs: &update.docs,
        unserved: &update.unserved,
        leaves: &update.leaves,
        index: Some(&update.index),
    }
}

/// `update` serves exactly what the eager build of the same disk serves:
/// stamp, leaves, unserved members, name index, revisions and every body.
fn assert_serves_eager(update: &fs::CorpusUpdate<Slots>, root: &fs::WorkspaceRoot) {
    let want = eager(root);
    assert_eq!(update.root, want.root, "stamp");
    assert_eq!(update.leaves, want.leaves, "leaves");
    assert_eq!(update.unserved, want.unserved, "unserved members");
    assert_eq!(*update.index, want.index, "name index");
    let catalog = Catalog::new(update.docs.clone(), update.index.clone(), PEAK);
    let revisions: Vec<_> = catalog
        .revisions()
        .map(|(path, rev)| (path.to_owned(), rev.clone()))
        .collect();
    let expected: Vec<_> = want
        .docs
        .iter()
        .map(|(path, doc)| (path.clone(), doc.root.node_rev.clone()))
        .collect();
    assert_eq!(revisions, expected, "membership and revisions");
    for (path, doc) in &want.docs {
        let lease = catalog.pin(path).unwrap().expect("a member leases");
        assert_eq!(
            lease.document().unwrap().raw.as_str(),
            doc.raw.as_str(),
            "{path}"
        );
    }
}

#[test]
fn a_cold_build_over_the_empty_generation_serves_the_eager_build() {
    let dir = workspace(&[
        (
            "a.md",
            b"---\naliases: [first, other]\n---\n# A\n\n[[b]] [[c:remote]]\n",
        ),
        ("notes/b.md", b"# B\n\nbody\n"),
        ("binary.md", b"# not utf-8 \xff\n"),
        ("empty.md", b""),
    ]);
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    let backing = tempfile::tempdir().unwrap();
    let store = RawStore::new(backing.path()).unwrap();
    let controller = Controller::new(0);

    let built = cold(&root, &controller, &store);

    assert_eq!(built.parsed, 3, "every servable member parsed once");
    assert!(
        built
            .docs
            .values()
            .all(|slot| slot.member.phase() == Phase::Cold),
        "the build keeps no parsed body: every slot is published cold"
    );
    assert_eq!(controller.report().members, 3);
    assert_serves_eager(&built, &root);
}

#[test]
fn a_rebuild_shares_unmoved_slots_and_loads_no_prior_version() {
    let dir = workspace(&[
        ("a.md", b"---\naliases: [old]\n---\n# A\n"),
        ("b.md", b"# B\n\nfirst\n"),
        ("c.md", b"# C\n"),
        ("d.md", b"# D\n\n[[old]]\n"),
    ]);
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    let backing = tempfile::tempdir().unwrap();
    let store = RawStore::new(backing.path()).unwrap();
    let controller = Controller::new(0);
    let first = cold(&root, &controller, &store);

    // a re-aliases, b changes content under the same rows, c goes, e comes;
    // d does not move.
    write(dir.path(), "a.md", b"---\naliases: [new]\n---\n# A\n");
    write(dir.path(), "b.md", b"# B\n\nsecond\n");
    std::fs::remove_file(dir.path().join("c.md")).unwrap();
    write(dir.path(), "e.md", b"# E\n");
    let second = pass(&root, &prior(&first), &controller, &store);

    assert!(
        Arc::ptr_eq(&first.docs["d.md"], &second.docs["d.md"]),
        "an unmoved member keeps its slot: one controller member across generations"
    );
    for (path, slot) in first.docs.iter() {
        assert_eq!(
            slot.member.phase(),
            Phase::Cold,
            "the rebuild loaded the prior version of {path}"
        );
    }
    assert_eq!(second.parsed, 3, "movers alone are parsed");
    assert_eq!(
        controller.report().members,
        7,
        "four first-generation versions and three movers"
    );
    assert_serves_eager(&second, &root);

    drop(first);
    assert_eq!(
        controller.report().members,
        4,
        "releasing the first generation releases the versions only it held"
    );
}
