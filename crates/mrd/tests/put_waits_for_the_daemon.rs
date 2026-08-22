//! Card `cli-face-put-false-negative` (session `21-22-compound-vnext`):
//! **a landed write must never report failure.**
//!
//! Measured 2026-08-21 by seat 547853b4 on the sessions root: every `mrd put`
//! that tripped a slow middleware (two `ctx.sql` calls over a 20k-file
//! corpus) printed `the daemon did not answer the write: Resource temporarily
//! unavailable (os error 35)` at 7 s and exited 1 — and the bytes were on
//! disk seconds later. `os error 35` is `EAGAIN`: the socket READ timeout the
//! write door inherited from the script host's wall clock
//! (`SocketDoor::connect` → `WALL_CLOCK`), not a disk error. The client gave
//! up on the daemon's answer while the daemon went on committing, and an
//! agent that believed the face either re-sent bytes the engine already held
//! or reported a landed write as failed.
//!
//! The law this file pins: once the write frame is on the wire, the CLI's
//! result is the daemon's answer. A slow answer is WAITED FOR — the wall
//! clock bounds the hello (a daemon that will not even greet is down, nothing
//! was sent), never the write's outcome. A connection that dies before the
//! answer is reported as what it is — outcome UNKNOWN, read before any
//! re-send — never as a failed write.
//!
//! Harness: a canned fake daemon at the derived socket path (identity-matched
//! so the 0025 skew law passes), the same shape `scoped_guards_client.rs`
//! drives. No real daemon: the delay under test is the daemon's to choose,
//! and a fake chooses it exactly.

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

mod common;

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

/// The identity this test's own compilation carries — the same crate stamp the
/// `mrd` binary under test bakes (one `build.rs` run feeds every target).
const OWN_BUILD: &str = env!("MRD_BUILD_SHA");

/// How long the fake daemon holds the splice before answering — past the
/// script host's 7 s wall clock (`crates/mrd/src/script/cmd.rs` `WALL_CLOCK`,
/// the socket read timeout the write door shares) by a margin no scheduler
/// jitter closes. The test measures the CLI from OUTSIDE: a write that
/// returns before this did not wait for its answer.
const HOLD: Duration = Duration::from_secs(9);

const DOC: &str = "# Alpha\n\none two three\n";
const EDIT: &str = r#"[{"target":{"hpath":[{"h":"Alpha"}]},"edit":{"match":{"old":"one two three","new":"one two three four"}}}]"#;

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

/// What the fake daemon does with the splice frame once it has read it.
#[derive(Clone, Copy)]
enum Splice {
    /// Hold the answer for [`HOLD`], then answer a commit.
    HoldThenCommit,
    /// Close the connection without answering — the daemon may or may not
    /// have committed; the client cannot know.
    Vanish,
}

impl Sandbox {
    fn socket(&self) -> PathBuf {
        common::child_socket_path(&self.home, &self.cache_home)
    }

    fn workspace(&self) -> PathBuf {
        let ws = self.tmp.path().join("project");
        std::fs::create_dir_all(ws.join(".git")).expect("git anchor");
        std::fs::write(ws.join("doc.md"), DOC).expect("doc");
        std::fs::canonicalize(&ws).expect("canonical ws")
    }

    /// A canned-frame fake daemon at the derived socket path: answers the
    /// control protocol and the hello at once, and treats the splice per
    /// `splice`.
    fn fake_daemon(&self, splice: Splice) {
        let socket = self.socket();
        std::fs::create_dir_all(socket.parent().expect("registry dir")).expect("mkdir");
        let listener = UnixListener::bind(&socket).expect("bind fake daemon socket");
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                std::thread::spawn(move || serve_connection(&stream, splice));
            }
        });
    }

    fn run(&self, cwd: &Path, args: &[&str], stdin: &str) -> Output {
        let mut child = Command::new(mrd_bin())
            .args(args)
            .current_dir(cwd)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env_remove("MERIDIAN_WORKSPACE")
            // No auto-spawn may hide a test bug: a dead socket must fail loud,
            // not quietly start a real daemon over the fake.
            .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn mrd");
        common::feed_stdin(&mut child, stdin.as_bytes());
        child.wait_with_output().expect("wait")
    }
}

/// One fake-daemon connection: NDJSON in, canned NDJSON out.
fn serve_connection(stream: &UnixStream, splice: Splice) {
    let mut writer = stream.try_clone().expect("clone fake stream");
    let reader = BufReader::new(stream.try_clone().expect("clone fake stream"));
    for line in reader.lines() {
        let Ok(line) = line else { return };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(frame) = serde_json::from_str::<Value>(&line) else {
            return;
        };
        let op = frame
            .get("op")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let answer = match op.as_str() {
            "ping" => json!({"status":"pong"}),
            "resolve_ws" => json!({"status":"miss"}),
            "hello" => json!({"ok":true,"body":{
                "proto":1,"server":"fake-daemon/0","contract":"v3",
                "caps": [],
                "identity": {"build": OWN_BUILD},
                "workspace": frame.get("workspace").cloned().unwrap_or(Value::Null)}}),
            "splice" => match splice {
                Splice::HoldThenCommit => {
                    std::thread::sleep(HOLD);
                    json!({"ok":true,"body":{
                        "path": frame.get("path").cloned().unwrap_or(Value::Null),
                        "fingerprint_before":"b3:before-token",
                        "fingerprint_after":"b3:after-token",
                        "file_rev_before":"b3:r1","file_rev_after":"b3:r2",
                        "armed":{"path":"doc.md","edits":[{"target":{"hpath":[{"h":"Alpha"}]}}]},
                        "verdicts":[]}})
                }
                Splice::Vanish => return,
            },
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

/// **The false negative, pinned.** The daemon holds the splice past the wall
/// clock and then commits. Before the fix the CLI returned at ~7 s with
/// `io_error … (os error 35)` and exit 1 while the daemon went on to commit;
/// now it waits for the answer and prints the commit it was given.
#[test]
fn a_write_the_daemon_answers_slowly_is_waited_for_and_reported_as_the_commit_it_is() {
    let sb = sandbox();
    let ws = sb.workspace();
    sb.fake_daemon(Splice::HoldThenCommit);

    let started = Instant::now();
    let out = sb.run(&ws, &["put", "doc.md", "--force", "--json"], EDIT);
    let elapsed = started.elapsed();
    let stdout = stdout_of(&out);
    let stderr = stderr_of(&out);

    assert!(
        elapsed >= HOLD,
        "the CLI returned after {elapsed:?}, before the daemon answered at {HOLD:?} — \
         it gave up on the write's outcome instead of waiting for it\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        out.status.success(),
        "a write the daemon committed must exit 0, got {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        out.status.code()
    );
    let frame: Value = serde_json::from_str(&stdout).expect("the --json face is one JSON frame");
    assert_eq!(
        frame.pointer("/put/file_rev_after").and_then(Value::as_str),
        Some("b3:r2"),
        "the commit the daemon answered is the commit the face prints:\n{stdout}"
    );
    assert!(
        !stderr.contains("os error 35") && !stderr.contains("did not answer"),
        "nothing on the face may read as a failed write:\n{stderr}"
    );
}

/// **The human face says the wait is a wait.** Same hold; no `--json`. The
/// caller watching a terminal learns at the wall clock that the write is in
/// flight — not failed, not to be re-sent — and then sees the commit.
#[test]
fn the_human_face_names_the_wait_and_then_the_commit() {
    let sb = sandbox();
    let ws = sb.workspace();
    sb.fake_daemon(Splice::HoldThenCommit);

    let out = sb.run(&ws, &["put", "doc.md", "--force"], EDIT);
    let stdout = stdout_of(&out);
    let stderr = stderr_of(&out);

    assert!(
        out.status.success(),
        "exit {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        out.status.code()
    );
    assert!(
        stderr.contains("still waiting") && stderr.contains("in flight"),
        "the wall-clock notice names an in-flight write being waited for:\n{stderr}"
    );
    assert!(
        !stderr.contains("os error 35") && !stderr.contains("did not answer"),
        "nothing on the face may read as a failed write:\n{stderr}"
    );
}

/// **A lost answer is not a failed write.** The daemon reads the splice frame
/// and drops the connection. The write may or may not have committed and the
/// client cannot know, so the face says UNKNOWN and routes through a read —
/// it never claims the write failed or that nothing was written.
#[test]
fn a_connection_that_dies_before_the_answer_reports_the_outcome_as_unknown() {
    let sb = sandbox();
    let ws = sb.workspace();
    sb.fake_daemon(Splice::Vanish);

    let out = sb.run(&ws, &["put", "doc.md", "--force", "--json"], EDIT);
    let stdout = stdout_of(&out);
    let stderr = stderr_of(&out);

    assert_eq!(
        out.status.code(),
        Some(1),
        "an unknown outcome is a refusal-class exit (1), never a clean 0 and never a tool fault (2)\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    let frame: Value = serde_json::from_str(&stdout).expect("the --json face is one JSON frame");
    let message = frame
        .pointer("/error/message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    for want in [
        "UNKNOWN",
        "may have committed",
        "Read the file back",
        "re-send",
    ] {
        assert!(
            message.contains(want),
            "{want:?} missing from the lost-answer face:\n{message}"
        );
    }
    for forbidden in [
        "did not answer",
        "nothing was written",
        "Nothing was written",
        "the write failed",
    ] {
        assert!(
            !message.contains(forbidden),
            "{forbidden:?} claims a fact nobody has after a lost answer:\n{message}"
        );
    }
}
