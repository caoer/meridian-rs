//! E2E gates for `sql` — corpus SQL over the wire (`docs/wire-contract.md`
//! § A.11): one statement over the workspace's fingerprint-pinned projection
//! cache, served by the resident engine under the `agent` sandbox, freshness
//! folded post-result. v3-only, advertised as cap `sql` (v3 projection);
//! the daemon is the cache file's single owner and its one append actor.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use registry::{Config, RunningServer};
use serde_json::{Value, json};
use tempfile::TempDir;

/// A daemon config rooted under `tmp`, with reap horizons large enough that
/// the background reaper never evicts state mid-test.
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

/// A persistent connection speaking raw NDJSON: one frame in, one frame out.
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

    fn hello_v3(&mut self, workspace: &Path) -> Value {
        self.call(&json!({
            "op": "hello", "proto": 1, "contract": "v3",
            "workspace": workspace.to_str().unwrap(),
        }))
    }

    fn sql(&mut self, id: u64, query: &str) -> Value {
        self.call(&json!({"id": id, "op": "sql", "query": query}))
    }
}

fn write(ws: &Path, rel: &str, body: &str) {
    let p = ws.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

/// The § A.11 lifecycle, one workspace: cap advertised; a read answers
/// FRESH with rows at the engine's pin; a corpus move appends delta-grain
/// (pin ledger observable through the op itself); a failed query answers a
/// SUCCESS body with `state: UNVERIFIED` + the engine's words; view-DML
/// refuses with the OQ1 teaching; the agent sandbox blocks external access.
#[test]
fn sql_lifecycle_over_the_wire() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ws");
    write(&ws, "a.md", "# A\n\nsee [[b]]\n");
    write(&ws, "b.md", "# B\n");

    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    let hi = conn.hello_v3(&ws);
    assert_eq!(hi["ok"], true, "hello: {hi}");
    let caps: Vec<&str> = hi["body"]["caps"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(caps.contains(&"sql"), "v3 hello advertises sql: {caps:?}");

    // A read at the warm pin: FRESH, rows, typed columns.
    let read = conn.sql(1, "SELECT path FROM doc ORDER BY path");
    assert_eq!(read["ok"], true, "sql read: {read}");
    let body = &read["body"];
    assert_eq!(
        body["state"], "FRESH_AT_SAMPLE",
        "warm pin is fresh: {body}"
    );
    assert_eq!(body["rows"], json!([["a.md"], ["b.md"]]));
    assert_eq!(body["row_count"], 2);
    assert_eq!(
        body["columns"],
        json!([{"name": "path", "type": "VARCHAR"}])
    );
    let as_of = body["as_of_fingerprint"].as_str().unwrap();
    assert!(as_of.starts_with("b3"), "the pin is a real fold: {as_of}");
    assert_eq!(body["live"].as_str().unwrap(), as_of);

    // The file was cold-built once: pin ledger at gen 1.
    let pins = conn.sql(2, "SELECT count(*) FROM hist.pin");
    assert_eq!(pins["body"]["rows"], json!([[1]]), "{pins}");

    // Move the corpus: the next call re-warms and appends delta-grain.
    write(&ws, "a.md", "# A moved\n\nsee [[b]]\n");
    let after = conn.sql(
        3,
        "SELECT (SELECT count(*) FROM hist.pin), (SELECT count(*) FROM hist.doc WHERE gen = 2)",
    );
    assert_eq!(after["ok"], true, "{after}");
    assert_eq!(
        after["body"]["rows"],
        json!([[2, 1]]),
        "one moved file = one appended doc version: {after}"
    );
    assert_eq!(after["body"]["state"], "FRESH_AT_SAMPLE");

    // A failed query is a SUCCESS body: UNVERIFIED + the engine's words.
    let failed = conn.sql(4, "SELECT nope FROM missing");
    assert_eq!(
        failed["ok"], true,
        "the frame is the honest report: {failed}"
    );
    assert_eq!(failed["body"]["state"], "UNVERIFIED");
    assert!(
        failed["body"]["error"]
            .as_str()
            .unwrap()
            .contains("missing"),
        "engine words verbatim: {failed}"
    );

    // View-DML refuses with the OQ1 teaching (the hist lane named).
    let dml = conn.sql(5, "UPDATE doc SET bytes = 0");
    assert_eq!(dml["ok"], true);
    assert!(
        dml["body"]["error"].as_str().unwrap().contains("hist"),
        "the refusal teaches: {dml}"
    );

    // The wire is the untrusted lane: agent sandbox, external access off.
    let escape = conn.sql(6, "SELECT * FROM read_csv('/etc/hosts')");
    assert_eq!(escape["ok"], true);
    assert!(
        escape["body"]["error"].is_string(),
        "external access refused under the agent profile: {escape}"
    );
}

/// v2 sessions never see the op (§3.2 discovery honesty): not in v2 caps,
/// and a v2 frame answers `unknown_op`.
#[test]
fn v2_session_answers_unknown_op_and_never_advertises_sql() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ws");
    write(&ws, "a.md", "# A\n");

    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut v2 = Conn::open(server.socket_path());
    let hi = v2.call(&json!({
        "op": "hello", "proto": 1,
        "workspace": ws.to_str().unwrap(),
    }));
    assert_eq!(hi["ok"], true, "v2 hello: {hi}");
    let caps: Vec<&str> = hi["body"]["caps"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        !caps.contains(&"sql"),
        "frozen v2 caps never grow: {caps:?}"
    );

    let refused = v2.sql(1, "SELECT 1");
    assert_eq!(refused["ok"], false);
    assert_eq!(refused["error"]["code"], "unknown_op", "{refused}");
}

/// The strict field wall: a `sql` frame carrying any field beyond `query`
/// refuses `bad_request` — profile, cwd, and row bounds are host concerns.
#[test]
fn sql_strict_field_wall_refuses_extras() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ws");
    write(&ws, "a.md", "# A\n");

    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    conn.hello_v3(&ws);

    let refused = conn.call(&json!({
        "id": 1, "op": "sql", "query": "SELECT 1", "execution_profile": "local",
    }));
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "bad_request");
}
