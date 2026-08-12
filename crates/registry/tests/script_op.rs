//! E2E gates for the § A.7 `script` op at the daemon door: `hello` binds and
//! negotiates v3, one `script` frame evaluates IN-PROCESS against the pinned
//! entry world, the commit rides the shared choke-point, and the next read
//! reflects it. A v2 session answers `unknown_op` — an op is in `caps` or
//! answers `unknown_op`, never both.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use registry::{Config, RunningServer};
use serde_json::{Value, json};
use tempfile::TempDir;

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

    fn hello(&mut self, ws: &Path, contract: &str) -> Value {
        self.call(&json!({
            "op": "hello",
            "proto": 1,
            "contract": contract,
            "workspace": ws.to_str().unwrap(),
        }))
    }
}

const NOTE: &str = "---\nstatus: open\n---\n# Alpha\n\nalpha body\n";

/// One `script` frame reads, arms, and commits in-process; the entry
/// fingerprint is the hello's; the next read reflects the commit; the trace
/// rides the body in v3 vocabulary with its embedded §4.4 leg.
#[test]
fn a_script_frame_evaluates_in_process_and_commits() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("note.md", NOTE)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    let hello = conn.hello(&ws, "v3");
    assert_eq!(hello["ok"], json!(true), "{hello}");
    let entry = hello["body"]["fingerprint"].as_str().unwrap().to_owned();
    let caps: Vec<&str> = hello["body"]["caps"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        caps.contains(&"script"),
        "v3 caps advertise the op: {caps:?}"
    );

    let response = conn.call(&json!({
        "id": 9,
        "op": "script",
        "source": "t = read(\"note.md\")\nput(\"note.md\", props={\"status\": \"done\"})\n",
        "actor": "agent:e2e",
        "now": "2026-08-12T00:00:00Z",
    }));
    assert_eq!(response["ok"], json!(true), "{response}");
    let body = &response["body"];
    assert_eq!(body["outcome"], json!("committed"), "{body}");
    assert_eq!(
        body["entry_fingerprint"].as_str().unwrap(),
        entry,
        "the entry fingerprint IS the pinned world's — the hello's own"
    );
    // The embedded commit leg is the §4.4 splice response in v3 vocabulary.
    let commit = &body["commit"];
    assert!(commit["fingerprint_after"].is_string(), "{commit}");
    assert_eq!(commit["armed"]["path"], json!("note.md"));
    // The armed digest rides with its domain tag.
    assert!(
        body["armed_digest"]
            .as_str()
            .unwrap()
            .starts_with("armed-set-path-edit:sha256:"),
        "{body}"
    );
    // Telemetry is unconditional.
    assert!(body["telemetry"]["reads_used"].as_u64().unwrap() >= 1);

    // The next read reflects the commit — same daemon, same connection.
    let read = conn.call(&json!({"id": 10, "op": "read", "path": "note.md"}));
    assert_eq!(read["ok"], json!(true), "{read}");
    let props = read["body"]["props"].as_array().unwrap();
    let status = props
        .iter()
        .find(|p| p["key"] == json!("status"))
        .expect("status prop");
    assert_eq!(status["value"], json!("done"), "{props:?}");
}

/// A v2 session answers `unknown_op` — the op is v3-only at dispatch, and the
/// frozen v2 caps never carry it.
#[test]
fn a_v2_session_answers_unknown_op() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("note.md", NOTE)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    let hello = conn.hello(&ws, "v2");
    assert_eq!(hello["ok"], json!(true));
    let caps: Vec<&str> = hello["body"]["caps"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        !caps.contains(&"script"),
        "frozen v2 caps stay byte-identical"
    );

    let response = conn.call(&json!({
        "id": 3,
        "op": "script",
        "source": "t = 1\n",
    }));
    assert_eq!(response["ok"], json!(false), "{response}");
    assert_eq!(response["error"]["code"], json!("unknown_op"));
}

/// The strict wall reaches the request's nested grains: an unknown field on
/// `budgets` refuses loud (§3.2 grain law), naming the legal set.
#[test]
fn an_unknown_budgets_field_refuses_loud() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("note.md", NOTE)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    conn.hello(&ws, "v3");

    let response = conn.call(&json!({
        "id": 4,
        "op": "script",
        "source": "t = 1\n",
        "budgets": {"fuel": 1000, "walls": 7},
    }));
    assert_eq!(response["ok"], json!(false), "{response}");
    assert_eq!(response["error"]["code"], json!("bad_request"));
}

/// The caller's `if_fingerprint` rides the v3 rename onto the guard: a stale
/// pin refuses pre-eval as `conflict` with `guard_expected` carried.
#[test]
fn a_stale_caller_guard_refuses_pre_eval_over_the_wire() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("note.md", NOTE)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    conn.hello(&ws, "v3");

    let response = conn.call(&json!({
        "id": 5,
        "op": "script",
        "source": "t = read(\"note.md\")\n",
        "if_fingerprint": "b3:0000000000000000000000000000000000000000000000000000000000000000",
    }));
    assert_eq!(response["ok"], json!(true), "{response}");
    let body = &response["body"];
    assert_eq!(body["outcome"], json!("conflict"));
    assert_eq!(
        body["guard_expected"],
        json!("b3:0000000000000000000000000000000000000000000000000000000000000000")
    );
    assert!(
        body["trace"].as_array().unwrap().is_empty(),
        "zero evaluation"
    );
}

/// The arm/commit split over the wire: leg one is `dry:true` (rehearsal,
/// nothing lands), leg two pins `if_fingerprint` + `expect_armed` from the
/// arm's own trace and commits — the courier property end-to-end.
#[test]
fn the_arm_commit_split_rides_the_wire() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("note.md", NOTE)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    conn.hello(&ws, "v3");

    let source = "t = read(\"note.md\")\nput(\"note.md\", props={\"status\": \"done\"})\n";
    let arm = conn.call(&json!({
        "id": 6, "op": "script", "source": source, "dry": true,
        "actor": "agent:e2e", "now": "2026-08-12T00:00:00Z",
    }));
    assert_eq!(arm["body"]["outcome"], json!("no_effect"), "{arm}");
    assert_eq!(arm["body"]["commit"]["dry"], json!(true));
    let entry = arm["body"]["entry_fingerprint"].as_str().unwrap();
    let digest = arm["body"]["armed_digest"].as_str().unwrap();

    let commit = conn.call(&json!({
        "id": 7, "op": "script", "source": source,
        "if_fingerprint": entry, "expect_armed": digest,
        "actor": "agent:e2e", "now": "2026-08-12T00:00:00Z",
    }));
    assert_eq!(commit["body"]["outcome"], json!("committed"), "{commit}");
}
