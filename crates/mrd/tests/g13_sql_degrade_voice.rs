//! G13 gates: `mrd sql`'s daemon-absent degrade gets a VOICE — the same voice
//! G1 gave `mrd read`, not a second one.
//!
//! Dogfood pass-3 measured `mrd sql` degrading at **248×** the warm cost on a
//! valid query (0.24s → 59.63s) and ~140× on the error path (0.70s → 99s), with
//! **stdout, stderr and the exit code byte-identical** between the two. A person
//! paid a minute for an answer and had no channel that could tell them why.
//!
//! These gates pin the fix together with the three constraints that make it
//! safe, because any one alone is satisfiable by a wrong change:
//!
//! 1. degraded → stderr names the source and the timing caveat;
//! 2. warm → stderr is EMPTY;
//! 3. **stdout and the exit code are byte-identical across the two**, on the
//!    success path AND the error path — the error path is where the worst
//!    measured silence was, so voicing only the success arm would leave the
//!    expensive case mute;
//! 4. tier-4 bare stays silent: `:memory:` is that tier's designed path, not a
//!    degrade, and a voice there would train the reader to ignore the line.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

const DOC: &str = "# Alpha\n\none two three\n\n## Beta\n\nfour five\n";

/// A query the view answers, and one it refuses — the two arms the dogfood
/// measured.
const GOOD_QUERY: &str = "SELECT count(*) FROM doc";
const BAD_QUERY: &str = "SELECT count(*) FROM no_such_table";

/// The two phrases the shared voice owes a reader: WHICH path served the
/// answer, and that this run's timing means nothing. Identical to G1's — one
/// voice, asserted the same way from both faces.
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

    /// Run spawn-impossible: no daemon can start, so the run degrades
    /// deterministically.
    fn run_degraded(&self, cwd: &Path, args: &[&str]) -> Output {
        self.base()
            .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("spawn mrd")
    }

    /// An ANCHORED workspace (tiers 1-3): the daemon is its sole builder, so a
    /// daemonless run here is a genuine degrade.
    fn workspace(&self) -> PathBuf {
        let ws = self.tmp.path().join("project");
        std::fs::create_dir_all(ws.join(".git")).expect("git anchor");
        std::fs::write(ws.join("doc.md"), DOC).expect("doc");
        std::fs::canonicalize(&ws).expect("canonical ws")
    }

    /// A BARE directory (tier-4): no anchor, no registration. `:memory:` is its
    /// designed path.
    fn bare_dir(&self) -> PathBuf {
        let ws = self.tmp.path().join("bare");
        std::fs::create_dir_all(&ws).expect("bare dir");
        std::fs::write(ws.join("doc.md"), DOC).expect("doc");
        std::fs::canonicalize(&ws).expect("canonical bare")
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

    /// Reap the auto-spawned daemon by its OWN pidfile — it is detached, so it
    /// is not a child, and `pgrep -f` would sweep a sibling test's daemon.
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

// Gate 1: the degrade speaks — and says both things a reader needs.
//
//

#[test]
fn g13_degraded_sql_voices_the_source_on_stderr() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run_degraded(&ws, &["sql", GOOD_QUERY]);

    assert_eq!(out.status.code(), Some(0), "sql exits 0: {}", stderr(&out));
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

// Gate 2 + 3: the A/B. Warm is silent; degraded speaks; stdout is IDENTICAL.
//
//

#[test]
fn g13_warm_is_silent_and_stdout_is_byte_identical_to_the_degrade() {
    let sb = sandbox();
    let ws = sb.workspace();

    // A: daemon-backed. The cold first use auto-spawns the resident daemon and
    // publishes the view file it queries.
    let warm = sb.run(&ws, &["sql", GOOD_QUERY]);

    // Reap BEFORE asserting, so a failed assertion never leaks the daemon.
    let pid = sb.reap();

    // B: the same query with no daemon reachable at all.
    let degraded = sb.run_degraded(&ws, &["sql", GOOD_QUERY]);

    assert!(pid.is_some(), "the auto-spawned daemon wrote a pidfile");
    assert!(
        stderr(&warm).is_empty(),
        "arm A must really be daemon-backed and silent, or this gate compares \
         two degrades: {:?}",
        stderr(&warm)
    );
    assert_eq!(warm.status.code(), degraded.status.code(), "same exit code");
    assert_eq!(
        warm.stdout, degraded.stdout,
        "the ANSWER is byte-identical across warm and degrade"
    );
    assert!(
        stderr(&degraded).contains(SOURCE_PHRASE),
        "the degrade is the only arm that speaks: {:?}",
        stderr(&degraded)
    );
}

// Gate 3b: the ERROR path — the most expensive silence the dogfood measured.
//
//

/// A refused query degraded 0.70s → 99s with stderr byte-identical, so the worst case was the
/// mute one. The refusal text and the exit code must survive unchanged; the voice is additive.
///
#[test]
fn g13_the_error_path_degrade_speaks_without_changing_the_refusal() {
    let sb = sandbox();
    let ws = sb.workspace();

    let warm = sb.run(&ws, &["sql", BAD_QUERY]);
    let pid = sb.reap();
    let degraded = sb.run_degraded(&ws, &["sql", BAD_QUERY]);

    assert!(pid.is_some(), "the auto-spawned daemon wrote a pidfile");
    assert_ne!(warm.status.code(), Some(0), "the bad query really refuses");
    assert_eq!(
        warm.status.code(),
        degraded.status.code(),
        "the exit code is unchanged by the degrade"
    );
    assert_eq!(
        warm.stdout, degraded.stdout,
        "stdout is byte-identical on the error path too"
    );
    let warm_err = stderr(&warm);
    let degraded_err = stderr(&degraded);
    assert!(
        !warm_err.contains(SOURCE_PHRASE),
        "the warm refusal says nothing about a degrade: {warm_err:?}"
    );
    assert!(
        degraded_err.contains(SOURCE_PHRASE),
        "the degraded refusal names its source: {degraded_err:?}"
    );
    assert!(
        degraded_err.contains(&warm_err),
        "the refusal text itself is unchanged — the voice is ADDITIVE, not a \
         rewrite: warm={warm_err:?} degraded={degraded_err:?}"
    );
}

// Gate 4: tier-4 bare is NOT a degrade, and stays silent.
//
//

/// `:memory:` is tier-4's designed path (§tier-4 — never the daemon, never a drawer). Crying
/// degrade on every correct run in an unregistered directory would teach a reader to filter the
/// line out, which costs the voice exactly where it matters.
///
#[test]
fn g13_tier4_bare_is_the_designed_path_and_says_nothing() {
    let sb = sandbox();
    let bare = sb.bare_dir();
    let out = sb.run_degraded(&bare, &["sql", GOOD_QUERY]);

    let err = stderr(&out);
    assert!(
        !err.contains(SOURCE_PHRASE),
        "tier-4 `:memory:` is not a degrade and must not claim to be: {err:?}"
    );
}

// Gate 4b: the `--json` face — voiced on stderr, OD9 document UNTOUCHED.
//
//

/// **The one policy fork this unit had, ruled by ZT-proxy (supervisor, overnight ): RULING A.**
/// `mrd read --json` already carried `"source"` before G1, so G1 never added a field — it only
/// voiced the human face. `mrd sql --json` has no such field, and minting one would be a new
/// surface on a VERSIONED document with existing consumers: architecture, which this unit has
/// PARKED. So the machine face gets the same stderr voice and its OD9 stdout stays
/// byte-for-byte what it was. A structural `source` field, if it is ever wanted, is a
/// deliberate OD9 amendment.
///
///
///
///
///
///
///
///
///
///
///
#[test]
fn g13_json_face_is_voiced_on_stderr_with_the_od9_document_unchanged() {
    let sb = sandbox();
    let ws = sb.workspace();

    let warm = sb.run(&ws, &["sql", GOOD_QUERY, "--json"]);
    let pid = sb.reap();
    let degraded = sb.run_degraded(&ws, &["sql", GOOD_QUERY, "--json"]);

    assert!(pid.is_some(), "the auto-spawned daemon wrote a pidfile");
    assert!(
        stderr(&warm).is_empty(),
        "arm A must really be warm and silent: {:?}",
        stderr(&warm)
    );
    assert_eq!(warm.status.code(), degraded.status.code(), "same exit code");

    let parse = |out: &Output| -> serde_json::Value {
        serde_json::from_slice(&out.stdout).expect("stdout is a parseable OD9 document")
    };
    let (mut warm_doc, mut degraded_doc) = (parse(&warm), parse(&degraded));
    assert!(
        warm_doc.get("changes_seq").is_some(),
        "the warm arm really carries the daemon's epoch: {warm_doc}"
    );
    assert!(
        degraded_doc.get("changes_seq").is_none(),
        "OD9 omits the epoch on a daemonless path — pre-existing, not this \
         unit's doing: {degraded_doc}"
    );
    for doc in [&mut warm_doc, &mut degraded_doc] {
        if let Some(obj) = doc.as_object_mut() {
            obj.remove("changes_seq");
        }
    }
    assert_eq!(
        warm_doc, degraded_doc,
        "modulo OD9's own `changes_seq` omission, this unit added NO delta to \
         the machine face"
    );
    assert!(
        stderr(&degraded).contains(SOURCE_PHRASE),
        "the machine face still voices the degrade, on stderr: {:?}",
        stderr(&degraded)
    );
    assert!(
        degraded_doc.get("source").is_none(),
        "RULING A: no `source` field is minted on the OD9 schema: {degraded_doc}"
    );
}

// Gate 5: the named cause — a socket path no daemon can bind.
//
//

/// The exact hazard G1 and G13 were both found through: an `XDG_CACHE_HOME` long enough that
/// `<cache>/meridian/registry/daemon.sock` exceeds `sun_path`. No daemon can bind it and none
/// can be dialled, so starting one is not the fix — and the voice must say which fix is. This
/// is the 248× case.
#[test]
fn g13_over_long_socket_path_names_the_sun_path_limit() {
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

    let out = sb.run(&ws, &["sql", GOOD_QUERY]);
    assert_eq!(out.status.code(), Some(0), "sql exits 0: {}", stderr(&out));
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
