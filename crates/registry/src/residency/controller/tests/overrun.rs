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
    shed(&budget, &member);
    let pin = member
        .pin(Access::Read, 1)
        .expect("a later load declares the learned bytes");
    assert_eq!(pin.bytes(), raw);
    drop(pin);
    let report = budget.report();
    assert_eq!((report.pinned, report.reserved), (0, 0));
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
