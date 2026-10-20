//! **A daemon whose stderr refuses every write keeps serving** (card
//! `mrd-stderr-enospc-stops-serving`).
//!
//! Measured on workstation-nyc-2, 2026-10-10: the root disk filled, the
//! daemon's threads panicked `failed printing to stderr: No space left on
//! device`, and the daemon stopped serving — 13.5k reads and 340 `sql`
//! requests failed with broken pipes. `eprintln!` panics on a failed write;
//! the daemon's diagnostics now go through `timing::say!`, which drops the
//! line (`docs/status.md` § The daemon's own lane).
//!
//! Each gate runs the real `mrd daemon` binary with a dead stderr — a pipe
//! whose reader is gone (`EPIPE`, every unix) or `/dev/full` (`ENOSPC`,
//! Linux) — and drives it over its own socket, so no client-side degrade can
//! answer in its place. The daemon says "listening on" before it serves, so a
//! daemon that panics on a failed write dies before the first request.

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

mod common;

/// How long the daemon may take to bind, and a warming corpus to serve.
const BUDGET: Duration = Duration::from_secs(30);

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

    /// `mrd daemon` in the foreground, its stderr on `sink`.
    fn daemon(&self, sink: Stdio) -> Child {
        common::mrd_command(&self.home, &self.cache_home)
            .arg("daemon")
            .env_remove("MRD_TIMING")
            .env_remove("MERIDIAN_WORKSPACE")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(sink)
            .spawn()
            .expect("mrd daemon spawns")
    }
}

/// A connection is accepted, the child is alive, or the gate fails naming why.
fn connect(child: &mut Child, socket: &Path) -> UnixStream {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            panic!("the daemon died with its stderr dead ({status}) — it must outlive a log line");
        }
        if let Ok(stream) = UnixStream::connect(socket) {
            return stream;
        }
        assert!(
            started.elapsed() < BUDGET,
            "the daemon never bound {}",
            socket.display()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// One NDJSON request, one response; a `corpus_warming` refusal is re-asked.
fn call(stream: &UnixStream, request: &Value) -> Value {
    let started = Instant::now();
    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
    let mut writer = stream;
    loop {
        let mut line = request.to_string();
        line.push('\n');
        writer
            .write_all(line.as_bytes())
            .expect("request reaches the daemon");
        let mut response = String::new();
        reader.read_line(&mut response).expect("response line");
        let frame: Value = serde_json::from_str(&response)
            .unwrap_or_else(|e| panic!("no JSON response ({e}): {response:?} to {request}"));
        if frame["error"]["code"] != "corpus_warming" {
            return frame;
        }
        assert!(started.elapsed() < BUDGET, "still warming: {frame}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The gate: hello, a read and two `sql` queries all answer, the daemon is
/// still running after them, and a SIGTERM ends it cleanly.
fn keeps_serving(sink: Stdio) {
    let sb = Sandbox::new();
    let mut child = sb.daemon(sink);
    let stream = connect(&mut child, &sb.socket());

    let ack = call(
        &stream,
        &json!({"op": "hello", "proto": 1, "contract": "v3",
                "workspace": sb.ws.to_str().expect("utf-8 ws")}),
    );
    assert_eq!(ack["ok"], json!(true), "hello: {ack}");
    let toc = call(&stream, &json!({"op": "toc", "path": "a.md"}));
    assert_eq!(toc["ok"], json!(true), "toc: {toc}");
    for query in ["SELECT path FROM doc", "SELECT count(*) FROM doc"] {
        let sql = call(&stream, &json!({"id": 1, "op": "sql", "query": query}));
        assert_eq!(sql["ok"], json!(true), "sql {query}: {sql}");
    }
    drop(stream);

    assert!(
        child.try_wait().expect("try_wait").is_none(),
        "the daemon exited after serving"
    );
    let pid = libc::pid_t::try_from(child.id()).expect("pid fits");
    // SAFETY: kill(2) on our own child, still unreaped (try_wait saw it alive).
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }
    let status = child.wait().expect("wait");
    assert_eq!(
        status.code(),
        Some(0),
        "a SIGTERM ends it cleanly: {status}"
    );
}

#[test]
fn a_daemon_whose_stderr_is_a_closed_pipe_keeps_serving() {
    let (reader, writer) = std::io::pipe().expect("pipe");
    drop(reader);
    keeps_serving(Stdio::from(writer));
}

#[cfg(target_os = "linux")]
#[test]
fn a_daemon_whose_stderr_is_a_full_disk_keeps_serving() {
    let full = std::fs::File::options()
        .write(true)
        .open("/dev/full")
        .expect("/dev/full");
    keeps_serving(Stdio::from(full));
}
