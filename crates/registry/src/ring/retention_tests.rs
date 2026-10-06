use super::*;
use model::bytes::{ByteIdentity, ByteLease, ByteRetention, RetainedBytes, SharedBytes};
use std::io;
use std::sync::atomic::AtomicUsize;

#[derive(Debug, Default)]
struct Counting(AtomicUsize, Arc<std::sync::atomic::AtomicBool>);
#[derive(Debug)]
struct Version(SharedBytes, Arc<std::sync::atomic::AtomicBool>);
impl Drop for Version {
    fn drop(&mut self) {
        assert!(
            !self.1.swap(false, Ordering::SeqCst),
            "injected retired driver version unwind"
        );
    }
}
impl RetainedBytes for Version {
    fn identity(&self) -> ByteIdentity {
        ByteIdentity {
            digest: model::leaf_digest(self.0.as_bytes()),
            len: self.0.len() as u64,
        }
    }
    fn pin(&self) -> io::Result<Box<dyn ByteLease>> {
        Ok(Box::new(self.0.clone()))
    }
}
impl ByteRetention for Counting {
    fn retain(&self, bytes: Vec<u8>) -> io::Result<Arc<dyn RetainedBytes>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::new(Version(SharedBytes::from(bytes), self.1.clone())))
    }
}
fn setup() -> (tempfile::TempDir, fs::WorkspaceRoot, crate::Registry) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("ws");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.md"), b"# A\n").unwrap();
    std::fs::write(root.join("b.md"), b"# B\n").unwrap();
    let mut config = crate::Config::for_cache_root(dir.path().join("cache"));
    config.socket_path = dir.path().join("daemon.sock");
    config.state_path = dir.path().join("state.json");
    let registry = crate::in_process_registry(&config).unwrap();
    (
        dir,
        fs::WorkspaceRoot(std::fs::canonicalize(root).unwrap()),
        registry,
    )
}
#[test]
fn driver_poisoned_gate_is_not_a_permanent_cycle_in_flight() {
    let (_dir, root, registry) = setup();
    let ring = WorkspaceRing::new(&root);
    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _held = ring.cycle_gate.lock().unwrap();
        panic!("injected prior cycle unwind");
    }));
    assert!(unwind.is_err());
    ring.detect(&root, &registry).unwrap();
    assert!(
        ring.state().watch.root().is_some(),
        "poisoned gate permanently skipped detection"
    );
}
#[test]
fn driver_prepares_retention_before_the_write_flock() {
    let (_dir, root, registry) = setup();
    let backing = Arc::new(Counting::default());
    let ring = WorkspaceRing::with_retention(&root, backing.clone());
    let write_lock = fs::WriteLock::acquire(&root).unwrap();
    let (announce, announced) = std::sync::mpsc::channel();
    let (resume, resumed) = std::sync::mpsc::channel();
    *ring.pause_after_prime_snapshot.lock().unwrap() = Some((announce, resumed));
    let count = std::thread::scope(|scope| {
        let prime = scope.spawn(|| ring.prime(&root, &registry));
        let reached = announced.recv_timeout(Duration::from_secs(10));
        let count = backing.0.load(Ordering::SeqCst);
        drop(write_lock);
        resume.send(()).unwrap();
        prime.join().unwrap().unwrap();
        reached.unwrap();
        count
    });
    assert_eq!(
        count, 2,
        "retention preparation stayed inside the write flock"
    );
    assert_eq!(ring.seq(), 0);
}

#[test]
fn an_active_cycle_still_coalesces_without_priming_twice() {
    let (_dir, root, registry) = setup();
    let ring = WorkspaceRing::new(&root);
    let active = ring.cycle_gate.lock().unwrap();
    assert!(!ring.detect(&root, &registry).unwrap());
    assert!(ring.state().watch.root().is_none());
    drop(active);
    ring.detect(&root, &registry).unwrap();
    assert!(ring.state().watch.root().is_some());
}

#[test]
fn a_retired_version_unwind_cannot_disable_future_detection() {
    let (_dir, root, registry) = setup();
    let backing = Arc::new(Counting::default());
    let ring = WorkspaceRing::with_retention(&root, backing.clone());
    ring.prime(&root, &registry).unwrap();
    std::fs::write(root.0.join("a.md"), b"# Changed once\n").unwrap();
    {
        let mut state = ring.state();
        state.last_detect = None;
        state.last_floor = None;
    }
    backing.1.store(true, Ordering::SeqCst);
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || ring.detect(&root, &registry)
        ))
        .is_err()
    );
    assert_eq!(ring.seq(), 1);
    std::fs::write(root.0.join("a.md"), b"# Changed a second time\n").unwrap();
    {
        let mut state = ring.state();
        state.last_detect = None;
        state.last_floor = None;
    }
    assert!(ring.detect(&root, &registry).unwrap());
    let frames = ring.frames_after(0);
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[1].delta.seq, 2);
    assert_eq!(frames[1].delta.root_before, frames[0].delta.root_after);
}

#[test]
fn retained_priming_reconciles_changes_after_its_unlocked_capture() {
    let (_dir, root, registry) = setup();
    let backing = Arc::new(Counting::default());
    let ring = WorkspaceRing::with_retention(&root, backing.clone());
    let (announce, announced) = std::sync::mpsc::channel();
    let (resume, resumed) = std::sync::mpsc::channel();
    *ring.pause_after_prime_snapshot.lock().unwrap() = Some((announce, resumed));
    let (observed, seq) = std::thread::scope(|scope| {
        let prime = scope.spawn(|| ring.prime(&root, &registry));
        announced.recv_timeout(Duration::from_secs(10)).unwrap();
        std::fs::write(root.0.join("a.md"), b"# After capture\n").unwrap();
        std::fs::remove_file(root.0.join("b.md")).unwrap();
        std::fs::write(root.0.join("c.md"), b"# Arrived\n").unwrap();
        resume.send(()).unwrap();
        prime.join().unwrap().unwrap()
    });
    assert_eq!(observed.0, fs::domain_snapshot(&root).unwrap().1.0);
    assert_eq!(seq, 0);
    assert_eq!(
        backing.0.load(Ordering::SeqCst),
        4,
        "only two changed/new versions should prepare under the flock"
    );
    std::fs::write(root.0.join("a.md"), b"# A\n").unwrap();
    std::fs::write(root.0.join("b.md"), b"# B\n").unwrap();
    std::fs::remove_file(root.0.join("c.md")).unwrap();
    assert!(ring.cycle(&root, &registry, true).unwrap());
    let frames = ring.frames_after(0);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].delta.root_before, observed);
    assert_eq!(frames[0].delta.files.len(), 3);
}

fn frame_rows(frame: &DeltaFrame) -> Vec<(String, wire::FileChange)> {
    let mut rows: Vec<_> = frame
        .delta
        .files
        .iter()
        .map(|file| (file.path.0.clone(), file.change))
        .collect();
    rows.sort_by(|left, right| left.0.cmp(&right.0));
    rows
}

#[test]
fn capture_contract_content_only_rewrite_uses_the_locked_bytes() {
    let (_dir, root, registry) = setup();
    let ring = WorkspaceRing::new(&root);
    let (announce, announced) = std::sync::mpsc::channel();
    let (resume, resumed) = std::sync::mpsc::channel();
    *ring.pause_after_prime_snapshot.lock().unwrap() = Some((announce, resumed));
    let (ack, seq) = std::thread::scope(|scope| {
        let prime = scope.spawn(|| ring.prime(&root, &registry));
        announced.recv_timeout(Duration::from_secs(10)).unwrap();
        std::fs::write(root.0.join("a.md"), b"# Changed after capture\n").unwrap();
        resume.send(()).unwrap();
        prime.join().unwrap().unwrap()
    });
    assert_eq!(ack.0, fs::domain_snapshot(&root).unwrap().1.0);
    assert_eq!(seq, 0);
    std::fs::write(root.0.join("a.md"), b"# A\n").unwrap();
    assert!(ring.cycle(&root, &registry, true).unwrap());
    let frames = ring.frames_after(0);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].delta.root_before, ack);
    assert_eq!(
        frame_rows(&frames[0]),
        vec![("a.md".to_owned(), wire::FileChange::Modified)],
        "captured content cannot masquerade as the locked baseline"
    );
}

fn tail_capture_contract(add: bool) {
    let (_dir, root, registry) = setup();
    let ring = WorkspaceRing::new(&root);
    let path = if add { "z.md" } else { "b.md" };
    let (announce, announced) = std::sync::mpsc::channel();
    let (resume, resumed) = std::sync::mpsc::channel();
    *ring.pause_after_prime_snapshot.lock().unwrap() = Some((announce, resumed));
    let (ack, seq) = std::thread::scope(|scope| {
        let prime = scope.spawn(|| ring.prime(&root, &registry));
        announced.recv_timeout(Duration::from_secs(10)).unwrap();
        if add {
            std::fs::write(root.0.join(path), b"# Last\n").unwrap();
        } else {
            std::fs::remove_file(root.0.join(path)).unwrap();
        }
        resume.send(()).unwrap();
        prime.join().unwrap().unwrap()
    });
    assert_eq!(ack.0, fs::domain_snapshot(&root).unwrap().1.0);
    assert_eq!(seq, 0);
    let expected = if add {
        std::fs::remove_file(root.0.join(path)).unwrap();
        wire::FileChange::Deleted
    } else {
        std::fs::write(root.0.join(path), b"# B\n").unwrap();
        wire::FileChange::Created
    };
    assert!(ring.cycle(&root, &registry, true).unwrap());
    let frames = ring.frames_after(0);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].delta.root_before, ack);
    assert_eq!(
        frame_rows(&frames[0]),
        vec![(path.to_owned(), expected)],
        "last-sorted member must reflect the locked membership"
    );
}
#[test]
fn capture_contract_tail_addition_is_not_lost() {
    tail_capture_contract(true);
}
#[test]
fn capture_contract_tail_deletion_is_not_retained() {
    tail_capture_contract(false);
}

/// Six members, each long enough that a body is worth shedding.
fn hosted_setup(
    host: Option<Arc<crate::residency::Host>>,
) -> (tempfile::TempDir, fs::WorkspaceRoot, crate::Registry) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("ws");
    std::fs::create_dir_all(&root).unwrap();
    let body = "a line that links [[m1]] and keeps going\n".repeat(64);
    for n in 0..6 {
        std::fs::write(root.join(format!("m{n}.md")), format!("# M{n}\n\n{body}")).unwrap();
    }
    let mut config = crate::Config::for_cache_root(dir.path().join("cache"));
    config.socket_path = dir.path().join("daemon.sock");
    config.state_path = dir.path().join("state.json");
    config.residency = host;
    let registry = crate::in_process_registry(&config).unwrap();
    let root = fs::WorkspaceRoot(std::fs::canonicalize(root).unwrap());
    registry.register(&root.0);
    (dir, root, registry)
}

/// One external batch told by a primed ring: a rewrite, a removal, an arrival.
fn external_batch(root: &fs::WorkspaceRoot, ring: &WorkspaceRing, registry: &crate::Registry) {
    std::fs::write(root.0.join("m2.md"), "# M2\n\nrewritten, back to [[m1]]\n").unwrap();
    std::fs::remove_file(root.0.join("m4.md")).unwrap();
    std::fs::write(root.0.join("m9.md"), "# M9\n\nnew\n").unwrap();
    assert!(ring.cycle(root, registry, true).unwrap());
}

#[test]
fn a_hosted_registry_rings_retain_their_baseline_through_the_host() {
    let backing = tempfile::tempdir().unwrap();
    let host = Arc::new(crate::residency::Host::new(
        crate::residency::controller::Controller::new(0),
        crate::raw_store::RawStore::new(backing.path()).unwrap(),
        1 << 20,
    ));
    let (_dir, root, hosted) = hosted_setup(Some(Arc::clone(&host)));
    let ring = hosted.ring(&root.0);
    ring.prime(&root, &hosted).unwrap();
    assert_eq!(
        host.controller.report().members,
        6,
        "the ring's baseline versions are the host's controller members"
    );
    hosted.warm_or_build(&root.0).unwrap();
    assert_eq!(
        host.controller.report().members,
        6,
        "a serving slot and a baseline of the same bytes are one member"
    );
    host.controller.set_limit(1);
    assert_eq!(host.controller.report().bodies, 0, "every body is cold");
    host.controller.set_limit(0);

    let (_eager_dir, eager_root, eager) = hosted_setup(None);
    let eager_ring = eager.ring(&eager_root.0);
    eager_ring.prime(&eager_root, &eager).unwrap();
    external_batch(&root, &ring, &hosted);
    external_batch(&eager_root, &eager_ring, &eager);
    let (told, want) = (ring.frames_after(0), eager_ring.frames_after(0));
    assert_eq!(told.len(), 1);
    assert_eq!(
        told[0].delta, want[0].delta,
        "a cold retained baseline tells the eager baseline's frame"
    );
}

#[test]
fn a_door_step_on_a_memo_moved_past_its_observation_tells_nothing() {
    let (_dir, root, registry) = setup();
    let ring = WorkspaceRing::new(&root);
    let (baseline, _) = ring.prime(&root, &registry).unwrap();
    let cache = registry.domain_cache(&root.0);
    std::fs::write(root.0.join("a.md"), b"# Seen by the door\n").unwrap();
    let door_saw = observe_past(&registry, &root, &cache, &baseline);
    // A reader observes a later edit before the door's step reads the memo.
    std::fs::write(root.0.join("a.md"), b"# Seen by a reader, later\n").unwrap();
    let reader_saw = observe_past(&registry, &root, &cache, &door_saw);
    assert_ne!(door_saw, reader_saw);

    let told = ring.reconcile_observed(&root, &cache, &door_saw).unwrap();
    assert!(
        !told && ring.frames_after(0).is_empty(),
        "a frame stamped {door_saw:?} over bytes the memo folds to {reader_saw:?}: {:?}",
        ring.frames_after(0)
    );
    assert_eq!(ring.state().watch.root(), Some(&baseline));

    assert!(ring.cycle(&root, &registry, true).unwrap());
    let frames = ring.frames_after(0);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].delta.root_before, baseline);
    assert_eq!(frames[0].delta.root_after, reader_saw);
}

/// A write door's settle step, raced: a reader observes a late rewrite
/// between the door's observation and its settle step (with an edit pending
/// at door entry when `pending`). The step classifies the memo's own pair,
/// so the settle frame tells the late rewrite under the memo's root — the
/// root a splice's or a set's `root_before` is taken from. The reader keeps
/// observing until it sees the write ([`super::observe_past`]).
fn settle_race(pending: bool) {
    let (_dir, root, registry) = setup();
    let ring = WorkspaceRing::new(&root);
    let (baseline, _) = ring.prime(&root, &registry).unwrap();
    let cache = registry.domain_cache(&root.0);
    let mut pre = baseline.clone();
    if pending {
        std::fs::write(root.0.join("a.md"), b"# Pending at the door\n").unwrap();
        pre = observe_past(&registry, &root, &cache, &baseline);
    }
    let (announce, announced) = std::sync::mpsc::channel();
    let (resume, resumed) = std::sync::mpsc::channel();
    *ring.pause_after_door_observation.lock().unwrap() = Some((announce, resumed));
    let plane = DoorPlane::new(&registry, &root, &ring, &cache);
    let (door_saw, reader_saw) = std::thread::scope(|scope| {
        let door = scope.spawn(|| plane.observe().unwrap());
        announced.recv_timeout(Duration::from_secs(10)).unwrap();
        std::fs::write(root.0.join("b.md"), b"# B, rewritten late\n").unwrap();
        let reader_saw = observe_past(&registry, &root, &cache, &pre);
        resume.send(()).unwrap();
        (Root(door.join().unwrap().0), reader_saw)
    });
    assert_ne!(door_saw, reader_saw);
    let frames = ring.frames_after(0);
    assert_eq!(frames.len(), 1, "one settle frame: {frames:?}");
    assert_eq!(frames[0].delta.root_before, baseline);
    assert_eq!(
        frames[0].delta.root_after, reader_saw,
        "the settle frame ends at the memo's root, which the commit starts from"
    );
    let mut want = vec![("b.md".to_owned(), wire::FileChange::Modified)];
    if pending {
        want.insert(0, ("a.md".to_owned(), wire::FileChange::Modified));
    }
    assert_eq!(frame_rows(&frames[0]), want);
    assert_eq!(ring.state().watch.root(), Some(&reader_saw));
}

#[test]
fn a_door_settles_onto_the_memo_a_reader_moved_past_its_pending_change() {
    settle_race(true);
}

#[test]
fn a_door_settles_onto_the_memo_a_reader_moved_past_a_quiet_observation() {
    settle_race(false);
}

/// The detector's flock-held cycle, raced: a reader moves the memo between
/// the cycle's observation and its read of the memo's pair. The cycle
/// classifies the pair it reads, so one frame tells both changes under the
/// memo's root.
#[test]
fn a_detect_cycle_classifies_the_memo_a_reader_moved_after_its_observation() {
    let (_dir, root, registry) = setup();
    let ring = WorkspaceRing::new(&root);
    let (baseline, _) = ring.prime(&root, &registry).unwrap();
    let cache = registry.domain_cache(&root.0);
    std::fs::write(root.0.join("a.md"), b"# Seen by the cycle\n").unwrap();
    let pre = observe_past(&registry, &root, &cache, &baseline);
    let (announce, announced) = std::sync::mpsc::channel();
    let (resume, resumed) = std::sync::mpsc::channel();
    *ring.pause_after_detect_observation.lock().unwrap() = Some((announce, resumed));
    let (told, reader_saw) = std::thread::scope(|scope| {
        let cycle = scope.spawn(|| ring.cycle(&root, &registry, true).unwrap());
        announced.recv_timeout(Duration::from_secs(10)).unwrap();
        std::fs::write(root.0.join("b.md"), b"# B, seen by a reader\n").unwrap();
        let reader_saw = observe_past(&registry, &root, &cache, &pre);
        resume.send(()).unwrap();
        (cycle.join().unwrap(), reader_saw)
    });
    assert!(told);
    let frames = ring.frames_after(0);
    assert_eq!(frames.len(), 1, "one frame: {frames:?}");
    assert_eq!(frames[0].delta.root_before, baseline);
    assert_eq!(
        frames[0].delta.root_after, reader_saw,
        "a frame stamped with a root other than the memo's it classified"
    );
    assert_eq!(
        frame_rows(&frames[0]),
        vec![
            ("a.md".to_owned(), wire::FileChange::Modified),
            ("b.md".to_owned(), wire::FileChange::Modified),
        ]
    );
}

/// `memo_pair` holds the memo across its root read and its leaves read, so
/// no reader can move the memo between them: parked between the two reads,
/// the memo is still locked. A reader observes the write first
/// ([`super::observe_past`]), so the cycle finds the memo past its baseline
/// and reaches `memo_pair` whatever its own barrier vouches.
#[test]
fn memo_pair_holds_the_memo_across_both_reads() {
    let (_dir, root, registry) = setup();
    let ring = WorkspaceRing::new(&root);
    let (baseline, _) = ring.prime(&root, &registry).unwrap();
    let cache = registry.domain_cache(&root.0);
    std::fs::write(root.0.join("a.md"), b"# Seen by the cycle\n").unwrap();
    observe_past(&registry, &root, &cache, &baseline);
    let (announce, announced) = std::sync::mpsc::channel();
    let (resume, resumed) = std::sync::mpsc::channel();
    *PAIR_GATE.lock().unwrap() = Some((Arc::as_ptr(&cache) as usize, (announce, resumed)));
    let free = std::thread::scope(|scope| {
        let cycle = scope.spawn(|| ring.cycle(&root, &registry, true).unwrap());
        let reached = announced.recv_timeout(Duration::from_secs(10));
        let free = cache.try_lock().is_ok();
        resume.send(()).unwrap();
        cycle.join().unwrap();
        reached.unwrap();
        free
    });
    assert!(
        !free,
        "the memo was free between memo_pair's root read and its leaves read"
    );
}
