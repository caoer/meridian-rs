//! The lease seam's admission policy (merkle-spec §6.4): which refusals it
//! retries against the same version, which it returns at once, and the class
//! each leaves with.

use super::*;
use std::cell::{Cell, RefCell};
use std::fmt::Write as _;

/// Run [`admit`] over a scripted sequence of outcomes; the last one repeats.
/// Returns the result, the attempts made and the pauses taken.
fn scripted(outcomes: &[Result<u8, AdmissionError>]) -> (io::Result<u8>, usize, Vec<Duration>) {
    let attempts = Cell::new(0);
    let pauses = RefCell::new(Vec::new());
    let result = admit(
        |_| {
            let turn = attempts.get();
            attempts.set(turn + 1);
            outcomes[turn.min(outcomes.len() - 1)].map_err(Error::Admission)
        },
        || false,
        &RETRY_PAUSES,
        |wait| pauses.borrow_mut().push(wait),
    );
    (result, attempts.get(), pauses.into_inner())
}

#[test]
fn a_retirement_or_settling_race_retries_the_same_version() {
    let (result, attempts, pauses) = scripted(&[
        Err(AdmissionError::Retiring),
        Err(AdmissionError::Settling),
        Ok(7),
    ]);
    assert_eq!(result.expect("both races end without the caller's help"), 7);
    assert_eq!(attempts, 3);
    assert_eq!(pauses, RETRY_PAUSES[..2]);
}

#[test]
fn a_race_that_outlasts_the_bound_leaves_as_a_transient_fault() {
    for race in [AdmissionError::Retiring, AdmissionError::Settling] {
        let (result, attempts, pauses) = scripted(&[Err(race)]);
        let error = result.expect_err("the retry is bounded");
        assert!(model::catalog::is_transient(&error), "{race:?}: {error}");
        assert_eq!(attempts, RETRY_PAUSES.len() + 1, "{race:?}");
        assert_eq!(pauses, RETRY_PAUSES, "{race:?}");
    }
}

#[test]
fn occupied_allowance_is_never_waited_on_and_is_transient() {
    let (result, attempts, pauses) = scripted(&[Err(AdmissionError::Busy), Ok(7)]);
    let error = result.expect_err("the caller's own pin may hold the allowance");
    assert!(model::catalog::is_transient(&error), "{error}");
    assert_eq!((attempts, pauses.len()), (1, 0));
}

/// The waiting oversized member asks again every [`WAIT_POLL`] until the
/// live loads have drained (merkle-spec §6.4): its admission does not depend
/// on its client asking again.
#[test]
fn the_waiting_oversized_member_retries_until_it_is_served() {
    let waiting = Err(AdmissionError::Waiting);
    let (result, attempts, pauses) = scripted(&[waiting, waiting, waiting, Ok(7)]);
    assert_eq!(result.expect("served once the live loads drained"), 7);
    assert_eq!(attempts, 4);
    assert_eq!(pauses, [WAIT_POLL; 3]);
}

/// Behind loads that do not drain, the waiting member's patience ends: it
/// leaves as occupied allowance, a transient fault.
#[test]
fn a_waiting_member_that_outlasts_its_patience_is_occupied_allowance() {
    let (result, attempts, pauses) = scripted(&[Err(AdmissionError::Waiting)]);
    let error = result.expect_err("the wait is bounded");
    assert!(model::catalog::is_transient(&error), "{error}");
    assert!(error.to_string().contains("occupied"), "{error}");
    assert_eq!(attempts, WAIT_POLLS + 1);
    assert_eq!(pauses.len(), WAIT_POLLS);
}

/// A load paused for another member's oversized load waits for it, and is
/// never refused for it: once its patience ends it asks past the pause.
#[test]
fn a_paused_load_waits_and_is_then_admitted_past_the_pause() {
    let refusal = Err(AdmissionError::Paused);
    let (result, attempts, pauses) = scripted(&[refusal, refusal, Ok(7)]);
    assert_eq!(result.expect("served once the oversized load ended"), 7);
    assert_eq!((attempts, pauses), (3, vec![WAIT_POLL; 2]));

    let (asked, past) = (Cell::new(0), Cell::new(0));
    let waits = Cell::new(0);
    let result = admit(
        |unpaused| {
            if unpaused {
                past.set(past.get() + 1);
                Ok(7)
            } else {
                asked.set(asked.get() + 1);
                Err(Error::Admission(AdmissionError::Paused))
            }
        },
        || false,
        &RETRY_PAUSES,
        |_| waits.set(waits.get() + 1),
    );
    assert_eq!(result.expect("a pause is never a refusal"), 7);
    assert_eq!(
        (asked.get(), waits.get(), past.get()),
        (WAIT_POLLS + 1, WAIT_POLLS, 1)
    );
}

/// A waiting or paused load whose caller's own leases alone keep it out is
/// not waited on: no retry cures it.
#[test]
fn a_wait_under_the_callers_own_lease_is_not_waited_on() {
    for refusal in [AdmissionError::Waiting, AdmissionError::Paused] {
        let error = admit(
            |_| -> Result<u8, Error> { Err(Error::Admission(refusal)) },
            || true,
            &RETRY_PAUSES,
            |_| panic!("a self-blocked load is never waited on"),
        )
        .expect_err("self-blocked");
        assert!(
            !model::catalog::is_transient(&error),
            "{refusal:?}: {error}"
        );
    }
}

/// The bar of merkle-spec §6.4 over the real controller: behind a live load
/// that outlasts the settling pauses and one waiting record, the oversized
/// member is served without asking twice, and another member's fitting
/// reader is paused and served, never refused.
#[test]
fn a_fitting_reader_is_paused_not_refused_while_an_oversized_member_waits() {
    let dir = tempfile::tempdir().unwrap();
    let backing = RawStore::new(dir.path()).unwrap();
    let controller = Controller::new(0);
    let page = |tag: &str, count: usize| {
        let mut text = format!("# {tag}\n\n");
        for i in 0..count {
            writeln!(text, "# Section {i}\n\nbody {i} of {tag}\n").unwrap();
        }
        controller.retain(Arc::new(backing.retain(text.as_bytes()).unwrap()))
    };
    let (big, other) = (page("Big", 30), page("Other", 10));
    controller.set_limit(controller.report().floor + 1_000_000);
    let live = controller.reserve(100_000).expect("a live load");

    let waiter = std::thread::spawn(move || pin(&big, Access::Read, 2_000_000).map(drop));
    // Time for the oversized member's first refusal to record it.
    std::thread::sleep(Duration::from_millis(20));
    let reader = std::thread::spawn(move || pin(&other, Access::Read, 100_000).map(drop));
    std::thread::sleep(Duration::from_millis(150));
    drop(live);
    waiter
        .join()
        .unwrap()
        .expect("the waiting member takes the slot when the live load drains");
    reader
        .join()
        .unwrap()
        .expect("the paused reader is served, not refused");
}

/// Occupied allowance the caller's own leases alone cause cannot be cured by
/// retrying the same request: a fault, not a `retry`.
#[test]
fn occupied_allowance_under_the_callers_own_lease_is_not_transient() {
    let attempts = Cell::new(0);
    let error = admit(
        |_| -> Result<u8, Error> {
            attempts.set(attempts.get() + 1);
            Err(Error::Admission(AdmissionError::Busy))
        },
        || true,
        &RETRY_PAUSES,
        |_| panic!("occupied allowance is never waited on"),
    )
    .expect_err("occupied");
    assert!(!model::catalog::is_transient(&error), "{error}");
    assert!(error.to_string().contains("allowance"), "{error}");
    assert_eq!(attempts.get(), 1);
}

#[test]
fn a_member_fault_is_neither_retried_nor_transient() {
    let (result, attempts, pauses) = scripted(&[Err(AdmissionError::Exhausted), Ok(7)]);
    let error = result.expect_err("an exhausted identity does not heal");
    assert!(!model::catalog::is_transient(&error), "{error}");
    assert_eq!((attempts, pauses.len()), (1, 0));
}

/// The pin has already retried an overrun under the learned bytes; one that
/// remains joined a load declared before they were learned, and the next
/// request declares them. Transient, never the member's fault, not retried
/// here.
#[test]
fn an_overrun_that_outlasts_the_pins_retry_is_transient() {
    let attempts = Cell::new(0);
    let error = admit(
        |_| -> Result<u8, Error> {
            attempts.set(attempts.get() + 1);
            Err(Error::PeakExceeded {
                reserved: 1,
                observed: 2,
            })
        },
        || false,
        &RETRY_PAUSES,
        |_| panic!("an overrun is not a race"),
    )
    .expect_err("the overrun stands for this request");
    assert!(model::catalog::is_transient(&error), "{error}");
    assert_eq!(attempts.get(), 1);
}

/// An overrun through the seam over the real controller. The learned
/// declaration exceeds the allowance above the floor: beside another live
/// load that never drains it waits out its patience and is then occupied
/// allowance, a transient fault; alone it takes the serialized slot and is
/// served. Never a member fault.
#[test]
fn a_learned_oversized_load_is_transient_beside_a_live_load_and_served_alone() {
    let dir = tempfile::tempdir().unwrap();
    let backing = RawStore::new(dir.path()).unwrap();
    let controller = Controller::new(0);
    let mut text = String::new();
    for i in 0..400 {
        writeln!(text, "# Section {i}\n\nbody {i}\n").unwrap();
    }
    let raw = text.into_bytes();
    let member = controller.retain(Arc::new(backing.retain(&raw).unwrap()));
    let pin = member
        .pin(Access::Read, crate::residency::load_peak(raw.len() as u64))
        .unwrap();
    let report = controller.report();
    let retained = report.raw + report.bodies;
    drop(pin);
    controller.set_limit(1);
    assert_eq!(member.phase(), crate::residency::Phase::Cold);

    // A one-byte load and another one-byte load fit; the learned one does not.
    controller.set_limit(controller.report().floor + retained / 2);
    let other = controller.reserve(1).expect("another live load");
    let attempts = Cell::new(0);
    let waits = Cell::new(0);
    let error = admit(
        |_| {
            attempts.set(attempts.get() + 1);
            member.pin(Access::Read, 1)
        },
        || false,
        &RETRY_PAUSES,
        |_| waits.set(waits.get() + 1),
    )
    .expect_err("occupied beside another live load");
    assert!(model::catalog::is_transient(&error), "{error}");
    assert_eq!(
        (attempts.get(), waits.get()),
        (WAIT_POLLS + 1, WAIT_POLLS),
        "the waiting member asks until its patience ends"
    );
    assert_eq!(member.phase(), crate::residency::Phase::Cold);

    drop(other);
    let pin = admit(
        |_| member.pin(Access::Read, 1),
        || false,
        &RETRY_PAUSES,
        pause,
    )
    .expect("alone, the learned load takes the serialized slot");
    assert_eq!(pin.bytes(), raw);
    assert!(controller.report().transient);
    drop(pin);
    let report = controller.report();
    assert_eq!((report.pinned, report.reserved), (0, 0));
}

/// The whole seam over the real controller. With a one-byte allowance the
/// first body is the serialized oversized member, so a second body beside it
/// meets occupied allowance. Held by ANOTHER request, that is a transient
/// fault naming the member on every consumer face; held by the SAME request,
/// no retry can cure it and it is an `io_error` naming the member. Neither is
/// an absent member or a read of the workspace file, and the member is
/// served once the first lease ends.
#[test]
fn a_second_body_beside_an_oversized_lease_is_classed_by_who_holds_the_first() {
    let dir = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let root = fs::WorkspaceRoot(work.path().to_path_buf());
    let backing = RawStore::new(dir.path()).unwrap();
    let controller = Controller::new(1);
    let docs: Docs = [("a.md", "# A\n\nfirst\n"), ("b.md", "# B\n\nsecond\n")]
        .into_iter()
        .map(|(path, raw)| {
            (
                path.to_owned(),
                Arc::new(model::build(raw.to_owned(), syntax::parse(raw))),
            )
        })
        .collect();
    std::fs::write(work.path().join("b.md"), "# Newer source\n").unwrap();
    let catalog = Catalog::capture(&docs, &controller, &backing).unwrap();
    drop(docs);
    let b = wire::Path("b.md".to_owned());
    // Both consumer faces for b.md: the scan fault's frame, and the named read.
    let faces = || {
        let scan = model::catalog::scan(&catalog, "b.md", |_| ()).expect_err("occupied");
        assert_eq!(scan.path, "b.md");
        let transient = scan.is_transient();
        let named =
            wire_serve::read::with_document::<()>(&root, &catalog, &BTreeMap::new(), &b, |_| {
                panic!("a refused lease lends nothing, and never the workspace file")
            })
            .expect_err("occupied");
        (transient, wire_serve::read::member_fault(scan), named)
    };

    // Another request holds the first body: retrying can succeed.
    std::thread::scope(|scope| {
        let _release = held_elsewhere(scope, &catalog, "a.md");
        let (transient, scan, named) = faces();
        assert!(transient, "another request's lease ends without this one");
        for frame in [scan, named] {
            assert_eq!(frame.code, wire::ErrorCode::WorkspaceBusy);
            assert_eq!(frame.recovery, wire::Recovery::Retry);
            assert_eq!(frame.path.as_ref(), Some(&b));
        }
    });

    // This request holds the first body itself: no retry admits the second.
    let own = catalog.pin_read("a.md").unwrap().unwrap();
    let (transient, scan, named) = faces();
    assert!(!transient, "the request's own lease occupies the allowance");
    for frame in [scan, named] {
        assert_eq!(frame.code, wire::ErrorCode::IoError);
        assert_eq!(frame.recovery, wire::Recovery::Env);
        assert_eq!(frame.path.as_ref(), Some(&b));
        assert!(
            frame
                .cause
                .as_deref()
                .is_some_and(|cause| cause.contains("b.md") && cause.contains("allowance")),
            "the cause names the member and the allowance: {frame:?}"
        );
    }

    drop(own);
    let served = model::catalog::read(&catalog, "b.md", |doc| doc.raw.as_bytes().to_vec())
        .expect("the same request, after the lease ended")
        .expect("b.md never stopped being a member");
    assert_eq!(served, b"# B\n\nsecond\n");
    let report = controller.report();
    assert_eq!((report.pinned, report.reserved), (0, 0));
}

/// Another request (thread) leasing a member until [`Elsewhere::end`], or
/// until this value drops: it lives in the caller's scope closure, so a
/// failing assertion there unwinds it and the other request ends instead of
/// waiting forever.
struct Elsewhere<'s> {
    release: std::sync::mpsc::Sender<()>,
    thread: std::thread::ScopedJoinHandle<'s, ()>,
}

impl Elsewhere<'_> {
    /// End the other request's lease and wait until its thread has dropped it.
    fn end(self) {
        drop(self.release);
        self.thread.join().unwrap();
    }
}

/// Another request (thread) leases `path` until the returned value ends.
fn held_elsewhere<'s>(
    scope: &'s std::thread::Scope<'s, '_>,
    catalog: &'s Catalog,
    path: &'static str,
) -> Elsewhere<'s> {
    let (taken, held) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let thread = scope.spawn(move || {
        let lease = catalog.pin_read(path).unwrap().unwrap();
        taken.send(()).unwrap();
        let _ = released.recv();
        drop(lease);
    });
    held.recv().unwrap();
    Elsewhere { release, thread }
}

/// The peak `path`'s load is admitted under.
fn peak(catalog: &Catalog, path: &str) -> usize {
    crate::residency::load_peak(catalog.entries[path].member.identity().len)
}

/// A catalog over three small pages, unlimited until the test sets a limit.
fn three_pages() -> (Catalog, Controller, (tempfile::TempDir, RawStore)) {
    let dir = tempfile::tempdir().unwrap();
    let backing = RawStore::new(dir.path()).unwrap();
    let controller = Controller::new(0);
    let docs: Docs = [
        ("a.md", "# A\n\nfirst\n"),
        ("b.md", "# B\n\nsecond\n"),
        ("c.md", "# C\n\nthird\n"),
    ]
    .into_iter()
    .map(|(path, raw)| {
        (
            path.to_owned(),
            Arc::new(model::build(raw.to_owned(), syntax::parse(raw))),
        )
    })
    .collect();
    let catalog = Catalog::capture(&docs, &controller, &backing).unwrap();
    (catalog, controller, (dir, backing))
}

/// Whether `path` is admitted now; else whether its refusal is transient.
fn admitted(catalog: &Catalog, path: &str) -> Result<(), bool> {
    match model::catalog::scan(catalog, path, |_| ()) {
        Ok(_) => Ok(()),
        Err(fault) => Err(fault.is_transient()),
    }
}

/// Below the oversized regime, with this request holding `a.md` and another
/// request holding `c.md`, the allowance fits `a.md` plus one more load but
/// not both pins plus one. The occupancy that keeps `b.md` out is the other
/// request's, so it is transient, and the same request holding `a.md` is
/// admitted once the other lease ends.
#[test]
fn occupancy_another_request_completes_is_transient_while_holding_a_lease() {
    let (catalog, controller, _backing) = three_pages();
    std::thread::scope(|scope| {
        let release = held_elsewhere(scope, &catalog, "c.md");
        let own = catalog.pin_read("a.md").unwrap().unwrap();
        let both = controller.report();
        controller.set_limit(both.total + peak(&catalog, "b.md") - 1);
        assert!(
            peak(&catalog, "b.md") <= controller.report().limit - controller.report().floor,
            "the load is not oversized: {:?}",
            controller.report()
        );
        assert_eq!(
            admitted(&catalog, "b.md"),
            Err(true),
            "the other request's lease ends without this one"
        );
        release.end();
        assert_eq!(
            admitted(&catalog, "b.md"),
            Ok(()),
            "admitted, a.md still held"
        );
        drop(own);
    });
}

/// Below the oversized regime, with nothing held but this request's own
/// `a.md`, an allowance that fits `a.md` and not one more load keeps `b.md`
/// out by this request's lease alone: not transient.
#[test]
fn own_leases_that_leave_no_room_for_the_load_are_not_transient() {
    let (catalog, controller, _backing) = three_pages();
    let own = catalog.pin_read("a.md").unwrap().unwrap();
    controller.set_limit(controller.report().total + peak(&catalog, "b.md") - 1);
    assert_eq!(admitted(&catalog, "b.md"), Err(false));
    drop(own);
    assert_eq!(admitted(&catalog, "b.md"), Ok(()));
}

/// In the oversized regime, every load exceeds the allowance above the floor,
/// so this request's own bytes plus the load always exceed it too. What keeps
/// `b.md` out here is the exclusive slot another request's oversized `a.md`
/// holds, not this request's `c.md`: transient.
#[test]
fn an_oversized_load_kept_out_by_another_requests_exclusive_slot_is_transient() {
    let (catalog, controller, _backing) = three_pages();
    let own = catalog.pin_read("c.md").unwrap().unwrap();
    controller.set_limit(1);
    std::thread::scope(|scope| {
        let _release = held_elsewhere(scope, &catalog, "a.md");
        assert!(
            controller.report().transient,
            "the other request holds the exclusive slot: {:?}",
            controller.report()
        );
        assert_eq!(admitted(&catalog, "b.md"), Err(true));
    });
    drop(own);
}

/// A lease that ended no longer counts against its thread, whether it was
/// dropped or unwound past: a later `Busy` another request causes on the
/// same thread is transient.
#[test]
fn a_lease_that_ended_no_longer_counts_against_its_thread() {
    let (catalog, controller, _backing) = three_pages();
    controller.set_limit(1);
    drop(catalog.pin_read("a.md").unwrap().unwrap());
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _lease = catalog.pin_read("c.md").unwrap().unwrap();
        panic!("unwinding past a live lease");
    }));
    assert!(unwound.is_err());
    std::thread::scope(|scope| {
        let _release = held_elsewhere(scope, &catalog, "a.md");
        assert_eq!(
            admitted(&catalog, "b.md"),
            Err(true),
            "this thread holds nothing"
        );
    });
}

/// A catalog over three small pages and one large one (`c.md`), unlimited
/// until the test sets a limit: the large page's body and raw bytes dwarf
/// the floor's growth while an admission is probed.
fn with_a_large_page() -> (Catalog, Controller, (tempfile::TempDir, RawStore)) {
    let dir = tempfile::tempdir().unwrap();
    let backing = RawStore::new(dir.path()).unwrap();
    let controller = Controller::new(0);
    let large = format!(
        "# C\n\n{}",
        "a paragraph of the big page with some words in it\n\n".repeat(150)
    );
    let docs: Docs = [
        ("a.md", "# A\n\nfirst\n".to_owned()),
        ("b.md", "# B\n\nsecond\n".to_owned()),
        ("d.md", "# D\n\nthird\n".to_owned()),
        ("c.md", large),
    ]
    .into_iter()
    .map(|(path, raw)| {
        let doc = model::build(raw.clone(), syntax::parse(&raw));
        (path.to_owned(), Arc::new(doc))
    })
    .collect();
    let catalog = Catalog::capture(&docs, &controller, &backing).unwrap();
    (catalog, controller, (dir, backing))
}

/// In the oversized regime this request's `a.md` is the serialized member,
/// but its own bytes fit the allowance above the floor: the exclusivity lasts
/// only because another request's large `c.md` keeps the total above the
/// limit. That occupancy ends without this request, so it is transient, and
/// the same request, still holding `a.md`, is admitted once `c.md` ends.
#[test]
fn a_serialized_member_whose_exclusivity_another_request_keeps_is_transient() {
    let (catalog, controller, _backing) = with_a_large_page();
    let before = controller.report().total;
    std::thread::scope(|scope| {
        let release = held_elsewhere(scope, &catalog, "c.md");
        let with_c = controller.report().total;
        controller.set_limit(before + (with_c - before) / 2);
        let own = catalog.pin_read("a.md").unwrap().unwrap();
        let report = controller.report();
        assert!(
            report.transient && peak(&catalog, "a.md") > report.limit.saturating_sub(report.floor),
            "a.md is the serialized oversized member: {report:?}"
        );
        assert_eq!(
            admitted(&catalog, "b.md"),
            Err(true),
            "the other request's c.md keeps the exclusivity: {report:?}"
        );
        release.end();
        assert_eq!(
            admitted(&catalog, "b.md"),
            Ok(()),
            "admitted, a.md still held"
        );
        drop(own);
    });
}

/// This request's own large `c.md` alone keeps `b.md` out, by its body and
/// its raw backing together; another request's `a.md` changes nothing. The
/// refusal is not transient while both are held, nor once `a.md` ends.
#[test]
fn own_bytes_that_keep_the_load_out_count_the_raw_backing() {
    let (catalog, controller, _backing) = with_a_large_page();
    let own = catalog.pin_read("c.md").unwrap().unwrap();
    let with_c = controller.report().total;
    std::thread::scope(|scope| {
        let release = held_elsewhere(scope, &catalog, "a.md");
        controller.set_limit(with_c + peak(&catalog, "b.md") - 1);
        assert_eq!(admitted(&catalog, "b.md"), Err(false));
        release.end();
        assert_eq!(admitted(&catalog, "b.md"), Err(false));
    });
    drop(own);
    assert_eq!(admitted(&catalog, "b.md"), Ok(()));
}

/// A member this request leases twice (a composed read of a page whose lock
/// target is on the page itself) is charged once. Its `a.md` beside another
/// request's `d.md` fills the allowance only together, so the refusal is
/// transient, and `b.md` is admitted once `d.md` ends.
#[test]
fn a_member_leased_twice_by_one_request_is_charged_once() {
    let (catalog, controller, _backing) = with_a_large_page();
    let own = catalog.pin_read("a.md").unwrap().unwrap();
    let again = catalog.pin("a.md").unwrap().unwrap();
    std::thread::scope(|scope| {
        let release = held_elsewhere(scope, &catalog, "d.md");
        controller.set_limit(controller.report().total + peak(&catalog, "b.md") - 1);
        assert_eq!(admitted(&catalog, "b.md"), Err(true));
        release.end();
        assert_eq!(admitted(&catalog, "b.md"), Ok(()));
    });
    drop(again);
    drop(own);
}
