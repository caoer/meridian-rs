//! Dogfood 2026-08-08, opus P3-2 — warm plane (card `p2-dogfood-refusal-teaching`).
//!
//! The daemon's single-file reads are hash-domain-scoped: a file that does not
//! exist and a real `.md` that sits outside the hash domain (wire-contract §12)
//! answer the SAME `file_not_found`. The refusal must say so — name the miss,
//! the domain-scoped-vs-missing distinction, and a servable Fix — instead of
//! echoing the path as a bare token. Code and recovery class stay frozen.
//!
//! Harness: the `corpus_refusal` precedent (real daemon over a Unix socket).

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

/// The refusal for `path`, taken through a warm `toc`, with its message
/// asserted present and returned for content pins.
fn file_not_found_message(conn: &mut Conn, path: &str) -> String {
    let refusal = conn.call(&json!({"op": "toc", "path": path}));
    assert_eq!(
        refusal["ok"],
        json!(false),
        "{path} does not serve: {refusal}"
    );
    let error = &refusal["error"];
    assert_eq!(
        error["code"],
        json!("file_not_found"),
        "code frozen: {refusal}"
    );
    assert_eq!(
        error["path"],
        json!(path),
        "the path stays echoed: {refusal}"
    );
    error["message"]
        .as_str()
        .unwrap_or_else(|| panic!("the refusal is a sentence, not a bare code: {refusal}"))
        .to_owned()
}

/// A file that truly does not exist: the refusal names the miss, the
/// domain-scoped grain of the answer, and a servable Fix.
#[test]
fn warm_read_of_a_missing_file_teaches_the_domain_scoped_miss() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("healthy.md", "# Healthy\n\nfine.\n")]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    let ack = conn.hello(&ws);
    assert_eq!(ack["ok"], json!(true), "corpus binds: {ack}");

    let m = file_not_found_message(&mut conn, "missing.md");
    assert!(m.contains("missing.md"), "names the file: {m}");
    // The distinction: this one refusal covers a missing file AND a real file
    // outside the hash domain — the caller must be told both readings exist.
    assert!(m.contains("hash domain"), "names the domain scoping: {m}");
    assert!(
        m.contains("Nothing was read") && m.contains("no rev was minted"),
        "discloses the partial state: {m}"
    );
    assert!(m.contains("Fix:"), "carries a fix clause: {m}");

    server.shutdown();
}

/// The trap's other face: a REAL `.md` on disk whose path the default ignore
/// excludes (dot-prefixed segment, §12.1) answers the same `file_not_found` —
/// and the same teaching must reach it, naming where the domain rules live.
#[test]
fn warm_read_of_a_real_but_domain_excluded_file_teaches_the_distinction() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(
        &tmp,
        &[
            ("healthy.md", "# Healthy\n\nfine.\n"),
            (".github/real.md", "# Real bytes, outside the domain\n"),
        ],
    );
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    let ack = conn.hello(&ws);
    assert_eq!(ack["ok"], json!(true), "corpus binds: {ack}");

    let m = file_not_found_message(&mut conn, ".github/real.md");
    assert!(m.contains(".github/real.md"), "names the file: {m}");
    assert!(m.contains("hash domain"), "names the domain scoping: {m}");
    // The servable half: where the exclusion rules live, so a caller holding a
    // real file can find out WHY it does not serve.
    assert!(
        m.contains("meridian/domain.md"),
        "names the standing domain declaration: {m}"
    );

    server.shutdown();
}
