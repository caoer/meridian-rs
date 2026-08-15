//! Observation-equivalence gates (card run-observation-unification): the
//! resident [`fs::DomainCache`] serving the run plane's observations must be
//! VALUE-identical to the fresh-walk instruments it replaces on the daemon
//! door — `domain_leaves_memoized` for the locked window, the strict walk of
//! [`fs::guard::StepGuard`] for the bracket — on the same tree, at every
//! state, warm or cold. The cache only moves WHERE listings and digests come
//! from; these gates pin that it never moves what they are.

use std::path::Path;

use fs::digestmemo::DigestMemo;
use fs::guard::{GovernedEdit, GuardError, StepGuard};
use fs::{DomainCache, WorkspaceRoot};

fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, contents).unwrap();
}

/// Rewrite with a forced timestamp tick, so a change is never hidden inside
/// one filesystem timestamp granule (the memo's disclosed blind spot).
fn rewrite(root: &Path, rel: &str, contents: &str) {
    std::thread::sleep(std::time::Duration::from_millis(10));
    write(root, rel, contents);
}

/// A workspace nested one level under the tempdir, so tests have a real
/// out-of-tree location for symlink targets.
fn workspace() -> (tempfile::TempDir, WorkspaceRoot) {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    write(&ws, "a.md", "# A\n");
    write(&ws, "notes/b.md", "# B\n");
    write(&ws, "notes/deep/c.md", "# C\n");
    let root = WorkspaceRoot(std::fs::canonicalize(&ws).unwrap());
    (tmp, root)
}

/// The locked-window observation: the cache-served leaves fold to the same
/// root as the drawer-memo instrument and the byte-derived snapshot, at every
/// state of the tree — add, modify, rename, remove — with the cache staying
/// warm across states while the CLI instrument walks fresh each time.
#[test]
fn cached_domain_leaves_equal_the_drawer_instrument_through_every_change() {
    let (_tmp, root) = workspace();
    let mut cache = DomainCache::new();

    let agree = |cache: &mut DomainCache, at: &str| {
        let cached = cache.domain_leaves(&root).unwrap().root();
        let drawer = fs::domain_leaves_memoized(&root, &mut DigestMemo::new())
            .unwrap()
            .root();
        let bytes = fs::domain_snapshot(&root).unwrap().1;
        assert_eq!(cached, drawer, "cache vs drawer instrument disagree {at}");
        assert_eq!(cached, bytes, "cache vs byte-derived root disagree {at}");
        cached
    };

    let r0 = agree(&mut cache, "at rest");

    rewrite(&root.0, "notes/b.md", "# B changed\n");
    let r1 = agree(&mut cache, "after a modify");
    assert_ne!(r0, r1);

    rewrite(&root.0, "notes/d.md", "# D\n");
    let r2 = agree(&mut cache, "after an add");
    assert_ne!(r1, r2);

    std::thread::sleep(std::time::Duration::from_millis(10));
    std::fs::rename(root.0.join("notes/d.md"), root.0.join("notes/e.md")).unwrap();
    let r3 = agree(&mut cache, "after a rename");
    assert_ne!(r2, r3);

    std::fs::remove_file(root.0.join("notes/e.md")).unwrap();
    let r4 = agree(&mut cache, "after a remove");
    assert_eq!(r1, r4, "removing the renamed addition restores the root");
}

/// The overlay seam the locked window folds through: overlaying the same
/// member digest onto both instruments' leaves yields the same root — the
/// post-phase-1 fold is source-independent.
#[test]
fn cached_leaves_overlay_folds_identically() {
    let (_tmp, root) = workspace();
    let mut cache = DomainCache::new();

    let mut cached = cache.domain_leaves(&root).unwrap();
    let mut drawer = fs::domain_leaves_memoized(&root, &mut DigestMemo::new()).unwrap();

    let digest = model::leaf_digest(b"# receipt page after commit\n");
    cached.overlay(Path::new("receipts/r.md"), digest);
    drawer.overlay(Path::new("receipts/r.md"), digest);
    assert_eq!(cached.root(), drawer.root(), "overlaid folds diverge");

    // A path outside the hash domain is ignored by both.
    cached.overlay(Path::new(".meridian/x.md"), digest);
    drawer.overlay(Path::new(".meridian/x.md"), digest);
    assert_eq!(cached.root(), drawer.root());
}

/// A clean bracket: open/close from the cache renders the same verified
/// post-step root as the fresh strict walk, over the same governed edits.
#[test]
fn cached_bracket_clean_close_matches_fresh_walk() {
    let (_tmp, root) = workspace();
    let mut cache = DomainCache::new();

    let fresh = StepGuard::open(&root).unwrap();
    let cached = StepGuard::open_cached(&root, &mut cache).unwrap();
    assert_eq!(
        fresh.pre_root(),
        cached.pre_root(),
        "pre-step baselines diverge"
    );

    rewrite(&root.0, "notes/b.md", "# B v2\n");
    let edits = [GovernedEdit {
        path: "notes/b.md".to_owned(),
        bytes: b"# B v2\n".to_vec(),
    }];

    let fresh_root = fresh.close(&edits).unwrap();
    let cached_root = cached.close_cached(&edits, &mut cache).unwrap();
    assert_eq!(fresh_root, cached_root, "verified post roots diverge");
}

/// The zero-descriptor cheat is named identically from both sources: same
/// delta lists, same wording (the residual's `Display` is the report line).
#[test]
fn cached_bracket_names_the_same_out_of_band_delta() {
    let (_tmp, root) = workspace();
    let mut cache = DomainCache::new();

    let fresh = StepGuard::open(&root).unwrap();
    let cached = StepGuard::open_cached(&root, &mut cache).unwrap();

    rewrite(&root.0, "notes/b.md", "sneaky rewrite\n");
    rewrite(&root.0, "notes/new.md", "undeclared\n");

    let fresh_err = fresh.close(&[]).unwrap_err();
    let cached_err = cached.close_cached(&[], &mut cache).unwrap_err();
    match (&fresh_err, &cached_err) {
        (GuardError::OutOfBand(a), GuardError::OutOfBand(b)) => {
            assert_eq!(a, b, "residual deltas diverge");
            assert_eq!(a.to_string(), b.to_string());
        }
        other => panic!("expected matching out-of-band verdicts, got {other:?}"),
    }
}

/// The symlink refusal keeps its count-plus-sorted-first shape from the
/// cache — including from a WARM cache whose dir memo predates the links
/// (their creation moved the directory's own timestamps, so the unmoved
/// listing law re-enumerates and sees them).
#[cfg(unix)]
#[test]
fn cached_bracket_refuses_symlinks_like_the_fresh_walk() {
    let (tmp, root) = workspace();
    let mut cache = DomainCache::new();

    // Warm the cache before any link exists.
    let _ = StepGuard::open_cached(&root, &mut cache).unwrap();

    let secret = tmp.path().join("secret.md");
    std::fs::write(&secret, "out-of-tree\n").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    std::os::unix::fs::symlink(&secret, root.0.join("notes/zz-linked.md")).unwrap();
    std::os::unix::fs::symlink(&secret, root.0.join("linked.md")).unwrap();

    let fresh_err = StepGuard::open(&root).unwrap_err();
    let cached_err = StepGuard::open_cached(&root, &mut cache).unwrap_err();
    match (&fresh_err, &cached_err) {
        (
            GuardError::Symlink { count: fc, first: ff },
            GuardError::Symlink { count: cc, first: cf },
        ) => {
            assert_eq!((fc, ff), (cc, cf), "symlink refusals diverge");
            assert_eq!(*cc, 2);
            assert_eq!(cf, "linked.md", "first offender is sorted, not walk-order");
        }
        other => panic!("expected matching symlink refusals, got {other:?}"),
    }
}

/// The domain's own ignore rules carve a symlink out of detection from both
/// sources (the sessions-root shape: the ignored entry IS the link).
#[cfg(unix)]
#[test]
fn cached_bracket_keeps_the_ignored_symlink_carve_out() {
    let (tmp, root) = workspace();
    write(
        &root.0,
        "meridian/domain.md",
        "---\nversion: 1\nignore:\n  - \"scratch*/\"\n---\n\n# Domain\n",
    );
    let target = tmp.path().join("elsewhere");
    std::fs::create_dir_all(&target).unwrap();
    std::os::unix::fs::symlink(&target, root.0.join("scratch-1")).unwrap();

    let fresh = StepGuard::open(&root).expect("fresh walk skips the ignored link");
    let mut cache = DomainCache::new();
    let cached = StepGuard::open_cached(&root, &mut cache)
        .expect("cached walk skips the ignored link identically");
    assert_eq!(fresh.pre_root(), cached.pre_root());
}

/// A mid-window config change refuses from the cache exactly as fresh — the
/// config bracket is read from disk on both paths, never from the memo.
#[test]
fn cached_bracket_refuses_a_mid_window_config_change() {
    let (_tmp, root) = workspace();
    let mut cache = DomainCache::new();
    let cached = StepGuard::open_cached(&root, &mut cache).unwrap();

    write(&root.0, "mdfs_config.yaml", "ignore:\n  - \"notes/**\"\n");

    assert!(matches!(
        cached.close_cached(&[], &mut cache),
        Err(GuardError::ConfigChanged)
    ));
}

/// The cost claim behind the unification, as an assertable invariant rather
/// than a timing: a second guarded observation over an unchanged tree
/// re-enumerates NOTHING and re-reads NOTHING — listings from the dir memo,
/// digests from the leaf memo — where the fresh strict walk pays every
/// `read_dir` again each time.
#[test]
fn a_warm_cache_observes_an_unchanged_tree_without_listing_or_reading() {
    let (_tmp, root) = workspace();
    let mut cache = DomainCache::new();

    let g1 = StepGuard::open_cached(&root, &mut cache).unwrap();
    let after_cold = (cache.listings(), cache.leaves_read());
    g1.close_cached(&[], &mut cache).unwrap();

    let g2 = StepGuard::open_cached(&root, &mut cache).unwrap();
    g2.close_cached(&[], &mut cache).unwrap();
    assert_eq!(
        (cache.listings(), cache.leaves_read()),
        after_cold,
        "a warm observation over an unchanged tree must be stat-only"
    );
}
