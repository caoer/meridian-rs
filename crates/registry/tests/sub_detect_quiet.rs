//! The watch plane's G11 twin: a quiet detect cycle must run zero full-corpus
//! folds (`fs::fold_count`), and concurrent subscribers must share ONE cycle.
//!
//! Deploy-7 live incident (2026-08-14): 23 subscribed connections each ran a
//! full-tree re-digest per detect cycle — `push_loop → WorkspaceRing::detect →
//! cycle → domain_snapshot → read_and_digest_members` — continuously, because
//! the pre-check re-read every domain byte and the coalescing gate only looks
//! at the COMPLETION time of the last cycle (a fold in flight coalesces
//! nothing). Multi-core pegged with zero ops in flight; face ops starved past
//! their 10s deadline. The prewarm plane had the same defect (G11,
//! `prewarm_quiet.rs`); the ring's detector never got the same cure.
//!
//! This binary must be the only fold-asserting work in its process
//! (`fold_count` is process-global; assert as a difference).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use registry::ring::{DETECT_CADENCE, WorkspaceRing};

/// Serializes the tests in this binary: `fold_count` is process-global, so a
/// difference assertion is only sound while nothing else in the process folds.
static FOLDS: Mutex<()> = Mutex::new(());

fn fold_guard() -> MutexGuard<'static, ()> {
    // A failed sibling test poisons the guard; the count discipline survives.
    FOLDS.lock().unwrap_or_else(PoisonError::into_inner)
}

fn workspace(tmp: &Path, files: &[(&str, &str)]) -> PathBuf {
    let ws = tmp.join("ws");
    for (rel, body) in files {
        let path = ws.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }
    std::fs::canonicalize(&ws).unwrap()
}

/// A second subscriber's prime on a quiet workspace must not re-fold the
/// corpus. Priming is the subscribe-time cycle (cadence ignored), so this is
/// exactly the production case of N connections subscribing to one workspace:
/// the FIRST prime pays the one legitimate fold (baseline adoption); every
/// later prime on an unchanged tree answers from the leaf memo's stat pass.
#[test]
fn a_second_prime_on_a_quiet_workspace_folds_nothing() {
    let _folds = fold_guard();
    let tmp = tempfile::tempdir().unwrap();
    let ws = workspace(tmp.path(), &[("a.md", "# A\n"), ("sub/b.md", "# B\n")]);
    let ws_root = fs::WorkspaceRoot(ws);
    let ring = WorkspaceRing::new(&ws_root);

    let first = ring.prime(&ws_root).expect("baseline prime");
    let folds_before = fs::fold_count();
    for _ in 0..20 {
        let again = ring.prime(&ws_root).expect("quiet prime");
        assert_eq!(again, first, "a quiet tree keeps its root");
    }
    assert_eq!(
        fs::fold_count(),
        folds_before,
        "20 quiet primes must read the corpus ZERO times — the old pre-check \
         re-read and re-digested every member on every cycle"
    );
}

/// A quiet detect cycle past the cadence must not fold the corpus. This is
/// the `push_loop` seat: every subscribed connection calls detect on every
/// 50ms tick, forever, on workspaces that are almost always quiet.
#[test]
fn a_quiet_detect_cycle_folds_nothing() {
    let _folds = fold_guard();
    let tmp = tempfile::tempdir().unwrap();
    let ws = workspace(tmp.path(), &[("a.md", "# A\n")]);
    let ws_root = fs::WorkspaceRoot(ws);
    let ring = WorkspaceRing::new(&ws_root);
    ring.prime(&ws_root).expect("baseline prime");

    let folds_before = fs::fold_count();
    for _ in 0..3 {
        std::thread::sleep(DETECT_CADENCE + Duration::from_millis(50));
        assert!(
            !ring.detect(&ws_root).expect("quiet detect"),
            "a quiet cycle emits nothing"
        );
    }
    assert_eq!(
        fs::fold_count(),
        folds_before,
        "quiet detect cycles must read the corpus ZERO times"
    );
}

/// N subscriber threads detecting one change share ONE cycle: one fold
/// (reconcile's own snapshot), one emitted frame. The incident shape was N
/// concurrent full-tree folds — the coalescing gate keyed on the last cycle's
/// COMPLETION, so every thread that arrived while a fold was in flight
/// started its own.
#[test]
fn concurrent_detects_on_one_change_fold_once_and_emit_one_frame() {
    let _folds = fold_guard();
    let tmp = tempfile::tempdir().unwrap();
    let ws = workspace(tmp.path(), &[("plan.md", "# Goals\n\nship by August\n")]);
    let ws_root = fs::WorkspaceRoot(ws.clone());
    let ring = Arc::new(WorkspaceRing::new(&ws_root));
    ring.prime(&ws_root).expect("baseline prime");

    std::thread::sleep(DETECT_CADENCE + Duration::from_millis(50));
    std::fs::write(ws.join("plan.md"), "# Goals\n\nship by September\n").unwrap();

    let folds_before = fs::fold_count();
    let threads = 8;
    let barrier = Arc::new(Barrier::new(threads));
    let emitted: usize = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                let ring = Arc::clone(&ring);
                let ws_root = fs::WorkspaceRoot(ws.clone());
                let barrier = Arc::clone(&barrier);
                scope.spawn(move || {
                    barrier.wait();
                    usize::from(ring.detect(&ws_root).expect("detect"))
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).sum()
    });

    assert_eq!(emitted, 1, "exactly one thread's cycle classifies the change");
    assert_eq!(
        ring.frames_after(0).len(),
        1,
        "one change is one frame — never one per subscriber"
    );
    assert_eq!(
        fs::fold_count(),
        folds_before + 1,
        "one shared cycle: reconcile's own snapshot is the only fold — the \
         incident shape was one full-tree fold PER subscriber thread"
    );
}
