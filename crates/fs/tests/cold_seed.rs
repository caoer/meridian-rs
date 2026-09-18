//! [`fs::DomainCache::cold_snapshot`] — the cold build's ONE observation
//! seeding the resident memo (merkle-spec §6.5, "the cold build seeds the
//! memo").
//!
//! The daemon's cold start used to read the corpus twice: the §6.2 floor pass
//! (`DomainCache::root` over an empty memo, every member read) to answer
//! currency, then `domain_snapshot_with_leaves` for the bytes the parse
//! needs. The seed is that snapshot with the memo filled as a side effect,
//! so the currency question that follows is the stat floor. These gates pin
//! the three things that make it safe: it IS the snapshot (same files, same
//! leaves, same root), it is the cold door ONLY (a memo with rows refuses),
//! and what it seeds behaves like observed rows — untrusted for a guard
//! until one pass covers them, stat-only over a settled tree, a mover
//! re-read.

use std::path::Path;
use std::sync::Mutex;

use fs::stable::{Calibration, GuardCurrency};
use fs::{DomainCache, WorkspaceRoot};

/// `fs::fold_count` is process-global and every test here folds: each takes
/// this lock so the deltas the assertions read are their own.
static FOLDS: Mutex<()> = Mutex::new(());

fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, contents).unwrap();
}

/// A nested corpus with unordered siblings, out-of-domain files and enough
/// members that the seed's zip of identities with read rows is non-trivial.
fn corpus() -> (tempfile::TempDir, WorkspaceRoot, usize) {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    write(&ws, "MERIDIAN.md", "# root\n");
    write(&ws, "notes/plan.md", "# Plan\n");
    write(&ws, "notes/b.md", "# b\n");
    write(&ws, "notes/a.md", "# a\n");
    write(&ws, "year=2026/month=08/deep/leaf.md", "# leaf\n");
    write(&ws, "year=2026/month=07/deep/leaf.md", "# other\n");
    write(&ws, "mdfs_config.yaml", "version: 0\nignore: []\n");
    write(&ws, ".github/README.md", "# CI notes\n");
    let root = WorkspaceRoot(std::fs::canonicalize(&ws).unwrap());
    (tmp, root, 6)
}

/// Let the backend's stamp quantum pass since the last write, sized from a
/// measured calibration, so the rows the next pass records are not racy
/// against their own watermark.
fn settle(cache: &DomainCache) {
    let Calibration::Measured { granule_ns } = cache
        .calibration()
        .expect("probed on first observe")
        .clone()
    else {
        panic!("a writable tempdir calibrates");
    };
    std::thread::sleep(std::time::Duration::from_nanos(granule_ns * 2 + 2_000_000));
}

/// The seed IS `domain_snapshot_with_leaves`: same files (same order, same
/// bytes), same leaf set, same root — and the memo comes out of it with a
/// baseline, every member read exactly once, every member stat'ed once.
#[test]
fn cold_snapshot_is_the_snapshot_with_the_memo_seeded() {
    let _folds = FOLDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (_tmp, root, n) = corpus();
    let mut cache = DomainCache::new();
    assert!(!cache.has_baseline(), "an empty memo has no baseline");
    assert!(matches!(
        cache.guard_currency(),
        GuardCurrency::Untrusted { .. }
    ));

    let folds_before = fs::fold_count();
    let (files, leaves, seeded) = cache.cold_snapshot(&root).unwrap();
    assert_eq!(
        fs::fold_count() - folds_before,
        1,
        "the seed is one full fold"
    );

    let (s_files, s_leaves, s_root) = fs::domain_snapshot_with_leaves(&root).unwrap();
    assert_eq!(seeded, s_root, "the seed's root is the snapshot's");
    assert_eq!(leaves, s_leaves, "the seed's leaf set is the snapshot's");
    assert_eq!(files, s_files, "the seed hands back the snapshot's bytes");
    assert_eq!(files.len(), n);

    assert!(cache.has_baseline(), "the seed is a baseline");
    assert_eq!(cache.leaves_read(), n as u64, "every member read once");
    assert_eq!(cache.member_stats(), n as u64, "every member stat'ed once");
    assert_eq!(cache.sweeps(), 1, "the seed is one sweep");
    assert_eq!(
        cache.leaf_digests(),
        s_leaves,
        "the memo's rows are exactly the snapshot's leaves"
    );
    assert_eq!(
        cache.served_cached(),
        Some(&seeded),
        "the seed's fold is the served value the next quiet pass hands back"
    );
}

/// The cold door only: a memo that already holds a baseline — seeded, or
/// observed — refuses a second seed, and refuses it WITHOUT touching its
/// rows (the refusal is at entry, before any I/O).
#[test]
fn a_memo_with_a_baseline_refuses_the_cold_door() {
    let _folds = FOLDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (_tmp, root, n) = corpus();

    let mut seeded = DomainCache::new();
    seeded.cold_snapshot(&root).unwrap();
    let folds_before = fs::fold_count();
    let err = seeded
        .cold_snapshot(&root)
        .expect_err("a seeded memo refuses a second seed");
    assert!(
        err.to_string().contains("already holds a baseline"),
        "the refusal names the reason: {err}"
    );
    assert_eq!(
        fs::fold_count(),
        folds_before,
        "a refused seed folds nothing"
    );
    assert_eq!(
        seeded.leaves_read(),
        n as u64,
        "a refused seed reads nothing"
    );
    assert_eq!(seeded.sweeps(), 1, "a refused seed sweeps nothing");

    let mut observed = DomainCache::new();
    observed.root(&root).unwrap();
    assert!(
        observed.cold_snapshot(&root).is_err(),
        "a memo with an observed baseline is not cold either"
    );
}

/// What the seed leaves behind behaves like observed rows on the currency
/// path: over a SETTLED tree the very next `root` is stat-only (no member
/// read, one sweep, every member stat'ed) and hands back the same root — the
/// daemon's post-build currency question, answered without a second read of
/// the corpus. And the seed's guard posture is the checkpoint's (§6.5):
/// untrusted until that one pass has covered the rows.
#[test]
fn the_seeded_memo_answers_the_next_currency_question_without_reading() {
    let _folds = FOLDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (_tmp, root, n) = corpus();

    // Learn the granule from a throwaway memo's calibration probe, then let
    // the writes above settle past it, so the seed records under a watermark
    // that clears every row.
    let mut probe = DomainCache::new();
    probe.root(&root).unwrap();
    settle(&probe);

    let mut cache = DomainCache::new();
    let (_, _, seeded) = cache.cold_snapshot(&root).unwrap();
    assert!(
        matches!(cache.guard_currency(), GuardCurrency::Untrusted { reason } if reason.contains("no observation has landed since")),
        "seeded rows are hypotheses for a guard until one pass covers them"
    );

    let folds_before = fs::fold_count();
    let served = cache.root(&root).unwrap();
    assert_eq!(served, seeded, "the stat floor serves the seeded root");
    assert_eq!(
        fs::fold_count(),
        folds_before,
        "the floor pass is not a full fold"
    );
    assert_eq!(
        cache.leaves_read(),
        n as u64,
        "the currency pass after the seed read NO member — the seed's rows served"
    );
    assert_eq!(
        cache.member_stats(),
        2 * n as u64,
        "one stat per member per pass"
    );
    assert_eq!(cache.sweeps(), 2);
    assert_eq!(
        cache.guard_currency(),
        GuardCurrency::Trusted,
        "one completed pass over the seeded rows makes them evidence"
    );

    // And it stays that way: the steady state is stat-only.
    cache.root(&root).unwrap();
    assert_eq!(cache.leaves_read(), n as u64);
}

/// A member that moves after the seed is re-read by the next pass — and only
/// it: the seed's rows are keyed by the members' pre-read identities, so a
/// changed identity misses exactly as an observed row's would, and the root
/// tracks the change (value-identical to the flat build over the new tree).
#[test]
fn a_mover_after_the_seed_is_re_read_and_moves_the_root() {
    let _folds = FOLDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (_tmp, root, n) = corpus();
    let mut probe = DomainCache::new();
    probe.root(&root).unwrap();
    settle(&probe);

    let mut cache = DomainCache::new();
    let (_, _, seeded) = cache.cold_snapshot(&root).unwrap();

    std::thread::sleep(std::time::Duration::from_millis(10));
    write(&root.0, "notes/plan.md", "# Plan, revised\n");

    let moved = cache.root(&root).unwrap();
    assert_ne!(moved, seeded, "an edited member moves the served root");
    assert_eq!(
        moved,
        fs::domain_snapshot(&root).unwrap().1,
        "the served root after the mover is the flat build's"
    );
    assert_eq!(
        cache.leaves_read(),
        n as u64 + 1,
        "exactly the mover was re-read; every seeded row served"
    );
}

/// The empty domain seeds too: no members, a baseline, the degenerate root —
/// the same token the flat build mints, not a short circuit.
#[test]
fn cold_snapshot_seeds_an_empty_domain() {
    let _folds = FOLDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "mdfs_config.yaml", "version: 0\nignore: []\n");
    let root = WorkspaceRoot(std::fs::canonicalize(tmp.path()).unwrap());
    let mut cache = DomainCache::new();

    let (files, leaves, seeded) = cache.cold_snapshot(&root).unwrap();
    assert!(files.is_empty());
    assert!(leaves.is_empty());
    assert_eq!(seeded, fs::domain_snapshot(&root).unwrap().1);
    assert!(
        cache.has_baseline(),
        "an observed empty domain is a baseline"
    );
    assert_eq!(cache.root(&root).unwrap(), seeded);
    assert_eq!(cache.leaves_read(), 0);
}
