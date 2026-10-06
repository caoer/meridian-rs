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
        &RETRY_PAUSES,
        |_| panic!("a load fault is not a race"),
    )
    .expect_err("the member's own fault");
    assert!(!model::catalog::is_transient(&error), "{error}");
    assert_eq!(attempts.get(), 1);
}

/// The whole seam over the real controller. With a one-byte allowance the
/// first body is the serialized oversized member, so a second body beside it
/// meets occupied allowance: a transient fault naming that member on every
/// consumer face, never an absent member, and served once the first lease
/// ends.
#[test]
fn a_second_body_beside_an_oversized_lease_is_a_transient_fault_on_every_face() {
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

    let held = catalog.pin_read("a.md").unwrap().unwrap();
    let scan = model::catalog::scan(&catalog, "b.md", |_| ()).expect_err("occupied");
    assert_eq!(scan.path, "b.md");
    assert!(scan.is_transient(), "{scan}");
    let frame = wire_serve::read::member_fault(scan);
    assert_eq!(frame.code, wire::ErrorCode::WorkspaceBusy);
    assert_eq!(frame.recovery, wire::Recovery::Retry);
    assert_eq!(frame.path.as_ref(), Some(&b));
    let named =
        wire_serve::read::with_document::<()>(&root, &catalog, &BTreeMap::new(), &b, |_| {
            panic!("a refused lease lends nothing, and never the workspace file")
        })
        .expect_err("occupied");
    assert_eq!(named.code, wire::ErrorCode::WorkspaceBusy);
    assert_eq!(named.path.as_ref(), Some(&b));

    drop(held);
    let served = model::catalog::read(&catalog, "b.md", |doc| doc.raw.as_bytes().to_vec())
        .expect("the same request, after the lease ended")
        .expect("b.md never stopped being a member");
    assert_eq!(served, b"# B\n\nsecond\n");
    let report = controller.report();
    assert_eq!((report.pinned, report.reserved), (0, 0));
}
