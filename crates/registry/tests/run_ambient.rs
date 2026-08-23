//! E2E gates for § A.8 birth-target resolution (md-create-ambient-paths
//! shape (c) 2026-08-18, boundary-as-data amendment 2026-08-19 #2): the face
//! path law on the run plane's birth lane — a baseless `md.create` path
//! births under the frame's `ambient` (the caller's ambient session
//! directory, statusd-resolved per call); an EXPLICIT target rides the
//! descriptor's `base` argument as a rooted `root:rel` ref, landing as named
//! when it names the bound workspace and refusing with a teaching when it
//! names a foreign root. The declared `path` stays the capability glob's
//! matching coordinate on every lane. `hello` advertises cap `run.ambient`.
//!
//! The mount-table lifecycle rides ONE test fn by design (edition 2024: env
//! mutation is unsafe, and `MERIDIAN_CONFIG` is process-global — the
//! `mounts_op.rs` precedent); the decode-wall gate below is env-free.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use registry::{Config, RunningServer};
use serde_json::{Value, json};
use tempfile::TempDir;

mod common;

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

    fn hello_v3(&mut self, ws: &Path) -> Value {
        self.call(&json!({
            "op": "hello", "proto": 1, "contract": "v3",
            "workspace": ws.to_str().unwrap(),
        }))
    }
}

/// The birth fixtures. `birther.md` declares a bare path (the ambient lane);
/// `elsewhere.md` passes a rooted same-root BASE (the explicit lane);
/// `foreign.md` passes a base naming a root that is not the bound workspace.
const BIRTHER: &str = "\
---
task.birth-card: \"[[#^birth-1]]\"
task.birth-card.caps: \"md.create:tasks/*.md\"
task.birth-card.args: slug
---

# Birther

```starlark
def run(ctx):
    create(path = \"tasks/\" + ctx.args[0] + \".md\", body = \"# born card\\n\\nbody\\n\")
```
^birth-1
";

const ELSEWHERE: &str = "\
---
task.birth-there: \"[[#^there-1]]\"
task.birth-there.caps: \"md.create:tasks/*.md\"
task.birth-there.args: slug
---

# Explicit rooted base on the bound root — a session-shaped target dir, the
# create-task `--target` lane. The declared path is the SAME string the
# ambient lane declares: one cap glob covers both.

```starlark
def run(ctx):
    create(path = \"tasks/\" + ctx.args[0] + \".md\", base = \"sessions:year=2026/month=08/19-01-elsewhere\", body = \"# there\\n\")
```
^there-1
";

const FOREIGN: &str = "\
---
task.birth-foreign: \"[[#^foreign-1]]\"
task.birth-foreign.caps: \"md.create:tasks/*.md\"
---

# Foreign-root base

```starlark
def run(ctx):
    create(path = \"tasks/escapee.md\", base = \"assets:drop\", body = \"# nope\\n\")
```
^foreign-1
";

/// The caller's ambient session directory, workspace-relative — the shape
/// statusd resolves per call.
const AMBIENT: &str = "year=2026/month=08/18-00-adhoc";

fn run_frame(id: u64, invocation: &str, targets: Value) -> Value {
    let mut frame = json!({"id": id, "op": "run", "invocation": invocation});
    frame["targets"] = targets;
    frame
}

fn rows_of(resp: &Value) -> Vec<Value> {
    assert_eq!(
        resp["ok"],
        json!(true),
        "the run op must answer rows whenever it reached the plane; got: {resp}"
    );
    resp["body"]["targets"].as_array().unwrap().clone()
}

/// The bound `sessions` root (the workspace) plus a foreign `assets` root,
/// both self-declared, bound by a `MERIDIAN_CONFIG` table the ONE env fn
/// points at.
fn sandbox(tmp: &TempDir) -> (PathBuf, PathBuf) {
    let ws = tmp.path().join("sessions");
    let assets = tmp.path().join("assets");
    fs::create_dir_all(&ws).unwrap();
    fs::create_dir_all(&assets).unwrap();
    fs::write(
        ws.join("MERIDIAN.md"),
        "---\ntype: meridian-root\nversion: 1\nname: sessions\n---\n\n# Sessions root\n",
    )
    .unwrap();
    fs::write(
        assets.join("MERIDIAN.md"),
        "---\ntype: meridian-root\nversion: 1\nname: assets\n---\n\n# Assets root\n",
    )
    .unwrap();
    fs::write(ws.join("birther.md"), BIRTHER).unwrap();
    fs::write(ws.join("elsewhere.md"), ELSEWHERE).unwrap();
    fs::write(ws.join("foreign.md"), FOREIGN).unwrap();
    let config = tmp.path().join("home/MERIDIAN.md");
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    fs::write(
        &config,
        format!(
            "---\ntype: meridian-config\nversion: 1\n---\n\n# Test roots\n\n\
             ```meridian-mount\nname: sessions\npath: {}\nvault: sessions\n```\n\n\
             ```meridian-mount\nname: assets\npath: {}\nvault: assets\n```\n",
            ws.display(),
            assets.display()
        ),
    )
    .unwrap();
    (ws, config)
}

// ---------------------------------------------------------------------------
// The mount-table lifecycle — ONE env-dependent fn (mounts_op precedent).
// ---------------------------------------------------------------------------

/// End to end on one server: hello advertises `run.ambient`; a bare birth
/// under `ambient` lands in the caller's session directory (and its dry row
/// shows the RESOLVED path); the occupied rerun refuses; a rooted same-root
/// target lands as named; a foreign-root target refuses with the teaching;
/// a frame WITHOUT ambient keeps the bare-door law (workspace-root-relative).
#[test]
#[allow(clippy::too_many_lines)] // one env-dependent fn by design (module docs)
fn ambient_and_rooted_birth_targets_resolve_on_the_wire_arm() {
    let tmp = TempDir::new().unwrap();
    let (ws, config) = sandbox(&tmp);
    // One env-dependent test fn by design (see module docs).
    unsafe { std::env::set_var("MERIDIAN_CONFIG", &config) };
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    let hello = conn.hello_v3(&ws);
    let caps: Vec<&str> = hello["body"]["caps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert!(
        caps.contains(&"run.ambient"),
        "hello advertises the ambient lane so hosts know to attach it: {hello}"
    );

    // The dry leg first: the rehearsed effect list shows the RESOLVED path
    // (dry-green predicts live-green, landing place included).
    let mut frame = run_frame(
        30,
        "amb-dry",
        json!([{"page": "birther.md", "task": "birth-card", "args": ["zz-amb"], "dry": true}]),
    );
    frame["ambient"] = json!(AMBIENT);
    let rows = rows_of(&conn.call(&frame));
    let effects = rows[0]["effects"].as_array().unwrap();
    assert_eq!(
        effects[0]["args"]["path"],
        json!(format!("{AMBIENT}/tasks/zz-amb.md")),
        "the dry effect names the resolved landing path: {rows:?}"
    );
    assert!(
        !ws.join(AMBIENT).exists(),
        "a dry target lands nothing on disk"
    );

    // The live ambient birth: bare `tasks/<slug>.md` lands on the CALLER's
    // board, not the workspace root's.
    let mut frame = run_frame(
        31,
        "amb-live",
        json!([{"page": "birther.md", "task": "birth-card", "args": ["zz-amb"]}]),
    );
    frame["ambient"] = json!(AMBIENT);
    frame["fields"] = json!({"session": "18-00-adhoc", "agent": "0bdfc81e"});
    let rows = rows_of(&conn.call(&frame));
    assert_eq!(
        rows[0]["state"],
        json!("applied"),
        "the birth lands: {rows:?}"
    );
    let born = fs::read_to_string(ws.join(AMBIENT).join("tasks/zz-amb.md")).unwrap();
    assert!(born.contains("# born card"), "born bytes on disk: {born}");
    assert!(
        !ws.join("tasks/zz-amb.md").exists(),
        "nothing landed at the workspace root — the original failure mode"
    );

    // The occupied rerun refuses through the door on the RESOLVED path.
    let mut frame = run_frame(
        32,
        "amb-rerun",
        json!([{"page": "birther.md", "task": "birth-card", "args": ["zz-amb"]}]),
    );
    frame["ambient"] = json!(AMBIENT);
    let rows = rows_of(&conn.call(&frame));
    assert_ne!(
        rows[0]["state"],
        json!("applied"),
        "an occupied path never lands twice: {rows:?}"
    );

    // The explicit lane (the create-task --target shape): a rooted BASE on
    // the BOUND root lands as named, ambient contributing nothing — and the
    // same `md.create:tasks/*.md` grant admits it (three-lane equivalence).
    let mut frame = run_frame(
        33,
        "rooted-live",
        json!([{"page": "elsewhere.md", "task": "birth-there", "args": ["zz-there"]}]),
    );
    frame["ambient"] = json!(AMBIENT);
    let rows = rows_of(&conn.call(&frame));
    assert_eq!(
        rows[0]["state"],
        json!("applied"),
        "the rooted same-root birth lands: {rows:?}"
    );
    let there = fs::read_to_string(ws.join("year=2026/month=08/19-01-elsewhere/tasks/zz-there.md"))
        .unwrap();
    assert!(there.contains("# there"), "explicit target bytes: {there}");
    assert!(
        !ws.join(AMBIENT).join("year=2026").exists(),
        "a based target never resolves under ambient"
    );

    // The foreign-root refusal: the run's births ride the bound workspace's
    // ring and locks, so a foreign tree refuses with the teaching.
    let frame = run_frame(
        34,
        "foreign",
        json!([{"page": "foreign.md", "task": "birth-foreign"}]),
    );
    let rows = rows_of(&conn.call(&frame));
    let refusal = rows[0]["refusal"]["reason"].as_str().unwrap_or_default();
    assert!(
        refusal.contains("not this run's bound workspace"),
        "the foreign-root teaching rides the row: {rows:?}"
    );
    assert!(
        !tmp.path().join("assets/drop/tasks/escapee.md").exists(),
        "nothing landed in the foreign tree"
    );

    // Compat: a frame WITHOUT ambient keeps the bare-door law — the bare
    // path lands workspace-root-relative, exactly as before the lane.
    let frame = run_frame(
        35,
        "bare-door",
        json!([{"page": "birther.md", "task": "birth-card", "args": ["zz-root"]}]),
    );
    let rows = rows_of(&conn.call(&frame));
    assert_eq!(rows[0]["state"], json!("applied"), "bare door: {rows:?}");
    assert!(
        ws.join("tasks/zz-root.md").exists(),
        "no ambient on the frame = workspace-root-relative, unchanged"
    );
}

// ---------------------------------------------------------------------------
// The strict wall — env-free (refuses before the serve path reads anything).
// ---------------------------------------------------------------------------

/// A malformed `ambient` refuses the whole frame at the decode wall: an
/// escaping path and a `root:` ref are both walls (ambient is a DIRECTORY,
/// never a ref), before any target runs.
#[test]
fn a_malformed_ambient_refuses_the_run_frame() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("project");
    fs::create_dir_all(&ws).unwrap();
    fs::write(ws.join("birther.md"), BIRTHER).unwrap();
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);

    for (id, bad) in [(40, "../escape"), (41, "sessions:year=2026"), (42, "")] {
        let mut frame = run_frame(id, "bad-ambient", json!([{"page": "birther.md"}]));
        frame["ambient"] = json!(bad);
        let resp = conn.call(&frame);
        assert_eq!(
            resp["ok"],
            json!(false),
            "ambient `{bad}` must refuse the frame: {resp}"
        );
        assert!(
            !ws.join("tasks").exists(),
            "nothing reached the plane on ambient `{bad}`"
        );
    }
}
