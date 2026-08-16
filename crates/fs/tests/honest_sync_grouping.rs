//! Contention-triggered grouping of the SHARED durability work.
//!
//! Two properties, and they are the whole contract of `honest_sync_dir`:
//! an uncontended writer syncs and waits for nobody, and contending writers
//! into one directory settle for fewer syncs than they asked for — one round
//! covering everyone queued behind it.

use std::sync::{Arc, Barrier, Mutex, MutexGuard, PoisonError};

use fs::honest_sync_count;
use fs::honest_sync_dir;

/// The counter is process-wide, so a test reading a delta holds this first —
/// the other test in this binary would otherwise inflate it.
static COUNTING: Mutex<()> = Mutex::new(());

fn counting() -> MutexGuard<'static, ()> {
    COUNTING.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The fast path: nobody else is in the directory, so the writer issues its
/// own sync and returns — exactly one, never a wait for a round to form.
#[test]
fn an_uncontended_writer_syncs_once_and_waits_for_nobody() {
    let _counting = counting();
    let dir = tempfile::tempdir().expect("tempdir");

    let before = honest_sync_count();
    honest_sync_dir(dir.path()).expect("sync");
    assert_eq!(
        honest_sync_count() - before,
        1,
        "an uncontended directory sync is one sync, issued by the writer itself"
    );
}

/// Contending writers share rounds: 8 threads asking for 200 directory syncs
/// of ONE directory issue fewer than 200, because a writer that arrives while
/// a round is in flight is covered by the next round instead of running its
/// own.
///
/// Every thread loops on the same directory from one barrier, so a call
/// landing while another is in flight is what the scheduler produces by
/// construction — grouping would have to save nothing at all, across 200
/// calls, for this to fail.
#[test]
fn contending_writers_settle_for_fewer_syncs_than_they_asked_for() {
    let _counting = counting();
    let dir = tempfile::tempdir().expect("tempdir");
    let threads = 8;
    let each = 25;
    let asked = u64::try_from(threads * each).unwrap();

    let barrier = Arc::new(Barrier::new(threads));
    let before = honest_sync_count();
    std::thread::scope(|scope| {
        for _ in 0..threads {
            let barrier = Arc::clone(&barrier);
            let path = dir.path();
            scope.spawn(move || {
                barrier.wait();
                for _ in 0..each {
                    honest_sync_dir(path).expect("grouped sync");
                }
            });
        }
    });
    let issued = honest_sync_count() - before;

    assert!(
        issued < asked,
        "grouping must save syncs under contention: {issued} issued for {asked} calls"
    );
}
