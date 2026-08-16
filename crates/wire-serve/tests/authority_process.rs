//! The authority-lease card's process gates, against the contract at pin
//! `95f7c248101e6ff6…`:
//!
//! 1. `kill -9` the lease holder: a successor reaches ready through the
//!    FULL recovery sequence — and could not serve before the kernel
//!    released the lease (acceptance gates 1–2's shape).
//! 2. A hung-but-alive holder produces the named watchdog behavior — wedged
//!    declaration, typed `wedged{retry_after}` refusals while takeover runs,
//!    termination then `SIGKILL`, successor to ready — never an indefinite
//!    `authority_unavailable` (acceptance gate 3's shape).
//! 3. The lease's explicit unlock releases it across a concurrently forked
//!    fd copy (§0.2's mandate for BOTH authority fds; the recovery-EX half
//!    is pinned by `crates/fs/tests/write_lock_release.rs`).
//!
//! The holder child is this test binary re-exec'd against one test fn
//! (`authority_child_entry`), so the child runs the REAL state machine —
//! fork+exec, no inherited authority fds, which is also the §2.1.1
//! supervisor's own spawn rule.

use std::io;
use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use wire_serve::authority::{
    AuthorityLease, AuthorityUnavailable, HeartbeatPolicy, Watchdog, WedgedOutcome,
};

const CHILD_ROOT_ENV: &str = "MERIDIAN_AUTHORITY_CHILD_ROOT";
const CHILD_MODE_ENV: &str = "MERIDIAN_AUTHORITY_CHILD_MODE";
/// The file the child lands once it holds the state its mode promises.
const CHILD_UP_MARKER: &str = "child-up";

/// The child entry. Not a gate: without the env it is a no-op that stays
/// green; WITH the env it becomes the spawned lease holder and never
/// returns (the parent's signal ends it).
#[test]
fn authority_child_entry() {
    let Some(root) = std::env::var_os(CHILD_ROOT_ENV) else {
        return;
    };
    let root = fs::WorkspaceRoot(std::path::PathBuf::from(root));
    let mode = std::env::var(CHILD_MODE_ENV).unwrap_or_default();
    match mode.as_str() {
        "hold" => child_hold(&root),
        "beat-then-hang" => child_beat_then_hang(&root),
        other => panic!("unknown child mode {other:?}"),
    }
}

/// Acquire the lease and park on it forever — the `kill -9` target.
fn child_hold(root: &fs::WorkspaceRoot) -> ! {
    let lease = AuthorityLease::acquire(root).expect("child wins the free lease");
    std::fs::write(root.0.join(CHILD_UP_MARKER), lease.epoch().hex()).expect("up marker");
    park()
}

/// Run the full sequence to ready, beat a few times, then hang while
/// ignoring SIGTERM — the wedged-but-alive holder, forced onto the
/// `SIGKILL` leg so the escalation is what the gate proves.
fn child_beat_then_hang(root: &fs::WorkspaceRoot) -> ! {
    // Disposition is process-wide, so the whole child survives termination
    // and only `SIGKILL` ends it — a holder wedged too deep to die politely.
    // SAFETY: installing SIG_IGN for SIGTERM; no handler code runs.
    unsafe { libc::signal(libc::SIGTERM, libc::SIG_IGN) };
    let lease = AuthorityLease::acquire(root).expect("child wins the free lease");
    let takeover = lease
        .begin_takeover(Duration::from_secs(5))
        .expect("child recovery EX");
    let mut ready = takeover
        .ready(wire::Root("b3:child".into()))
        .expect("child ready");
    for _ in 0..3 {
        ready.beat().expect("child beats while healthy");
        std::thread::sleep(Duration::from_millis(40));
    }
    std::fs::write(root.0.join(CHILD_UP_MARKER), ready.epoch().hex()).expect("up marker");
    // The hang: alive, lease held, progress never advancing again.
    park()
}

fn park() -> ! {
    loop {
        std::thread::sleep(Duration::from_hours(1));
    }
}

/// A spawned holder child, killed and reaped even when the test panics —
/// a parked orphan would outlive the suite otherwise.
struct HolderChild(Child);

impl HolderChild {
    fn spawn(root: &fs::WorkspaceRoot, mode: &str) -> Self {
        let exe = std::env::current_exe().expect("test binary path");
        let child = Command::new(exe)
            .args(["authority_child_entry", "--exact", "--nocapture"])
            .env(CHILD_ROOT_ENV, &root.0)
            .env(CHILD_MODE_ENV, mode)
            .spawn()
            .expect("spawn the holder child");
        HolderChild(child)
    }

    fn pid(&self) -> i32 {
        i32::try_from(self.0.id()).expect("pid fits i32")
    }

    fn kill9_and_reap(mut self) {
        self.0.kill().expect("SIGKILL the holder");
        self.0.wait().expect("reap the holder");
        // Consumed: Drop has nothing left to do (kill/wait tolerate both).
    }

    /// Reap after something ELSE (the watchdog) killed it.
    ///
    /// The watchdog's `wait_death` waitpid's this process's child — same as
    /// a supervisor would — so the kernel may already have reaped the pid
    /// slot. `ECHILD` is that case, not a leak.
    fn reap(mut self) {
        if let Err(e) = self.0.wait() {
            assert!(
                e.raw_os_error() == Some(libc::ECHILD),
                "reap the watchdog-killed holder: {e}"
            );
        }
    }
}

impl Drop for HolderChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait_for(path: &Path, budget: Duration) {
    let deadline = Instant::now() + budget;
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "the child never landed {} — its spawn or acquire failed",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Card gate 1: `kill -9` the lease holder; the successor reaches ready
/// through the full recovery sequence — lease, recovery EX, recovery
/// window, current root, EX release, admissions — and the lease refused
/// the successor while the holder lived (no serve before kernel release).
#[test]
fn gate_kill9_the_lease_holder_a_successor_recovers_to_ready() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());

    let child = HolderChild::spawn(&root, "hold");
    wait_for(&root.0.join(CHILD_UP_MARKER), Duration::from_secs(15));

    // Before the kill: the kernel lock is the fence — the successor's
    // acquire refuses typed, and the hint names the living holder.
    match AuthorityLease::acquire(&root) {
        Err(AuthorityUnavailable::Held { hint }) => {
            let hint = hint.expect("the holder landed its routing hint");
            assert_eq!(hint.pid, child.pid(), "the hint names the holder");
        }
        other => panic!("a live holder must refuse the successor, got {other:?}"),
    }

    let holder_pid = child.pid();
    child.kill9_and_reap();

    // The kernel janitor released the lease at death (§0.2): the successor
    // wins WITHOUT any owner-record scavenging — the stale record is still
    // on disk and proves nothing.
    let stale = AuthorityLease::owner_hint(&root).expect("the dead holder's record survives");
    assert_eq!(
        stale.pid, holder_pid,
        "the stale record still names the corpse"
    );
    let lease = AuthorityLease::acquire(&root).expect("kernel-released lease");

    // The full sequence: recovery EX held through the window…
    let takeover = lease
        .begin_takeover(Duration::from_secs(5))
        .expect("recovery EX");
    assert_eq!(
        fs::WriteLock::acquire(&root)
            .expect_err("recovery EX excludes cooperating writers during recovery")
            .kind(),
        io::ErrorKind::WouldBlock
    );
    // …the recovery WORK happens here (durable intents are the
    // parallel-commits half; the order slot is what this machine owns)…
    let _fence = takeover.recovery_ex();

    // …then ready: EX released BEFORE admissions, and publication takes no
    // flock — write.lock is acquirable by a bystander while the successor
    // mints permits (the LOCK_EX demotion, observed).
    let mut ready = takeover
        .ready(wire::Root("b3:successor".into()))
        .expect("ready");
    let bystander = fs::WriteLock::acquire(&root)
        .expect("recovery EX explicitly released at ready (§2.3 step 7)");
    let permit = ready.permit();
    assert_eq!(
        permit.epoch(),
        ready.epoch(),
        "publication binds to the live epoch"
    );
    ready.beat().expect("the ready authority advances progress");
    drop(bystander);

    // The successor's record replaced the corpse's.
    let now = AuthorityLease::owner_hint(&root).expect("fresh record");
    assert_eq!(now.epoch_hex, ready.epoch().hex());
}

/// Card gate 2: a hung-but-alive holder produces the named watchdog
/// behavior — declaration after the miss budget, `wedged{retry_after}`
/// refusals while takeover is in progress, termination then `SIGKILL`,
/// kernel release, successor to ready — never an indefinite
/// `authority_unavailable`.
#[test]
fn gate_a_wedged_holder_is_terminated_and_a_successor_reaches_ready() {
    let started = Instant::now();
    let dir = tempfile::tempdir().expect("tempdir");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());

    let child = HolderChild::spawn(&root, "beat-then-hang");
    wait_for(&root.0.join(CHILD_UP_MARKER), Duration::from_secs(15));

    // Test-scaled policy constants — the contract states 1 s / 30 / 5 s as
    // proposed policy, injectable by design.
    let policy = HeartbeatPolicy {
        beat: Duration::from_millis(50),
        misses_to_wedged: 6,
        term_grace: Duration::from_millis(600),
    };
    let mut dog = Watchdog::new(&root, child.pid(), policy);

    // The supervisor declares the hang after the miss budget and captures
    // the diagnostic.
    let report = dog
        .watch_until_wedged()
        .expect("the declaration marker lands")
        .expect("the holder is alive, so the budget — not death — ends the watch");
    assert_eq!(report.pid, child.pid());
    assert!(report.missed >= policy.misses_to_wedged);
    assert!(report.diagnostic.contains("no heartbeat progress"));

    // While the wedged holder still holds the lease, refusals are TYPED
    // wedged with the retry budget — the client teaching of §2.1.1.
    match AuthorityLease::acquire(&root) {
        Err(AuthorityUnavailable::Wedged {
            retry_after,
            report: seen,
        }) => {
            assert_eq!(retry_after, policy.term_grace);
            assert_eq!(seen, report, "the refusal serves the standing declaration");
        }
        other => panic!("takeover in progress must refuse wedged, got {other:?}"),
    }

    // Enforcement: the holder ignores termination (wedged too deep), so the
    // grace budget elapses and the `SIGKILL` leg fires.
    match dog.enforce(report) {
        WedgedOutcome::RequiredKill(_) => {}
        other => panic!("a TERM-ignoring holder must reach the SIGKILL leg, got {other:?}"),
    }
    child.reap();

    // Only kernel-confirmed release lets the successor in; it recovers to
    // ready and the episode closes — the marker is gone, refusals are Held.
    let lease = AuthorityLease::acquire(&root).expect("kernel released at SIGKILL");
    let takeover = lease
        .begin_takeover(Duration::from_secs(5))
        .expect("successor recovery EX");
    let ready = takeover
        .ready(wire::Root("b3:after-wedge".into()))
        .expect("successor ready");
    match AuthorityLease::acquire(&root) {
        Err(AuthorityUnavailable::Held { .. }) => {}
        other => panic!("the wedged episode is over — plain Held, got {other:?}"),
    }
    drop(ready);

    // "Never indefinite": the whole episode — declaration, enforcement,
    // recovery — completed inside one bounded budget.
    assert!(
        started.elapsed() < Duration::from_mins(1),
        "the posture must bound the outage, took {:?}",
        started.elapsed()
    );
}

/// §0.2's explicit-unlock mandate, lease half: dropping the lease releases
/// it even while a forked child holds a copy of the fd (`FD_CLOEXEC` acts
/// at exec, not fork). Same shape as `fs/tests/write_lock_release.rs`,
/// which pins the recovery-EX half.
#[test]
fn the_lease_unlock_is_explicit_and_survives_a_forked_fd_copy() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());

    let lease = AuthorityLease::acquire(&root).expect("acquire");
    // SAFETY: fork(2); the child branch calls only async-signal-safe
    // functions (alarm/pause) and never returns into Rust.
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0, "fork: {}", io::Error::last_os_error());
    if pid == 0 {
        // SAFETY: the child: async-signal-safe calls only, never returns —
        // the parent's SIGKILL ends it; the alarm is the orphan backstop.
        unsafe {
            libc::alarm(60);
            loop {
                libc::pause();
            }
        }
    }
    drop(lease);

    let mut status: libc::c_int = 0;
    // SAFETY: non-blocking waitpid on our own child into a local.
    let parked = unsafe { libc::waitpid(pid, &raw mut status, libc::WNOHANG) } == 0;
    assert!(
        parked,
        "the forked child died before the reacquire — the window never opened"
    );

    let reacquired = AuthorityLease::acquire(&root);
    // SAFETY: signalling then reaping our own child by the pid fork returned.
    unsafe {
        libc::kill(pid, libc::SIGKILL);
        libc::waitpid(pid, &raw mut status, 0);
    }
    reacquired.map_or_else(
        |e| {
            panic!(
                "the lease was NOT released by its guard's drop ({e}) — a forked fd copy is \
                 holding the description, so the lease is releasing by fd close instead of \
                 an explicit LOCK_UN"
            )
        },
        drop,
    );
}

/// Belt for the fork test's premise: the flock rides the authority
/// DIRECTORY's stable inode, so record churn beside it (owner hints land by
/// tmp+rename) can never blink the exclusion — a lock on a renamed-over
/// file would exclude nobody (the cache drawer-lock lesson, §2.1).
#[test]
fn the_lock_inode_is_the_directory_not_a_swappable_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    let lease = AuthorityLease::acquire(&root).expect("acquire");

    for i in 0..3 {
        std::fs::write(
            root.0
                .join(wire_serve::authority::AUTHORITY_DIR)
                .join("owner.json"),
            format!("{{\"pid\": {i}, \"epoch\": \"0\", \"workspace\": \"w\"}}"),
        )
        .expect("churn the hint");
        match AuthorityLease::acquire(&root) {
            Err(AuthorityUnavailable::Held { .. }) => {}
            other => panic!("record churn must never blink the lease, got {other:?}"),
        }
    }
    drop(lease);
    AuthorityLease::acquire(&root)
        .map(drop)
        .expect("free after the holder drops");
}
