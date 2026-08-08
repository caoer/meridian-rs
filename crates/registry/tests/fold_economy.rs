//! §5.3a fold economy, counted not timed — ALONE IN ITS OWN TEST BINARY.
//!
//! `fs::fold_count` / `fs::leaf_reread_count` are process-global counters, and
//! cargo runs a binary's tests on parallel threads: any sibling test's cold
//! `hello` warm folds the corpus once, so a sibling overlapping this test's
//! counter window moves the count and reds the assertion with the mechanism
//! innocent. Measured on the run-2 artifact (2026-08-08): 0/20 failures with
//! the test alone in the process, 3/20 at the binary's default parallelism,
//! delta 1–2 tracking how many warms overlapped. A dedicated integration-test
//! binary is a dedicated process, so this isolation cannot be lost by a new
//! test added next to it — serialization by construction, not by discipline.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use registry::{Config, RunningServer};
use serde_json::{Value, json};
use tempfile::TempDir;

/// A daemon config rooted under `tmp`, with reap horizons large enough that the
/// background reaper never evicts a warm engine mid-test.
// `Duration::from_hours` is not const-stable at MSRV 1.96; the seconds form is
// the workspace precedent.
#[allow(clippy::duration_suboptimal_units)]
fn test_config(tmp: &TempDir) -> Config {
    let forever = Duration::from_secs(365 * 24 * 60 * 60);
    let dir = tmp.path().join("registry");
    let mut config = Config::for_cache_root(tmp.path().join("cache"));
    config.socket_path = dir.join("daemon.sock");
    config.state_path = dir.join("state.json");
    config.idle_threshold = forever;
    config.reap_interval = forever;
    config.prewarm_interval = forever;
    config.prewarm_quiet_max = forever;
    // No idle exit: a daemon that reaped itself mid-assertion would flake.
    config.idle_exit = None;
    config
}

/// A workspace `tmp/ws` seeded with `files` — a sibling of the cache root, so
/// the corpus walk never sees the drawer.
fn write_ws(tmp: &TempDir, files: &[(&str, &str)]) -> PathBuf {
    let ws = tmp.path().join("ws");
    for (rel, content) in files {
        let path = ws.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }
    ws
}

/// A persistent NDJSON connection: `hello` binds the workspace, then the wire
/// read ops ride the same connection.
struct Conn {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Conn {
    fn open(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).unwrap();
        Conn {
            writer: stream.try_clone().unwrap(),
            reader: BufReader::new(stream),
        }
    }

    /// Send one request frame, read one response frame.
    fn call(&mut self, request: &Value) -> Value {
        let mut line = serde_json::to_string(request).unwrap();
        line.push('\n');
        self.writer.write_all(line.as_bytes()).unwrap();
        self.writer.flush().unwrap();
        let mut response = String::new();
        self.reader.read_line(&mut response).unwrap();
        serde_json::from_str(&response).unwrap()
    }

    /// The resident-engine handshake: bind `ws`, warming its engine, over v3.
    fn hello(&mut self, ws: &Path) -> Value {
        self.call(&json!({
            "op": "hello",
            "proto": 1,
            "contract": "v3",
            "workspace": ws.to_str().unwrap(),
        }))
    }
}

/// §5.3a fold economy, counted not timed: once the engine is warm, quiet
/// per-document serves run NO whole-corpus fold (`fs::fold_count`) and re-read
/// NO file bytes (`fs::leaf_reread_count`). At the pinned base every one of
/// these round trips folded the whole corpus — the O(domain bytes) per round
/// trip this wave removes.
#[test]
fn quiet_serves_fold_nothing_and_re_read_nothing() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(
        &tmp,
        &[("a.md", "# A\n\nsee [[b]]\n"), ("b.md", "# B\n"), ("c.md", "# C\n")],
    );
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    assert_eq!(conn.hello(&ws)["ok"], json!(true));

    // Warm is done; open the quiet window.
    let folds = ::fs::fold_count();
    let rereads = ::fs::leaf_reread_count();

    for _ in 0..3 {
        assert_eq!(conn.call(&json!({"op": "toc", "path": "a.md"}))["ok"], json!(true));
        assert_eq!(
            conn.call(&json!({"op": "cat", "path": "b.md"}))["ok"],
            json!(true)
        );
        assert_eq!(
            conn.call(&json!({"op": "read", "path": "c.md"}))["ok"],
            json!(true)
        );
        assert_eq!(conn.call(&json!({"op": "fingerprint"}))["ok"], json!(true));
    }

    assert_eq!(
        ::fs::fold_count() - folds,
        0,
        "a quiet window serves without one whole-corpus fold"
    );
    assert_eq!(
        ::fs::leaf_reread_count() - rereads,
        0,
        "a quiet window re-reads zero file bytes — corpus-grain legs are stat-grade"
    );

    server.shutdown();
}
