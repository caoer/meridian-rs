//! Deterministic regressions for rv-341; production parent is 7bb73dd7.
use super::*;

fn member_of(budget: &Controller, store: &RawStore, raw: &[u8]) -> Member {
    budget.retain(Arc::new(store.retain(raw).unwrap()))
}

/// P1 schedule. T1 observes the member cold with no calls, releases the ledger
/// and has not yet reached `admit`. T2 loads and finishes the whole wave. T3
/// begins a hot hit (no reservation). T1's `admit` then opens a wave on the hot
/// row; T1's second `begin` pass joins it as a hot hit, returns, and its spare
/// and call reservation drop while T3 is still in flight.
#[test]
fn review_admit_after_a_completed_load_must_not_strand_a_hot_hit() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let member = member_of(&budget, &store, b"# Race\n");
    drop(member.pin(Access::Read, 131_072).unwrap()); // T2's completed wave
    let hot_hit = member.begin(Access::Read, 131_072).unwrap(); // T3 in flight
    let revision = lock(&budget.0.ledger).rows[&member.key]
        .resident
        .snapshot()
        .revision;
    let spare = budget.admit(Some(member.key), 131_072, revision).unwrap(); // T1
    let t1 = member.pin(Access::Read, 131_072).unwrap(); // T1's second pass
    drop(spare);
    drop(t1);
    let stranded = {
        let state = lock(&budget.0.ledger);
        state.rows[&member.key]
            .wave
            .is_some_and(|id| !state.permits.contains_key(&id))
    };
    // Preserve the invariant failure as the red assertion, but repair the
    // dangling diagnostic link before teardown so this control does not abort
    // the test process. Green runs never enter this repair branch.
    if stranded {
        lock(&budget.0.ledger)
            .rows
            .get_mut(&member.key)
            .unwrap()
            .wave = None;
    }
    drop(hot_hit);
    assert!(member.pin(Access::Read, 131_072).is_ok());
    assert_eq!(lock(&budget.0.ledger).rows[&member.key].calls, 0);
    assert!(
        !stranded,
        "admission stranded a hot hit on a removed permit"
    );
}

/// P2a. An oversized member's live raw lease keeps `transient`. A different
/// member whose declared peak fits the remaining allowance is refused, and the
/// refused attempt first sheds an unrelated unpinned body.
#[test]
fn review_oversized_raw_lease_must_not_refuse_a_fitting_admission() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let a = member_of(&budget, &store, b"# A\n");
    let k = member_of(&budget, &store, b"# K\n");
    let b = member_of(&budget, &store, b"# B\n");
    budget.set_limit(budget.report().floor + 600_000);
    let a_pin = a.pin(Access::Read, 131_072).unwrap();
    // K's declared peak exceeds the allowance; its actual body is tiny.
    let k_pin = k.pin(Access::Read, 1_000_000).unwrap();
    let transient_admitted = budget.report().transient;
    drop(a_pin);
    let raw = k_pin.raw(); // a long-lived raw owner, e.g. a watcher baseline
    drop(k_pin);
    let before = budget.report();
    let a_hot_before = a.phase() == Phase::Hot;
    let room = before.limit - before.total;
    let result = b.pin(Access::Read, 131_072);
    let a_hot_after = a.phase() == Phase::Hot;
    println!(
        "review-p2a: transient_admitted={transient_admitted} k_phase={:?} transient={} limit={} total={} room={room} b_peak=131072 b_result={:?} a_hot_before={a_hot_before} a_hot_after={a_hot_after}",
        k.phase(),
        before.transient,
        before.limit,
        before.total,
        result.as_ref().map(|_| "pinned"),
    );
    assert!(room > 131_072);
    assert!(
        result.is_ok(),
        "fitting admission refused while an oversized raw lease lives"
    );
    assert!(
        a_hot_after,
        "refused admission shed an unrelated unpinned body"
    );
    drop(raw);
}

/// P2b. When reservations already make the requested peak impossible, the
/// refusal is certain; shedding unrelated bodies first cannot change it.
#[test]
fn review_certain_refusal_must_not_shed_unrelated_bodies() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let a = member_of(&budget, &store, b"# A\n");
    let b = member_of(&budget, &store, b"# B\n");
    budget.set_limit(budget.report().floor + 300_000);
    drop(a.pin(Access::Read, 131_072).unwrap());
    let staging = budget.reserve(250_000).unwrap(); // e.g. SQL staging
    let a_hot_before = a.phase() == Phase::Hot;
    let result = b.pin(Access::Read, 131_072);
    let a_hot_after = a.phase() == Phase::Hot;
    println!(
        "review-p2b: limit={} reserved={} b_result={:?} a_hot_before={a_hot_before} a_hot_after={a_hot_after}",
        budget.report().limit,
        budget.report().reserved,
        result.as_ref().map(|_| "pinned"),
    );
    assert!(matches!(
        result,
        Err(Error::Admission(AdmissionError::Busy))
    ));
    assert!(
        a_hot_after,
        "a refusal shedding could not avoid still shed A"
    );
    drop(staging);
}

#[test]
fn fitting_staging_and_reads_preserve_cache_before_oversized_refusal() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let oversized = member_of(&budget, &store, b"# Oversized\n");
    let other = member_of(&budget, &store, b"# Other\n");
    budget.set_limit(budget.report().floor + 600_000);
    let large = oversized.pin(Access::Read, 1_000_000).unwrap();
    let raw = large.raw();
    drop(large);
    let staging = budget.reserve(64_000).expect("ordinary fitting staging");
    let small = other
        .pin(Access::Read, 131_072)
        .expect("ordinary fitting read");
    drop(small);
    let before = budget.report();
    assert_eq!(other.phase(), Phase::Hot);
    assert!(
        matches!(
            other.controller.admit(Some(other.key), 1_000_000, 0),
            Ok(None)
        ),
        "a hot row must reselect without opening any wave"
    );
    let third = member_of(&budget, &store, b"# Third\n");
    assert!(matches!(
        third.pin(Access::Read, 1_000_000),
        Err(Error::Admission(AdmissionError::Busy))
    ));
    assert_eq!(
        other.phase(),
        Phase::Hot,
        "exclusive refusal evicted unrelated cache"
    );
    assert!(budget.report().total >= before.total);
    drop(staging);
    drop(raw);
}

#[test]
fn unmanaged_raw_alias_remains_charged_without_owning_exclusivity() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(1);
    let member = member_of(&budget, &store, b"# Alias\n");
    let pin = member.pin(Access::Read, 131_072).unwrap();
    // Deliberately exercise defensive weak accounting, not the supported
    // managed-lease contract. An alias cannot authorize exclusive admission.
    let alias = pin.document().unwrap().raw.clone();
    drop(pin);
    assert!(!budget.report().transient);
    assert!(budget.report().raw > 0);
    drop(alias);
    assert_eq!(budget.report().raw, 0);
}

#[test]
fn limit_disable_and_reenable_preserves_live_transient_ownership() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(1);
    let a = member_of(&budget, &store, b"# A\n");
    let b = member_of(&budget, &store, b"# B\n");
    let held = a.pin(Access::Read, 131_072).unwrap();
    budget.set_limit(0);
    budget.set_limit(1);
    assert!(matches!(
        b.pin(Access::Read, 131_072),
        Err(Error::Admission(AdmissionError::Busy))
    ));
    drop(held);
    drop(b.pin(Access::Read, 131_072).unwrap());
}

fn counters_match_rows(budget: &Controller) {
    let state = lock(&budget.0.ledger);
    let mut eligible = 0;
    let mut bytes = 0_u128;
    let mut raws = 0;
    let mut retiring = 0;
    for row in state.rows.values() {
        retiring += usize::from(row.retiring);
        if row.body == 0 || row.calls != 0 || row.pins != 0 || row.retiring {
            continue;
        }
        eligible += 1;
        bytes += row.body as u128;
        if let Some(raw) = row.raw.and_then(|id| state.raw.get(&id))
            && raw.owners == 1
        {
            bytes += raw.bytes as u128;
            raws += 1;
        }
    }
    assert_eq!(state.reclaimable, eligible);
    assert_eq!(state.reclaimable_bytes, bytes);
    assert_eq!(state.reclaimable_raws, raws);
    assert_eq!(state.retiring, retiring);
}

#[test]
fn cached_reclaimability_tracks_calls_raw_leases_retirement_and_removal() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let a = member_of(&budget, &store, b"# A\n");
    let b = member_of(&budget, &store, b"# B\n");
    counters_match_rows(&budget);
    let pin = a.pin(Access::Read, 131_072).unwrap();
    counters_match_rows(&budget);
    let raw = pin.raw();
    let twin = raw.clone();
    drop(pin);
    counters_match_rows(&budget);
    drop(raw);
    counters_match_rows(&budget);
    drop(twin);
    counters_match_rows(&budget);
    let call = a.begin(Access::Scan, 0).unwrap();
    counters_match_rows(&budget);
    drop(call);
    counters_match_rows(&budget);
    drop(b.pin(Access::Read, 131_072).unwrap());
    counters_match_rows(&budget);
    assert!(budget.shed_one_with(true, |resident, ticket| {
        resident.shed_with(ticket, || counters_match_rows(&budget))
    }));
    counters_match_rows(&budget);
    drop(a);
    counters_match_rows(&budget);
    drop(b);
    counters_match_rows(&budget);
    assert_eq!(lock(&budget.0.ledger).reclaimable, 0);
}
