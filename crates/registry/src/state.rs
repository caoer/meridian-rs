//! The single-writer state file: warm registrations that survive a daemon
//! restart.
//!
//! One writer (the daemon; the in-memory map lock serializes every mutation),
//! written atomically — a temp file in the same directory, fsynced, then
//! renamed over the target, then the directory fsynced. A crash leaves at most
//! an orphan temp, never a half-written state file.
//!
//! Corrupt is cold: an absent, unreadable, unparseable, or future-schema state
//! file loads as the empty set — the daemon starts with no warm registrations
//! rather than refusing to start or adopting garbage.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::protocol::WorkspaceEntry;

/// The on-disk state schema this binary writes and understands. A file whose
/// `schema` is greater is a *future* format from a newer binary; it loads as
/// empty (cold start) rather than being decoded into the current shape.
const STATE_SCHEMA: u32 = 1;

/// The persisted registry: a schema tag plus the registered set.
#[derive(Debug, Serialize, Deserialize)]
struct StateFile {
    schema: u32,
    entries: Vec<WorkspaceEntry>,
}

/// The atomic reader/writer for the registry state file at a fixed path.
#[derive(Debug, Clone)]
pub(crate) struct StateStore {
    path: PathBuf,
}

impl StateStore {
    /// Bind a store to `path` (its parent directory must already exist — the
    /// daemon creates the registry directory before constructing the store).
    pub(crate) fn new(path: PathBuf) -> Self {
        StateStore { path }
    }

    /// Load the registered set. Any failure — absent, unreadable, corrupt, or
    /// a future schema — returns the empty set and logs to stderr.
    pub(crate) fn load(&self) -> Vec<WorkspaceEntry> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Vec::new(),
            Err(e) => {
                eprintln!(
                    "registry: cannot read state file {} ({e}); starting cold",
                    self.path.display()
                );
                return Vec::new();
            }
        };
        match serde_json::from_slice::<StateFile>(&bytes) {
            Ok(state) if state.schema <= STATE_SCHEMA => state.entries,
            Ok(state) => {
                eprintln!(
                    "registry: state file {} is schema {} (newer than {STATE_SCHEMA}); starting cold",
                    self.path.display(),
                    state.schema
                );
                Vec::new()
            }
            Err(e) => {
                eprintln!(
                    "registry: state file {} is corrupt ({e}); starting cold",
                    self.path.display()
                );
                Vec::new()
            }
        }
    }

    /// Atomically overwrite the state file with `entries`: temp + fsync +
    /// rename + directory fsync, mode 0600.
    ///
    /// # Errors
    ///
    /// Propagates I/O failures from the temp write or the rename. A failed dir
    /// fsync only weakens durability and is swallowed (the rename already made
    /// the new state visible).
    pub(crate) fn save(&self, entries: &[WorkspaceEntry]) -> io::Result<()> {
        let dir = self
            .path
            .parent()
            .ok_or_else(|| io::Error::other("state path has no parent directory"))?;
        let state = StateFile {
            schema: STATE_SCHEMA,
            entries: entries.to_vec(),
        };
        let bytes = serde_json::to_vec_pretty(&state).map_err(io::Error::other)?;
        let tmp = write_tmp(dir, &bytes)?;
        // rename(2) is atomic within a directory: a reader sees either the old
        // file or the new one, never a partial write.
        fs::rename(&tmp, &self.path).inspect_err(|_| {
            let _ = fs::remove_file(&tmp);
        })?;
        fsync_dir(dir);
        Ok(())
    }
}

/// Write `data` to a uniquely-named temp file in `dir`, fsynced, mode 0600. The
/// name never collides with the state file, so a crash before the rename
/// leaves an orphan temp that `load` never reads.
fn write_tmp(dir: &Path, data: &[u8]) -> io::Result<PathBuf> {
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let mut counter: u64 = 0;
    loop {
        let path = dir.join(format!(".state.json.{pid}.{nanos}.{counter}.tmp"));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(mut f) => {
                f.write_all(data)?;
                f.sync_all()?;
                return Ok(path);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => counter += 1,
            Err(e) => return Err(e),
        }
    }
}

/// fsync a directory so a completed rename survives power loss. Best-effort: a
/// failed dir sync must never turn a committed write into a reported failure.
fn fsync_dir(dir: &Path) {
    if let Ok(f) = File::open(dir) {
        let _ = f.sync_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, last_use: u64) -> WorkspaceEntry {
        WorkspaceEntry {
            workspace: PathBuf::from(path),
            registered_at: 1,
            last_use,
        }
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path().join("state.json"));
        let entries = vec![entry("/a", 10), entry("/b", 20)];
        store.save(&entries).unwrap();
        let loaded = store.load();
        assert_eq!(loaded, entries);
    }

    #[test]
    fn absent_file_loads_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path().join("nope.json"));
        assert!(store.load().is_empty());
    }

    #[test]
    fn corrupt_file_loads_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        fs::write(&path, b"{ not valid json at all").unwrap();
        let store = StateStore::new(path);
        assert!(store.load().is_empty(), "corrupt state must load as empty");
    }

    #[test]
    fn future_schema_loads_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        fs::write(&path, br#"{"schema":999,"entries":[]}"#).unwrap();
        let store = StateStore::new(path);
        assert!(
            store.load().is_empty(),
            "a newer-schema state file must load cold, never be decoded"
        );
    }

    #[test]
    fn save_is_atomic_no_temp_left_behind() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path().join("state.json"));
        store.save(&[entry("/a", 1)]).unwrap();
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "no temp file may survive a save");
    }
}
