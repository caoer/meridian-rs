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
    let provider = Arc::new(Retention::new(controller.clone(), store.clone(), 131_072));
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
    let provider = Retention::new(controller.clone(), store, 131_072);
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
