//! `docs/wire-contract.md` § A.10, the CLI ladder: **a daemon that accepted
//! the connection is the answer — `mrd walk` never builds the corpus in its
//! own process beside it.**
//!
//! Measured on devstation-nyc-1: the resident daemon was cold and answering
//! `corpus_warming`, every `mrd walk` took that refusal as licence to build
//! the workspace and every root its locks address in-process, and fifteen of
//! them at about 2 GiB each pushed the host into swap — which kept the daemon
//! from ever finishing the warm they were all waiting on.
//!
//! The same file pins the bound on the lane that is left, the workspace no
//! daemon serves: one in-process build at a time per cache root
//! (`docs/status.md` § The in-process build gate).
//!
//! Harness: a canned fake daemon at the derived socket path (identity-matched
//! so the 0025 skew law passes), the shape `put_waits_for_the_daemon.rs`
//! drives. No real daemon: what the daemon answers is the variable under test.

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

mod common;

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

/// The identity this test's own compilation carries — the same crate stamp the
/// `mrd` binary under test bakes (one `build.rs` run feeds every target).
const OWN_BUILD: &str = env!("MRD_BUILD_SHA");

/// The voice of the in-process lane (`engine::voice_degrade`): present exactly
/// when this process built the corpus itself.
const DEGRADE_VOICE: &str = "source: ephemeral";

struct Sandbox {
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
}

fn sandbox() -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_home = tmp.path().join("c");
    let home = tmp.path().join("h");
    std::fs::create_dir_all(&home).expect("home");
    Sandbox {
        tmp,
        cache_home,
        home,
    }
}

/// What the fake daemon does with a `walk` frame.
#[derive(Clone, Copy)]
enum Walk {
    /// Answer `corpus_warming` to the first `refusals` asks, then the listing.
    WarmThenServe { refusals: usize },
    /// Close the connection without answering.
    Vanish,
    /// Answer `file_not_found`.
    Miss,
}

impl Sandbox {
    fn socket(&self) -> PathBuf {
        common::child_socket_path(&self.home, &self.cache_home)
    }

    fn workspace(&self) -> PathBuf {
        let ws = self.tmp.path().join("project");
        std::fs::create_dir_all(ws.join(".git")).expect("git anchor");
        std::fs::write(ws.join("a.md"), "# Alpha\n\none\n").expect("doc");
        std::fs::canonicalize(&ws).expect("canonical ws")
    }

    /// A canned-frame fake daemon at the derived socket path. Returns the
    /// count of `walk` frames it has read.
    fn fake_daemon(&self, walk: Walk) -> Arc<AtomicUsize> {
        let socket = self.socket();
        std::fs::create_dir_all(socket.parent().expect("socket dir")).expect("mkdir");
        let listener = UnixListener::bind(&socket).expect("bind fake daemon socket");
        let asked = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&asked);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let counter = Arc::clone(&counter);
                std::thread::spawn(move || serve_connection(&stream, walk, &counter));
            }
        });
        asked
    }

    fn mrd(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut command = Command::new(mrd_bin());
        command
            .args(args)
            .current_dir(cwd)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env_remove("MERIDIAN_WORKSPACE")
            // No auto-spawn may hide a test bug: a dead socket must reach the
            // in-process lane, not quietly start a real daemon.
            .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        self.mrd(cwd, args).output().expect("run mrd")
    }
}

/// One fake-daemon connection: NDJSON in, canned NDJSON out.
fn serve_connection(stream: &UnixStream, walk: Walk, asked: &AtomicUsize) {
    let mut writer = stream.try_clone().expect("clone fake stream");
    let reader = BufReader::new(stream.try_clone().expect("clone fake stream"));
    for line in reader.lines() {
        let Ok(line) = line else { return };
        let Ok(frame) = serde_json::from_str::<Value>(&line) else {
            return;
        };
        let answer = match frame.get("op").and_then(Value::as_str).unwrap_or_default() {
            "ping" => json!({"status":"pong"}),
            "resolve_ws" => json!({"status":"miss"}),
            "hello" => json!({"ok":true,"body":{
                "proto":1,"server":"fake-daemon/0","contract":"v3",
                "caps": ["walk"],
                "identity": {"build": OWN_BUILD},
                "workspace": frame.get("workspace").cloned().unwrap_or(Value::Null)}}),
            "walk" => {
                let nth = asked.fetch_add(1, Ordering::SeqCst);
                match walk {
                    Walk::WarmThenServe { refusals } if nth < refusals => {
                        json!({"ok":false,"error":{
                            "code":"corpus_warming","recovery":"retry",
                            "message":"the drawer is still warming"}})
                    }
                    Walk::WarmThenServe { .. } => json!({"ok":true,"body":{
                        "direction":"up","page":"a.md","entries":[],
                        "revs_read":[{"path":"a.md","doc_rev":"b3:from-the-daemon"}]}}),
                    Walk::Vanish => return,
                    Walk::Miss => json!({"ok":false,"error":{
                        "code":"file_not_found","recovery":"fix_request",
                        "path":"missing.md","message":"file_not_found: no file"}}),
                }
            }
            _ => json!({"ok":false,"error":{
                "code":"bad_request","recovery":"fix_request",
                "message":"fake daemon: unexpected op"}}),
        };
        let mut out = serde_json::to_string(&answer).expect("encode");
        out.push('\n');
        if writer.write_all(out.as_bytes()).is_err() {
            return;
        }
    }
}

fn stdout_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}
fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// **The amplifier, pinned.** The daemon answers `corpus_warming` and then
/// serves. Before the fix the first refusal sent `walk` to the in-process
/// build; now it re-asks on the held connection and prints the daemon's
/// listing — the rev only the daemon could have supplied.
#[test]
fn a_warming_daemon_is_waited_on_and_serves_the_walk() {
    let sb = sandbox();
    let ws = sb.workspace();
    let asked = sb.fake_daemon(Walk::WarmThenServe { refusals: 3 });

    let out = sb.run(&ws, &["walk", "a.md"]);

    assert!(out.status.success(), "walk failed: {}", stderr_of(&out));
    assert!(
        stdout_of(&out).contains("b3:from-the-daemon"),
        "the listing is not the daemon's: {}",
        stdout_of(&out)
    );
    assert!(
        !stderr_of(&out).contains(DEGRADE_VOICE),
        "walk built the corpus in-process beside a warming daemon: {}",
        stderr_of(&out)
    );
    assert_eq!(asked.load(Ordering::SeqCst), 4, "three refusals, then the answer");
}

/// A daemon that accepts the walk and closes without answering holds the
/// corpus all the same: the walk refuses, exit 2, and builds nothing.
#[test]
fn a_daemon_that_drops_the_walk_refuses_and_builds_nothing() {
    let sb = sandbox();
    let ws = sb.workspace();
    sb.fake_daemon(Walk::Vanish);

    let out = sb.run(&ws, &["walk", "a.md"]);

    assert_eq!(out.status.code(), Some(2), "stderr: {}", stderr_of(&out));
    let stderr = stderr_of(&out);
    assert!(
        stderr.contains("did not complete it") && stderr.contains("not built in-process"),
        "the refusal does not name what happened: {stderr}"
    );
    assert!(!stderr.contains(DEGRADE_VOICE), "walk degraded: {stderr}");
    assert!(stdout_of(&out).is_empty(), "a refused walk printed a listing");
}

/// The daemon's `file_not_found` is the answer, in the words the in-process
/// lane speaks for the same miss — and no corpus is built to say them.
#[test]
fn a_missing_root_is_the_daemons_answer_in_the_in_process_words() {
    let sb = sandbox();
    let ws = sb.workspace();
    sb.fake_daemon(Walk::Miss);

    let out = sb.run(&ws, &["walk", "missing.md"]);

    assert_eq!(out.status.code(), Some(2), "stderr: {}", stderr_of(&out));
    let stderr = stderr_of(&out);
    assert!(
        stderr.contains("walk root not in the corpus: missing.md"),
        "the miss is not spoken in the walk's own words: {stderr}"
    );
    assert!(!stderr.contains(DEGRADE_VOICE), "walk degraded: {stderr}");
}

/// **The bound on the lane that is left.** No daemon serves the workspace, so
/// the walk builds in-process — and a second such build waits for the first
/// instead of holding a second corpus beside it.
#[test]
fn a_second_in_process_build_waits_for_the_first() {
    let sb = sandbox();
    let ws = sb.workspace();
    // The first build, stood in for by its hold on the gate.
    let gate = sb.socket().with_extension("build");
    std::fs::create_dir_all(&gate).expect("gate dir");
    let first = cache::DrawerLock::acquire(&gate).expect("hold the gate");

    let mut child = sb.mrd(&ws, &["walk", "a.md"]).spawn().expect("spawn mrd");
    std::thread::sleep(Duration::from_millis(1500));
    assert!(
        child.try_wait().expect("probe the child").is_none(),
        "the second build ran beside the first instead of waiting for the gate"
    );

    drop(first);
    let out = child.wait_with_output().expect("wait");
    assert!(out.status.success(), "walk failed: {}", stderr_of(&out));
    let stderr = stderr_of(&out);
    assert!(
        stderr.contains("waiting for it") && stderr.contains(DEGRADE_VOICE),
        "the wait was not voiced on the in-process lane: {stderr}"
    );
    assert!(stdout_of(&out).contains("a.md"), "no listing: {}", stdout_of(&out));
}
