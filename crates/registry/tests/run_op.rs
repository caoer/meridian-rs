//! E2E gates for the § A.8 `run` op — page-task execution over the wire
//! (run-crossing ruling, 2026-08-13; `docs/wire-contract.md` § A.8,
//! `docs/run-plane.md`).
//!
//! Written RED-FIRST against the docs-first contract commit: every test in
//! this file fails on a daemon that does not serve the op. The pins:
//!
//! - a LIST of targets answers per-target rows in request order, and a
//!   refused target halts nothing after it — no aggregate boolean anywhere;
//! - the §9 identity law: per-target invocation ids derive from the host
//!   base, and a supplied `actor` threads into the receipt's actor fact;
//! - the plane's own receipts land at `receipts/run.md` under `r-<id>`;
//! - the several-tasks listing rides a `class:"invocation"` refusal row;
//! - `dry` rows rehearse without disk effect (starlark effects listed, bash
//!   shown-not-executed);
//! - the § A.8 U16 amendment: a bash step's cwd on THIS op is the bound
//!   workspace root;
//! - frame faults (`bad_request` family, v2 `unknown_op`) answer §8 frames —
//!   pre-plane only.

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

    fn hello_v2(&mut self, ws: &Path) -> Value {
        self.call(&json!({
            "op": "hello", "proto": 1,
            "workspace": ws.to_str().unwrap(),
        }))
    }

    fn fingerprint(&mut self) -> String {
        let resp = self.call(&json!({"id": 90, "op": "fingerprint"}));
        assert_eq!(resp["ok"], json!(true), "fingerprint op: {resp}");
        resp["body"]["fingerprint"].as_str().unwrap().to_owned()
    }
}

/// The runnable fixture: starlark with caps, bash, a two-task page for the
/// listing leg, and a cwd-probe bash task (§ A.8 U16 amendment).
const TASKS: &str = "\
---
task.fix-note: \"[[#^note-1]]\"
task.fix-note.caps: md.set_field
task.fix-note.args: value
task.sh-note: \"[[#^sh-1]]\"
task.pwd-check: \"[[#^pwd-1]]\"
---

# Tasks

```starlark
def run(ctx):
    set_field(field = \"status\", value = ctx.args[0])
```
^note-1

```bash
echo nope
```
^sh-1

```bash
test \"$(pwd)\" = \"$MERIDIAN_PROJECT_ROOT\"
```
^pwd-1
";

/// Two declared tasks, no single default — the listing refusal fixture.
const PAIR: &str = "\
---
task.alpha: \"[[#^a-1]]\"
task.beta: \"[[#^b-1]]\"
---

# Tasks

```starlark
def run(ctx):
    pass
```
^a-1

```starlark
def run(ctx):
    pass
```
^b-1
";

fn seeded(tmp: &TempDir) -> PathBuf {
    let ws = tmp.path().join("project");
    fs::create_dir_all(&ws).unwrap();
    fs::write(ws.join("tasks.md"), TASKS).unwrap();
    fs::write(ws.join("pair.md"), PAIR).unwrap();
    ws
}

fn run_frame(id: u64, invocation: &str, targets: Value) -> Value {
    let mut frame = json!({"id": id, "op": "run", "invocation": invocation});
    frame["targets"] = targets;
    frame
}

/// The per-target rows of an `ok:true` run response, with the §8-frame case
/// named in the panic message so a red run reads as the missing op.
fn rows_of(resp: &Value) -> Vec<Value> {
    assert_eq!(
        resp["ok"],
        json!(true),
        "the run op must answer rows whenever it reached the plane; got: {resp}"
    );
    let body = resp["body"].as_object().unwrap();
    assert_eq!(
        body.keys().collect::<Vec<_>>(),
        vec!["targets"],
        "no aggregate boolean, no second body key (§ A.8): {resp}"
    );
    body["targets"].as_array().unwrap().clone()
}

// ---------------------------------------------------------------------------
// The ZT requirement, engine half: a LIST, per-target rows, nothing halts.
// ---------------------------------------------------------------------------

/// Three targets — a starlark apply, a missing page, a bash step. Rows come
/// back in request order; the middle refusal halts nothing; ids derive from
/// the base; the starlark row's effect landed on disk.
#[test]
fn a_list_answers_per_target_rows_in_order_and_a_refused_target_halts_nothing() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);

    let resp = conn.call(&run_frame(
        11,
        "run-777-9",
        json!([
            {"page": "tasks.md", "task": "fix-note", "args": ["done"]},
            {"page": "missing.md"},
            {"page": "tasks.md", "task": "sh-note"},
        ]),
    ));
    let rows = rows_of(&resp);
    assert_eq!(rows.len(), 3, "one row per target: {resp}");

    // Row 0: the starlark apply, in place.
    assert_eq!(rows[0]["page"], json!("tasks.md"));
    assert_eq!(rows[0]["invocation"], json!("run-777-9-t0"));
    assert_eq!(rows[0]["state"], json!("applied"), "row 0: {resp}");
    assert_eq!(rows[0]["task"], json!("fix-note"));
    let doc = fs::read_to_string(ws.join("tasks.md")).unwrap();
    assert!(
        doc.contains("status: done"),
        "the effect landed on disk: {doc}"
    );

    // Row 1: the refusal names its class and halts nothing after it.
    assert_eq!(rows[1]["page"], json!("missing.md"));
    assert_eq!(
        rows[1]["refusal"]["class"],
        json!("invocation"),
        "an addressing fault is the exit-2 family: {resp}"
    );

    // Row 2: the bash step still ran.
    assert_eq!(rows[2]["invocation"], json!("run-777-9-t2"));
    assert_eq!(rows[2]["exec"]["exit_code"], json!(0), "row 2: {resp}");
}

/// A supplied `actor` threads into the receipt's actor fact, and the receipt
/// lands at the plane's own file under the derived per-target anchor.
#[test]
fn a_threaded_actor_lands_in_the_plane_receipt_under_the_derived_anchor() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);

    let mut frame = run_frame(
        12,
        "run-778-1",
        json!([{"page": "tasks.md", "task": "fix-note", "args": ["done"]}]),
    );
    frame["actor"] = json!("agent:b0864fb2");
    let rows = rows_of(&conn.call(&frame));
    assert_eq!(rows[0]["state"], json!("applied"));
    assert_eq!(
        rows[0]["receipt"],
        json!("receipts/run.md §^r-run-778-1-t0"),
        "the § A.8 receipt address, live grammar (2026-08-14 ruling): \
         `path §^anchor`, never the retired `#` join"
    );

    let receipts = fs::read_to_string(ws.join("receipts/run.md")).unwrap();
    assert!(
        receipts.contains("^r-run-778-1-t0"),
        "the derived anchor is on the line: {receipts}"
    );
    assert!(
        receipts.contains("agent:b0864fb2"),
        "the threaded actor is the receipt's actor fact: {receipts}"
    );
}

/// TASK omitted with several declared: the row lists them and refuses on the
/// invocation class — the CLI's list-then-exit-2, in row tense.
#[test]
fn several_declared_tasks_without_a_name_answer_the_listing_refusal_row() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);

    let rows = rows_of(&conn.call(&run_frame(13, "run-779-1", json!([{"page": "pair.md"}]))));
    assert_eq!(rows[0]["refusal"]["class"], json!("invocation"));
    assert_eq!(
        rows[0]["refusal"]["declared_tasks"],
        json!(["alpha", "beta"]),
        "the declared tasks ride the row: {rows:?}"
    );
}

/// `dry` rows rehearse without disk effect: starlark lists its full effect
/// set with `applied:false`; bash shows the block with `executed:false`; the
/// workspace fingerprint does not move.
#[test]
fn dry_rows_rehearse_without_disk_effect() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);
    let before = conn.fingerprint();

    let rows = rows_of(&conn.call(&run_frame(
        14,
        "run-780-1",
        json!([
            {"page": "tasks.md", "task": "fix-note", "args": ["done"], "dry": true},
            {"page": "tasks.md", "task": "sh-note", "dry": true},
        ]),
    )));
    assert_eq!(rows[0]["dry"], json!(true));
    assert_eq!(rows[0]["applied"], json!(false), "starlark dry: {rows:?}");
    assert!(
        rows[0]["effects"].as_array().is_some_and(|e| !e.is_empty()),
        "the full effect set is listed: {rows:?}"
    );
    assert_eq!(rows[1]["executed"], json!(false), "bash dry: {rows:?}");
    assert!(
        rows[1]["source"].as_str().unwrap().contains("echo nope"),
        "the block is shown: {rows:?}"
    );

    assert_eq!(conn.fingerprint(), before, "nothing landed");
}

/// § A.8's U16 amendment: on THIS op the bash step's working directory is the
/// bound workspace root. The probe exits 0 iff `pwd` equals the project root —
/// on the CLI's inherited-cwd law it would exit 1 here, because the daemon's
/// own cwd is the test process's, never the workspace.
#[test]
fn a_bash_step_runs_at_the_workspace_root_on_this_op() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);

    let rows = rows_of(&conn.call(&run_frame(
        15,
        "run-781-1",
        json!([{"page": "tasks.md", "task": "pwd-check"}]),
    )));
    assert_eq!(
        rows[0]["exec"]["exit_code"],
        json!(0),
        "pwd == workspace root on the wire arm: {rows:?}"
    );
}

// ---------------------------------------------------------------------------
// §8 frames: pre-plane faults only.
// ---------------------------------------------------------------------------

/// The frame family: v2 sessions answer `unknown_op`; malformed frames answer
/// `bad_request` before anything reaches the plane.
#[test]
fn frame_faults_answer_section_8_frames_and_nothing_reaches_the_plane() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();

    // v2 session: discovery honesty.
    let mut v2 = Conn::open(&test_config(&tmp).socket_path);
    v2.hello_v2(&ws);
    let resp = v2.call(&run_frame(20, "run-1", json!([{"page": "tasks.md"}])));
    assert_eq!(resp["ok"], json!(false));
    assert_eq!(resp["error"]["code"], json!("unknown_op"), "{resp}");

    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);

    // Empty targets.
    let resp = conn.call(&run_frame(21, "run-1", json!([])));
    assert_eq!(resp["error"]["code"], json!("bad_request"), "{resp}");

    // Over the ceiling.
    let many: Vec<Value> = (0..65).map(|_| json!({"page": "tasks.md"})).collect();
    let resp = conn.call(&run_frame(22, "run-1", json!(many)));
    assert_eq!(resp["error"]["code"], json!("bad_request"), "{resp}");

    // Missing invocation.
    let resp = conn.call(&json!({
        "id": 23, "op": "run", "targets": [{"page": "tasks.md"}]}));
    assert_eq!(resp["error"]["code"], json!("bad_request"), "{resp}");

    // Unknown target field — the strict wall at the target grain.
    let resp = conn.call(&run_frame(
        24,
        "run-1",
        json!([{"page": "tasks.md", "receipt": "receipts/evil.md"}]),
    ));
    assert_eq!(resp["error"]["code"], json!("bad_request"), "{resp}");
    assert!(
        resp["error"]["message"]
            .as_str()
            .unwrap()
            .contains("receipt"),
        "the wall names the field: {resp}"
    );

    // The workspace never moved: no run receipt exists.
    assert!(
        !ws.join("receipts/run.md").exists(),
        "nothing reached the plane"
    );
}

// ---------------------------------------------------------------------------
// Delta honesty (§ A.8, run-delta ruling 2026-08-14): run applies mint
// attributed Deltas on the workspace ring — a `sub` consumer sees a governed
// run as a governed run, never as detector-cadence external change.
// ---------------------------------------------------------------------------

/// A push subscriber on a second connection. After `sub` acks, the
/// connection is push-only; frames arrive within `PUSH_WAIT` or not at all.
const PUSH_WAIT: Duration = Duration::from_secs(10);

impl Conn {
    fn sub(&mut self, from_seq: u64) -> Value {
        self.writer
            .try_clone()
            .unwrap()
            .set_read_timeout(Some(PUSH_WAIT))
            .unwrap();
        self.call(&json!({"op": "sub", "from_seq": from_seq}))
    }

    /// Next Notification, or `None` within [`PUSH_WAIT`].
    fn next_frame(&mut self) -> Option<Value> {
        let mut line = String::new();
        match self.reader.read_line(&mut line) {
            Ok(n) if n > 0 => Some(serde_json::from_str(&line).expect("notification is JSON")),
            _ => None,
        }
    }
}

/// The core red/green of the run-delta ruling: one starlark apply through the
/// § A.8 op mints exactly ONE frame — attributed with the caller's actor and
/// now, the content page and the receipt file as two entries of one `files`,
/// chained onto the subscriber's ack root — and the detector emits NO
/// duplicate: the next frame after an external edit is contiguous.
///
/// RED before the ruling's code: the apply advances the fingerprint through
/// the plane's own executor, no frame is minted, and the subscriber sees the
/// change only as actorless detector-cadence external change.
#[test]
fn a_run_apply_pushes_one_attributed_delta_frame() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    let mut sub = Conn::open(server.socket_path());
    assert_eq!(sub.hello_v2(&ws)["ok"], json!(true));
    let ack = sub.sub(0);
    assert_eq!(ack["ok"], json!(true), "sub is served: {ack}");
    let ack_root = ack["body"]["root"].as_str().unwrap().to_owned();

    let mut conn = Conn::open(server.socket_path());
    conn.hello_v3(&ws);
    let resp = conn.call(&json!({
        "id": 31, "op": "run", "invocation": "run-d1", "actor": "seat-77",
        "now": "2026-08-14T00:00:00Z",
        "targets": [{"page": "tasks.md", "task": "fix-note", "args": ["done"]}],
    }));
    let rows = rows_of(&resp);
    assert!(rows[0].get("refusal").is_none(), "the apply landed: {resp}");

    let frame = sub.next_frame().expect("a run apply pushes a Delta frame");
    let delta = &frame["delta"];
    assert_eq!(delta["seq"], json!(1), "first frame of the epoch: {frame}");
    assert_eq!(
        delta["actor"],
        json!("seat-77"),
        "§9: the supplied actor threads into the frame: {frame}"
    );
    assert_eq!(
        delta["now"],
        json!("2026-08-14T00:00:00Z"),
        "§9: the caller's time fact rides the frame: {frame}"
    );
    assert_eq!(
        delta["root_before"].as_str().unwrap(),
        ack_root,
        "the frame chains onto the subscriber's ack root: {frame}"
    );
    let paths: Vec<&str> = delta["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        paths,
        vec!["tasks.md", "receipts/run.md"],
        "one committed batch = one frame; content first, then receipt: {frame}"
    );

    // No duplicate follows: the detector's next real cycle sees the ring tip
    // already carrying the moved root and rebases silently (the
    // internal-commit arm), so the next frame is the external edit,
    // contiguous with the run frame. The wait clears the detect-cadence
    // coalesce window first — an edit landing INSIDE the window diffs from
    // the pre-commit baseline, the splice lane's own pre-existing posture.
    std::thread::sleep(Duration::from_secs(1));
    external_edit_note(&ws);
    let next = sub.next_frame().expect("the external edit is detected");
    assert_eq!(
        next["delta"]["seq"],
        json!(2),
        "no duplicate frame for the run apply in between: {next}"
    );
    assert_eq!(
        next["delta"]["root_before"], frame["delta"]["root_after"],
        "the chain is contiguous across both producers: {next}"
    );
    assert!(
        next["delta"].get("actor").is_none(),
        "the external edit stays unattributed — §7.1: {next}"
    );
    server.shutdown();
}

/// An out-of-band note write — the external door, for chain-contiguity gates.
fn external_edit_note(ws: &Path) {
    fs::write(ws.join("note.md"), "# Note\n\nout of band\n").unwrap();
}

/// §9 on the frame, absent-actor arm: a run without a supplied actor is still
/// a GOVERNED effect — the frame carries the plane's own `run:<task>`
/// self-label, the same fact the receipt's actor field attests. Actor-absent
/// on the feed keeps meaning exactly "edited outside the face".
#[test]
fn a_run_apply_without_actor_carries_the_planes_self_label() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    let mut sub = Conn::open(server.socket_path());
    assert_eq!(sub.hello_v2(&ws)["ok"], json!(true));
    assert_eq!(sub.sub(0)["ok"], json!(true));

    let mut conn = Conn::open(server.socket_path());
    conn.hello_v3(&ws);
    let resp = conn.call(&run_frame(
        32,
        "run-d2",
        json!([{"page": "tasks.md", "task": "fix-note", "args": ["later"]}]),
    ));
    assert!(
        rows_of(&resp)[0].get("refusal").is_none(),
        "the apply landed: {resp}"
    );

    let frame = sub.next_frame().expect("the run apply pushes its frame");
    assert_eq!(
        frame["delta"]["actor"],
        json!("run:fix-note"),
        "absent caller actor keeps the plane's self-label: {frame}"
    );
    server.shutdown();
}

/// The bash path commits twice — the phase-1 pre-exec receipt and the phase-2
/// completion — and each committed batch mints its own contiguous frame
/// (§7.1: one batch, one root advance, one Delta). `echo nope` emits no
/// descriptor, so both frames name only the receipt file.
#[test]
fn a_bash_run_mints_one_frame_per_committed_batch() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    let mut sub = Conn::open(server.socket_path());
    assert_eq!(sub.hello_v2(&ws)["ok"], json!(true));
    assert_eq!(sub.sub(0)["ok"], json!(true));

    let mut conn = Conn::open(server.socket_path());
    conn.hello_v3(&ws);
    let resp = conn.call(&json!({
        "id": 33, "op": "run", "invocation": "run-d3", "actor": "seat-78",
        "targets": [{"page": "tasks.md", "task": "sh-note"}],
    }));
    let rows = rows_of(&resp);
    assert!(
        rows[0].get("refusal").is_none(),
        "the bash run landed: {resp}"
    );

    let first = sub.next_frame().expect("phase 1 mints a frame");
    let second = sub.next_frame().expect("phase 2 mints a frame");
    for (name, frame) in [("phase 1", &first), ("phase 2", &second)] {
        assert_eq!(
            frame["delta"]["actor"],
            json!("seat-78"),
            "{name} frame is attributed: {frame}"
        );
        let paths: Vec<&str> = frame["delta"]["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["path"].as_str().unwrap())
            .collect();
        assert_eq!(
            paths,
            vec!["receipts/run.md"],
            "{name}: a receipt-only batch names the receipt file: {frame}"
        );
    }
    assert_eq!(first["delta"]["seq"], json!(1));
    assert_eq!(second["delta"]["seq"], json!(2));
    assert_eq!(
        second["delta"]["root_before"], first["delta"]["root_after"],
        "the two commits chain: {second}"
    );
    server.shutdown();
}

/// The lock-bracket regression gate (run-delta amendment 2026-08-14b): the
/// run plane's `run.lock` does not exclude the detector, so without the
/// WRITE-flock bracket a detect cycle under load classifies a half-landed
/// run commit as external, actorless change (shed twice in one workspace
/// suite run before the bracket). Twelve governed runs under a live
/// subscriber — EVERY frame that names the run's files carries the actor.
#[test]
fn under_a_live_subscriber_every_run_frame_stays_attributed() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    let mut sub = Conn::open(server.socket_path());
    assert_eq!(sub.hello_v2(&ws)["ok"], json!(true));
    assert_eq!(sub.sub(0)["ok"], json!(true));

    let mut conn = Conn::open(server.socket_path());
    conn.hello_v3(&ws);
    for i in 0..12 {
        let resp = conn.call(&json!({
            "id": 60 + i, "op": "run", "invocation": format!("run-s{i}"),
            "actor": "stress-actor",
            "targets": [{"page": "tasks.md", "task": "fix-note", "args": [format!("v{i}")]}],
        }));
        assert!(
            rows_of(&resp)[0].get("refusal").is_none(),
            "run {i} landed: {resp}"
        );
    }

    let mut governed = 0;
    while let Some(frame) = sub.next_frame() {
        let names_run_files = frame["delta"]["files"].as_array().unwrap().iter().any(|f| {
            let p = f["path"].as_str().unwrap();
            p == "tasks.md" || p == "receipts/run.md"
        });
        if names_run_files {
            assert_eq!(
                frame["delta"]["actor"],
                json!("stress-actor"),
                "a governed run frame lost its actor — the detector won the \
                 bracket: {frame}"
            );
            governed += 1;
        }
        if governed >= 12 {
            break;
        }
    }
    assert!(governed >= 12, "only {governed} governed frames arrived");
    server.shutdown();
}
