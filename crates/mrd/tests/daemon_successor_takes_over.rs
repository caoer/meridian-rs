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
    tmp: tempfile::TempDir,
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
            tmp,
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
        self.daemon_with_residency(successor, None)
    }

    /// `mrd daemon [--successor]` with an optional `MRD_RESIDENCY_HOST_BYTES`.
    fn daemon_with_residency(&self, successor: bool, residency_host_bytes: Option<u64>) -> Daemon {
        let mut command = common::mrd_command(&self.home, &self.cache_home);
        command.arg("daemon");
        if successor {
            command.arg("--successor");
        }
        if let Some(bytes) = residency_host_bytes {
            command.env(registry::RESIDENCY_HOST_BYTES_ENV, bytes.to_string());
        } else {
            command.env_remove(registry::RESIDENCY_HOST_BYTES_ENV);
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

    fn backing_dir(&self) -> PathBuf {
        self.cache_home
            .join("meridian")
            .join("registry")
            .join("backing")
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
    let jail = sb.tmp.path().join("rule-edit-jail");
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

    let jail = sb.tmp.path().join("vanished-no-unreg");
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

/// Names of `raw-*` store directories currently under `backing_dir`, sorted.
fn backing_stores(backing_dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(backing_dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_owned();
            name.starts_with("raw-").then_some(name)
        })
        .collect();
    names.sort();
    names
}

/// Wait until a `raw-<pid>-*` store directory appears under `backing_dir` with
/// at least one `.raw` segment file written inside it.
fn wait_for_pid_backing(backing_dir: &Path, pid: u32) -> String {
    let prefix = format!("raw-{pid}-");
    let started = Instant::now();
    loop {
        if let Some(name) = backing_stores(backing_dir)
            .into_iter()
            .find(|n| n.starts_with(&prefix))
        {
            let dir = backing_dir.join(&name);
            let has_segment = std::fs::read_dir(&dir).is_ok_and(|entries| {
                entries.flatten().any(|e| {
                    e.path()
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("raw"))
                })
            });
            if has_segment {
                return name;
            }
        }
        assert!(
            started.elapsed() < BUDGET,
            "backing store for pid {pid} with segments never appeared in {} (saw {:?})",
            backing_dir.display(),
            backing_stores(backing_dir),
        );
        thread::sleep(Duration::from_millis(20));
    }
}

/// Hold a partial request line open on `socket` so the resident daemon answers
/// `busy` to handover asks instead of committing succession.
fn stall_resident(sb: &Sandbox) -> UnixStream {
    let mut stalled = sb.dial();
    stalled
        .write_all(br#"{"op": "toc", "#)
        .expect("partial request writes");
    // Longer than the daemon's connection park tick so the resident sees an
    // in-flight request when handover drains.
    thread::sleep(Duration::from_millis(300));
    stalled
}

#[test]
fn a_stopped_successor_removes_its_backing_store_and_leaves_the_serving_ones_alone() {
    let sb = Sandbox::new();
    let budget = 1 << 30;
    let mut old = sb.daemon_with_residency(false, Some(budget));
    wait_bound(&sb.socket());
    warm_workspace(&sb.socket(), &sb.ws);
    let old_store = wait_for_pid_backing(&sb.backing_dir(), old.child.id());

    // Keep the resident busy so the successor warms its own backing store and
    // stays pre-commit until SIGTERM arrives (the nyc-1 engine-swap shape).
    let mut stalled = stall_resident(&sb);

    let mut succ = sb.daemon_with_residency(true, Some(budget));
    let succ_store = wait_for_pid_backing(&sb.backing_dir(), succ.child.id());
    assert_ne!(succ_store, old_store);

    // Stop the successor with SIGTERM as `ccc-statusd` does on swap timeout.
    let stop_started = Instant::now();
    let (status, rest) = succ.stop();
    assert!(
        stop_started.elapsed() < Duration::from_secs(5),
        "successor took {:?} to exit on SIGTERM (past 5 s reap bound)",
        stop_started.elapsed()
    );
    assert!(
        !status.success(),
        "stopped pre-commit successor must exit non-zero: {status}"
    );
    assert_eq!(
        succ.first_line(),
        "",
        "stopped successor must not print serving"
    );
    assert_eq!(rest, "");

    // Immediately after the successor exits, its backing store must be gone
    // and the serving daemon's store must be untouched.
    assert_eq!(
        backing_stores(&sb.backing_dir()),
        vec![old_store.clone()],
        "stopped successor left a backing store behind (or touched the serving one)"
    );

    // Finish the stalled request and confirm the serving daemon still serves.
    stalled
        .write_all(b"\"path\": \"a.md\"}\n")
        .expect("complete stalled request");
    let mut line = String::new();
    BufReader::new(stalled.try_clone().expect("clone"))
        .read_line(&mut line)
        .expect("stalled response");
    let answer: Value = serde_json::from_str(&line).expect("json");
    assert_eq!(answer["ok"], json!(true), "resident still serves: {answer}");
    assert_eq!(
        toc(&sb.dial())["ok"],
        json!(true),
        "fresh dial still serves"
    );
    assert_eq!(backing_stores(&sb.backing_dir()), vec![old_store]);

    let (status, _) = old.stop();
    assert_eq!(status.code(), Some(0));
}

#[test]
fn a_sigkilled_successors_backing_store_is_swept_by_the_next_successor() {
    let sb = Sandbox::new();
    let budget = 1 << 30;
    let mut old = sb.daemon_with_residency(false, Some(budget));
    wait_bound(&sb.socket());
    warm_workspace(&sb.socket(), &sb.ws);
    let old_store = wait_for_pid_backing(&sb.backing_dir(), old.child.id());

    let _stalled = stall_resident(&sb);

    // First successor warms, writes segments, then dies abruptly via SIGKILL.
    let mut first = sb.daemon_with_residency(true, Some(budget));
    let first_store = wait_for_pid_backing(&sb.backing_dir(), first.child.id());
    first.child.kill().expect("SIGKILL first successor");
    let _ = first.child.wait();
    assert!(
        sb.backing_dir().join(&first_store).is_dir(),
        "SIGKILL leaves the dead successor's store on disk before the sweep"
    );

    // A second successor starting beside the serving daemon sweeps the dead
    // predecessor's store while leaving the serving daemon's store intact.
    let mut second = sb.daemon_with_residency(true, Some(budget));
    let second_store = wait_for_pid_backing(&sb.backing_dir(), second.child.id());
    assert!(
        !sb.backing_dir().join(&first_store).exists(),
        "dead SIGKILLed successor's store was swept on next successor start"
    );
    assert!(
        sb.backing_dir().join(&old_store).is_dir(),
        "serving engine's store remained untouched"
    );

    // Stopping the second successor with SIGTERM leaves only the resident's.
    let (status, _) = second.stop();
    assert!(!status.success());
    assert!(!sb.backing_dir().join(&second_store).exists());
    assert_eq!(backing_stores(&sb.backing_dir()), vec![old_store]);

    let (status, _) = old.stop();
    assert_eq!(status.code(), Some(0));
}
