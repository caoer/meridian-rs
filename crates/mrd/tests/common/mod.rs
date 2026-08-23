//! Shared test helpers for CLI-IPC write tests.
//!
//! Each integration-test binary compiles this module separately and uses a
//! different subset of it, so an unused helper here is the normal case, not
//! debt — hence the module-wide allow. (Without it, adding a helper reddens
//! `-D warnings` in every test file that does not happen to call it.)
//!
//! # Fixture rule: a fixture that starts a daemon owns its teardown
//!
//! Full statement and the measured cause: `crates/registry/tests/common/mod.rs`
//! § Fixture rule. In THIS tree the rule has two shapes, because a fixture here
//! may start a daemon either way:
//!
//! 1. **In-process** (`registry::RunningServer` inside the test binary): hold a
//!    `registry::TestServer`, which owns the server and its `TempDir` together
//!    and stops the server in its own `Drop::drop`. One field, so there is no
//!    teardown order left to get wrong.
//! 2. **As a subprocess** (`mrd` auto-spawning its own daemon): build the
//!    command with [`mrd_command`], the one site in this tree that sets
//!    `MRD_DRAIN_COLD_BUILDS`. Such a test holds no `Config` at all, so the
//!    environment is the only lever it has.
//!
//! Enforced at runtime, in both directions, by a `debug_assert` on
//! `registry::Config::drain_budget_hazard` — in `RunningServer::start`, and in
//! the `mrd` client before it auto-spawns a daemon, because that daemon's
//! stderr is `Stdio::null()` and its own panic reaches nobody.
#![allow(dead_code)]

use std::io::{ErrorKind, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

/// The fixture drain budget, in whole seconds — the unit
/// [`registry::DRAIN_COLD_BUILDS_ENV`] is read in.
const FIXTURE_DRAIN_SECS: &str = "30";

/// An `mrd` command for a sandbox at `home`/`cache_home`, carrying the fixture
/// drain budget.
///
/// **This is the ONE place `MRD_DRAIN_COLD_BUILDS` is set in this tree.**
///
/// Why it has to exist. A test here drives `mrd` as a subprocess, and that
/// `mrd` AUTO-SPAWNS its daemon — so the test never constructs a
/// `registry::Config` and cannot set `drain_cold_builds` on it. The daemon
/// resolves its own layout through `Config::resolve`, whose only lever from
/// outside is the environment. Without it the daemon runs the 2 s PRODUCTION
/// budget against a `TempDir` cache root, which is the class-2 flake, and it
/// fails **silently**: an auto-spawned daemon's stderr is `Stdio::null()`, so
/// it dies unheard and the client degrades to the ephemeral engine ~5 s later
/// with exit 0. Measured: 5.02 s and an ephemeral answer without this, 0.12 s
/// and a live daemon with it (card `fixture-drain-guard-followups`).
///
/// **What it does NOT do, stated because the opposite is the tempting claim.**
/// This helper CREATES a chokepoint; it does not retrofit one. Files in this
/// tree build their `mrd` command at ~73 independent `Command::new` sites, and
/// a file that does not call this function gains nothing from its existence.
/// Adopted so far: the population measured to trip the assert. Every other
/// candidate stays exposed until it adopts this, and that residue is tracked on
/// its own card — not implied by this helper's presence.
///
/// The binary honours an `MRD_BIN` override, because 14 of the adopting files
/// had their own `mrd_bin()` that did — adopting the helper without it would
/// have deleted a capability while the commit claimed to be mechanical. The
/// other adopters gain the override; that is the point of one chokepoint.
///
/// Callers chain their own `.args()`, `.current_dir()` and any further `.env()`.
pub(crate) fn mrd_command(home: &Path, cache_home: &Path) -> Command {
    let bin = std::env::var_os("MRD_BIN")
        .map_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_mrd")), PathBuf::from);
    let mut cmd = Command::new(bin);
    cmd.env("HOME", home)
        .env("XDG_CACHE_HOME", cache_home)
        .env(registry::DRAIN_COLD_BUILDS_ENV, FIXTURE_DRAIN_SECS);
    cmd
}

/// Feed a spawned child's stdin, tolerating a child that has already stopped
/// reading, then close the pipe so the child sees EOF.
///
/// **`BrokenPipe` here is DATA, not failure.** A CLI that refuses FAST exits
/// before the parent finishes writing; the parent's `write_all` then takes
/// `EPIPE`, and a bare `.expect("write stdin")` panics in the TEST instead of
/// surfacing the child's refusal. The failure is load-dependent — which test
/// dies varies run to run — so it reads as a flaky suite rather than as the
/// harness bug it is. Measured twice: 2026-08-19 on `scoped_guards_client`
/// (one sha, three full-suite runs, three different outcomes; 5/5 green in
/// isolation), and 2026-08-20 as PR #158 pipeline 985 red on
/// `put_dry_and_validate_together_refuse_loudly` while pipeline 984 was green
/// on that same sha.
///
/// This weakens no assertion. A fast refusal is asserted on the child's own
/// output through `wait_with_output` — which is where the refusal always
/// lived; this only stops the parent from panicking first and hiding it.
/// Every io error that is NOT `BrokenPipe` still panics loud, and a child
/// that never refuses still has to produce whatever its test asserts.
pub(crate) fn feed_stdin(child: &mut Child, bytes: &[u8]) {
    let mut pipe = child
        .stdin
        .take()
        .expect("child was spawned with piped stdin");
    if let Err(e) = pipe.write_all(bytes).and_then(|()| pipe.flush())
        && e.kind() != ErrorKind::BrokenPipe
    {
        panic!("feeding child stdin failed with a non-EPIPE error: {e}");
    }
    // Dropping `pipe` closes the child's stdin: a child still reading sees EOF.
}

/// The socket path an `mrd` CHILD spawned with `HOME=<home>` and
/// `XDG_CACHE_HOME=<cache_home>` will derive (short-sock law: the socket is
/// hash-keyed under a short per-user base, no longer under the cache root).
///
/// Mirrors `registry::socket_path_for_cache_root` lane by lane: the Linux
/// `XDG_RUNTIME_DIR` lane reads env the child INHERITS from this very test
/// process, so the production function answers for the child too; the HOME
/// lane must be computed against the sandbox's `home`, not this process's.
pub(crate) fn child_socket_path(home: &Path, cache_home: &Path) -> PathBuf {
    let cache_root = cache_home.join("meridian");
    #[cfg(target_os = "linux")]
    if std::env::var_os("XDG_RUNTIME_DIR").is_some_and(|v| !v.is_empty()) {
        return registry::socket_path_for_cache_root(&cache_root);
    }
    registry::socket_path_under_home(home, &cache_root)
}

/// The pidfile the child's auto-spawned daemon writes: beside the socket,
/// `<socket-stem>.pid`.
pub(crate) fn child_daemon_pidfile(home: &Path, cache_home: &Path) -> PathBuf {
    child_socket_path(home, cache_home).with_extension("pid")
}

/// How long [`reap_daemon`] waits for a terminated (SIGTERM) daemon to exit before
/// escalating to SIGKILL, and again after the kill. A daemon shuts down in
/// milliseconds; this only bounds a wedged one.
const REAP_WAIT: Duration = Duration::from_secs(2);

/// Reap the resident daemon whose pidfile belongs to this sandbox
/// (`HOME=<home>`, `XDG_CACHE_HOME=<cache_home>`): SIGTERM, wait for the pid to
/// vanish, SIGKILL if it will not. Returns the pid it reaped, `None` when no
/// daemon was ever spawned (no pidfile, or one that no longer names a live
/// process). Never panics — it runs from `Drop`, on the panic path too.
///
/// **Daemon strategy for the e2e fixtures (card e2e-daemon-leak-fixture,
/// 2026-08-22).** Every sandbox that can auto-spawn a detached daemon owns its
/// lifetime and reaps it on `Drop`. Measured 2026-08-19/20: a full `-p mrd`
/// test run left ~55 idle daemons behind — one per tempdir `XDG_CACHE_HOME`,
/// each parked for `registry::DEFAULT_IDLE_EXIT` (15 min) because the
/// daemon is detached (`setsid`) and outlives the test that spawned it.
/// The alternative — a no-daemon env knob — was rejected: writes are IPC-only
/// (`write_ipc.rs`), so a suite without a daemon cannot exercise `put` at all.
/// Reaping at sandbox drop keeps every suite exercising exactly what it did
/// (auto-spawn, warm reuse across calls, IPC writes); what the suite no
/// longer exercises is the daemon's 15-minute idle self-exit, which
/// `perf_daemon_teardown` and the registry's own tests cover directly.
pub(crate) fn reap_daemon(home: &Path, cache_home: &Path) -> Option<i32> {
    let pidfile = child_daemon_pidfile(home, cache_home);
    let text = std::fs::read_to_string(pidfile).ok()?;
    let pid = text.trim().parse::<i32>().ok()?;
    // A suite that runs the daemon IN-PROCESS (`registry::RunningServer::start`
    // inside the test binary) writes its own pid here — reaping that would
    // SIGTERM the test (measured 2026-08-22 on `s2fix_cross_surface`).
    if pid <= 0 || pid == std::process::id().cast_signed() || !pid_alive(pid) {
        return None;
    }
    signal_pid(pid, libc::SIGTERM);
    if !wait_dead(pid, REAP_WAIT) {
        signal_pid(pid, libc::SIGKILL);
        let _ = wait_dead(pid, REAP_WAIT);
    }
    Some(pid)
}

/// Holds a sandbox's daemon lifetime for tests that keep `home`/`cache_home`
/// as loose locals instead of a `Sandbox` struct: reaps on `Drop`.
pub(crate) struct DaemonReaper {
    pub(crate) home: PathBuf,
    pub(crate) cache_home: PathBuf,
}

impl Drop for DaemonReaper {
    fn drop(&mut self) {
        let _ = reap_daemon(&self.home, &self.cache_home);
    }
}

/// Send `signal` to a detached daemon this process does not own as a child.
fn signal_pid(pid: i32, signal: libc::c_int) {
    // SAFETY: plain kill(2) to a pid the daemon wrote to its own pidfile.
    unsafe {
        libc::kill(pid, signal);
    }
}

/// `kill(pid, 0)` liveness probe. A zombie still answers alive; the daemon is
/// reparented to init, which reaps it, so that window is brief.
fn pid_alive(pid: i32) -> bool {
    // SAFETY: signal 0 delivers nothing; it only checks for the pid.
    unsafe { libc::kill(pid, 0) == 0 }
}

/// Poll until `pid` is gone or `timeout` elapses.
fn wait_dead(pid: i32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !pid_alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    !pid_alive(pid)
}
