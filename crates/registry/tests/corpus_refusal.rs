//! Corpus-scoped refusals name their scope and their offending member
//! (Law A-3c, session 07-05 decisions/0002 gate f).
//!
//! The motivating incident: one poison member (non-UTF-8 bytes) landed in a
//! live corpus, and every read — of any file — refused with a bare
//! `invalid_utf8` the caller could only attribute to the file they asked for.
//! The engine was right to refuse the corpus (wire-contract §8: refuse, never
//! lossy-decode); the refusal was useless because it named no locus. These
//! gates pin the actionable shape: the frame carries the poison member's path
//! and a message naming corpus scope, member, and condition.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use registry::{Config, RunningServer};
use serde_json::{Value, json};
use tempfile::TempDir;

/// A daemon config rooted under `tmp`, reap horizons too large to interfere
/// (the `engine_rpc` precedent).
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
    config
}

/// A workspace `tmp/ws` seeded with `files` — a sibling of the cache root.
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
        let mut line = serde_json::to_string(request).unwrap();
        line.push('\n');
        self.writer.write_all(line.as_bytes()).unwrap();
        self.writer.flush().unwrap();
        let mut response = String::new();
        self.reader.read_line(&mut response).unwrap();
        serde_json::from_str(&response).unwrap()
    }

    fn hello(&mut self, ws: &Path) -> Value {
        self.call(&json!({
            "op": "hello",
            "proto": 1,
            "contract": "v3",
            "workspace": ws.to_str().unwrap(),
        }))
    }
}

/// The exemplar, reproduced: a healthy corpus binds and serves; then one
/// poison member lands through ordinary external activity; a read of a
/// HEALTHY member now refuses — and the refusal must name the corpus scope
/// and the poison member, not strand the caller with a bare `invalid_utf8`
/// they can only pin on the file they asked for.
#[test]
fn corpus_refusal_names_scope_and_poison_member() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("healthy.md", "# Healthy\n\nfine.\n")]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    let ack = conn.hello(&ws);
    assert_eq!(ack["ok"], json!(true), "healthy corpus binds: {ack}");
    let toc = conn.call(&json!({"op": "toc", "path": "healthy.md"}));
    assert_eq!(toc["ok"], json!(true), "healthy member serves: {toc}");

    // The poison lands behind the engine's back (the incident's shape:
    // a non-UTF-8 fixture copied into a live corpus).
    fs::create_dir_all(ws.join("notes")).unwrap();
    fs::write(ws.join("notes/poison.md"), b"# Poison\n\n\xff\xfe raw bytes\n").unwrap();

    let refusal = conn.call(&json!({"op": "toc", "path": "healthy.md"}));
    assert_eq!(refusal["ok"], json!(false), "poisoned corpus refuses: {refusal}");
    let error = &refusal["error"];
    assert_eq!(
        error["code"],
        json!("invalid_utf8"),
        "the condition keeps its closed-taxonomy code: {refusal}"
    );
    assert_eq!(
        error["recovery"],
        json!("env"),
        "invalid_utf8 stays env-class: {refusal}"
    );
    // Law A-3c, clause "member named": the frame's path is the POISON member,
    // never the healthy file the caller asked for.
    assert_eq!(
        error["path"],
        json!("notes/poison.md"),
        "the refusal names the offending member: {refusal}"
    );
    // Law A-3c, clause "scope named": the message states the corpus scope and
    // the condition, so the caller learns the file they asked for is fine.
    let message = error["message"].as_str().unwrap_or_else(|| {
        panic!("a corpus-scoped refusal carries a message: {refusal}")
    });
    assert!(
        message.contains("corpus") && message.contains("notes/poison.md") && message.contains("UTF-8"),
        "the message names scope, member, and condition: {refusal}"
    );

    server.shutdown();
}

/// The same law at the OTHER wire door: a `hello` that declares a poisoned
/// workspace refuses at bind, naming the member.
#[test]
fn hello_at_a_poisoned_workspace_names_the_member() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("healthy.md", "# Healthy\n")]);
    fs::write(ws.join("poison.md"), b"# P\n\xc3\x28\n").unwrap();
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    let refusal = conn.hello(&ws);
    assert_eq!(refusal["ok"], json!(false), "poisoned bind refuses: {refusal}");
    assert_eq!(refusal["error"]["code"], json!("invalid_utf8"), "{refusal}");
    assert_eq!(
        refusal["error"]["path"],
        json!("poison.md"),
        "hello's refusal names the offending member: {refusal}"
    );

    server.shutdown();
}

/// A healthy corpus is unchanged: no refusal, no message where none is due —
/// the success path serves exactly as before.
#[test]
fn healthy_corpus_serves_unchanged() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("a.md", "# A\n\nbody\n"), ("b.md", "# B\n")]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    assert_eq!(conn.hello(&ws)["ok"], json!(true));
    let toc = conn.call(&json!({"op": "toc", "path": "a.md"}));
    assert_eq!(toc["ok"], json!(true), "healthy corpus serves: {toc}");
    assert_eq!(toc["body"]["path"], json!("a.md"));

    server.shutdown();
}
