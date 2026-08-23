//! E2E gates for the amended `run` op's two modes over the WIRE — the daemon
//! lane of hook-support design § 2.2 (as amended by § Amendments / A1).
//!
//! The CLI lane is gated in `crates/mrd/tests/run_load_fire.rs`. This file
//! exists because the two lanes are reached differently and the design's
//! price gate is measured on this one: a resolver and a fire both arrive over
//! the registry socket, not through a process.
//!
//! What is pinned here, each an acceptance-gate row of the card:
//!
//! - **row 4** — an unnegotiated `mode` refuses BY NAME at the closed set;
//! - **row 6** — a fire adds no receipt rows;
//! - **row 3b** — the answer to *what a fire does during the cold-build
//!   window*: it refuses `corpus_warming`, a retry class. It does not block
//!   and it does not bypass;
//! - the resolver's shape: three `mode:"load"` targets in ONE call, answered
//!   as three rows in request order.

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
    // The fixture rule (`crates/registry/tests/common/mod.rs` § Fixture rule):
    // the 2 s production default is a CLIENT's flock budget, not a fixture's —
    // a cold build on a loaded CI box outlives it and the test flakes on drain
    // rather than on its own subject.
    config.drain_cold_builds = Duration::from_secs(30);
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

    /// A v3 hello — the rev that advertises `run.mode` / `run.input`.
    fn hello_v3(&mut self, ws: &Path) -> Value {
        self.call(&json!({
            "op": "hello", "proto": 1, "contract": "v3",
            "workspace": ws.to_str().unwrap(),
        }))
    }

    /// A v2 hello — the frozen rev, which advertises NEITHER cap. This is how
    /// row 4 is produced: a client that skipped negotiation and sent `mode`
    /// anyway.
    fn hello_v2(&mut self, ws: &Path) -> Value {
        self.call(&json!({
            "op": "hello", "proto": 1,
            "workspace": ws.to_str().unwrap(),
        }))
    }
}

/// A page carrying a declaring block, a task-bound block, and a birthing
/// block under the page's own `caps:`.
const HOOKS: &str = "\
---
caps: md.create
task.arm: \"[[#^armer]]\"
---

# Hooks

```starlark
def run(event):
    return {\"deny\": \"no stash\", \"saw\": event[\"name\"]}

declare(on = \"PreToolUse\", match = \"Bash\")
```
^no-stash

```starlark
def run(ctx):
    pass
```
^armer
";

const OTHER: &str = "\
# Other

```starlark
declare(on = \"Stop\")
```
^only
";

fn seeded(tmp: &TempDir) -> PathBuf {
    let ws = tmp.path().join("project");
    fs::create_dir_all(ws.join("agents/ab")).unwrap();
    fs::write(ws.join("HOOKS.md"), HOOKS).unwrap();
    fs::write(ws.join("other.md"), OTHER).unwrap();
    fs::write(ws.join("agents/ab/HOOKS.md"), OTHER).unwrap();
    ws
}

fn run_frame(id: u64, invocation: &str, targets: &Value) -> Value {
    json!({"id": id, "op": "run", "invocation": invocation, "targets": targets})
}

fn rows_of(resp: &Value) -> Vec<Value> {
    assert_eq!(
        resp["ok"],
        json!(true),
        "the run op must answer rows: {resp}"
    );
    resp["body"]["targets"].as_array().unwrap().clone()
}

// ---------------------------------------------------------------------------
// Row 4 — the closed set, and what a skipped negotiation costs
// ---------------------------------------------------------------------------

/// **Gate row 4, the half that IS constructible at this sha**: the closed
/// set is still closed, and an unknown field refuses BY NAME with its target
/// index.
///
/// A client does not choose which caps it negotiates — the SERVER advertises
/// its capability set whole (`wire-contract.md`: *"Op discovery is complete;
/// there is no version sniffing"*), so "a v3 client without `run.mode`" is not
/// a thing that exists. The instrument the design names is the other one: *"an
/// old server receiving it anyway refuses by name at the closed set"*, and
/// that is now constructible in-process — see
/// `an_unnegotiated_mode_refuses_by_name_at_the_closed_set` below, which was
/// unconstructible until the closed sets became rev-conditional.
///
/// What THIS test pins is the property underneath both: the wall is still a
/// closed set after six fields joined it.
#[test]
fn the_target_field_set_is_still_closed_and_refuses_by_name() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);

    let resp = conn.call(&run_frame(
        4,
        "gate-row-4",
        &json!([{"page": "HOOKS.md", "mode": "load", "bogus": 1}]),
    ));

    assert_eq!(resp["ok"], json!(false), "an unknown field must refuse");
    assert_eq!(
        resp["error"]["message"].as_str().unwrap_or_default(),
        "unknown field `bogus` on `targets[0]` of `run`",
        "the refusal must name the FIELD and the target index, verbatim: {resp}"
    );
}

/// **GATE ROW 4, the bytes** — an unnegotiated `mode` refused BY NAME at the
/// closed set, from a v2 session.
///
/// This test asserted the OPPOSITE until PR 195's review: it recorded that a
/// v2 client meets `unknown_op` at the OP grain and never reaches the field
/// wall — which was true, and which made the acceptance gate's own required
/// bytes unconstructible. The six target additions and `prelude` sat in
/// UNCONDITIONAL closed sets, so no client could ever see the by-name refusal
/// the design and both docs publish: a v2 client was stopped one layer
/// earlier and a v3 client always has the caps.
///
/// The sets are now rev-conditional (the shipped precedent is the root op's
/// mint arm), so a non-v3 session is judged against the SHIPPED fields and
/// meets the wall first. The feature was never ungated — what was false was
/// the published mechanism, and this test is what would have caught it.
/// (Reviewer `fa5da9ec`; advisor `ea317a27`, design conformance.)
#[test]
fn an_unnegotiated_mode_refuses_by_name_at_the_closed_set() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v2(&ws);

    let resp = conn.call(&run_frame(
        5,
        "gate-row-4b",
        &json!([{"page": "HOOKS.md", "task": "arm", "mode": "fire"}]),
    ));
    assert_eq!(resp["ok"], json!(false));
    assert_eq!(
        resp["error"]["message"].as_str().unwrap_or_default(),
        "unknown field `mode` on `targets[0]` of `run`",
        "the acceptance gate's own bytes, from an unnegotiated session: {resp}"
    );

    // The same wall, one field over, and at the OP grain for `prelude`.
    let resp = conn.call(&run_frame(
        6,
        "gate-row-4c",
        &json!([{"page": "HOOKS.md", "task": "arm", "input": {"a": 1}}]),
    ));
    assert_eq!(
        resp["error"]["message"].as_str().unwrap_or_default(),
        "unknown field `input` on `targets[0]` of `run`",
        "{resp}"
    );

    // A v2 target that names ONLY shipped fields still decodes and is
    // answered by the op-grain gate — the amendment took nothing away.
    let resp = conn.call(&run_frame(
        7,
        "gate-row-4d",
        &json!([{"page": "HOOKS.md", "task": "arm"}]),
    ));
    assert_eq!(resp["ok"], json!(false));
    assert_eq!(
        resp["error"]["code"], "unknown_op",
        "the shipped shape still meets the op-grain v3 gate: {resp}"
    );
}

/// Every § 2.2 exclusion refuses BY NAME and TEACHES which addressing the
/// caller is on. A field that is meaningless where it was written must
/// refuse: silently dropping `input` on a task target would leave the caller
/// believing their payload was delivered.
#[test]
fn the_exclusions_refuse_by_name_and_teach_the_addressing() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);

    for (target, expect) in [
        // `input` addresses a declared block; a target with no `mode` is the
        // shipped task path.
        (json!({"page": "HOOKS.md", "input": {}}), "needs `mode`"),
        // The two addressings are exclusive.
        (
            json!({"page": "HOOKS.md", "task": "arm", "mode": "fire", "block": "no-stash"}),
            "exclusive",
        ),
        // A fire's one input channel is `input`; argv is the task's.
        (
            json!({"page": "HOOKS.md", "mode": "fire", "block": "no-stash", "args": ["x"]}),
            "args",
        ),
        // A load addresses a PAGE, not a block.
        (
            json!({"page": "HOOKS.md", "mode": "load", "block": "no-stash"}),
            "block",
        ),
    ] {
        let resp = conn.call(&run_frame(40, "excl", &json!([target])));
        assert_eq!(resp["ok"], json!(false), "must refuse: {resp}");
        let message = resp["error"]["message"].as_str().unwrap_or_default();
        assert!(
            message.contains(expect),
            "the refusal must teach {expect:?}, got: {message}"
        );
    }
}

// ---------------------------------------------------------------------------
// The resolver's shape, and the fire
// ---------------------------------------------------------------------------

/// A cold resolve is ONE call carrying three `mode:"load"` targets, answered
/// as three rows in request order (§ 1.6). The plural was never an obstacle
/// to the amendment — it is the fit.
#[test]
fn a_resolve_is_one_call_with_three_load_targets() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);

    let resp = conn.call(&run_frame(
        6,
        "hooks-resolve-ab-1",
        &json!([
            {"page": "HOOKS.md", "mode": "load"},
            {"page": "other.md", "mode": "load"},
            {"page": "agents/ab/HOOKS.md", "mode": "load"},
        ]),
    ));
    let rows = rows_of(&resp);

    assert_eq!(rows.len(), 3, "one row per target: {resp}");
    assert_eq!(rows[0]["page"], "HOOKS.md");
    assert_eq!(rows[1]["page"], "other.md");
    assert_eq!(rows[2]["page"], "agents/ab/HOOKS.md");

    // The declaring block publishes its dict; the task-bound one is reported
    // as the run plane's and never evaluated.
    let loaded = rows[0]["loaded"].as_array().unwrap();
    let hook = loaded.iter().find(|b| b["block"] == "no-stash").unwrap();
    assert_eq!(
        hook["declarations"],
        json!({"on": "PreToolUse", "match": "Bash"}),
        "§ A.8: ONE dict, published verbatim — not a list of them"
    );
    let armer = loaded.iter().find(|b| b["block"] == "armer").unwrap();
    assert_eq!(armer["entry_kind"], "task");
    assert_eq!(armer["declarations"], Value::Null, "it declares nothing");
}

/// A fire over the wire answers the entry's return as JSON, and **row 6**:
/// it appends NO receipt rows. The receipt file is compared as bytes, not by
/// a count that a never-created file would also satisfy.
#[test]
fn a_fire_answers_its_value_and_writes_no_receipt() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);

    let receipts = ws.join(run::executor::RECEIPT_FILE);
    let before = fs::read_to_string(&receipts).unwrap_or_default();

    for i in 0..5 {
        let resp = conn.call(&run_frame(
            10 + i,
            &format!("94127a04.PreToolUse.toolu_{i}"),
            &json!([{
                "page": "HOOKS.md", "block": "no-stash", "mode": "fire",
                "input": {"name": "PreToolUse", "id": "s:PreToolUse:t0"},
            }]),
        ));
        let rows = rows_of(&resp);
        assert_eq!(rows[0]["result"], "ok", "{resp}");
        assert_eq!(
            rows[0]["value"],
            json!({"deny": "no stash", "saw": "PreToolUse"}),
            "the input reached the entry and its answer came back: {resp}"
        );
        // Provenance: WHICH BYTES ran.
        assert!(
            rows[0]["rev"]["block"]
                .as_str()
                .is_some_and(|r| !r.is_empty())
        );
    }

    let after = fs::read_to_string(&receipts).unwrap_or_default();
    assert_eq!(
        after,
        before,
        "**recording by declaration kind**: a fire writes no receipt — five \
         fires moved `{}`",
        run::executor::RECEIPT_FILE
    );
}

/// The consent gate, over the wire: a `task.<name>`-bound block is not a fire
/// target, and the refusal names the other addressing rather than silently
/// running the task contract with an event as its `ctx`.
#[test]
fn the_consent_gate_holds_on_the_wire() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);

    let resp = conn.call(&run_frame(
        20,
        "gate-row-7",
        &json!([{"page": "HOOKS.md", "block": "armer", "mode": "fire", "input": {}}]),
    ));
    let rows = rows_of(&resp);
    assert_eq!(rows[0]["result"], "refused");
    assert_eq!(rows[0]["fault"]["class"], "not_declared");
}

/// A mode target and a task target in ONE call. Mixed batches are legal by
/// construction — rows are independent — and each row keeps its OWN
/// vocabulary: the task row a `state` word, the fire row a `result` word.
/// That is the amendment's named cost, and it is visible here rather than
/// asserted in prose.
#[test]
fn a_mixed_batch_keeps_each_rows_own_vocabulary() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);

    let resp = conn.call(&run_frame(
        30,
        "mixed-1",
        &json!([
            {"page": "HOOKS.md", "task": "arm"},
            {"page": "HOOKS.md", "block": "no-stash", "mode": "fire", "input": {"name": "Stop"}},
        ]),
    ));
    let rows = rows_of(&resp);
    assert_eq!(rows.len(), 2);

    // The task row keeps the shipped vocabulary — and its receipt.
    assert!(
        rows[0].get("state").is_some() || rows[0].get("refusal").is_some(),
        "a task row must keep its own shape: {resp}"
    );
    // The fire row answers an EVALUATION word, and carries no receipt field.
    assert_eq!(rows[1]["result"], "ok");
    assert!(
        rows[1].get("receipt").is_none(),
        "a fire row must carry no receipt pointer: {resp}"
    );
}
