//! Dry/live parity gates for the § A.8 `run` op — dogfood r2 F2
//! (run-dry-parity): a rehearsal must run EVERY gate the live run enforces
//! (addressing, contract/arity, capability), answer in the SAME grammar, and
//! never leak interpreter internals for an input a gate refuses.
//!
//! Written RED-FIRST against the F2 defect: on the pre-fix engine `dry`
//! rehearses addressing only — a contract-violating or capability-denied
//! target answers a green rehearsal (or a raw Starlark traceback) where the
//! live call answers a typed refusal.
//!
//! The law, pinned pairwise: one frame carries the SAME faulty target twice,
//! `dry:true` then live. The two rows must carry EQUAL refusal objects —
//! class and reason byte-identical — so rehearsal-green predicts live-green
//! by construction.

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

    fn fingerprint(&mut self) -> String {
        let resp = self.call(&json!({"id": 90, "op": "fingerprint"}));
        assert_eq!(resp["ok"], json!(true), "fingerprint op: {resp}");
        resp["body"]["fingerprint"].as_str().unwrap().to_owned()
    }
}

/// The F2 fixture, one page: a granted task with a one-arg contract, a
/// capability-less task that still emits `set_field`, a granted task whose
/// name sits under the builtin `check-*` read-only ceiling, a bash task with
/// an empty contract, and a dangling binding.
const F2_TASKS: &str = "\
---
task.grant: \"[[#^g-1]]\"
task.grant.caps: md.edit
task.grant.args: value
task.nocaps: \"[[#^n-1]]\"
task.check-granted: \"[[#^c-1]]\"
task.check-granted.caps: md.edit
task.sh: \"[[#^s-1]]\"
task.dangling: \"[[#^nowhere]]\"
---

# Tasks

```starlark
def run(ctx):
    set_field(field = \"status\", value = ctx.args[0])
```
^g-1

```starlark
def run(ctx):
    set_field(field = \"status\", value = \"x\")
```
^n-1

```starlark
def run(ctx):
    set_field(field = \"status\", value = \"y\")
```
^c-1

```bash
echo hi
```
^s-1
";

fn seeded(tmp: &TempDir) -> PathBuf {
    let ws = tmp.path().join("project");
    fs::create_dir_all(&ws).unwrap();
    fs::write(ws.join("tasks.md"), F2_TASKS).unwrap();
    ws
}

fn run_frame(id: u64, invocation: &str, targets: Value) -> Value {
    let mut frame = json!({"id": id, "op": "run", "invocation": invocation});
    frame["targets"] = targets;
    frame
}

fn rows_of(resp: &Value) -> Vec<Value> {
    assert_eq!(
        resp["ok"],
        json!(true),
        "the run op answers rows whenever it reached the plane; got: {resp}"
    );
    resp["body"]["targets"].as_array().unwrap().clone()
}

/// Send one faulty target twice — rehearsed, then live — and return the two
/// rows. The pair rides ONE frame: per-target independence keeps the rows
/// independent, and both targets refuse, so nothing lands between them.
fn paired_rows(ws: &Path, socket: &Path, target: &Value) -> (Value, Value) {
    let mut conn = Conn::open(socket);
    conn.hello_v3(ws);
    let mut dry = target.clone();
    dry["dry"] = json!(true);
    let rows = rows_of(&conn.call(&run_frame(31, "run-parity-1", json!([dry, target]))));
    assert_eq!(rows.len(), 2, "one row per target: {rows:?}");
    (rows[0].clone(), rows[1].clone())
}

/// The parity law on one pair: the dry row REFUSES, and its refusal object
/// (class + reason, byte-identical) equals the live row's.
fn assert_refusal_parity(dry: &Value, live: &Value, gate: &str) {
    assert!(
        live["refusal"].is_object(),
        "{gate}: the live row must refuse — the fixture is wrong otherwise: {live}"
    );
    assert!(
        dry["refusal"].is_object(),
        "{gate}: the rehearsal passed what the live call refuses ({}): {dry}",
        live["refusal"]["reason"]
    );
    assert_eq!(
        dry["refusal"], live["refusal"],
        "{gate}: one gate, one grammar — the dry refusal must equal the live refusal"
    );
}

// ---------------------------------------------------------------------------
// F2's demonstrated pairs, each a gate class.
// ---------------------------------------------------------------------------

/// Arity: `grant` takes one arg; three supplied. Live refuses on the
/// invocation class — the rehearsal must answer the identical refusal.
#[test]
fn arity_violation_refuses_identically_dry_and_live() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let (dry, live) = paired_rows(
        &ws,
        &test_config(&tmp).socket_path,
        &json!({"page": "tasks.md", "task": "grant", "args": ["a", "b", "c"]}),
    );
    assert_refusal_parity(&dry, &live, "arity");
    assert_eq!(dry["refusal"]["class"], json!("invocation"));
    assert!(
        dry["refusal"]["reason"]
            .as_str()
            .unwrap()
            .contains("takes 1 arg(s) (value), got 3"),
        "the typed contract violation, verbatim: {dry}"
    );
}

/// The sharper half of F2: `args: []` under dry used to slip PAST the arity
/// gate into eval and answer a raw Starlark traceback (`Index 0 is out of
/// bound`) for what is a contract fault. The rehearsal must answer the
/// contract refusal — and never interpreter internals.
#[test]
fn empty_args_answer_the_contract_refusal_never_a_traceback() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let (dry, live) = paired_rows(
        &ws,
        &test_config(&tmp).socket_path,
        &json!({"page": "tasks.md", "task": "grant", "args": []}),
    );
    assert_refusal_parity(&dry, &live, "arity/empty");
    assert_eq!(dry["refusal"]["class"], json!("invocation"));
    let reason = dry["refusal"]["reason"].as_str().unwrap();
    assert!(
        reason.contains("takes 1 arg(s) (value), got 0"),
        "the clean typed refusal: {reason}"
    );
    assert!(
        !reason.contains("Traceback"),
        "no interpreter internals on a gate-refused input: {reason}"
    );
}

/// Env contract: `sh` declares no env; supplying one refuses on the live
/// call. The rehearsal must answer the identical refusal.
#[test]
fn undeclared_env_refuses_identically_dry_and_live() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let (dry, live) = paired_rows(
        &ws,
        &test_config(&tmp).socket_path,
        &json!({"page": "tasks.md", "task": "sh", "env": {"FOO": "bar"}}),
    );
    assert_refusal_parity(&dry, &live, "env");
    assert_eq!(dry["refusal"]["class"], json!("invocation"));
    assert!(
        dry["refusal"]["reason"]
            .as_str()
            .unwrap()
            .contains("does not declare env 'FOO'"),
        "the typed contract violation, verbatim: {dry}"
    );
}

/// Capability, deny-by-default: `nocaps` declares nothing and emits
/// `set_field`. Live refuses at the executor's choke point — the rehearsal
/// must run the SAME admission and answer the identical refusal.
#[test]
fn capability_denial_refuses_identically_dry_and_live() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let (dry, live) = paired_rows(
        &ws,
        &test_config(&tmp).socket_path,
        &json!({"page": "tasks.md", "task": "nocaps"}),
    );
    assert_refusal_parity(&dry, &live, "caps");
    assert_eq!(dry["refusal"]["class"], json!("run"));
    assert!(
        dry["refusal"]["reason"]
            .as_str()
            .unwrap()
            .contains("capability denied: md.edit on 'tasks.md'"),
        "the choke point's own words: {dry}"
    );
}

/// Capability, ceiling-narrowed: `check-granted` declares `md.set_field`,
/// and the builtin `check-*` read-only ceiling narrows it away. The
/// rehearsal must answer the same denial WITH the ceiling teaching.
#[test]
fn ceiling_denial_refuses_identically_dry_and_live() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let (dry, live) = paired_rows(
        &ws,
        &test_config(&tmp).socket_path,
        &json!({"page": "tasks.md", "task": "check-granted"}),
    );
    assert_refusal_parity(&dry, &live, "ceiling");
    assert_eq!(dry["refusal"]["class"], json!("run"));
    assert!(
        dry["refusal"]["reason"]
            .as_str()
            .unwrap()
            .contains("narrowed away by"),
        "the ceiling is named — the only remedy that repairs this denial: {dry}"
    );
}

/// Addressing, the gate F2 measured as already correct: a dangling binding
/// refuses identically on both tenses. Pinned so the fixed rehearsal never
/// regresses the one gate that worked.
#[test]
fn dangling_binding_refuses_identically_dry_and_live() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let (dry, live) = paired_rows(
        &ws,
        &test_config(&tmp).socket_path,
        &json!({"page": "tasks.md", "task": "dangling"}),
    );
    assert_refusal_parity(&dry, &live, "addressing");
    assert_eq!(dry["refusal"]["class"], json!("invocation"));
}

// ---------------------------------------------------------------------------
// The rehearsal stays a rehearsal.
// ---------------------------------------------------------------------------

/// A frame of refused rehearsals moves nothing: no receipt, no fingerprint
/// motion — a refusal under dry is still dry.
#[test]
fn refused_rehearsals_move_nothing() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);
    let before = conn.fingerprint();

    let rows = rows_of(&conn.call(&run_frame(
        32,
        "run-parity-2",
        json!([
            {"page": "tasks.md", "task": "grant", "args": [], "dry": true},
            {"page": "tasks.md", "task": "nocaps", "dry": true},
            {"page": "tasks.md", "task": "check-granted", "dry": true},
            {"page": "tasks.md", "task": "sh", "env": {"FOO": "bar"}, "dry": true},
        ]),
    )));
    for (i, row) in rows.iter().enumerate() {
        assert!(
            row["refusal"].is_object(),
            "row {i} must refuse under the rehearsed gates: {row}"
        );
    }
    assert_eq!(conn.fingerprint(), before, "nothing landed");
    assert!(
        !ws.join("receipts/run.md").exists(),
        "no receipt on any rehearsal"
    );
}

/// A gate-green rehearsal still answers the full effect set — the fix gates
/// the rehearsal, it does not neuter it.
#[test]
fn a_gate_green_rehearsal_still_answers_the_effect_set() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);
    let before = conn.fingerprint();

    let rows = rows_of(&conn.call(&run_frame(
        33,
        "run-parity-3",
        json!([{"page": "tasks.md", "task": "grant", "args": ["done"], "dry": true}]),
    )));
    assert_eq!(rows[0]["dry"], json!(true));
    assert_eq!(rows[0]["applied"], json!(false));
    assert_eq!(
        rows[0]["effects"][0]["kind"],
        json!("md.set_field"),
        "the declared effect set, listed: {rows:?}"
    );
    assert_eq!(conn.fingerprint(), before, "nothing landed");
}
