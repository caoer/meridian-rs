//! The host floor (merkle-spec §6.4): a registry built through a residency
//! host charges its controller one floor, the census of everything it
//! reaches, after each publish and eviction and at the reaper's tick.

use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

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

/// An in-process registry built through a host with a limit no test reaches.
fn hosted(tmp: &Path) -> (Arc<Host>, registry::Registry) {
    let host = Arc::new(Host::new(
        Controller::new(1 << 30),
        RawStore::new(&tmp.join("backing")).unwrap(),
        1 << 20,
    ));
    let mut config = registry::Config::for_cache_root(tmp.join("cache"));
    config.socket_path = tmp.join("daemon.sock");
    config.state_path = tmp.join("state.json");
    config.residency = Some(Arc::clone(&host));
    let reg = registry::in_process_registry(&config).unwrap();
    (host, reg)
}

/// A workspace of `members` pages under `root`, canonical.
fn seeded(root: &Path, members: usize) -> PathBuf {
    for n in 0..members {
        write(root, &format!("dir-{}/member-{n}.md", n % 5), &member(n));
    }
    std::fs::canonicalize(root).unwrap()
}

/// Hold `memo` on another thread until the returned sender drops.
fn held(
    memo: Arc<std::sync::Mutex<fs::DomainCache>>,
) -> (mpsc::Sender<()>, std::thread::JoinHandle<()>) {
    let (locked, is_held) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let holder = std::thread::spawn(move || {
        let _guard = memo.lock().unwrap();
        locked.send(()).unwrap();
        let _ = released.recv();
    });
    is_held.recv().unwrap();
    (release, holder)
}

/// How long a census may take before the test calls it waiting.
const PROMPT: Duration = Duration::from_secs(10);

/// A census never waits on a memo (merkle-spec §6.4, the host floor): a
/// publish on one workspace finishes while another workspace's memo is held,
/// and charges that memo what its last measurement found.
#[test]
fn a_publish_does_not_wait_while_another_workspaces_memo_is_held() {
    let tmp = tempfile::tempdir().unwrap();
    let (host, reg) = hosted(tmp.path());
    let reg = Arc::new(reg);
    let a = seeded(&tmp.path().join("a"), 8);
    let b = seeded(&tmp.path().join("b"), 8);
    for ws in [&a, &b] {
        reg.register(ws);
        reg.warm_or_build(ws).unwrap();
    }
    // Borrowing a memo applies its feed's pending set: before the census
    // whose measurement the held memo is charged at.
    let memo = reg.domain_cache(&b);
    reg.charge_floor();
    let (release, holder) = held(memo);
    write(&a, "dir-0/member-0.md", "# Member 0\n\nrewritten\n");
    let (done, finished) = mpsc::channel();
    let publishing = {
        let (reg, a) = (Arc::clone(&reg), a.clone());
        std::thread::spawn(move || {
            reg.warm_or_build(&a).unwrap();
            let _ = done.send(());
        })
    };
    let published = finished.recv_timeout(PROMPT);
    let floor = host.floor();
    drop(release);
    holder.join().unwrap();
    publishing.join().unwrap();
    assert!(
        published.is_ok(),
        "a publish on one workspace waited on another workspace's held memo"
    );
    assert_eq!(
        floor,
        reg.floor_census().unwrap(),
        "the held memo was charged its last measurement"
    );
}

/// An idle reap's census runs at once beside a held memo (merkle-spec §6.4,
/// the host floor): the reap does not wait, and the floor it leaves is the
/// census of what remains, the evicted generations gone.
#[test]
fn an_idle_reap_charges_what_remains_while_another_workspaces_memo_is_held() {
    let tmp = tempfile::tempdir().unwrap();
    let (host, reg) = hosted(tmp.path());
    let reg = Arc::new(reg);
    let roots: Vec<PathBuf> = ["a", "b", "c", "d"]
        .iter()
        .map(|name| seeded(&tmp.path().join(name), 64))
        .collect();
    for ws in &roots {
        reg.register(ws);
        reg.warm_or_build(ws).unwrap();
    }
    let _sub = reg.subscribe(&roots[3]);
    // Borrowing a memo applies its feed's pending set: before the census
    // whose measurement the held memo is charged at.
    let memo = reg.domain_cache(&roots[3]);
    reg.charge_floor();
    let warm = host.floor();

    let (release, holder) = held(memo);
    let (done, finished) = mpsc::channel();
    let reaping = {
        let reg = Arc::clone(&reg);
        std::thread::spawn(move || {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            let _ = done.send(reg.reap(now + 3600, 0));
        })
    };
    let reaped = finished.recv_timeout(PROMPT);
    let after_reap = host.floor();
    drop(release);
    holder.join().unwrap();
    reaping.join().unwrap();
    let reaped = reaped.expect("an idle reap waited on a subscribed workspace's held memo");
    assert_eq!(
        reaped.len(),
        3,
        "the three unsubscribed workspaces: {reaped:?}"
    );
    assert!(
        after_reap < warm,
        "the reap's own census dropped the floor: {after_reap} >= {warm}"
    );
    reg.charge_floor();
    assert_eq!(
        after_reap,
        host.floor(),
        "the reap's census charged what remains, the held memo at its last measurement"
    );
}

/// The reaper's tick is the census point for ring growth no publish or
/// eviction saw (merkle-spec §6.4, the host floor): a ring primed after the
/// publish is in the floor once a tick has run.
#[allow(clippy::duration_suboptimal_units)]
#[test]
fn the_reapers_tick_charges_a_ring_primed_after_the_publish() {
    let forever = Duration::from_secs(365 * 24 * 60 * 60);
    let tmp = tempfile::tempdir().unwrap();
    let ws = seeded(&tmp.path().join("ws"), 64);
    let host = Arc::new(Host::new(
        Controller::new(1 << 30),
        RawStore::new(&tmp.path().join("backing")).unwrap(),
        1 << 20,
    ));
    let dir = tmp.path().join("registry");
    let mut config = registry::Config::for_cache_root(tmp.path().join("cache"));
    config.socket_path = dir.join("daemon.sock");
    config.state_path = dir.join("state.json");
    config.idle_threshold = forever;
    config.reap_interval = Duration::from_millis(300);
    config.prewarm_interval = forever;
    config.prewarm_quiet_max = forever;
    config.idle_exit = None;
    config.drain_cold_builds = Duration::from_secs(30);
    config.residency = Some(Arc::clone(&host));
    let server = registry::RunningServer::start(config).unwrap();
    let reg = server.registry();
    reg.register(&ws);
    reg.warm_or_build(&ws).unwrap();
    let published = host.floor();
    let ring = reg.ring(&ws);
    ring.prime(&fs::WorkspaceRoot(ws.clone()), reg)
        .expect("baseline prime");
    let ring_heap = ring.heap_bytes();
    assert!(ring_heap > 0, "a primed ring holds its baseline map");

    let started = Instant::now();
    while host.floor() < published + ring_heap && started.elapsed() < PROMPT {
        std::thread::sleep(Duration::from_millis(50));
    }
    let ticked = host.floor();
    drop(ring);
    server.shutdown();
    assert!(
        ticked >= published + ring_heap,
        "the reaper's tick charged the primed ring: {ticked} < {published} + {ring_heap}"
    );
}

/// The host's record of published generations is pruned at each census
/// (merkle-spec §6.4, the host floor): many publishes with nothing held
/// leave the floor where the same content left it before them.
#[test]
fn publishes_with_nothing_held_leave_no_tracking_in_the_floor() {
    let tmp = tempfile::tempdir().unwrap();
    let (host, reg) = hosted(tmp.path());
    let ws = seeded(&tmp.path().join("ws"), 8);
    reg.register(&ws);
    reg.warm_or_build(&ws).unwrap();
    let publish = |n: usize| {
        let body = format!("# Member 0\n\n{}\n", "edit ".repeat(1 + n % 2));
        write(&ws, "dir-0/member-0.md", &body);
        assert!(
            matches!(
                reg.warm_or_build(&ws).unwrap(),
                registry::WarmOutcome::Built { .. }
            ),
            "edit {n} was published"
        );
        host.floor()
    };
    publish(0);
    let settled = publish(1);
    let mut after = settled;
    for n in 2..514 {
        after = publish(n);
    }
    assert!(
        after <= settled + 1024,
        "512 publishes later the same content charges {after}, against {settled}"
    );
}
