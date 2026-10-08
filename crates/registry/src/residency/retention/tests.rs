use super::*;
use wire_serve::ring::RootRing;
use wire_serve::watch::{WatchState, reconcile};

#[test]
fn watcher_and_serving_share_a_cold_version_across_source_replacement() {
    let work = tempfile::tempdir().unwrap();
    let backing = tempfile::tempdir().unwrap();
    let root = fs::WorkspaceRoot(work.path().to_path_buf());
    let before = b"# Historical\n\nkept bytes\n";
    std::fs::write(root.0.join("a.md"), before).unwrap();
    std::fs::write(root.0.join("b.md"), b"# Removed\n").unwrap();
    let store = Arc::new(RawStore::new(backing.path()).unwrap());
    let controller = Controller::new(0);
    let provider = Arc::new(Retention::new(controller.clone(), store.clone()));
    let mut watch = WatchState::with_retention(&root, provider);
    let mut ring = RootRing::new();
    reconcile(&root, &mut ring, &mut watch).unwrap();
    assert_eq!(controller.report().members, 2);
    assert_eq!(
        controller.report().bodies,
        0,
        "baseline descriptors must start cold"
    );
    let serving = controller.retain(Arc::new(store.retain(before).unwrap()));
    let read = serving.pin(Access::Read, 131_072).unwrap();
    let rev = read.document().unwrap().root.node_rev.clone();
    assert_eq!(
        controller.report().members,
        2,
        "watcher and serving duplicated the slot"
    );
    drop(read);
    controller.set_limit(1);
    assert_eq!(serving.phase(), super::super::Phase::Cold);
    let instance = ring.instance().to_owned();
    std::fs::write(root.0.join("a.md"), b"# Current\n").unwrap();
    std::fs::remove_file(root.0.join("b.md")).unwrap();
    let frame = reconcile(&root, &mut ring, &mut watch).unwrap().unwrap();
    assert_eq!(frame.delta.seq, 1);
    assert_eq!(ring.instance(), instance);
    assert_eq!(frame.delta.files.len(), 2);
    let old = serving.pin(Access::Read, 131_072).unwrap();
    assert_eq!(old.bytes(), before);
    assert_eq!(old.document().unwrap().root.node_rev, rev);
    assert_eq!(
        controller.report().members,
        2,
        "old serving version and new baseline must coexist"
    );
    drop(old);
    drop(serving);
    assert_eq!(controller.report().members, 1);
    drop(watch);
    assert_eq!(controller.report().members, 0);
    assert_eq!(controller.report().raw, 0);
}

#[test]
fn a_managed_baseline_lease_keeps_bytes_and_charge_until_release() {
    let dir = tempfile::tempdir().unwrap();
    let controller = Controller::new(1);
    let store = Arc::new(RawStore::new(dir.path()).unwrap());
    let provider = Retention::new(controller.clone(), store);
    let version = provider.retain(b"# Lease\n".to_vec()).unwrap();
    let lease = version.pin().unwrap();
    drop(version);
    drop(provider);
    assert_eq!(lease.bytes(), b"# Lease\n");
    assert!(controller.report().raw > 0);
    assert_eq!(controller.report().bodies, 0);
    drop(lease);
    assert_eq!(controller.report().raw, 0);
    assert!(!controller.report().transient);
}

/// rv-360 P2-1: a baseline version loads under its length rule. With another
/// lease held so the load is not oversized, an allowance 16 KiB short of
/// `load_peak(len)` refuses a 64 KiB version and 4 KiB over it admits it. The
/// margins cover the ledger tables admission reclaims or grows, and stay
/// below every wrong declaration's error (half the length, no fixed part,
/// the old 1 MiB constant).
#[test]
fn a_baseline_version_is_admitted_under_exactly_its_length_rule() {
    const SHORT: usize = 16 * 1024;
    const TABLES: usize = 4 * 1024;
    let dir = tempfile::tempdir().unwrap();
    let controller = Controller::new(0);
    let store = Arc::new(RawStore::new(dir.path()).unwrap());
    let provider = Retention::new(controller.clone(), store);
    // The held raw bytes exceed the shortfall, so the load stays below the
    // oversized regime, which admits a load alone whatever the allowance.
    let held = provider
        .retain(
            "bytes another lease keeps resident\n"
                .repeat(1024)
                .into_bytes(),
        )
        .unwrap();
    let lease = held.pin().unwrap();
    // Shed the held version's cached body: only its raw lease stays.
    controller.set_limit(1);
    assert_eq!(controller.report().bodies, 0);
    let watched = "plain words of a watched page\n".repeat(2260);
    assert!(watched.len() > 64 * 1024);
    let version = provider.retain(watched.clone().into_bytes()).unwrap();
    let peak = crate::residency::load_peak(version.identity().len);
    let room = |bytes: usize| controller.set_limit(controller.report().total + bytes);
    room(peak - SHORT);
    let report = controller.report();
    assert!(
        peak <= report.limit - report.floor,
        "the load is not oversized: {report:?}"
    );
    assert!(
        version.pin().is_err(),
        "admitted 16 KiB short of its declaration"
    );
    room(peak + TABLES);
    let pin = version.pin().expect("admitted under its declaration");
    assert_eq!(pin.bytes(), watched.as_bytes());
    drop(pin);
    drop(lease);
}

/// A baseline pin goes through the catalog's admission and retry: a load
/// refused for occupied allowance reaches the watcher as a transient fault,
/// not a plain I/O error, and is served once the allowance frees.
#[test]
fn a_baseline_pin_refused_for_occupied_allowance_is_transient() {
    const LIVE: usize = 2_000;
    let dir = tempfile::tempdir().unwrap();
    let controller = Controller::new(0);
    let store = Arc::new(RawStore::new(dir.path()).unwrap());
    let provider = Retention::new(controller.clone(), store);
    let version = provider.retain(b"# Watched\n\nbody\n".to_vec()).unwrap();
    let peak = crate::residency::load_peak(version.identity().len);
    let live = controller.reserve(LIVE).expect("another live load");
    // The load fits the allowance above the floor, but not beside the live one.
    controller.set_limit(controller.report().total + peak - LIVE / 2);
    let Err(refused) = version.pin() else {
        panic!("admitted beside the live load");
    };
    assert!(model::catalog::is_transient(&refused), "{refused}");
    drop(live);
    let lease = version.pin().expect("served once the live load ended");
    assert_eq!(lease.bytes(), b"# Watched\n\nbody\n");
}
