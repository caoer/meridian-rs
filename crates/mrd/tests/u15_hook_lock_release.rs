//! The fence's install lock releases when its guard drops — even while this
//! process has a forked child holding a copy of the lock fd (R19, U15).
//!
//! # Why this path in particular, and why the fence is where R19 lands first
//! `mrd hook install` **spawns subprocesses inside its critical section by
//! definition**: the lock is taken as soon as the common dir is known, and the
//! submodule query, the `core.hooksPath` query and the top-level query are three
//! more `git` processes that all run while it is held. A `flock` lock belongs to
//! the open file DESCRIPTION and `fork` duplicates every descriptor, so each of
//! those spawns transiently holds a copy of this lock's fd between its fork and
//! its exec — `FD_CLOEXEC` notwithstanding, because CLOEXEC acts at exec. If
//! [`mrd::hook::HookLock`] released by closing its fd, dropping the guard inside
//! that window would NOT release the lock, and the next installer would refuse
//! for a critical section that had already finished.
//!
//! # This is the FOURTH instantiation of the anti-vacuity harness
//! `crates/fs/tests/write_lock_release.rs` (first), `crates/run` (second),
//! `crates/cache/tests/drawer_lock_release.rs` (third), this file (fourth). The
//! mechanism is copied deliberately rather than shared: each one has to prove the
//! window is open **in its own test binary**, because a control that lives
//! somewhere else proves nothing about the process this lock is dropped in.
//!
//! # Why this file forks by hand (the lesson the first version of it taught)
//! Driving the window with `Command::spawn` does not open it: `spawn` returns
//! only after the child has exec'd, and exec closes the `O_CLOEXEC` lock fd, so a
//! sequential acquire → spawn → drop → reacquire never has a live fd copy at the
//! moment of the drop. The window it aims at is already shut. So the mechanism
//! here is a raw `fork(2)` whose child parks on `pause()` and **never execs** —
//! its inherited copy of the lock fd stays open for exactly as long as this test
//! wants it to, with no timing and no sleep.

use std::io;
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::process::Command;

use mrd::hook::HookLock;

/// Only one forked holder may exist in this process at a time. A holder parks on
/// EVERY fd this process had open at its fork, so two overlapping holders would
/// each pin the other's lock fd and both tests would read a lock as held by the
/// wrong child.
static ONE_HOLDER_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A child forked while a lock is held, parked so that it OUTLIVES the parent's
/// release while still holding its inherited copy of the lock fd.
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
/// [`HookLock`] does. `None` is the busy refusal.
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
/// regression tests below cannot go quietly vacuous.
///
/// Release by fd close, the pre-R19 mechanism, against a raw `flock` that `mrd`
/// does not own: while the forked child is parked, closing our fd must leave the
/// lock HELD, and reaping the child must free it. If this test ever passes
/// trivially — the second acquire succeeding while the child is parked — the fork
/// window is not open in this test binary and every conclusion the tests below
/// draw is worthless.
#[test]
fn the_fork_window_is_real_a_forked_child_holds_the_lock_past_our_close() {
    let _serialized = ONE_HOLDER_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("control.lock");

    let held = raw_flock(&path).expect("the control lock must be free to start");
    let child = ForkedFdHolder::fork();
    drop(held); // fd close = the pre-R19 release mechanism, verbatim

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
         the fork/exec hazard does not reproduce here, so this file's regression tests prove \
         nothing"
    );
    assert!(
        raw_flock(&path).is_some(),
        "the lock stayed held after the child was reaped — something other than the child's fd \
         copy is holding it, so the control does not isolate what it claims to"
    );
}

/// The regression proper: the guard's drop releases the lock even though a forked
/// child is holding a copy of its fd at that instant.
///
/// The assert IS the claim — `HookLock::acquire` succeeding while the window is
/// provably open. With the explicit `LOCK_UN` in `HookLock::drop` reverted to a
/// bare fd close this fails deterministically, because the control above proves
/// the child's copy keeps the description alive.
#[test]
fn dropping_the_install_lock_releases_it_across_a_forked_child() {
    let _serialized = ONE_HOLDER_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().expect("tempdir");

    let lock = HookLock::acquire(dir.path()).expect("the lock must be free to start");
    // Forked WHILE the lock is held: the child takes no lock of its own, it only
    // has to still be holding the inherited fd when the guard drops below. This
    // is `mrd hook install`'s own shape — it spawns git three times inside this
    // critical section.
    let child = ForkedFdHolder::fork();
    drop(lock);

    assert!(
        child.is_parked(),
        "the forked child died before the assert — the window was never open"
    );
    let reacquired = HookLock::acquire(dir.path());
    child.release_and_reap();

    reacquired.unwrap_or_else(|e| {
        panic!(
            "the install lock was NOT released by its guard's drop ({e}) — a forked child's copy \
             of the fd is holding the open file description, so HookLock is releasing by fd close \
             instead of by an explicit LOCK_UN"
        )
    });
}

/// The claim about the REAL PATH: `hook::install` runs its git queries and its
/// write inside the lock, and the lock is free once it returns.
///
/// # MEASURED INSENSITIVE to the R19 defect, and that is stated rather than
/// # implied
/// With the explicit `LOCK_UN` reverted to a bare fd close, **this test still
/// passes** — measured, `/tmp/u15-redden-r19.log`. `install` spawns git through
/// `Command::output`, which returns only after the child has exec'd, and exec
/// closes the `O_CLOEXEC` lock fd; so by the time the guard drops there is no
/// live fd copy and the fd close happens to suffice. That is the same shape the
/// `fs` harness header records about `Command::spawn`, arriving here.
///
/// **The assertion that carries the R19 claim is
/// `dropping_the_install_lock_releases_it_across_a_forked_child`**, which fails
/// deterministically under the same revert. This test carries a different and
/// real claim — that install writes an executable hook and holds no lock past its
/// return — and it is kept for that, not offered as evidence of the release
/// mechanism. A gate that could not have failed is not evidence that the fix
/// worked, so it says which one it is.
#[test]
fn install_spawns_git_inside_the_lock_and_still_leaves_it_free() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().canonicalize().expect("canonical tempdir");
    init_repo(&root);

    let (fenceable, _state) =
        mrd::hook::install(&root, &mrd::hook::Force::No).expect("a clean git repo is fenceable");

    assert_eq!(
        fenceable.hook_paths.len(),
        mrd::hook::FENCED_HOOKS.len(),
        "the install set is a set: one path per door git dispatches for a commit"
    );
    for path in &fenceable.hook_paths {
        assert!(
            path.exists(),
            "install reported success without writing {}",
            path.display()
        );
        let mode = std::os::unix::fs::PermissionsExt::mode(
            &std::fs::metadata(path)
                .expect("stat the hook")
                .permissions(),
        );
        assert_eq!(
            mode & 0o111,
            0o111,
            "a hook git cannot execute is a hook git skips: {} mode {mode:o}",
            path.display()
        );
    }

    HookLock::acquire(&fenceable.common_dir).unwrap_or_else(|e| {
        panic!(
            "the install lock is still held after `install` returned ({e}) — install spawns git \
             inside its critical section, so a lock released by fd close leaks into one of those \
             children and the next installer refuses for a section that already finished"
        )
    });
}

/// The other half, unchanged by the fix: a lock that is genuinely HELD refuses
/// the second acquire immediately (`WouldBlock`, never a wait) — in-process
/// contention included, since `flock` contends per open file description.
#[test]
fn a_held_install_lock_still_refuses_a_second_acquire_without_blocking() {
    let dir = tempfile::tempdir().expect("tempdir");
    let held = HookLock::acquire(dir.path()).expect("first acquire");
    let err = HookLock::acquire(dir.path()).expect_err("a held lock refuses");
    assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
    drop(held);
    HookLock::acquire(dir.path()).expect("free again once the holder drops");
}

/// A real `git init` — the fence is only ever asserted against real git.
fn init_repo(root: &Path) {
    for args in [
        vec!["init", "--quiet"],
        vec!["config", "user.email", "fence@example.invalid"],
        vec!["config", "user.name", "fence"],
    ] {
        let out = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(&args)
            .output()
            .expect("run git");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
