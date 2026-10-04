//! Required raw-version backing (merkle §6.4), independent of optional snapshots.
//! Handles retain exact versions without retaining their raw bodies in memory.

use std::io;
use std::path::Path;
use std::sync::Arc;

/// Process-local backing store; its index must not own historical versions.
#[derive(Debug)]
pub struct RawStore;

/// An exact byte version, retained independently of the store's owner.
#[derive(Debug, Clone)]
pub struct RawHandle;

/// Live logical payload and actual segment storage, counted separately.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RawStoreStats {
    pub live_records: usize,
    pub live_bytes: u64,
    pub stored_bytes: u64,
    pub segments: usize,
}

impl RawStore {
    /// Create a private store below `parent`.
    ///
    /// # Errors
    /// Directory creation failure.
    pub fn new(_parent: &Path) -> io::Result<Self> {
        Ok(Self)
    }

    /// Retain verified bytes; publication failure leaves prior versions intact.
    ///
    /// # Errors
    /// Storage or verification failure.
    pub fn retain(&self, _bytes: &[u8]) -> io::Result<RawHandle> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "raw backing is not implemented",
        ))
    }

    /// Reclaim obsolete records without invalidating a live handle.
    ///
    /// # Errors
    /// Verification or storage failure; no handle is moved on failure.
    pub fn compact(&self) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "raw compaction is not implemented",
        ))
    }

    /// Read logical and physical storage usage.
    ///
    /// # Errors
    /// Segment metadata cannot be read.
    pub fn stats(&self) -> io::Result<RawStoreStats> {
        Ok(RawStoreStats::default())
    }
}

impl RawHandle {
    /// The retained content's model leaf digest.
    #[must_use]
    pub fn digest(&self) -> [u8; 32] {
        [0; 32]
    }

    /// The retained raw length, independent of current source files.
    #[must_use]
    pub fn len(&self) -> u64 {
        0
    }

    /// Whether the retained version is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Load and verify this exact version. Never reads a workspace path.
    ///
    /// # Errors
    /// Missing, corrupt or unreadable required storage, or allocation failure.
    pub fn read(&self) -> io::Result<Arc<[u8]>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "raw hydration is not implemented",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, OpenOptions};
    use std::os::unix::fs::FileExt;
    use std::path::PathBuf;

    fn files(dir: &Path) -> Vec<PathBuf> {
        let mut found = Vec::new();
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                found.extend(files(&path));
            } else {
                found.push(path);
            }
        }
        found.sort();
        found
    }

    fn physical_bytes(dir: &Path) -> u64 {
        files(dir)
            .iter()
            .map(|p| fs::metadata(p).unwrap().len())
            .sum()
    }

    #[test]
    fn entry_version_survives_source_loss_and_all_other_owners() {
        let home = tempfile::tempdir().unwrap();
        let parent = home.path().join("backing");
        let source = home.path().join("HOOKS.md");
        let original = b"# Hook\n\nversion A\n";
        fs::write(&source, original).unwrap();
        let store = RawStore::new(&parent).unwrap();
        let current = store.retain(original).expect("publish historical bytes");
        let baseline = current.clone();
        let entry = current.clone();
        assert!(physical_bytes(&parent) >= u64::try_from(original.len()).unwrap());
        fs::write(&source, b"version B").unwrap();
        let next = store.retain(b"version B").unwrap();
        drop(current);
        drop(baseline);
        store.compact().unwrap();
        assert_eq!(&*entry.read().unwrap(), original);
        assert_eq!(entry.digest(), model::leaf_digest(original));
        assert_eq!(entry.len(), u64::try_from(original.len()).unwrap());
        fs::remove_file(&source).unwrap();
        drop(next);
        drop(store);
        assert_eq!(
            &*entry.read().unwrap(),
            original,
            "entry owns its backing without the registry"
        );
        drop(entry);
        assert!(
            fs::read_dir(&parent).unwrap().next().is_none(),
            "last lease reclaims physical storage"
        );
    }

    #[test]
    fn repeated_versions_reclaim_real_segments_after_the_last_lease() {
        let home = tempfile::tempdir().unwrap();
        let store = RawStore::new(home.path()).unwrap();
        let entry = store.retain(&vec![b'A'; 8192]).unwrap();
        let mut current = entry.clone();
        for i in 1..=64 {
            let text = format!("version {i}\n{}", "x".repeat(8192));
            current = store.retain(text.as_bytes()).unwrap();
            store.compact().unwrap();
            let stats = store.stats().unwrap();
            assert_eq!(stats.live_records, 2);
            assert_eq!(stats.live_bytes, entry.len() + current.len());
            assert_eq!(stats.stored_bytes, physical_bytes(home.path()));
            assert!(
                stats.stored_bytes < 3 * 8192,
                "dead history accumulated: {stats:?}"
            );
            assert_eq!(&*entry.read().unwrap(), &vec![b'A'; 8192]);
        }
        drop(entry);
        store.compact().unwrap();
        assert_eq!(store.stats().unwrap().live_records, 1);
        drop(current);
        store.compact().unwrap();
        assert_eq!(store.stats().unwrap(), RawStoreStats::default());
        assert_eq!(physical_bytes(home.path()), 0);
    }

    #[test]
    fn concurrent_same_version_retention_has_one_physical_record() {
        let home = tempfile::tempdir().unwrap();
        let store = RawStore::new(home.path()).unwrap();
        let barrier = std::sync::Barrier::new(8);
        let handles = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        store.retain(b"one shared version").unwrap()
                    })
                })
                .collect();
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(store.stats().unwrap().live_records, 1);
        let bytes = physical_bytes(home.path());
        assert!(bytes < 256, "duplicate backing records: {bytes}");
        for handle in &handles {
            assert_eq!(&*handle.read().unwrap(), b"one shared version");
        }
        drop(handles);
        store.compact().unwrap();
        assert_eq!(physical_bytes(home.path()), 0);
    }

    #[test]
    fn corrupt_required_bytes_are_errors_and_compaction_does_not_publish_a_prefix() {
        let home = tempfile::tempdir().unwrap();
        let store = RawStore::new(home.path()).unwrap();
        let first = store.retain(b"first").unwrap();
        let second = store.retain(b"second").unwrap();
        let stored = files(home.path());
        assert_eq!(stored.len(), 1);
        let file = OpenOptions::new().write(true).open(&stored[0]).unwrap();
        let end = file.metadata().unwrap().len();
        file.write_all_at(b"!", end - 1).unwrap();
        let before: Vec<_> = files(home.path())
            .into_iter()
            .map(|p| {
                let bytes = fs::read(&p).unwrap();
                (p, bytes)
            })
            .collect();
        assert_eq!(
            second.read().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert!(
            store.compact().is_err(),
            "late bad record refuses compaction"
        );
        let after: Vec<_> = files(home.path())
            .into_iter()
            .map(|p| {
                let bytes = fs::read(&p).unwrap();
                (p, bytes)
            })
            .collect();
        assert_eq!(before, after, "no replacement prefix is published");
        assert_eq!(&*first.read().unwrap(), b"first");
        assert_eq!(
            second.read().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
