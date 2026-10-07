use super::*;
use crate::raw_store::RawStore;

#[test]
fn shed_rehydrates_the_selected_version_after_source_and_store_drop() {
    let parent = tempfile::tempdir().unwrap();
    let path = parent.path().join("member.md");
    let raw = b"# Version A\n\n[[retained]]\n";
    std::fs::write(&path, raw).unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let handle = store.retain(raw).unwrap();
    let resident = Resident::new(Arc::new(handle));
    let identity = resident.identity();
    let pin = resident.pin(Access::Read).expect("load retained version");
    let expected = pin.document().unwrap().root.clone();
    assert_eq!(pin.bytes().as_bytes(), raw);
    assert!(resident.eviction_ticket().is_none());
    drop(pin);
    let ticket = resident
        .eviction_ticket()
        .expect("unpinned body is evictable");
    assert!(resident.try_shed(&ticket));
    std::fs::write(&path, b"# Version B\n").unwrap();
    std::fs::remove_file(&path).unwrap();
    drop(store);
    let pin = resident
        .pin(Access::Read)
        .expect("rehydrate from retained backing");
    assert_eq!(resident.identity(), identity);
    assert_eq!(pin.bytes().as_bytes(), raw);
    assert_eq!(pin.document().unwrap().root, expected);
}

use crate::raw_store::RawHandle;
use fs::hydration::HydrationError;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(10);

#[derive(Debug, Default)]
struct Gate {
    state: Mutex<(bool, bool)>, // entered, released
    changed: Condvar,
}

impl Gate {
    fn enter(&self) {
        let mut state = self.state.lock().unwrap();
        state.0 = true;
        self.changed.notify_all();
        while !state.1 {
            state = self.changed.wait(state).unwrap();
        }
    }

    fn entered(&self) -> bool {
        let state = self.state.lock().unwrap();
        let (state, _) = self
            .changed
            .wait_timeout_while(state, WAIT, |s| !s.0)
            .unwrap();
        state.0
    }

    fn release(&self) {
        self.state.lock().unwrap().1 = true;
        self.changed.notify_all();
    }
}

#[derive(Debug)]
struct Controlled {
    handle: RawHandle,
    calls: AtomicUsize,
    gate: Gate,
    first: First,
}

#[derive(Debug, Clone, Copy)]
enum First {
    Success,
    Fault,
    Panic,
    WrongIdentity,
}

impl ByteSource for Controlled {
    fn identity(&self) -> ByteIdentity {
        self.handle.identity()
    }

    fn read(&self) -> io::Result<SharedBytes> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            self.gate.enter();
            match self.first {
                First::Success => (),
                First::Fault => return Err(io::Error::other("retained source fault")),
                First::Panic => panic!("injected loader unwind"),
                First::WrongIdentity => return Ok(SharedBytes::from("wrong".to_owned())),
            }
        }
        ByteSource::read(&self.handle)
    }
}

fn controlled(first: First) -> (tempfile::TempDir, Arc<Controlled>, Resident) {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let source = Arc::new(Controlled {
        handle: store.retain(b"# Retained\n\nbody\n").unwrap(),
        calls: AtomicUsize::new(0),
        gate: Gate::default(),
        first,
    });
    let resident = Resident::new(source.clone());
    (parent, source, resident)
}

// Backstops fail rather than hanging the suite. Release the loader before
// joining or asserting any observation, including an unexpected slot-lock hold.
type Wave = (
    tempfile::TempDir,
    Arc<Controlled>,
    Resident,
    Vec<Result<BodyPin, Arc<LoadError>>>,
);

fn wave(first: First) -> Wave {
    let (parent, source, resident) = controlled(first);
    let leader = {
        let resident = resident.clone();
        thread::spawn(move || resident.pin(Access::Read))
    };
    let entered = source.gate.entered();
    let followers: Vec<_> = (0..4)
        .map(|_| {
            let resident = resident.clone();
            thread::spawn(move || resident.pin(Access::Read))
        })
        .collect();
    let (tx, rx) = mpsc::channel();
    let observer = {
        let resident = resident.clone();
        thread::spawn(move || {
            let deadline = Instant::now() + WAIT;
            loop {
                let snapshot = resident.snapshot();
                if snapshot.waiters == 4 {
                    tx.send(
                        snapshot.phase == Phase::Loading && resident.eviction_ticket().is_none(),
                    )
                    .unwrap();
                    break;
                }
                if Instant::now() >= deadline {
                    let _ = tx.send(false);
                    break;
                }
                thread::yield_now();
            }
        })
    };
    let ready = rx.recv_timeout(WAIT);
    source.gate.release();
    let leader = leader.join();
    let mut results: Vec<_> = followers.into_iter().map(|j| j.join().unwrap()).collect();
    observer.join().unwrap();
    assert!(entered, "loader did not enter its source");
    assert!(ready.unwrap(), "waiters or lock-free observation missing");
    match first {
        First::Panic => assert!(leader.is_err()),
        _ => results.push(leader.unwrap()),
    }
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    (parent, source, resident, results)
}

#[test]
fn concurrent_readers_share_one_load_and_one_body() {
    let (_parent, _, resident, pins) = wave(First::Success);
    let pins: Vec<_> = pins.into_iter().map(Result::unwrap).collect();
    let doc = pins[0].document().unwrap();
    for pin in &pins {
        assert!(std::ptr::eq(pin.document().unwrap(), doc));
        assert_eq!(pin.bytes().allocation_id(), pins[0].bytes().allocation_id());
    }
    let clone = pins[0].clone();
    drop(pins);
    assert!(resident.eviction_ticket().is_none());
    drop(clone);
    assert!(resident.try_shed(&resident.eviction_ticket().unwrap()));
}

#[test]
fn failed_wave_shares_error_and_next_wave_can_retry() {
    let (_parent, source, resident, results) = wave(First::Fault);
    let errors: Vec<_> = results.into_iter().map(Result::unwrap_err).collect();
    for error in &errors {
        assert!(Arc::ptr_eq(error, &errors[0]));
        assert!(
            matches!(error.as_ref(), LoadError::Hydration(HydrationError::Source(e)) if e.to_string() == "retained source fault")
        );
    }
    assert_eq!(resident.snapshot().phase, Phase::Cold);
    assert_eq!(
        resident.pin(Access::Read).unwrap().bytes().as_bytes(),
        b"# Retained\n\nbody\n"
    );
    assert_eq!(source.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn unwinding_loader_wakes_followers_and_releases_loading_state() {
    let (_parent, _, resident, results) = wave(First::Panic);
    let errors: Vec<_> = results.into_iter().map(Result::unwrap_err).collect();
    for error in &errors {
        assert!(Arc::ptr_eq(error, &errors[0]));
        assert!(matches!(error.as_ref(), LoadError::Abandoned));
    }
    assert_eq!(resident.snapshot().phase, Phase::Cold);
}

#[test]
fn faults_and_unwinds_allow_successful_retry_with_backing_intact() {
    for first in [First::Fault, First::Panic, First::WrongIdentity] {
        let (_parent, source, resident) = controlled(first);
        source.gate.release();
        let attempt =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| resident.pin(Access::Read)));
        match first {
            First::Panic => assert!(attempt.is_err()),
            First::WrongIdentity => assert!(matches!(
                attempt.unwrap().unwrap_err().as_ref(),
                LoadError::Hydration(HydrationError::IdentityMismatch)
            )),
            _ => assert!(attempt.unwrap().is_err()),
        }
        assert_eq!(resident.snapshot().phase, Phase::Cold);
        let pin = resident.pin(Access::Read).unwrap();
        assert_eq!(pin.bytes().as_bytes(), b"# Retained\n\nbody\n");
        assert_eq!(source.calls.load(Ordering::SeqCst), 2);
    }
}

#[test]
fn read_invalidates_ticket_but_scan_only_protects_while_pinned() {
    let (_parent, source, resident) = controlled(First::Success);
    source.gate.release();
    drop(resident.pin(Access::Read).unwrap());
    let stale = resident.eviction_ticket().unwrap();
    drop(resident.pin(Access::Read).unwrap());
    assert!(!resident.try_shed(&stale));
    let ticket = resident.eviction_ticket().unwrap();
    let scan = resident.pin(Access::Scan).unwrap();
    assert!(!resident.try_shed(&ticket));
    let clone = scan.clone();
    drop(scan);
    assert!(!resident.try_shed(&ticket));
    drop(clone);
    assert!(resident.try_shed(&ticket));
    drop(resident.pin(Access::Scan).unwrap());
    assert!(
        !resident.try_shed(&ticket),
        "old load ticket survived rehydration"
    );
    let other = Resident::new(source);
    drop(other.pin(Access::Read).unwrap());
    assert!(!other.try_shed(&resident.eviction_ticket().unwrap()));
}

#[test]
fn escaped_raw_stays_charged_and_is_reused_until_its_last_owner_drops() {
    let (_parent, source, resident) = controlled(First::Success);
    source.gate.release();
    let pin = resident.pin(Access::Read).unwrap();
    let raw = pin.document().unwrap().raw.clone();
    let allocation = pin.bytes().allocation_id();
    let charge = pin.bytes().heap_bytes();
    assert!(resident.snapshot().tree_bytes > 0);
    drop(pin);
    assert!(resident.try_shed(&resident.eviction_ticket().unwrap()));
    let snapshot = resident.snapshot();
    assert_eq!(snapshot.phase, Phase::Cold);
    assert_eq!(snapshot.tree_bytes, 0);
    assert_eq!(snapshot.raw_bytes, charge);
    let pin = resident.pin(Access::Read).unwrap();
    assert_eq!(pin.bytes().allocation_id(), allocation);
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    drop(pin);
    assert!(resident.try_shed(&resident.eviction_ticket().unwrap()));
    drop(raw);
    assert_eq!(resident.snapshot().raw_bytes, 0);
    drop(resident.pin(Access::Read).unwrap());
    assert_eq!(source.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn invalid_utf8_and_empty_members_preserve_their_exact_bytes() {
    for bytes in [vec![0xff, 0, 0xfe], Vec::new()] {
        let parent = tempfile::tempdir().unwrap();
        let store = RawStore::new(parent.path()).unwrap();
        let resident = Resident::new(Arc::new(store.retain(&bytes).unwrap()));
        for _ in 0..2 {
            let pin = resident.pin(Access::Read).unwrap();
            assert_eq!(pin.bytes().as_bytes(), bytes);
            assert_eq!(pin.document().is_ok(), bytes.is_empty());
            if !bytes.is_empty() {
                assert!(matches!(pin.document(), Err(HydrationError::InvalidUtf8)));
            }
            drop(pin);
            assert!(resident.try_shed(&resident.eviction_ticket().unwrap()));
        }
    }
}

#[test]
fn pin_retains_descriptor_and_backing_after_all_resident_handles_drop() {
    let (_parent, source, resident) = controlled(First::Success);
    source.gate.release();
    let weak = Arc::downgrade(&source);
    let pin = resident.pin(Access::Read).unwrap();
    drop(resident);
    drop(source);
    assert!(weak.upgrade().is_some());
    assert_eq!(pin.bytes().as_bytes(), b"# Retained\n\nbody\n");
    drop(pin);
    assert!(weak.upgrade().is_none());
}

#[test]
fn truncated_backing_is_an_explicit_fault_and_keeps_the_descriptor() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let resident = Resident::new(Arc::new(store.retain(b"retained").unwrap()));
    let identity = resident.identity();
    drop(resident.pin(Access::Read).unwrap());
    assert!(resident.try_shed(&resident.eviction_ticket().unwrap()));
    for dir in std::fs::read_dir(parent.path()).unwrap() {
        for file in std::fs::read_dir(dir.unwrap().path()).unwrap() {
            // Truncate the actual retained inode. Unlinking only its name
            // leaves the RawHandle's open descriptor readable.
            std::fs::write(file.unwrap().path(), []).unwrap();
        }
    }
    assert!(
        matches!(resident.pin(Access::Read).unwrap_err().as_ref(), LoadError::Hydration(HydrationError::Source(e)) if e.kind() == io::ErrorKind::UnexpectedEof)
    );
    assert_eq!(resident.identity(), identity);
    assert_eq!(resident.snapshot().phase, Phase::Cold);
    assert_eq!(resident.snapshot().raw_bytes, 0);
}

#[test]
fn revision_exhaustion_cannot_make_an_old_ticket_current_again() {
    let (_parent, source, resident) = controlled(First::Success);
    source.gate.release();
    drop(resident.pin(Access::Read).unwrap());
    let ticket = resident.eviction_ticket().unwrap();
    lock(&resident.0.state).revision = u64::MAX;
    assert!(matches!(
        resident.pin(Access::Read).unwrap_err().as_ref(),
        LoadError::RevisionExhausted
    ));
    assert!(!resident.try_shed(&ticket));
    let final_ticket = resident.eviction_ticket().unwrap();
    assert!(resident.try_shed(&final_ticket));
    assert!(matches!(
        resident.pin(Access::Scan).unwrap_err().as_ref(),
        LoadError::RevisionExhausted
    ));
    assert_eq!(resident.snapshot().phase, Phase::Cold);
}

// Hold a registered waiter just before its condvar wait, allowing the leader
// and a subsequent caller to run first. This fixes the scheduling point that
// an ordinary thread stress test cannot force deterministically.
fn delayed_waiter(resident: &Resident) -> Arc<Flight> {
    let state = lock(&resident.0.state);
    let StateBody::Loading(flight) = &state.body else {
        panic!("source gate must hold the loading wave");
    };
    lock(&flight.state).waiters += 1;
    flight.clone()
}

#[test]
fn retry_cannot_replace_a_delayed_waiters_original_error() {
    let (_parent, source, resident) = controlled(First::Fault);
    let leader = {
        let resident = resident.clone();
        thread::spawn(move || resident.pin(Access::Read))
    };
    if !source.gate.entered() {
        source.gate.release();
        let _ = leader.join();
        panic!("loader did not enter source");
    }
    let waiter = delayed_waiter(&resident);
    source.gate.release();
    let original = leader.join().unwrap().unwrap_err();
    let retried = resident.pin(Access::Read).unwrap();
    let delayed = waiter.wait().unwrap_err();
    assert!(Arc::ptr_eq(&original, &delayed));
    assert_eq!(retried.bytes().as_bytes(), b"# Retained\n\nbody\n");
    assert_eq!(resident.snapshot().phase, Phase::Hot);
}

#[test]
fn completed_wave_protects_body_until_delayed_waiter_receives_it() {
    let (_parent, source, resident) = controlled(First::Success);
    let leader = {
        let resident = resident.clone();
        thread::spawn(move || resident.pin(Access::Read))
    };
    if !source.gate.entered() {
        source.gate.release();
        let _ = leader.join();
        panic!("loader did not enter source");
    }
    let waiter = delayed_waiter(&resident);
    source.gate.release();
    drop(leader.join().unwrap().unwrap());
    assert!(resident.eviction_ticket().is_none());
    let body = waiter.wait().unwrap();
    drop(waiter);
    assert!(resident.eviction_ticket().is_none());
    assert_eq!(body.bytes.as_bytes(), b"# Retained\n\nbody\n");
    drop(body);
    assert!(resident.try_shed(&resident.eviction_ticket().unwrap()));
}

/// Two censuses overlap: the one that took the later ticket keeps the floor
/// whichever finishes last (merkle-spec §6.4, the host floor).
#[test]
fn an_older_census_never_overwrites_a_newer_floor() {
    let backing = tempfile::tempdir().unwrap();
    let host = Host::new(
        controller::Controller::new(0),
        RawStore::new(backing.path()).unwrap(),
        1,
    );
    let older = host.ticket();
    let newer = host.ticket();
    host.set_floor(newer, 200);
    host.set_floor(older, 100);
    assert_eq!(host.floor(), 200, "the older census left the newer floor");
    let charged = host.controller.report().floor;
    host.set_floor(host.ticket(), 300);
    assert_eq!(
        host.controller.report().floor,
        charged + 100,
        "the floor is one lease, moved, not stacked"
    );
}

/// A census that measured before an eviction and returns after the
/// eviction's own census took the later ticket does not set the floor: the
/// ticket is taken before measuring (merkle-spec §6.4, the host floor).
#[test]
fn a_census_overtaken_by_an_evictions_census_leaves_the_later_floor() {
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(Host::new(
        controller::Controller::new(0),
        RawStore::new(&dir.path().join("backing")).unwrap(),
        1 << 20,
    ));
    let mut config = crate::Config::for_cache_root(dir.path().join("cache"));
    config.socket_path = dir.path().join("daemon.sock");
    config.state_path = dir.path().join("state.json");
    config.residency = Some(Arc::clone(&host));
    let registry = Arc::new(crate::in_process_registry(&config).unwrap());
    let roots: Vec<std::path::PathBuf> = ["a", "b"]
        .iter()
        .map(|name| {
            let ws = dir.path().join(name);
            std::fs::create_dir_all(&ws).unwrap();
            for n in 0..8 {
                std::fs::write(ws.join(format!("m{n}.md")), format!("# M{n}\n\n[[m0]]\n")).unwrap();
            }
            let ws = std::fs::canonicalize(ws).unwrap();
            registry.register(&ws);
            registry.warm_or_build(&ws).unwrap();
            ws
        })
        .collect();
    let warm = host.floor();

    let (arrived, at_gate) = mpsc::channel();
    let (release, released) = mpsc::channel();
    *registry.pause_after_census.lock().unwrap() = Some((arrived, released));
    let overtaken = {
        let registry = Arc::clone(&registry);
        thread::spawn(move || registry.charge_floor())
    };
    at_gate.recv().expect("the census reached its gate");

    assert!(registry.unregister(&roots[1]));
    let evicted = host.floor();
    assert!(
        evicted < warm,
        "the eviction's census dropped the floor: {evicted} >= {warm}"
    );
    release.send(()).expect("the census parked on its gate");
    overtaken.join().unwrap();
    assert_eq!(
        host.floor(),
        evicted,
        "the overtaken census measured before the eviction and must not set the floor"
    );
}

type PauseGate = Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>;

/// Arm a one-shot test gate: (the pass arrived, release it).
fn arm(gate: &PauseGate) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
    let (arrived, at_gate) = mpsc::channel();
    let (release, released) = mpsc::channel();
    *gate.lock().unwrap() = Some((arrived, released));
    (at_gate, release)
}

/// A registry built through an unlimited host over one warm workspace of
/// `members` pages per name.
fn hosted_with(
    dir: &std::path::Path,
    names: &[&str],
    members: usize,
) -> (Arc<Host>, Arc<crate::Registry>, Vec<std::path::PathBuf>) {
    let host = Arc::new(Host::new(
        controller::Controller::new(0),
        RawStore::new(&dir.join("backing")).unwrap(),
        1 << 20,
    ));
    let mut config = crate::Config::for_cache_root(dir.join("cache"));
    config.socket_path = dir.join("daemon.sock");
    config.state_path = dir.join("state.json");
    config.residency = Some(Arc::clone(&host));
    let registry = Arc::new(crate::in_process_registry(&config).unwrap());
    let roots = names
        .iter()
        .map(|name| {
            let ws = dir.join(name);
            std::fs::create_dir_all(&ws).unwrap();
            for n in 0..members {
                let page = format!("# M{n}\n\nlinks [[m0]] and text ^b{n}\n");
                std::fs::write(ws.join(format!("m{n}.md")), page).unwrap();
            }
            let ws = std::fs::canonicalize(ws).unwrap();
            registry.register(&ws);
            registry.warm_or_build(&ws).unwrap();
            ws
        })
        .collect();
    (host, registry, roots)
}

/// The sweep's censuses run alone (merkle-spec §6.4, the reaper under a
/// host): beside a census that holds every generation, an overage one
/// eviction meets costs one workspace, not a second for the evicted
/// generation the other census kept alive.
#[test]
fn the_sweep_evicts_one_workspace_for_a_one_eviction_overage_beside_a_census_holding_generations() {
    let dir = tempfile::tempdir().unwrap();
    let (host, registry, _roots) = hosted_with(dir.path(), &["a", "b", "c", "d"], 64);
    // The floor the controller's limit is compared with: the host's charge
    // and the controller's own descriptors.
    let floor = || host.controller.report().floor;
    // What one eviction frees: a sweep one byte over evicts one workspace.
    registry.charge_floor();
    let warm = floor();
    host.controller.set_limit(warm - 1);
    assert_eq!(registry.reap_to_budget(1).len(), 1);
    let settled = floor();
    let freed = warm - settled;

    // Three quarters of an eviction over, a census parked on its generations.
    host.controller.set_limit(settled - freed / 4 * 3);
    let (at_gate, release) = arm(&registry.pause_in_census_generations);
    let parked = {
        let registry = Arc::clone(&registry);
        thread::spawn(move || registry.charge_floor())
    };
    at_gate.recv().expect("the census reached its gate");
    let (done, swept) = mpsc::channel();
    let sweep = {
        let registry = Arc::clone(&registry);
        thread::spawn(move || {
            let _ = done.send(registry.reap_to_budget(1));
        })
    };
    // The sweep either finishes beside the parked census or waits for it.
    let beside = swept.recv_timeout(Duration::from_millis(500));
    release.send(()).expect("the census parked on its gate");
    parked.join().unwrap();
    let victims = beside
        .or_else(|_| swept.recv_timeout(WAIT))
        .expect("the sweep finished");
    sweep.join().unwrap();
    assert_eq!(
        victims.len(),
        1,
        "one eviction frees {freed} and meets the overage: {victims:?} (floor {settled} -> {})",
        floor()
    );
    assert_eq!(host.controller.report().floor_overage, 0);
}

/// A census holds the generations only while it measures them (merkle-spec
/// §6.4, the host floor): one parked after its measurement keeps none alive,
/// so an idle reap's census charges what remains.
#[test]
fn an_idle_reaps_census_charges_no_generation_a_finished_measurement_held() {
    let dir = tempfile::tempdir().unwrap();
    let (host, registry, _roots) = hosted_with(dir.path(), &["a", "b", "c"], 64);
    registry.charge_floor();
    let warm = host.floor();

    let (at_gate, release) = arm(&registry.pause_after_census);
    let parked = {
        let registry = Arc::clone(&registry);
        thread::spawn(move || registry.charge_floor())
    };
    at_gate.recv().expect("the census reached its gate");
    let reaped = registry.reap(crate::now_secs() + 3600, 0);
    let after_reap = host.floor();
    release.send(()).expect("the census parked on its gate");
    parked.join().unwrap();
    assert_eq!(reaped.len(), 3, "{reaped:?}");
    assert!(
        after_reap < warm,
        "the reap's census dropped the floor: {after_reap} >= {warm}"
    );
    registry.charge_floor();
    assert_eq!(
        after_reap,
        host.floor(),
        "the reap's census charged a generation the parked census had measured"
    );
}
