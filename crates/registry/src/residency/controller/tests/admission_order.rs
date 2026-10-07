//! Who is admitted or shed first when the allowance is contended
//! (merkle-spec §6.4): the one waiting oversized member, a retirement-only
//! block, a stale under-declared reservation, and an overrun's body.
use super::*;
use std::fmt::Write as _;

/// `count` heading sections under a `tag` heading, distinct per tag.
fn sections(count: usize, tag: &str) -> Vec<u8> {
    let mut text = format!("# {tag}\n\n");
    for i in 0..count {
        writeln!(text, "# Section {i}\n\nbody {i} of {tag}\n").unwrap();
    }
    text.into_bytes()
}

const FITS: usize = 100_000;
const OVERSIZED: usize = 2_000_000;

/// A refused oversized load waits: while it does, a new load of another
/// member that fits is answered `Settling`, so the live load drains and the
/// waiting member's next attempt takes the serialized slot.
#[test]
fn a_refused_oversized_load_pauses_other_members_new_loads_until_it_is_served() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let live = source(&store, &sections(20, "Live"), false, false);
    let loading = budget.retain(live.clone());
    let big = budget.retain(Arc::new(store.retain(&sections(30, "Big")).unwrap()));
    let other = budget.retain(Arc::new(store.retain(&sections(10, "Other")).unwrap()));
    budget.set_limit(budget.report().floor + 1_000_000);
    let flight = thread::spawn(move || loading.pin(Access::Read, FITS).map(drop));
    assert!(live.gate.entered(), "the fitting load is live");
    let refused = big.pin_once(Access::Read, OVERSIZED).map(drop);
    assert!(
        matches!(refused, Err(Error::Admission(AdmissionError::Busy))),
        "the oversized load is refused beside a live load: {refused:?}"
    );
    let paused = other.pin_once(Access::Read, FITS).map(drop);
    assert!(
        matches!(paused, Err(Error::Admission(AdmissionError::Settling))),
        "a new fitting load of another member waits for the oversized one: {paused:?}"
    );
    live.gate.release();
    flight.join().unwrap().expect("the live load completes");
    let served = big
        .pin_once(Access::Read, OVERSIZED)
        .expect("the waiting member takes the slot once the live load drained");
    drop(served);
    drop(
        other
            .pin_once(Access::Read, FITS)
            .expect("no pause once the waiting member is served"),
    );
}

/// An abandoned waiting record expires: a member that waits and never comes
/// back pauses other members' new loads for 100 ms at most.
#[test]
fn an_abandoned_waiting_member_stops_pausing_others() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let big = budget.retain(Arc::new(store.retain(&sections(30, "Big")).unwrap()));
    let other = budget.retain(Arc::new(store.retain(&sections(10, "Other")).unwrap()));
    budget.set_limit(budget.report().floor + 1_000_000);
    let live = budget.reserve(FITS).expect("a live load");
    assert!(big.pin_once(Access::Read, OVERSIZED).is_err());
    thread::sleep(Duration::from_millis(150));
    drop(
        other
            .pin_once(Access::Read, FITS)
            .expect("the waiting record expired"),
    );
    drop(live);
}

/// An oversized load that only another body's retirement keeps out of the
/// serialized slot answers `Retiring`, which the catalog retries, not
/// occupied allowance.
#[test]
fn an_oversized_load_kept_out_only_by_a_retirement_answers_retiring() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let cached = budget.retain(Arc::new(store.retain(&sections(20, "Cached")).unwrap()));
    drop(cached.pin(Access::Read, FITS).unwrap());
    let big = budget.retain(Arc::new(store.retain(&sections(30, "Big")).unwrap()));
    budget.set_limit(budget.report().total + 1_000_000);
    let mut answer = None;
    assert!(budget.shed_one_with(true, |resident, ticket| {
        answer = Some(big.pin_once(Access::Read, OVERSIZED).map(drop));
        resident.try_shed(ticket)
    }));
    assert!(
        matches!(
            answer,
            Some(Err(Error::Admission(AdmissionError::Retiring)))
        ),
        "{answer:?}"
    );
    assert_eq!(cached.phase(), Phase::Cold);
    drop(
        big.pin_once(Access::Read, OVERSIZED)
            .expect("served once the retirement settled"),
    );
}

/// A caller that shared an overrunning load and has not yet returned keeps
/// that load's reservation after the calls ended and the body was shed. A
/// retry under the learned declaration, oversized here, settles until that
/// reservation is gone instead of meeting it at the serialized slot.
#[test]
fn a_retry_settles_while_the_stale_wave_s_reservation_outlives_its_calls() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let gated = source(&store, &sections(400, "Learner"), false, false);
    let member = budget.retain(gated.clone());
    let first = {
        let member = member.clone();
        thread::spawn(move || member.pin_once(Access::Read, 1).map(drop))
    };
    assert!(gated.gate.entered(), "the under-declared load is live");
    // Another caller on that load, still to return: its reservation reference.
    let stale = {
        let mut state = lock(&budget.0.ledger);
        let id = state.rows[&member.key].wave.expect("the load's wave");
        state.permits.get_mut(&id).unwrap().references += 1;
        Reservation {
            controller: budget.clone(),
            id,
        }
    };
    gated.gate.release();
    let overran = first.join().unwrap();
    assert!(
        matches!(overran, Err(Error::PeakExceeded { .. })),
        "{overran:?}"
    );
    assert_eq!(member.phase(), Phase::Cold);
    let learned = lock(&budget.0.ledger).rows[&member.key].learned;
    budget.set_limit(budget.report().floor + learned / 2);
    let retry = member.pin_once(Access::Read, 1).map(drop);
    assert!(
        matches!(retry, Err(Error::Admission(AdmissionError::Settling))),
        "{retry:?}"
    );
    drop(stale);
    let pin = member
        .pin_once(Access::Read, 1)
        .expect("alone, the learned load takes the serialized slot");
    assert!(budget.report().transient);
    drop(pin);
}

/// The trim after an overrun sheds the overrunning body before a colder
/// cached one: shedding it alone restores the limit.
#[test]
fn an_overrun_s_body_is_shed_before_a_colder_cached_body() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let colder = budget.retain(Arc::new(store.retain(&sections(100, "Colder")).unwrap()));
    drop(
        colder
            .pin(
                Access::Read,
                crate::residency::load_peak(colder.identity().len),
            )
            .unwrap(),
    );
    let report = budget.report();
    let cached = report.raw + report.bodies;
    let learner = budget.retain(Arc::new(store.retain(&sections(400, "Learner")).unwrap()));
    // Room for the cached body and a small declaration, not the learner's body.
    budget.set_limit(budget.report().floor + cached + 20_000);
    let overran = learner.pin_once(Access::Read, 1000).map(drop);
    assert!(
        matches!(overran, Err(Error::PeakExceeded { .. })),
        "{overran:?}"
    );
    assert_eq!(learner.phase(), Phase::Cold, "the overrun's body is shed");
    assert_eq!(colder.phase(), Phase::Hot, "the colder body stays cached");
    assert_eq!(budget.report().overage, 0);
}

/// The overrun's own body is shed, but the body its retry loads under the
/// learned declaration is cached like any other.
#[test]
fn an_overrun_s_retry_is_cached_once_its_pin_ends() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let counted = source(&store, &sections(400, "Learner"), false, false);
    counted.gate.release();
    let member = budget.retain(counted.clone());
    drop(member.pin(Access::Read, 1).expect("served by the retry"));
    assert_eq!(budget.report().peak_overruns, 1);
    assert_eq!(member.phase(), Phase::Hot, "the retried body is cached");
    drop(member.pin(Access::Read, 1).unwrap());
    assert_eq!(
        counted.reads.load(Ordering::SeqCst),
        2,
        "the overrun's load and its retry; the next pin is a hit"
    );
}
