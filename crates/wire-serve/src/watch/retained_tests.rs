use super::*;
use model::bytes::{ByteIdentity, ByteLease, ByteRetention, RetainedBytes, SharedBytes};
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Debug, Default)]
struct Backing {
    retains: AtomicUsize,
    wrong_identity: AtomicBool,
    fail_retain: AtomicUsize,
    fail_read: Arc<AtomicBool>,
    corrupt_read: Arc<AtomicBool>,
    panic_drop: Arc<AtomicBool>,
}
#[derive(Debug)]
struct Version {
    raw: SharedBytes,
    fail: Arc<AtomicBool>,
    corrupt: Arc<AtomicBool>,
    panic_drop: Arc<AtomicBool>,
}
impl Drop for Version {
    fn drop(&mut self) {
        assert!(
            !self.panic_drop.swap(false, Ordering::SeqCst),
            "injected retired baseline destructor unwind"
        );
    }
}
impl RetainedBytes for Version {
    fn identity(&self) -> ByteIdentity {
        ByteIdentity {
            digest: model::leaf_digest(self.raw.as_bytes()),
            len: self.raw.len() as u64,
        }
    }
    fn pin(&self) -> io::Result<Box<dyn ByteLease>> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(io::Error::other("injected required backing fault"));
        }
        if self.corrupt.load(Ordering::SeqCst) {
            return Ok(Box::new(SharedBytes::from(b"wrong bytes".to_vec())));
        }
        Ok(Box::new(self.raw.clone()))
    }
}
impl ByteRetention for Backing {
    fn retain(&self, bytes: Vec<u8>) -> io::Result<Arc<dyn RetainedBytes>> {
        let nth = self.retains.fetch_add(1, Ordering::SeqCst) + 1;
        if nth == self.fail_retain.load(Ordering::SeqCst) {
            return Err(io::Error::other("injected late retain fault"));
        }
        Ok(Arc::new(Version {
            raw: SharedBytes::from(if self.wrong_identity.load(Ordering::SeqCst) {
                b"wrong retained version".to_vec()
            } else {
                bytes
            }),
            fail: self.fail_read.clone(),
            corrupt: self.corrupt_read.clone(),
            panic_drop: self.panic_drop.clone(),
        }))
    }
}
fn fixture() -> (
    tempfile::TempDir,
    fs::WorkspaceRoot,
    Arc<Backing>,
    WatchState,
    RootRing,
) {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    std::fs::write(root.0.join("a.md"), b"# A\n").unwrap();
    std::fs::write(root.0.join("b.md"), b"# B\n").unwrap();
    let backing = Arc::new(Backing::default());
    let mut watch = WatchState::with_retention(&root, backing.clone());
    let mut ring = RootRing::new();
    assert!(reconcile(&root, &mut ring, &mut watch).unwrap().is_none());
    (dir, root, backing, watch, ring)
}
#[test]
fn migration_watcher_retains_versions_in_host_backing() {
    let (_dir, root, backing, mut watch, mut ring) = fixture();
    assert_eq!(
        backing.retains.load(Ordering::SeqCst),
        2,
        "watcher ignored retained baseline backing"
    );
    std::fs::remove_file(root.0.join("a.md")).unwrap();
    let frame = reconcile(&root, &mut ring, &mut watch).unwrap().unwrap();
    let removed = frame
        .delta
        .files
        .iter()
        .find(|f| f.path.0 == "a.md")
        .unwrap();
    assert_eq!(removed.change, FileChange::Deleted);
    assert!(removed.file_rev_before.is_some());
    assert_eq!(frame.delta.seq, 1);
}
#[test]
fn migration_late_retain_fault_keeps_baseline_and_sequence() {
    let (_dir, root, backing, mut watch, mut ring) = fixture();
    let before = watch.root().cloned();
    backing
        .fail_retain
        .store(backing.retains.load(Ordering::SeqCst) + 2, Ordering::SeqCst);
    std::fs::write(root.0.join("a.md"), b"# A new\n").unwrap();
    std::fs::write(root.0.join("b.md"), b"# B new\n").unwrap();
    assert!(
        reconcile(&root, &mut ring, &mut watch).is_err(),
        "late backing failure published the transition"
    );
    assert_eq!(watch.root().cloned(), before);
    assert_eq!(ring.seq(), 0);
    backing.fail_retain.store(0, Ordering::SeqCst);
    let frame = reconcile(&root, &mut ring, &mut watch).unwrap().unwrap();
    assert_eq!(frame.delta.seq, 1, "a failed prepare consumed a sequence");
    assert_eq!(frame.delta.files.len(), 2);
}
#[test]
fn migration_required_before_fault_never_falls_back_to_live_source() {
    let (_dir, root, backing, mut watch, mut ring) = fixture();
    let before = watch.root().cloned();
    backing.fail_read.store(true, Ordering::SeqCst);
    std::fs::write(root.0.join("b.md"), b"# Replacement\n").unwrap();
    assert!(
        reconcile(&root, &mut ring, &mut watch).is_err(),
        "required before fault silently used eager or live bytes"
    );
    assert_eq!(watch.root().cloned(), before);
    assert_eq!(ring.seq(), 0);
    backing.fail_read.store(false, Ordering::SeqCst);
    assert_eq!(
        reconcile(&root, &mut ring, &mut watch)
            .unwrap()
            .unwrap()
            .delta
            .seq,
        1
    );
}

fn observed(root: &fs::WorkspaceRoot) -> (std::collections::BTreeMap<String, [u8; 32]>, Root) {
    let (_, leaves, folded) = fs::domain_snapshot_with_leaves(root).unwrap();
    (
        leaves
            .into_iter()
            .map(|(path, digest)| (path.to_str().unwrap().to_owned(), digest))
            .collect(),
        Root(folded.0),
    )
}

#[test]
fn retained_delta_faults_keep_the_same_transition_retryable() {
    for read_fault in [true, false] {
        let (_dir, root, backing, mut watch, mut ring) = fixture();
        let before = watch.root().cloned();
        if read_fault {
            backing.fail_read.store(true, Ordering::SeqCst);
        } else {
            backing
                .fail_retain
                .store(backing.retains.load(Ordering::SeqCst) + 2, Ordering::SeqCst);
        }
        std::fs::write(root.0.join("a.md"), b"# A new\n").unwrap();
        std::fs::write(root.0.join("b.md"), b"# B new\n").unwrap();
        let (leaves, disk) = observed(&root);
        assert!(reconcile_delta(&root, &mut ring, &mut watch, &leaves, &disk).is_err());
        assert_eq!(watch.root().cloned(), before);
        assert_eq!(ring.seq(), 0);
        backing.fail_read.store(false, Ordering::SeqCst);
        backing.fail_retain.store(0, Ordering::SeqCst);
        let frame = reconcile_delta(&root, &mut ring, &mut watch, &leaves, &disk)
            .unwrap()
            .unwrap();
        assert_eq!(frame.delta.seq, 1);
        assert_eq!(frame.delta.files.len(), 2);
    }
}

#[test]
fn retained_prime_failure_does_not_create_a_partial_baseline() {
    let (_dir, root, backing, _, _) = fixture();
    let mut watch = WatchState::with_retention(&root, backing.clone());
    let (leaves, disk) = observed(&root);
    backing
        .fail_retain
        .store(backing.retains.load(Ordering::SeqCst) + 2, Ordering::SeqCst);
    assert!(
        prime(
            &root,
            &mut watch,
            PrimeSnapshot::take(&root).unwrap(),
            &leaves,
            &disk
        )
        .is_err()
    );
    assert!(watch.root().is_none());
    backing.fail_retain.store(0, Ordering::SeqCst);
    assert!(
        prime(
            &root,
            &mut watch,
            PrimeSnapshot::take(&root).unwrap(),
            &leaves,
            &disk
        )
        .unwrap()
    );
    assert_eq!(watch.root(), Some(&disk));
}

#[test]
fn retained_invalid_utf8_still_has_exact_before_membership() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    std::fs::write(root.0.join("poison.md"), [255, 254]).unwrap();
    let backing = Arc::new(Backing::default());
    let mut watch = WatchState::with_retention(&root, backing);
    let mut ring = RootRing::new();
    reconcile(&root, &mut ring, &mut watch).unwrap();
    std::fs::write(root.0.join("poison.md"), b"# Repaired\n").unwrap();
    let frame = reconcile(&root, &mut ring, &mut watch).unwrap().unwrap();
    let file = &frame.delta.files[0];
    assert_eq!(file.change, FileChange::Modified);
    assert!(file.file_rev_before.is_none());
    assert!(file.file_rev_after.is_some());
}

#[test]
fn retained_bytes_are_verified_before_classification() {
    let (_dir, root, backing, mut watch, mut ring) = fixture();
    let before = watch.root().cloned();
    backing.corrupt_read.store(true, Ordering::SeqCst);
    std::fs::remove_file(root.0.join("a.md")).unwrap();
    let error = reconcile(&root, &mut ring, &mut watch).unwrap_err();
    assert!(error.cause.as_deref().unwrap().contains("identity"));
    assert_eq!(watch.root().cloned(), before);
    assert_eq!(ring.seq(), 0);
    backing.corrupt_read.store(false, Ordering::SeqCst);
    assert_eq!(
        reconcile(&root, &mut ring, &mut watch)
            .unwrap()
            .unwrap()
            .delta
            .seq,
        1
    );
}

#[test]
fn retired_baseline_destructors_run_after_the_whole_publication() {
    let (_dir, root, backing, mut watch, mut ring) = fixture();
    std::fs::write(root.0.join("a.md"), b"# Published\n").unwrap();
    let (_, disk) = observed(&root);
    backing.panic_drop.store(true, Ordering::SeqCst);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        reconcile(&root, &mut ring, &mut watch)
    }));
    assert!(result.is_err());
    assert_eq!(watch.root(), Some(&disk));
    assert_eq!(ring.seq(), 1);
    assert!(reconcile(&root, &mut ring, &mut watch).unwrap().is_none());
}

#[test]
fn retention_cannot_publish_another_versions_identity() {
    let (_dir, root, backing, _, _) = fixture();
    let mut watch = WatchState::with_retention(&root, backing.clone());
    let mut ring = RootRing::new();
    backing.wrong_identity.store(true, Ordering::SeqCst);
    assert!(reconcile(&root, &mut ring, &mut watch).is_err());
    assert!(watch.root().is_none());
    assert_eq!(ring.seq(), 0);
    backing.wrong_identity.store(false, Ordering::SeqCst);
    assert!(reconcile(&root, &mut ring, &mut watch).unwrap().is_none());
    assert!(watch.root().is_some());
}
