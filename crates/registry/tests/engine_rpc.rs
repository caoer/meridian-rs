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

    // v3 session: the cursor is spelled `fingerprint`.
    let before = conn.hello(&ws)["body"]["fingerprint"]
        .as_str()
        .expect("hello carries the warm fingerprint")
        .to_string();
    assert_eq!(
        server.registry().warm_or_build(&ws).unwrap(),
        registry::WarmOutcome::Reused,
        "unchanged corpus reuses the warm engine (zero parses)"
    );

    fs::write(ws.join("a.md"), "# A changed\n\nnew body\n").unwrap();

    let after = conn.hello(&ws)["body"]["fingerprint"]
        .as_str()
        .expect("hello carries the warm fingerprint")
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

/// §5.3a fold economy, counted not timed: once the engine is warm, quiet
/// per-document serves run NO whole-corpus fold (`fs::fold_count`) and re-read
/// NO file bytes (`fs::leaf_reread_count`). At the pinned base every one of
/// these round trips folded the whole corpus — the O(domain bytes) per round
/// trip this wave removes.
///
/// Serial-only counters: this test must not run concurrently with other
/// folding tests in one process, so it takes the instruments in one test body
/// (cold warm, then the quiet window) rather than splitting across tests.
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

/// Reads are LIVE at document grain: an out-of-band write to the served file
/// (the rename discipline every real writer uses) is visible on the very next
/// serve of that file — no sweep, no subscription, no second call.
#[test]
fn an_out_of_band_write_is_served_on_the_next_read_of_that_file() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("a.md", "# A\n"), ("b.md", "# B\n")]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    assert_eq!(conn.hello(&ws)["ok"], json!(true));

    let before = conn.call(&json!({"op": "toc", "path": "a.md"}));
    assert_eq!(before["ok"], json!(true));
    let stamp_before = before["body"]["fingerprint"].as_str().unwrap().to_owned();

    // Out-of-band writer: tmp + rename, the discipline editors and engines share.
    let tmp_file = ws.join(".a.md.oob.tmp");
    fs::write(&tmp_file, "# A moved\n\nnew body\n").unwrap();
    fs::rename(&tmp_file, ws.join("a.md")).unwrap();

    let after = conn.call(&json!({"op": "toc", "path": "a.md"}));
    assert_eq!(after["ok"], json!(true), "the moved file serves: {after}");
    let titles: Vec<&str> = after["body"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|n| n["text_prefix_16b"].as_str())
        .collect();
    assert!(
        titles.iter().any(|t| t.starts_with("# A moved")),
        "the NEXT read of the moved file serves the new bytes: {titles:?}"
    );
    assert_ne!(
        after["body"]["fingerprint"].as_str().unwrap(),
        stamp_before,
        "the stamp advanced with the world"
    );

    server.shutdown();
}

/// The write-door-ran-ahead regression, pinned: a daemon-door write advances
/// the leaf cache before any engine rebuild. A per-document serve of an
/// UNTOUCHED file must not let the stale engine vouch for itself — the serve
/// after the write carries the post-write fingerprint, not the pre-write one.
#[test]
fn a_serve_after_a_daemon_write_never_stamps_the_pre_write_root() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("a.md", "# A\n")]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    assert_eq!(conn.hello(&ws)["ok"], json!(true));

    let before = conn.call(&json!({"op": "toc", "path": "a.md"}));
    let stamp_before = before["body"]["fingerprint"].as_str().unwrap().to_owned();

    let born = conn.call(&json!({
        "op": "create",
        "path": "born.md",
        "body": "# Born\n",
        "actor": "94ef2cb0",
    }));
    assert_eq!(born["ok"], json!(true), "the birth lands: {born}");
    let root_after = born["body"]["fingerprint_after"]
        .as_str()
        .expect("a birth reports the advanced root")
        .to_owned();
    assert_ne!(root_after, stamp_before, "the birth moved the root");

    let after = conn.call(&json!({"op": "toc", "path": "a.md"}));
    assert_eq!(after["ok"], json!(true));
    assert_eq!(
        after["body"]["fingerprint"].as_str().unwrap(),
        root_after,
        "an untouched file's serve after the write stamps the post-write root — \
         the cache that ran ahead may not vouch for the stale engine"
    );

    server.shutdown();
}
