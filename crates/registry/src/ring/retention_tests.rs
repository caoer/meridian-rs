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
