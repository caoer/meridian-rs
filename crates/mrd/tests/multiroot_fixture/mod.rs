//! **The shared fixture behind the multi-root CPU gates** (`status`, `walk`, `check`). Why this
//! is one module and not one copy per target The gate these targets run is only as honest as
//! the table they measure through, and the W2 investigations finding was precisely a fixture
//! that had stopped populating the input it claimed to bound: `status_walltime.rs` set `HOME`
//! to a bare temp dir, so the mount table was EMPTY, so the eager loader had no roots to walk —
//! a green light with no lamp behind it, for months.
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
#![allow(dead_code, unreachable_pub)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;

/// Declared mount roots in the fixture's table. Four is the field shape (the W2
/// investigation measured four bound roots on the dogfood machine).
pub const ROOTS: usize = 4;
/// Directories per root. The defect is directory ENUMERATION, so the corpus is shaped like the
/// sharpest field case — `meridian-rs`, 200 markdown files behind 20,178 directories — rather
/// than like a document pile.
pub const DIRS_PER_ROOT: usize = 25_000;
/// Markdown pages per root, so the skipped work includes real parsing too.
pub const PAGES_PER_ROOT: usize = 2_000;

/// The binary every drive goes through — the real CLI, never a library call. `MRD_BIN` points
/// it at another engine, which is how the BEFORE arm of an A/B is measured: the negative
/// control for these gates is a BINARY SWAP, not an edit to the source under test, so the
/// reddening needs no code change to reproduce and cannot be left half-applied.
///
pub fn mrd_bin() -> PathBuf {
    std::env::var_os("MRD_BIN")
        .map_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_mrd")), PathBuf::from)
}

pub struct Sandbox {
    /// Held for its Drop: the tree is deleted when this goes out of scope.
    #[allow(dead_code)]
    pub tmp: tempfile::TempDir,
    pub home: PathBuf,
    pub cache_home: PathBuf,
    pub config: PathBuf,
}

pub fn sandbox() -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    Sandbox {
        cache_home: tmp.path().join("xdg-cache"),
        config: home.join("MERIDIAN.md"),
        home,
        tmp,
    }
}

pub fn run(sb: &Sandbox, cwd: &Path, args: &[&str]) -> Output {
    Command::new(mrd_bin())
        .args(args)
        .current_dir(cwd)
        .env("HOME", &sb.home)
        .env("XDG_CACHE_HOME", &sb.cache_home)
        .env("MERIDIAN_CONFIG", &sb.config)
        .env_remove("MERIDIAN_WORKSPACE")
        .output()
        .expect("spawn mrd")
}

/// One declared root: its canonical-name declaration (INV-5 — without it the bind renders
/// undeclared and the table under test is vacuous), a dirent-heavy tree, and a scatter of real
/// pages.
fn plant_root(dir: &Path, name: &str) {
    std::fs::create_dir_all(dir).expect("root dir");
    std::fs::write(
        dir.join("MERIDIAN.md"),
        format!("---\ntype: meridian-root\nversion: 1\nname: {name}\n---\n\n# {name}\n"),
    )
    .expect("root declaration");

    // Two levels, so the walk recurses rather than reading one wide directory.
    let fanout = 100usize;
    let deep = DIRS_PER_ROOT / fanout;
    for a in 0..fanout {
        for b in 0..deep {
            std::fs::create_dir_all(dir.join(format!("d{a:02}/s{b:02}"))).expect("mkdir");
        }
    }
    let pages = dir.join("pages");
    std::fs::create_dir_all(&pages).expect("pages dir");
    for i in 0..PAGES_PER_ROOT {
        std::fs::write(
            pages.join(format!("page-{i:04}.md")),
            format!("# {name} page {i}\n\n## Body\n\nA paragraph of body text for page {i}.\n"),
        )
        .expect("page");
    }
}

/// Plant [`ROOTS`] dirent-heavy roots and write the mount table that declares
/// them. Returns their canonical names.
pub fn plant_declared_roots(sb: &Sandbox) -> Vec<String> {
    let names: Vec<String> = (0..ROOTS).map(|i| format!("root{i}")).collect();
    let mut table = String::from("---\ntype: meridian-config\nversion: 1\n---\n\n# Perf roots\n\n");
    for name in &names {
        let dir = sb.tmp.path().join(name);
        plant_root(&dir, name);
        writeln!(
            table,
            "```meridian-mount\nname: {name}\npath: {}\nkind: vault\nvault: {name}vault\n```\n",
            dir.display()
        )
        .expect("writing into a String cannot fail");
    }
    std::fs::write(&sb.config, &table).expect("mount table");
    names
}

/// **The fixtures own anti-blindness assert.** This is the check whose absence let
/// `status_walltime.rs` measure an empty table for months: prove the table under test is
/// POPULATED and BOUND before trusting anything measured through it. A fixture that quietly
/// stops declaring roots must fail here, loudly, rather than pass a CPU budget below for the
/// wrong reason.
pub fn assert_table_is_populated(sb: &Sandbox, ws: &Path, names: &[String]) {
    let cfg = run(sb, ws, &["config"]);
    let cfg_out = String::from_utf8_lossy(&cfg.stdout).into_owned();
    for name in names {
        assert!(
            cfg_out.contains(name.as_str()),
            "the mount table under test must DECLARE {name} — it read:\n{cfg_out}"
        );
    }
    assert_eq!(
        cfg_out.matches("bound").count(),
        ROOTS,
        "all {ROOTS} declared roots must BIND, or the corpora this gate measures \
         the absence of were never buildable in the first place — it read:\n{cfg_out}"
    );
}

/// An ordinary workspace whose locks name NO root — the common case, and the one
/// the narrowing is supposed to make cheap.
pub fn init_workspace(sb: &Sandbox) -> PathBuf {
    let ws = sb.tmp.path().join("ws");
    std::fs::create_dir_all(&ws).expect("ws");
    let init = run(sb, &ws, &["init"]);
    assert!(
        init.status.success(),
        "init: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    ws
}

/// The childs user+sys CPU, cumulative over every child this process has reaped. A delta of it
/// around one `.output()` call attributes to that child **only while this process spawns
/// nothing else concurrently**, which is why each target using this holds exactly ONE `[test]`.
///
///
///
///
///
pub fn children_cpu() -> Duration {
    // SAFETY: `getrusage` writes a fully-initialised `rusage` into the out
    // pointer and reads nothing else; `RUSAGE_CHILDREN` is a valid `who`.
    let mut usage = unsafe { std::mem::zeroed::<libc::rusage>() };
    let rc = unsafe { libc::getrusage(libc::RUSAGE_CHILDREN, &raw mut usage) };
    assert_eq!(rc, 0, "getrusage(RUSAGE_CHILDREN)");
    let secs = |t: libc::timeval| {
        Duration::new(
            u64::try_from(t.tv_sec).expect("non-negative seconds"),
            u32::try_from(t.tv_usec).expect("microseconds fit") * 1_000,
        )
    };
    secs(usage.ru_utime) + secs(usage.ru_stime)
}

// Daemon teardown — the perf lanes hygiene for resident auto-spawns A sandboxed
// `XDG_CACHE_HOME` dies with the tempdir; a detached `mrd daemon` does not. On a long-lived
// self-hosted runner every leak accumulates (W6 measured 16 under one worktrees debug binary).
//
//
//
//
//
//
//
//
//
//

/// The resident daemon's pidfile under this sandbox's cache root.
pub fn daemon_pidfile(sb: &Sandbox) -> PathBuf {
    sb.cache_home
        .join("meridian")
        .join("registry")
        .join("daemon.pid")
}

/// Read the pid the daemon wrote, if the pidfile is present and parseable.
pub fn read_daemon_pid(sb: &Sandbox) -> Option<i32> {
    let text = std::fs::read_to_string(daemon_pidfile(sb)).ok()?;
    text.trim().parse().ok()
}

/// Probe liveness with `kill(pid, 0)` — never a process-table substring.
pub fn pid_alive(pid: i32) -> bool {
    // SAFETY: signal 0 probes existence without delivering a signal.
    unsafe { libc::kill(pid, 0) == 0 }
}

/// Send `signal` to a detached daemon we do not own as a child.
fn signal_pid(pid: i32, signal: libc::c_int) {
    // SAFETY: plain kill(2) to a pid read from the daemon's own pidfile.
    unsafe {
        libc::kill(pid, signal);
    }
}

fn wait_dead(pid: i32, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if !pid_alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    !pid_alive(pid)
}

/// Best-effort reap: TERM → verify → KILL → verify. Never panics (Drop path).
/// Returns the pid that was signalled, if any.
pub fn try_teardown_daemon(sb: &Sandbox) -> Option<i32> {
    let pid = read_daemon_pid(sb)?;
    signal_pid(pid, libc::SIGTERM);
    if !wait_dead(pid, Duration::from_secs(2)) {
        signal_pid(pid, libc::SIGKILL);
        let _ = wait_dead(pid, Duration::from_secs(2));
    }
    Some(pid)
}

/// **Asserted** teardown for the control gate: if a pidfile names a live daemon, kill it and
/// ASSERT it is gone. If there is no pidfile, this is a no-op only when the caller has already
/// proved no spawn was expected; the control target never relies on that branch — it asserts
/// the spawn first.
pub fn teardown_daemon(sb: &Sandbox) {
    let Some(pid) = read_daemon_pid(sb) else {
        return;
    };
    signal_pid(pid, libc::SIGTERM);
    if !wait_dead(pid, Duration::from_secs(2)) {
        signal_pid(pid, libc::SIGKILL);
        assert!(
            wait_dead(pid, Duration::from_secs(2)),
            "daemon pid {pid} survived SIGKILL — the harness cannot claim a clean teardown"
        );
    }
    assert!(
        !pid_alive(pid),
        "control after teardown: pid {pid} must be dead (pidfile path, never pgrep)"
    );
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        // Best-effort only: a panicking test must not leave a resident behind, and Drop itself must
        // not panic. The control targets asserted path is [`teardown_daemon`], not this.
        //
        let _ = try_teardown_daemon(self);
    }
}
