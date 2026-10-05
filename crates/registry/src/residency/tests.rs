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
    let observed = rx.recv_timeout(WAIT);
    source.gate.release();
    let leader = leader.join();
    let mut results: Vec<_> = followers.into_iter().map(|j| j.join().unwrap()).collect();
    observer.join().unwrap();
    assert!(entered, "loader did not enter its source");
    assert!(
        observed.unwrap(),
        "waiters or lock-free observation missing"
    );
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
    let doc = pins[0].document().unwrap() as *const model::Document;
    for pin in &pins {
        assert_eq!(pin.document().unwrap() as *const model::Document, doc);
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
fn missing_backing_is_an_explicit_fault_and_keeps_the_descriptor() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let resident = Resident::new(Arc::new(store.retain(b"retained").unwrap()));
    let identity = resident.identity();
    drop(resident.pin(Access::Read).unwrap());
    assert!(resident.try_shed(&resident.eviction_ticket().unwrap()));
    for dir in std::fs::read_dir(parent.path()).unwrap() {
        for file in std::fs::read_dir(dir.unwrap().path()).unwrap() {
            std::fs::remove_file(file.unwrap().path()).unwrap();
        }
    }
    assert!(
        matches!(resident.pin(Access::Read).unwrap_err().as_ref(), LoadError::Hydration(HydrationError::Source(e)) if e.kind() == io::ErrorKind::NotFound)
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
