//! The cold entry's gates (merkle-spec §6.4, the cold entry). A workspace's
//! watcher arm and its §6.5 restore are slow on a large root, and neither may
//! park an operation on another workspace. A borrower of the same workspace
//! waits for that entry, and the write door waits within its budget. One
//! entry arms one watcher, and an `unregister` or a budget eviction that lands
//! during an entry gets nothing published back.

use super::*;
use std::sync::mpsc;

/// How long an operation may take while another workspace's entry is
/// parked: far above a healthy operation on a two-member workspace, and the
/// park itself lasts until the test releases it.
const ANSWERS_WITHIN: Duration = Duration::from_secs(10);

/// The write door's budget in these gates.
const DOOR_BUDGET: Duration = Duration::from_millis(200);

/// Releases a parked entry when dropped, so a failing assertion never leaves
/// the parked thread, and the scope joining it, waiting forever.
struct Release(Option<mpsc::Sender<()>>);

impl Release {
    fn now(&mut self) {
        if let Some(release) = self.0.take() {
            let _ = release.send(());
        }
    }
}

impl Drop for Release {
    fn drop(&mut self) {
        self.now();
    }
}

fn registry_in(home: &Path) -> Registry {
    let cache_root = home.join("cache");
    std::fs::create_dir_all(&cache_root).unwrap();
    Registry::new(
        StateStore::new(home.join("state.json")),
        cache_root,
        Vec::new(),
    )
}

/// `home/<name>` with two members, canonical and registered.
fn workspace(reg: &Registry, home: &Path, name: &str) -> PathBuf {
    let ws = home.join(name);
    std::fs::create_dir_all(ws.join("notes")).unwrap();
    std::fs::write(ws.join("a.md"), "# A\n").unwrap();
    std::fs::write(ws.join("notes/b.md"), "# B\n").unwrap();
    let ws = workspace::canonicalize(&ws).unwrap();
    reg.register(&ws);
    ws
}

/// Arm `gate` for `ws`: the arrival announcement and the release handle.
fn arm(gate: &WorkspaceGate, ws: &Path) -> (mpsc::Receiver<()>, Release) {
    let (arrived_tx, arrived) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    *gate.lock().unwrap() = Some((ws.to_path_buf(), arrived_tx, release_rx));
    (arrived, Release(Some(release)))
}

/// A registry restarted over `home` in which workspace `name` has a
/// checkpoint on disk: the first registry warmed it and saved it at
/// shutdown.
fn restarted_with_checkpoint(home: &Path, name: &str) -> (Registry, PathBuf) {
    let ws = {
        let reg = registry_in(home);
        let ws = workspace(&reg, home, name);
        reg.currency_refresh(&ws, DOOR_COOKIE_TIMEOUT).unwrap();
        reg.save_checkpoints();
        ws
    };
    assert!(
        crate::checkpoint::path(&home.join("cache"), &ws).exists(),
        "the shutdown hook persisted the checkpoint"
    );
    let reg = registry_in(home);
    reg.register(&ws);
    (reg, ws)
}

/// What another workspace runs through the memo maps — a borrow of a warm
/// memo, a currency pass on it, and a third workspace's cold entry — run on
/// its own thread. `Ok(true)` when all three answered within
/// [`ANSWERS_WITHIN`] and the currency pass succeeded.
fn others_answer<'scope, 'env>(
    s: &'scope std::thread::Scope<'scope, 'env>,
    reg: &'env Registry,
    warm: &'env Path,
    cold: &'env Path,
) -> Result<bool, mpsc::RecvTimeoutError> {
    let (done_tx, done) = mpsc::channel();
    s.spawn(move || {
        let _ = reg.domain_cache(warm);
        let refreshed = reg.currency_refresh(warm, DOOR_COOKIE_TIMEOUT).is_ok();
        let _ = reg.domain_cache(cold);
        let _ = done_tx.send(refreshed);
    });
    done.recv_timeout(ANSWERS_WITHIN)
}

/// The write door on `ws`, run on its own thread: its outcome within
/// [`ANSWERS_WITHIN`].
fn door<'scope, 'env>(
    s: &'scope std::thread::Scope<'scope, 'env>,
    reg: &'env Registry,
    ws: &'env Path,
) -> Result<Result<(), io::ErrorKind>, mpsc::RecvTimeoutError> {
    let (door_tx, door) = mpsc::channel();
    s.spawn(move || {
        let outcome = reg.domain_cache_within(ws, DOOR_BUDGET).map(|_| ());
        let _ = door_tx.send(outcome.map_err(|e| e.kind()));
    });
    door.recv_timeout(ANSWERS_WITHIN)
}

/// The stall this reproduces: arming a cold root's recursive watch held the
/// registry-wide feed map for minutes, and every root's borrow and currency
/// pass waited behind it. With A's arm parked, B (warm) and C (cold) are
/// served, and A's entry still lands its feed once released.
#[test]
fn a_parked_watcher_arm_on_one_workspace_parks_nothing_on_another() {
    let home = tempfile::tempdir().unwrap();
    let reg = &registry_in(home.path());
    let a = &workspace(reg, home.path(), "a");
    let b = &workspace(reg, home.path(), "b");
    let c = &workspace(reg, home.path(), "c");
    reg.currency_refresh(b, DOOR_COOKIE_TIMEOUT).unwrap();
    assert!(reg.live_feed(b).is_some(), "B is warm, its feed live");

    let (arrived, mut release) = arm(&reg.pause_in_feed_start, a);
    std::thread::scope(|s| {
        let entering = s.spawn(move || reg.domain_cache(a));
        arrived
            .recv()
            .expect("A's cold entry parks with its watcher about to arm");
        let answered = others_answer(s, reg, b, c);
        release.now();
        assert_eq!(
            answered,
            Ok(true),
            "B's borrow and currency pass and C's cold entry must answer while A's watcher \
             arm is parked"
        );
        let _ = entering.join().unwrap();
    });
    assert!(reg.live_feed(a).is_some(), "A's entry lands its feed");
    assert!(
        reg.live_feed(c).is_some(),
        "C's own entry armed its own feed"
    );
}

/// The same-workspace half: while A's arm is parked, the write door on A
/// refuses within its budget instead of waiting behind the arm, and a read
/// borrow of A waits — it never answers with a memo that has no feed behind
/// it. A second entry would have run past the one-shot gate and answered
/// early, so the early silence is also the one-watcher proof; once released,
/// both borrows hold the one published memo, and A has a live feed.
#[test]
fn a_borrower_of_an_entering_workspace_waits_and_the_door_refuses_past_its_budget() {
    let home = tempfile::tempdir().unwrap();
    let reg = &registry_in(home.path());
    let a = &workspace(reg, home.path(), "a");

    let (arrived, mut release) = arm(&reg.pause_in_feed_start, a);
    std::thread::scope(|s| {
        let first = s.spawn(move || reg.domain_cache(a));
        arrived.recv().expect("A's cold entry parks");
        let refused = door(s, reg, a);

        let (read_tx, read) = mpsc::channel();
        let second = s.spawn(move || {
            let memo = reg.domain_cache(a);
            let _ = read_tx.send(());
            memo
        });
        let early = read.recv_timeout(Duration::from_millis(300));
        release.now();

        assert_eq!(
            refused,
            Ok(Err(io::ErrorKind::WouldBlock)),
            "the write door refuses within its budget while A's watcher arm is parked"
        );
        assert!(
            early.is_err(),
            "a read borrow of A must not answer before A's entry lands its feed"
        );
        let first = first.join().unwrap();
        let second = second.join().unwrap();
        assert!(Arc::ptr_eq(&first, &second), "both borrows hold one memo");
        let published = Arc::clone(reg.domain_caches.lock().unwrap().get(a).unwrap());
        assert!(
            Arc::ptr_eq(&first, &published),
            "and it is the published one"
        );
    });
    assert!(reg.live_feed(a).is_some(), "with a live feed behind it");
}

/// The restore half of the stall: a §6.5 restore's pre-serve pass stats
/// every member, and it ran under the registry-wide memo map. With A's
/// restore parked, B and C are served, the write door on A refuses within
/// its budget, and A's entry adopts its checkpoint once released.
#[test]
fn a_parked_checkpoint_restore_on_one_workspace_parks_nothing_on_another() {
    let home = tempfile::tempdir().unwrap();
    let (reg, a) = restarted_with_checkpoint(home.path(), "a");
    let (reg, a) = (&reg, &a);
    let b = &workspace(reg, home.path(), "b");
    let c = &workspace(reg, home.path(), "c");
    reg.currency_refresh(b, DOOR_COOKIE_TIMEOUT).unwrap();

    let (arrived, mut release) = arm(&reg.pause_in_restore, a);
    std::thread::scope(|s| {
        let entering = s.spawn(move || reg.domain_cache(a));
        arrived.recv().expect("A's restore parks");
        let answered = others_answer(s, reg, b, c);
        let refused = door(s, reg, a);
        release.now();

        assert_eq!(
            answered,
            Ok(true),
            "B's borrow and currency pass and C's cold entry must answer while A's restore is \
             parked"
        );
        assert_eq!(
            refused,
            Ok(Err(io::ErrorKind::WouldBlock)),
            "the write door refuses within its budget while A's restore is parked"
        );
        let _ = entering.join().unwrap();
    });
    assert!(
        reg.checkpoint_receipt(a).is_some(),
        "A's entry adopted its checkpoint"
    );
    assert!(reg.live_feed(a).is_some(), "with a live feed behind it");

    // The restored memo reports into the cell its watcher was armed with: an
    // event the watcher accepts advances the memo's own generation.
    let memo = reg.domain_cache(a);
    let before = memo.lock().unwrap().feed_gen().generation();
    std::fs::write(a.join("a.md"), "# A moved\n").unwrap();
    let deadline = Instant::now() + ANSWERS_WITHIN;
    while memo.lock().unwrap().feed_gen().generation() == before && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        memo.lock().unwrap().feed_gen().generation() > before,
        "the watcher armed for A's entry advances the restored memo's feed-generation cell"
    );
}

/// An `unregister` or a budget eviction that lands while A's entry is
/// parked — in its watcher arm or in its restore — runs to completion
/// without waiting on it, and the parked leader publishes nothing back: no
/// feed, no memo, no checkpoint receipt. A borrower that was waiting on the
/// entry gets nothing published back after an `unregister` either; after an
/// eviction the registration survives, so it enters afresh.
#[test]
fn an_unregister_or_eviction_during_an_entry_gets_nothing_published_back() {
    for parked_in in ["arm", "restore"] {
        for removal in ["unregister", "evict"] {
            let case = format!("{removal} during A's {parked_in}");
            let home = tempfile::tempdir().unwrap();
            let (reg, a) = restarted_with_checkpoint(home.path(), "a");
            let (reg, a) = (&reg, &a);
            let gate = if parked_in == "arm" {
                &reg.pause_in_feed_start
            } else {
                &reg.pause_in_restore
            };
            let (arrived, mut release) = arm(gate, a);
            let (leader, waiter) = std::thread::scope(|s| {
                let entering = s.spawn(move || reg.domain_cache(a));
                arrived.recv().expect("A's cold entry parks");
                let (waiting_tx, waiting_rx) = mpsc::channel();
                let waiting = s.spawn(move || {
                    let memo = reg.domain_cache(a);
                    let _ = waiting_tx.send(());
                    memo
                });
                let early = waiting_rx.recv_timeout(Duration::from_millis(300));
                let (gone_tx, gone) = mpsc::channel();
                s.spawn(move || {
                    if removal == "unregister" {
                        reg.unregister(a);
                    } else {
                        reg.evict(std::slice::from_ref(a), true);
                    }
                    let _ = gone_tx.send(());
                });
                let removed = gone.recv_timeout(ANSWERS_WITHIN);
                release.now();
                assert!(
                    early.is_err(),
                    "{case}: the second borrower waits on the entry"
                );
                assert!(
                    removed.is_ok(),
                    "{case}: the removal must not wait behind the parked entry"
                );
                (entering.join().unwrap(), waiting.join().unwrap())
            });
            let published = reg.domain_caches.lock().unwrap().get(a).cloned();
            assert!(
                published.as_ref().is_none_or(|p| !Arc::ptr_eq(p, &leader)),
                "{case}: the cancelled leader's memo is never published"
            );
            if removal == "unregister" {
                assert!(published.is_none(), "{case}: no memo published back");
                assert!(
                    !reg.feeds.lock().unwrap().contains_key(a),
                    "{case}: no feed published back"
                );
                assert!(
                    reg.checkpoint_receipt(a).is_none(),
                    "{case}: no checkpoint receipt published back"
                );
                assert!(
                    !Arc::ptr_eq(&leader, &waiter),
                    "{case}: the waiter holds a private memo of its own"
                );
            } else {
                let published = published.expect("the waiter's own entry published");
                assert!(
                    Arc::ptr_eq(&published, &waiter),
                    "{case}: the waiter entered afresh and published its own entry"
                );
                assert!(reg.live_feed(a).is_some(), "{case}: with its own feed");
            }
            let next = reg.domain_cache(a);
            assert!(
                !Arc::ptr_eq(&leader, &next),
                "{case}: no later borrow sees the cancelled leader's memo"
            );
            assert!(
                reg.live_feed(a).is_some(),
                "{case}: a later borrow has a feed"
            );
        }
    }
}

/// A write door that waited on A's entry, which an eviction then cancelled,
/// refuses within its own call instead of leading the arm it waited on: the
/// door's budget bounds its whole call, never a watcher arm it starts late.
#[test]
fn a_door_that_waited_on_an_evicted_entry_refuses_rather_than_leads() {
    let home = tempfile::tempdir().unwrap();
    let reg = &registry_in(home.path());
    let a = &workspace(reg, home.path(), "a");

    let (arrived, mut release) = arm(&reg.pause_in_feed_start, a);
    let refused = std::thread::scope(|s| {
        let entering = s.spawn(move || reg.domain_cache(a));
        arrived.recv().expect("A's cold entry parks");
        let (door_tx, door) = mpsc::channel();
        s.spawn(move || {
            // A budget far past the park: the door wakes on the cancelled
            // entry, not on its own deadline.
            let outcome = reg.domain_cache_within(a, ANSWERS_WITHIN).map(|_| ());
            let _ = door_tx.send(outcome.map_err(|e| e.kind()));
        });
        std::thread::sleep(Duration::from_millis(300));
        reg.evict(std::slice::from_ref(a), true);
        release.now();
        let refused = door.recv_timeout(ANSWERS_WITHIN);
        let _ = entering.join().unwrap();
        refused
    });
    assert_eq!(
        refused,
        Ok(Err(io::ErrorKind::WouldBlock)),
        "the door refuses after its entry was cancelled"
    );
    assert!(
        !reg.domain_caches.lock().unwrap().contains_key(a),
        "and nothing was published for A"
    );
}

/// A sticky-failed watcher start stays failed through a later cold entry:
/// after an idle reap drops the memo (no live feed covers the gap), the next
/// borrow restores a memo and arms nothing.
#[test]
fn a_failed_start_arms_nothing_on_a_later_entry() {
    let home = tempfile::tempdir().unwrap();
    let reg = &registry_in(home.path());
    let a = &workspace(reg, home.path(), "a");
    let _ = reg.domain_cache(a);
    reg.fail_feed(a);
    reg.reap(u64::MAX, 0);
    assert!(
        !reg.domain_caches.lock().unwrap().contains_key(a),
        "with no live feed the idle reap drops the memo"
    );

    let (arrived, _release) = arm(&reg.pause_in_feed_start, a);
    let (done_tx, done) = mpsc::channel();
    std::thread::scope(|s| {
        s.spawn(move || {
            let _ = reg.domain_cache(a);
            let _ = done_tx.send(());
        });
        assert!(
            done.recv_timeout(ANSWERS_WITHIN).is_ok(),
            "the entry runs without reaching a watcher arm"
        );
    });
    assert!(arrived.try_recv().is_err(), "no watcher arm was attempted");
    assert!(
        matches!(reg.feeds.lock().unwrap().get(a), Some(FeedSlot::Failed)),
        "the failed slot stays sticky"
    );
    assert!(
        reg.domain_caches.lock().unwrap().contains_key(a),
        "and the memo is published without a feed"
    );
}
