//! The daemon at rest makes no system call (`docs/status.md` § The daemon at
//! rest): with no connection, no armed `sub` and no sweep due, every resident
//! thread is parked in the kernel, so the process as a whole stops switching
//! context.
//!
//! The instrument is the kernel's own count of voluntary context switches
//! per task, summed over the process — a thread that ticks shows up as one
//! switch per tick, whatever its CPU cost, so the gate discriminates where a
//! CPU-time budget would not (a 20 ms tick costs microseconds of CPU and
//! fifty switches a second). Linux-only by the instrument; the code under
//! test is the same on every unix.

#![cfg(target_os = "linux")]

use std::fs;
use std::path::Path;
use std::time::Duration;

use registry::{Config, RunningServer};

/// Over this window the daemon at rest may switch context at most
/// [`SWITCH_BUDGET`] times, process-wide. The old loops alone made ~65 a
/// second (accept every 20 ms, pre-warm every 100 ms, reaper every 200 ms).
const WINDOW: Duration = Duration::from_secs(2);

/// Headroom for the measuring thread's own sleep, one pre-warm sweep should
/// its backoff land inside the window, and scheduler noise. Well under one
/// second of any tick this gate exists to refuse.
const SWITCH_BUDGET: u64 = 12;

/// Voluntary context switches summed over every task of this process.
fn voluntary_switches() -> u64 {
    let mut total = 0;
    for task in fs::read_dir("/proc/self/task").expect("procfs") {
        let status = task.expect("task").path().join("status");
        let Ok(text) = fs::read_to_string(status) else {
            continue; // a task that exited between listing and reading
        };
        total += text
            .lines()
            .find_map(|line| line.strip_prefix("voluntary_ctxt_switches:"))
            .and_then(|n| n.trim().parse::<u64>().ok())
            .unwrap_or(0);
    }
    total
}

fn production_intervals(tmp: &Path) -> Config {
    let dir = tmp.join("registry");
    fs::create_dir_all(&dir).unwrap();
    // The production layout's own intervals: a reap every minute, a pre-warm
    // sweep that backs off toward a minute while quiet. Only the paths move
    // under the sandbox, and idle exit is off so the daemon outlives the
    // window whatever the clock says.
    let mut config = Config::for_cache_root(tmp.join("cache"));
    config.socket_path = dir.join("daemon.sock");
    config.state_path = dir.join("state.json");
    config.idle_exit = None;
    config.drain_cold_builds = Duration::from_secs(30);
    config
}

#[test]
fn a_daemon_nobody_is_talking_to_stops_switching_context() {
    let tmp = tempfile::tempdir().unwrap();
    let server = RunningServer::start(production_intervals(tmp.path())).unwrap();

    // Let start-up settle: the first pre-warm sweep fires at one second and
    // the backoff doubles from there, so after this the next sweep is at
    // least two seconds out.
    std::thread::sleep(Duration::from_millis(1500));

    let before = voluntary_switches();
    std::thread::sleep(WINDOW);
    let switched = voluntary_switches() - before;

    eprintln!(
        "daemon at rest: {switched} voluntary context switches over {} ms (budget {SWITCH_BUDGET})",
        WINDOW.as_millis()
    );
    assert!(
        switched <= SWITCH_BUDGET,
        "a daemon at rest must park, not tick: {switched} context switches in \
         {} ms against a budget of {SWITCH_BUDGET} — some resident thread is \
         waking on a clock",
        WINDOW.as_millis()
    );
    server.shutdown();
}
