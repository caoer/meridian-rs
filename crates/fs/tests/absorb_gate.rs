//! The §6.8 absorb-path gates (merkle-spec §6.8, "deriving the answer at the
//! cost of the change"):
//!
//! - **Overlay-path equivalence** — after each own-write mutation class
//!   (leaf, remove, membership) the served fold from the resident tree is
//!   byte-identical to the flat oracle ([`fs::served_root`]) over the memo's
//!   own leaf set: the lockstep invariant, gated at the memo grain. (The
//!   observation path's identity is `resident_gate`'s cold/warm/delete gate;
//!   the §6.5 restore verifies the same binding at adoption.)
//! - **The fold-free rebuild stamp** — an incremental pass whose built leaf
//!   set is byte-equal to its snapshot stamps the snapshot's own minted
//!   root; a divergent set folds what was actually built; no snapshot root
//!   means the flat fold, unchanged.
//! - **Carry by reference** — an unmoved member's document is pointer-equal
//!   across the rebuild (`Arc::ptr_eq`), a mover's is a fresh allocation:
//!   the no-clone proof.

use std::fs as stdfs;
use std::path::Path;
use std::sync::Arc;

use fs::{DomainCache, WorkspaceRoot};

fn write(root: &Path, rel: &str, contents: &[u8]) {
    let abs = root.join(rel);
    if let Some(parent) = abs.parent() {
        stdfs::create_dir_all(parent).expect("mkdir");
    }
    stdfs::write(abs, contents).expect("write fixture");
}

fn workspace() -> (tempfile::TempDir, WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

/// The flat oracle: [`fs::served_root`] over the memo's current leaf set at
/// the workspace's current domain version — the §6.8 "fresh build over the
/// same leaves" the resident fold must equal.
fn oracle(cache: &mut DomainCache, root: &WorkspaceRoot) -> model::MerkleRoot {
    let version = fs::domain::Domain::load(root).expect("domain").version();
    let leaves = cache.leaf_digests();
    let refs: Vec<(&[u8], [u8; 32])> = leaves
        .iter()
        .map(|(rel, d)| (fs::hash_name(rel), *d))
        .collect();
    fs::served_root(&refs, version)
}

/// Overlay-path equivalence: each own-write mutation class advances the tree
/// and the leaf memo in lockstep, so the served fold equals the flat oracle
/// after every step — including through a §4.4 collision composed and
/// cleared by the overlay itself.
#[test]
fn overlay_serves_the_flat_oracle_after_every_mutation_class() {
    let (_dir, root) = workspace();
    write(&root.0, "notes.md", b"# Notes\n");
    write(&root.0, "tasks/a.md", b"# A\n");
    let mut cache = DomainCache::new();
    cache.root(&root).expect("baseline observation");

    // Insert/update half: a new leaf, then an update of an existing one.
    for (rel, bytes) in [
        ("tasks/b.md", b"# B\n".as_slice()),
        ("notes.md", b"# Notes v2\n".as_slice()),
        // §4.4 collision composed through the overlay: `tasks/a.md` is a
        // file; a leaf UNDER that name makes the key both kinds.
        ("tasks/a.md/child.md", b"# C\n".as_slice()),
    ] {
        assert!(
            cache
                .overlay_leaf(Path::new(rel), model::leaf_digest(bytes))
                .expect("overlay leaf"),
            "{rel}: overlay advances"
        );
        assert_eq!(
            cache.overlay_root().expect("served"),
            oracle(&mut cache, &root),
            "{rel}: served fold diverged from the flat oracle"
        );
    }

    // Removal half — clearing the collision child and a plain member.
    for rel in ["tasks/a.md/child.md", "tasks/b.md"] {
        assert!(
            cache
                .overlay_remove(Path::new(rel))
                .expect("overlay remove"),
            "{rel}: removal advances"
        );
        assert_eq!(
            cache.overlay_root().expect("served"),
            oracle(&mut cache, &root),
            "{rel}: post-remove fold diverged from the flat oracle"
        );
    }

    // Membership half: impose a config generation that departs `tasks/`.
    write(
        &root.0,
        "meridian/domain.md",
        b"---\nversion: 2\nignore:\n  - \"tasks/\"\n---\n\n# Domain\n",
    );
    let domain = fs::domain::Domain::load(&root).expect("load config");
    assert!(
        cache
            .overlay_membership(domain)
            .expect("overlay membership"),
        "membership imposition advances"
    );
    assert_eq!(
        cache.overlay_root().expect("served"),
        oracle(&mut cache, &root),
        "post-membership fold diverged from the flat oracle"
    );
}

/// The §6.8 fold-free rebuild stamp, all three arms: equal set → the
/// snapshot's own root; divergent set → the fold of what was built; no
/// snapshot root → the flat fold. Every arm's stamp equals the flat oracle
/// of the leaf set it describes.
#[test]
fn rebuild_stamp_rides_input_equality() {
    let (_dir, root) = workspace();
    write(&root.0, "a.md", b"# A\n");
    write(&root.0, "b.md", b"# B\n");
    let (files, prior_leaves, prior_root) =
        fs::domain_snapshot_with_leaves(&root).expect("cold snapshot");
    let (_, prior_docs, prior_unserved) = fs::build_corpus(files);

    // Arm 1 — equal set: the memo's snapshot (leaves + minted root, one
    // lock) IS the built set; the stamp is that root, and it equals the
    // flat oracle by construction.
    let mut cache = DomainCache::new();
    cache.root(&root).expect("baseline");
    let fresh = cache.leaf_digests();
    let fresh_root = cache.overlay_root().expect("minted snapshot root");
    let update = fs::update_corpus(
        &root,
        &prior_docs,
        &prior_unserved,
        &prior_leaves,
        &fresh,
        Some(&fresh_root),
    )
    .expect("equal-set pass");
    assert_eq!(update.leaves, fresh, "nothing diverged");
    assert_eq!(
        update.root, fresh_root,
        "equal set: the stamp is the snapshot's own minted root"
    );
    assert_eq!(update.root, prior_root, "…which is the tree's true fold");

    // Arm 2 — divergence: a MOVER vanishes between the leaf pass and the
    // read. (A vanished member whose digest never moved is CARRIED on the
    // snapshot's evidence — the memo's §6.2 grade, unchanged here; only a
    // mover is read now, so only a mover can diverge the built set.)
    write(&root.0, "b.md", b"# B v2\n");
    cache.root(&root).expect("observe the move");
    let fresh = cache.leaf_digests();
    let fresh_root = cache.overlay_root().expect("moved snapshot root");
    stdfs::remove_file(root.0.join("b.md")).expect("rm the mover");
    let update = fs::update_corpus(
        &root,
        &prior_docs,
        &prior_unserved,
        &prior_leaves,
        &fresh,
        Some(&fresh_root),
    )
    .expect("divergent pass");
    assert_ne!(update.leaves, fresh, "the built set diverged");
    assert_ne!(
        update.root, fresh_root,
        "divergence: the snapshot root would be a lie"
    );
    let (_, survivor_snapshot) = fs::domain_snapshot(&root).expect("survivor oracle");
    assert_eq!(
        update.root, survivor_snapshot,
        "divergence: the stamp folds the built set"
    );

    // Arm 3 — no snapshot root: the flat fold, unchanged.
    let update = fs::update_corpus(
        &root,
        &prior_docs,
        &prior_unserved,
        &prior_leaves,
        &update.leaves.clone(),
        None,
    )
    .expect("no-snapshot pass");
    assert_eq!(update.root, survivor_snapshot, "None: the flat fold serves");
}

/// Carry by reference: the incremental pass clones pointers for unmoved
/// members and allocates only movers — `Arc::ptr_eq` is the no-clone proof.
#[test]
fn rebuild_carries_unmoved_documents_by_pointer() {
    let (_dir, root) = workspace();
    write(&root.0, "kept.md", b"# Kept\n\nbody\n");
    write(&root.0, "moved.md", b"# Moved\n\nv1\n");
    let (files, prior_leaves, _) = fs::domain_snapshot_with_leaves(&root).expect("cold");
    let (_, prior_docs, prior_unserved) = fs::build_corpus(files);

    write(&root.0, "moved.md", b"# Moved\n\nv2\n");
    let mut cache = DomainCache::new();
    cache.root(&root).expect("observe the move");
    let fresh = cache.leaf_digests();
    let fresh_root = cache.overlay_root().expect("snapshot root");
    let update = fs::update_corpus(
        &root,
        &prior_docs,
        &prior_unserved,
        &prior_leaves,
        &fresh,
        Some(&fresh_root),
    )
    .expect("incremental pass");

    assert_eq!(update.parsed, 1, "exactly the mover parses");
    assert!(
        Arc::ptr_eq(&update.docs["kept.md"], &prior_docs["kept.md"]),
        "an unmoved member's document is carried by pointer, never copied"
    );
    assert!(
        !Arc::ptr_eq(&update.docs["moved.md"], &prior_docs["moved.md"]),
        "a mover's document is a fresh allocation"
    );
    assert_eq!(update.docs["moved.md"].raw, "# Moved\n\nv2\n");
}
