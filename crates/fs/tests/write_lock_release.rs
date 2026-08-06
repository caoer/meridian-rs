//! The write flock releases when its guard drops — even while this process has a
//! forked child holding a copy of the lock fd.
//!
//! A `flock` lock belongs to the open file description, and `fork` duplicates
//! every descriptor, so a guard released by closing its fd stays held while any
//! forked child holds a copy — `FD_CLOEXEC` acts at exec, not at fork.
//! `WriteLock::drop` therefore unlocks explicitly with `LOCK_UN`, which acts on
//! the description itself.
//!
//! The window is opened with a raw `fork(2)` whose child parks and never execs:
//! `Command::spawn` returns only after the child has exec'd, closing the
//! `O_CLOEXEC` lock fd, so a spawn-driven test passes even with the explicit
//! unlock reverted. `the_fork_window_is_real_…` is the control — against a raw
//! `flock` this crate does not own, it asserts that closing the fd does not
//! release a lock while the child is parked, and fails the day the mechanism
//! stops reproducing the hazard.

use std::io;
use std::os::unix::io::AsRawFd;
use std::path::Path;

/// Only one forked holder may exist in this process at a time. A holder parks
/// on EVERY fd this process had open at its fork, so two overlapping holders
/// would each pin the other's lock fd and both tests would read a lock as held
/// by the wrong child. Other test binaries are separate processes and share no
/// descriptors, so this lock is enough.
static ONE_HOLDER_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A child forked while a lock is held, parked so that it OUTLIVES the parent's
/// release while still holding its inherited copy of the lock fd.
///
/// The child never execs, so `FD_CLOEXEC` never fires: this is the fork→exec
/// window of the real hazard, held open on demand instead of raced for.
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
    ///
    /// Asserted immediately before every reacquire, so a child that died early
    /// fails the test loudly instead of passing it vacuously.
    fn is_parked(&self) -> bool {
        let mut status: libc::c_int = 0;
        // SAFETY: waitpid on our own child, non-blocking, into a local.
        unsafe { libc::waitpid(self.pid, &raw mut status, libc::WNOHANG) == 0 }
    }

    /// Close the window: kill the child, and reap it. Signalled rather than
    /// asked, so no fd another thread may have inherited can delay it.
    fn release_and_reap(self) {
        // SAFETY: signalling our own child by the pid fork(2) returned.
        unsafe { libc::kill(self.pid, libc::SIGKILL) };
        let mut status: libc::c_int = 0;
        // SAFETY: waitpid on our own child; blocks only until it is reaped.
        unsafe { libc::waitpid(self.pid, &raw mut status, 0) };
    }
}

/// A `flock` this crate does not own, acquired the same `LOCK_EX | LOCK_NB` way
/// [`fs::WriteLock`] does. `None` is the busy refusal.
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

/// The control: release by fd close, the pre-fix mechanism, against a raw
/// `flock` that `fs` does not own. While the forked child is parked, closing our
/// fd must leave the lock held, and reaping the child must free it. A trivial
/// pass here means the fork window is not open on this platform, and the
/// regression test below proves nothing.
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

/// The regression proper: the guard's drop releases the lock even though a
/// forked child holds a copy of its fd at that instant. With the explicit
/// `LOCK_UN` reverted to a bare fd close this fails deterministically.
#[test]
fn dropping_the_lock_releases_it_across_a_forked_child() {
    let _serialized = ONE_HOLDER_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().expect("tempdir");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());

    let lock = fs::WriteLock::acquire(&root).expect("the lock must be free to start");
    // Forked WHILE the lock is held: the child takes no lock of its own, it only
    // has to still be holding the inherited fd when the guard drops below.
    let child = ForkedFdHolder::fork();
    drop(lock);

    assert!(
        child.is_parked(),
        "the forked child died before the assert — the window was never open"
    );
    let reacquired = fs::WriteLock::acquire(&root);
    child.release_and_reap();

    reacquired.unwrap_or_else(|e| {
        panic!(
            "the lock was NOT released by its guard's drop ({e}) — a forked child's copy of \
             the fd is holding the open file description, so WriteLock is releasing by fd \
             close instead of by an explicit LOCK_UN"
        )
    });
}

/// The other half, unchanged by the fix: a lock that is genuinely held refuses
/// the second acquire immediately (`WouldBlock`, never a wait) — in-process
/// contention included, since `flock` contends per open file description.
#[test]
fn a_held_lock_still_refuses_a_second_acquire_without_blocking() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    let held = fs::WriteLock::acquire(&root).expect("first acquire");
    let err = fs::WriteLock::acquire(&root).expect_err("a held lock refuses");
    assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
    drop(held);
    fs::WriteLock::acquire(&root).expect("free again once the holder drops");
}
