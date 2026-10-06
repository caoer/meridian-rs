//! The host floor (merkle-spec §6.4): a registry built through a residency
//! host charges its controller one floor, the census of everything it
//! reaches, after each publish and eviction and at the reaper's tick.

use std::path::Path;
use std::sync::Arc;

use registry::raw_store::RawStore;
use registry::residency::Host;
use registry::residency::controller::Controller;

fn member(n: usize) -> String {
    format!(
        "---\naliases: [alias-{n}]\n---\n# Member {n}\n\nLinks [[member-{}]].\n\ntext ^b{n}\n",
        (n + 1) % 64
    )
}

fn write(root: &Path, rel: &str, body: &str) {
    let file = root.join(rel);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, body).unwrap();
}

#[test]
fn the_host_floor_is_the_registry_census_at_each_publish_and_eviction() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    for n in 0..64 {
        write(&ws, &format!("dir-{}/member-{n}.md", n % 5), &member(n));
    }
    let ws = std::fs::canonicalize(&ws).unwrap();
    let backing = tmp.path().join("backing");
    let host = Arc::new(Host::new(
        Controller::new(1 << 30),
        RawStore::new(&backing).unwrap(),
        1 << 20,
    ));
    let mut config = registry::Config::for_cache_root(tmp.path().join("cache"));
    config.socket_path = tmp.path().join("daemon.sock");
    config.state_path = tmp.path().join("state.json");
    config.residency = Some(Arc::clone(&host));
    let reg = registry::in_process_registry(&config).unwrap();
    let census = |reg: &registry::Registry| reg.floor_census().expect("the store's census reads");
    reg.register(&ws);

    // A publish charges the census.
    reg.warm_or_build(&ws).unwrap();
    let first = host.floor();
    assert!(first > 0, "a published generation charges the host floor");
    assert_eq!(
        first,
        census(&reg),
        "after a publish the floor is the census"
    );

    // A ring is charged from the reaper's next tick.
    let ring = reg.ring(&ws);
    ring.prime(&fs::WorkspaceRoot(ws.clone()), &reg)
        .expect("baseline prime");
    let ring_heap = ring.heap_bytes();
    assert!(ring_heap > 0, "a primed ring holds its baseline map");
    reg.charge_floor();
    let ringed = host.floor();
    assert_eq!(ringed, census(&reg));
    assert!(
        ringed >= first + ring_heap,
        "the ring's frames and watch state are in the floor: {ringed} < {first} + {ring_heap}"
    );

    // A generation an operation holds past a newer publish stays charged.
    let held = reg.engine_snapshot(&ws).expect("a warm generation");
    write(&ws, "dir-0/member-64.md", &member(64));
    reg.warm_or_build(&ws).unwrap();
    let with_held = host.floor();
    assert_eq!(with_held, census(&reg), "after a second publish");
    assert!(
        with_held > ringed,
        "the new generation and the held one: {with_held} <= {ringed}"
    );

    // Released, it leaves the floor at the next census.
    drop(held);
    reg.charge_floor();
    let released = host.floor();
    assert_eq!(released, census(&reg));
    assert!(
        released < with_held,
        "the released generation's metadata left the floor: {released} >= {with_held}"
    );

    // An eviction re-censuses: the memo is gone.
    assert!(reg.unregister(&ws));
    let evicted = host.floor();
    assert_eq!(evicted, census(&reg), "after an eviction");
    assert!(
        evicted < released,
        "the evicted memo left the floor: {evicted} >= {released}"
    );
}
