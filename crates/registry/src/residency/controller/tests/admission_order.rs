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
/// member that fits is answered `Paused`, so the live load drains and the
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
        matches!(refused, Err(Error::Admission(AdmissionError::Waiting))),
        "the oversized load waits beside a live load: {refused:?}"
    );
    let paused = other.pin_once(Access::Read, FITS).map(drop);
    assert!(
        matches!(paused, Err(Error::Admission(AdmissionError::Paused))),
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

/// A load paused for one oversized member takes its turn before the next
/// oversized load: when the first leaves the slot, a second oversized member
/// neither takes the slot nor records itself until the paused load has been
/// admitted (merkle-spec §6.4, rv-368b P2-1).
#[test]
fn a_paused_load_takes_its_turn_before_the_next_oversized_load() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let gated = source(&store, &sections(30, "Big"), false, false);
    let big = budget.retain(gated.clone());
    let next = budget.retain(Arc::new(store.retain(&sections(30, "Next")).unwrap()));
    let other = budget.retain(Arc::new(store.retain(&sections(10, "Other")).unwrap()));
    budget.set_limit(budget.report().floor + 1_000_000);
    let live = budget.reserve(FITS).expect("a live load");
    let waits = big.pin_once(Access::Read, OVERSIZED).map(drop);
    assert!(
        matches!(waits, Err(Error::Admission(AdmissionError::Waiting))),
        "{waits:?}"
    );
    let paused = other.pin_once(Access::Read, FITS).map(drop);
    assert!(
        matches!(paused, Err(Error::Admission(AdmissionError::Paused))),
        "{paused:?}"
    );
    drop(live);
    let flight = thread::spawn(move || big.pin(Access::Read, OVERSIZED).map(drop));
    assert!(
        gated.gate.entered(),
        "the waiting member is not kept out by the load its record paused"
    );
    let paused = other.pin_once(Access::Read, FITS).map(drop);
    assert!(
        matches!(paused, Err(Error::Admission(AdmissionError::Paused))),
        "paused again by the held slot: {paused:?}"
    );
    gated.gate.release();
    flight
        .join()
        .unwrap()
        .expect("the oversized load is served");
    let kept_out = next.pin_once(Access::Read, OVERSIZED).map(drop);
    assert!(
        matches!(kept_out, Err(Error::Admission(AdmissionError::Waiting))),
        "the next oversized load leaves the turn to the paused load: {kept_out:?}"
    );
    drop(
        other
            .pin_once(Access::Read, FITS)
            .expect("the paused load is admitted: no record formed against it"),
    );
    drop(
        next.pin_once(Access::Read, OVERSIZED)
            .expect("the next oversized load follows the paused one"),
    );
}

/// A load paused only by the waiting record is owed its turn too, when the
/// waiting member's load has come and gone without the paused load asking
/// again.
#[test]
fn a_load_the_record_paused_is_owed_a_turn_after_the_waiting_member_is_served() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let big = budget.retain(Arc::new(store.retain(&sections(30, "Big")).unwrap()));
    let next = budget.retain(Arc::new(store.retain(&sections(30, "Next")).unwrap()));
    let other = budget.retain(Arc::new(store.retain(&sections(10, "Other")).unwrap()));
    budget.set_limit(budget.report().floor + 1_000_000);
    let live = budget.reserve(FITS).expect("a live load");
    assert!(big.pin_once(Access::Read, OVERSIZED).is_err());
    let paused = other.pin_once(Access::Read, FITS).map(drop);
    assert!(
        matches!(paused, Err(Error::Admission(AdmissionError::Paused))),
        "{paused:?}"
    );
    drop(live);
    drop(
        big.pin_once(Access::Read, OVERSIZED)
            .expect("the waiting member is served"),
    );
    let kept_out = next.pin_once(Access::Read, OVERSIZED).map(drop);
    assert!(
        matches!(kept_out, Err(Error::Admission(AdmissionError::Waiting))),
        "{kept_out:?}"
    );
}

/// A paused load that never comes back holds no oversized load out past
/// 100 ms.
#[test]
fn an_abandoned_paused_load_stops_keeping_oversized_loads_out() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let gated = source(&store, &sections(30, "Big"), false, false);
    let big = budget.retain(gated.clone());
    let next = budget.retain(Arc::new(store.retain(&sections(30, "Next")).unwrap()));
    let other = budget.retain(Arc::new(store.retain(&sections(10, "Other")).unwrap()));
    budget.set_limit(budget.report().floor + 1_000_000);
    let flight = thread::spawn(move || big.pin(Access::Read, OVERSIZED).map(drop));
    assert!(gated.gate.entered(), "alone, it takes the slot");
    let paused = other.pin_once(Access::Read, FITS).map(drop);
    assert!(
        matches!(paused, Err(Error::Admission(AdmissionError::Paused))),
        "{paused:?}"
    );
    gated.gate.release();
    flight
        .join()
        .unwrap()
        .expect("the oversized load is served");
    let kept_out = next.pin_once(Access::Read, OVERSIZED).map(drop);
    assert!(
        matches!(kept_out, Err(Error::Admission(AdmissionError::Waiting))),
        "a load the held slot paused is owed a turn: {kept_out:?}"
    );
    thread::sleep(Duration::from_millis(150));
    drop(
        next.pin_once(Access::Read, OVERSIZED)
            .expect("the paused load's turn expired"),
    );
}

/// No waiting record forms while another member holds the slot: a load the
/// held slot paused takes its turn before the member that asked during that
/// load, so it waits out one oversized load, not two (rv-368c P2-1).
#[test]
fn no_record_forms_while_another_member_holds_the_slot() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let gated = source(&store, &sections(30, "Big"), false, false);
    let big = budget.retain(gated.clone());
    let next = budget.retain(Arc::new(store.retain(&sections(30, "Next")).unwrap()));
    let other = budget.retain(Arc::new(store.retain(&sections(10, "Other")).unwrap()));
    budget.set_limit(budget.report().floor + 1_000_000);
    let flight = thread::spawn(move || big.pin(Access::Read, OVERSIZED).map(drop));
    assert!(gated.gate.entered(), "alone, it takes the slot");
    let waits = next.pin_once(Access::Read, OVERSIZED).map(drop);
    assert!(
        matches!(waits, Err(Error::Admission(AdmissionError::Waiting))),
        "{waits:?}"
    );
    assert_eq!(
        lock(&budget.0.ledger).waiting,
        None,
        "a record formed while another member held the slot"
    );
    let paused = other.pin_once(Access::Read, FITS).map(drop);
    assert!(
        matches!(paused, Err(Error::Admission(AdmissionError::Paused))),
        "{paused:?}"
    );
    gated.gate.release();
    flight
        .join()
        .unwrap()
        .expect("the oversized load is served");
    let kept_out = next.pin_once(Access::Read, OVERSIZED).map(drop);
    assert!(
        matches!(kept_out, Err(Error::Admission(AdmissionError::Waiting))),
        "the load the slot paused goes first: {kept_out:?}"
    );
    drop(
        other
            .pin_once(Access::Read, FITS)
            .expect("the paused load takes its turn"),
    );
    drop(
        next.pin_once(Access::Read, OVERSIZED)
            .expect("the next oversized load follows it"),
    );
}

/// One member waits at a time at the slot too: once the live loads drain,
/// an oversized load of another member does not take the slot ahead of the
/// live record, whose paused loads would then wait out both (rv-368c P2-1).
#[test]
fn an_oversized_load_does_not_take_the_slot_past_another_members_record() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let recorded = budget.retain(Arc::new(store.retain(&sections(30, "Big")).unwrap()));
    let other = budget.retain(Arc::new(store.retain(&sections(30, "Other")).unwrap()));
    budget.set_limit(budget.report().floor + 1_000_000);
    let live = budget.reserve(FITS).expect("a live load");
    let waits = recorded.pin_once(Access::Read, OVERSIZED).map(drop);
    assert!(
        matches!(waits, Err(Error::Admission(AdmissionError::Waiting))),
        "{waits:?}"
    );
    drop(live);
    let past = other.pin_once(Access::Read, OVERSIZED).map(drop);
    assert!(
        matches!(past, Err(Error::Admission(AdmissionError::Busy))),
        "another member took the slot past the live record: {past:?}"
    );
    drop(
        recorded
            .pin_once(Access::Read, OVERSIZED)
            .expect("the waiting member takes the slot"),
    );
}

/// A paused load's turn lasts 100 ms after its last attempt, each attempt
/// renewing it, and keeps every oversized poll inside it out, while the slot
/// is held and after (rv-368c P3-1).
#[test]
fn a_paused_load_s_turn_lasts_100_ms_past_its_last_attempt() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let gated = source(&store, &sections(30, "Big"), false, false);
    let big = budget.retain(gated.clone());
    let next = budget.retain(Arc::new(store.retain(&sections(30, "Next")).unwrap()));
    let other = budget.retain(Arc::new(store.retain(&sections(10, "Other")).unwrap()));
    budget.set_limit(budget.report().floor + 1_000_000);
    let flight = thread::spawn(move || big.pin(Access::Read, OVERSIZED).map(drop));
    assert!(gated.gate.entered(), "alone, it takes the slot");
    // Attempts at 0, 50, 100 and 150 ms: the first turn would end at 100.
    for attempt in 0..4 {
        if attempt > 0 {
            thread::sleep(Duration::from_millis(50));
        }
        let paused = other.pin_once(Access::Read, FITS).map(drop);
        assert!(
            matches!(paused, Err(Error::Admission(AdmissionError::Paused))),
            "{paused:?}"
        );
    }
    let last = Instant::now();
    thread::sleep(Duration::from_millis(10));
    let mut polls = Vec::new();
    for _ in 0..2 {
        polls.push(next.pin_once(Access::Read, OVERSIZED).map(drop));
    }
    gated.gate.release();
    flight
        .join()
        .unwrap()
        .expect("the oversized load is served");
    for _ in 0..2 {
        polls.push(next.pin_once(Access::Read, OVERSIZED).map(drop));
    }
    assert!(
        last.elapsed() < Duration::from_millis(100),
        "probe invalid: the polls ran past the turn ({:?})",
        last.elapsed()
    );
    assert!(
        polls
            .iter()
            .all(|poll| matches!(poll, Err(Error::Admission(AdmissionError::Waiting)))),
        "two polls in the slot, two after it: {polls:?}"
    );
    drop(
        other
            .pin_once(Access::Read, FITS)
            .expect("the paused load takes its turn"),
    );
    drop(
        next.pin_once(Access::Read, OVERSIZED)
            .expect("the next oversized load follows it"),
    );
}

/// A forced row selected for retirement is `retiring` and unlinked, but its
/// body is still hot and unshared until the shed takes it, so it has an
/// eviction ticket. A trim that runs in that window must not select it
/// again: only the idle test keeps the forced walk off it (rv-368b P3-1).
#[test]
fn a_row_under_retirement_is_not_taken_again_by_the_forced_walk() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let forced = budget.retain(Arc::new(store.retain(&sections(20, "Forced")).unwrap()));
    let other = budget.retain(Arc::new(store.retain(&sections(20, "Other")).unwrap()));
    drop(other.pin(Access::Read, FITS).unwrap());
    drop(forced.pin(Access::Read, FITS).unwrap());
    budget.set_limit(budget.report().total + 10_000_000);
    assert_eq!((forced.phase(), other.phase()), (Phase::Hot, Phase::Hot));
    lock(&budget.0.ledger).force_shed(forced.key);
    let mut again = None;
    let shed = budget.shed_one_with(false, |resident, ticket| {
        again = Some(budget.shed_one(false));
        resident.try_shed(ticket)
    });
    assert_eq!(
        again,
        Some(false),
        "the forced walk took a row under retirement"
    );
    assert!(shed, "the forced row is shed once");
    assert_eq!(forced.phase(), Phase::Cold);
    assert_eq!(other.phase(), Phase::Hot, "a body that is not forced stays");
    assert!(lock(&budget.0.ledger).forced.is_empty());
    assert_eq!(budget.report().overage, 0);
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

/// An abandoned waiting record stops refusing other oversized members too
/// (rv-368d P3-1): once its 100 ms have passed, another member's oversized
/// load takes the free slot.
#[test]
fn an_abandoned_waiting_record_stops_refusing_other_oversized_members() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let big = budget.retain(Arc::new(store.retain(&sections(30, "Big")).unwrap()));
    let other = budget.retain(Arc::new(store.retain(&sections(30, "Other")).unwrap()));
    budget.set_limit(budget.report().floor + 1_000_000);
    let live = budget.reserve(FITS).expect("a live load");
    assert!(
        big.pin_once(Access::Read, OVERSIZED).is_err(),
        "refused beside a live load, its record forms"
    );
    drop(live);
    thread::sleep(Duration::from_millis(150));
    drop(
        other
            .pin_once(Access::Read, OVERSIZED)
            .expect("the abandoned record expired"),
    );
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

/// A paused load asks past the pause once its patience ends
/// (`Member::pin_unpaused`): it is admitted beside the waiting member's
/// record, on the terms of any fitting load.
#[test]
fn an_unpaused_load_is_admitted_beside_a_waiting_member() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let big = budget.retain(Arc::new(store.retain(&sections(30, "Big")).unwrap()));
    let other = budget.retain(Arc::new(store.retain(&sections(10, "Other")).unwrap()));
    budget.set_limit(budget.report().floor + 1_000_000);
    let live = budget.reserve(FITS).expect("a live load");
    assert!(big.pin_once(Access::Read, OVERSIZED).is_err());
    let paused = other.pin_once(Access::Read, FITS).map(drop);
    assert!(
        matches!(paused, Err(Error::Admission(AdmissionError::Paused))),
        "{paused:?}"
    );
    drop(
        other
            .pin_unpaused(Access::Read, FITS)
            .expect("past the pause, a fitting load is admitted"),
    );
    drop(live);
}

/// While another member's oversized load runs in the serialized slot, a
/// new fitting load is paused, not refused as occupied allowance: the load
/// ends without this caller's help. Past the pause it is occupied allowance.
#[test]
fn a_new_load_is_paused_while_another_members_oversized_load_runs() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let gated = source(&store, &sections(30, "Big"), false, false);
    let big = budget.retain(gated.clone());
    let other = budget.retain(Arc::new(store.retain(&sections(10, "Other")).unwrap()));
    budget.set_limit(budget.report().floor + 1_000_000);
    let flight = thread::spawn(move || big.pin(Access::Read, OVERSIZED).map(drop));
    assert!(gated.gate.entered(), "the oversized load is live");
    let beside = other.pin_once(Access::Read, FITS).map(drop);
    assert!(
        matches!(beside, Err(Error::Admission(AdmissionError::Paused))),
        "beside the oversized load: {beside:?}"
    );
    let past = other.pin_unpaused(Access::Read, FITS).map(drop);
    assert!(
        matches!(past, Err(Error::Admission(AdmissionError::Busy))),
        "past the pause the slot is occupied allowance: {past:?}"
    );
    gated.gate.release();
    flight
        .join()
        .unwrap()
        .expect("the oversized load is served");
    drop(
        other
            .pin_once(Access::Read, FITS)
            .expect("served once the slot is free"),
    );
}

/// Another member's paused attempts do not renew the record: an abandoned
/// waiting member stops pausing a steadily asking reader within its 100 ms.
#[test]
fn paused_attempts_do_not_renew_the_waiting_record() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let big = budget.retain(Arc::new(store.retain(&sections(30, "Big")).unwrap()));
    let other = budget.retain(Arc::new(store.retain(&sections(10, "Other")).unwrap()));
    budget.set_limit(budget.report().floor + 1_000_000);
    let live = budget.reserve(FITS).expect("a live load");
    let asked = Instant::now();
    assert!(big.pin_once(Access::Read, OVERSIZED).is_err());
    let served = loop {
        match other.pin_once(Access::Read, FITS) {
            Ok(pin) => {
                drop(pin);
                break asked.elapsed();
            }
            Err(Error::Admission(AdmissionError::Paused)) => {}
            Err(error) => panic!("{error:?}"),
        }
        assert!(
            asked.elapsed() < Duration::from_secs(2),
            "the record outlived 2 s of paused attempts"
        );
        thread::sleep(Duration::from_millis(2));
    };
    drop(live);
    assert!(served >= Duration::from_millis(100), "{served:?}");
}

/// One member waits at a time: a second oversized member's refusal neither
/// takes nor renews the first one's record, and is plain occupied allowance.
#[test]
fn a_second_oversized_refusal_neither_takes_nor_renews_the_record() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let first = budget.retain(Arc::new(store.retain(&sections(30, "Big1")).unwrap()));
    let second = budget.retain(Arc::new(store.retain(&sections(30, "Big2")).unwrap()));
    budget.set_limit(budget.report().floor + 1_000_000);
    let live = budget.reserve(FITS).expect("a live load");
    let record = || lock(&budget.0.ledger).waiting;
    assert!(matches!(
        first.pin_once(Access::Read, OVERSIZED).map(drop),
        Err(Error::Admission(AdmissionError::Waiting))
    ));
    let recorded = record().expect("the first refusal waits");
    assert_eq!(recorded.0, first.key);
    thread::sleep(Duration::from_millis(5));
    let other = second.pin_once(Access::Read, OVERSIZED).map(drop);
    assert!(
        matches!(other, Err(Error::Admission(AdmissionError::Busy))),
        "{other:?}"
    );
    assert_eq!(
        record(),
        Some(recorded),
        "another member took or renewed it"
    );
    drop(live);
    drop(
        first
            .pin_once(Access::Read, OVERSIZED)
            .expect("alone, the waiting member is served"),
    );
    assert!(record().is_none(), "the record ends with the admission");
}

/// The force-shed list and the rows' flags agree at every instant while
/// members overrun, shed, re-cache and are collected on four threads, and
/// the walk takes only idle rows: a worker that met a row shed under its
/// call would panic.
#[test]
fn the_forced_list_matches_the_rows_under_concurrency() {
    let parent = tempfile::tempdir().unwrap();
    let store = Arc::new(RawStore::new(parent.path()).unwrap());
    let budget = Controller::new(0);
    let pages: Arc<Vec<Vec<u8>>> = Arc::new(
        (0..8)
            .map(|i| sections(150 + 40 * i, &format!("M{i}")))
            .collect(),
    );
    let pinned = budget.retain(Arc::new(store.retain(&sections(60, "Pinned")).unwrap()));
    let held = pinned.pin(Access::Read, FITS).unwrap();
    budget.set_limit(budget.report().total + 600_000);
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let monitor = {
        let budget = budget.clone();
        let stop = Arc::clone(&stop);
        thread::spawn(move || {
            let mut checks = 0usize;
            while !stop.load(Ordering::SeqCst) {
                {
                    let state = lock(&budget.0.ledger);
                    let flagged = state.rows.values().filter(|row| row.force_shed).count();
                    let mut keys = state.forced.clone();
                    keys.sort_unstable();
                    keys.dedup();
                    assert_eq!(keys.len(), state.forced.len(), "a duplicate key");
                    assert_eq!(flagged, state.forced.len(), "list and flags differ");
                    for key in &state.forced {
                        assert!(state.rows.get(key).is_some_and(|row| row.force_shed));
                    }
                }
                checks += 1;
                thread::sleep(Duration::from_micros(50));
            }
            checks
        })
    };
    let workers: Vec<_> = (0..4usize)
        .map(|worker| {
            let budget = budget.clone();
            let store = Arc::clone(&store);
            let pages = Arc::clone(&pages);
            let stop = Arc::clone(&stop);
            thread::spawn(move || {
                let mut n = worker;
                while !stop.load(Ordering::SeqCst) {
                    n = n.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                    let page = &pages[(n >> 8) % pages.len()];
                    let member = budget.retain(Arc::new(store.retain(page).unwrap()));
                    match member.pin(Access::Read, 1) {
                        Ok(pin) => {
                            if n.is_multiple_of(3) {
                                thread::sleep(Duration::from_micros(200));
                            }
                            drop(pin);
                        }
                        Err(Error::Admission(_) | Error::PeakExceeded { .. }) => {}
                        Err(error) => panic!("{error:?}"),
                    }
                    if !n.is_multiple_of(2) {
                        thread::sleep(Duration::from_micros(100));
                    }
                    drop(member);
                }
            })
        })
        .collect();
    thread::sleep(Duration::from_secs(3));
    stop.store(true, Ordering::SeqCst);
    for worker in workers {
        worker
            .join()
            .expect("no worker met a row shed under its call");
    }
    assert!(monitor.join().expect("the list matched the flags") > 0);
    drop(held);
    assert!(
        budget.report().peak_overruns > 0,
        "probe invalid: no overrun"
    );
    assert!(lock(&budget.0.ledger).forced.is_empty());
}
