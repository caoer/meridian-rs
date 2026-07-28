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
        prewarm_interval: Duration::from_secs(365 * 24 * 60 * 60),
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

    // hello binds the workspace, warming the resident engine; capture its cursor.
    // This is a v3 session, so the cursor is spelled `fingerprint`.
    let ack = conn.hello(&ws);
    assert_eq!(ack["ok"], json!(true), "hello ok: {ack}");
    let fingerprint_before_hello = ack["body"]["fingerprint"]
        .as_str()
        .expect("hello fingerprint")
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
        body["fingerprint_before"],
        json!(fingerprint_before_hello),
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
// The BIRTH op on the resident host (the second dispatch arm).
//
// `create` has two dispatch arms — the per-workspace sidecar's and this one.
// Two arms is two chances to drift, so the daemon's arm gets its own gate: the
// same guarded door, the same journal row, the same CAS refusal.
// ---------------------------------------------------------------------------

/// The daemon births a file through the shared guarded door, journals it, and
/// serves the newborn on the NEXT read over the same connection.
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

    // The caller's exact bytes, on disk.
    assert_eq!(
        fs::read_to_string(ws.join("notes/newborn.md")).expect("newborn exists"),
        body_bytes,
        "birth is byte-transparent on the daemon too"
    );

    // The journal row the guarded door writes — the assertion a daemon-side
    // bypass could not satisfy.
    let anchor = birth["body"]["journal_anchor"]
        .as_str()
        .expect("the birth reports its journal anchor");
    let journal = fs::read_to_string(ws.join("meridian/journal.md")).expect("journal written");
    let row = journal
        .lines()
        .find(|l| l.contains(&format!("^{anchor}")))
        .unwrap_or_else(|| panic!("anchor `{anchor}` names a real row:\n{journal}"));
    assert!(
        row.contains("create"),
        "the row records the birth op: {row}"
    );
    assert!(
        row.contains("notes/newborn.md"),
        "the row names the path: {row}"
    );

    // v3 vocabulary, and a real root advance.
    assert_ne!(
        birth["body"]["fingerprint_after"], birth["body"]["fingerprint_before"],
        "the birth advanced the fingerprint: {birth}"
    );

    // The resident engine rebuilds on the moved fingerprint, so the newborn is
    // readable over the SAME connection.
    let toc = conn.call(&json!({"id": 12, "op": "toc", "path": "notes/newborn.md"}));
    assert_eq!(toc["ok"], json!(true), "the newborn is readable: {toc}");
    assert_eq!(
        toc["body"]["file_rev"], birth["body"]["file_rev_after"],
        "the rev the birth reported IS the rev the read serves"
    );
}

/// The daemon's arm refuses a clobber for the SAME designed reason the sidecar's
/// does — asserted by cause, and with the occupant proven byte-untouched.
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
/// birth op nor serves it. Both halves on a PROVEN-LIVE handshake — a refused
/// hello would fake the same absence.
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
