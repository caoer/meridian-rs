//! Dogfood 2026-08-08 opus P3-2, amended by dogfood 2026-08-09 s10 — warm plane.
//!
//! The daemon's single-file reads are NOT hash-domain-scoped: §12.1's hash
//! domain ⊂ addressable domain holds at every door, so a real `.md` outside the
//! domain SERVES by explicit path (the write door always committed to it) and
//! `file_not_found` means one thing only — no file under the workspace root.
//! The refusal names the miss, discloses the partial state, carries a servable
//! Fix, and never offers domain exclusion as a second reading of the miss.
//! Code and recovery class stay frozen.
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

mod common;

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
    config.drain_cold_builds = Duration::from_secs(30);
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

/// A file that truly does not exist: the refusal names the miss, discloses the
/// partial state, carries a servable Fix, and states that domain exclusion is
/// NOT what this refusal means (§12.1).
#[test]
fn warm_read_of_a_missing_file_teaches_the_miss_and_not_domain_exclusion() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("healthy.md", "# Healthy\n\nfine.\n")]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    let ack = conn.hello(&ws);
    assert_eq!(ack["ok"], json!(true), "corpus binds: {ack}");

    let m = file_not_found_message(&mut conn, "missing.md");
    assert!(m.contains("missing.md"), "names the file: {m}");
    assert!(
        m.contains("Nothing was read") && m.contains("no rev was minted"),
        "discloses the partial state: {m}"
    );
    assert!(m.contains("Fix:"), "carries a fix clause: {m}");
    // The inverted teaching this refusal used to carry (dogfood s10): offering
    // domain exclusion as the second reading taught the caller that an ignored
    // path is unservable — the opposite of §12.1. The sentence must now deny it.
    assert!(
        m.contains("NOT this refusal"),
        "denies domain exclusion as a reading of the miss: {m}"
    );

    server.shutdown();
}

/// §12.1 addressability at the warm read door: a REAL `.md` the default ignore
/// excludes (dot-prefixed segment) SERVES by explicit path — same spans, same
/// `file_rev` the guarded write door needs — while the workspace fingerprint
/// never moves. Before the disk fallback in `doc_or_refusal` this path answered
/// `file_not_found` while `splice` on the SAME path committed (dogfood s10).
///
/// *Mutation:* drop the fallback — the `toc` below refuses `file_not_found`.
#[test]
fn warm_read_of_a_real_but_domain_excluded_file_serves_it() {
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

    let before = conn.call(&json!({"op": "root"}));
    assert_eq!(before["ok"], json!(true), "{before}");

    let toc = conn.call(&json!({"op": "toc", "path": ".github/real.md"}));
    assert_eq!(
        toc["ok"],
        json!(true),
        "the ignored path is addressable (§12.1): {toc}"
    );

    // The read door mints the CAS token a guarded write needs — the asymmetry
    // was that only the write door could mint it for this page.
    let read = conn.call(&json!({
        "op": "read",
        "path": ".github/real.md",
        "display_path": ".github/real.md",
    }));
    assert_eq!(read["ok"], json!(true), "the composed read serves: {read}");
    assert!(
        read["body"]["file_rev"].is_string(),
        "the read mints file_rev for an out-of-domain page: {read}"
    );

    // Reading out-of-domain bytes never moves the fingerprint.
    let after = conn.call(&json!({"op": "root"}));
    assert_eq!(
        after["body"]["fingerprint"], before["body"]["fingerprint"],
        "out-of-domain reads are fingerprint-neutral: {before} -> {after}"
    );

    // Control: a path with no file behind it still refuses.
    let missing = conn.call(&json!({"op": "toc", "path": ".github/ghost.md"}));
    assert_eq!(missing["ok"], json!(false), "{missing}");
    assert_eq!(
        missing["error"]["code"],
        json!("file_not_found"),
        "{missing}"
    );

    server.shutdown();
}
