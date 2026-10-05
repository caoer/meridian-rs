use super::*;
use crate::raw_store::RawStore;

#[test]
fn global_budget_sheds_unpinned_members_and_retries_after_last_pin() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let a = budget.retain(Arc::new(store.retain(b"# A\n\nold\n").unwrap()));
    let b = budget.retain(Arc::new(store.retain(b"# B\n\nnew\n").unwrap()));
    drop(a.pin(Access::Read, 8192).unwrap());
    let pinned = b.pin(Access::Read, 8192).unwrap();
    budget.set_limit(1);
    assert_eq!(a.phase(), Phase::Cold, "global limit did not shed member A");
    assert_eq!(b.phase(), Phase::Hot, "operation pin must protect member B");
    drop(pinned);
    assert_eq!(
        b.phase(),
        Phase::Cold,
        "last pin did not enforce the budget"
    );
    let old = a.pin(Access::Read, 8192).unwrap();
    assert_eq!(old.document().unwrap().raw.as_str(), "# A\n\nold\n");
    drop(old);
    assert_eq!(a.phase(), Phase::Cold, "oversized transient became cached");
}

use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(10);

#[derive(Debug, Default)]
struct Gate {
    state: Mutex<(bool, bool)>,
    wake: Condvar,
}
impl Gate {
    fn enter(&self) {
        let mut state = lock(&self.state);
        state.0 = true;
        self.wake.notify_all();
        while !state.1 {
            state = self.wake.wait(state).unwrap();
        }
    }
    fn entered(&self) -> bool {
        let (state, _) = self
            .wake
            .wait_timeout_while(lock(&self.state), WAIT, |s| !s.0)
            .unwrap();
        state.0
    }
    fn release(&self) {
        lock(&self.state).1 = true;
        self.wake.notify_all();
    }
}

#[derive(Debug)]
struct Source {
    handle: crate::raw_store::RawHandle,
    reads: AtomicUsize,
    gate: Gate,
    fault: bool,
    panic: bool,
}
impl ByteSource for Source {
    fn identity(&self) -> ByteIdentity {
        self.handle.identity()
    }
    fn read(&self) -> io::Result<SharedBytes> {
        if self.reads.fetch_add(1, Ordering::SeqCst) == 0 {
            self.gate.enter();
            assert!(!self.panic, "injected controller load unwind");
            if self.fault {
                return Err(io::Error::other("controller source fault"));
            }
        }
        ByteSource::read(&self.handle)
    }
}
fn source(store: &RawStore, raw: &[u8], fault: bool, panic: bool) -> Arc<Source> {
    Arc::new(Source {
        handle: store.retain(raw).unwrap(),
        reads: AtomicUsize::new(0),
        gate: Gate::default(),
        fault,
        panic,
    })
}

#[test]
fn roots_and_generations_share_one_descriptor_and_body() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let source = source(&store, b"# Shared\n", false, false);
    source.gate.release();
    let a = budget.retain(source.clone());
    let b = budget.retain(source.clone());
    let left = a.pin(Access::Read, 8192).unwrap();
    let right = b.pin(Access::Read, 8192).unwrap();
    assert!(std::ptr::eq(
        left.document().unwrap(),
        right.document().unwrap()
    ));
    assert_eq!(budget.report().members, 1);
    assert_eq!(source.reads.load(Ordering::SeqCst), 1);
    drop(a);
    drop(b);
    drop(left);
    assert_eq!(budget.report().members, 1);
    drop(right);
    assert_eq!(budget.report().members, 0);
    assert_eq!(budget.report().raw, 0);
    assert_eq!(budget.report().bodies, 0);
}

#[test]
fn read_recency_is_global_and_scans_do_not_heat_it() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let a = budget.retain(Arc::new(store.retain(b"# A\n").unwrap()));
    let b = budget.retain(Arc::new(store.retain(b"# B\n").unwrap()));
    let c = budget.retain(Arc::new(store.retain(b"# C\n").unwrap()));
    for member in [&a, &b, &c] {
        drop(member.pin(Access::Read, 8192).unwrap());
    }
    drop(a.pin(Access::Read, 0).unwrap());
    drop(b.pin(Access::Scan, 0).unwrap());
    let before = budget.report();
    let body = lock(&budget.0.ledger).rows[&b.key].body;
    let raw = lock(&budget.0.ledger).raw.values().next().unwrap().bytes;
    budget.set_limit(before.total - body - raw);
    assert_eq!(b.phase(), Phase::Cold);
    assert_eq!(a.phase(), Phase::Hot);
    assert_eq!(c.phase(), Phase::Hot);
    assert_eq!(budget.report().overage, 0);
}

#[test]
fn raw_lease_survives_descriptor_destruction_and_is_charged_once() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let source = source(&store, b"# Raw\n", false, false);
    source.gate.release();
    let member = budget.retain(source.clone());
    let pin = member.pin(Access::Read, 8192).unwrap();
    let raw = pin.raw();
    let twin = raw.clone();
    let charge = budget.report().raw;
    drop(pin);
    budget.set_limit(1);
    assert_eq!(member.phase(), Phase::Cold);
    assert_eq!(budget.report().raw, charge);
    let next = member.pin(Access::Read, 8192).unwrap();
    assert_eq!(next.raw().allocation_id(), raw.allocation_id());
    assert_eq!(source.reads.load(Ordering::SeqCst), 1);
    drop(next);
    drop(member);
    drop(raw);
    assert_eq!(budget.report().members, 0);
    assert_eq!(budget.report().raw, charge);
    assert_eq!(twin.bytes(), b"# Raw\n");
    drop(twin);
    assert_eq!(budget.report().raw, 0);
}

#[test]
fn reservations_and_floor_overflow_do_not_wrap_or_disappear_early() {
    let budget = Controller::new(0);
    let huge = budget.charge_floor(usize::MAX).unwrap();
    let small = budget.charge_floor(71).unwrap();
    budget.set_limit(usize::MAX);
    assert!(budget.report().floor_overage >= 71);
    assert!(matches!(budget.reserve(1), Err(AdmissionError::Busy)));
    drop(huge);
    assert!(budget.report().floor >= 71);
    budget.set_limit(0);
    let reservation = budget.reserve(1024).unwrap();
    let clone = reservation.clone();
    assert_eq!(budget.report().reserved, 1024);
    drop(reservation);
    assert_eq!(budget.report().reserved, 1024);
    drop(clone);
    drop(small);
    assert_eq!(budget.report().reserved, 0);
    assert_eq!(budget.report().reservations, 0);
}

#[test]
fn oversized_transient_excludes_other_cold_loads_until_last_pin() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(1);
    let a = budget.retain(Arc::new(store.retain(b"# A\n").unwrap()));
    let b = budget.retain(Arc::new(store.retain(b"# B\n").unwrap()));
    let pin = a.pin(Access::Read, 8192).unwrap();
    let clone = pin.clone();
    assert!(budget.report().transient);
    assert!(matches!(
        b.pin(Access::Read, 8192),
        Err(Error::Admission(AdmissionError::Busy))
    ));
    drop(pin);
    assert!(budget.report().transient);
    drop(clone);
    assert!(!budget.report().transient);
    assert_eq!(a.phase(), Phase::Cold);
    drop(b.pin(Access::Read, 8192).unwrap());
    assert_eq!(b.phase(), Phase::Cold);
}

#[test]
fn failed_error_keeps_its_wave_reserved_until_the_error_is_released() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let source = source(&store, b"# Fault\n", true, false);
    source.gate.release();
    let member = budget.retain(source);
    let failure = member.pin(Access::Read, 8192).unwrap_err();
    assert!(
        matches!(&failure, Error::Load(error) if matches!(error.error(), LoadError::Hydration(_)))
    );
    assert_eq!(member.phase(), Phase::Cold);
    assert_eq!(budget.report().reserved, 8192);
    drop(failure);
    assert_eq!(budget.report().reserved, 0);
    drop(member.pin(Access::Read, 8192).unwrap());
}

#[test]
fn unwind_clears_controller_call_and_reservation_bookkeeping() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(1);
    let source = source(&store, b"# Panic\n", false, true);
    source.gate.release();
    let member = budget.retain(source);
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || member.pin(Access::Read, 8192)
        ))
        .is_err()
    );
    assert_eq!(budget.report().reserved, 0);
    assert!(!budget.report().transient);
    drop(member.pin(Access::Read, 8192).unwrap());
    assert_eq!(member.phase(), Phase::Cold);
}

#[test]
fn underdeclared_peak_is_explicit_and_cannot_leave_an_unbudgeted_cache() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let member = budget.retain(Arc::new(store.retain(b"# Too large\n").unwrap()));
    assert!(
        matches!(member.pin(Access::Read, 1), Err(Error::PeakExceeded { reserved: 1, observed }) if observed > 1)
    );
    assert_eq!(member.phase(), Phase::Cold);
    assert!(budget.report().peak_shortfall > 0);
    assert_eq!(budget.report().raw, 0);
    assert_eq!(budget.report().bodies, 0);
    assert_eq!(budget.report().reserved, 0);
}

#[test]
fn retirement_stays_charged_while_the_local_slot_is_already_cold() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let member = budget.retain(Arc::new(store.retain(b"# Retire\n").unwrap()));
    drop(member.pin(Access::Read, 8192).unwrap());
    let before = budget.report();
    let gate = Arc::new(Gate::default());
    let worker = {
        let budget = budget.clone();
        let gate = gate.clone();
        thread::spawn(move || {
            budget.shed_one_with(true, |resident, ticket| {
                resident.shed_with(ticket, || gate.enter())
            })
        })
    };
    let entered = gate.entered();
    let (tx, rx) = mpsc::channel();
    let observer = {
        let budget = budget.clone();
        let member = member.clone();
        thread::spawn(move || {
            let report = budget.report();
            let cold = member.phase() == Phase::Cold;
            let busy = matches!(
                member.pin(Access::Read, 8192),
                Err(Error::Admission(AdmissionError::Busy))
            );
            tx.send((report, cold, busy)).unwrap();
        })
    };
    let result = rx.recv_timeout(WAIT);
    gate.release();
    let shed = worker.join().unwrap();
    observer.join().unwrap();
    assert!(entered);
    assert!(shed);
    let (during, cold, busy) = result.unwrap();
    assert!(cold && busy);
    assert_eq!(during.bodies, before.bodies);
    assert_eq!(during.raw, before.raw);
    assert!(during.retiring >= before.bodies);
    assert_eq!(budget.report().bodies, 0);
    assert_eq!(budget.report().raw, 0);
}

#[test]
fn concurrent_callers_share_one_load_reservation_and_source_read() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let source = source(&store, b"# Flight\n", false, false);
    let member = budget.retain(source.clone());
    let workers: Vec<_> = (0..5)
        .map(|_| {
            let member = member.clone();
            thread::spawn(move || member.pin(Access::Read, 8192))
        })
        .collect();
    let entered = source.gate.entered();
    let deadline = Instant::now() + WAIT;
    let mut joined = false;
    while Instant::now() < deadline {
        if lock(&budget.0.ledger).rows[&member.key]
            .resident
            .snapshot()
            .waiters
            == 4
        {
            joined = true;
            break;
        }
        thread::yield_now();
    }
    let during = budget.report();
    source.gate.release();
    let pins: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap().unwrap())
        .collect();
    assert!(entered && joined);
    assert_eq!(during.reserved, 8192);
    assert_eq!(during.reservations, 1);
    assert_eq!(source.reads.load(Ordering::SeqCst), 1);
    assert_eq!(budget.report().reserved, 0);
    for pin in &pins {
        assert!(std::ptr::eq(
            pin.document().unwrap(),
            pins[0].document().unwrap()
        ));
    }
}

#[test]
fn an_oversized_raw_lease_keeps_exclusive_admission_after_its_tree_is_shed() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(1);
    let source_a = source(&store, b"# A\n", false, false);
    source_a.gate.release();
    let a = budget.retain(source_a.clone());
    let b = budget.retain(Arc::new(store.retain(b"# B\n").unwrap()));
    let pin = a.pin(Access::Read, 8192).unwrap();
    let raw = pin.raw();
    drop(pin);
    assert_eq!(a.phase(), Phase::Cold);
    assert!(
        budget.report().transient,
        "raw use ended transient admission too early"
    );
    assert!(matches!(
        b.pin(Access::Read, 8192),
        Err(Error::Admission(AdmissionError::Busy))
    ));
    drop(a);
    assert!(budget.report().transient);
    let same = budget.retain(source_a.clone());
    let reloaded = same.pin(Access::Read, 8192).unwrap();
    assert_eq!(reloaded.raw().allocation_id(), raw.allocation_id());
    assert_eq!(source_a.reads.load(Ordering::SeqCst), 1);
    drop(reloaded);
    drop(same);
    assert!(budget.report().transient);
    drop(raw);
    assert!(!budget.report().transient);
    drop(b.pin(Access::Read, 8192).unwrap());
}

#[test]
fn pending_same_version_admissions_share_one_reservation_before_local_loading() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let member = budget.retain(Arc::new(store.retain(b"# Pending\n").unwrap()));
    // Hold each pin caller exactly between admission and begin's second lock.
    let first = budget.admit(Some(member.key), 8192, 0).unwrap();
    let second = budget.admit(Some(member.key), 8192, 0).unwrap();
    assert_eq!(
        budget.report().reserved,
        8192,
        "pending callers reserved the same wave twice"
    );
    drop(first);
    drop(second);
    assert_eq!(budget.report().reserved, 0);
    drop(member.pin(Access::Read, 8192).unwrap());
}

#[test]
fn ordinary_admission_refuses_before_io_when_staging_occupies_allowance() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let source = source(&store, b"# Admission\n", false, false);
    source.gate.release();
    let member = budget.retain(source.clone());
    budget.set_limit(budget.report().floor + 20_480);
    let occupied = budget.reserve(16_384).unwrap();
    assert!(matches!(
        member.pin(Access::Read, 8192),
        Err(Error::Admission(AdmissionError::Busy))
    ));
    assert_eq!(source.reads.load(Ordering::SeqCst), 0);
    drop(occupied);
    let pin = member.pin(Access::Read, 8192).unwrap();
    assert!(!budget.report().transient);
    assert_eq!(budget.report().overage, 0);
    drop(pin);
    assert_eq!(member.phase(), Phase::Hot);
}

#[test]
fn retirement_unwind_reconciles_only_after_retired_value_destruction() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let member = budget.retain(Arc::new(store.retain(b"# Retirement unwind\n").unwrap()));
    drop(member.pin(Access::Read, 8192).unwrap());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        budget.shed_one_with(true, |resident, ticket| {
            resident.shed_with(ticket, || panic!("retirement unwind"))
        })
    }));
    assert!(result.is_err());
    assert_eq!(member.phase(), Phase::Cold);
    assert_eq!(budget.report().retiring, 0);
    assert_eq!(budget.report().bodies, 0);
    assert_eq!(budget.report().raw, 0);
    drop(member.pin(Access::Read, 8192).unwrap());
}

#[derive(Debug)]
struct DroppingSource {
    handle: crate::raw_store::RawHandle,
    gate: Arc<Gate>,
    panic: bool,
}
impl ByteSource for DroppingSource {
    fn identity(&self) -> ByteIdentity {
        self.handle.identity()
    }
    fn read(&self) -> io::Result<SharedBytes> {
        ByteSource::read(&self.handle)
    }
}
impl Drop for DroppingSource {
    fn drop(&mut self) {
        self.gate.enter();
        assert!(!self.panic, "descriptor destructor unwind");
    }
}

#[test]
fn descriptor_charge_survives_blocking_source_destruction_outside_ledger_lock() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let gate = Arc::new(Gate::default());
    let member = budget.retain(Arc::new(DroppingSource {
        handle: store.retain(b"# Drop\n").unwrap(),
        gate: gate.clone(),
        panic: false,
    }));
    drop(member.pin(Access::Read, 8192).unwrap());
    let before = budget.report();
    let worker = thread::spawn(move || drop(member));
    let entered = gate.entered();
    let (tx, rx) = mpsc::channel();
    let observer = {
        let budget = budget.clone();
        thread::spawn(move || tx.send(budget.report()).unwrap())
    };
    let during = rx.recv_timeout(WAIT);
    gate.release();
    worker.join().unwrap();
    observer.join().unwrap();
    assert!(entered);
    let during = during.unwrap();
    assert_eq!(during.members, 0);
    assert_eq!(during.bodies, before.bodies);
    assert_eq!(during.raw, before.raw);
    assert!(during.retiring > before.bodies);
    assert_eq!(budget.report().retiring, 0);
    assert_eq!(budget.report().bodies, 0);
    assert_eq!(budget.report().raw, 0);
}

#[test]
fn descriptor_destructor_unwind_releases_its_charge_after_all_fields_drop() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let gate = Arc::new(Gate::default());
    gate.release();
    let member = budget.retain(Arc::new(DroppingSource {
        handle: store.retain(b"# Drop panic\n").unwrap(),
        gate,
        panic: true,
    }));
    drop(member.pin(Access::Read, 8192).unwrap());
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(member))).is_err());
    assert_eq!(budget.report().members, 0);
    assert_eq!(budget.report().retiring, 0);
    assert_eq!(budget.report().bodies, 0);
    assert_eq!(budget.report().raw, 0);
}
