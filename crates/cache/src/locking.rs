//! Per-drawer advisory lock (amendment m3).
//!
//! Registration mutations, last-use stamping, GC, and targeted removal all take
//! this lock so the reaper never races a live workspace. The lock is `flock(2)`
//! on the **drawer directory** fd — the directory inode is stable (never
//! renamed), so it correctly serializes callers that atomically replace the
//! sentinel via tmp+rename (which swaps the sentinel inode out from under any
//! lock held on the sentinel file itself).
//!
//! `flock` locks are associated with the open file description, so two
//! independent opens of one directory — even within a single process — contend:
//! a held exclusive lock makes another exclusive `try_acquire` return `None`.

use std::fs::File;
use std::io;
use std::os::unix::io::AsRawFd;
use std::path::Path;

/// An exclusive advisory lock on a drawer directory, released on drop — by an
/// EXPLICIT unlock, not by the fd close (see the [`Drop`] impl: relying on the
/// close leaks the lock into any concurrently forking subprocess, and this lock
/// is the one whose leak HANGS).
#[derive(Debug)]
pub struct DrawerLock {
    // Held open for its fd; released by the explicit `flock(LOCK_UN)` in Drop.
    dir: File,
}

/// Release the lock EXPLICITLY, before the fd closes — the S7/R19 fix, applied
/// here third, and here it matters most.
///
/// # Why the fd close is not enough (measured on the two siblings, not theorised)
/// A `flock` lock belongs to the open file DESCRIPTION, and a `fork` duplicates
/// every descriptor. Any thread in this process spawning any subprocess — the
/// `git` the registry daemon's connection threads fork inside `splice`, a bash
/// task, anything — transiently holds a copy of this fd between its fork and its
/// exec, even with `FD_CLOEXEC` set (CLOEXEC acts at exec, not at fork). If this
/// guard dropped in that window, closing our fd would NOT release the lock: the
/// child's copy keeps the description alive until it execs.
///
/// **The consequence here is worse than on either sibling.** [`Self::acquire`]
/// is the only blocking acquire in the codebase (`LOCK_EX` with no `LOCK_NB`),
/// so a leaked description does not degrade to `fs::WriteLock`'s and
/// `run::executor::WorkspaceLock`'s fast typed `workspace_busy` refusal — it
/// degrades to a HANG on a lock whose holder has already finished. The two live
/// paths, both inside the registry daemon: the process-lifetime singleton, whose
/// leak makes a successor refuse *"another daemon is already running"* for a
/// daemon that has exited; and the per-drawer lock [`crate::register`] takes
/// from a connection thread.
///
/// `LOCK_UN` acts on the description itself, so one unlock here releases the
/// lock no matter how many copies of the fd exist. Pinned by
/// `tests/drawer_lock_release.rs`, whose control test proves the fork window it
/// measures is genuinely open.
impl Drop for DrawerLock {
    fn drop(&mut self) {
        // SAFETY: flock on a valid open fd we own; the fd outlives the call.
        unsafe { libc::flock(self.dir.as_raw_fd(), libc::LOCK_UN) };
    }
}

impl DrawerLock {
    /// Take the exclusive lock, blocking until it is available.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the directory cannot be opened (e.g. it does not
    /// exist) or `flock` fails for a reason other than contention.
    pub fn acquire(drawer_dir: &Path) -> io::Result<Self> {
        flock(drawer_dir, libc::LOCK_EX)
    }

    /// Try to take the exclusive lock without blocking. Returns `Ok(None)` when
    /// another holder currently owns the lock (the drawer is live).
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the directory cannot be opened or `flock` fails
    /// for a reason other than contention.
    pub fn try_acquire(drawer_dir: &Path) -> io::Result<Option<Self>> {
        match flock(drawer_dir, libc::LOCK_EX | libc::LOCK_NB) {
            Ok(lock) => Ok(Some(lock)),
            Err(e) if e.raw_os_error() == Some(libc::EWOULDBLOCK) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

fn flock(drawer_dir: &Path, op: libc::c_int) -> io::Result<DrawerLock> {
    let dir = File::open(drawer_dir)?;
    // SAFETY: `dir` owns a valid open fd for the duration of the call; `flock`
    // only reads it and takes an advisory lock.
    let rc = unsafe { libc::flock(dir.as_raw_fd(), op) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(DrawerLock { dir })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusive_lock_blocks_a_second_try() {
        let dir = tempfile::tempdir().unwrap();
        let held = DrawerLock::acquire(dir.path()).unwrap();
        // Same process, independent open → contends.
        assert!(
            DrawerLock::try_acquire(dir.path()).unwrap().is_none(),
            "a held exclusive lock must block a non-blocking try"
        );
        drop(held);
        assert!(
            DrawerLock::try_acquire(dir.path()).unwrap().is_some(),
            "released lock is re-acquirable"
        );
    }

    #[test]
    fn acquire_on_missing_dir_errors() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope");
        assert!(DrawerLock::acquire(&missing).is_err());
    }
}
