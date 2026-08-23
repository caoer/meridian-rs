//! E2E gates for the resident daemon's wire read ops over the unified socket
//! (decision 0002): `hello` binds and warms a workspace, then
//! `toc`/`links`/`resolve`/`root`/`diff` answer from that warm state on the
//! same connection. Warm reuse is proven via the in-process handle.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use registry::{Config, RunningServer};
use serde_json::{Value, json};
use tempfile::TempDir;

mod common;

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
    config.drain_cold_builds = Duration::from_secs(30);
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

/// `hello` binds + warms the resident engine, `toc`/`links` answer from that
/// warm state, and warm reuse is proven via the in-process handle after.
#[test]
fn hello_binds_then_toc_and_links_serve_from_the_resident_engine() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("a.md", "# A\n\nsee [[b]]\n"), ("b.md", "# B\n")]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    let ack = conn.hello(&ws);
    assert_eq!(ack["ok"], json!(true), "hello ok: {ack}");
    assert_eq!(ack["body"]["proto"], json!(1), "v3 speaks proto 1: {ack}");
    assert!(
        ack["body"]["storage"].is_string(),
        "hello pins a storage drawer: {ack}"
    );

    let toc = conn.call(&json!({"op": "toc", "path": "a.md"}));
    assert_eq!(toc["ok"], json!(true), "toc ok: {toc}");
    assert_eq!(toc["body"]["path"], json!("a.md"));
    let nodes = toc["body"]["nodes"].as_array().expect("toc nodes array");
    assert!(
        nodes.iter().any(|n| n["kind"] == json!("heading")),
        "toc carries the heading node: {toc}"
    );

    let links = conn.call(&json!({"op": "links", "path": "a.md"}));
    assert_eq!(links["ok"], json!(true), "links ok: {links}");
    assert_eq!(
        links["body"]["files"]["a.md"]["resolved"]["b.md"],
        json!(1),
        "the resident index resolves [[b]] → b.md: {links}"
    );

    // A second hello at the unchanged corpus stays ok (idempotent bind).
    assert_eq!(conn.hello(&ws)["ok"], json!(true), "re-hello ok");

    assert_eq!(
        server.registry().warm_or_build(&ws).unwrap(),
        registry::WarmOutcome::Reused,
        "the resident engine stayed warm through the socket reads"
    );

    server.shutdown();
}

/// A corpus change forces exactly one rebuild; warm reuse resumes after.
#[test]
fn corpus_change_rebuilds_once_then_reuses() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("a.md", "# A\n"), ("b.md", "# B\n")]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    // Hello binds at config cost (§3.2); the `fingerprint` op is the warm
    // cursor read (v3 spelling).
    assert_eq!(conn.hello(&ws)["ok"], json!(true));
    let before = conn.call(&json!({"op": "fingerprint"}))["body"]["fingerprint"]
        .as_str()
        .expect("the fingerprint op carries the warm fingerprint")
        .to_string();
    assert_eq!(
        server.registry().warm_or_build(&ws).unwrap(),
        registry::WarmOutcome::Reused,
        "unchanged corpus reuses the warm engine (zero parses)"
    );

    fs::write(ws.join("a.md"), "# A changed\n\nnew body\n").unwrap();

    let after = conn.call(&json!({"op": "fingerprint"}))["body"]["fingerprint"]
        .as_str()
        .expect("the fingerprint op carries the warm fingerprint")
        .to_string();
    assert_ne!(
        before, after,
        "a corpus change moves the warm engine's root"
    );

    assert_eq!(
        server.registry().warm_or_build(&ws).unwrap(),
        registry::WarmOutcome::Reused,
        "the rebuild is once — reuse resumes at the new fingerprint"
    );

    server.shutdown();
}

/// A wire read op before any `hello` is refused loudly — the connection has no
/// workspace bound to serve from.
#[test]
fn read_op_without_hello_is_refused() {
    let tmp = TempDir::new().unwrap();
    write_ws(&tmp, &[("a.md", "# A\n")]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    let toc = conn.call(&json!({"op": "toc", "path": "a.md"}));
    assert_eq!(toc["ok"], json!(false), "unbound read is refused: {toc}");
    assert!(
        toc["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("hello")),
        "the refusal names the missing hello: {toc}"
    );
}

/// `resolve` (walk plane), `fingerprint`, and `diff` answer over the socket.
/// No delta ring yet, so a same-cursor diff is empty and any other range
/// resyncs.
#[test]
fn resolve_root_and_diff_answer_over_the_socket() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("a.md", "# A\n\nsee [[b]]\n"), ("b.md", "# B\n")]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    conn.hello(&ws);

    let resolved = conn.call(&json!({"op": "resolve", "from": "a.md", "ref": "b"}));
    assert_eq!(resolved["ok"], json!(true), "resolve ok: {resolved}");
    assert_eq!(
        resolved["body"]["dest"],
        json!("b.md"),
        "resolve walks [[b]] → b.md: {resolved}"
    );

    let cursor_frame = conn.call(&json!({"op": "fingerprint"}));
    assert_eq!(
        cursor_frame["ok"],
        json!(true),
        "fingerprint ok: {cursor_frame}"
    );
    assert_eq!(
        cursor_frame["body"]["seq"],
        json!(0),
        "no batches emitted yet: {cursor_frame}"
    );
    let cursor = cursor_frame["body"]["fingerprint"]
        .as_str()
        .expect("fingerprint cursor")
        .to_string();

    let same = conn.call(&json!({
        "op": "diff", "from_fingerprint": cursor, "to_fingerprint": cursor,
    }));
    assert_eq!(same["ok"], json!(true), "same-cursor diff ok: {same}");
    assert_eq!(
        same["body"]["batches"],
        json!([]),
        "same-cursor diff replays nothing: {same}"
    );

    let unknown = conn.call(&json!({
        "op": "diff", "from_fingerprint": "b3:deadbeef", "to_fingerprint": cursor,
    }));
    assert_eq!(
        unknown["ok"],
        json!(false),
        "stale range refuses: {unknown}"
    );
    assert_eq!(
        unknown["error"]["code"],
        json!("fingerprint_unknown"),
        "an unknown range is fingerprint_unknown → resync: {unknown}"
    );

    server.shutdown();
}

/// The admin verbs `register`/`resolve_ws`/`list` (mrd's `Client` contract)
/// still ride the same socket, driven with raw frames to prove the tag routing.
#[test]
fn admin_verbs_share_the_socket_after_unification() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("a.md", "# A\n")]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    assert_eq!(
        conn.call(&json!({"op": "ping"}))["status"],
        json!("pong"),
        "admin ping still answers on the unified socket"
    );

    let registered = conn.call(&json!({"op": "register", "path": ws.to_str().unwrap()}));
    assert_eq!(
        registered["status"],
        json!("registered"),
        "admin register succeeds: {registered}"
    );
    let listed = conn.call(&json!({"op": "list"}));
    assert_eq!(listed["status"], json!("listed"));
    assert_eq!(
        listed["entries"].as_array().map(Vec::len),
        Some(1),
        "the registered workspace lists: {listed}"
    );

    server.shutdown();
}
