//! The Cargo-grade last-use GC sweep, drawer listing (`cache ls`), and targeted removal
//! (`unregister` / `clean`).
//!
//! A path-keyed drawer store without a reaper is the Bazel/VSCode disk-leak class
//! (decision 0001 round 4). GC reaps only drawers with a **valid current-schema
//! sentinel** whose last-use is past the threshold and that are **not currently locked**
//! by a live workspace (amendment m3). Corrupt and future-schema drawers are left for
//! explicit `clean` — GC never guesses a workspace path any way but the sentinel
//! (amendment C3).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::locking::DrawerLock;
use crate::now_secs;
use crate::sentinel::{Sentinel, SentinelState, classify};

/// One drawer's listing, sourced entirely from its sentinel (the reverse-map
/// authority) plus on-disk size.
#[derive(Debug, Clone)]
pub struct DrawerInfo {
    /// The drawer-key directory name (`sha256(workspace)[:16]`).
    pub key: String,
    /// The `<version>-<salt>` segment directory name.
    pub version_segment: String,
    /// The drawer directory, `<cache-root>/<key>/<version_segment>/`.
    pub drawer_dir: PathBuf,
    /// The canonical workspace path, read from the sentinel.
    pub workspace: String,
    /// Total payload size of the drawer in bytes.
    pub size_bytes: u64,
    /// Unix seconds of last use, from the sentinel.
    pub last_use: u64,
    /// The sentinel's `superseded_by`, if set.
    pub superseded_by: Option<String>,
}

/// What a GC sweep did.
#[derive(Debug, Default, Clone)]
pub struct GcReport {
    /// Drawers removed (past threshold, not locked).
    pub removed: Vec<PathBuf>,
    /// Past-threshold drawers skipped because a live workspace held the lock.
    pub skipped_locked: Vec<PathBuf>,
    /// Total bytes freed by removals.
    pub freed_bytes: u64,
}

/// List every registered drawer under `cache_root`.
///
/// Walks `<cache-root>/<key>/<version-segment>/` two levels deep and reads each
/// valid sentinel. Drawers without a valid current-schema sentinel (half-created,
/// corrupt, future) are skipped — they have no authoritative workspace path.
///
/// # Errors
///
/// Propagates I/O errors from reading the cache-root directory levels. A missing
/// cache root is not an error — it yields an empty list.
pub fn list_drawers(cache_root: &Path) -> io::Result<Vec<DrawerInfo>> {
    let mut out = Vec::new();
    for (key, key_dir) in read_subdirs(cache_root)? {
        for (version_segment, drawer_dir) in read_subdirs(&key_dir)? {
            let SentinelState::Valid(sentinel) =
                classify(&drawer_dir.join(crate::sentinel::SENTINEL))
            else {
                continue;
            };
            let Sentinel {
                workspace,
                last_use,
                superseded_by,
                ..
            } = sentinel;
            out.push(DrawerInfo {
                key: key.clone(),
                version_segment,
                size_bytes: dir_size(&drawer_dir),
                workspace,
                last_use,
                superseded_by,
                drawer_dir,
            });
        }
    }
    Ok(out)
}

/// Sweep drawers whose last-use exceeds `threshold`, using the current clock.
///
/// # Errors
///
/// Propagates I/O errors from listing or removal.
pub fn gc(cache_root: &Path, threshold: Duration) -> io::Result<GcReport> {
    gc_at(cache_root, threshold, now_secs())
}

/// Sweep at an explicit `now` (unix seconds) — the injectable-clock form.
///
/// A drawer is reaped only when `now - last_use > threshold` **and** its
/// per-drawer lock is free. A live workspace holding the lock survives the sweep.
///
/// # Errors
///
/// Propagates I/O errors from listing or removal.
pub(crate) fn gc_at(cache_root: &Path, threshold: Duration, now: u64) -> io::Result<GcReport> {
    let horizon = threshold.as_secs();
    let mut report = GcReport::default();
    for info in list_drawers(cache_root)? {
        let age = now.saturating_sub(info.last_use);
        if age <= horizon {
            continue;
        }
        match DrawerLock::try_acquire(&info.drawer_dir)? {
            None => report.skipped_locked.push(info.drawer_dir),
            Some(_lock) => {
                fs::remove_dir_all(&info.drawer_dir)?;
                remove_empty_parent(&info.drawer_dir);
                report.freed_bytes += info.size_bytes;
                report.removed.push(info.drawer_dir);
            }
        }
    }
    Ok(report)
}

/// Remove one drawer directory (for `unregister` / `clean`), taking the
/// per-drawer lock first. A missing drawer is a no-op success.
///
/// # Errors
///
/// Propagates I/O errors from locking or removal.
pub fn remove_drawer(drawer_dir: &Path) -> io::Result<()> {
    if !drawer_dir.exists() {
        return Ok(());
    }
    let _lock = DrawerLock::acquire(drawer_dir)?;
    fs::remove_dir_all(drawer_dir)?;
    remove_empty_parent(drawer_dir);
    Ok(())
}

/// Read the immediate subdirectories of `dir` as `(name, path)` pairs. A missing
/// `dir` yields an empty list (not an error).
fn read_subdirs(dir: &Path) -> io::Result<Vec<(String, PathBuf)>> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        if let Some(name) = entry.file_name().to_str() {
            out.push((name.to_owned(), entry.path()));
        }
    }
    Ok(out)
}

/// Total size in bytes of all regular files under `dir`, recursively.
/// Best-effort: unreadable entries contribute zero.
fn dir_size(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let mut total = 0;
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            total += dir_size(&entry.path());
        } else if let Ok(meta) = entry.metadata() {
            total += meta.len();
        }
    }
    total
}

/// Remove the parent (drawer-key) directory if it is now empty. Best-effort.
fn remove_empty_parent(drawer_dir: &Path) {
    let Some(parent) = drawer_dir.parent() else {
        return;
    };
    if let Ok(mut entries) = fs::read_dir(parent)
        && entries.next().is_none()
    {
        let _ = fs::remove_dir(parent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sentinel::register;

    const DAY: u64 = 24 * 60 * 60;

    // Register a drawer under `root` keyed by a distinct workspace path, then
    // return its directory. The sentinel's last_use is "now" at registration.
    fn make_drawer(root: &Path, workspace: &str) -> PathBuf {
        let dir = crate::layout::drawer_dir(root, Path::new(workspace));
        register(&dir, Path::new(workspace)).unwrap();
        dir
    }

    #[test]
    fn gc_reaps_only_past_threshold() {
        let root = tempfile::tempdir().unwrap();
        let stale = make_drawer(root.path(), "/ws/stale");
        let fresh = make_drawer(root.path(), "/ws/fresh");

        // last_use == registration time (t0). Freshen one drawer to t0 + 40d, and
        // sweep as if now == t0 + 40d, threshold 30d: only the un-freshened
        // (stale, age 40d) drawer is past threshold.
        let t0 = now_secs();
        stamp_at(&fresh, t0 + 40 * DAY);
        let report = gc_at(root.path(), Duration::from_secs(30 * DAY), t0 + 40 * DAY).unwrap();

        assert_eq!(report.removed, vec![stale.clone()], "stale drawer reaped");
        assert!(!stale.exists(), "stale drawer dir gone");
        assert!(fresh.exists(), "fresh drawer survives");
        assert!(report.skipped_locked.is_empty());
    }

    #[test]
    fn gc_skips_flocked_live_drawer() {
        let root = tempfile::tempdir().unwrap();
        let live = make_drawer(root.path(), "/ws/live");
        let dead = make_drawer(root.path(), "/ws/dead");

        let t0 = now_secs();
        // Both are past threshold; a live workspace holds the lock on `live`.
        let held = DrawerLock::acquire(&live).unwrap();
        let report = gc_at(root.path(), Duration::from_secs(30 * DAY), t0 + 40 * DAY).unwrap();
        drop(held);

        assert_eq!(
            report.removed,
            vec![dead.clone()],
            "unlocked past-threshold reaped"
        );
        assert_eq!(
            report.skipped_locked,
            vec![live.clone()],
            "locked drawer skipped"
        );
        assert!(live.exists(), "flocked live drawer survives GC");
        assert!(!dead.exists());
    }

    #[test]
    fn list_reports_workspace_from_sentinel() {
        let root = tempfile::tempdir().unwrap();
        make_drawer(root.path(), "/ws/one");
        make_drawer(root.path(), "/ws/two");
        let mut listed: Vec<String> = list_drawers(root.path())
            .unwrap()
            .into_iter()
            .map(|d| d.workspace)
            .collect();
        listed.sort();
        assert_eq!(listed, vec!["/ws/one".to_owned(), "/ws/two".to_owned()]);
    }

    #[test]
    fn remove_drawer_is_idempotent() {
        let root = tempfile::tempdir().unwrap();
        let dir = make_drawer(root.path(), "/ws/gone");
        remove_drawer(&dir).unwrap();
        assert!(!dir.exists());
        remove_drawer(&dir).unwrap(); // missing → no-op success
    }

    #[test]
    fn list_and_gc_ignore_missing_cache_root() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("never-created");
        assert!(list_drawers(&missing).unwrap().is_empty());
        let report = gc_at(&missing, Duration::from_secs(DAY), now_secs()).unwrap();
        assert!(report.removed.is_empty());
    }

    // Rewrite a drawer's sentinel last_use to an explicit time (test seam for the
    // injectable-clock GC; production stamping always uses the wall clock).
    fn stamp_at(drawer_dir: &Path, when: u64) {
        let path = drawer_dir.join(crate::sentinel::SENTINEL);
        let SentinelState::Valid(mut s) = classify(&path) else {
            panic!("expected a valid sentinel to re-stamp");
        };
        s.last_use = when;
        fs::write(&path, serde_json::to_vec_pretty(&s).unwrap()).unwrap();
    }
}
