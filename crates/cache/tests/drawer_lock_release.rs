//! The m3 drawer flock releases when its guard drops — even while this process has a
//! forked child holding a copy of the lock fd.
//!
//! # The bug this pins (R19's third instance, and the worst of the three)
//! A `flock` lock belongs to the open file DESCRIPTION, and `fork` duplicates every
//! descriptor; `FD_CLOEXEC` acts at exec, not at fork. So any thread spawning any
//! subprocess transiently holds a copy of every open fd between its fork and its exec.
//! `DrawerLock` had **no `Drop` impl at all** — it released by letting its `File` field
//! close — so a guard dropped inside that window did NOT release the lock: the child's
//! copy kept the description alive.
//!
//! `fs::WriteLock` and `run::executor::WorkspaceLock` carried the identical defect and
//! were fixed first (measured on the fs one: 12 of 60 unrelated writes refused). This
//! lock is the same defect with a **worse failure mode**, because it is the only BLOCKING
//! acquire in the codebase: `DrawerLock::acquire` takes `LOCK_EX` without `LOCK_NB`, so a
//! leaked description does not degrade to the fast typed `workspace_busy` refusal its two
//! fixed siblings give — it degrades to a **HANG**.
//!
//! # The two live paths, both inside the registry daemon
//! 1. **The process-lifetime singleton.** `registry::server` takes a `DrawerLock` on the
//!    registry directory for the daemon's whole lifetime (`server.rs:159`, `_singleton`)
//!    while its connection threads fork `git` inside `splice` (`server.rs:843` →
//!    `wire_serve::write::splice` → `write.rs:1674` `git::Repo::at` → `git/src/lib.rs:312`
//!    `Command::new`). The leak bites at **shutdown**: a successor daemon's `try_acquire`
//!    returns `None` and it refuses *"another meridian registry daemon is already
//!    running"* (`server.rs:160`) for a daemon that has already exited.
//! 2. **Per-drawer locks** taken by `cache::register` (`sentinel.rs:150`) from a
//!    connection thread. Here the leak is the hang: the next `register` on that drawer
//!    waits forever on a holder that is gone.
//!
//! Both paths are asserted below, each through the acquire mode its production site uses
//! — non-blocking `try_acquire` for the singleton, blocking `acquire` for the drawer.
//!
//! # Why this file forks by hand
//! `Command::spawn` returns only after the child has exec'd, and exec closes the
//! `O_CLOEXEC` lock fd, so a spawn-driven test has no live fd copy at the moment of the
//! drop and passes with the defect fully restored (measured on the fs sibling: 400/400
//! with the explicit unlock reverted, two reviewers independently). The child here parks
//! on `pause()` and never execs, so the window is held open on demand rather than raced
//! for — and the control test below asserts that it really is open, so this file cannot
//! go quietly vacuous.
//!
//!
//!

use std::io;
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// How long a blocking acquire may take before this harness calls it hung.
///
/// **Why a bound exists at all:** the defect's signature on the blocking path is
/// a wait that never ends. A test that simply calls `DrawerLock::acquire` and
/// waits for the truth would, on the un-fixed tree, hang the whole suite instead
/// of reporting a failure. So the wait is bounded and the bound IS the assert.
///
/// **How the number was chosen:** the operation under test is one `flock(2)` on
/// an fd that was just opened. When the lock is free it returns in microseconds
/// — a single uncontended syscall, no I/O and no wait — so ten seconds is about
/// six orders of magnitude of headroom. A timeout here cannot be scheduler noise
/// or build contention on a loaded host; it is the lock genuinely still held.
/// Shorter would trade that certainty for nothing (the green path never spends
/// it); longer would only make a red run slower to report. The bound also caps
/// this file's worst case: a regression costs seconds, never forever.
const ACQUIRE_BOUND: Duration = Duration::from_secs(10);

/// Every test in this file is serialized, not just the forking ones.
///
/// A forked holder parks on EVERY fd this process had open at its fork, so an
/// overlapping test's *held lock* — or its parked blocking waiter — would be
/// pinned by a sibling test's child, and the tests would read locks as held by
/// the wrong child. (Measured on the fs sibling: its first draft used a release
/// pipe and the sibling test's holder kept the pipe's write end open — the very
/// hazard under test, arriving through the harness.) Other test binaries are
/// separate processes and share no descriptors, so this lock is enough.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

/// Run a blocking acquire on a helper thread and wait at most [`ACQUIRE_BOUND`].
///
/// `None` means it did not finish inside the bound. On a blocking `LOCK_EX` that
/// is not an inconclusive result — it IS the observable defect: the lock is
/// still held and the caller is hanging.
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

/// `DrawerLock` locks the drawer DIRECTORY fd, so the control must lock a
/// directory too — otherwise it would be proving the hazard on a different kind
/// of inode from the one production uses.
fn open_for_lock(path: &Path) -> std::fs::File {
    std::fs::File::open(path).expect("open the directory to lock")
}

/// THE CONTROL — it proves the fork window is open, and it is the reason every
/// regression test below cannot go quietly vacuous.
///
/// Release by fd close, the pre-fix mechanism verbatim, against a raw `flock`
/// that `cache` does not own: while the forked child is parked, closing our fd
/// must leave the lock HELD. It is asserted through BOTH probes, because the two
/// production paths use different acquire modes — the drawer path blocks
/// (`LOCK_EX`) and the daemon singleton does not (`LOCK_EX | LOCK_NB`).
///
/// It also validates [`within_bound`] itself, in both directions: the bounded
/// probe must report a timeout while the lock is held, and must report success
/// once the child is reaped. A helper that could only ever time out, or only
/// ever succeed, would make every assert below meaningless.
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

    // Probe 2, the drawer's mode: a blocking acquire must NOT complete. This is
    // the hang, observed as a bounded wait that expires.
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

    // And the acceptance, in the same breath (S3-R8(c)): with the child reaped
    // the lock must be free, through both probes. This is what proves the
    // child's fd copy was the SOLE holder, and that a bounded probe can report
    // success at all.
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

/// PATH 2, at the seam: the guard's drop releases the drawer lock even though a
/// forked child is holding a copy of its fd at that instant.
///
/// The assert IS the claim — `DrawerLock::acquire` completing while the window
/// is provably open. Without the explicit `LOCK_UN` this does not fail fast: it
/// HANGS, because the control above proves the child's copy keeps the
/// description alive and this acquire has no `LOCK_NB`. The bound is what turns
/// that hang into a report.
#[test]
fn dropping_a_drawer_lock_releases_it_across_a_forked_child() {
    let _serialized = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().expect("tempdir");
    let drawer = tmp.path().join("drawer");
    std::fs::create_dir(&drawer).expect("create the drawer directory");

    let lock = cache::DrawerLock::acquire(&drawer).expect("the drawer lock must be free to start");
    // Forked WHILE the lock is held: the child takes no lock of its own, it only
    // has to still be holding the inherited fd when the guard drops below. This
    // is the `git` a connection thread forks inside `splice`.
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

/// PATH 2, at the production entry point: `cache::register` is the caller whose
/// hang this defect produces, so it is asserted by name and not only by its
/// lock.
///
/// `register` takes the same blocking `DrawerLock::acquire` at `sentinel.rs:150`
/// that the test above takes directly. Here the released guard is the previous
/// registrar's and the victim is the next `register` on that drawer — the
/// connection-thread shape of live path 2, verbatim.
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

/// PATH 1: the registry daemon's process-lifetime singleton releases at
/// shutdown, so a SUCCESSOR daemon can start.
///
/// `registry::server` takes this lock with `try_acquire` on the registry
/// directory (`server.rs:159`) and holds it for the whole process lifetime while
/// connection threads fork `git` inside `splice`. The child forked here is that
/// `git`; the drop is the daemon exiting; the second `try_acquire` is the
/// successor's. `None` there is verbatim the *"another meridian registry daemon
/// is already running"* refusal at `server.rs:160` — issued for a daemon that
/// has already exited.
///
/// This asserts the seam rather than booting a daemon: `registry` depends on
/// `cache`, so the assert lives on the `cache` side of that edge, in the acquire
/// mode and release shape the daemon uses.
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
    // The `git` a connection thread forks inside `splice`, still between its
    // fork and its exec when the daemon exits.
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

/// The other half, unchanged by the fix: a lock that is genuinely HELD still
/// excludes, through both acquire modes.
///
/// A release that fires too eagerly would pass every test above and destroy the
/// guard's whole purpose — the reaper racing a live workspace is the hazard m3
/// exists to prevent. So the refusal and the acceptance are asserted in one
/// breath: held ⇒ `try_acquire` refuses AND a blocking `acquire` genuinely waits
/// past the bound; dropped ⇒ both succeed.
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
