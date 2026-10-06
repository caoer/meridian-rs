//! The lease seam's admission policy (merkle-spec §6.4): which refusals it
//! retries against the same version, which it returns at once, and the class
//! each leaves with.

use super::*;
use std::cell::{Cell, RefCell};

/// Run [`admit`] over a scripted sequence of outcomes; the last one repeats.
/// Returns the result, the attempts made and the pauses taken.
fn scripted(outcomes: &[Result<u8, AdmissionError>]) -> (io::Result<u8>, usize, Vec<Duration>) {
    let attempts = Cell::new(0);
    let pauses = RefCell::new(Vec::new());
    let result = admit(
        || {
            let turn = attempts.get();
            attempts.set(turn + 1);
            outcomes[turn.min(outcomes.len() - 1)].map_err(Error::Admission)
        },
        false,
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

/// The caller's own lease is part of what occupies the allowance, so the
/// same request cannot be admitted by retrying: a fault, not a `retry`.
#[test]
fn occupied_allowance_under_the_callers_own_lease_is_not_transient() {
    let attempts = Cell::new(0);
    let error = admit(
        || -> Result<u8, Error> {
            attempts.set(attempts.get() + 1);
            Err(Error::Admission(AdmissionError::Busy))
        },
        true,
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

    let attempts = Cell::new(0);
    let error = admit(
        || -> Result<u8, Error> {
            attempts.set(attempts.get() + 1);
            Err(Error::PeakExceeded {
                reserved: 1,
                observed: 2,
            })
        },
        false,
        &RETRY_PAUSES,
        |_| panic!("a load fault is not a race"),
    )
    .expect_err("the member's own fault");
    assert!(!model::catalog::is_transient(&error), "{error}");
    assert_eq!(attempts.get(), 1);
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
    const PEAK: usize = 131_072;
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
    let catalog = Catalog::capture(&docs, &controller, &backing, PEAK).unwrap();
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
    let (taken, held) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel::<()>();
    std::thread::scope(|scope| {
        let other = &catalog;
        scope.spawn(move || {
            let lease = other.pin_read("a.md").unwrap().unwrap();
            taken.send(()).unwrap();
            let _ = released.recv();
            drop(lease);
        });
        held.recv().unwrap();
        let (transient, scan, named) = faces();
        assert!(transient, "another request's lease ends without this one");
        for frame in [scan, named] {
            assert_eq!(frame.code, wire::ErrorCode::WorkspaceBusy);
            assert_eq!(frame.recovery, wire::Recovery::Retry);
            assert_eq!(frame.path.as_ref(), Some(&b));
        }
        release.send(()).unwrap();
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
