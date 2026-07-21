//! E2E gates for the resident daemon's wire read ops over the unified socket
//! (decision 0002, U2 — the binding folded onto `hello` at U3). A real client
//! `hello`s a workspace — resolving, pinning, and warming its resident engine —
//! then gets correct `toc`/`links`/`resolve`/`root`/`diff` answers from that
//! warm state, all on the SAME connection. Warm reuse (no reparse) is proven via
//! the in-process handle: after the socket reads, `warm_or_build` answers
//! `Reused` (U1's residency, served over the socket).

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
    let dir = tmp.path().join("registry");
    Config {
        socket_path: dir.join("daemon.sock"),
        state_path: dir.join("state.json"),
        cache_root: tmp.path().join("cache"),
        idle_threshold: Duration::from_secs(365 * 24 * 60 * 60),
        reap_interval: Duration::from_secs(365 * 24 * 60 * 60),
    }
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

/// A persistent connection speaking raw NDJSON: `hello` binds the workspace,
/// then the wire read ops ride the SAME connection (the per-connection binding
/// the handshake sets up).
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

/// The spine gate: `hello` binds + warms the resident engine, `toc`/`links`
/// answer correctly from that warm state, and warm reuse (no reparse) is proven
/// via the in-process handle after the socket reads.
#[test]
fn hello_binds_then_toc_and_links_serve_from_the_resident_engine() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("a.md", "# A\n\nsee [[b]]\n"), ("b.md", "# B\n")]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    // Hello binds the workspace, pins storage, warms the engine, negotiates v3.
    let ack = conn.hello(&ws);
    assert_eq!(ack["ok"], json!(true), "hello ok: {ack}");
    assert_eq!(ack["body"]["proto"], json!(1), "v3 speaks proto 1: {ack}");
    assert!(
        ack["body"]["storage"].is_string(),
        "hello pins a storage drawer: {ack}"
    );

    // toc a.md — served from the resident engine, correct shape.
    let toc = conn.call(&json!({"op": "toc", "path": "a.md"}));
    assert_eq!(toc["ok"], json!(true), "toc ok: {toc}");
    assert_eq!(toc["body"]["path"], json!("a.md"));
    let nodes = toc["body"]["nodes"].as_array().expect("toc nodes array");
    assert!(
        nodes.iter().any(|n| n["kind"] == json!("heading")),
        "toc carries the heading node: {toc}"
    );

    // links a.md — the edge map resolves [[b]] → b.md.
    let links = conn.call(&json!({"op": "links", "path": "a.md"}));
    assert_eq!(links["ok"], json!(true), "links ok: {links}");
    assert_eq!(
        links["body"]["files"]["a.md"]["resolved"]["b.md"],
        json!(1),
        "the resident index resolves [[b]] → b.md: {links}"
    );

    // A second hello at the unchanged corpus stays ok (idempotent bind).
    assert_eq!(conn.hello(&ws)["ok"], json!(true), "re-hello ok");

    // The no-reparse proof rides the in-process handle: after the socket reads
    // and the second hello, warming again parses NOTHING (the warm engine is
    // reused — `Reused` reaches no parse site).
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

    // Hello binds + warms; its response carries the warm engine's ambient root.
    let before = conn.hello(&ws)["body"]["root"]
        .as_str()
        .expect("hello carries the warm root")
        .to_string();
    assert_eq!(
        server.registry().warm_or_build(&ws).unwrap(),
        registry::WarmOutcome::Reused,
        "unchanged corpus reuses the warm engine (zero parses)"
    );

    // Mutate a file → the corpus content hash changes.
    fs::write(ws.join("a.md"), "# A changed\n\nnew body\n").unwrap();

    // The next hello re-reads disk: its ambient root moves to the new fingerprint.
    let after = conn.hello(&ws)["body"]["root"]
        .as_str()
        .expect("hello carries the warm root")
        .to_string();
    assert_ne!(
        before, after,
        "a corpus change moves the warm engine's root"
    );

    // The rebuild was once, not a storm: warm reuse resumes at the new fingerprint.
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

/// `resolve` (walk plane), `root`, and `diff` answer over the socket. `resolve`
/// is served cold from the walk-plane corpus; `root`/`diff` come from the warm
/// engine's fingerprint (no delta ring yet → same-root diff empty, else resync).
#[test]
fn resolve_root_and_diff_answer_over_the_socket() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("a.md", "# A\n\nsee [[b]]\n"), ("b.md", "# B\n")]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    conn.hello(&ws);

    // resolve the raw linktext `b` from a.md → b.md (the app-compatible walk).
    let resolved = conn.call(&json!({"op": "resolve", "from": "a.md", "ref": "b"}));
    assert_eq!(resolved["ok"], json!(true), "resolve ok: {resolved}");
    assert_eq!(
        resolved["body"]["dest"],
        json!("b.md"),
        "resolve walks [[b]] → b.md: {resolved}"
    );

    // root: the world-grain cursor + seq (0 — no batches in this daemon epoch).
    let root = conn.call(&json!({"op": "root"}));
    assert_eq!(root["ok"], json!(true), "root ok: {root}");
    assert_eq!(
        root["body"]["seq"],
        json!(0),
        "no batches emitted yet: {root}"
    );
    let cursor = root["body"]["root"]
        .as_str()
        .expect("root cursor")
        .to_string();

    // diff over the same root is truthfully empty.
    let same = conn.call(&json!({"op": "diff", "from_root": cursor, "to_root": cursor}));
    assert_eq!(same["ok"], json!(true), "same-root diff ok: {same}");
    assert_eq!(
        same["body"]["batches"],
        json!([]),
        "same-root diff replays nothing: {same}"
    );

    // diff over an unknown range degrades to full resync (root_unknown).
    let unknown = conn.call(&json!({"op": "diff", "from_root": "b3:deadbeef", "to_root": cursor}));
    assert_eq!(
        unknown["ok"],
        json!(false),
        "stale range refuses: {unknown}"
    );
    assert_eq!(
        unknown["error"]["code"],
        json!("root_unknown"),
        "an unknown root range is root_unknown → resync: {unknown}"
    );

    server.shutdown();
}

/// The admin verbs still ride the SAME socket, unchanged — the unification did
/// not disturb `register`/`resolve_ws`/`list` (mrd's `Client` contract). Driven
/// here with a raw admin frame to prove the tag routing, alongside the wire ops.
#[test]
fn admin_verbs_share_the_socket_after_unification() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("a.md", "# A\n")]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    // ping (admin) answers pong.
    assert_eq!(
        conn.call(&json!({"op": "ping"}))["status"],
        json!("pong"),
        "admin ping still answers on the unified socket"
    );

    // register (admin) then list (admin) — the workspace-registry path is intact.
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
