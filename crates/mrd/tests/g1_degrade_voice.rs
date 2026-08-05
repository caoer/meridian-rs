//! G1 gates: the silent ephemeral degrade gets a VOICE on the human face. `--json` has always
//! carried `"source"`, so a machine reader could see that an answer came from the in-process
//! degrade rather than the resident daemon. `mrd read`'s human face prints the rendered
//! projection and nothing else, so a person got a correct answer from a slower path with no
//! signal at all — and roughly twenty measurements taken on this engine were measurements of
//! the wrong path, unknowably.
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
// Gate 4: the named cause — a socket path no daemon can bind.
// ---------------------------------------------------------------------------

/// The exact hazard G1 was found through: an `XDG_CACHE_HOME` long enough that
/// `<cache>/meridian/registry/daemon.sock` exceeds `sun_path`. No daemon can
/// bind it and none can be dialled, so starting one is not the fix — and the
/// voice must say which fix is.
#[test]
fn g1_over_long_socket_path_names_the_sun_path_limit() {
    let deep: String = std::iter::repeat_n("averylongcachedirectorysegment", 6)
        .collect::<Vec<_>>()
        .join("/");
    let sb = sandbox_at(&deep);
    let ws = sb.workspace();
    let socket = sb.cache_root.join("registry").join("daemon.sock");
    assert!(
        socket.as_os_str().len() > 104,
        "the fixture must actually exceed sun_path: {} bytes",
        socket.as_os_str().len()
    );

    let out = sb.run(&ws, &["read", "doc.md"]);
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
        err.contains("XDG_CACHE_HOME"),
        "and it names the knob that fixes it: {err:?}"
    );
}
