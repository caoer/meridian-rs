//! G15 gates: `mrd view status` names the unbindable socket instead of letting `NO_VIEW` be
//! read as "this workspace has no view". Dogfood pass-3 ran `mrd view status` under an
//! over-long `XDG_CACHE_HOME`. No daemon can bind or dial a socket at or over `sun_path`, so
//! the command fell back to the cold drawer and printed `source: absent`, `state: NO_VIEW` at
//! exit 0 with an EMPTY stderr.
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

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

const DOC: &str = "# Alpha\n\none two three\n";

/// The phrase that says a daemon did not answer this report.
const DAEMONLESS_PHRASE: &str = "no daemon answered";
/// The phrase that stops `NO_VIEW` being read as "the workspace has no view".
const DRAWER_PHRASE: &str = "this cache drawer holds no published view";
/// The named cause g1 already owns — shared here, never re-authored.
const SUN_PATH_PHRASE: &str = "sun_path limit";

struct Sandbox {
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
    cache_root: PathBuf,
}

/// A cache home deep enough that `<cache>/meridian/registry/daemon.sock`
/// exceeds `sun_path` on every supported platform.
fn deep_cache_rel() -> String {
    std::iter::repeat_n("averylongcachedirectorysegment", 6)
        .collect::<Vec<_>>()
        .join("/")
}

fn sandbox_at(cache_rel: &str) -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_home = tmp.path().join(cache_rel);
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    let cache_root = cache_home.join("meridian");
    Sandbox {
        tmp,
        cache_home,
        home,
        cache_root,
    }
}

impl Sandbox {
    fn base(&self) -> Command {
        let mut cmd = Command::new(mrd_bin());
        cmd.env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env_remove("MERIDIAN_WORKSPACE");
        cmd
    }

    /// Run with the real daemon reachable (auto-spawn allowed).
    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        self.base()
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("spawn mrd")
    }

    /// Run spawn-impossible: no daemon can start, so the report is a disk
    /// reading deterministically.
    fn run_daemonless(&self, cwd: &Path, args: &[&str]) -> Output {
        self.base()
            .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("spawn mrd")
    }

    /// An anchored workspace holding the fixture doc.
    fn workspace(&self) -> PathBuf {
        let ws = self.tmp.path().join("project");
        std::fs::create_dir_all(ws.join(".git")).expect("git anchor");
        std::fs::write(ws.join("doc.md"), DOC).expect("doc");
        std::fs::canonicalize(&ws).expect("canonical ws")
    }

    fn daemon_pidfile(&self) -> PathBuf {
        self.cache_root.join("registry").join("daemon.pid")
    }

    fn wait_daemon_pid(&self, timeout: Duration) -> Option<i32> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Ok(text) = std::fs::read_to_string(self.daemon_pidfile())
                && let Ok(pid) = text.trim().parse::<i32>()
            {
                return Some(pid);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        None
    }

    /// Reap the auto-spawned daemon by its own pidfile — it is detached, so it
    /// is never a child of this test, and a leaked one holds the socket.
    fn reap(&self) -> Option<i32> {
        let pid = self.wait_daemon_pid(Duration::from_secs(10))?;
        signal(pid, libc::SIGTERM);
        wait_dead(pid, Duration::from_secs(10));
        Some(pid)
    }
}

/// Send `signal` to `pid` (a detached daemon we do not own as a child).
fn signal(pid: i32, signal: libc::c_int) {
    // SAFETY: a plain `kill(2)` on a pid the daemon wrote to its own pidfile.
    unsafe {
        libc::kill(pid, signal);
    }
}

/// Poll until `pid` is gone, so the killed daemon has released its socket.
fn wait_dead(pid: i32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        // SAFETY: signal 0 probes existence without delivering a signal.
        if unsafe { libc::kill(pid, 0) } == -1 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    false
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

// ---------------------------------------------------------------------------
// Gate 1: the measured hazard — an unbindable socket, named.
// ---------------------------------------------------------------------------

/// The exact dogfood configuration: an `XDG_CACHE_HOME` long enough that the
/// socket exceeds `sun_path`. Base printed `absent` / `NO_VIEW` with an empty
/// stderr; the fix must keep the verdict and add the cause.
#[test]
fn g15_unbindable_socket_names_the_socket_not_just_no_view() {
    let sb = sandbox_at(&deep_cache_rel());
    let ws = sb.workspace();
    let socket = sb.cache_root.join("registry").join("daemon.sock");
    assert!(
        socket.as_os_str().len() > 104,
        "the fixture must actually exceed sun_path: {} bytes",
        socket.as_os_str().len()
    );

    let out = sb.run(&ws, &["view", "status"]);
    let err = stderr(&out);

    assert_eq!(out.status.code(), Some(0), "view status exits 0: {err:?}");
    assert!(
        stdout(&out).contains("NO_VIEW"),
        "the drawer verdict is unchanged — the drawer really is empty: {}",
        stdout(&out)
    );
    assert!(
        err.contains(DAEMONLESS_PHRASE),
        "the report says it is not daemon telemetry: {err:?}"
    );
    assert!(
        err.contains(DRAWER_PHRASE),
        "and it says what NO_VIEW does and does not mean: {err:?}"
    );
    assert!(
        err.contains(SUN_PATH_PHRASE),
        "the real cause is named, not left to be guessed: {err:?}"
    );
    assert!(
        err.contains("XDG_CACHE_HOME"),
        "and the knob that fixes it: {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Gate 2: a view PRESENT but unreachable — the cold arm speaks too.
// ---------------------------------------------------------------------------

/// A published `view.duckdb` in the drawer, then the socket made unbindable.
/// The drawer stamp is real, so the verdict is `cold` rather than `NO_VIEW` —
/// but it is still a disk reading standing in for daemon telemetry, and the
/// unbindable socket is still the reason. Both facts must reach stderr.
#[test]
fn g15_present_view_under_an_unbindable_socket_still_names_the_socket() {
    // Publish warm under a SHORT cache home first: only the daemon publishes a
    // view, and no daemon can run under the long one.
    let sb = sandbox_at("c");
    let ws = sb.workspace();
    let warm = sb.run(&ws, &["sql", "select 1"]);
    let pid = sb.reap();

    assert_eq!(out_code(&warm), 0, "warm publish run: {}", stderr(&warm));
    assert!(pid.is_some(), "the auto-spawned daemon wrote a pidfile");
    let view = find_view(sb.tmp.path()).expect("the warm run published a view.duckdb");
    assert!(view.is_file());

    // The drawer is keyed by the WORKSPACE path, so moving the cache root to a
    // long path keeps the published view and takes the socket over sun_path.
    let deep = sb.tmp.path().join(deep_cache_rel());
    std::fs::create_dir_all(deep.parent().expect("parent")).expect("deep parent");
    std::fs::rename(&sb.cache_home, &deep).expect("move cache root");
    let long = Sandbox {
        cache_root: deep.join("meridian"),
        cache_home: deep,
        home: sb.home.clone(),
        tmp: sb.tmp,
    };

    let out = long.run(&ws, &["view", "status"]);
    let err = stderr(&out);

    assert_eq!(out.status.code(), Some(0), "view status exits 0: {err:?}");
    assert!(
        stdout(&out).contains("source: cold"),
        "the published view is still found on disk: {}",
        stdout(&out)
    );
    assert!(
        err.contains(DAEMONLESS_PHRASE),
        "a cold stamp is not daemon telemetry, and says so: {err:?}"
    );
    assert!(
        err.contains(SUN_PATH_PHRASE),
        "the unbindable socket is named on the cold arm too: {err:?}"
    );
    assert!(
        !err.contains(DRAWER_PHRASE),
        "the NO_VIEW caveat belongs only to the arm that printed NO_VIEW: {err:?}"
    );
}

fn out_code(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}

fn find_view(root: &Path) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).ok()?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().is_some_and(|n| n == "view.duckdb") {
                return Some(path);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Gate 3: no misdiagnosis traded for another. A genuinely absent view still
// reports NO_VIEW.
//
// **The "both paths" arm is VACUOUS, and this gate says so instead of faking
// it.** Dialling `view_path` MATERIALISES the view, so a daemon-backed
// `view status` on a never-published workspace answers `FRESH_AT_SAMPLE`, not
// `NO_VIEW` — measured, and asserted below. `NO_VIEW` is therefore structurally
// a daemonless verdict, which is exactly why its silence about the daemon was
// the whole G15 defect. The gate pins the daemonless arm.
// ---------------------------------------------------------------------------

#[test]
fn g15_genuinely_absent_view_still_reports_no_view_daemonless() {
    let sb = sandbox_at("c");
    let ws = sb.workspace();

    // A: daemon-backed, nothing ever published — it builds rather than
    // reporting absence. Pinned so the vacuity is a measurement, not a claim.
    let warm = sb.run(&ws, &["view", "status"]);
    let warm_json = sb.run(&ws, &["view", "status", "--json"]);
    sb.reap();

    // B: a never-published workspace with no daemon reachable. It needs its own
    // sandbox — arm A materialised a view in this one, so reusing it would test
    // the cold arm and quietly void the gate.
    let fresh = sandbox_at("c");
    let fresh_ws = fresh.workspace();
    let daemonless = fresh.run_daemonless(&fresh_ws, &["view", "status"]);

    assert_eq!(warm.status.code(), Some(0), "warm exits 0");
    assert_eq!(daemonless.status.code(), Some(0), "daemonless exits 0");
    assert!(
        stdout(&warm).contains("source: daemon") && !stdout(&warm).contains("NO_VIEW"),
        "the daemon MATERIALISES the view rather than reporting NO_VIEW: {}",
        stdout(&warm)
    );
    assert!(
        stdout(&warm_json).contains("\"source\": \"daemon\""),
        "and the json face is on the same path: {}",
        stdout(&warm_json)
    );
    assert!(
        stdout(&daemonless).contains("NO_VIEW"),
        "the daemonless read still reports NO_VIEW — the verdict never moved: {}",
        stdout(&daemonless)
    );
    assert!(
        stderr(&daemonless).contains(DAEMONLESS_PHRASE),
        "the daemonless arm still says a daemon did not answer: {:?}",
        stderr(&daemonless)
    );
    assert!(
        !stderr(&daemonless).contains(SUN_PATH_PHRASE),
        "a SHORT socket path is not blamed for a genuinely absent view: {:?}",
        stderr(&daemonless)
    );
}

// ---------------------------------------------------------------------------
// Gate 4: the daemon path is untouched — silent, and byte-identical on both
// faces. The voice can never be accused of changing what it reports on.
// ---------------------------------------------------------------------------

#[test]
fn g15_warm_path_is_silent_on_both_faces() {
    let sb = sandbox_at("c");
    let ws = sb.workspace();

    // Publish a view so the warm report carries real telemetry, then read both
    // faces while the daemon is still up.
    sb.run(&ws, &["sql", "select 1"]);
    let human = sb.run(&ws, &["view", "status"]);
    let json = sb.run(&ws, &["view", "status", "--json"]);
    let pid = sb.reap();

    assert!(pid.is_some(), "the auto-spawned daemon wrote a pidfile");
    assert!(
        stdout(&human).contains("source: daemon"),
        "arm A must really be daemon-backed, or this gate proves nothing: {}",
        stdout(&human)
    );
    assert!(
        stdout(&json).contains("\"source\": \"daemon\""),
        "the json face too: {}",
        stdout(&json)
    );
    assert_eq!(
        stderr(&human),
        "",
        "a daemon-backed report says nothing at all"
    );
    assert_eq!(stderr(&json), "", "and neither does the json face");
    assert_eq!(human.status.code(), Some(0), "warm human exits 0");
    assert_eq!(json.status.code(), Some(0), "warm json exits 0");
}
