//! E2E gates for the resident daemon's WRITE path (decision 0002, W1). A real
//! client `hello`s a workspace, then `splice`s over the SAME connection: the edit
//! lands on disk through the ONE shared `splice → commit` choke-point, the
//! response is a BARE meridian-fs commit (no rule packs ⇒ `verdicts: []`), and the
//! NEXT read reflects the write — the resident engine rebuilds on the moved
//! fingerprint (U1 `warm_or_build`), so `cat` serves the committed bytes. A dry
//! splice writes nothing.

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

/// A workspace `tmp/ws` seeded with `files` — a sibling of the cache root, so the
/// corpus walk never sees the drawer.
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

/// A persistent connection speaking raw NDJSON: `hello` binds the workspace, then
/// every op rides the SAME connection (the per-connection binding the handshake
/// sets up).
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

const PLAN: &str = "# Goals\n\nship by August\n";
const PLAN_AFTER: &str = "# Goals\n\nship by September\n";

/// A guarded `match` edit inside the Goals section: `ship by August` →
/// `ship by September`.
fn splice_frame() -> Value {
    json!({
        "id": 7,
        "op": "splice",
        "path": "plan.md",
        "edits": [{
            "target": {"hpath": [{"h": "Goals"}]},
            "edit": {"match": {"old": "ship by August", "new": "ship by September"}},
        }],
    })
}

/// The main gate: `hello` binds + warms; `splice` commits the edit to disk with a
/// BARE `verdicts: []`; the NEXT `cat` over the socket serves the committed bytes
/// (the resident engine rebuilt on the moved fingerprint).
#[test]
fn splice_lands_on_disk_and_the_next_read_reflects_it() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    // hello binds the workspace, warming the resident engine; capture its root.
    let ack = conn.hello(&ws);
    assert_eq!(ack["ok"], json!(true), "hello ok: {ack}");
    let root_before_hello = ack["body"]["root"]
        .as_str()
        .expect("hello root")
        .to_string();

    // splice: the edit commits through the shared choke-point.
    let splice = conn.call(&splice_frame());
    assert_eq!(splice["ok"], json!(true), "splice ok: {splice}");
    assert_eq!(
        splice["id"],
        json!(7),
        "the response echoes the frame id: {splice}"
    );
    let body = &splice["body"];
    assert_eq!(
        body["root_before"],
        json!(root_before_hello),
        "root_before is the pre-commit ambient root: {splice}"
    );
    assert!(
        body["root_after"].is_string(),
        "a real commit advances the root: {splice}"
    );
    assert_ne!(
        body["root_after"], body["root_before"],
        "the root actually moved: {splice}"
    );
    assert_eq!(body["seq"], json!(1), "first commit this epoch: {splice}");
    assert_eq!(
        body["verdicts"],
        json!([]),
        "a BARE commit loads no packs — verdicts empty: {splice}"
    );
    assert_eq!(
        body["armed"]["edits"][0]["target"]["hpath"][0]["h"],
        json!("Goals"),
        "the armed edit names the target: {splice}"
    );

    // The write LANDED on disk.
    let on_disk = fs::read_to_string(ws.join("plan.md")).unwrap();
    assert_eq!(on_disk, PLAN_AFTER, "the edit landed on disk byte-for-byte");

    // The NEXT read over the socket reflects it — the resident engine rebuilt on
    // the moved fingerprint (U1), so `cat` serves the committed bytes.
    let cat = conn.call(&json!({"op": "cat", "path": "plan.md"}));
    assert_eq!(cat["ok"], json!(true), "cat ok: {cat}");
    assert_eq!(
        cat["body"]["content"],
        json!(PLAN_AFTER),
        "the resident engine reflects the write on the next read: {cat}"
    );

    // Warm reuse resumed at the new fingerprint (the rebuild was once, on the cat
    // read's `warm_or_build`, not a storm).
    assert_eq!(
        server.registry().warm_or_build(&ws).unwrap(),
        registry::WarmOutcome::Reused,
        "the post-splice read rebuilt once; the engine is warm at the new root"
    );

    server.shutdown();
}

/// A dry splice runs everything except disk: same response shape, `root_after`
/// null, `verdicts` present, and NOT one byte changes on disk.
#[test]
fn dry_splice_writes_nothing() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    conn.hello(&ws);

    let mut frame = splice_frame();
    frame["dry"] = json!(true);
    let splice = conn.call(&frame);
    assert_eq!(splice["ok"], json!(true), "dry splice ok: {splice}");
    assert_eq!(splice["body"]["dry"], json!(true), "dry flagged: {splice}");
    assert_eq!(
        splice["body"]["root_after"],
        Value::Null,
        "a dry run advances no root: {splice}"
    );
    assert_eq!(
        splice["body"]["verdicts"],
        json!([]),
        "dry verdicts: {splice}"
    );

    // Zero disk effects means zero: the file is byte-for-byte unchanged.
    let on_disk = fs::read_to_string(ws.join("plan.md")).unwrap();
    assert_eq!(on_disk, PLAN, "a dry splice writes nothing");

    server.shutdown();
}

/// A `splice` before any `hello` is refused loudly — the connection has no
/// workspace bound to commit against.
#[test]
fn splice_without_hello_is_refused() {
    let tmp = TempDir::new().unwrap();
    write_ws(&tmp, &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    let splice = conn.call(&splice_frame());
    assert_eq!(
        splice["ok"],
        json!(false),
        "unbound splice is refused: {splice}"
    );
    assert!(
        splice["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("hello")),
        "the refusal names the missing hello: {splice}"
    );

    // And no byte reached disk.
    let on_disk = fs::read_to_string(tmp.path().join("ws").join("plan.md")).unwrap();
    assert_eq!(on_disk, PLAN, "a refused splice writes nothing");

    server.shutdown();
}

/// `splice` rides the daemon's advertised caps (§3.2 discovery honesty): the
/// `hello` cap set names `splice` + its `splice.*` amendments.
#[test]
fn hello_advertises_the_splice_caps() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    let ack = conn.hello(&ws);
    let caps: Vec<&str> = ack["body"]["caps"]
        .as_array()
        .expect("caps array")
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    for cap in [
        "splice",
        "splice.if_node_rev",
        "splice.if_root",
        "splice.dry",
        "splice.receipt",
        "splice.verdicts",
    ] {
        assert!(caps.contains(&cap), "caps advertise `{cap}`: {ack}");
    }

    server.shutdown();
}
