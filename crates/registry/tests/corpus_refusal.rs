//! Per-file UTF-8 degradation at the wire doors (node-rev-merkle-spec §3,
//! line 52 — ratified): files that are not valid UTF-8 "still get leaf hashes
//! (blake3 needs no UTF-8) and participate in the root; they simply serve no
//! spans/nodes (wire `invalid_utf8` law). Integrity coverage and span service
//! are independent properties."
//!
//! The motivating incident (dogfood 2026-08-08, P1): one poison member
//! (non-UTF-8 bytes) landed in a live corpus and the daemon refused the ENTIRE
//! workspace at `hello` — every `mrd script` fleet-wide died "cannot dial the
//! daemon" until the file was hunted down and removed. The ruled degradation
//! grain is the FILE: the poison member itself refuses `invalid_utf8` naming
//! itself (wire-contract §8: refuse, never lossy-decode), every healthy member
//! keeps serving, and the poison bytes stay under integrity coverage (they
//! participate in the root).
//!
//! Corpus-scoped refusals still exist — a member the snapshot cannot READ has
//! no bytes to hash, and an ambiguous domain config leaves no domain at all —
//! and those keep Law A-3c's shape: scope named, member named. UTF-8 is just
//! not one of them.

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

/// The incident, replayed under the ruled grain: a healthy corpus binds and
/// serves; one poison member lands through ordinary external activity; every
/// HEALTHY member keeps serving, and only the poison member itself refuses —
/// per-file `invalid_utf8`, naming itself.
#[test]
fn a_poison_member_degrades_itself_not_the_corpus() {
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
    fs::write(
        ws.join("notes/poison.md"),
        b"# Poison\n\n\xff\xfe raw bytes\n",
    )
    .unwrap();

    // §52 clause "healthy members serve": the corpus is NOT refused.
    let toc = conn.call(&json!({"op": "toc", "path": "healthy.md"}));
    assert_eq!(
        toc["ok"],
        json!(true),
        "one poison member must not poison the whole workspace: {toc}"
    );

    // §52 clause "serves no spans/nodes": the poison member itself refuses,
    // per-file, wearing the closed-taxonomy code and naming itself.
    let refusal = conn.call(&json!({"op": "toc", "path": "notes/poison.md"}));
    assert_eq!(
        refusal["ok"],
        json!(false),
        "the poison member serves nothing: {refusal}"
    );
    let error = &refusal["error"];
    assert_eq!(error["code"], json!("invalid_utf8"), "{refusal}");
    assert_eq!(
        error["recovery"],
        json!("env"),
        "invalid_utf8 stays env-class: {refusal}"
    );
    assert_eq!(
        error["path"],
        json!("notes/poison.md"),
        "the per-file refusal names the file itself: {refusal}"
    );
    let message = error["message"]
        .as_str()
        .unwrap_or_else(|| panic!("the refusal carries a message: {refusal}"));
    assert!(
        message.contains("notes/poison.md") && message.contains("UTF-8"),
        "the message names the member and the condition: {refusal}"
    );

    server.shutdown();
}

/// The links door, same grain (defect-ledger RES-B): a DIRECT poison-path
/// `links` query answers the typed per-file `invalid_utf8` naming the member,
/// its condition, and where its bytes stand — never `file_not_found`, which
/// claims a miss for a member that exists on disk. And the whole-corpus map
/// still serves beside it.
///
/// *Mutation:* revert the links doors' miss split (`links_miss`) to the bare
/// `docs.contains_key` check — the direct query answers `file_not_found`.
#[test]
fn a_direct_poison_path_links_query_answers_typed_invalid_utf8() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("healthy.md", "# Healthy\n\n[[poison]]\n")]);
    fs::write(ws.join("poison.md"), b"# P\n\xff\xfe\n").unwrap();
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    assert_eq!(conn.hello(&ws)["ok"], json!(true));

    // The whole-corpus map serves — degradation never widens past the file.
    let map = conn.call(&json!({"op": "links"}));
    assert_eq!(map["ok"], json!(true), "the corpus map still serves: {map}");
    assert!(
        map["body"]["files"]["healthy.md"].is_object(),
        "the healthy member's edges are in the map: {map}"
    );

    // The direct ask gets the per-file refusal, typed and teaching.
    let refusal = conn.call(&json!({"op": "links", "path": "poison.md"}));
    assert_eq!(refusal["ok"], json!(false), "{refusal}");
    let error = &refusal["error"];
    assert_eq!(
        error["code"],
        json!("invalid_utf8"),
        "an unserved member is not a miss: {refusal}"
    );
    assert_eq!(error["recovery"], json!("env"), "{refusal}");
    assert_eq!(error["path"], json!("poison.md"), "{refusal}");
    let message = error["message"]
        .as_str()
        .unwrap_or_else(|| panic!("the refusal carries its teaching: {refusal}"));
    assert!(
        message.contains("poison.md")
            && message.contains("UTF-8")
            && message.contains("bytes stay under the root"),
        "path + condition + recovery, the read doors' exact frame: {refusal}"
    );

    // Control: a path that truly is absent keeps `file_not_found`.
    let missing = conn.call(&json!({"op": "links", "path": "missing.md"}));
    assert_eq!(missing["ok"], json!(false), "{missing}");
    assert_eq!(
        missing["error"]["code"],
        json!("file_not_found"),
        "{missing}"
    );

    server.shutdown();
}

/// The other wire door, same grain: a `hello` that declares a workspace with a
/// poison member BINDS and serves — the fleet-killing shape (refuse the entire
/// workspace at handshake) is exactly what §52 rules out.
#[test]
fn hello_at_a_poisoned_workspace_binds_and_serves() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("healthy.md", "# Healthy\n")]);
    fs::write(ws.join("poison.md"), b"# P\n\xc3\x28\n").unwrap();
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    let ack = conn.hello(&ws);
    assert_eq!(
        ack["ok"],
        json!(true),
        "a poisoned workspace still binds — degradation is per-file: {ack}"
    );
    let toc = conn.call(&json!({"op": "toc", "path": "healthy.md"}));
    assert_eq!(
        toc["ok"],
        json!(true),
        "healthy member serves at once: {toc}"
    );

    server.shutdown();
}

/// §52's other half: the poison bytes stay under INTEGRITY coverage — the file
/// gets a leaf hash and participates in the root, so changing its (still
/// non-UTF-8) bytes moves the workspace root. Span service and integrity
/// coverage are independent properties.
#[test]
fn poison_bytes_participate_in_the_root() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("healthy.md", "# Healthy\n")]);
    fs::write(ws.join("poison.md"), b"# P\n\xff\xfe v1\n").unwrap();
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    assert_eq!(conn.hello(&ws)["ok"], json!(true));
    // v3 spells the world-grain op `fingerprint` (rev projection root ↔
    // fingerprint) — the `script_golden_live` dial precedent.
    let before = conn.call(&json!({"op": "fingerprint"}));
    assert_eq!(before["ok"], json!(true), "{before}");
    let before_fp = before["body"]["fingerprint"]
        .as_str()
        .unwrap_or_else(|| panic!("the answer carries the fingerprint: {before}"))
        .to_owned();

    fs::write(ws.join("poison.md"), b"# P\n\xff\xfe v2\n").unwrap();
    let after = conn.call(&json!({"op": "fingerprint"}));
    assert_eq!(after["ok"], json!(true), "{after}");
    let after_fp = after["body"]["fingerprint"]
        .as_str()
        .unwrap_or_else(|| panic!("the answer carries the fingerprint: {after}"))
        .to_owned();
    assert_ne!(
        before_fp, after_fp,
        "a poison member's bytes must move the root — leaf hashes need no UTF-8"
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

/// The same law read from the other side: a code names a CONDITION, so a
/// condition that is not "these bytes are not UTF-8" must not wear
/// `invalid_utf8`. Two domain configs both decode perfectly — the workspace is
/// ambiguous, not corrupt — so the refusal rides `io_error{cause}`, still env
/// class, still carrying the remedy. This refusal IS corpus-scoped (there is
/// no domain to build any corpus with) and keeps Law A-3c's shape.
#[test]
fn two_domain_configs_refuse_as_io_error_not_invalid_utf8() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(
        &tmp,
        &[
            ("healthy.md", "# Healthy\n"),
            ("meridian/domain.md", "---\nignore:\n  - \"a/**\"\n---\n"),
            ("mdfs_config.yaml", "ignore:\n  - \"b/**\"\n"),
        ],
    );
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    let refusal = conn.hello(&ws);
    assert_eq!(
        refusal["ok"],
        json!(false),
        "an ambiguous domain refuses: {refusal}"
    );
    let error = &refusal["error"];
    assert_eq!(
        error["code"],
        json!("io_error"),
        "the ambiguity is not a UTF-8 condition: {refusal}"
    );
    assert_eq!(
        error["recovery"],
        json!("env"),
        "io_error is env class: {refusal}"
    );
    let cause = error["cause"]
        .as_str()
        .unwrap_or_else(|| panic!("io_error carries its cause: {refusal}"));
    assert!(
        cause.contains("meridian/domain.md")
            && cause.contains("mdfs_config.yaml")
            && cause.contains("Remedy:"),
        "the remedy survives the remap: {refusal}"
    );

    server.shutdown();
}
