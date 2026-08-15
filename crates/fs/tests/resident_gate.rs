//! The resident-tree gates (merkle-spec §6.1; merged plan §6 step 3, card
//! `resident-tree`):
//!
//! - **Byte identity, law 1** — the served root through the resident
//!   structure equals `model::merkle_root_of_leaves` / `domain_snapshot` /
//!   `DomainLeaves::overlay` over the same tree (the fable-bench assertion,
//!   re-run against this implementation).
//! - **Byte identity, law 2** — the resident fingerprint equals the spec §5.1
//!   pinned values on the spec's own workspace, through the whole tree end to
//!   end, including the incremental splice.
//! - **The interim served-token law** — the law-1 serve recomputes only when
//!   the root advances (`flat_folds` counts serves, not passes).
//! - **§4.4 collisions** — composition lints, the fold keeps both kinds, the
//!   key and paths through it refuse, and clearing one kind recovers.
//! - **§7 scope rows** — folder/file/`absent`/kind-conflict answers.

use std::fs as stdfs;
use std::path::Path;

use fs::resident::{RefusalReason, ScopeFold};
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

fn hex(v: [u8; 32]) -> String {
    blake3::Hash::from_bytes(v).to_hex().to_string()
}

/// Spec §5's two-file workspace, byte-exact.
const NOTES: &[u8] = b"# Notes\n\nhello\n";
const X_V0: &[u8] = b"---\ntitle: demo\n---\n\n# Alpha\n\nbody line one\n\n## Beta\n\nbeta body\n";
const X_V2: &[u8] =
    b"---\ntitle: demo\n---\n\n# Alpha\n\nbody line one\n\n## Beta\n\nbeta body v2\n";

/// Spec §5.1's pinned law-2 values (card radix-map's receipt — here re-served
/// through the resident tree end to end).
const PIN_FINGERPRINT: &str = "d53c447167825d40f442c65b10f5ae2c6176a49e1e2d8237902d7eaa3008319e";
const PIN_DIR_TASKS: &str = "ef0e7e2eca3cacfcc3bf8fded1454d65645a5a20359c770d6e2dea009d285bd2";
const PIN_FINGERPRINT_V2: &str = "6aab1dd1ef89648508430e0ded866c6ad964b1074fc9b624d025f5c27d10fc58";
const PIN_DIR_TASKS_V2: &str = "e4f51f04970d9feb5c680de5534e1824b27d2660577395e5fadcd9d82fb8a967";

/// The fable-bench assertion, re-run: the served root through the resident
/// structure is byte-identical to the engine's own from-scratch fold, cold,
/// after a foreign change, and after a deletion that prunes a directory.
#[test]
fn served_root_byte_identical_to_domain_snapshot() {
    let (_dir, root) = workspace();
    write(&root.0, "notes.md", NOTES);
    write(&root.0, "tasks/x.md", X_V0);
    write(&root.0, "a/deep/leaf.md", b"deep\n");
    let mut cache = DomainCache::new();

    let served = cache.root(&root).expect("cold root");
    let (_, snapshot) = fs::domain_snapshot(&root).expect("snapshot");
    assert_eq!(
        served, snapshot,
        "cold serve diverges from the engine's own fold"
    );

    // A foreign change lands on disk; the next observation must serve the
    // post-change fold, byte-identical to a fresh snapshot.
    write(&root.0, "tasks/x.md", X_V2);
    write(&root.0, "b/new.md", b"new member\n");
    let served = cache.root(&root).expect("warm root");
    let (_, snapshot) = fs::domain_snapshot(&root).expect("snapshot");
    assert_eq!(served, snapshot, "post-change serve diverges");

    // A deletion that empties a directory: the resident tree prunes; the
    // served fold matches the engine's own over the survivor set.
    stdfs::remove_file(root.0.join("a/deep/leaf.md")).expect("rm");
    let served = cache.root(&root).expect("post-delete root");
    let (_, snapshot) = fs::domain_snapshot(&root).expect("snapshot");
    assert_eq!(served, snapshot, "post-delete serve diverges");
}

/// The interim served-token law (merged plan §6 step 3): the law-1 fold runs
/// once per ADVANCE, never once per pass.
#[test]
fn served_root_recomputes_only_when_the_root_advances() {
    let (_dir, root) = workspace();
    write(&root.0, "notes.md", NOTES);
    let mut cache = DomainCache::new();

    cache.root(&root).expect("cold");
    assert_eq!(cache.flat_folds(), 1, "cold serve folds once");
    cache.root(&root).expect("quiet pass");
    cache.root(&root).expect("quiet pass");
    assert_eq!(cache.flat_folds(), 1, "a quiet corpus re-serves the cache");

    write(&root.0, "notes.md", b"# Notes\n\nmoved\n");
    cache.root(&root).expect("advanced");
    assert_eq!(cache.flat_folds(), 2, "an advance folds exactly once more");
    cache.root(&root).expect("quiet again");
    assert_eq!(cache.flat_folds(), 2);
}

/// The own-write overlay against the correctness law: `DomainLeaves::overlay`
/// and the resident overlay serve one root — and that root deliberately
/// EXCLUDES a foreign write racing the commit (more computed than a re-read),
/// while the next live observation folds the foreign bytes in.
#[test]
fn overlay_root_matches_the_domain_leaves_overlay_law() {
    let (_dir, root) = workspace();
    write(&root.0, "notes.md", NOTES);
    write(&root.0, "tasks/x.md", X_V0);
    let mut cache = DomainCache::new();
    cache.root(&root).expect("baseline");

    // The correctness law's answer over the same baseline + same edit.
    let mut law = cache.domain_leaves(&root).expect("baseline leaves");

    // The commit writes x.md; a foreign write lands on notes.md in the same
    // window. The overlay knows ONLY its own write.
    write(&root.0, "tasks/x.md", X_V2);
    write(&root.0, "notes.md", b"# Notes\n\nforeign racer\n");

    law.overlay(Path::new("tasks/x.md"), model::leaf_digest(X_V2));
    let advanced = cache
        .overlay_leaf(Path::new("tasks/x.md"), model::leaf_digest(X_V2))
        .expect("overlay after baseline");
    assert!(advanced, "the edit moves the tree");
    let overlay_root = cache.overlay_root().expect("overlay serve");
    assert_eq!(overlay_root, law.root(), "overlay diverges from the law");

    // More computed than a re-read: the foreign racer is NOT in this root...
    let (_, disk_now) = fs::domain_snapshot(&root).expect("snapshot");
    assert_ne!(
        overlay_root, disk_now,
        "a re-read would have leaked the racer in"
    );

    // ...and an idempotent re-serve pays no fold.
    let folds = cache.flat_folds();
    let again = cache.overlay_root().expect("re-serve");
    assert_eq!(again, overlay_root);
    assert_eq!(
        cache.flat_folds(),
        folds,
        "an unadvanced serve is a cache hit"
    );

    // The next live observation folds the racer in — byte-identical to the
    // engine's own fold over the full post-race tree.
    assert_eq!(cache.root(&root).expect("live"), disk_now);
}

/// An out-of-domain overlay path is ignored (the `DomainLeaves::overlay`
/// filter), and an overlay before any observation refuses loudly.
#[test]
fn overlay_filters_domain_and_refuses_without_baseline() {
    let (_dir, root) = workspace();
    write(&root.0, "notes.md", NOTES);

    let mut cold = DomainCache::new();
    assert!(
        cold.overlay_leaf(Path::new("notes.md"), [7; 32]).is_err(),
        "overlay without a baseline must refuse"
    );

    let mut cache = DomainCache::new();
    let before = cache.root(&root).expect("baseline");
    let advanced = cache
        .overlay_leaf(Path::new("outside.txt"), [7; 32])
        .expect("baseline landed");
    assert!(!advanced, "a non-domain path never enters the fold");
    assert_eq!(cache.overlay_root().expect("serve"), before);
}

/// The spec §5.1 workspace through the whole resident tree: build from disk,
/// pin the fingerprint and the `tasks/` scope value, splice via the overlay,
/// pin both again. Two implementations, two languages, one encoding — now
/// three layers deep (generator, child map, resident tree).
#[test]
fn law2_fingerprint_matches_the_spec_worked_example() {
    let (_dir, root) = workspace();
    write(&root.0, "notes.md", NOTES);
    write(&root.0, "tasks/x.md", X_V0);
    let mut cache = DomainCache::new();
    cache.root(&root).expect("observe");

    assert_eq!(hex(cache.law2_fingerprint()), PIN_FINGERPRINT);
    let ScopeFold::Value(tasks) = cache.fold_at(Path::new("tasks")).expect("tasks resolves") else {
        panic!("tasks is present");
    };
    assert_eq!(hex(tasks), PIN_DIR_TASKS);

    // The §5 incremental splice, applied as the own-write overlay.
    write(&root.0, "tasks/x.md", X_V2);
    cache
        .overlay_leaf(Path::new("tasks/x.md"), model::leaf_digest(X_V2))
        .expect("overlay");
    assert_eq!(hex(cache.law2_fingerprint()), PIN_FINGERPRINT_V2);
    let ScopeFold::Value(tasks) = cache.fold_at(Path::new("tasks")).expect("tasks resolves") else {
        panic!("tasks is present");
    };
    assert_eq!(hex(tasks), PIN_DIR_TASKS_V2);

    // The file-leaf scope row is the §3 leaf, law-independent.
    assert_eq!(
        cache.fold_at(Path::new("tasks/x.md")),
        Ok(ScopeFold::Value(model::leaf_digest(X_V2)))
    );
}

/// §4.4 on a fixture corpus: the overlay composes a directory through an
/// existing file name — the lint fires, the fold keeps serving with both
/// kinds inside, the colliding path and everything through it refuse, and
/// removing the file arm clears the key.
#[test]
fn collision_lints_and_refuses_scope_unresolved() {
    let (_dir, root) = workspace();
    write(&root.0, "a.md", b"file arm\n");
    write(&root.0, "other.md", b"bystander\n");
    let mut cache = DomainCache::new();
    cache.root(&root).expect("observe");
    assert!(
        cache.collision_paths().is_empty(),
        "a real walk composes none"
    );

    // A racing composition: the overlay learns of `a.md/inner.md` while the
    // observed generation still holds file `a.md`.
    cache
        .overlay_leaf(Path::new("a.md/inner.md"), model::leaf_digest(b"dir arm\n"))
        .expect("overlay");
    assert_eq!(cache.collision_paths(), vec!["a.md".to_owned()]);

    let refusal = cache
        .fold_at(Path::new("a.md"))
        .expect_err("the collision key refuses");
    assert_eq!(refusal.reason, RefusalReason::Collision);
    assert_eq!(refusal.path, "a.md");
    let through = cache
        .fold_at(Path::new("a.md/inner.md"))
        .expect_err("paths through the key refuse");
    assert_eq!(through.reason, RefusalReason::Collision);

    // Serving continues around it: root and bystander still answer.
    assert!(matches!(
        cache.fold_at(Path::new("")),
        Ok(ScopeFold::Value(_))
    ));
    assert!(matches!(
        cache.fold_at(Path::new("other.md")),
        Ok(ScopeFold::Value(_))
    ));

    // Removing the file arm clears the key; the directory arm serves.
    cache.overlay_remove(Path::new("a.md")).expect("remove");
    assert!(cache.collision_paths().is_empty());
    assert!(matches!(
        cache.fold_at(Path::new("a.md")),
        Ok(ScopeFold::Value(_))
    ));
}

/// §7 scope rows through the cache: `absent` chains (never-created and
/// emptied-then-pruned), and the kind-conflict refusal on a file prefix.
#[test]
fn scope_rows_absent_and_kind_conflict() {
    let (_dir, root) = workspace();
    write(&root.0, "notes.md", NOTES);
    write(&root.0, "tasks/x.md", X_V0);
    let mut cache = DomainCache::new();
    cache.root(&root).expect("observe");

    assert_eq!(
        cache.fold_at(Path::new("never/created/chain")),
        Ok(ScopeFold::Absent),
        "whole-prefix absence is still absent (the chain law)"
    );
    let conflict = cache
        .fold_at(Path::new("notes.md/under/file"))
        .expect_err("a file prefix conflicts in kind");
    assert_eq!(conflict.reason, RefusalReason::KindConflict);
    assert_eq!(conflict.path, "notes.md");

    // Empty the tasks directory on disk: the observation prunes the node and
    // the emptied path mints `absent` (§8 — no tombstone).
    stdfs::remove_file(root.0.join("tasks/x.md")).expect("rm");
    cache.root(&root).expect("observe the delete");
    assert_eq!(cache.fold_at(Path::new("tasks")), Ok(ScopeFold::Absent));
    assert_eq!(
        cache.fold_at(Path::new("tasks/x.md")),
        Ok(ScopeFold::Absent)
    );
}

/// Foreign changes mark the touched chain and lazy refolds land the same
/// values a fresh build computes — at the root and at every touched scope.
#[test]
fn foreign_changes_refold_to_the_fresh_build_values() {
    let (_dir, root) = workspace();
    write(&root.0, "a/one.md", b"one\n");
    write(&root.0, "a/two.md", b"two\n");
    write(&root.0, "b/three.md", b"three\n");
    let mut cache = DomainCache::new();
    cache.root(&root).expect("baseline");
    let ScopeFold::Value(b_before) = cache.fold_at(Path::new("b")).expect("b resolves") else {
        panic!("b is present");
    };

    // Foreign edits: one member changes, one arrives, one dies.
    write(&root.0, "a/one.md", b"one v2\n");
    write(&root.0, "b/four.md", b"four\n");
    stdfs::remove_file(root.0.join("a/two.md")).expect("rm");
    cache.root(&root).expect("observe the churn");

    // A fresh cache over the same disk is the from-scratch oracle.
    let mut fresh = DomainCache::new();
    fresh.root(&root).expect("fresh observe");
    assert_eq!(cache.law2_fingerprint(), fresh.law2_fingerprint());
    assert_eq!(cache.fold_at(Path::new("a")), fresh.fold_at(Path::new("a")));
    let ScopeFold::Value(b_after) = cache.fold_at(Path::new("b")).expect("b resolves") else {
        panic!("b is present");
    };
    assert_ne!(b_before, b_after, "the touched sibling scope moved");
}

/// `PathBuf` keys and byte keys agree: a nested fixture with names around
/// the `/` sort boundary folds identically through the resident tree and the
/// from-scratch law-1 recursion (the segmentation is one law, not two).
#[test]
fn nested_names_across_the_sort_boundary_stay_byte_identical() {
    let (_dir, root) = workspace();
    // `a.md` < `a/…` in law-1's component walk but `a.md` > `a/x.md` in raw
    // byte order ('.' = 0x2e < '/' = 0x2f): the two laws sort differently
    // INSIDE their encodings while describing the same member set.
    write(&root.0, "a/x.md", b"x\n");
    write(&root.0, "a.md", b"a\n");
    write(&root.0, "a-b.md", b"ab\n");
    let mut cache = DomainCache::new();
    let served = cache.root(&root).expect("observe");
    let (_, snapshot) = fs::domain_snapshot(&root).expect("snapshot");
    assert_eq!(served, snapshot);
    let mut fresh = DomainCache::new();
    fresh.root(&root).expect("observe");
    assert_eq!(cache.law2_fingerprint(), fresh.law2_fingerprint());
}
