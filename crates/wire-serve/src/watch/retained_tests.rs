use super::*;
use model::bytes::{ByteIdentity, ByteLease, ByteRetention, RetainedBytes, SharedBytes};
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Debug, Default)]
struct Backing {
    retains: AtomicUsize,
    fail_retain: AtomicUsize,
    fail_read: Arc<AtomicBool>,
}
#[derive(Debug)]
struct Version {
    raw: SharedBytes,
    fail: Arc<AtomicBool>,
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
            raw: SharedBytes::from(bytes),
            fail: self.fail_read.clone(),
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
    let removed = frame.files.iter().find(|f| f.path.0 == "a.md").unwrap();
    assert_eq!(removed.change, FileChange::Deleted);
    assert!(removed.file_rev_before.is_some());
    assert_eq!(frame.seq, 1);
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
    assert_eq!(frame.seq, 1, "a failed prepare consumed a sequence");
    assert_eq!(frame.files.len(), 2);
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
            .seq,
        1
    );
}
