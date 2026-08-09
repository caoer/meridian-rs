//! The read face carries the engine's TEACHING refusal — path, message,
//! recovery — on both planes. The daemon's per-file `invalid_utf8` frame
//! (registry `doc_or_refusal`) names the unserved member, its condition, and
//! where its bytes stand; wire-contract §8 binds `invalid_utf8{path,message}`.
//! A face that discards that frame and remints the bare code token strands the
//! operator: `mrd: invalid_utf8` says neither WHICH file nor WHAT to do
//! (cross-team finding, grok G-P1-1 / opus P2-1).
//!
//! Two planes under gate:
//! - warm: the daemon ships the typed refusal; the CLI must surface it
//!   verbatim, never degrade past it and remint.
//! - degrade: the in-process `wire_serve::load_doc` mint must itself carry
//!   path + message per §8, not the bare code.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

const GUIDE: &str = "# Guide\n\n## Usage\n\nA healthy page beside the poison.\n";

/// The three facts the refusal owes the operator: WHICH member, WHAT is wrong
/// with it, and WHERE its bytes stand (the §8 env-class teaching — the file
/// needs fixing, but nothing was lost).
const PATH_PHRASE: &str = "poison.md";
const CONDITION_PHRASE: &str = "not UTF-8";
const RECOVERY_PHRASE: &str = "bytes stay under the root";

struct Sandbox {
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
    cache_root: PathBuf,
}

fn sandbox() -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_home = tmp.path().join("xdg-cache");
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
    fn run_warm(&self, cwd: &Path, args: &[&str]) -> Output {
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

    /// An anchored workspace holding a healthy page and a poison member: a
    /// valid markdown head so the fixture is unmistakably a page, then the
    /// byte no UTF-8 decode admits.
    fn poisoned_workspace(&self) -> PathBuf {
        let ws = self.tmp.path().join("project");
        std::fs::create_dir_all(ws.join(".git")).expect("git anchor");
        std::fs::write(ws.join("guide.md"), GUIDE).expect("guide");
        let mut bytes = b"# P\n\n## Body\n\nprose the read never serves\n".to_vec();
        bytes.extend_from_slice(b"\xFF\n");
        std::fs::write(ws.join("poison.md"), bytes).expect("poison member");
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

/// Poll until `pid` is gone, so a failed assertion never leaks the daemon.
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
fn code(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}

fn assert_teaching_refusal(out: &Output, plane: &str) {
    assert_eq!(
        code(out),
        1,
        "{plane}: a poison-member read is a findings refusal (exit 1): {}",
        stderr(out)
    );
    let err = stderr(out);
    for phrase in [PATH_PHRASE, CONDITION_PHRASE, RECOVERY_PHRASE] {
        assert!(
            err.contains(phrase),
            "{plane}: the refusal must carry {phrase:?} (path + message + recovery, \
             wire-contract §8 `invalid_utf8{{path,message}}`) — a bare code token \
             strands the operator. stderr was: {err:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Gate 1 — degrade plane: the in-process mint itself teaches.
// ---------------------------------------------------------------------------

#[test]
fn degraded_read_of_a_poison_member_teaches_path_message_recovery() {
    let sb = sandbox();
    let ws = sb.poisoned_workspace();
    let out = sb.run_degraded(&ws, &["read", "poison.md"]);
    assert_teaching_refusal(&out, "degrade");
}

/// The links face, same law (defect-ledger RES-B): a DIRECT poison-path
/// `links` query surfaces the typed per-file `invalid_utf8` — code, path,
/// condition, recovery — never a `file_not_found` miss for a member that
/// exists on disk. Spawn-impossible so the answer is deterministically the
/// in-process degrade (the leg the ledger names).
#[test]
fn degraded_links_of_a_poison_member_answers_typed_invalid_utf8() {
    let sb = sandbox();
    let ws = sb.poisoned_workspace();
    let out = sb.run_degraded(&ws, &["links", "poison.md"]);
    assert_ne!(code(&out), 0, "a poison-member links query refuses");
    let err = stderr(&out);
    assert!(
        err.contains("invalid_utf8"),
        "the refusal wears its typed code: {err:?}"
    );
    assert!(
        !err.contains("file_not_found"),
        "a member that exists on disk is never a miss: {err:?}"
    );
    for phrase in [PATH_PHRASE, CONDITION_PHRASE, RECOVERY_PHRASE] {
        assert!(
            err.contains(phrase),
            "the links refusal teaches like the read doors — {phrase:?} \
             (wire-contract §8 `invalid_utf8{{path,message}}`): {err:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Gate 2 — warm plane: the daemon's typed refusal surfaces VERBATIM.
// ---------------------------------------------------------------------------

#[test]
fn warm_read_of_a_poison_member_surfaces_the_daemons_teaching_frame() {
    let sb = sandbox();
    let ws = sb.poisoned_workspace();

    // Prove the daemon arm really is warm FIRST (g1's control law): without
    // this, a spawn-broken environment turns the gate into a second degrade
    // test and quietly voids the warm control.
    let warm_json = sb.run_warm(&ws, &["read", "guide.md", "--json"]);
    let poison = sb.run_warm(&ws, &["read", "poison.md"]);

    // Reap the auto-spawned daemon BEFORE asserting, so a failed assertion
    // never leaks it — it is detached, so it is signalled by its own pidfile.
    let pid = sb.wait_daemon_pid(Duration::from_secs(5));
    if let Some(pid) = pid {
        signal(pid, libc::SIGTERM);
        wait_dead(pid, Duration::from_secs(5));
    }

    assert!(pid.is_some(), "the auto-spawned daemon wrote a pidfile");
    assert!(
        stdout(&warm_json).contains("\"source\": \"daemon\""),
        "the control read must really be daemon-backed: {}",
        stdout(&warm_json)
    );
    assert_teaching_refusal(&poison, "warm");
    // The daemon's condition carries the decode detail (`fs::build_corpus`:
    // "is not UTF-8 (invalid utf-8 sequence …)"). Its presence proves the
    // surfaced frame IS the daemon's — a degrade remint does not know the
    // byte index, so this phrase cannot come from the fallthrough path.
    assert!(
        stderr(&poison).contains("invalid utf-8 sequence"),
        "the warm refusal is the daemon's frame verbatim, not a remint: {:?}",
        stderr(&poison)
    );
}

// ---------------------------------------------------------------------------
// Control — the general mechanism: other refusal codes keep their teaching
// across the warm path (no drift against the in-process remint).
// ---------------------------------------------------------------------------

#[test]
fn warm_read_of_a_missing_file_names_the_path() {
    let sb = sandbox();
    let ws = sb.poisoned_workspace();

    let warm_json = sb.run_warm(&ws, &["read", "guide.md", "--json"]);
    let missing = sb.run_warm(&ws, &["read", "missing.md"]);

    let pid = sb.wait_daemon_pid(Duration::from_secs(5));
    if let Some(pid) = pid {
        signal(pid, libc::SIGTERM);
        wait_dead(pid, Duration::from_secs(5));
    }

    assert!(pid.is_some(), "the auto-spawned daemon wrote a pidfile");
    assert!(
        stdout(&warm_json).contains("\"source\": \"daemon\""),
        "the control read must really be daemon-backed: {}",
        stdout(&warm_json)
    );
    assert_eq!(code(&missing), 1, "file_not_found is exit 1");
    let err = stderr(&missing);
    assert!(
        err.contains("file_not_found") && err.contains("missing.md"),
        "the refusal names its code and the path it echoes: {err:?}"
    );
}
