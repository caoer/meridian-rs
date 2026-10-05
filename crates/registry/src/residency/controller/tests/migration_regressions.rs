use super::*;

fn member_of(budget: &Controller, store: &RawStore, raw: &[u8]) -> Member {
    budget.retain(Arc::new(store.retain(raw).unwrap()))
}

#[test]
fn migration_failed_flight_completion_is_retryable_not_pressure() {
    let dir = tempfile::tempdir().unwrap();
    let store = RawStore::new(dir.path()).unwrap();
    let budget = Controller::new(0);
    let source = source(&store, b"# Retry\n", true, false);
    source.gate.release();
    let member = budget.retain(source);
    let mut pending = member.begin(Access::Read, 131_072).unwrap();
    assert!(pending.resolve().is_err());
    assert!(
        matches!(
            member.begin(Access::Read, 131_072),
            Err(AdmissionError::Settling)
        ),
        "failed-flight completion was reported as byte pressure"
    );
    drop(pending);
    assert!(member.pin(Access::Read, 131_072).is_ok());
}

#[test]
fn migration_small_baseline_raw_does_not_keep_load_peak_exclusive() {
    let dir = tempfile::tempdir().unwrap();
    let store = RawStore::new(dir.path()).unwrap();
    let budget = Controller::new(0);
    let a = member_of(&budget, &store, b"# A\n");
    let b = member_of(&budget, &store, b"# B\n");
    budget.set_limit(budget.report().floor + 600_000);
    let pin = a.pin(Access::Scan, 1_000_000).unwrap();
    let baseline = pin.raw();
    drop(pin);
    assert!(budget.report().total < budget.report().limit);
    assert!(
        b.pin(Access::Read, 1_000_000).is_ok(),
        "a small retained baseline kept an obsolete load peak exclusive"
    );
    assert_eq!(baseline.bytes(), b"# A\n");
}

#[test]
fn migration_refusal_does_not_credit_unshrinkable_tables() {
    let parent = tempfile::tempdir().unwrap();
    let store = RawStore::new(parent.path()).unwrap();
    let budget = Controller::new(0);
    let mut leased = Vec::new();
    let mut leased_members = Vec::new();
    for i in 0..20 {
        let member = member_of(&budget, &store, format!("# L{i}\n").as_bytes());
        let pin = member.pin(Access::Read, 131_072).unwrap();
        leased.push(pin.raw());
        drop(pin);
        leased_members.push(member);
    }
    budget.set_limit(1); // shed the leased bodies; their raw stays leased
    budget.set_limit(0);
    let a = member_of(&budget, &store, b"# A\n\nunrelated cached body\n");
    let b = member_of(&budget, &store, b"# B\n");
    drop(a.pin(Access::Read, 131_072).unwrap());
    let warm = budget.reserve(1).unwrap(); // size the permit table first
    let staging = budget.reserve(100_000).unwrap();
    let peak: u128 = 131_072;
    let (floor, total, exact, upper, reclaimable) = {
        let state = lock(&budget.0.ledger);
        (
            state.floor_exact(),
            state.total_exact(),
            state.reclaimable_bytes,
            state.reclaim_upper_bound(),
            state.reclaimable,
        )
    };
    let slack = {
        let state = lock(&budget.0.ledger);
        (state.raw.allocation_size() + state.raw_versions.allocation_size()) as u128
    };
    let limit = total - exact + peak - 1 - slack / 2;
    assert!(limit >= total && limit - floor >= peak && total + peak > limit);
    assert!(total - exact + peak > limit);
    budget.set_limit(usize::try_from(limit).unwrap());
    let a_hot_before = a.phase() == Phase::Hot;
    let result = b.pin(Access::Read, 131_072);
    let a_hot_after = a.phase() == Phase::Hot;
    let after = lock(&budget.0.ledger).total_exact();
    println!(
        "rv341b-demo-slack: reclaimable={reclaimable} exact_reclaim={exact} upper_bound={upper} table_slack={slack} limit={limit} total_before={total} total_after={after} b_peak={peak} b_result={:?} a_hot_before={a_hot_before} a_hot_after={a_hot_after}",
        result.as_ref().map(|_| "pinned"),
    );
    let refused = matches!(result, Err(Error::Admission(AdmissionError::Busy)));
    drop(result);
    drop((staging, warm, leased, leased_members));
    assert!(refused, "probe precondition: B is refused");
    assert!(a_hot_after, "a refusal certain after shedding still shed A");
}
