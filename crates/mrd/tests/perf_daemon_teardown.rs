//! Perf-lane daemon teardown control — the gate that the status/walk/check CPU targets cannot
//! be, because those verbs are pure-local and never spawn. Each execution sandboxes
//! `XDG_CACHE_HOME` into a fresh tempdir; a detached daemon outlives that tempdir and is
//! never re-found by the next run, so on a long-lived bench runner residents accumulate
//! forever (G11's idle-exit bounds the class at the product root; this file is the harness
//! half).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

struct Sandbox {
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
}

fn sandbox() -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let sb = Sandbox {
        cache_home: tmp.path().join("xdg-cache"),
        home: tmp.path().join("home"),
        tmp,
    };
    std::fs::create_dir_all(&sb.home).expect("home");
    sb
}

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

fn run(sb: &Sandbox, cwd: &Path, args: &[&str]) -> Output {
    Command::new(mrd_bin())
        .args(args)
        .current_dir(cwd)
        .env("XDG_CACHE_HOME", &sb.cache_home)
        .env("HOME", &sb.home)
        .env_remove("MERIDIAN_WORKSPACE")
        .output()
        .expect("spawn mrd")
}

fn daemon_pidfile(sb: &Sandbox) -> PathBuf {
    sb.cache_home
        .join("meridian")
        .join("registry")
        .join("daemon.pid")
}

fn wait_daemon_pid(sb: &Sandbox, timeout: Duration) -> Option<i32> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(text) = std::fs::read_to_string(daemon_pidfile(sb))
            && let Ok(pid) = text.trim().parse::<i32>()
        {
            return Some(pid);
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    None
}

fn pid_alive(pid: i32) -> bool {
    // SAFETY: signal 0 probes existence without delivering a signal.
    unsafe { libc::kill(pid, 0) == 0 }
}

fn signal_pid(pid: i32, sig: libc::c_int) {
    // SAFETY: plain kill(2) to a pid the daemon wrote to its own pidfile.
    unsafe {
        libc::kill(pid, sig);
    }
}

fn wait_dead(pid: i32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !pid_alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    !pid_alive(pid)
}

/// TERM → verify → KILL → verify, then hard-assert death. Never skip.
fn teardown_asserted(pid: i32) {
    signal_pid(pid, libc::SIGTERM);
    if !wait_dead(pid, Duration::from_secs(2)) {
        signal_pid(pid, libc::SIGKILL);
        assert!(
            wait_dead(pid, Duration::from_secs(2)),
            "daemon pid {pid} survived SIGKILL"
        );
    }
    assert!(
        !pid_alive(pid),
        "control after teardown: pid {pid} must be dead"
    );
}

/// One cycle: spawn via `links`, prove warm, teardown, prove dead.
/// Returns the pid that lived, for the census trail.
fn one_cycle(label: &str) -> i32 {
    let sb = sandbox();
    let ws = sb.tmp.path().join("ws");
    std::fs::create_dir_all(ws.join(".git")).expect("git anchor");
    std::fs::write(ws.join("a.md"), "# A\n\nsee [[b]]\n").expect("a");
    std::fs::write(ws.join("b.md"), "# B\n").expect("b");
    let init = run(&sb, &ws, &["init"]);
    assert!(
        init.status.success(),
        "{label} init: {}",
        String::from_utf8_lossy(&init.stderr)
    );

    // Cold client auto-spawns. Capture the answer WHILE the daemon is up.
    let warm = run(&sb, &ws, &["links", "--json"]);
    let pid = wait_daemon_pid(&sb, Duration::from_secs(5));

    // Reap BEFORE any further assert that might panic and skip the control.
    let Some(pid) = pid else {
        panic!("{label}: auto-spawn wrote a pidfile");
    };
    assert!(
        pid_alive(pid),
        "{label}: pidfile named {pid} but the process is already gone"
    );
    teardown_asserted(pid);

    // Control read AFTER teardown — always asserts, never skip-and-pass.
    // (a) the pid is dead (already asserted inside teardown_asserted)
    // (b) a forced-degrade links answers ephemeral, proving no resident remains
    //     for this sandbox to dial.
    let cold = Command::new(mrd_bin())
        .args(["links", "--json"])
        .current_dir(&ws)
        .env("XDG_CACHE_HOME", &sb.cache_home)
        .env("HOME", &sb.home)
        .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
        .env_remove("MERIDIAN_WORKSPACE")
        .output()
        .expect("spawn mrd");

    assert!(
        warm.status.success(),
        "{label} warm links: {}",
        String::from_utf8_lossy(&warm.stderr)
    );
    let warm_body = String::from_utf8_lossy(&warm.stdout);
    assert!(
        warm_body.contains("\"source\": \"daemon\""),
        "{label}: arm A must be daemon-backed, or the control compares two degrades: {warm_body}"
    );
    assert!(
        cold.status.success(),
        "{label} cold links: {}",
        String::from_utf8_lossy(&cold.stderr)
    );
    let cold_body = String::from_utf8_lossy(&cold.stdout);
    assert!(
        cold_body.contains("\"source\": \"ephemeral\""),
        "{label}: control after teardown must degrade (no resident): {cold_body}"
    );
    assert!(
        !pid_alive(pid),
        "{label}: control after teardown — pid {pid} must still be dead"
    );

    eprintln!("{label}: spawned pid={pid}, torn down, control read = ephemeral");
    pid
}

/// Field-exact count of daemons whose argv[0] is exactly this binary.
/// Shape match only — never `pgrep -f` (matches agent prompts; misses live daemons).
fn census_this_binary() -> usize {
    let bin = mrd_bin();
    let out = Command::new("ps")
        .args(["-eo", "args="])
        .output()
        .expect("ps");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|line| {
            let line = line.trim_start();
            // Exact shape: "<bin> daemon" or "<bin> daemon <more>"
            line == format!("{bin} daemon") || line.starts_with(&format!("{bin} daemon "))
        })
        .count()
}

#[test]
fn two_cycles_leave_zero_residents_of_this_binary() {
    let before = census_this_binary();
    eprintln!("census before: {before} (this binary = {})", mrd_bin());

    let pid1 = one_cycle("cycle-1");
    let mid = census_this_binary();
    eprintln!("census after cycle-1 (pid {pid1} reaped): {mid}");
    assert_eq!(
        mid, before,
        "cycle-1 must not leave a resident of this binary (before={before}, after={mid})"
    );

    let pid2 = one_cycle("cycle-2");
    let after = census_this_binary();
    eprintln!("census after cycle-2 (pid {pid2} reaped): {after}");
    assert_eq!(
        after, before,
        "two cycles must leave the census unchanged (before={before}, after={after}) — \
         without teardown this would be before+2"
    );
    assert_ne!(pid1, pid2, "each cycle spawns a fresh daemon");
}
