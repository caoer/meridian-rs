//! An overrun is learned on the version's row, never a member fault
//! (merkle-spec §6.4).
use super::*;
use std::fmt::Write as _;

/// `count` heading sections: a retained body many times a one-byte
/// declaration.
fn sections(count: usize) -> Vec<u8> {
    let mut text = String::new();
    for i in 0..count {
        writeln!(text, "# Section {i}\n\nbody {i}\n").unwrap();
    }
    text.into_bytes()
}

/// The member's raw and body bytes as the ledger charges them once loaded
/// under an ample declaration; the member is cold again on return.
fn retained(budget: &Controller, member: &Member) -> usize {
    let pin = member
        .pin(
            Access::Read,
            crate::residency::load_peak(member.identity().len),
        )
        .unwrap();
    let report = budget.report();
    drop(pin);
    shed(budget, member);
    report.raw + report.bodies
}

/// Shed every unpinned body, then lift the limit again.
fn shed(budget: &Controller, member: &Member) {
    budget.set_limit(1);
    budget.set_limit(0);
    assert_eq!(member.phase(), Phase::Cold);
}

/// A version's parse is deterministic: the load that outgrew its declaration
/// is retried by the same pin under the learned bytes and served, and every
/// later load of the version declares them.
#[test]
fn an_underdeclared_load_is_learned_and_served_by_the_same_pin() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let raw = sections(400);
    let member = budget.retain(Arc::new(store.retain(&raw).unwrap()));
    let pin = member
        .pin(Access::Read, 1)
        .expect("an overrun is learned and the load retried under it");
    assert_eq!(pin.bytes(), raw);
    drop(pin);
    assert_eq!(budget.report().peak_overruns, 1);
    shed(&budget, &member);
    let pin = member
        .pin_once(Access::Read, 1)
        .expect("a later load declares the learned bytes");
    assert_eq!(pin.bytes(), raw);
    drop(pin);
    let report = budget.report();
    assert_eq!((report.pinned, report.reserved), (0, 0));
    assert_eq!(report.peak_overruns, 1);
}

/// The self-blocked question is asked of the declaration a load is admitted
/// under: the rule's peak plus the learned bytes. A one-byte peak beside the
/// held body fits; the learned declaration beside it does not, though it fits
/// the allowance alone.
#[test]
fn the_self_blocked_question_asks_the_learned_declaration() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let learner = budget.retain(Arc::new(store.retain(&sections(400)).unwrap()));
    let held = budget.retain(Arc::new(store.retain(&sections(200)).unwrap()));
    let observed = retained(&budget, &learner);
    drop(learner.pin(Access::Read, 1));
    shed(&budget, &learner);

    let lease = held
        .pin(
            Access::Read,
            crate::residency::load_peak(held.identity().len),
        )
        .unwrap();
    let report = budget.report();
    let own = report.raw + report.bodies;
    assert!(
        own < observed && own > 4096,
        "the held body is about half the learner's: held {own}, learned {observed}"
    );
    // Room for the learned load alone, with half the held body to spare.
    budget.set_limit(report.floor + observed + own / 2);
    assert!(
        learner.held_blocks(&[lease.member_id()], 1),
        "the held body and the learned declaration exceed the allowance"
    );
    drop(lease);
    assert!(!learner.held_blocks(&[], 1));
}

/// A source that keeps the ledger's report as each read begins.
#[derive(Debug)]
struct Recording {
    handle: crate::raw_store::RawHandle,
    budget: Controller,
    seen: Mutex<Vec<Report>>,
}
impl ByteSource for Recording {
    fn identity(&self) -> ByteIdentity {
        self.handle.identity()
    }
    fn read(&self) -> io::Result<SharedBytes> {
        lock(&self.seen).push(self.budget.report());
        ByteSource::read(&self.handle)
    }
}

/// The retry and every later load declare the caller's peak on top of the
/// learned bytes, not the learned bytes alone: a peak well short of the
/// body, and an allowance that holds the body but not peak and body.
#[test]
fn the_rule_part_is_declared_on_top_of_what_was_learned() {
    const PEAK: usize = 50_000;
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let source = Arc::new(Recording {
        handle: store.retain(&sections(400)).unwrap(),
        budget: budget.clone(),
        seen: Mutex::new(Vec::new()),
    });
    let member = budget.retain(source.clone());
    let pin = member.pin(Access::Read, PEAK).expect("served by the retry");
    let held = budget.report();
    let observed = held.raw + held.bodies;
    drop(pin);
    shed(&budget, &member);
    assert!(observed > PEAK, "the load overran: observed {observed}");
    budget.set_limit(budget.report().floor + observed + PEAK / 2);
    drop(member.pin_once(Access::Read, PEAK));
    let seen = lock(&source.seen).clone();
    assert_eq!(seen[1].reserved, PEAK + observed, "the retry's declaration");
    assert_eq!(seen[2].reserved, PEAK + observed, "a later declaration");
    assert!(seen[2].transient, "peak + learned is oversized here");
}

/// Retry a member pin the way the catalog retries a settling version, but
/// without its pause budget: this test asks what the controller answers,
/// not how long a caller waits.
fn settled(member: &Member) -> Result<(), Error> {
    let deadline = Instant::now() + WAIT;
    loop {
        match member.pin(Access::Read, 1).map(drop) {
            Err(Error::Admission(AdmissionError::Settling | AdmissionError::Retiring))
                if Instant::now() < deadline =>
            {
                thread::yield_now();
            }
            outcome => return outcome,
        }
    }
}

/// Callers that share one overrunning load: it is counted once, no caller's
/// retry rejoins the load declared before the bytes were learned, and every
/// caller is served.
#[test]
fn callers_sharing_an_overrunning_load_are_served_and_it_is_counted_once() {
    const ROUNDS: usize = 50;
    const CALLERS: usize = 4;
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    for round in 0..ROUNDS {
        let mut raw = sections(60);
        raw.extend_from_slice(format!("round {round}\n").as_bytes());
        let source = source(&store, &raw, false, false);
        let member = budget.retain(source.clone());
        let before = budget.report().peak_overruns;
        let workers: Vec<_> = (0..CALLERS)
            .map(|_| {
                let member = member.clone();
                thread::spawn(move || settled(&member))
            })
            .collect();
        assert!(source.gate.entered(), "round {round}: the load began");
        let deadline = Instant::now() + WAIT;
        while lock(&budget.0.ledger).rows[&member.key]
            .resident
            .snapshot()
            .waiters
            < CALLERS - 1
        {
            assert!(Instant::now() < deadline, "round {round}: callers joined");
            thread::yield_now();
        }
        source.gate.release();
        for worker in workers {
            let outcome = worker.join().unwrap();
            assert!(outcome.is_ok(), "round {round}: {outcome:?}");
        }
        assert_eq!(
            budget.report().peak_overruns - before,
            1,
            "round {round}: one overrun, counted once"
        );
        assert!(
            source.reads.load(Ordering::SeqCst) <= 2,
            "round {round}: the overrunning load and at most one under the learned bytes"
        );
    }
}
