//! The § A.7 entry-world cost instrument — ALONE in its own test binary on
//! purpose: `fs::fold_count()` is process-global, and the sibling gates in
//! `script_op.rs` run splices in parallel threads of one process, each commit
//! folding the domain. One test per file = one process = a counter this
//! measurement owns exclusively (the in-process `RunningServer` shares the
//! process, which is exactly what lets the instrument see the daemon's own
//! folds).

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use registry::{Config, RunningServer};
use serde_json::{Value, json};
use tempfile::TempDir;

mod common;

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
    config.idle_exit = None;
    config.drain_cold_builds = Duration::from_secs(30);
    config
}

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

    fn call(&mut self, request: &Value) -> Value {
        common::honour_retry(|| {
            let mut line = serde_json::to_string(request).unwrap();
            line.push('\n');
            self.writer.write_all(line.as_bytes()).unwrap();
            self.writer.flush().unwrap();
            let mut response = String::new();
            self.reader.read_line(&mut response).unwrap();
            serde_json::from_str(&response).unwrap()
        })
    }
}

const DOC: &str =
    "---\nstatus: open\ntitle: Alpha\n---\n# Alpha\n\none two three\n\n## Beta\n\nfour five\n";

fn seeded(tmp: &TempDir) -> PathBuf {
    let ws = tmp.path().join("project");
    for (rel, content) in [("doc.md", DOC), ("logs/receipts.md", "# Receipts\n")] {
        let path = ws.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }
    ws
}

/// Reads serve from the pinned entry state at memory speed: a multi-read
/// program on a warm engine performs ZERO byte-folds — the strongest
/// instrument the tree has for "no per-read currency pass".
#[test]
fn reads_serve_from_the_entry_world_with_zero_byte_folds() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.call(&json!({
        "op": "hello", "proto": 1, "contract": "v3",
        "workspace": ws.to_str().unwrap(),
    })); // binds at config cost (§3.2)
    // The first read pays the warm — done here so the program below runs on a
    // warm engine, which is the property under test.
    conn.call(&json!({"op": "toc", "path": "doc.md"}));

    let before = fs_fold_count();
    let resp = conn.call(&json!({
        "id": 8,
        "op": "script",
        "source": "a = read(\"doc.md\")\nb = read(\"doc.md\", section=\"Alpha/Beta\")\nc = read(\"doc.md\")\nd = read(\"logs/receipts.md\")\ne = read(\"doc.md\", section=\"Alpha\")\n",
    }));
    let trace = resp["body"].clone();
    assert_eq!(
        resp["ok"],
        json!(true),
        "the script op answers a trace: {resp}"
    );
    assert_eq!(trace["outcome"], json!("no_effect"), "trace: {trace}");
    assert_eq!(trace["telemetry"]["reads_used"], json!(5));
    let after = fs_fold_count();
    assert_eq!(
        after - before,
        0,
        "a read-only program folds no domain bytes: the pass is at entry, \
         served from the memoized world, and reads are in-process"
    );
}

/// The in-process server shares this test process, so the fs fold instrument
/// reads the daemon's own counter. `::fs` is the engine crate (a declared
/// dependency of `registry`), not `std::fs`.
fn fs_fold_count() -> u64 {
    ::fs::fold_count()
}
