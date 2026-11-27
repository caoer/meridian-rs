//! **`mrd daemon --successor` replaces a serving daemon without a window**
//! (`docs/wire-contract.md` §A.3, Succession; `docs/status.md`, `mrd daemon`).
//!
//! The gates run the real binary, as whoever installs a new one does: a
//! serving `mrd daemon`, then `mrd daemon --successor` beside it. The
//! successor's stdout is the line `serving` and nothing else, the daemon it
//! replaced exits 0 by itself, the pidfile names the successor, and a
//! connection opened before the swap is answered after it with no
//! `corpus_warming`.

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

mod common;

/// How long a daemon may take to bind, warm, hand over or exit.
const BUDGET: Duration = Duration::from_secs(60);

struct Sandbox {
    _tmp: tempfile::TempDir,
    home: PathBuf,
    cache_home: PathBuf,
    ws: PathBuf,
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        common::reap_daemon(&self.home, &self.cache_home);
    }
}

impl Sandbox {
    fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path().join("home");
        let cache_home = tmp.path().join("xdg-cache");
        let ws = tmp.path().join("project");
        std::fs::create_dir_all(&home).expect("home");
        std::fs::create_dir_all(ws.join(".git")).expect("git anchor");
        std::fs::write(ws.join("a.md"), "# Alpha\n\nsee [[beta]]\n").expect("doc");
        let ws = std::fs::canonicalize(&ws).expect("canonical ws");
        Self {
            _tmp: tmp,
            home,
            cache_home,
            ws,
        }
    }

    fn socket(&self) -> PathBuf {
        common::child_socket_path(&self.home, &self.cache_home)
    }

    /// The pid the pidfile names.
    fn pidfile(&self) -> u32 {
        let path = common::child_daemon_pidfile(&self.home, &self.cache_home);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("no pidfile at {}: {e}", path.display()))
            .trim()
            .parse()
            .expect("a pid")
    }

    /// `mrd daemon [--successor]` in the foreground, stdout piped.
    fn daemon(&self, successor: bool) -> Daemon {
        let mut command = common::mrd_command(&self.home, &self.cache_home);
        command.arg("daemon");
        if successor {
            command.arg("--successor");
        }
        let mut child = command
            .env_remove("MRD_TIMING")
            .env_remove("MERIDIAN_WORKSPACE")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("mrd daemon spawns");
        let stdout = child.stdout.take().expect("piped stdout");
        let (tx, first_line) = mpsc::channel();
        let rest = thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            let _ = reader.read_line(&mut line);
            let _ = tx.send(line);
            let mut rest = String::new();
            let _ = reader.read_to_string(&mut rest);
            rest
        });
        Daemon {
            child,
            first_line,
            rest: Some(rest),
        }
    }

    /// A connection bound to the workspace.
    fn dial(&self) -> UnixStream {
        let stream = UnixStream::connect(self.socket()).expect("the socket accepts");
        // A starved read fails loud instead of hanging the gate.
        stream.set_read_timeout(Some(BUDGET)).expect("timeout");
        let ack = ask(
            &stream,
            &json!({"op": "hello", "proto": 1, "contract": "v3",
                    "workspace": self.ws.to_str().expect("utf-8 ws")}),
        );
        assert_eq!(ack["ok"], json!(true), "hello: {ack}");
        stream
    }
}

/// One daemon process of this test and its stdout. Dropping it ends the
/// process, so a failed gate leaves none behind.
struct Daemon {
    child: Child,
    first_line: mpsc::Receiver<String>,
    rest: Option<thread::JoinHandle<String>>,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Daemon {
    /// The first line of stdout; empty when the process ended without one.
    fn first_line(&self) -> String {
        self.first_line
            .recv_timeout(BUDGET)
            .expect("a first stdout line, or the end of stdout")
    }

    /// The exit status, once the process ends by itself.
    fn exit(&mut self) -> ExitStatus {
        let started = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().expect("try_wait") {
                return status;
            }
            assert!(started.elapsed() < BUDGET, "the daemon never exited");
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// SIGTERM, then the exit status and everything stdout carried after its
    /// first line.
    fn stop(&mut self) -> (ExitStatus, String) {
        let pid = libc::pid_t::try_from(self.child.id()).expect("pid fits");
        // SAFETY: kill(2) on our own child, still unreaped.
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
        let status = self.exit();
        let rest = self.rest.take().expect("stopped once");
        (status, rest.join().expect("stdout reader"))
    }
}

/// The socket accepts, or the gate fails.
fn wait_bound(socket: &Path) {
    let started = Instant::now();
    while UnixStream::connect(socket).is_err() {
        assert!(
            started.elapsed() < BUDGET,
            "the daemon never bound {}",
            socket.display()
        );
        thread::sleep(Duration::from_millis(20));
    }
}

/// One NDJSON request, one response, as the daemon gave it.
fn ask(stream: &UnixStream, request: &Value) -> Value {
    let mut line = request.to_string();
    line.push('\n');
    let mut writer = stream;
    writer
        .write_all(line.as_bytes())
        .expect("request reaches the daemon");
    let mut response = String::new();
    BufReader::new(stream.try_clone().expect("clone"))
        .read_line(&mut response)
        .expect("response line");
    serde_json::from_str(&response)
        .unwrap_or_else(|e| panic!("no JSON response ({e}): {response:?} to {request}"))
}

fn toc(stream: &UnixStream) -> Value {
    ask(stream, &json!({"op": "toc", "path": "a.md"}))
}

#[test]
fn a_successor_says_serving_and_the_daemon_it_replaced_exits_zero() {
    let sb = Sandbox::new();
    let mut old = sb.daemon(false);
    wait_bound(&sb.socket());
    assert_eq!(sb.pidfile(), old.child.id(), "the serving daemon's pidfile");

    // A connection opened before the swap, warm on the serving daemon.
    let held = sb.dial();
    let started = Instant::now();
    loop {
        let answer = toc(&held);
        if answer["ok"] == json!(true) {
            break;
        }
        assert_eq!(answer["error"]["code"], json!("corpus_warming"), "{answer}");
        assert!(started.elapsed() < BUDGET, "still warming: {answer}");
        thread::sleep(Duration::from_millis(20));
    }

    let mut new = sb.daemon(true);
    assert_eq!(new.first_line(), "serving\n", "the successor's one line");
    assert_eq!(
        sb.pidfile(),
        new.child.id(),
        "the pidfile names the successor"
    );

    // The replaced daemon exits by itself and says nothing on stdout.
    let status = old.exit();
    assert_eq!(status.code(), Some(0), "the replaced daemon: {status}");
    assert_eq!(old.first_line(), "", "the replaced daemon's stdout");

    // Its exit took nothing with it: one ask each, no retry through warming.
    let carried = toc(&held);
    assert_eq!(
        carried["ok"],
        json!(true),
        "the carried connection: {carried}"
    );
    let fresh = toc(&sb.dial());
    assert_eq!(fresh["ok"], json!(true), "a fresh dial: {fresh}");
    assert_eq!(
        sb.pidfile(),
        new.child.id(),
        "the pidfile after the old exit"
    );

    let (status, rest) = new.stop();
    assert_eq!(
        status.code(),
        Some(0),
        "a SIGTERM ends it cleanly: {status}"
    );
    assert_eq!(rest, "", "stdout carries nothing after `serving`");
}

#[test]
fn a_successor_with_no_daemon_serving_starts_as_the_daemon() {
    let sb = Sandbox::new();
    let mut only = sb.daemon(true);
    assert_eq!(only.first_line(), "serving\n");
    assert_eq!(sb.pidfile(), only.child.id());
    drop(sb.dial());

    let (status, rest) = only.stop();
    assert_eq!(
        status.code(),
        Some(0),
        "a SIGTERM ends it cleanly: {status}"
    );
    assert_eq!(rest, "");
}

/// Warm `ws_path` on the running daemon through a fresh connection.
fn warm_workspace(socket: &Path, ws_path: &Path) {
    let stream = UnixStream::connect(socket).expect("socket accepts");
    stream.set_read_timeout(Some(BUDGET)).expect("timeout");
    let ack = ask(
        &stream,
        &json!({
            "op": "hello",
            "proto": 1,
            "contract": "v3",
            "workspace": ws_path.to_str().expect("utf-8 ws"),
        }),
    );
    assert_eq!(ack["ok"], json!(true), "hello failed: {ack}");
    let started = Instant::now();
    loop {
        let answer = toc(&stream);
        if answer["ok"] == json!(true) {
            return;
        }
        assert_eq!(answer["error"]["code"], json!("corpus_warming"), "{answer}");
        assert!(started.elapsed() < BUDGET, "still warming: {answer}");
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_successor_takes_over_after_a_warmed_workspace_was_unregistered_and_deleted() {
    let sb = Sandbox::new();
    let mut old = sb.daemon(false);
    wait_bound(&sb.socket());

    let held = sb.dial();
    let started = Instant::now();
    loop {
        let answer = toc(&held);
        if answer["ok"] == json!(true) {
            break;
        }
        assert_eq!(answer["error"]["code"], json!("corpus_warming"), "{answer}");
        assert!(started.elapsed() < BUDGET, "still warming: {answer}");
        thread::sleep(Duration::from_millis(20));
    }

    // Reproduce `bin/rule-edit`: warm a temp jail, unregister it, delete it.
    let jail = sb._tmp.path().join("rule-edit-jail");
    std::fs::create_dir_all(jail.join(".git")).expect("jail git");
    std::fs::write(jail.join("a.md"), "# Jail\n").expect("jail doc");
    let jail = std::fs::canonicalize(&jail).expect("canonical jail");
    warm_workspace(&sb.socket(), &jail);

    let unreg = common::mrd_command(&sb.home, &sb.cache_home)
        .env("MERIDIAN_WORKSPACE", &jail)
        .args(["unregister", jail.to_str().expect("utf-8 jail")])
        .output()
        .expect("mrd unregister runs");
    assert!(
        unreg.status.success(),
        "mrd unregister failed: {}",
        String::from_utf8_lossy(&unreg.stderr)
    );
    std::fs::remove_dir_all(&jail).expect("delete jail");

    let mut new = sb.daemon(true);
    assert_eq!(
        new.first_line(),
        "serving\n",
        "the successor takes over warm"
    );
    assert_eq!(old.exit().code(), Some(0));
    assert_eq!(toc(&held)["ok"], json!(true), "carried conn stays warm");

    let (status, rest) = new.stop();
    assert_eq!(status.code(), Some(0));
    assert_eq!(rest, "");
}

#[test]
fn a_successor_takes_over_when_a_warmed_workspace_vanished_without_unregister() {
    let sb = Sandbox::new();
    let mut old = sb.daemon(false);
    wait_bound(&sb.socket());

    let held = sb.dial();
    let started = Instant::now();
    loop {
        let answer = toc(&held);
        if answer["ok"] == json!(true) {
            break;
        }
        assert_eq!(answer["error"]["code"], json!("corpus_warming"), "{answer}");
        assert!(started.elapsed() < BUDGET, "still warming: {answer}");
        thread::sleep(Duration::from_millis(20));
    }

    let jail = sb._tmp.path().join("vanished-no-unreg");
    std::fs::create_dir_all(jail.join(".git")).expect("jail git");
    std::fs::write(jail.join("a.md"), "# Gone\n").expect("jail doc");
    let jail = std::fs::canonicalize(&jail).expect("canonical jail");
    warm_workspace(&sb.socket(), &jail);
    std::fs::remove_dir_all(&jail).expect("delete jail without unregister");

    let mut new = sb.daemon(true);
    assert_eq!(
        new.first_line(),
        "serving\n",
        "the successor takes over warm"
    );
    assert_eq!(old.exit().code(), Some(0));
    assert_eq!(toc(&held)["ok"], json!(true), "carried conn stays warm");

    let (status, rest) = new.stop();
    assert_eq!(status.code(), Some(0));
    assert_eq!(rest, "");
}
