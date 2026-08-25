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
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// How long a probe that must **not** complete is watched before this harness
/// accepts that the lock really is still held.
///
/// A timeout here is the *expected* outcome, so a plain constant is the right
/// shape: waiting longer makes the suite slower without making the assertion
/// truer. The opposite polarity — a probe that must complete — is
/// [`within_floor`], and it must never share a constant with this one.
const HELD_BOUND: Duration = Duration::from_secs(10);

/// The nominal bound for a probe that **must** complete — and only the FLOOR
/// under it, never the deadline itself.
///
/// A wall-clock bound answers "is the lock still held?" only for someone who
/// already knows how fast this box is right now. On a CI agent nobody does,
/// and the two questions are not separable by waiting: the operation this
/// harness times is not the bare `flock(2)` this comment used to claim — at
/// the `cache::register` seam it is a directory create, a sentinel read, a
/// tmp write with an `fsync`, a `hard_link` and a directory `fsync`, all
/// inside the timed window, on whatever filesystem the agent gave us.
///
/// So the bound is a floor, not a deadline: it starts here and is raised only
/// by a measured fact about this box at this moment. See [`within_floor`].
const FREE_FLOOR: Duration = Duration::from_secs(10);

/// How many times the *measured* cost of the same operation on a free drawer
/// the bound may become, once [`FREE_FLOOR`] has expired without a verdict.
const FREE_SLACK: u32 = 32;

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

/// A blocking acquire run on a helper thread under a bound.
///
/// A timed-out probe is still parked inside `flock(2)`: it acquires the lock
/// the instant the lock frees, and releases it again only when its thread ends.
/// That thread is a second contender for the rest of the test, so the caller
/// must [`settle`](Self::settle) it — never merely drop it.
struct BoundedProbe<T> {
    /// What the acquire returned inside the bound; `None` is the timeout.
    outcome: Option<T>,
    helper: thread::JoinHandle<()>,
    /// The bound this probe actually enforced — [`HELD_BOUND`] for a probe
    /// that must not complete, or the floored deadline for one that must.
    bound: Duration,
    /// What an identical operation cost on a free drawer, measured only if
    /// [`FREE_FLOOR`] expired without a verdict. `None` = never needed.
    reference: Option<Duration>,
}

impl<T> BoundedProbe<T> {
    /// Did the acquire fail to finish inside the bound?
    ///
    /// For a probe raised through [`within_floor`] this is the defect: the
    /// bound it outlived was already floored to this box's own measured speed,
    /// so slowness has been excluded by measurement rather than by assumption.
    /// For a probe on the fixed [`HELD_BOUND`] it is the asserted, expected
    /// outcome — the lock is genuinely held and must stay that way.
    ///
    /// A bare wall-clock timeout, on its own, is neither: it is the reading
    /// that made two seats spend a shift proving a red was not real.
    fn timed_out(&self) -> bool {
        self.outcome.is_none()
    }

    /// How the enforced bound was arrived at, so a failure message tells the
    /// next reader which of the two hypotheses the numbers actually rule out.
    fn bound_report(&self) -> String {
        match self.reference {
            None => format!("{:?}", self.bound),
            Some(reference) => format!(
                "{:?} — FLOORED: the nominal bound expired, so an identical operation was timed \
                 on a free drawer on this box at that moment and cost {reference:?}; the bound \
                 became x{FREE_SLACK} of that. Slowness is therefore excluded by measurement, \
                 not by assumption",
                self.bound
            ),
        }
    }

    /// Join the helper thread and take the outcome. Once the lock is free the
    /// helper acquires it, drops what it acquired (its receiver is gone) and
    /// exits — so joining here makes that release *happen-before* whatever the
    /// caller probes next, instead of racing it.
    ///
    /// Call it only after the lock under test has been freed; a helper still
    /// waiting on a genuinely held lock never returns.
    fn settle(self) -> Option<T> {
        self.helper
            .join()
            .expect("the bounded-acquire helper thread panicked");
        self.outcome
    }
}

/// Run a blocking acquire on a helper thread and wait at most `bound`.
///
/// For probes whose timeout is the asserted outcome — a lock that is genuinely
/// held must not be acquirable. A probe that must *complete* takes
/// [`within_floor`] instead.
fn within_bound<T: Send + 'static>(
    bound: Duration,
    acquire: impl FnOnce() -> T + Send + 'static,
) -> BoundedProbe<T> {
    let (tx, rx) = mpsc::channel();
    let helper = thread::spawn(move || {
        let _ = tx.send(acquire());
    });
    BoundedProbe {
        outcome: rx.recv_timeout(bound).ok(),
        helper,
        bound,
        reference: None,
    }
}

/// Run a blocking acquire on a helper thread under a FLOOR: first the nominal
/// [`FREE_FLOOR`], and if that expires, a bound re-derived from what the same
/// operation costs on a free drawer, on this box, at this moment.
///
/// **Why a floor and not a bigger constant.** A bigger constant moves the
/// cliff; it does not remove it, because the quantity it is racing — how long
/// this agent takes to do ordinary work — has no upper bound anyone can write
/// down. The floor removes the cliff by making the bound a function of a
/// measurement instead of a guess. `reference` is run only on the slow path,
/// so a healthy box pays nothing and behaves exactly as before.
///
/// **What still makes a genuinely stuck system fail.** `reference` must be the
/// same operation on a *different* drawer directory — one with no forked child
/// holding a copy of any fd on it. The defect under test is a `flock` leaked
/// into a fork window, and a `flock` is scoped to the open file description of
/// *its own* drawer directory's inode, so the defect cannot inflate the
/// reference. The reference therefore always completes, the derived bound is
/// therefore always finite, and a lock that is never released exceeds every
/// finite bound. A hang is still a failure; only slowness stopped being one.
fn within_floor<T: Send + 'static>(
    acquire: impl FnOnce() -> T + Send + 'static,
    reference: impl FnOnce(),
) -> BoundedProbe<T> {
    within_floor_from(FREE_FLOOR, acquire, reference)
}

/// [`within_floor`] with the nominal floor named explicitly, so the regression
/// test can drive the slow path in milliseconds instead of tens of seconds.
fn within_floor_from<T: Send + 'static>(
    nominal: Duration,
    acquire: impl FnOnce() -> T + Send + 'static,
    reference: impl FnOnce(),
) -> BoundedProbe<T> {
    let (tx, rx) = mpsc::channel();
    let helper = thread::spawn(move || {
        let _ = tx.send(acquire());
    });

    // Fast path: on a box that is not in trouble this is the whole story, and
    // the reference below is never measured at all.
    if let Ok(value) = rx.recv_timeout(nominal) {
        return BoundedProbe {
            outcome: Some(value),
            helper,
            bound: nominal,
            reference: None,
        };
    }

    // The nominal floor expired. That is the defect's signature only on a box
    // already known to be healthy — so measure this one rather than assume it.
    let measured = {
        let start = Instant::now();
        reference();
        start.elapsed()
    };
    let bound = nominal.max(measured.saturating_mul(FREE_SLACK));
    let outcome = rx.recv_timeout(bound.saturating_sub(nominal)).ok();
    BoundedProbe {
        outcome,
        helper,
        bound,
        reference: Some(measured),
    }
}

/// A fresh, definitely-free drawer directory under `parent` — the subject of a
/// [`within_floor`] reference measurement, never of the probe itself.
///
/// Call it **after** any [`ForkedFdHolder::fork`] in the test: a directory that
/// did not exist at the fork cannot be held by the child's inherited fds, which
/// is what keeps the reference measurable while the fork window is open.
fn scratch_drawer(parent: &Path, name: &str) -> PathBuf {
    let dir = parent.join(name);
    std::fs::create_dir_all(&dir).expect("create the scratch drawer for the reference measurement");
    dir
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
/// acquire modes. Also validates the probe helpers in both directions:
/// [`within_bound`] must expire on a held lock, [`within_floor`] must not on a
/// free one.
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
    let hung_probe = within_bound(HELD_BOUND, move || raw_flock_blocking(&probe_dir));
    let hung_while_parked = hung_probe.timed_out();

    child.release_and_reap();
    // The parked probe wins the freed lock; settle it so its release is done
    // before the acceptance probes below, which would otherwise race it.
    drop(hung_probe.settle());

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
    let reference_dir = scratch_drawer(tmp.path(), "control-reference");
    assert!(
        within_floor(
            move || raw_flock_blocking(&free_dir),
            || drop(raw_flock_blocking(&reference_dir)),
        )
        .settle()
        .is_some(),
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
    let reference_dir = scratch_drawer(tmp.path(), "reacquire-reference");
    let probe = within_floor(
        move || cache::DrawerLock::acquire(&reacquire_dir),
        || drop(cache::DrawerLock::acquire(&reference_dir)),
    );
    let bound = probe.bound_report();
    child.release_and_reap();
    let reacquired = probe.settle();

    match reacquired {
        None => panic!(
            "DrawerLock::acquire did not return within {bound} — the lock was NOT \
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
    // The reference is a whole `register` — not a bare acquire — because the
    // probe times a whole `register`: a create, a read, a tmp write with an
    // fsync, a hard_link and a directory fsync all sit inside the bound, and
    // any of them can be what a loaded agent is slow at.
    let reference_drawer = scratch_drawer(tmp.path(), "register-reference");
    let reference_ws = scratch_drawer(tmp.path(), "register-reference-ws");
    let probe = within_floor(
        move || cache::register(&register_dir, &register_ws),
        || {
            cache::register(&reference_drawer, &reference_ws)
                .expect("the reference register must succeed on a free scratch drawer");
        },
    );
    let bound = probe.bound_report();
    child.release_and_reap();
    let registered = probe.settle();

    let sentinel = match registered {
        None => panic!(
            "cache::register did not return within {bound} — it is blocked on the \
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
    let waiting = within_bound(HELD_BOUND, move || cache::DrawerLock::acquire(&waiting_dir));
    assert!(
        waiting.timed_out(),
        "a blocking acquire completed while the lock was genuinely held — the fix released the \
         lock early and the drawer is no longer serialized against the reaper"
    );
    drop(held);
    // `waiting`'s helper is still queued on the flock: it takes the lock the
    // moment the drop above frees it, and gives it back only when its thread
    // ends. Settle it here so the acceptance probes below contend with nothing
    // but each other.
    drop(waiting.settle());

    // The acceptance: once the holder is really gone, both modes proceed.
    let free_dir = drawer.clone();
    let reference_dir = scratch_drawer(tmp.path(), "held-reference");
    within_floor(
        move || cache::DrawerLock::acquire(&free_dir),
        || drop(cache::DrawerLock::acquire(&reference_dir)),
    )
    .settle()
    .expect("the blocking acquire must complete once the holder drops")
    .expect("and it must succeed");
    assert!(
        cache::DrawerLock::try_acquire(&drawer)
            .expect("try_acquire must not error")
            .is_some(),
        "the lock must be re-acquirable once its holder drops"
    );
}

/// **The bound is a FLOOR, not a deadline** — the whole mechanism of the
/// same-sha green/red split at `register_completes_…`, reduced to something
/// that needs no CI agent, no load, no forked child and no sleep longer than a
/// blink: an operation that is merely SLOW must stop reading as a held lock.
///
/// The first assert is the red this file kept producing. `cache::register` is
/// not the bare `flock(2)` the old bound was sized for — a directory create, a
/// sentinel read, a tmp write with an `fsync`, a `hard_link` and a directory
/// `fsync` all sit inside the timed window — so on a loaded agent the whole
/// probe can outlive a fixed bound while the lock is provably free. A fixed
/// bound cannot tell that apart from the defect; the floor can, because it
/// asks the box how slow it is instead of guessing.
#[test]
fn a_slow_operation_is_not_a_held_lock() {
    // Serialized like every other test here: this one takes no lock and opens
    // no fd, but it does assert on elapsed time, and a sibling's forked child
    // is exactly the neighbour that would perturb it.
    let _serialized = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let nominal = Duration::from_millis(20);
    let slow = Duration::from_millis(120);
    // How slow this box is right now, as the floor would measure it.
    let reference = Duration::from_millis(10);

    // Without the floor: the fixed bound expires and the operation reads as a
    // hang. This is instance 7 and instance 8, in miniature.
    let fixed = within_bound(nominal, move || thread::sleep(slow));
    assert!(
        fixed.timed_out(),
        "a fixed bound shorter than the operation must expire — otherwise this test is not \
         reproducing the defect it exists to pin"
    );
    let _ = fixed.settle();

    // With the floor: the same operation, the same nominal bound, and the
    // verdict flips — because the bound was re-derived from a measurement.
    let floored = within_floor_from(
        nominal,
        move || thread::sleep(slow),
        move || thread::sleep(reference),
    );
    assert!(
        !floored.timed_out(),
        "a slow-but-progressing operation was reported as a held lock even under the floor — \
         the floor is not raising the bound from the measured reference, and the same-sha \
         split-pair family is not fixed"
    );
    let _ = floored.settle();
}

/// The other half, and the reason the floor is safe to install: a lock that is
/// **genuinely never released** still fails, floor or no floor.
///
/// The guarantee is structural, not a matter of choosing a big enough number.
/// The reference is timed on a *different* drawer directory, and a `flock` is
/// scoped to the open file description of its own directory's inode — so the
/// defect under test can never inflate the reference. The reference therefore
/// always completes, the derived bound is therefore always finite, and a wait
/// that never ends outlives every finite bound.
#[test]
fn a_genuinely_held_lock_still_fails_under_the_floor() {
    let _serialized = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().expect("tempdir");
    let drawer = tmp.path().join("drawer");
    std::fs::create_dir(&drawer).expect("create the drawer directory");
    let reference_dir = scratch_drawer(tmp.path(), "stuck-reference");

    // Held for the whole probe, and never released until after it.
    let held = cache::DrawerLock::acquire(&drawer).expect("first acquire");

    let waiting_dir = drawer.clone();
    // A reference slow enough that the floor demonstrably RAISES the bound —
    // the point being that the probe still expires anyway.
    let probe = within_floor_from(
        Duration::from_millis(20),
        move || cache::DrawerLock::acquire(&waiting_dir),
        move || {
            thread::sleep(Duration::from_millis(5));
            drop(cache::DrawerLock::acquire(&reference_dir));
        },
    );
    let timed_out = probe.timed_out();
    let bound = probe.bound_report();
    assert!(
        bound.contains("FLOORED"),
        "the floor did not engage, so this test is not exercising the guarantee it claims — \
         bound report was: {bound}"
    );
    assert!(
        timed_out,
        "a genuinely held lock completed under the floored bound — the floor has stopped \
         failing on the defect it exists to keep failing on, and a real lost wakeup in the \
         release path would now ship silently. Bound was: {bound}"
    );

    // Release, then settle the parked helper so it is not left contending.
    drop(held);
    drop(probe.settle());
}
