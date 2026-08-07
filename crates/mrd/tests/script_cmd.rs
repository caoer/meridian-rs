//! `mrd script` — the wire client, held to the laws that make it safe to run as
//! the caller: **zero wire delta**, a guard that refuses before it reads, a
//! read-class exit that touches nothing, and a commit that never retries.
//!
//! The tests drive the verb through its `Door` seam rather than a live daemon,
//! which is what makes the ops it puts on the socket assertable at all: a test
//! against a real socket can only see what changed, never what was said.

use std::io;

use effects::ReadFace;
use mrd::script::cmd::attempt;
use mrd::script::{Door, ScriptOutcome, TraceEntry};
use serde_json::{Value, json};

/// The entry fingerprint the fake daemon reports (§4.7).
const ENTRY: &str = "b3:a90f13c7ba0e1d4f5c6b7a8990112233445566778899aabbccddeeff00112233";
/// Where the world had moved to, for the conflict leg.
const MOVED: &str = "b3:88aa1f4700112233445566778899aabbccddeeff00112233445566778899aabb";

/// The one page the fake daemon serves: an unowned card, exactly golden
/// scenario 1's premise.
const CARD: &str = "tasks/0011-token-audit.md";

/// The card's whole-file word count, as the wire toc face reports it.
const WORDS: usize = 41;

/// The golden scenario 1 script, verbatim in shape: read the card, claim it if
/// nobody holds it.
const CLAIM: &str = r#"
card = read("tasks/0011-token-audit.md")
if card.fm["owner"] == "":
    put("tasks/0011-token-audit.md", props={"owner": me(), "status": "doing"})
"#;

/// A fake daemon behind the one door: it records every op it is asked for, and
/// answers the five this verb is allowed to speak.
struct Fake {
    /// Every `op` in call order — the socket census the zero-delta law needs.
    ops: Vec<String>,
    /// Every request in call order, for the shape assertions.
    requests: Vec<Value>,
    /// What the `splice` op answers. `None` = the ordinary commit frame.
    splice: Option<String>,
}

impl Fake {
    fn new() -> Self {
        Self {
            ops: Vec::new(),
            requests: Vec::new(),
            splice: None,
        }
    }

    /// Answer `splice` with `line` instead of the ordinary commit frame.
    fn answering_splice(mut self, line: &str) -> Self {
        self.splice = Some(line.to_owned());
        self
    }

    fn asked(&self, op: &str) -> bool {
        self.ops.iter().any(|seen| seen == op)
    }

    /// The one request for `op`.
    fn request(&self, op: &str) -> &Value {
        self.requests
            .iter()
            .find(|r| r["op"] == json!(op))
            .unwrap_or_else(|| panic!("no `{op}` request was sent: {:?}", self.ops))
    }
}

impl Door for Fake {
    fn call(&mut self, request: &Value) -> io::Result<String> {
        let op = request["op"].as_str().expect("every request names an op");
        self.ops.push(op.to_owned());
        self.requests.push(request.clone());
        Ok(match op {
            "fingerprint" => format!(r#"{{"ok":true,"body":{{"fingerprint":"{ENTRY}","seq":2}}}}"#),
            "toc" => json!({"ok": true, "body": {
                "path": CARD,
                "file_rev": "7c40e1a8b2f9d356",
                "fingerprint": ENTRY,
                "nodes": [
                    {"kind": "frontmatter", "span": [0, 32], "node_rev": "26796ebec5d0bf1a",
                     "text_prefix_16b": "---\nowner:\n", "keys": ["owner", "status"]},
                    {"kind": "heading", "level": 1, "hpath": [{"h": "Goals"}],
                     "span": [32, 140], "content_span": [40, 140],
                     "node_rev": "a6665baff294bd04", "text_prefix_16b": "# Goals\n\nship th"},
                ],
            }})
            .to_string(),
            // The composed read, toc mode: the op that carries `words_total`.
            "read" => json!({"ok": true, "body": {
                "path": CARD,
                "file_rev": "7c40e1a8b2f9d356",
                "root": ENTRY,
                "words_total": WORDS,
                "toc": [],
                "anchors": [],
                "rendered_text": "",
            }})
            .to_string(),
            "cat" => {
                let content = match request["sec"]["fm_key"].as_str() {
                    Some("owner") => "owner:\n",
                    Some("status") => "status: todo\n",
                    _ => "## Goals\n\nship the script entry\n",
                };
                json!({"ok": true, "body": {
                    "span": [4, 12], "node_rev": "33d5b0e1b27cb48b", "content": content
                }})
                .to_string()
            }
            "splice" => self.splice.clone().unwrap_or_else(|| {
                format!(
                    r#"{{"ok":true,"body":{{"armed":{{"edits":2}},"fingerprint_before":"{ENTRY}",
                     "fingerprint_after":"b3:c4e91d02","seq":3,"verdicts":[],
                     "receipt":{{"path":"receipts/2026-08-07.md","anchor":"r-000118"}}}}}}"#
                )
                .replace('\n', "")
            }),
            other => panic!("the script entry asked for an op it must not know: {other}"),
        })
    }
}

/// Run the claim script with `args`, against `door`.
fn claim(door: &mut Fake, flags: &[&str]) -> mrd::script::ScriptTrace {
    let argv: Vec<String> = flags.iter().map(|flag| (*flag).to_owned()).collect();
    attempt(&argv, CLAIM, door).expect("the attempt runs")
}

// ── 1. zero wire delta ────────────────────────────────────────────────────────

/// **The invariant this whole feature is sold on.** A script does all its I/O as
/// an ordinary wire client, so the ops it puts on the socket must be ops the
/// contract already declares. A new op name here is a schema delta however
/// harmless it looks — asserted as a closed set, so an addition fails rather
/// than passing unnoticed.
///
/// `hello` (§3.2) is issued by the door itself when it dials; the five this
/// attempt speaks are `fingerprint` (§4.7), `toc` (§4.1), `read` (§4.1, the
/// composed read that carries `words_total`), `cat` (§4.2) and `splice` (§4.4).
#[test]
fn the_ops_on_the_socket_are_only_the_ones_the_contract_already_declares() {
    const DECLARED: [&str; 6] = ["hello", "fingerprint", "toc", "cat", "read", "splice"];

    let mut door = Fake::new();
    let trace = claim(&mut door, &["--actor", "8ab41c02"]);
    for op in &door.ops {
        assert!(
            DECLARED.contains(&op.as_str()),
            "`{op}` is not on the wire — the script entry invents no op. Census: {:?}",
            door.ops
        );
    }
    // And the census is not vacuous: this run really did read and really did
    // commit through the one write op.
    assert!(door.asked("fingerprint") && door.asked("toc") && door.asked("splice"));
    assert_eq!(trace.outcome, ScriptOutcome::Committed);
}

/// `read(path)` IS the wire toc face 1:1, so the wire's own `words_total` rides
/// the recorded face — a delivered fact the host carries, never one it computes
/// (ruling 2026-08-07). The count comes from the composed `read` op: the `toc`
/// op's body is `{path, file_rev, root, nodes}` and carries none. Answering 0
/// instead renders `words:0` on a live face while the goldens render the true
/// count, and nothing in a passing suite says so.
#[test]
fn the_recorded_toc_face_carries_the_wire_word_count() {
    let mut door = Fake::new();
    let trace = claim(&mut door, &["--actor", "8ab41c02"]);

    let TraceEntry::Echo(read) = &trace.trace[0] else {
        panic!(
            "the first traced entry is the echoed read: {:?}",
            trace.trace
        );
    };
    let ReadFace::Toc(facts) = &read.face else {
        panic!("read(path) records the toc face");
    };
    assert_eq!(
        facts.words, WORDS,
        "the composed read's words_total rides the face"
    );
}

/// The commit is ONE splice, and it speaks the wire's second edit dialect: the
/// armed list IS `plan_edits[]` (ruling B′), mutually exclusive with `edits[]`.
/// Its guard is the ENTRY fingerprint — the premise the reads were consistent
/// with, never a value re-sampled at commit time, which would guard on nothing.
#[test]
fn the_one_splice_carries_plan_edits_guarded_on_the_entry_fingerprint() {
    let mut door = Fake::new();
    claim(
        &mut door,
        &["--actor", "8ab41c02", "--now", "2026-08-07T12:00:00Z"],
    );

    assert_eq!(
        door.ops.iter().filter(|op| *op == "splice").count(),
        1,
        "one script, one commit"
    );
    let splice = door.request("splice");
    assert_eq!(splice["if_fingerprint"], json!(ENTRY));
    assert_eq!(splice["path"], json!(CARD));
    assert_eq!(splice["actor"], json!("8ab41c02"));
    assert_eq!(splice["now"], json!("2026-08-07T12:00:00Z"));
    assert!(
        splice["edits"].is_null(),
        "plan_edits and edits are mutually exclusive: {splice}"
    );
    let plan = splice["plan_edits"].as_array().expect("plan_edits[]");
    assert_eq!(plan.len(), 2, "one set_property per key, sorted: {splice}");
    assert_eq!(plan[0]["set_property"]["key"], json!("owner"));
    assert_eq!(plan[0]["set_property"]["value"], json!("8ab41c02"));
    assert_eq!(plan[1]["set_property"]["key"], json!("status"));
}

// ── 1b. the per-row CAS token ─────────────────────────────────────────────────

/// Every wire door demands a fingerprint for an edit that changes existing
/// content, at the grain the row writes: `set_property` takes the FILE rev,
/// because frontmatter semantics are file-scoped.
///
/// The token is the one the script ITSELF read — the read-then-write CAS,
/// carried from the recording rather than minted at commit time. The trace shows
/// the same row that went on the wire, so a reader can see what the write was
/// guarded on.
#[test]
fn a_property_row_carries_the_file_rev_the_script_read() {
    let mut door = Fake::new();
    let trace = claim(&mut door, &["--actor", "8ab41c02"]);

    let plan = door.request("splice")["plan_edits"]
        .as_array()
        .expect("plan_edits[]")
        .clone();
    for row in &plan {
        assert_eq!(
            row["set_property"]["rev"],
            json!("7c40e1a8b2f9d356"),
            "the file rev the toc read published: {row}"
        );
    }
    let traced = serde_json::to_value(trace.armed_entries().next().expect("an armed row"))
        .expect("the trace serializes");
    assert_eq!(
        traced["edit"], plan[0],
        "the trace shows what went on the wire, token included"
    );
}

/// An `append` is guarded at NODE grain — the rev of the section it lands in,
/// which the same toc read already published in its section map.
#[test]
fn an_append_row_carries_the_section_rev_from_the_same_read() {
    let mut door = Fake::new();
    let argv = ["--actor".to_owned(), "8ab41c02".to_owned()];
    let source = "board = read(\"tasks/0011-token-audit.md\")\n\
                  put(\"tasks/0011-token-audit.md\", section=\"Goals\", append=\"- done\\n\")\n";
    attempt(&argv, source, &mut door).expect("the attempt runs");

    let row = &door.request("splice")["plan_edits"][0]["append"];
    assert_eq!(
        row["hpath"],
        json!([{"h": "Goals"}]),
        "addresses are segments"
    );
    assert_eq!(
        row["rev"],
        json!("a6665baff294bd04"),
        "the Goals row's node rev, from the script's own toc read"
    );
}

/// A write to a page the script never read carries NO token — and meets the
/// engine's own teaching refusal. Nothing is minted to get past the guard: a
/// script that writes what it did not read has no picture to guard on.
#[test]
fn a_write_to_an_unread_page_carries_no_token_and_the_engine_refuses_it() {
    const REFUSAL: &str = r#"{"ok":false,"error":{"code":"guard_required","message":"frontmatter key \"owner\" changes existing content with no fingerprint"}}"#;
    let mut door = Fake::new().answering_splice(REFUSAL);
    let argv = ["--actor".to_owned(), "8ab41c02".to_owned()];
    let trace = attempt(
        &argv,
        "put(\"tasks/0011-token-audit.md\", props={\"owner\": me()})\n",
        &mut door,
    )
    .expect("the attempt runs");

    assert!(!door.asked("toc"), "the script read nothing");
    assert!(
        door.request("splice")["plan_edits"][0]["set_property"]["rev"].is_null(),
        "absence stays absence"
    );
    assert_eq!(trace.outcome, ScriptOutcome::Refused);
}

// ── 2. the guard refuses before it reads ──────────────────────────────────────

/// A caller guard that does not match the entry fingerprint refuses at the door:
/// **zero evaluation** — no read reaches the socket, nothing is armed, and no
/// splice is issued. The recorded census is the proof; a test that only checked
/// the outcome could not tell a pre-eval refusal from a post-eval one.
#[test]
fn a_stale_caller_guard_performs_zero_reads_and_zero_splices() {
    let mut door = Fake::new();
    let trace = claim(&mut door, &["--if-fingerprint", MOVED]);

    assert_eq!(
        door.ops,
        ["fingerprint"],
        "the guard refuses on the fingerprint alone"
    );
    assert_eq!(trace.outcome, ScriptOutcome::Conflict);
    assert!(trace.trace.is_empty(), "nothing was read and nothing armed");
    assert!(trace.commit.is_none(), "no splice, so no commit leg");
    assert_eq!(trace.telemetry.reads_used, 0);

    // The face's extras line needs BOTH tokens, and it renders from this trace
    // and nothing else: `actual` IS the entry fingerprint, and `expected` is the
    // caller's pinned value carried in band — the same pair the commit-time
    // mismatch embeds, so one family renders one way.
    assert_eq!(trace.entry_fingerprint, ENTRY, "actual");
    assert_eq!(trace.guard_expected.as_deref(), Some(MOVED), "expected");
    let json: Value = serde_json::to_value(&trace).expect("the contract serializes");
    assert_eq!(json["guard_expected"], json!(MOVED));
}

/// `guard_expected` is present EXACTLY on a pre-eval guard refusal — it is the
/// discriminator U7/U8 render "guard refused" from, so a run that reached the
/// commit must not carry it or the two conflict terminals stop grepping apart.
#[test]
fn only_a_pre_eval_guard_refusal_carries_the_pinned_expected_token() {
    let mut door = Fake::new();
    let committed = claim(
        &mut door,
        &["--actor", "8ab41c02", "--if-fingerprint", ENTRY],
    );
    assert_eq!(committed.outcome, ScriptOutcome::Committed);
    assert!(
        committed.guard_expected.is_none(),
        "the guard passed, so there is no refused premise to name"
    );
    assert!(
        serde_json::to_value(&committed).expect("json")["guard_expected"].is_null(),
        "absence stays absence on the wire-facing contract too"
    );
}

/// The same guard, matching, is a courtesy check and nothing more: the run
/// proceeds and the commit still carries the value, so a world that moves DURING
/// evaluation is still caught. Two checks, one value.
#[test]
fn a_matching_caller_guard_still_rides_the_commit() {
    let mut door = Fake::new();
    let trace = claim(
        &mut door,
        &["--actor", "8ab41c02", "--if-fingerprint", ENTRY],
    );

    assert_eq!(trace.outcome, ScriptOutcome::Committed);
    assert_eq!(door.request("splice")["if_fingerprint"], json!(ENTRY));
}

// ── 3. the read-class exit ────────────────────────────────────────────────────

/// A script that arms nothing is a READ. It issues no splice at all — no
/// receipt, no fingerprint advance, nothing on disk — and `no_effect` is a
/// first-class success, not a failure with a nicer word.
#[test]
fn a_zero_armed_run_issues_no_splice_at_all() {
    let mut door = Fake::new();
    let argv = ["--actor".to_owned(), "8ab41c02".to_owned()];
    let trace = attempt(
        &argv,
        "card = read(\"tasks/0011-token-audit.md\")\n",
        &mut door,
    )
    .expect("the attempt runs");

    assert!(!door.asked("splice"), "census: {:?}", door.ops);
    assert_eq!(trace.outcome, ScriptOutcome::NoEffect);
    assert!(trace.commit.is_none());
    assert_eq!(
        trace.trace.len(),
        1,
        "the read is still traced — reading is not nothing"
    );
    assert!(matches!(trace.trace[0], TraceEntry::Echo(_)));
}

// ── 4. the world moved ────────────────────────────────────────────────────────

/// A write that lands between the entry fingerprint and the commit makes the
/// splice refuse `fingerprint_mismatch`, and the ENTIRE batch fails: the
/// workspace is untouched. The mismatch extras reach the trace as the daemon's
/// own bytes — no re-typing, so `{expected, actual, changed}` cannot drift.
///
/// The entry does not retry. It answers `conflict` once and hands the decision
/// up, because a retry loop inside the entry would re-run reads against a world
/// the caller has not seen.
#[test]
fn a_world_that_moved_yields_conflict_with_the_mismatch_extras_verbatim() {
    const MISMATCH: &str = r#"{"ok":false,"error":{"code":"fingerprint_mismatch","recovery":"resync","expected":"ENTRY_FP","actual":"MOVED_FP","changed":["tasks/0011-token-audit.md"]}}"#;
    let mut door = Fake::new().answering_splice(
        &MISMATCH
            .replace("ENTRY_FP", ENTRY)
            .replace("MOVED_FP", MOVED),
    );
    let trace = claim(&mut door, &["--actor", "8ab41c02"]);

    assert_eq!(trace.outcome, ScriptOutcome::Conflict);
    assert_eq!(
        door.ops.iter().filter(|op| *op == "splice").count(),
        1,
        "the entry NEVER retries"
    );
    let commit: Value = serde_json::from_str(trace.commit.as_ref().expect("the leg").get())
        .expect("the leg is the daemon's own JSON");
    assert_eq!(commit["code"], json!("fingerprint_mismatch"));
    assert_eq!(commit["expected"], json!(ENTRY));
    assert_eq!(commit["actual"], json!(MOVED));
    assert_eq!(commit["changed"], json!([CARD]));
    // The armed edit is still rendered, flagged as never applied — the honesty
    // law: a reader sees what the script wanted, and that it did not happen.
    let armed: Vec<_> = trace.armed_entries().collect();
    assert_eq!(armed.len(), 2);
    assert!(armed.iter().all(|entry| !entry.committed));
}

/// A commit-time refusal that is NOT the world moving — `would_corrupt`,
/// `foreign_edit`, an overlap — is a refusal, not a fault, and the two grep
/// apart. The engine's own wording rides verbatim rather than being re-phrased
/// here, which would fork the text into two places.
#[test]
fn a_commit_time_refusal_is_refused_and_carries_the_engines_own_words() {
    const REFUSAL: &str = r#"{"ok":false,"error":{"code":"would_corrupt","message":"the candidate loses containment of \"Goals\""}}"#;
    let mut door = Fake::new().answering_splice(REFUSAL);
    let trace = claim(&mut door, &["--actor", "8ab41c02"]);

    assert_eq!(trace.outcome, ScriptOutcome::Refused);
    assert!(
        trace.commit.is_none(),
        "nothing was applied, so no commit leg"
    );
    let fault = trace.fault.as_ref().expect("a refusal names itself");
    assert!(
        fault.reason.contains("would_corrupt") && fault.reason.contains("loses containment"),
        "the engine's own wording, verbatim: {}",
        fault.reason
    );
}

// ── 5. the rehearsal ──────────────────────────────────────────────────────────

/// `--dry` runs everything except disk: the daemon builds the whole effect set
/// and applies none of it. The trace therefore carries the full commit leg with
/// `fingerprint_after: null`, and every armed entry stays `[not committed]` —
/// the workspace is unchanged, which is the promise `--dry` exists to make.
#[test]
fn a_dry_run_returns_the_full_effect_set_with_no_fingerprint_after() {
    let mut door = Fake::new().answering_splice(&format!(
        r#"{{"ok":true,"body":{{"armed":{{"edits":2}},"fingerprint_before":"{ENTRY}","fingerprint_after":null,"dry":true,"verdicts":[]}}}}"#
    ));
    let trace = claim(&mut door, &["--actor", "8ab41c02", "--dry"]);

    assert_eq!(door.request("splice")["dry"], json!(true));
    assert_eq!(
        trace.outcome,
        ScriptOutcome::NoEffect,
        "a rehearsal changes nothing, so it landed no effect"
    );
    let commit: Value = serde_json::from_str(trace.commit.as_ref().expect("the leg").get())
        .expect("the effect set is still returned");
    assert_eq!(commit["fingerprint_after"], Value::Null);
    assert_eq!(commit["armed"]["edits"], json!(2));
    assert_eq!(commit["dry"], json!(true));
    let armed: Vec<_> = trace.armed_entries().collect();
    assert_eq!(armed.len(), 2);
    assert!(
        armed.iter().all(|entry| !entry.committed),
        "a rehearsal commits nothing"
    );
}

// ── 6. the inert inputs ───────────────────────────────────────────────────────

/// `files[]` is paths only, sorted by the host, and `args` is a JSON **object**
/// bound as a dict — callers name their inputs, they do not count them. Both are
/// inert bindings: a script reaches content only through `read()`, which is what
/// keeps a run replayable.
#[test]
fn files_are_sorted_paths_and_args_is_a_json_object() {
    let mut door = Fake::new();
    let argv: Vec<String> = [
        "--files",
        "tasks/z.md",
        "--files",
        "tasks/a.md",
        "--args",
        r#"{"who":"one","what":"two"}"#,
    ]
    .iter()
    .map(|a| (*a).to_owned())
    .collect();
    let trace = attempt(
        &argv,
        "seen = [files[0], files[1], args[\"who\"], sorted(args.keys())]\n",
        &mut door,
    )
    .expect("the attempt runs");

    assert_eq!(trace.outcome, ScriptOutcome::NoEffect);
    assert!(!door.asked("toc"), "no read() call, so no read op");
}

/// `--args` that is not a JSON object of strings refuses BEFORE anything is
/// evaluated — a bad invocation, not a fault, and the socket is never dialled.
/// A JSON **array** is the shape this entry used to take, so it must refuse by
/// name rather than decode into something adjacent.
#[test]
fn a_malformed_args_json_refuses_before_evaluation() {
    for bad in [r#"["one","two"]"#, r#"{"n":3}"#, "not json"] {
        let mut door = Fake::new();
        let argv = ["--args".to_owned(), bad.to_owned()];
        let refusal = attempt(&argv, CLAIM, &mut door).expect_err("a bad invocation");

        assert!(
            refusal.contains("JSON object of strings"),
            "the refusal names the shape it wanted, for {bad}: {refusal}"
        );
        assert!(door.ops.is_empty(), "nothing was said on the wire");
    }
}
