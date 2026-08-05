//! Decision-#9 run flock releases when its guard drops — even while a forked
//! child still holds a duplicate fd. Proves the S7/R19 explicit-release path:
//! `WorkspaceLock` unlocks before close so waiters are not blocked by orphaned
//! fds. Companion to the executor's `LOCK_NB` busy refusal.
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!

use std::io;
use std::os::unix::io::AsRawFd;
use std::path::Path;

use run::executor::WorkspaceLock;

/// Only one forked holder may exist in this process at a time: a holder parks on
/// EVERY fd open at its fork, so two overlapping holders would each pin the
/// other's lock fd. Other test binaries are separate processes and share no
/// descriptors, so this lock is enough.
static ONE_HOLDER_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A child forked while the lock is held, parked so that it OUTLIVES the
/// parent's release while still holding its inherited copy of the lock fd.
struct ForkedFdHolder {
    pid: libc::pid_t,
}

impl ForkedFdHolder {
    /// Fork a child holding a copy of every fd this process has open right now.
    fn fork() -> Self {
        // SAFETY: fork(2) from a threaded test binary. The child branch below
        // calls only async-signal-safe functions (alarm / pause) and never
        // returns into Rust, which is that contract in full.
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork: {}", io::Error::last_os_error());
        if pid == 0 {
            // SAFETY: the child. Async-signal-safe calls only, and it never
            // returns — the parent's SIGKILL is what ends it.
            unsafe {
                // Backstop only: never outlive a parent that died mid-test.
                libc::alarm(60);
                loop {
                    libc::pause();
                }
            }
        }
        Self { pid }
    }

    /// Is the child still parked — i.e. is the fork window open AT THIS MOMENT?
    /// Asserted immediately before every reacquire, so a child that died early
    /// fails the test loudly instead of passing it vacuously.
    fn is_parked(&self) -> bool {
        let mut status: libc::c_int = 0;
        // SAFETY: waitpid on our own child, non-blocking, into a local.
        unsafe { libc::waitpid(self.pid, &raw mut status, libc::WNOHANG) == 0 }
    }

    /// Close the window: kill the child, and reap it.
    fn release_and_reap(self) {
        // SAFETY: signalling our own child by the pid fork(2) returned.
        unsafe { libc::kill(self.pid, libc::SIGKILL) };
        let mut status: libc::c_int = 0;
        // SAFETY: waitpid on our own child; blocks only until it is reaped.
        unsafe { libc::waitpid(self.pid, &raw mut status, 0) };
    }
}

/// A `flock` this crate does not own, acquired the same `LOCK_EX | LOCK_NB` way
/// [`WorkspaceLock`] does. `None` is the busy refusal.
fn raw_flock(path: &Path) -> Option<std::fs::File> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .expect("open the lock file");
    // SAFETY: flock on a valid open fd we own; the fd outlives the call.
    (unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0).then_some(file)
}

/// THE CONTROL — it proves the window is open, and it is the reason the
/// regression test below cannot go quietly vacuous. Release by fd close, the
/// pre-fix mechanism, against a raw `flock` this crate does not own: while the
/// forked child is parked, closing our fd must leave the lock HELD, and reaping
/// the child must free it.
#[test]
fn the_fork_window_is_real_a_forked_child_holds_the_lock_past_our_close() {
    let _serialized = ONE_HOLDER_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("control.lock");

    let held = raw_flock(&path).expect("the control lock must be free to start");
    let child = ForkedFdHolder::fork();
    drop(held); // fd close = the pre-fix release mechanism, verbatim

    assert!(
        child.is_parked(),
        "the forked child died before the assert — the window it exists to hold was never open"
    );
    let during = raw_flock(&path);
    let still_held = during.is_none();
    drop(during);
    child.release_and_reap();

    assert!(
        still_held,
        "closing the fd released the lock even with a forked child holding a copy of it — \
         the fork/exec hazard does not reproduce here, so this file's regression test proves nothing"
    );
    assert!(
        raw_flock(&path).is_some(),
        "the lock stayed held after the child was reaped — something other than the child's \
         fd copy is holding it, so the control does not isolate what it claims to"
    );
}

/// The regression proper: the guard's drop releases the run lock even though a
/// forked child is holding a copy of its fd at that instant.
///
/// The assert IS the claim — `WorkspaceLock::acquire` succeeding while the
/// window is provably open. With the explicit `LOCK_UN` removed this fails
/// deterministically, because the control above proves the child's copy keeps
/// the description alive.
#[test]
fn dropping_the_run_lock_releases_it_across_a_forked_child() {
    let _serialized = ONE_HOLDER_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();

    let lock = WorkspaceLock::acquire(root).expect("the run lock must be free to start");
    // Forked WHILE the lock is held: the child takes no lock of its own, it only
    // has to still be holding the inherited fd when the guard drops below. This
    // is a sibling run's task child, forked while this run holds the lock.
    let child = ForkedFdHolder::fork();
    drop(lock);

    assert!(
        child.is_parked(),
        "the forked child died before the assert — the window was never open"
    );
    let reacquired = WorkspaceLock::acquire(root);
    child.release_and_reap();

    reacquired.unwrap_or_else(|e| {
        panic!(
            "the run lock was NOT released by its guard's drop ({e}) — a forked child's copy \
             of the fd is holding the open file description, so WorkspaceLock is releasing by \
             fd close instead of by an explicit LOCK_UN"
        )
    });
}

/// The other half, unchanged by the fix: a lock that is genuinely HELD refuses
/// the second acquire immediately (`WouldBlock`, never a wait) — in-process
/// contention included, since `flock` contends per open file description.
#[test]
fn a_held_run_lock_still_refuses_a_second_acquire_without_blocking() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    let held = WorkspaceLock::acquire(root).expect("first acquire");
    let err = WorkspaceLock::acquire(root).expect_err("a held lock refuses");
    assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
    drop(held);
    WorkspaceLock::acquire(root).expect("free again once the holder drops");
}
