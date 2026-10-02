//! The run plane's delta-mint bracket folds no corpus — ALONE in its own test
//! binary because `fs::fold_count()` is process-global: one test per file is
//! one process, so the counter is this measurement's own (the in-process
//! `RunningServer` shares the process, which is what lets it see the daemon's
//! folds).
//!
//! The bracket holds `.meridian/write.lock`; every corpus fold inside it was a
//! hold every other writer waited out. It observes `root_before` through the
//! resident memo and takes `root_after` from the commit's own overlay
//! (wire-contract § A.8), so a committing fire — which reads the pinned world
//! — folds nothing, and a committing task run folds once: its root-at-eval
//! observation, before the bracket opens (`run-plane.md` § Timing phases).

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

/// A hook block whose fire edits its own page, and a task that does the same.
const HOOKS: &str = "\
---
caps: md.edit
task.stamp-task: \"[[#^task-1]]\"
task.stamp-task.caps: md.edit
task.stamp-task.args: value
---

# Hooks

```starlark
def run(event):
    set_field(field = \"seen\", value = event[\"name\"])
    return None

declare(on = \"Stop\")
```
^stamp

```starlark
def run(ctx):
    set_field(field = \"status\", value = ctx.args[0])
```
^task-1
";

fn seeded(tmp: &TempDir) -> PathBuf {
    let ws = tmp.path().join("project");
    fs::create_dir_all(&ws).unwrap();
    fs::write(ws.join("HOOKS.md"), HOOKS).unwrap();
    ws
}

fn fire(conn: &mut Conn, id: u64, name: &str) -> Value {
    let resp = conn.call(&json!({
        "id": id, "op": "run", "invocation": format!("fire-{id}"),
        "targets": [{"page": "HOOKS.md", "block": "stamp", "mode": "fire", "input": {"name": name}}],
    }));
    assert_eq!(resp["ok"], json!(true), "{resp}");
    resp["body"]["targets"][0].clone()
}

#[test]
fn a_committing_run_folds_no_corpus_inside_its_bracket() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    let hello = conn.call(&json!({
        "op": "hello", "proto": 1, "contract": "v3", "workspace": ws.to_str().unwrap(),
    }));
    assert_eq!(hello["ok"], json!(true), "{hello}");

    // Warm-up: the first fire pins a world, which builds the resident engine.
    let row = fire(&mut conn, 1, "Warm");
    assert_eq!(row["result"], json!("ok"), "{row}");

    let before = ::fs::fold_count();
    let row = fire(&mut conn, 2, "Second");
    let after = ::fs::fold_count();
    assert_eq!(row["result"], json!("ok"), "{row}");
    assert!(
        fs::read_to_string(ws.join("HOOKS.md"))
            .unwrap()
            .contains("seen: Second"),
        "the fire committed"
    );
    assert_eq!(
        after - before,
        0,
        "a committing fire folds no corpus: the bracket observes through the memo"
    );

    let before = ::fs::fold_count();
    let resp = conn.call(&json!({
        "id": 3, "op": "run", "invocation": "task-3",
        "targets": [{"page": "HOOKS.md", "task": "stamp-task", "args": ["done"]}],
    }));
    let after = ::fs::fold_count();
    assert_eq!(
        resp["body"]["targets"][0]["state"],
        json!("applied"),
        "{resp}"
    );
    assert_eq!(
        after - before,
        1,
        "a committing task run folds once — its root-at-eval observation, before \
         the bracket — and nothing inside it"
    );
    server.shutdown();
}
