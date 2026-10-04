//! Required raw-version backing (merkle §6.4), independent of optional snapshots.
//! Handles retain exact versions without retaining their raw bodies in memory.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::{DirBuilderExt, FileExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{SystemTime, UNIX_EPOCH};

const MAGIC: &[u8; 8] = b"MRDRAW01";
const HEADER: usize = 80;
const HEADER_BYTES: u64 = 80;
const SEGMENT_BYTES: u64 = 4 * 1024 * 1024;
const COPY_BYTES: usize = 64 * 1024;
static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn lock<T>(mutex: &Mutex<T>) -> io::Result<MutexGuard<'_, T>> {
    mutex
        .lock()
        .map_err(|_| io::Error::other("raw backing lock poisoned"))
}

#[derive(Debug)]
struct Directory(PathBuf);

impl Drop for Directory {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir(&self.0)
            && error.kind() != io::ErrorKind::NotFound
        {
            eprintln!("raw-store: cannot reclaim {}: {error}", self.0.display());
        }
    }
}

#[derive(Debug)]
struct Segment {
    id: u64,
    file: File,
    path: PathBuf,
    // Last field: the file closes before the last directory lease drops.
    _directory: Arc<Directory>,
}

impl Drop for Segment {
    fn drop(&mut self) {
        // Unix unlink preserves an in-flight reader's open file. A reader
        // also owns this Segment, so normal reclamation follows its release.
        if let Err(error) = fs::remove_file(&self.path)
            && error.kind() != io::ErrorKind::NotFound
        {
            eprintln!("raw-store: cannot reclaim {}: {error}", self.path.display());
        }
    }
}

#[derive(Clone, Debug)]
struct Location {
    segment: Arc<Segment>,
    offset: u64,
}

#[derive(Debug)]
struct Record {
    digest: [u8; 32],
    len: u64,
    location: Mutex<Location>,
}

#[derive(Debug, Default)]
struct Index {
    records: HashMap<[u8; 32], Weak<Record>>,
    segments: Vec<Weak<Segment>>,
}

#[derive(Debug, Default)]
struct Writer {
    active: Option<Arc<Segment>>,
    end: u64,
    next_segment: u64,
}

/// Process-local backing store; its index must not own historical versions.
/// The owner calls [`Self::compact`] at maintenance boundaries. No worker,
/// recovery scan, or global strong index retains obsolete history.
#[derive(Debug)]
pub struct RawStore {
    directory: Arc<Directory>,
    index: Mutex<Index>,
    // The I/O serialization gate, not a registry-map or document-slot lock.
    writer: Mutex<Writer>,
}

/// An exact byte version, retained independently of the store's owner.
/// Last-owner release can unlink storage; callers drop retired handles outside
/// shared lookup/slot locks.
#[derive(Debug, Clone)]
pub struct RawHandle(Arc<Record>);

/// Live logical payload and actual segment storage, counted separately.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RawStoreStats {
    /// Distinct records with live handles (the index owns only weak references).
    pub live_records: usize,
    /// Raw payload bytes for those records.
    pub live_bytes: u64,
    /// Segment file lengths, including obsolete records, failed-unlink debris
    /// and files held open by reader leases.
    pub stored_bytes: u64,
    /// Filesystem-allocated blocks for those segments, in bytes. Compression and
    /// allocation granularity can make this differ from `stored_bytes`.
    pub allocated_bytes: u64,
    /// Live segment files, including an append segment or an in-flight reader.
    pub segments: usize,
}

impl RawStore {
    /// Create a private store below `parent`.
    ///
    /// # Errors
    /// Directory creation failure.
    pub fn new(parent: &Path) -> io::Result<Self> {
        fs::create_dir_all(parent)?;
        let parent = fs::canonicalize(parent)?;
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for _ in 0..128 {
            let serial = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!("raw-{}-{time:x}-{serial:x}", std::process::id()));
            match fs::DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => {
                    return Ok(Self {
                        directory: Arc::new(Directory(path)),
                        index: Mutex::new(Index::default()),
                        writer: Mutex::new(Writer::default()),
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "raw store directory collision",
        ))
    }

    /// Retain verified bytes; publication failure leaves prior versions intact.
    ///
    /// # Errors
    /// Storage or verification failure.
    pub fn retain(&self, bytes: &[u8]) -> io::Result<RawHandle> {
        let digest = model::leaf_digest(bytes);
        let len = u64::try_from(bytes.len()).map_err(|_| invalid("raw length does not fit"))?;
        let mut writer = lock(&self.writer)?;
        let existing = lock(&self.index)?
            .records
            .get(&digest)
            .and_then(Weak::upgrade);
        if let Some(record) = &existing {
            if record.len != len {
                return Err(invalid("conflicting raw identity length"));
            }
            let location = lock(&record.location)?.clone();
            if visit(&location, len, &digest, |_| Ok(())).is_ok() {
                return Ok(RawHandle(Arc::clone(record)));
            }
            // Fresh verified caller bytes may repair this SAME identity. All
            // existing handles move only after the replacement is complete.
        }
        let location = self.append(&mut writer, len, &digest, |file, offset| {
            file.write_all_at(bytes, offset)
        })?;
        let record = if let Some(record) = existing {
            let retired = {
                let mut slot = lock(&record.location)?;
                std::mem::replace(&mut *slot, location)
            };
            drop(retired);
            record
        } else {
            let record = Arc::new(Record {
                digest,
                len,
                location: Mutex::new(location),
            });
            lock(&self.index)?
                .records
                .insert(digest, Arc::downgrade(&record));
            record
        };
        Ok(RawHandle(record))
    }

    /// Reclaim obsolete records without invalidating a live handle.
    ///
    /// # Errors
    /// Verification/copy failure moves no handle. A later reclamation failure
    /// is reported after publication; all replacement handles remain valid.
    pub fn compact(&self) -> io::Result<()> {
        let mut writer = lock(&self.writer)?;
        let records: Vec<_> = {
            let mut index = lock(&self.index)?;
            index.records.retain(|_, record| record.strong_count() != 0);
            index.records.values().filter_map(Weak::upgrade).collect()
        };
        let mut sources = records
            .iter()
            .map(|record| Ok((Arc::clone(record), lock(&record.location)?.clone())))
            .collect::<io::Result<Vec<_>>>()?;
        sources.sort_by_key(|(_, location)| (location.segment.id, location.offset));
        let mut replacement = Writer {
            next_segment: writer.next_segment,
            ..Writer::default()
        };
        let prepared: io::Result<Vec<Location>> = (|| {
            let mut locations = Vec::with_capacity(sources.len());
            for (record, source) in &sources {
                let location = self.append(
                    &mut replacement,
                    record.len,
                    &record.digest,
                    |file, offset| {
                        let mut cursor = offset;
                        visit(source, record.len, &record.digest, |chunk| {
                            file.write_all_at(chunk, cursor)?;
                            cursor +=
                                u64::try_from(chunk.len()).map_err(|_| invalid("chunk length"))?;
                            Ok(())
                        })
                    },
                )?;
                locations.push(location);
            }
            Ok(locations)
        })();
        // Never reuse a staging filename, even when a failed pass is discarded.
        writer.next_segment = replacement.next_segment;
        let locations: Vec<Location> = prepared?;
        // Acquire all publication locks BEFORE moving any handle: poison/fault
        // cannot publish a prefix. No I/O runs under these locks. `sources`
        // keeps every old segment alive until the locks have been released.
        let mut guards = sources
            .iter()
            .map(|(record, _)| lock(&record.location))
            .collect::<io::Result<Vec<_>>>()?;
        for (guard, location) in guards.iter_mut().zip(locations) {
            **guard = location;
        }
        drop(guards);
        *writer = replacement;
        drop(sources);
        drop(records);
        {
            let mut index = lock(&self.index)?;
            index.records.retain(|_, record| record.strong_count() != 0);
            index.segments.retain(|segment| segment.strong_count() != 0);
        }
        self.reclaim(writer.next_segment)
    }

    /// Read logical and physical storage usage, including failed-unlink debris.
    /// This is a storage census, not a RAM accountant. Concurrent operations
    /// can change the census while it is read; quiesce them for exact receipts.
    ///
    /// # Errors
    /// Segment metadata cannot be read.
    pub fn stats(&self) -> io::Result<RawStoreStats> {
        let (records, segments): (Vec<_>, Vec<_>) = {
            let mut index = lock(&self.index)?;
            index.records.retain(|_, record| record.strong_count() != 0);
            index.segments.retain(|segment| segment.strong_count() != 0);
            (
                index.records.values().filter_map(Weak::upgrade).collect(),
                index.segments.iter().filter_map(Weak::upgrade).collect(),
            )
        };
        let mut stats = RawStoreStats {
            live_records: records.len(),
            live_bytes: records.iter().try_fold(0_u64, |sum, record| {
                sum.checked_add(record.len)
                    .ok_or_else(|| invalid("raw statistics overflow"))
            })?,
            ..RawStoreStats::default()
        };
        let mut seen = HashSet::new();
        for segment in &segments {
            add_metadata(&mut stats, &mut seen, &segment.file.metadata()?)?;
        }
        for path in self.owned_files()? {
            match fs::symlink_metadata(path) {
                Ok(metadata) => add_metadata(&mut stats, &mut seen, &metadata)?,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(stats)
    }

    fn owned_files(&self) -> io::Result<Vec<PathBuf>> {
        let mut paths = Vec::new();
        for entry in fs::read_dir(&self.directory.0)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name
                .to_str()
                .ok_or_else(|| invalid("unknown raw-store entry"))?;
            let id = name
                .strip_suffix(".raw")
                .ok_or_else(|| invalid("unknown raw-store entry"))?;
            let kind = match entry.file_type() {
                Ok(kind) => kind,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            if id.len() != 16 || u64::from_str_radix(id, 16).is_err() || !kind.is_file() {
                return Err(invalid("unknown raw-store entry"));
            }
            paths.push(entry.path());
        }
        Ok(paths)
    }

    // Caller holds the writer gate, so no new append/staging segment can
    // appear after this live set is captured. Reader leases keep their files.
    fn reclaim(&self, next_segment: u64) -> io::Result<()> {
        let live: Vec<_> = lock(&self.index)?
            .segments
            .iter()
            .filter_map(Weak::upgrade)
            .collect();
        let paths: HashSet<_> = live.iter().map(|segment| segment.path.clone()).collect();
        for path in self.owned_files()? {
            let id = path
                .file_stem()
                .and_then(|name| name.to_str())
                .and_then(|name| u64::from_str_radix(name, 16).ok())
                .ok_or_else(|| invalid("unknown raw-store entry"))?;
            if id >= next_segment {
                return Err(invalid("unowned raw-store segment"));
            }
            if !paths.contains(&path) {
                match fs::remove_file(path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
        }
        Ok(())
    }

    fn append(
        &self,
        writer: &mut Writer,
        len: u64,
        digest: &[u8; 32],
        payload: impl FnOnce(&File, u64) -> io::Result<()>,
    ) -> io::Result<Location> {
        let size = HEADER_BYTES
            .checked_add(len)
            .ok_or_else(|| invalid("raw record overflow"))?;
        if writer.active.is_none()
            || writer
                .end
                .checked_add(size)
                .is_none_or(|end| end > SEGMENT_BYTES)
        {
            let id = writer.next_segment;
            writer.next_segment = id
                .checked_add(1)
                .ok_or_else(|| invalid("segment id overflow"))?;
            let path = self.directory.0.join(format!("{id:016x}.raw"));
            let file = OpenOptions::new()
                .create_new(true)
                .read(true)
                .write(true)
                .mode(0o600)
                .open(&path)?;
            let segment = Arc::new(Segment {
                id,
                file,
                path,
                _directory: Arc::clone(&self.directory),
            });
            lock(&self.index)?.segments.push(Arc::downgrade(&segment));
            writer.active = Some(segment);
            writer.end = 0;
        }
        let segment = Arc::clone(
            writer
                .active
                .as_ref()
                .ok_or_else(|| invalid("no append segment"))?,
        );
        let offset = writer.end;
        let end = offset
            .checked_add(size)
            .ok_or_else(|| invalid("raw offset overflow"))?;
        let location = Location { segment, offset };
        let result = (|| {
            let file = &location.segment.file;
            let mut header = header_prefix(len, digest);
            file.write_all_at(&header, offset)?;
            payload(file, offset + HEADER_BYTES)?;
            // Check the actual written payload with bounded memory before
            // publishing a checksum or a handle, including after compaction.
            let checksum = hash_payload(&location, len, digest, &header[..48], |_| Ok(()))?;
            header[48..].copy_from_slice(&checksum);
            file.write_all_at(&header, offset)?;
            Ok(())
        })();
        if let Err(error) = result {
            // Retire a partial append segment. Existing handles still own its
            // valid prefix; a future compaction removes any failed tail.
            writer.active = None;
            writer.end = 0;
            return Err(error);
        }
        writer.end = end;
        Ok(location)
    }
}

impl RawHandle {
    /// The retained content's model leaf digest.
    #[must_use]
    pub fn digest(&self) -> [u8; 32] {
        self.0.digest
    }

    /// The retained raw length, independent of current source files.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.0.len
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
    pub fn read(&self) -> io::Result<Vec<u8>> {
        let location = lock(&self.0.location)?.clone();
        // Framing is verified BEFORE allocation; the allocation size comes
        // from the retained identity, never from untrusted on-disk length.
        read_header(&location, self.len(), &self.digest())?;
        let len = usize::try_from(self.len()).map_err(|_| invalid("raw allocation length"))?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(len).map_err(io::Error::other)?;
        visit(&location, self.len(), &self.digest(), |chunk| {
            bytes.extend_from_slice(chunk);
            Ok(())
        })?;
        Ok(bytes)
    }
}

fn add_metadata(
    stats: &mut RawStoreStats,
    seen: &mut HashSet<(u64, u64)>,
    metadata: &fs::Metadata,
) -> io::Result<()> {
    if seen.insert((metadata.dev(), metadata.ino())) {
        stats.segments += 1;
        stats.stored_bytes = stats
            .stored_bytes
            .checked_add(metadata.len())
            .ok_or_else(|| invalid("raw statistics overflow"))?;
        let allocated = metadata
            .blocks()
            .checked_mul(512)
            .ok_or_else(|| invalid("raw statistics overflow"))?;
        stats.allocated_bytes = stats
            .allocated_bytes
            .checked_add(allocated)
            .ok_or_else(|| invalid("raw statistics overflow"))?;
    }
    Ok(())
}

fn header_prefix(len: u64, digest: &[u8; 32]) -> [u8; HEADER] {
    let mut header = [0; HEADER];
    header[..8].copy_from_slice(MAGIC);
    header[8..16].copy_from_slice(&len.to_be_bytes());
    header[16..48].copy_from_slice(digest);
    header
}

fn read_header(location: &Location, len: u64, digest: &[u8; 32]) -> io::Result<[u8; HEADER]> {
    let mut header = [0; HEADER];
    location
        .segment
        .file
        .read_exact_at(&mut header, location.offset)?;
    if header[..48] != header_prefix(len, digest)[..48] {
        return Err(invalid(
            "raw backing header does not match the retained identity",
        ));
    }
    Ok(header)
}

fn visit(
    location: &Location,
    len: u64,
    digest: &[u8; 32],
    visitor: impl FnMut(&[u8]) -> io::Result<()>,
) -> io::Result<()> {
    let header = read_header(location, len, digest)?;
    let checksum = hash_payload(location, len, digest, &header[..48], visitor)?;
    if header[48..] != checksum {
        return Err(invalid("raw backing checksum mismatch"));
    }
    Ok(())
}

fn hash_payload(
    location: &Location,
    len: u64,
    digest: &[u8; 32],
    prefix: &[u8],
    mut visitor: impl FnMut(&[u8]) -> io::Result<()>,
) -> io::Result<[u8; 32]> {
    let mut checksum = blake3::Hasher::new();
    checksum.update(b"meridian.raw-record.v1\0");
    checksum.update(prefix);
    // Streaming form of model::leaf_digest (blake3(raw)); the expected
    // identity was minted by that owner, and every pass checks it.
    let mut leaf = blake3::Hasher::new();
    let mut buffer = vec![0; COPY_BYTES];
    let mut remaining = len;
    let mut offset = location
        .offset
        .checked_add(HEADER_BYTES)
        .ok_or_else(|| invalid("raw offset"))?;
    while remaining != 0 {
        let count = usize::try_from(remaining.min(COPY_BYTES as u64))
            .map_err(|_| invalid("chunk length"))?;
        let bytes = &mut buffer[..count];
        location.segment.file.read_exact_at(bytes, offset)?;
        leaf.update(bytes);
        checksum.update(bytes);
        visitor(bytes)?;
        remaining -= u64::try_from(count).map_err(|_| invalid("chunk length"))?;
        offset += u64::try_from(count).map_err(|_| invalid("chunk length"))?;
    }
    if leaf.finalize().as_bytes() != digest {
        return Err(invalid("raw backing content digest mismatch"));
    }
    Ok(*checksum.finalize().as_bytes())
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
    fn empty_and_invalid_utf8_are_bytes_but_truncated_backing_is_an_error() {
        let home = tempfile::tempdir().unwrap();
        let store = RawStore::new(home.path()).unwrap();
        let empty = store.retain(b"").unwrap();
        let binary = store.retain(b"\xff\x00\xfe").unwrap();
        assert!(empty.is_empty());
        assert_eq!(empty.digest(), model::leaf_digest(b""));
        assert!(empty.read().unwrap().is_empty());
        assert_eq!(&*binary.read().unwrap(), b"\xff\x00\xfe");
        let stored = files(home.path());
        OpenOptions::new()
            .write(true)
            .open(&stored[0])
            .unwrap()
            .set_len(0)
            .unwrap();
        assert!(
            empty.read().is_err(),
            "missing required framing is not an empty document"
        );
        assert!(binary.read().is_err());
    }

    #[test]
    fn failed_publication_keeps_existing_versions_and_publishes_no_new_record() {
        let home = tempfile::tempdir().unwrap();
        let store = RawStore::new(home.path()).unwrap();
        // Oversized records also exercise bounded streaming in compaction.
        let large = vec![b'A'; 5 * 1024 * 1024];
        let prior = store.retain(&large).unwrap();
        let directory = fs::read_dir(home.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let moved = home.path().join("moved");
        fs::rename(&directory, &moved).unwrap();
        let failed = store.retain(b"cannot create the next segment");
        fs::rename(&moved, &directory).unwrap();
        assert!(failed.is_err());
        assert_eq!(store.stats().unwrap().live_records, 1);
        assert_eq!(&*prior.read().unwrap(), &large);
        store.compact().unwrap();
        assert_eq!(&*prior.read().unwrap(), &large);
    }

    #[test]
    fn readers_keep_exact_segments_while_compaction_moves_their_handles() {
        let home = tempfile::tempdir().unwrap();
        let store = RawStore::new(home.path()).unwrap();
        let payload = vec![b'R'; 65537];
        let entry = store.retain(&payload).unwrap();
        let barrier = std::sync::Barrier::new(4);
        std::thread::scope(|scope| {
            for _ in 0..3 {
                scope.spawn(|| {
                    barrier.wait();
                    for _ in 0..64 {
                        assert_eq!(&*entry.read().unwrap(), &payload);
                    }
                });
            }
            barrier.wait();
            for i in 0..16 {
                let temporary = store.retain(format!("unused {i}").as_bytes()).unwrap();
                drop(temporary);
                store.compact().unwrap();
            }
        });
        assert_eq!(store.stats().unwrap().live_records, 1);
        assert_eq!(&*entry.read().unwrap(), &payload);
    }

    #[test]
    fn corrupt_length_is_refused_before_any_payload_allocation() {
        let home = tempfile::tempdir().unwrap();
        let store = RawStore::new(home.path()).unwrap();
        let handle = store.retain(b"small").unwrap();
        let stored = files(home.path());
        let file = OpenOptions::new().write(true).open(&stored[0]).unwrap();
        file.write_all_at(&u64::MAX.to_be_bytes(), 8).unwrap();
        assert_eq!(
            handle.read().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            handle.len(),
            5,
            "identity length is not learned from corrupt storage"
        );
    }

    #[test]
    fn verified_caller_bytes_repair_the_same_identity_for_all_handles() {
        let home = tempfile::tempdir().unwrap();
        let store = RawStore::new(home.path()).unwrap();
        let old = store.retain(b"original").unwrap();
        let entry = old.clone();
        let stored = files(home.path());
        let file = OpenOptions::new().write(true).open(&stored[0]).unwrap();
        file.write_all_at(b"broken!!", 0).unwrap();
        assert!(entry.read().is_err());
        let repaired = store.retain(b"original").unwrap();
        for handle in [&old, &entry, &repaired] {
            assert_eq!(handle.read().unwrap(), b"original");
            assert_eq!(handle.digest(), model::leaf_digest(b"original"));
        }
        assert_eq!(store.stats().unwrap().live_records, 1);
        store.compact().unwrap();
        assert_eq!(entry.read().unwrap(), b"original");
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
