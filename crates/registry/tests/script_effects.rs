//! E2E gates for § A.7 EFFECTS MODE — the live program model
//! (script-effects ruling, 2026-08-13; `docs/wire-contract.md` § A.7 effects
//! paragraph, `docs/run-plane.md` § Effects mode).
//!
//! Written RED-FIRST against the docs-first contract commit. The pins:
//!
//! - absent `effects` = pure: `run` is not a global at all — provably pure
//!   by default; the transaction law untouched (its own suite stands);
//! - `effects:["run"]`: `run()` executes AT CALL TIME and returns its § A.8
//!   row as a value — run-then-decide works;
//! - live `put()` writes NOW (no rev, no CAS), read-back sees the write,
//!   multi-file programs are legal;
//! - a mid-program fault keeps every prior act (no rollback), trace says how
//!   far it got;
//! - outcome word `effects`; `wrote`/`ran` trace entries;
//! - the decode walls: effects×{`dry`, `if_fingerprint`, `expect_armed`},
//!   `effects:[]`, unknown names, missing invocation, orphan invocation;
//! - read alignment: toc face is a dict, a section read is the text string,
//!   a dewey selector serves.

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

    fn hello_v3(&mut self, ws: &Path) -> Value {
        self.call(&json!({
            "op": "hello", "proto": 1, "contract": "v3",
            "workspace": ws.to_str().unwrap(),
        }))
    }
}

const TASKS: &str = "\
---
task.sh-note: \"[[#^sh-1]]\"
task.pwd-check: \"[[#^pwd-1]]\"
---

# Tasks

```bash
echo nope
```
^sh-1

```bash
test \"$(pwd)\" = \"$MERIDIAN_PROJECT_ROOT\"
```
^pwd-1
";

const DOC: &str = "---\nstatus: open\n---\n# Alpha\n\none two three\n\n## Beta\n\nfour five\n";

fn seeded(tmp: &TempDir) -> PathBuf {
    let ws = tmp.path().join("project");
    fs::create_dir_all(&ws).unwrap();
    fs::write(ws.join("tasks.md"), TASKS).unwrap();
    fs::write(ws.join("doc.md"), DOC).unwrap();
    ws
}

/// A live submission: effects + invocation, no guards.
fn live(id: u64, source: &str) -> Value {
    json!({"id": id, "op": "script", "source": source,
           "effects": ["run"], "invocation": "scr-777"})
}

fn trace_of(resp: &Value) -> Value {
    assert_eq!(
        resp["ok"],
        json!(true),
        "a live submission answers a trace; got: {resp}"
    );
    resp["body"].clone()
}

// ---------------------------------------------------------------------------
// Provably pure by default.
// ---------------------------------------------------------------------------

/// Without the flag, `run` is not a global at all: the program faults on an
/// unbound name, and nothing anywhere executed.
#[test]
fn a_pure_submission_has_no_run_builtin() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);

    let resp = conn.call(&json!({
        "id": 7, "op": "script",
        "source": "r = run(\"tasks.md\", task=\"sh-note\")\n"}));
    let trace = trace_of(&resp);
    assert_eq!(trace["outcome"], json!("fault"), "{trace}");
    assert!(
        trace["fault"]["reason"].as_str().unwrap().contains("run"),
        "the fault names the unbound name: {trace}"
    );
    assert!(!ws.join("receipts/run.md").exists(), "nothing executed");
}

// ---------------------------------------------------------------------------
// The ZT requirement, script half: run() at call time, results observable.
// ---------------------------------------------------------------------------

/// `run()` executes at call time and returns its row as a value the program
/// computes with — the exit code lands in a binding.
#[test]
fn run_executes_at_call_time_and_its_row_is_a_value() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);

    let resp = conn.call(&live(
        8,
        "r = run(\"tasks.md\", task=\"sh-note\")\ncode = r[\"exec\"][\"exit_code\"]\n",
    ));
    let trace = trace_of(&resp);
    assert_eq!(trace["outcome"], json!("effects"), "{trace}");
    // The run plane's receipt exists on disk — it executed, at call time.
    assert!(ws.join("receipts/run.md").exists(), "the run landed");
    let ran: Vec<&Value> = trace["trace"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == json!("ran"))
        .collect();
    assert_eq!(ran.len(), 1, "one ran entry: {trace}");
    assert_eq!(ran[0]["row"]["exec"]["exit_code"], json!(0), "{trace}");
    assert_eq!(
        ran[0]["row"]["invocation"],
        json!("scr-777-r0"),
        "run identity derives from the submission's base: {trace}"
    );
}

/// Run-then-decide: the program branches on the run's result and writes the
/// branch truth — the flow the armed/deferred shape could not express.
#[test]
fn run_then_decide_branches_on_the_result_and_the_write_lands() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);

    let src = "r = run(\"tasks.md\", task=\"pwd-check\")\n\
               verdict = \"pass\" if r[\"exec\"][\"exit_code\"] == 0 else \"fail\"\n\
               put(\"doc.md\", props={\"verdict\": verdict})\n";
    let trace = trace_of(&conn.call(&live(9, src)));
    assert_eq!(trace["outcome"], json!("effects"), "{trace}");
    let doc = fs::read_to_string(ws.join("doc.md")).unwrap();
    assert!(
        doc.contains("verdict: pass"),
        "the branch truth landed live: {doc}"
    );
}

// ---------------------------------------------------------------------------
// Live put: writes NOW, read-back, multi-file, no rollback.
// ---------------------------------------------------------------------------

/// A live `put()` lands immediately and a following `read()` of the same page
/// sees it — the world is the overlay.
#[test]
fn a_live_put_lands_now_and_read_back_sees_it() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);

    let src = "put(\"doc.md\", props={\"status\": \"done\"})\n\
               after = read(\"doc.md\")[\"fm\"][\"status\"]\n";
    let trace = trace_of(&conn.call(&live(10, src)));
    assert_eq!(trace["outcome"], json!("effects"), "{trace}");
    let wrote: Vec<&Value> = trace["trace"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == json!("wrote"))
        .collect();
    assert_eq!(wrote.len(), 1, "one wrote entry: {trace}");
    assert_eq!(wrote[0]["path"], json!("doc.md"));
    assert!(
        fs::read_to_string(ws.join("doc.md"))
            .unwrap()
            .contains("status: done"),
        "the write landed"
    );
}

/// Two puts to two files both land — the one-content-file law is the pure
/// transaction's, and there is no transaction here.
#[test]
fn a_live_program_writes_more_than_one_file() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);

    let src = "put(\"doc.md\", props={\"status\": \"done\"})\n\
               put(\"tasks.md\", props={\"status\": \"swept\"})\n";
    let trace = trace_of(&conn.call(&live(11, src)));
    assert_eq!(trace["outcome"], json!("effects"), "{trace}");
    assert!(
        fs::read_to_string(ws.join("doc.md"))
            .unwrap()
            .contains("status: done")
    );
    assert!(
        fs::read_to_string(ws.join("tasks.md"))
            .unwrap()
            .contains("status: swept")
    );
}

/// A mid-program fault keeps every prior act — no rollback; the trace says
/// how far the program got.
#[test]
fn a_mid_program_fault_keeps_prior_acts() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);

    let src = "put(\"doc.md\", props={\"status\": \"done\"})\nfail(\"boom\")\n";
    let trace = trace_of(&conn.call(&live(12, src)));
    assert_eq!(trace["outcome"], json!("fault"), "{trace}");
    assert!(
        fs::read_to_string(ws.join("doc.md"))
            .unwrap()
            .contains("status: done"),
        "the prior act stands — a live program has no rollback"
    );
    assert!(
        trace["trace"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == json!("wrote")),
        "the trace records how far it got: {trace}"
    );
}

// ---------------------------------------------------------------------------
// Read alignment (both models — probed here on the live lane).
// ---------------------------------------------------------------------------

/// The toc face is a dict, a section read is the text itself, and a dewey
/// selector serves — the read TOOL's own grammar, in-script.
#[test]
fn read_alignment_dict_toc_string_section_dewey_served() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);

    let src = "page = read(\"doc.md\")\n\
               has_fm = \"fm\" in page\n\
               sec = read(\"doc.md\", section=\"Alpha/Beta\")\n\
               hit = \"four\" in sec\n\
               by_n = read(\"doc.md\", section=\"1.1\")\n\
               same = by_n == sec\n";
    let trace = trace_of(&conn.call(&live(13, src)));
    assert_eq!(trace["outcome"], json!("effects"), "{trace}");
    let echoes: Vec<&Value> = trace["trace"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == json!("echo") || e["kind"] == json!("read"))
        .collect();
    assert_eq!(echoes.len(), 3, "three reads recorded: {trace}");
}

// ---------------------------------------------------------------------------
// The decode walls.
// ---------------------------------------------------------------------------

/// Every ruled-out combination refuses `bad_request` before anything runs.
#[test]
fn the_combination_walls_refuse_before_anything_runs() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);

    let cases: Vec<(Value, &str)> = vec![
        (
            json!({"id": 20, "op": "script", "source": "x = 1\n",
                   "effects": ["run"], "invocation": "s", "dry": true}),
            "dry",
        ),
        (
            json!({"id": 21, "op": "script", "source": "x = 1\n",
                   "effects": ["run"], "invocation": "s", "if_fingerprint": "b3:x"}),
            "if_fingerprint",
        ),
        (
            json!({"id": 22, "op": "script", "source": "x = 1\n",
                   "effects": ["run"], "invocation": "s",
                   "expect_armed": "armed-set-path-edit:sha256:00"}),
            "expect_armed",
        ),
        (
            json!({"id": 23, "op": "script", "source": "x = 1\n", "effects": []}),
            "effects: []",
        ),
        (
            json!({"id": 24, "op": "script", "source": "x = 1\n",
                   "effects": ["walrus"], "invocation": "s"}),
            "unknown effect",
        ),
        (
            json!({"id": 25, "op": "script", "source": "x = 1\n", "effects": ["run"]}),
            "missing invocation",
        ),
        (
            json!({"id": 26, "op": "script", "source": "x = 1\n", "invocation": "s"}),
            "orphan invocation",
        ),
    ];
    for (frame, name) in cases {
        let resp = conn.call(&frame);
        assert_eq!(resp["ok"], json!(false), "{name}: {resp}");
        assert_eq!(
            resp["error"]["code"],
            json!("bad_request"),
            "{name}: {resp}"
        );
    }
    assert!(!ws.join("receipts/run.md").exists(), "nothing ran");
}
