//! E2E gates for the resident daemon's write path (decision 0002): `hello`
//! binds a workspace, `splice` commits over the same connection through the
//! shared `splice → commit` choke-point, and the next read reflects it.

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

/// A persistent NDJSON connection: `hello` binds the workspace, then every op
/// rides the same connection.
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

const PLAN: &str = "# Goals\n\nship by August\n";
const PLAN_AFTER: &str = "# Goals\n\nship by September\n";

/// A guarded `match` edit inside Goals: `ship by August` → `ship by September`.
/// A wire-origin content change carries the section's live fingerprint, so the
/// frame reads the `toc` for it first.
fn splice_frame(conn: &mut Conn) -> Value {
    let toc = conn.call(&json!({"op": "toc", "path": "plan.md"}));
    let rev = toc["body"]["nodes"]
        .as_array()
        .expect("toc nodes")
        .iter()
        .find(|n| n["hpath"][0]["h"] == json!("Goals"))
        .unwrap_or_else(|| panic!("Goals in toc: {toc}"))["node_rev"]
        .as_str()
        .expect("node_rev")
        .to_string();
    json!({
        "id": 7,
        "op": "splice",
        "path": "plan.md",
        "edits": [{
            "target": {"hpath": [{"h": "Goals"}]},
            "edit": {"match": {"old": "ship by August", "new": "ship by September"}},
            "if_node_rev": rev,
        }],
    })
}

/// `hello` binds (config-grade, §3.2); the `fingerprint` op warms; `splice` commits to disk with a bare `verdicts: []`;
/// the next `cat` over the socket serves the committed bytes.
#[test]
fn splice_lands_on_disk_and_the_next_read_reflects_it() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    // Hello binds at config cost (§3.2); the `fingerprint` op reads the warm
    // cursor (v3 spelling).
    let ack = conn.hello(&ws);
    assert_eq!(ack["ok"], json!(true), "hello ok: {ack}");
    let fingerprint_before_splice = conn.call(&json!({"op": "fingerprint"}))["body"]["fingerprint"]
        .as_str()
        .expect("the fingerprint op carries the ambient cursor")
        .to_string();

    let splice = {
        let f = splice_frame(&mut conn);
        conn.call(&f)
    };
    assert_eq!(splice["ok"], json!(true), "splice ok: {splice}");
    assert_eq!(
        splice["id"],
        json!(7),
        "the response echoes the frame id: {splice}"
    );
    let body = &splice["body"];
    assert_eq!(
        body["fingerprint_before"],
        json!(fingerprint_before_splice),
        "fingerprint_before is the pre-commit ambient cursor: {splice}"
    );
    assert!(
        body["fingerprint_after"].is_string(),
        "a real commit advances the fingerprint: {splice}"
    );
    assert_ne!(
        body["fingerprint_after"], body["fingerprint_before"],
        "the fingerprint actually moved: {splice}"
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

    let on_disk = fs::read_to_string(ws.join("plan.md")).unwrap();
    assert_eq!(on_disk, PLAN_AFTER, "the edit landed on disk byte-for-byte");

    let cat = conn.call(&json!({"op": "cat", "path": "plan.md"}));
    assert_eq!(cat["ok"], json!(true), "cat ok: {cat}");
    assert_eq!(
        cat["body"]["content"],
        json!(PLAN_AFTER),
        "the resident engine reflects the write on the next read: {cat}"
    );

    assert_eq!(
        server.registry().warm_or_build(&ws).unwrap(),
        registry::WarmOutcome::Reused,
        "the post-splice read rebuilt once; the engine is warm at the new root"
    );

    server.shutdown();
}

/// A dry splice runs everything except disk: same response shape, `root_after`
/// null, `verdicts` present, and not one byte changes on disk.
#[test]
fn dry_splice_writes_nothing() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    conn.hello(&ws);

    let mut frame = splice_frame(&mut conn);
    frame["dry"] = json!(true);
    let splice = conn.call(&frame);
    assert_eq!(splice["ok"], json!(true), "dry splice ok: {splice}");
    assert_eq!(splice["body"]["dry"], json!(true), "dry flagged: {splice}");
    // v3 session: the null transition slot is spelled `fingerprint_after`.
    assert_eq!(
        splice["body"]["fingerprint_after"],
        Value::Null,
        "a dry run advances no fingerprint: {splice}"
    );
    assert_eq!(
        splice["body"]["verdicts"],
        json!([]),
        "dry verdicts: {splice}"
    );

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

    // Written out by hand: an unbound connection cannot read a `toc` either.
    let splice = conn.call(&json!({
        "id": 7,
        "op": "splice",
        "path": "plan.md",
        "edits": [{
            "target": {"hpath": [{"h": "Goals"}]},
            "edit": {"match": {"old": "ship by August", "new": "ship by September"}},
        }],
    }));
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
    // v3 session: the root-guard amendment cap is spelled `splice.if_fingerprint`.
    for cap in [
        "splice",
        "splice.if_node_rev",
        "splice.if_fingerprint",
        "splice.dry",
        "splice.receipt",
        "splice.verdicts",
    ] {
        assert!(caps.contains(&cap), "caps advertise `{cap}`: {ack}");
    }

    server.shutdown();
}

// ---------------------------------------------------------------------------
// The birth op on the resident host (the second `create` dispatch arm).
// ---------------------------------------------------------------------------

/// The daemon births a file through the shared guarded door, journals it, and
/// serves the newborn on the next read over the same connection.
#[test]
fn create_births_on_the_daemon_and_the_next_read_serves_the_newborn() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    let ack = conn.hello(&ws);
    assert_eq!(ack["ok"], json!(true), "hello ok: {ack}");

    let body_bytes = "---\ntype: note\n---\n\n# Newborn\n\nborn on the daemon\n";
    let birth = conn.call(&json!({
        "id": 11, "op": "create", "path": "notes/newborn.md", "body": body_bytes,
        "actor": "agent:daemon-test", "now": "2026-07-26T13:30:00-04:00",
    }));
    assert_eq!(birth["ok"], json!(true), "the birth landed: {birth}");
    assert_eq!(birth["id"], json!(11), "the response echoes the frame id");

    assert_eq!(
        fs::read_to_string(ws.join("notes/newborn.md")).expect("newborn exists"),
        body_bytes,
        "birth is byte-transparent on the daemon too"
    );

    assert!(
        birth["body"]["journal_anchor"].is_null(),
        "the retired journal anchor is gone from the birth reply: {birth}"
    );

    assert_ne!(
        birth["body"]["fingerprint_after"], birth["body"]["fingerprint_before"],
        "the birth advanced the fingerprint: {birth}"
    );

    let toc = conn.call(&json!({"id": 12, "op": "toc", "path": "notes/newborn.md"}));
    assert_eq!(toc["ok"], json!(true), "the newborn is readable: {toc}");
    assert_eq!(
        toc["body"]["file_rev"], birth["body"]["file_rev_after"],
        "the rev the birth reported IS the rev the read serves"
    );
}

/// The daemon's arm refuses a clobber by cause, leaving the occupant
/// byte-untouched.
#[test]
fn create_on_an_occupied_path_refuses_cas_mismatch_on_the_daemon() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    assert_eq!(conn.hello(&ws)["ok"], json!(true));

    let clobber = conn.call(&json!({
        "id": 13, "op": "create", "path": "plan.md", "body": "# Clobbered\n",
    }));
    assert_eq!(clobber["ok"], json!(false));
    assert_eq!(
        clobber["error"]["code"],
        json!("cas_mismatch"),
        "an occupied path is a CAS failure, not a generic error: {clobber}"
    );
    assert_eq!(
        clobber["error"]["recovery"],
        json!("refresh"),
        "the recovery class teaches `re-read, it exists`: {clobber}"
    );
    assert_eq!(
        fs::read_to_string(ws.join("plan.md")).unwrap(),
        PLAN,
        "the occupant is byte-untouched — there is no clobbering birth"
    );
}

/// §3.2 discovery honesty on the daemon: a v2 session neither advertises the
/// birth op nor serves it — asserted on a proven-live handshake, since a
/// refused hello would fake the same absence.
#[test]
fn a_v2_daemon_session_neither_advertises_nor_serves_create() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    let ack = conn.call(&json!({
        "op": "hello", "proto": 1, "workspace": ws.to_str().unwrap(),
    }));
    assert_eq!(ack["ok"], json!(true), "the v2 handshake is LIVE: {ack}");
    let caps = ack["body"]["caps"].as_array().expect("caps");
    assert!(
        !caps.contains(&json!("create")),
        "a v2 session never advertises the birth op: {caps:?}"
    );

    let refused = conn.call(&json!({
        "id": 14, "op": "create", "path": "newborn.md", "body": "# X\n",
    }));
    assert_eq!(refused["error"]["code"], json!("unknown_op"), "{refused}");
    assert!(
        !ws.join("newborn.md").exists(),
        "a refused op births nothing"
    );
}
