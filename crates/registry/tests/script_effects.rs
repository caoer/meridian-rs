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
//!   a dewey selector serves;
//! - `token_count` (leg B of the `token_count` ruling, 2026-08-13): declared
//!   `effects:["token_count"]` admits the builtin, which measures NOW through
//!   the NDJSON endpoint the frame's `token_count_endpoint` names — the
//!   engine never counts tokens itself; undeclared use is an unbound name,
//!   no endpoint refuses "unbound", the endpoint's own error travels whole,
//!   and the dial deadline caps at the remaining wall clock.

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

// ---------------------------------------------------------------------------
// token_count — leg B of the token_count ruling (2026-08-13). The builtin is
// a socket call wearing a function: the engine holds no tokenizer and no
// credentials, so `token_count(text)` dials the NDJSON endpoint the frame's
// `token_count_endpoint` names (the ccc-statusd daemon.sock token_count verb,
// identityless default) and answers the count as an int.
// ---------------------------------------------------------------------------

/// One fake harness endpoint: a unix listener answering the daemon.sock
/// `token_count` wire. `script` decides the answer from the received frame;
/// received lines are collected for assertion.
fn fake_harness(
    dir: &Path,
    answer: impl Fn(&Value) -> Option<String> + Send + 'static,
) -> (PathBuf, std::sync::mpsc::Receiver<Value>) {
    let socket = dir.join("harness.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { return };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() || line.is_empty() {
                continue;
            }
            let frame: Value = serde_json::from_str(&line).unwrap();
            let reply = answer(&frame);
            let _ = tx.send(frame);
            if let Some(mut reply) = reply {
                reply.push('\n');
                let mut w = stream;
                let _ = w.write_all(reply.as_bytes());
            }
            // No reply: hold the connection open — the deadline pin.
            else {
                std::thread::sleep(Duration::from_secs(30));
            }
        }
    });
    (socket, rx)
}

/// A live `token_count` submission bound to `endpoint`.
fn live_tc(id: u64, source: &str, endpoint: &Path) -> Value {
    json!({"id": id, "op": "script", "source": source,
           "effects": ["token_count"], "invocation": "scr-tc",
           "token_count_endpoint": endpoint.to_str().unwrap()})
}

/// Undeclared use is an unbound name: a submission that did not admit
/// `token_count` has no such global — on the pure model and on a live
/// program that admitted only `run`.
#[test]
fn token_count_is_not_a_global_unless_declared() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);

    let frames = vec![
        json!({"id": 30, "op": "script", "source": "n = token_count(\"hi\")\n"}),
        live(31, "n = token_count(\"hi\")\n"),
    ];
    for frame in frames {
        let trace = trace_of(&conn.call(&frame));
        assert_eq!(trace["outcome"], json!("fault"), "{trace}");
        assert!(
            trace["fault"]["reason"]
                .as_str()
                .unwrap()
                .contains("token_count"),
            "the fault names the unbound name: {trace}"
        );
    }
}

/// The effect declared but no endpoint on the frame: the call refuses
/// "unbound" — only a lane handed an endpoint can measure.
#[test]
fn token_count_without_endpoint_refuses_unbound() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);

    let trace = trace_of(&conn.call(&json!({
        "id": 32, "op": "script", "source": "n = token_count(\"hi\")\n",
        "effects": ["token_count"], "invocation": "scr-tc"})));
    assert_eq!(trace["outcome"], json!("fault"), "{trace}");
    assert!(
        trace["fault"]["reason"]
            .as_str()
            .unwrap()
            .contains("unbound"),
        "the refusal says unbound: {trace}"
    );
}

/// The ruled flow: a declared effect, a bound endpoint, a live measurement —
/// the count comes back as an int the program computes with, and the wire
/// frame the endpoint received is the daemon.sock `token_count` verb's
/// identityless default.
#[test]
fn token_count_measures_live_through_the_endpoint() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let (endpoint, rx) = fake_harness(tmp.path(), |frame| {
        let chars = frame["text"].as_str().unwrap().len();
        Some(format!(
            "{{\"type\":\"response\",\"data\":{{\"tokens\":42,\"chars\":{chars},\"session\":\"fake\",\"model\":\"fake-tokenizer\"}}}}"
        ))
    });
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);

    let src = "n = token_count(\"hello tokens\")\nfits = n < 100\n";
    let trace = trace_of(&conn.call(&live_tc(33, src, &endpoint)));
    assert_eq!(trace["outcome"], json!("effects"), "{trace}");
    assert_eq!(trace["bindings"]["n"], json!("42"), "{trace}");
    assert_eq!(trace["bindings"]["fits"], json!("True"), "{trace}");

    let seen = rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(seen["type"], json!("token_count"), "{seen}");
    assert_eq!(seen["text"], json!("hello tokens"), "{seen}");
    assert!(
        seen.get("session_id").is_none(),
        "identityless default — the daemon picks the instrument: {seen}"
    );
}

/// The endpoint's own refusal (no live instrument, oversize, …) travels
/// whole: the program faults with the endpoint's words, not a paraphrase.
#[test]
fn token_count_endpoint_error_travels_whole() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let (endpoint, _rx) = fake_harness(tmp.path(), |_| {
        Some(
            "{\"type\":\"response\",\"error\":\"no live Claude Code session to measure through\"}"
                .to_owned(),
        )
    });
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);

    let trace = trace_of(&conn.call(&live_tc(34, "n = token_count(\"hi\")\n", &endpoint)));
    assert_eq!(trace["outcome"], json!("fault"), "{trace}");
    assert!(
        trace["fault"]["reason"]
            .as_str()
            .unwrap()
            .contains("no live Claude Code session"),
        "the endpoint's words travel whole: {trace}"
    );
}

/// The dial deadline caps at the REMAINING wall clock: an endpoint that
/// never answers faults the program when the script clock lapses — the call
/// never outlives the entry's own budget.
#[test]
fn token_count_deadline_caps_at_remaining_wall_clock() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let (endpoint, _rx) = fake_harness(tmp.path(), |_| None);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);

    let started = std::time::Instant::now();
    let trace = trace_of(&conn.call(&live_tc(35, "n = token_count(\"hi\")\n", &endpoint)));
    let elapsed = started.elapsed();
    assert_eq!(trace["outcome"], json!("fault"), "{trace}");
    assert!(
        trace["fault"]["reason"]
            .as_str()
            .unwrap()
            .contains("wall clock"),
        "the fault names the clock: {trace}"
    );
    assert!(
        elapsed < Duration::from_secs(15),
        "the wall clock bound the wait: {elapsed:?}"
    );
}

/// The endpoint field walls: `token_count_endpoint` rides the `token_count`
/// effect only — orphan on a pure script, orphan beside other effects, and
/// an explicit empty endpoint all refuse at decode.
#[test]
fn token_count_endpoint_decode_walls() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);

    let cases: Vec<(Value, &str)> = vec![
        (
            json!({"id": 40, "op": "script", "source": "x = 1\n",
                   "token_count_endpoint": "/tmp/x.sock"}),
            "orphan endpoint on a pure script",
        ),
        (
            json!({"id": 41, "op": "script", "source": "x = 1\n",
                   "effects": ["run"], "invocation": "s",
                   "token_count_endpoint": "/tmp/x.sock"}),
            "endpoint without the token_count effect",
        ),
        (
            json!({"id": 42, "op": "script", "source": "x = 1\n",
                   "effects": ["token_count"], "invocation": "s",
                   "token_count_endpoint": ""}),
            "explicit empty endpoint",
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
}

// ---------------------------------------------------------------------------
// Delta honesty (§ A.7 effects paragraph, run-delta ruling 2026-08-14):
// a live run() mints per committed batch through the run plane's delta sink,
// exactly as at § A.8 — the script door and the op door share the seam.
// ---------------------------------------------------------------------------

const PUSH_WAIT: Duration = Duration::from_secs(10);

/// A live `run()` inside a script pushes the run plane's attributed frames to a
/// subscriber: the script's actor threads through the shared row seam into
/// the frames the bash task's two commits mint.
///
/// RED before the ruling's code: the run lands, the fingerprint advances, and
/// the subscriber sees only actorless detector-cadence external change.
#[test]
fn a_live_run_pushes_attributed_frames() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    let mut sub = Conn::open(&test_config(&tmp).socket_path);
    assert_eq!(
        sub.call(&json!({
            "op": "hello", "proto": 1,
            "workspace": ws.to_str().unwrap(),
        }))["ok"],
        json!(true)
    );
    sub.writer.set_read_timeout(Some(PUSH_WAIT)).unwrap();
    assert_eq!(sub.call(&json!({"op": "sub"}))["ok"], json!(true));

    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);
    let resp = conn.call(&json!({
        "id": 41, "op": "script", "effects": ["run"], "invocation": "scr-d1",
        "actor": "seat-79",
        "source": "r = run(\"tasks.md\", task=\"sh-note\")\n",
    }));
    assert_eq!(trace_of(&resp)["outcome"], json!("effects"), "{resp}");

    // The bash task commits twice (pre-exec receipt, completion receipt);
    // both frames arrive attributed with the script's actor.
    for phase in ["phase 1", "phase 2"] {
        let mut line = String::new();
        let n = sub.reader.read_line(&mut line).unwrap_or(0);
        assert!(n > 0, "{phase}: a frame arrives");
        let frame: Value = serde_json::from_str(&line).expect("notification is JSON");
        assert_eq!(
            frame["delta"]["actor"],
            json!("seat-79"),
            "{phase}: the script's actor threads into the run frame: {frame}"
        );
    }
    server.shutdown();
}
