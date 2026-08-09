//! The m3 drawer flock releases when its guard drops — even while this process
//! has a forked child holding a copy of the lock fd (R19).
//!
//! A `flock` lock belongs to the open file description; `fork` duplicates
//! every descriptor and `FD_CLOEXEC` acts at exec, not fork. A guard that
//! released by letting its `File` close would leak the lock into any fork
//! window — and `DrawerLock::acquire` blocks (`LOCK_EX`, no `LOCK_NB`), so
//! the leak is a hang, not a typed refusal.
//!
//! Two live paths, both inside the registry daemon: the process-lifetime
//! singleton (a leaked description makes a successor daemon's `try_acquire`
//! refuse for a daemon that has exited) and the per-drawer lock
//! `cache::register` takes from a connection thread (the next `register`
//! hangs). Each is asserted through the acquire mode its production site uses.
//!
//! This file forks by hand: `Command::spawn` returns only after the child has
//! exec'd (which closes the `O_CLOEXEC` lock fd), so a spawn-driven test has
//! no live fd copy at the moment of the drop. The child parks on `pause()`
//! and never execs, holding the window open; the control test asserts the
//! window really is open, so this file cannot go quietly vacuous.

use std::io;
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// How long a blocking acquire may take before this harness calls it hung.
///
/// The defect's signature on the blocking path is a wait that never ends, so
/// the wait is bounded and the bound is the assert. The operation under test
/// is one uncontended `flock(2)` — microseconds when free — so ten seconds is
/// six orders of magnitude of headroom: a timeout is the lock genuinely still
/// held, never scheduler noise.
const ACQUIRE_BOUND: Duration = Duration::from_secs(10);

/// Every test in this file is serialized, not just the forking ones: a forked
/// holder parks on every fd this process had open at its fork, so an
/// overlapping test's held lock would be pinned by a sibling test's child.
/// Other test binaries are separate processes and share no descriptors.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A child forked while a lock is held, parked so that it outlives the
/// parent's release while still holding its inherited copy of the lock fd.
/// It never execs, so `FD_CLOEXEC` never fires — the fork→exec window of the
/// real hazard, held open on demand instead of raced for.
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

    /// Is the child still parked — i.e. is the fork window open right now?
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

/// Run a blocking acquire on a helper thread and wait at most [`ACQUIRE_BOUND`].
///
/// `None` means it did not finish inside the bound — on a blocking `LOCK_EX`
/// that is the observable defect, not an inconclusive result.
///
/// The helper thread is left parked on timeout. It completes on its own once the
/// lock frees (every test here frees it by reaping its child), and its value is
/// dropped in the thread because the receiver is gone — so a timed-out probe
/// leaks neither the lock nor the fd past the end of the test.
fn within_bound<T: Send + 'static>(acquire: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(acquire());
    });
    rx.recv_timeout(ACQUIRE_BOUND).ok()
}

/// A `flock` this crate does not own, taken the same blocking `LOCK_EX` way
/// [`cache::DrawerLock::acquire`] takes it.
fn raw_flock_blocking(path: &Path) -> std::fs::File {
    let file = open_for_lock(path);
    // SAFETY: flock on a valid open fd we own; the fd outlives the call.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
    assert_eq!(rc, 0, "raw blocking flock: {}", io::Error::last_os_error());
    file
}

/// The same raw `flock`, taken the non-blocking `LOCK_EX | LOCK_NB` way
/// [`cache::DrawerLock::try_acquire`] takes it. `None` is the busy refusal.
fn raw_flock_try(path: &Path) -> Option<std::fs::File> {
    let file = open_for_lock(path);
    // SAFETY: flock on a valid open fd we own; the fd outlives the call.
    (unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0).then_some(file)
}

/// `DrawerLock` locks the drawer directory fd, so the control must lock a
/// directory too — the same kind of inode production uses.
fn open_for_lock(path: &Path) -> std::fs::File {
    std::fs::File::open(path).expect("open the directory to lock")
}

/// The control — proves the fork window is open, so the regression tests
/// below cannot go quietly vacuous. Release by fd close (the pre-fix
/// mechanism) against a raw `flock` this crate does not own: while the forked
/// child is parked, closing our fd must leave the lock held, through both
/// acquire modes. Also validates [`within_bound`] in both directions.
#[test]
fn the_fork_window_is_real_a_forked_child_holds_the_lock_past_our_close() {
    let _serialized = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join("control");
    std::fs::create_dir(&dir).expect("create the control directory");

    let held = raw_flock_blocking(&dir);
    let child = ForkedFdHolder::fork();
    drop(held); // fd close = the pre-R19 release mechanism, verbatim

    assert!(
        child.is_parked(),
        "the forked child died before the assert — the window it exists to hold was never open"
    );

    // Probe 1, the singleton's mode: a non-blocking try must refuse.
    let refused_while_parked = raw_flock_try(&dir).is_none();

    // Probe 2, the drawer's mode: a blocking acquire must not complete — the
    // hang, observed as a bounded wait that expires.
    let probe_dir = dir.clone();
    let hung_while_parked = within_bound(move || raw_flock_blocking(&probe_dir)).is_none();

    child.release_and_reap();

    assert!(
        refused_while_parked,
        "closing the fd released the lock even with a forked child holding a copy of it — \
         the fork/exec hazard does not reproduce here, so this file's regression tests prove nothing"
    );
    assert!(
        hung_while_parked,
        "a blocking acquire completed while the lock was still held by the child's fd copy — \
         either the hazard does not reproduce here, or within_bound is not actually waiting, \
         and either way the bounded asserts below prove nothing"
    );

    // The acceptance: with the child reaped the lock must be free, through
    // both probes — proving the child's fd copy was the sole holder.
    let free_dir = dir.clone();
    assert!(
        within_bound(move || raw_flock_blocking(&free_dir)).is_some(),
        "the lock stayed held after the child was reaped — something other than the child's \
         fd copy is holding it, so the control does not isolate what it claims to"
    );
    assert!(
        raw_flock_try(&dir).is_some(),
        "the non-blocking probe still refuses after the child was reaped — the control does \
         not isolate what it claims to"
    );
}

/// Path 2, at the seam: the guard's drop releases the drawer lock even though
/// a forked child holds a copy of its fd at that instant. Without the
/// explicit `LOCK_UN` this hangs rather than failing fast; the bound turns
/// the hang into a report.
#[test]
fn dropping_a_drawer_lock_releases_it_across_a_forked_child() {
    let _serialized = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().expect("tempdir");
    let drawer = tmp.path().join("drawer");
    std::fs::create_dir(&drawer).expect("create the drawer directory");

    let lock = cache::DrawerLock::acquire(&drawer).expect("the drawer lock must be free to start");
    // Forked while the lock is held: the child takes no lock of its own, it
    // only has to still hold the inherited fd when the guard drops below.
    let child = ForkedFdHolder::fork();
    drop(lock);

    assert!(
        child.is_parked(),
        "the forked child died before the assert — the window was never open"
    );
    let reacquire_dir = drawer.clone();
    let reacquired = within_bound(move || cache::DrawerLock::acquire(&reacquire_dir));
    child.release_and_reap();

    match reacquired {
        None => panic!(
            "DrawerLock::acquire did not return within {ACQUIRE_BOUND:?} — the lock was NOT \
             released by its guard's drop. A forked child's copy of the fd is holding the open \
             file description, so DrawerLock is releasing by fd close instead of by an explicit \
             LOCK_UN. This acquire is blocking (LOCK_EX, no LOCK_NB), so in production this is \
             a HANG, not a refusal"
        ),
        Some(Err(e)) => panic!("DrawerLock::acquire failed for an unrelated reason: {e}"),
        Some(Ok(_)) => {}
    }
}

/// Path 2, at the production entry point: `cache::register` takes the same
/// blocking `DrawerLock::acquire` the test above takes directly. The released
/// guard is the previous registrar's; the victim is the next `register` on
/// that drawer.
#[test]
fn register_completes_after_a_drawer_lock_drops_across_a_forked_child() {
    let _serialized = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().expect("tempdir");
    let drawer = tmp.path().join("drawer");
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir(&drawer).expect("create the drawer directory");
    std::fs::create_dir(&workspace).expect("create the workspace directory");

    let lock = cache::DrawerLock::acquire(&drawer).expect("the drawer lock must be free to start");
    let child = ForkedFdHolder::fork();
    drop(lock);

    assert!(
        child.is_parked(),
        "the forked child died before the assert — the window was never open"
    );
    let (register_dir, register_ws) = (drawer.clone(), workspace.clone());
    let registered = within_bound(move || cache::register(&register_dir, &register_ws));
    child.release_and_reap();

    let sentinel = match registered {
        None => panic!(
            "cache::register did not return within {ACQUIRE_BOUND:?} — it is blocked on the \
             drawer lock a dropped guard should have released. In the registry daemon this is a \
             connection thread hung forever on a drawer whose previous registrar has finished"
        ),
        Some(result) => result.expect("register must succeed once the lock is genuinely free"),
    };
    assert_eq!(
        sentinel.workspace,
        workspace.to_string_lossy(),
        "register returned a sentinel for the wrong workspace — it completed, but not by doing \
         its job"
    );
    assert!(
        drawer.join("registered.json").exists(),
        "register returned without writing the sentinel — an exit is not evidence that it did \
         anything (R40)"
    );
}

/// Path 1: the registry daemon's process-lifetime singleton releases at
/// shutdown, so a successor daemon can start. The child forked here is the
/// `git` a connection thread forks inside `splice`; the drop is the daemon
/// exiting; the second `try_acquire` is the successor's. Asserted at the
/// `cache` seam rather than by booting a daemon, in the acquire mode and
/// release shape the daemon uses.
#[test]
fn the_daemon_singleton_lock_releases_across_a_forked_child() {
    let _serialized = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().expect("tempdir");
    let registry_dir = tmp.path().join("registry");
    std::fs::create_dir(&registry_dir).expect("create the registry directory");

    let singleton = cache::DrawerLock::try_acquire(&registry_dir)
        .expect("try_acquire must not error on a fresh registry directory")
        .expect("the singleton must be free before the first daemon starts");
    // The forked `git`, still between its fork and its exec at daemon exit.
    let child = ForkedFdHolder::fork();
    drop(singleton); // the daemon exits

    assert!(
        child.is_parked(),
        "the forked child died before the assert — the window was never open"
    );
    let successor = cache::DrawerLock::try_acquire(&registry_dir)
        .expect("try_acquire must not error for the successor");
    child.release_and_reap();

    assert!(
        successor.is_some(),
        "the successor daemon was refused the singleton — it would print \"another meridian \
         registry daemon is already running\" for a daemon that has already exited. The \
         predecessor's guard dropped, but a forked child's copy of the fd is still holding the \
         open file description, so DrawerLock is releasing by fd close instead of by an \
         explicit LOCK_UN"
    );
}

/// The other half, unchanged by the fix: a genuinely held lock still excludes
/// through both acquire modes. A release that fired too eagerly would pass
/// every test above and let the reaper race a live workspace — the hazard m3
/// exists to prevent.
#[test]
fn a_genuinely_held_drawer_lock_still_excludes_both_acquire_modes() {
    let _serialized = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().expect("tempdir");
    let drawer = tmp.path().join("drawer");
    std::fs::create_dir(&drawer).expect("create the drawer directory");

    let held = cache::DrawerLock::acquire(&drawer).expect("first acquire");
    assert!(
        cache::DrawerLock::try_acquire(&drawer)
            .expect("try_acquire must not error")
            .is_none(),
        "a held exclusive lock must refuse a non-blocking try — the fix must not release early"
    );
    let waiting_dir = drawer.clone();
    assert!(
        within_bound(move || cache::DrawerLock::acquire(&waiting_dir)).is_none(),
        "a blocking acquire completed while the lock was genuinely held — the fix released the \
         lock early and the drawer is no longer serialized against the reaper"
    );
    drop(held);

    // The acceptance: once the holder is really gone, both modes proceed.
    let free_dir = drawer.clone();
    within_bound(move || cache::DrawerLock::acquire(&free_dir))
        .expect("the blocking acquire must complete once the holder drops")
        .expect("and it must succeed");
    assert!(
        cache::DrawerLock::try_acquire(&drawer)
            .expect("try_acquire must not error")
            .is_some(),
        "the lock must be re-acquirable once its holder drops"
    );
}
