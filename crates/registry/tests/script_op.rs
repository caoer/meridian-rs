//! E2E gates for the § A.7 `script` op — in-process script submission
//! (phase-2 script-plane ruling, 2026-08-12; `docs/wire-contract.md` § A.7,
//! `docs/run-plane.md` § The script entry, the entry-world amendment).
//!
//! Written RED-FIRST against the docs-first contract commit: every test in
//! this file fails on a daemon that does not serve the op. The pins:
//!
//! - the trace is the response body (`ok:true` even for faults/refusals);
//! - the entry world: reads serve at the entry fingerprint, zero byte-folds;
//! - read-your-own-writes: an armed target reads back armed content;
//! - CAS relaxation (ruling 2026-08-13): a write whose target was never read
//!   commits on an unmoved world — tokens thread from the entry state, and
//!   consistency lives at the commit's world guard, not in a read ritual;
//! - the caller guard fast-fails pre-eval (zero reads, `conflict`);
//! - `expect_armed` mismatch refuses pre-splice (nothing lands);
//! - fuel exhaustion answers a `budget` fault and the daemon SURVIVES;
//! - a second content path refuses at arm time (nothing lands);
//! - `dry` rehearses without disk effect;
//! - strict decode refuses unknown fields; v2 sessions answer `unknown_op`;
//! - §12.1 addressability: an out-of-domain path serves on this lane as on
//!   the CLI lane (live single-file load), and writes without moving the
//!   fingerprint.
//!
//! Mid-eval foreign-edit invisibility, entry-rev-vs-overlay-rev threading,
//! wall-clock sites, and panic containment need in-process seams and are
//! pinned at the module grain in `crates/registry/src/script_op.rs` tests.

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

fn write_ws(tmp: &TempDir, name: &str, files: &[(&str, &str)]) -> PathBuf {
    let ws = tmp.path().join(name);
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

const DOC: &str =
    "---\nstatus: open\ntitle: Alpha\n---\n# Alpha\n\none two three\n\n## Beta\n\nfour five\n";

/// A doc + a receipts file, the ordinary two-file workspace of the fixtures.
fn seeded(tmp: &TempDir) -> PathBuf {
    write_ws(
        tmp,
        "project",
        &[("doc.md", DOC), ("logs/receipts.md", "# Receipts\n")],
    )
}

fn script(id: u64, source: &str) -> Value {
    json!({"id": id, "op": "script", "source": source})
}

/// The trace body of an `ok:true` script response, with the §8-frame case
/// named in the panic message so a red run reads as the missing op.
fn trace_of(resp: &Value) -> Value {
    assert_eq!(
        resp["ok"],
        json!(true),
        "the script op must answer a trace whenever the entry ran; got: {resp}"
    );
    resp["body"].clone()
}

// ---------------------------------------------------------------------------
// The trace is the body; the entry world serves reads at the entry fingerprint.
// ---------------------------------------------------------------------------

/// A read-only program answers `no_effect` with its reads recorded, telemetry
/// present, and `entry_fingerprint` equal to what the `fingerprint` op serves.
#[test]
fn a_read_only_program_answers_a_no_effect_trace_at_the_entry_fingerprint() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);
    let ambient = conn.fingerprint();

    let resp = conn.call(&script(7, "card = read(\"doc.md\")\n"));
    let trace = trace_of(&resp);

    assert_eq!(trace["outcome"], json!("no_effect"), "trace: {trace}");
    assert_eq!(
        trace["entry_fingerprint"].as_str().unwrap(),
        ambient,
        "the entry fingerprint is the ambient one"
    );
    let rows = trace["trace"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "one recorded read: {trace}");
    assert_eq!(rows[0]["kind"], json!("echo"));
    assert_eq!(rows[0]["path"], json!("doc.md"));
    let fm = &rows[0]["face"]["Toc"]["fm"];
    assert_eq!(
        fm["status"],
        json!("open"),
        "decoded fm on the face: {trace}"
    );
    assert!(
        trace["telemetry"]["reads_used"] == json!(1),
        "telemetry is unconditional: {trace}"
    );
    assert!(
        trace.get("commit").is_none(),
        "no splice was issued: {trace}"
    );
}

// ---------------------------------------------------------------------------
// Read-your-own-writes; the commit; write-follows-read.
// ---------------------------------------------------------------------------

/// A read of a target the program itself armed serves the ARMED content —
/// and the commit lands it exactly once, with the armed row marked committed
/// and the armed digest published.
#[test]
fn an_armed_target_reads_back_its_own_armed_content_and_commits_once() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);
    let entry = conn.fingerprint();

    let resp = conn.call(&script(
        9,
        "t = read(\"doc.md\")\nput(\"doc.md\", props={\"status\": \"done\"})\nt2 = read(\"doc.md\")\n",
    ));
    let trace = trace_of(&resp);
    assert_eq!(trace["outcome"], json!("committed"), "trace: {trace}");

    // The overlay read: the SECOND read row serves the armed value while the
    // first served the entry value.
    let rows = trace["trace"].as_array().unwrap();
    let read_rows: Vec<&Value> = rows
        .iter()
        .filter(|r| r["kind"] == json!("echo") || r["kind"] == json!("read"))
        .collect();
    assert_eq!(read_rows.len(), 2, "two recorded reads: {trace}");
    assert_eq!(read_rows[0]["face"]["Toc"]["fm"]["status"], json!("open"));
    assert_eq!(
        read_rows[1]["face"]["Toc"]["fm"]["status"],
        json!("done"),
        "read-your-own-writes: the post-arm read serves the armed content"
    );
    // What you read is what is hashed: the overlay face carries its own rev,
    // distinct from the entry rev.
    assert_ne!(
        read_rows[0]["face"]["Toc"]["rev"], read_rows[1]["face"]["Toc"]["rev"],
        "the overlay face hashes the overlay bytes"
    );

    // The armed row committed, and the digest is published.
    let armed_rows: Vec<&Value> = rows
        .iter()
        .filter(|r| r["kind"] == json!("armed"))
        .collect();
    assert_eq!(armed_rows.len(), 1);
    assert_eq!(armed_rows[0]["committed"], json!(true));
    assert!(
        trace["armed_digest"]
            .as_str()
            .unwrap()
            .starts_with("armed-set-path-edit:sha256:"),
        "the domain-tagged digest rides the trace: {trace}"
    );

    // The commit leg is the §4.4 splice response verbatim, and it landed:
    // disk carries the value, the fingerprint advanced.
    let leg = &trace["commit"];
    assert_eq!(leg["fingerprint_before"].as_str().unwrap(), entry);
    assert!(
        leg["fingerprint_after"]
            .as_str()
            .unwrap()
            .starts_with("b3:")
    );
    let on_disk = fs::read_to_string(ws.join("doc.md")).unwrap();
    assert!(on_disk.contains("status: done"), "landed: {on_disk}");
    assert_eq!(on_disk.matches("status:").count(), 1, "exactly once");
    assert_eq!(
        conn.fingerprint(),
        leg["fingerprint_after"].as_str().unwrap()
    );
}

/// The CAS relaxation (ZT ruling 2026-08-13, dissolves F-S2): a destructive
/// row whose target the attempt never read is auto-guarded by the entry
/// fingerprint the commit already carries — its token threads from the ENTRY
/// world, no read ritual licenses it, and it commits on an unmoved world.
/// Consistency enforcement lives at COMMIT (§5.1 world guard, checked first),
/// pinned by the module test
/// `a_foreign_edit_after_entry_is_invisible_to_reads_and_refuses_the_commit`.
#[test]
fn a_props_write_with_no_read_commits_on_an_unmoved_world() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);
    let before = conn.fingerprint();

    let resp = conn.call(&script(
        10,
        "put(\"doc.md\", props={\"status\": \"done\"})\n",
    ));
    let trace = trace_of(&resp);
    assert_eq!(trace["outcome"], json!("committed"), "trace: {trace}");
    let leg = &trace["commit"];
    assert_eq!(leg["fingerprint_before"].as_str().unwrap(), before);
    let on_disk = fs::read_to_string(ws.join("doc.md")).unwrap();
    assert!(on_disk.contains("status: done"), "landed: {on_disk}");
    assert_eq!(
        conn.fingerprint(),
        leg["fingerprint_after"].as_str().unwrap()
    );
}

/// Put parity (same ruling): an append inside script goes rev-free — the
/// author reads nothing, the engine threads the entry section's own token,
/// and the append lands. Append cannot clobber; a moved world still refuses
/// the whole commit at §5.1.
#[test]
fn an_append_with_no_read_commits_on_an_unmoved_world() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);
    let before = conn.fingerprint();

    let resp = conn.call(&script(
        16,
        "put(\"doc.md\", section=\"Alpha/Beta\", append=\"six seven\\n\")\n",
    ));
    let trace = trace_of(&resp);
    assert_eq!(trace["outcome"], json!("committed"), "trace: {trace}");
    let leg = &trace["commit"];
    assert_eq!(leg["fingerprint_before"].as_str().unwrap(), before);
    let on_disk = fs::read_to_string(ws.join("doc.md")).unwrap();
    assert!(on_disk.contains("six seven"), "landed: {on_disk}");
    assert_eq!(
        conn.fingerprint(),
        leg["fingerprint_after"].as_str().unwrap()
    );
}

/// Entry currency across calls: a foreign write between two attempts is
/// visible to the second attempt's entry — the world is pinned per attempt,
/// never cached across attempts.
#[test]
fn a_later_attempt_enters_at_the_moved_world() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let socket = test_config(&tmp).socket_path;
    let mut conn = Conn::open(&socket);
    conn.hello_v3(&ws);

    let first = trace_of(&conn.call(&script(11, "t = read(\"doc.md\")\n")));
    assert_eq!(
        first["trace"][0]["face"]["Toc"]["fm"]["status"],
        json!("open")
    );

    // A foreign writer lands via the ordinary wire door on a second conn.
    let mut foreign = Conn::open(&socket);
    foreign.hello_v3(&ws);
    let splice = foreign.call(&json!({
        "id": 40, "op": "splice", "path": "doc.md",
        "plan_edits": [{"set_property": {"key": "status", "value": "parked"}}],
        "force": true,
    }));
    assert_eq!(splice["ok"], json!(true), "foreign write: {splice}");

    let second = trace_of(&conn.call(&script(12, "t = read(\"doc.md\")\n")));
    assert_eq!(
        second["trace"][0]["face"]["Toc"]["fm"]["status"],
        json!("parked"),
        "the second attempt's entry pass saw the foreign write"
    );
    assert_ne!(
        first["entry_fingerprint"], second["entry_fingerprint"],
        "two attempts, two entry fingerprints"
    );
}

// ---------------------------------------------------------------------------
// §12.1 addressability: out-of-domain paths serve on this lane as on the CLI
// lane (ruled 2026-08-12: "mrd mcp should be same as cli").
// ---------------------------------------------------------------------------

/// A real file under the root but outside the hash domain — a dot-directory
/// page, a non-md file — serves by explicit path exactly as it does on the
/// wire-client lane: hash domain ⊂ addressable domain, one answer at every
/// door. Before this fold the same script answered `no_effect` on the
/// subprocess lane and a fault here (review B2, live-confirmed).
#[test]
fn an_out_of_domain_path_serves_on_this_lane_as_on_the_cli_lane() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    fs::create_dir_all(ws.join(".obsidian")).unwrap();
    fs::write(
        ws.join(".obsidian/hidden.md"),
        "---\nstatus: open\n---\n# Hidden\n",
    )
    .unwrap();
    fs::write(ws.join("notes.txt"), "# Notes\n\nplain text\n").unwrap();
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);

    let resp = conn.call(&script(
        14,
        "a = read(\".obsidian/hidden.md\")\nb = read(\"notes.txt\")\n",
    ));
    let trace = trace_of(&resp);
    assert_eq!(
        trace["outcome"],
        json!("no_effect"),
        "both reads serve — the CLI lane's own answer: {trace}"
    );
    assert!(trace.get("fault").is_none(), "no fault: {trace}");
    let rows = trace["trace"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "two recorded reads: {trace}");
    assert_eq!(
        rows[0]["face"]["Toc"]["fm"]["status"],
        json!("open"),
        "the dot-directory page serves its real face: {trace}"
    );
    assert!(
        rows[1]["face"]["Toc"]["rev"].as_str().is_some(),
        "the non-md file serves spans and mints a rev like any member: {trace}"
    );
}

/// The read-then-write flow on an out-of-domain target: the read licenses the
/// row, the CAS token values from the live disk file (§12.1: mintable at the
/// read door like any other), and the commit lands WITHOUT moving the
/// fingerprint — `fingerprint_before == fingerprint_after` across such a
/// write, the domain filter gating hashing, never load.
#[test]
fn an_out_of_domain_write_commits_without_moving_the_fingerprint() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    fs::create_dir_all(ws.join(".obsidian")).unwrap();
    fs::write(
        ws.join(".obsidian/hidden.md"),
        "---\nstatus: open\n---\n# Hidden\n",
    )
    .unwrap();
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);
    let entry = conn.fingerprint();

    let resp = conn.call(&script(
        15,
        "t = read(\".obsidian/hidden.md\")\nput(\".obsidian/hidden.md\", props={\"status\": \"done\"})\n",
    ));
    let trace = trace_of(&resp);
    assert_eq!(trace["outcome"], json!("committed"), "trace: {trace}");
    let leg = &trace["commit"];
    assert_eq!(leg["fingerprint_before"].as_str().unwrap(), entry);
    assert_eq!(
        leg["fingerprint_after"].as_str().unwrap(),
        entry,
        "out-of-domain bytes never move the fingerprint: {trace}"
    );
    let on_disk = fs::read_to_string(ws.join(".obsidian/hidden.md")).unwrap();
    assert!(on_disk.contains("status: done"), "landed: {on_disk}");
    assert_eq!(conn.fingerprint(), entry, "the ambient world stands still");
}

// ---------------------------------------------------------------------------
// Guards: the caller's pre-eval fast-fail; the pre-splice armed-set gate.
// ---------------------------------------------------------------------------

/// A stale caller `if_fingerprint` refuses pre-eval: `conflict`, zero reads,
/// `guard_expected` carried, no commit leg.
#[test]
fn a_stale_caller_guard_refuses_pre_eval_with_zero_reads() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);

    let stale = "b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9";
    let resp = conn.call(&json!({
        "id": 13, "op": "script",
        "source": "t = read(\"doc.md\")\n",
        "if_fingerprint": stale,
    }));
    let trace = trace_of(&resp);
    assert_eq!(trace["outcome"], json!("conflict"), "trace: {trace}");
    assert_eq!(trace["guard_expected"], json!(stale));
    assert_eq!(
        trace["telemetry"]["reads_used"],
        json!(0),
        "zero evaluation"
    );
    assert!(trace.get("commit").is_none(), "no splice was issued");
}

/// An `expect_armed` mismatch refuses BEFORE the splice is issued: nothing
/// lands, the fingerprint does not advance, and the class is `fix`.
#[test]
fn an_expect_armed_mismatch_refuses_pre_splice() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);
    let before = conn.fingerprint();

    let resp = conn.call(&json!({
        "id": 14, "op": "script",
        "source": "t = read(\"doc.md\")\nput(\"doc.md\", props={\"status\": \"done\"})\n",
        "expect_armed": "armed-set-path-edit:sha256:0000000000000000000000000000000000000000000000000000000000000000",
    }));
    let trace = trace_of(&resp);
    assert_eq!(trace["outcome"], json!("refused"), "trace: {trace}");
    assert_eq!(trace["fault"]["recovery"], json!("fix"), "trace: {trace}");
    assert_eq!(
        fs::read_to_string(ws.join("doc.md")).unwrap(),
        DOC,
        "pre-splice: nothing was sent, nothing landed"
    );
    assert_eq!(conn.fingerprint(), before);
}

// ---------------------------------------------------------------------------
// Containment at the eval boundary.
// ---------------------------------------------------------------------------

/// Fuel exhaustion answers a `budget` fault trace — and the daemon serves the
/// next frame on the same connection: the containment IS the boundary.
#[test]
fn fuel_exhaustion_answers_a_budget_fault_and_the_daemon_survives() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);

    let resp = conn.call(&script(
        15,
        "x = 0\nfor i in range(1000000000):\n    x += 1\n",
    ));
    let trace = trace_of(&resp);
    assert_eq!(trace["outcome"], json!("fault"), "trace: {trace}");
    assert_eq!(trace["fault"]["class"], json!("budget"), "trace: {trace}");

    let toc = conn.call(&json!({"id": 16, "op": "toc", "path": "doc.md"}));
    assert_eq!(
        toc["ok"],
        json!(true),
        "the daemon serves the next frame: {toc}"
    );
}

/// A second content path arms a §4.4 SET — and a member that cannot validate
/// (`logs/receipts.md` has no frontmatter for `set_property`) refuses the set
/// WHOLE: the refusal names the measuring member, both armed rows stay traced
/// not-committed, and the fingerprint does not move (validate-all-then-apply;
/// the arm-time single-write-file law is retired — run-plane.md § One COMMIT
/// per attempt).
#[test]
fn a_set_member_that_cannot_validate_refuses_the_whole_set() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);
    let before = conn.fingerprint();

    let resp = conn.call(&script(
        17,
        "a = read(\"doc.md\")\nb = read(\"logs/receipts.md\")\nput(\"doc.md\", props={\"status\": \"done\"})\nput(\"logs/receipts.md\", props={\"status\": \"done\"})\n",
    ));
    let trace = trace_of(&resp);
    assert_eq!(trace["outcome"], json!("refused"), "trace: {trace}");
    let reason = trace["fault"]["reason"].as_str().unwrap();
    assert!(
        reason.contains("files[1]")
            && reason.contains("logs/receipts.md")
            && reason.contains("nothing landed"),
        "the whole-set refusal names the measuring member: {trace}"
    );
    let armed: Vec<_> = trace["trace"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == json!("armed"))
        .collect();
    assert_eq!(armed.len(), 2, "both arms stay traced: {trace}");
    assert!(
        armed.iter().all(|e| e["committed"] == json!(false)),
        "no armed row claims a commit: {trace}"
    );
    assert_eq!(conn.fingerprint(), before, "nothing landed");
}

/// § A.7 patterns: a `files[]` member carrying `*` expands at entry against
/// the entry world's hash-domain membership — the binding is the EXPANDED
/// list, the trace opens with the recorded `{pattern, matched}` row, and a
/// zero-match pattern is data (`no_effect`), never a refusal.
#[test]
fn a_files_pattern_expands_against_the_entry_world() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);

    let resp = conn.call(&json!({
        "id": 31, "op": "script",
        "source": "n = len(files)\n",
        "files": ["logs/*.md", "doc.md"],
    }));
    let trace = trace_of(&resp);
    assert_eq!(trace["outcome"], json!("no_effect"), "trace: {trace}");
    assert_eq!(
        trace["trace"][0],
        json!({"kind": "expanded", "pattern": "logs/*.md", "matched": ["logs/receipts.md"]}),
        "the expansion row opens the trace: {trace}"
    );
    assert_eq!(
        trace["bindings"]["n"],
        json!("2"),
        "the binding is the EXPANDED list (logs/receipts.md + doc.md): {trace}"
    );

    let resp = conn.call(&json!({
        "id": 32, "op": "script",
        "source": "n = len(files)\n",
        "files": ["gone/*.md"],
    }));
    let trace = trace_of(&resp);
    assert_eq!(trace["outcome"], json!("no_effect"), "zero matches is data");
    assert_eq!(
        trace["trace"][0]["matched"],
        json!([]),
        "the zero is observable, never silent: {trace}"
    );
    assert_eq!(trace["bindings"]["n"], json!("0"), "trace: {trace}");
}

/// `dry` rehearses: same trace shape, `no_effect`, no disk change, and the
/// embedded rehearsal leg carries `fingerprint_after: null`.
#[test]
fn a_dry_run_rehearses_and_lands_nothing() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);
    let before = conn.fingerprint();

    let resp = conn.call(&json!({
        "id": 18, "op": "script",
        "source": "t = read(\"doc.md\")\nput(\"doc.md\", props={\"status\": \"done\"})\n",
        "dry": true,
    }));
    let trace = trace_of(&resp);
    assert_eq!(trace["outcome"], json!("no_effect"), "trace: {trace}");
    assert_eq!(
        trace["commit"]["dry"],
        json!(true),
        "the rehearsal leg: {trace}"
    );
    assert_eq!(trace["commit"]["fingerprint_after"], Value::Null);
    assert_eq!(fs::read_to_string(ws.join("doc.md")).unwrap(), DOC);
    assert_eq!(conn.fingerprint(), before);
}

// ---------------------------------------------------------------------------
// The frame wall: strict decode; v3-only dispatch.
// ---------------------------------------------------------------------------

/// The strict wall holds at this op: an unknown request field refuses
/// `bad_request` naming the legal set — never a silent drop, never
/// `unknown_op` (which would misreport a served op as absent).
#[test]
fn an_unknown_request_field_refuses_bad_request() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(&test_config(&tmp).socket_path);
    conn.hello_v3(&ws);

    let resp = conn.call(&json!({
        "id": 19, "op": "script", "source": "x = 1\n", "budget": 5,
    }));
    assert_eq!(resp["ok"], json!(false), "{resp}");
    assert_eq!(resp["error"]["code"], json!("bad_request"), "{resp}");
}

/// The op is v3-only at dispatch: a v2 session answers `unknown_op`, and the
/// v3 caps advertise `script` while the v2 caps stay byte-identical.
#[test]
fn a_v2_session_answers_unknown_op_and_caps_split_by_rev() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let _server = RunningServer::start(test_config(&tmp)).unwrap();
    let socket = test_config(&tmp).socket_path;

    let mut v3 = Conn::open(&socket);
    let hello3 = v3.hello_v3(&ws);
    let caps3: Vec<String> = hello3["body"]["caps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap().to_owned())
        .collect();
    assert!(
        caps3.contains(&"script".to_owned()),
        "v3 advertises: {caps3:?}"
    );

    let mut v2 = Conn::open(&socket);
    let hello2 = v2.hello_v2(&ws);
    let caps2: Vec<String> = hello2["body"]["caps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap().to_owned())
        .collect();
    assert!(
        !caps2.contains(&"script".to_owned()),
        "v2 stays frozen: {caps2:?}"
    );

    let resp = v2.call(&script(20, "x = 1\n"));
    assert_eq!(resp["ok"], json!(false));
    assert_eq!(resp["error"]["code"], json!("unknown_op"), "{resp}");
}
