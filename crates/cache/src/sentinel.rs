//! The sentinel (`registered.json`): the registration signal, its atomic +
//! create-exclusive write, and corrupt-is-a-miss probing.
//!
//! `mkdir` is not registration — a crash after `mkdir` but before the payload would leave
//! a bare directory that a dir-stat probe reads as a valid registration forever,
//! inverting the shipped safety property "a corrupt shard is a miss, never a hit"
//! (amendment C1). The sentinel is the signal: the probe stats the sentinel, never the
//! directory.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::locking::DrawerLock;
use crate::now_secs;

/// The registration sentinel filename inside a drawer.
pub(crate) const SENTINEL: &str = "registered.json";

/// The sentinel JSON schema this binary writes and understands. A sentinel whose
/// `schema` is greater is a *future* format from a newer binary: the probe
/// treats it as a miss (cold start) and registration never clobbers it — an old
/// binary against a newer drawer tree must degrade, never corrupt (amendment M4).
const SENTINEL_SCHEMA: u32 = 1;

/// The drawer's registration record and the sole authority for the reverse map
/// (drawer hash → canonical workspace path, amendment C3). Never reconstruct a
/// workspace path any other way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sentinel {
    /// Sentinel JSON schema version (see [`SENTINEL_SCHEMA`]).
    pub schema: u32,
    /// The canonical workspace path this drawer belongs to.
    pub workspace: String,
    /// Unix seconds at first registration.
    pub created_at: u64,
    /// Unix seconds of the most recent probe-hit / open / registration.
    pub last_use: u64,
    /// Set by `init` reconciliation to retire a superseded descendant drawer
    /// (amendment M2). This crate only carries the field; the CLI consumes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
}

impl Sentinel {
    /// A freshly-registered sentinel for `workspace`, `created_at == last_use ==`
    /// now.
    #[must_use]
    pub(crate) fn fresh(workspace: &Path) -> Self {
        let now = now_secs();
        Sentinel {
            schema: SENTINEL_SCHEMA,
            workspace: workspace.to_string_lossy().into_owned(),
            created_at: now,
            last_use: now,
            superseded_by: None,
        }
    }
}

/// The result of a drawer probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probe {
    /// A valid, current-schema sentinel: the drawer is registered.
    Hit(Sentinel),
    /// No sentinel, a half-created drawer, a corrupt sentinel, or a future-schema
    /// sentinel. Always cold — never an error, never a panic.
    Miss,
}

/// Minimal front-matter read: the schema gate is applied before a full decode so
/// a future-schema sentinel is recognized even if its other fields changed type.
#[derive(Deserialize)]
struct SchemaProbe {
    schema: u32,
}

/// The classified state of a sentinel file, driving both probing and the
/// registration adopt/heal decision.
pub(crate) enum SentinelState {
    /// Valid and current-schema.
    Valid(Sentinel),
    /// Parseable but a newer schema than this binary understands. Leave it
    /// untouched — a downgrade must never overwrite a newer format.
    Future,
    /// Unreadable, unparseable, or current-schema-but-wrong-shape. Safe to heal.
    Garbage,
    /// No sentinel file present.
    Absent,
}

pub(crate) fn classify(sentinel_path: &Path) -> SentinelState {
    let bytes = match fs::read(sentinel_path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return SentinelState::Absent,
        Err(_) => return SentinelState::Garbage,
    };
    let schema = match serde_json::from_slice::<SchemaProbe>(&bytes) {
        Ok(sp) => sp.schema,
        Err(_) => return SentinelState::Garbage,
    };
    if schema > SENTINEL_SCHEMA {
        return SentinelState::Future;
    }
    match serde_json::from_slice::<Sentinel>(&bytes) {
        Ok(sentinel) => SentinelState::Valid(sentinel),
        Err(_) => SentinelState::Garbage,
    }
}

/// Probe a drawer by stating its **sentinel**, never the directory.
///
/// A directory without a sentinel, a sentinel that fails to parse, a sentinel
/// with an unknown/future schema, or a wrong-shape sentinel are all a plain
/// [`Probe::Miss`] — never surfaced to the caller as an error or a panic.
#[must_use]
pub fn probe(drawer_dir: &Path) -> Probe {
    match classify(&drawer_dir.join(SENTINEL)) {
        SentinelState::Valid(sentinel) => Probe::Hit(sentinel),
        SentinelState::Future | SentinelState::Garbage | SentinelState::Absent => Probe::Miss,
    }
}

/// Register a drawer, returning the winning sentinel.
///
/// Creates the drawer directory (which is *not* the registration signal), takes
/// the per-drawer lock, then writes `registered.json` with two guarantees:
///
/// * **Atomic + crash-safe** — the record is written to a temp file, fsynced,
///   then linked/renamed into place; a crash leaves at most an orphan temp,
///   never a half-written sentinel a probe could adopt.
/// * **Create-exclusive** — concurrent first-registrars serialize on the drawer
///   lock; the loser adopts the winner's sentinel, so both callers converge on
///   one identity (amendment C2).
///
/// A pre-existing valid sentinel is adopted. A pre-existing *future* sentinel is
/// left untouched (downgrade safety) and the caller's fresh identity is returned
/// unpersisted. A pre-existing *garbage* sentinel is healed (overwritten).
///
/// # Errors
///
/// Propagates I/O failures from directory creation or the sentinel write.
pub fn register(drawer_dir: &Path, workspace: &Path) -> io::Result<Sentinel> {
    fs::create_dir_all(drawer_dir)?;
    let _ = fs::set_permissions(drawer_dir, fs::Permissions::from_mode(0o700));

    let _lock = DrawerLock::acquire(drawer_dir)?;
    let sentinel_path = drawer_dir.join(SENTINEL);

    // Fast path: a valid sentinel already exists (we won a prior race or a
    // concurrent registrar won this one). Adopt it — identities converge.
    if let SentinelState::Valid(sentinel) = classify(&sentinel_path) {
        return Ok(sentinel);
    }

    let mine = Sentinel::fresh(workspace);
    let bytes = to_bytes(&mine)?;
    let tmp = write_tmp(drawer_dir, &bytes)?;

    // hard_link is atomic and fails with AlreadyExists if the sentinel exists —
    // exclusive-create and atomicity in one primitive.
    match fs::hard_link(&tmp, &sentinel_path) {
        Ok(()) => {
            fsync_dir(drawer_dir);
            let _ = fs::remove_file(&tmp);
            Ok(mine)
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => match classify(&sentinel_path) {
            // Winner beat us to it under the same lock generation — adopt.
            SentinelState::Valid(sentinel) => {
                let _ = fs::remove_file(&tmp);
                Ok(sentinel)
            }
            // Newer format — never clobber a downgrade; report our identity
            // (same workspace) without persisting.
            SentinelState::Future => {
                let _ = fs::remove_file(&tmp);
                Ok(mine)
            }
            // Garbage or vanished — heal by atomically replacing it (we hold
            // the lock, so this is the single writer).
            SentinelState::Garbage | SentinelState::Absent => {
                fs::rename(&tmp, &sentinel_path)?;
                fsync_dir(drawer_dir);
                Ok(mine)
            }
        },
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Stamp a drawer's `last_use` to now under the per-drawer lock.
///
/// A no-op when the drawer has no valid current-schema sentinel (nothing to
/// stamp). Because the lock is held on the stable directory inode, the atomic
/// tmp+rename replacement is the single writer.
///
/// # Errors
///
/// Propagates I/O failures from the sentinel rewrite.
pub fn stamp_last_use(drawer_dir: &Path) -> io::Result<()> {
    if !drawer_dir.exists() {
        return Ok(());
    }
    let _lock = DrawerLock::acquire(drawer_dir)?;
    let sentinel_path = drawer_dir.join(SENTINEL);
    let SentinelState::Valid(mut sentinel) = classify(&sentinel_path) else {
        return Ok(());
    };
    sentinel.last_use = now_secs();
    let bytes = to_bytes(&sentinel)?;
    let tmp = write_tmp(drawer_dir, &bytes)?;
    fs::rename(&tmp, &sentinel_path)?;
    fsync_dir(drawer_dir);
    Ok(())
}

/// Stamp a drawer's `superseded_by` under the per-drawer lock (amendment M2):
/// `mrd init` calls this to retire a tier-4 descendant drawer whose workspace
/// is now inside a newer marker root, recording the superseding workspace path.
///
/// A no-op when the drawer has no valid current-schema sentinel — a
/// half-created, corrupt, or future-schema drawer is left untouched (never
/// heal a downgrade). Returns whether the sentinel was stamped. The atomic
/// tmp+rename replacement under the held lock is the single writer.
///
/// # Errors
///
/// Propagates I/O failures from the sentinel rewrite.
pub fn supersede(drawer_dir: &Path, superseded_by: &str) -> io::Result<bool> {
    if !drawer_dir.exists() {
        return Ok(false);
    }
    let _lock = DrawerLock::acquire(drawer_dir)?;
    let sentinel_path = drawer_dir.join(SENTINEL);
    let SentinelState::Valid(mut sentinel) = classify(&sentinel_path) else {
        return Ok(false);
    };
    sentinel.superseded_by = Some(superseded_by.to_owned());
    let bytes = to_bytes(&sentinel)?;
    let tmp = write_tmp(drawer_dir, &bytes)?;
    fs::rename(&tmp, &sentinel_path)?;
    fsync_dir(drawer_dir);
    Ok(true)
}

fn to_bytes(sentinel: &Sentinel) -> io::Result<Vec<u8>> {
    serde_json::to_vec_pretty(sentinel).map_err(io::Error::other)
}

/// Write `data` to a uniquely-named temp file in `dir`, fsynced, mode 0600. The
/// name is never `registered.json`, so a crash before link/rename leaves an
/// orphan temp that a probe never adopts.
fn write_tmp(dir: &Path, data: &[u8]) -> io::Result<PathBuf> {
    let pid = std::process::id();
    let nanos = SystemTimeNanos::now();
    let mut counter: u64 = 0;
    loop {
        let path = dir.join(format!(".{SENTINEL}.{pid}.{nanos}.{counter}.tmp"));
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
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                counter += 1;
            }
            Err(e) => return Err(e),
        }
    }
}

/// fsync a directory so a completed link/rename survives power loss. Best-effort:
/// the entry is already visible to readers, so a failed dir sync only weakens
/// durability — it must never turn a committed write into a reported failure.
fn fsync_dir(dir: &Path) {
    if let Ok(f) = File::open(dir) {
        let _ = f.sync_all();
    }
}

/// Whole-nanosecond clock reading for temp-file name uniqueness.
struct SystemTimeNanos(u128);

impl SystemTimeNanos {
    fn now() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        SystemTimeNanos(nanos)
    }
}

impl std::fmt::Display for SystemTimeNanos {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supersede_stamps_a_valid_sentinel_and_keeps_it_a_hit() {
        let dir = tempfile::tempdir().unwrap();
        let drawer = dir.path().join("drawer");
        register(&drawer, Path::new("/ws/leaf")).unwrap();

        assert!(supersede(&drawer, "/ws/marker-root").unwrap(), "stamped");
        let Probe::Hit(s) = probe(&drawer) else {
            panic!("a retired drawer stays a valid Hit, just marked superseded");
        };
        assert_eq!(s.superseded_by.as_deref(), Some("/ws/marker-root"));
        assert_eq!(s.workspace, "/ws/leaf", "identity unchanged by retirement");
    }

    #[test]
    fn supersede_never_clobbers_a_future_schema_sentinel() {
        let dir = tempfile::tempdir().unwrap();
        let drawer = dir.path().join("drawer");
        fs::create_dir_all(&drawer).unwrap();
        let future = drawer.join(SENTINEL);
        fs::write(&future, br#"{"schema":999,"workspace":"/ws/new"}"#).unwrap();

        assert!(
            !supersede(&drawer, "/ws/marker-root").unwrap(),
            "a downgrade must never rewrite a newer-schema sentinel"
        );
        // The future sentinel is left byte-identical.
        assert_eq!(
            fs::read_to_string(&future).unwrap(),
            r#"{"schema":999,"workspace":"/ws/new"}"#
        );
    }

    #[test]
    fn supersede_missing_drawer_is_a_noop() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!supersede(&dir.path().join("absent"), "/ws/x").unwrap());
    }
}
