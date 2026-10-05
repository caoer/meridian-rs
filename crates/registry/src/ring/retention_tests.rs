use super::*;
use model::bytes::{ByteIdentity, ByteLease, ByteRetention, RetainedBytes, SharedBytes};
use std::io;
use std::sync::atomic::AtomicUsize;

#[derive(Debug, Default)]
struct Counting(AtomicUsize);
#[derive(Debug)]
struct Version(SharedBytes);
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
        Ok(Arc::new(Version(SharedBytes::from(bytes))))
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
