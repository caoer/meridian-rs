//! G1 gates: the silent ephemeral degrade gets a VOICE on the human face. `--json` has always
//! carried `"source"`, so a machine reader could see that an answer came from the in-process
//! degrade rather than the resident daemon. `mrd read`'s human face prints the rendered
//! projection and nothing else, so a person got a correct answer from a slower path with no
//! signal at all — and roughly twenty measurements taken on this engine were measurements of
//! the wrong path, unknowably.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

mod common;

const DOC: &str = "# Alpha\n\none two three\n\n## Beta\n\nfour five\n";

/// The two phrases the voice owes a reader: WHICH path served the answer, and
/// that the timing of this run means nothing.
const SOURCE_PHRASE: &str = "source: ephemeral";
const TIMING_PHRASE: &str = "TIMING is not";

struct Sandbox {
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
    cache_root: PathBuf,
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

fn sandbox() -> Sandbox {
    sandbox_at("xdg-cache")
}

impl Sandbox {
    fn base(&self) -> Command {
        let mut cmd = common::mrd_command(&self.home, &self.cache_home);
        cmd.env_remove("MERIDIAN_WORKSPACE");
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

    /// Run spawn-impossible: no daemon can start, so the answer degrades
    /// in-process deterministically.
    fn run_degraded(&self, cwd: &Path, args: &[&str]) -> Output {
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
        common::child_daemon_pidfile(&self.home, &self.cache_home)
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
}

/// Send `signal` to `pid` (a detached daemon we do not own as a child).
fn signal(pid: i32, signal: libc::c_int) {
    // SAFETY: a plain `kill(2)` on a pid the daemon wrote to its own pidfile.
    unsafe {
        libc::kill(pid, signal);
    }
}

/// Poll until `pid` is gone, so the killed daemon has released its socket
/// before the next client dials.
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

// ---------------------------------------------------------------------------
// Gate 1: the degrade speaks — and says both things a reader needs.
// ---------------------------------------------------------------------------

#[test]
fn g1_degraded_human_read_voices_the_source_on_stderr() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run_degraded(&ws, &["read", "doc.md"]);

    assert_eq!(out.status.code(), Some(0), "read exits 0: {}", stderr(&out));
    let err = stderr(&out);
    assert!(
        err.contains(SOURCE_PHRASE),
        "the degrade names its source: {err:?}"
    );
    assert!(
        err.contains(TIMING_PHRASE),
        "the degrade warns that this run cannot be measured: {err:?}"
    );
    assert!(
        !out.stdout.is_empty(),
        "the answer still rides stdout: {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Gate 2 + 3: the A/B. Warm is silent; degraded speaks; stdout is IDENTICAL.
// ---------------------------------------------------------------------------

#[test]
fn g1_warm_is_silent_and_stdout_is_byte_identical_to_the_degrade() {
    let sb = sandbox();
    let ws = sb.workspace();

    // A: daemon-backed. The cold first use auto-spawns the resident daemon.
    // The `--json` arm is taken FIRST and while the daemon is still up: it is
    // the proof that arm A really was warm, and reading it after the reap
    // would report the degrade and quietly void the whole control.
    let warm_json = sb.run(&ws, &["read", "doc.md", "--json"]);
    let warm = sb.run(&ws, &["read", "doc.md"]);

    // Reap the auto-spawned daemon BEFORE asserting, so a failed assertion
    // never leaks it — it is detached, so it is signalled by its own pidfile.
    let pid = sb.wait_daemon_pid(Duration::from_secs(5));
    if let Some(pid) = pid {
        signal(pid, libc::SIGTERM);
        wait_dead(pid, Duration::from_secs(5));
    }

    // B: the same read with no daemon reachable at all.
    let degraded = sb.run_degraded(&ws, &["read", "doc.md"]);

    assert!(pid.is_some(), "the auto-spawned daemon wrote a pidfile");
    assert!(
        String::from_utf8_lossy(&warm_json.stdout).contains("\"source\": \"daemon\""),
        "arm A must really be daemon-backed, or this gate compares two degrades: {}",
        String::from_utf8_lossy(&warm_json.stdout)
    );
    assert_eq!(warm.status.code(), degraded.status.code(), "same exit code");
    assert_eq!(
        warm.stdout, degraded.stdout,
        "the ANSWER is byte-identical across warm and degrade"
    );
    assert!(
        !stderr(&warm).contains(SOURCE_PHRASE),
        "a daemon-backed read says nothing: {:?}",
        stderr(&warm)
    );
    assert!(
        stderr(&degraded).contains(SOURCE_PHRASE),
        "the degrade is the only arm that speaks: {:?}",
        stderr(&degraded)
    );
}

// ---------------------------------------------------------------------------
// Gate 4 (the short-sock law): a DEEP cache root does not kill the socket —
// the daemon binds the hash-keyed short path and serves WARM. The exact
// hazard G1 was found through, closed at the root instead of voiced.
// ---------------------------------------------------------------------------

/// An `XDG_CACHE_HOME` long enough that the OLD in-root socket placement
/// (`<cache>/meridian/registry/daemon.sock`) would exceed `sun_path`. Under
/// the short-sock law the socket rides `hash(cache_root)` under a short
/// per-user base, so the daemon binds, the client dials, and the answer is
/// daemon-backed — no degrade, no TMPDIR/XDG length requirement.
#[test]
fn g1_deep_cache_root_still_binds_and_serves_warm() {
    let deep: String = std::iter::repeat_n("averylongcachedirectorysegment", 6)
        .collect::<Vec<_>>()
        .join("/");
    let sb = sandbox_at(&deep);
    let ws = sb.workspace();
    let old_socket = sb.cache_root.join("registry").join("daemon.sock");
    assert!(
        old_socket.as_os_str().len() > 104,
        "the fixture must exceed sun_path under the OLD placement: {} bytes",
        old_socket.as_os_str().len()
    );
    let socket = common::child_socket_path(&sb.home, &sb.cache_home);
    assert!(
        socket.as_os_str().len() < 104,
        "the derived short sock must fit sun_path: {} bytes ({})",
        socket.as_os_str().len(),
        socket.display()
    );

    // Auto-spawn allowed: the cold first use starts the resident daemon at
    // the short sock. `--json` carries `source`, the warm proof.
    let out = sb.run(&ws, &["read", "doc.md", "--json"]);

    // Reap BEFORE asserting so a failed assertion never leaks the daemon.
    let pid = sb.wait_daemon_pid(Duration::from_secs(5));
    if let Some(pid) = pid {
        signal(pid, libc::SIGTERM);
        wait_dead(pid, Duration::from_secs(5));
    }

    assert_eq!(out.status.code(), Some(0), "read exits 0: {}", stderr(&out));
    assert!(
        pid.is_some(),
        "the daemon wrote its pidfile beside the short sock"
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("\"source\": \"daemon\""),
        "a deep cache root is served WARM under the short-sock law: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// A stale socket at the OLD in-root placement is dead to the client: even a
/// LIVE daemon bound there is never dialled — the client derives only the
/// short hash-keyed path, and an old-binary resident PUBLISHED nothing (the
/// publication postdates the short-sock law; a daemon that does publish is
/// the lock holder and is dialled by design — `registry_published_socket.rs`).
/// If the client still dialled the old path unbidden, the answer would come
/// back warm and this gate would catch the regression.
#[test]
fn g1_stale_daemon_at_the_old_path_is_not_dialled() {
    let sb = sandbox();
    let ws = sb.workspace();
    let old_socket = sb.cache_root.join("registry").join("daemon.sock");
    std::fs::create_dir_all(old_socket.parent().expect("registry dir")).expect("mkdir");
    #[allow(clippy::duration_suboptimal_units)]
    let forever = Duration::from_secs(365 * 24 * 60 * 60);
    let mut config = registry::Config::for_cache_root(sb.cache_root.clone());
    config.socket_path = old_socket;
    config.idle_threshold = forever;
    config.reap_interval = forever;
    config.prewarm_interval = forever;
    config.prewarm_quiet_max = forever;
    config.idle_exit = None;
    config.build_sha = Some(env!("MRD_BUILD_SHA").to_owned());
    config.drain_cold_builds = Duration::from_secs(30);
    let server = registry::RunningServer::start(config).expect("old-path daemon binds");
    // An old binary never published its socket; this build's in-process daemon
    // just did, so take the publication away to model the stale predecessor.
    std::fs::remove_file(sb.cache_root.join("registry").join("daemon.sock-path"))
        .expect("the current build publishes its socket; an old one did not");

    let out = sb.run_degraded(&ws, &["read", "doc.md", "--json"]);
    server.shutdown();

    assert_eq!(out.status.code(), Some(0), "read exits 0: {}", stderr(&out));
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("\"source\": \"ephemeral\""),
        "the old-path daemon must be invisible — the client dials only the short sock: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// The overflow voice survives where it still can happen: a HOME so deep that
/// even the short hash-keyed path exceeds `sun_path` (pathological — the
/// short path adds ~30 bytes to the base). The degrade names the cause and
/// the knob that now matters.
#[test]
fn g1_pathological_home_still_names_the_sun_path_limit() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let deep: String = std::iter::repeat_n("averylonghomedirectorysegment", 6)
        .collect::<Vec<_>>()
        .join("/");
    let home = tmp.path().join(deep);
    std::fs::create_dir_all(&home).expect("deep home");
    let cache_home = tmp.path().join("xdg-cache");
    let ws = tmp.path().join("project");
    std::fs::create_dir_all(ws.join(".git")).expect("git anchor");
    std::fs::write(ws.join("doc.md"), DOC).expect("doc");
    let ws = std::fs::canonicalize(&ws).expect("canonical ws");

    let socket = registry::socket_path_under_home(&home, &cache_home.join("meridian"));
    assert!(
        socket.as_os_str().len() >= 104,
        "the fixture HOME must push even the short sock past sun_path: {} bytes",
        socket.as_os_str().len()
    );

    let out = common::mrd_command(&home, &cache_home)
        .env_remove("MERIDIAN_WORKSPACE")
        // Force the HOME lane so the fixture's depth is the one that counts.
        .env_remove("XDG_RUNTIME_DIR")
        .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
        .args(["read", "doc.md"])
        .current_dir(&ws)
        .output()
        .expect("spawn mrd");

    assert_eq!(out.status.code(), Some(0), "read exits 0: {}", stderr(&out));
    let err = stderr(&out);
    assert!(
        err.contains(SOURCE_PHRASE),
        "an unbindable socket degrades, and the degrade speaks: {err:?}"
    );
    assert!(
        err.contains("sun_path limit"),
        "the voice names the cause rather than leaving it to be guessed: {err:?}"
    );
    assert!(
        err.contains("HOME"),
        "and it names the knob that fixes it now (the short base, not the cache root): {err:?}"
    );
}
