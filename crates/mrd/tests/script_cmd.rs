//! `mrd script` — the wire client, held to the laws that make it safe to run as
//! the caller: **zero wire delta**, **one lane**, a request that carries exactly
//! the caller's own inputs and invents no premise, and a trace it renders
//! without re-typing a single daemon fact.
//!
//! ⭐ **THE SUBJECT MOVED** (card
//! `script-door-commit-premise-world-grain-vs-touch-set`). Until this card the
//! verb had TWO lanes: a `files[]` carrying a glob forwarded to the wire
//! `script` op, and everything else drove a LOCAL transaction here — reads
//! lowered to `toc`/`cat`, the commit went out as a `splice`, and that splice
//! carried `if_fingerprint` = the whole-corpus entry fingerprint. That
//! world-grain premise is the law `run-plane.md`:929-942 records as amended and
//! DELETED: it refused a 64-file slice on fleet churn that never touched one of
//! its 64 targets.
//!
//! So the fork is gone and the whole attempt is ONE wire `script` op. What this
//! file can still assert is therefore exactly what the CLI still does: which ops
//! it speaks, what rides the request, and what it does with the answer. The
//! behaviors that moved into the daemon are not dropped silently — § Retired
//! against named twins, at the bottom, names the daemon-side test that owns each
//! one.
//!
//! The tests drive the verb through its `Door` seam rather than a live daemon,
//! which is what makes the ops it puts on the socket assertable at all: a test
//! against a real socket can only see what changed, never what was said.

use std::io;

use mrd::script::cmd::attempt;
use mrd::script::{Door, FaultClass, ScriptOutcome, ScriptTrace};
use serde_json::{Value, json};
use wire::Recovery;

/// The entry fingerprint the fake daemon reports inside its trace (§4.7).
/// **It is the DAEMON's value**: this lane no longer mints one.
const ENTRY: &str = "b3:a90f13c7ba0e1d4f5c6b7a8990112233445566778899aabbccddeeff00112233";
/// Where the world had moved to, for the conflict leg and the caller's guard.
const MOVED: &str = "b3:88aa1f4700112233445566778899aabbccddeeff00112233445566778899aabb";

/// The one page the scripts here name: an unowned card, golden scenario 1's
/// premise.
const CARD: &str = "tasks/0011-token-audit.md";

/// The golden scenario 1 script, verbatim in shape: read the card, claim it if
/// nobody holds it. Under one lane it is evaluated by the DAEMON — this process
/// only puts it on the wire.
const CLAIM: &str = r#"
card = read("tasks/0011-token-audit.md")
if card["fm"]["owner"] == "":
    put("tasks/0011-token-audit.md", props={"owner": me(), "status": "doing"})
"#;

/// A whole `ScriptTrace` as the daemon serves a committed run. `forward()`
/// deserializes the body as one, so a fake answering `script` produces a
/// complete trace, never a fragment.
fn committed() -> Value {
    json!({
        "entry_fingerprint": ENTRY,
        "outcome": "committed",
        "trace": [],
        "commit": {"armed": {"edits": 2}, "fingerprint_before": ENTRY,
                   "fingerprint_after": "b3:c4e91d02", "seq": 3, "verdicts": [],
                   "receipt": {"path": "receipts/2026-08-07.md", "anchor": "r-000118"}},
        "armed_digest": "armed-set-path-edit:sha256:11223344556677889900aabbccddeeff\
                         11223344556677889900aabbccddeeff",
        "telemetry": {"fuel_used": 812, "mem_used": 4096, "reads_used": 1, "wall_ms": 3},
    })
}

/// The daemon's own `conflict`: the §4.6 touch-set guard refusing, with the
/// mismatch extras — `scope` included — riding the commit leg verbatim.
fn touch_set_conflict() -> Value {
    json!({
        "entry_fingerprint": ENTRY,
        "outcome": "conflict",
        "trace": [],
        "commit": {"code": "fingerprint_mismatch", "recovery": "resync",
                   "expected": ENTRY, "actual": MOVED, "scope": CARD},
        "telemetry": {"fuel_used": 812, "mem_used": 4096, "reads_used": 1, "wall_ms": 3},
    })
}

/// A refused trace carrying the §8 triple the daemon derived
/// (`registry::script_op::refusal_of`). `recovery` is `None` to spell a frame
/// that carried no class this engine could type.
fn refused(code: Option<&str>, recovery: Option<&str>, reason: &str) -> Value {
    let mut fault = json!({"class": "refused", "reason": reason});
    if let Some(code) = code {
        fault["code"] = json!(code);
    }
    if let Some(recovery) = recovery {
        fault["recovery"] = json!(recovery);
    }
    json!({
        "entry_fingerprint": ENTRY,
        "outcome": "refused",
        "trace": [],
        "fault": fault,
        "telemetry": {"fuel_used": 812, "mem_used": 4096, "reads_used": 1, "wall_ms": 3},
    })
}

/// A fake daemon behind the one door. It records every op it is asked for — the
/// socket census the zero-delta and one-lane laws need — and answers the ONE op
/// this verb is allowed to speak.
struct Fake {
    /// Every `op` in call order.
    ops: Vec<String>,
    /// Every request in call order, for the shape assertions.
    requests: Vec<Value>,
    /// The trace body to answer with. `None` = the ordinary committed trace.
    body: Option<Value>,
}

impl Fake {
    fn new() -> Self {
        Self {
            ops: Vec::new(),
            requests: Vec::new(),
            body: None,
        }
    }

    /// Answer `script` with `body` instead of the ordinary committed trace.
    fn answering(mut self, body: Value) -> Self {
        self.body = Some(body);
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
        assert_eq!(
            op, "script",
            "ONE LANE: the whole attempt is the wire `script` op. A `{op}` trip \
             means the local transaction is alive again: {request}"
        );
        let body = self.body.clone().unwrap_or_else(committed);
        Ok(json!({"id": null, "ok": true, "body": body}).to_string())
    }
}

/// Run the claim script with `args`, against `door`.
fn claim(door: &mut Fake, flags: &[&str]) -> ScriptTrace {
    let argv: Vec<String> = flags.iter().map(|flag| (*flag).to_owned()).collect();
    attempt(&argv, CLAIM, door).expect("the attempt runs")
}

// ── 1. zero wire delta, and ONE lane ─────────────────────────────────────────

/// **The invariant this whole feature is sold on, now at its sharpest.** A
/// script does all its I/O as an ordinary wire client, so the ops it puts on the
/// socket must be ops the contract already declares. Asserted as a closed set,
/// so an addition fails rather than passing unnoticed.
///
/// The set SHRANK with this card: `hello` (§3.2) is issued by the door itself
/// when it dials, and the attempt speaks exactly one op — `script` (§ A.7). The
/// five the local transaction used to speak (`fingerprint`, `toc`, `cat`,
/// `read`, `splice`) are the daemon's now, spoken in-process where they cost no
/// round trip.
#[test]
fn the_ops_on_the_socket_are_only_the_ones_the_contract_already_declares() {
    const DECLARED: [&str; 2] = ["hello", "script"];

    let mut door = Fake::new();
    let trace = claim(&mut door, &["--actor", "8ab41c02"]);
    for op in &door.ops {
        assert!(
            DECLARED.contains(&op.as_str()),
            "`{op}` is not on the wire — the script entry invents no op. Census: {:?}",
            door.ops
        );
    }
    // And the census is not vacuous: this run really did forward, exactly once.
    assert_eq!(door.ops, ["script"], "one attempt, one trip");
    assert_eq!(trace.outcome, ScriptOutcome::Committed);
}

/// ⭐ **THE FLIP. This test used to be named
/// `the_one_splice_carries_plan_edits_guarded_on_the_entry_fingerprint`, and it
/// pinned the defect.**
///
/// It asserted that the CLI minted a whole-corpus entry fingerprint and put it
/// on its own `splice` as `if_fingerprint` — the world-grain premise that
/// refused a 64-file slice because a memo landed somewhere else in the corpus.
/// `run-plane.md`:929-942 records that law as amended and DELETED, and :930
/// names this lane: *"the touch-set law covers ALL script lanes (S1), same
/// product as MCP `script`"*.
///
/// So the fixture is inverted rather than deleted. What must now be true is that
/// this lane pins NOTHING of its own: no `fingerprint` trip, no `splice`, and no
/// `if_fingerprint` on the one request it does send. The commit's authority is
/// the daemon's touch set, and the only way a world-grain premise can ride again
/// is if the CALLER asks for one (§ D-04, pinned below).
#[test]
fn the_attempt_pins_no_world_grain_premise_of_its_own() {
    let mut door = Fake::new();
    claim(
        &mut door,
        &["--actor", "8ab41c02", "--now", "2026-08-07T12:00:00Z"],
    );

    assert!(
        !door.asked("fingerprint"),
        "no entry is minted HERE — the daemon pins it: {:?}",
        door.ops
    );
    assert!(
        !door.asked("splice"),
        "no second commit path exists: {:?}",
        door.ops
    );
    let sent = door.request("script");
    assert!(
        sent.get("if_fingerprint").is_none(),
        "THE DEFECT, INVERTED: a caller who pinned nothing gets no world-grain \
         premise. A token here refuses every run that fleet churn overtakes, \
         wherever in the corpus it lands: {sent}"
    );
    // The caller's own inputs still ride, unchanged — the flip removes a premise
    // this lane invented, never an input the caller gave.
    assert_eq!(sent["source"], json!(CLAIM));
    assert_eq!(sent["actor"], json!("8ab41c02"));
    assert_eq!(sent["now"], json!("2026-08-07T12:00:00Z"));
}

/// The request-surface census: every field the parsed invocation holds reaches
/// the wire, and nothing else does. A field silently dropped here is a flag that
/// stops working with no diagnostic — the failure mode a one-lane forward makes
/// possible, so it gets its own row.
#[test]
fn every_parsed_input_rides_the_one_request_and_nothing_is_invented() {
    let mut door = Fake::new();
    claim(
        &mut door,
        &[
            "--actor",
            "8ab41c02",
            "--now",
            "2026-08-07T12:00:00Z",
            "--receipt",
            "receipts/2026-08-07.md#r-000118",
            "--if-fingerprint",
            ENTRY,
            "--expect-armed",
            "armed-set-path-edit:sha256:00",
            "--files",
            "tasks/z.md",
            "--files",
            "tasks/a.md",
            "--args",
            r#"{"who":"one"}"#,
            "--dry",
        ],
    );

    let sent = door.request("script");
    assert_eq!(sent["op"], json!("script"));
    assert_eq!(sent["source"], json!(CLAIM));
    assert_eq!(sent["actor"], json!("8ab41c02"));
    assert_eq!(sent["now"], json!("2026-08-07T12:00:00Z"));
    assert_eq!(
        sent["receipt"],
        json!({"path": "receipts/2026-08-07.md", "anchor": "r-000118"})
    );
    assert_eq!(sent["if_fingerprint"], json!(ENTRY));
    assert_eq!(sent["expect_armed"], json!("armed-set-path-edit:sha256:00"));
    assert_eq!(
        sent["files"],
        json!(["tasks/z.md", "tasks/a.md"]),
        "files[i] is the i-th --files the caller typed, not the lexical first \
         (order-bind ruling): {sent}"
    );
    assert_eq!(
        sent["args"],
        json!({"who": "one"}),
        "--args is a JSON OBJECT bound as a dict — callers name their inputs, \
         they do not count them: {sent}"
    );
    assert_eq!(sent["dry"], json!(true));
}

// ── 2. the caller's own guard — D-04, preserved ──────────────────────────────

/// **D-04 preserved.** Deleting the lane's own world-grain premise does not
/// delete the caller's. `--if-fingerprint` still rides the wire verbatim, as the
/// §A.7 request's own declared field, and the daemon applies it as a WIDENING
/// premise on top of the touch set — strictest wins.
///
/// The pair matters: absent flag ⇒ absent field (the flip above), present flag ⇒
/// the caller's exact bytes. A lane that dropped the field would silently ungate
/// every guarded call, which is worse than the defect this card fixes.
#[test]
fn a_caller_pin_still_rides_the_wire_as_the_declared_if_fingerprint_field() {
    let mut door = Fake::new();
    let trace = claim(
        &mut door,
        &["--actor", "8ab41c02", "--if-fingerprint", ENTRY],
    );

    assert_eq!(
        door.request("script")["if_fingerprint"],
        json!(ENTRY),
        "the caller's own token, byte for byte — never re-minted here"
    );
    assert_eq!(trace.outcome, ScriptOutcome::Committed);
}

/// A malformed pin is the DAEMON's to refuse — it owns the §A.7 malformed arm
/// now — and this lane must not pre-judge it: the bytes ride verbatim, however
/// hostile, so the refusal a caller reads is the engine's own teaching rather
/// than a second opinion minted here.
#[test]
fn a_malformed_caller_pin_rides_verbatim_rather_than_being_judged_here() {
    let spaced = format!(" {ENTRY}");
    for pin in [spaced.as_str(), "b3c:abcd", "not-a-token", "b3c:", "absent"] {
        let mut door = Fake::new().answering(refused(
            None,
            Some("fix"),
            &format!("the entry pin {pin:?} is not a Root-family token"),
        ));
        let trace = claim(&mut door, &["--if-fingerprint", pin]);

        assert_eq!(
            door.request("script")["if_fingerprint"],
            json!(pin),
            "carried verbatim, not normalized: {pin:?}"
        );
        assert_eq!(trace.outcome, ScriptOutcome::Refused, "{pin:?}");
        assert!(
            trace
                .fault
                .as_ref()
                .expect("a refusal names itself")
                .reason
                .contains(&format!("{pin:?}")),
            "the daemon's teaching debug-quotes the bytes and this lane carries \
             it: {pin:?}"
        );
    }
}

// ── 3. the world moved ───────────────────────────────────────────────────────

/// ⭐ **THE SECOND FLIP. This test used to be named
/// `a_world_that_moved_yields_conflict_with_the_mismatch_extras_verbatim`, and
/// its fixture was a `splice` answering `fingerprint_mismatch` on the
/// WHOLE-CORPUS guard.**
///
/// The conflict survives; its grain does not. A moved world no longer refuses
/// this run — a foreign write OUTSIDE the touch set never reaches the guard at
/// all. What refuses is a move INSIDE the set the attempt actually touched, and
/// the §5.7 refusal names that premise's SCOPE. The old fixture could not carry
/// `scope`, because a root-grain premise has none: that missing field is the
/// measured signature of the deleted law (card § Context — the production
/// refusal "did not name the moved premise's SCOPE, which run-plane.md says a
/// due refusal does").
///
/// The lane's own duty is unchanged and still asserted: carry the daemon's bytes
/// verbatim, and NEVER retry.
#[test]
fn a_moved_touch_set_yields_conflict_with_the_scope_bearing_extras_verbatim() {
    let mut door = Fake::new().answering(touch_set_conflict());
    let trace = claim(&mut door, &["--actor", "8ab41c02"]);

    assert_eq!(trace.outcome, ScriptOutcome::Conflict);
    assert_eq!(
        door.ops.iter().filter(|op| *op == "script").count(),
        1,
        "the entry NEVER retries — the retry loop is the host's"
    );
    let commit: Value = serde_json::from_str(trace.commit.as_ref().expect("the leg").get())
        .expect("the leg is the daemon's own JSON");
    assert_eq!(commit["code"], json!("fingerprint_mismatch"));
    assert_eq!(commit["expected"], json!(ENTRY));
    assert_eq!(commit["actual"], json!(MOVED));
    assert_eq!(
        commit["scope"],
        json!(CARD),
        "the touch-set refusal names the moved premise's scope (§5.7). A refusal \
         with no scope is the world-grain guard this card deleted: {commit}"
    );
    // `changed` is STRUCK (§18 row 2, ruled 2026-08-10): no producer ever
    // minted it, so the old assertion here passed forever over a shape no
    // daemon can emit. Asserting its ABSENCE is what a re-introduction would
    // have to answer for.
    assert!(
        commit.get("changed").is_none(),
        "`changed` was struck because nothing mints it; a fixture that carries \
         it again is imagination, not a record: {commit}"
    );
    assert!(
        trace.fault.is_none(),
        "a moved world is an ANSWER, not a fault — a consumer greps the two apart"
    );
}

// ── 4. the inert inputs, at the seam that still owns them ────────────────────

/// `--args` that is not a JSON object of strings refuses BEFORE anything is
/// evaluated — a bad invocation, not a fault, and the socket is never dialled.
/// A JSON **array** is the shape this entry used to take, so it must refuse by
/// name rather than decode into something adjacent.
///
/// This law stayed here through the lane change: argv parsing is the CLI's, and
/// a refusal that never touches the wire is the CLI's to prove.
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

// ── 5. the refusal triple crosses the boundary TYPED ─────────────────────────

/// The only thing a class-reading consumer is allowed to look at.
///
/// It reads the TYPED field and nothing else — no prose, no code spelling. A
/// consumer written this way cannot be broken by a re-worded engine message,
/// which is the whole point of carrying the class instead of rendering it.
fn transient(trace: &ScriptTrace) -> bool {
    trace
        .fault
        .as_ref()
        .and_then(|fault| fault.recovery)
        .is_some_and(|recovery| recovery == Recovery::Retry)
}

/// **The boundary crossing, which is the half that stayed here.** The daemon
/// DERIVES the triple (`registry::script_op::refusal_of` — frame's own class
/// first, then the §8 frozen table via the code, then absence); this lane's duty
/// is to carry all three across the trace boundary without flattening any of
/// them into prose. That was destroyed before the boundary until the triple
/// existed: the frame carried the class, the script path rendered it away, and
/// no host-side change could recover it.
///
/// Three rows in one table, because the property is one property and the
/// distinctions are inputs: a transient class reads transient, a permanent one
/// reads permanent, and an absent one stays ABSENT — never a guess and never a
/// default. A defaulted class would read as the daemon's word when nothing said
/// it.
#[test]
fn the_typed_triple_crosses_the_trace_boundary_and_absence_stays_absence() {
    const CASES: [(&str, Option<&str>, Option<Recovery>, bool); 3] = [
        // the daemon's own `workspace_busy` — the contention a cooperating
        // writer causes, and the one class a host may retry on
        ("workspace_busy", Some("retry"), Some(Recovery::Retry), true),
        // permanent: the caller's to fix, and it must not read as transient
        ("would_corrupt", Some("fix"), Some(Recovery::Fix), false),
        // a code from a later engine, with no class beside it
        ("a_code_from_a_later_engine", None, None, false),
    ];

    for (code, class, expected, is_transient) in CASES {
        let mut door = Fake::new().answering(refused(
            Some(code),
            class,
            "the engine's own wording, verbatim",
        ));
        let trace = claim(&mut door, &["--actor", "8ab41c02"]);

        assert_eq!(trace.outcome, ScriptOutcome::Refused, "for {code}");
        let fault = trace.fault.as_ref().expect("a refusal names itself");
        assert_eq!(fault.class, FaultClass::Refused, "for {code}");
        assert_eq!(
            fault.code.as_deref(),
            Some(code),
            "the code is carried verbatim even when this engine cannot type it"
        );
        assert_eq!(
            fault.recovery, expected,
            "{code}: the class crosses typed, and absence stays absence"
        );
        assert_eq!(
            transient(&trace),
            is_transient,
            "{code}: a consumer that reads only the type sees the class"
        );
        assert_eq!(
            fault.reason, "the engine's own wording, verbatim",
            "the rendering is unchanged to the byte — the class rides beside it, \
             never inside it"
        );
    }
}

/// **The migration is additive, measured rather than argued.** A trace
/// serialized before `code` and `recovery` existed still deserializes, with both
/// new fields absent.
#[test]
fn the_typed_class_is_additive_to_every_consumer_that_reads_the_prose() {
    // A trace serialized by the engine BEFORE the two fields existed.
    const PRE_CHANGE: &str = r#"{
      "entry_fingerprint": "b3:a90f13c7",
      "outcome": "refused",
      "trace": [],
      "fault": {"class": "refused", "reason": "would_corrupt: an older engine's words"},
      "telemetry": {"fuel_used": 0, "mem_used": 0, "reads_used": 0, "wall_ms": 0}
    }"#;

    let old: ScriptTrace =
        serde_json::from_str(PRE_CHANGE).expect("a pre-change trace still deserializes");
    let fault = old.fault.expect("its fault survives the round trip");
    assert_eq!(fault.code, None);
    assert_eq!(fault.recovery, None);
    assert_eq!(fault.reason, "would_corrupt: an older engine's words");
}

// ── 6. one address grammar, one parser, on the whole script path ─────────────

/// **`ReadSel::parse` is the SINGLE entry for a section spelling on the script
/// path.** Two parsers for one token is how `^r-000118` became a heading named
/// `^r-000118` on the write face while the read face addressed a block, and how
/// a leading slash decided a CAS token. The rule is only worth stating if it is
/// mechanically held, so this reads the sources: any `split('/')` on the script
/// path is a second parser being born.
///
/// It names its files rather than scanning a tree, so a new file on this path
/// joins the list deliberately — the same discipline `scriptexecgate_test.go`
/// applies to exec sites in the host.
///
/// **PR 2 note, discharged.** `src/script/cmd.rs` (`section_rev_of`) and
/// `src/script/wire_host.rs` (`sec_ref`) were on this list while the local
/// transaction stood. PR 2 deleted both functions with the lane, so neither file
/// handles a section spelling any more and both rows left with them — a list
/// naming a file that does not parse sections is the drift this test exists to
/// catch, in the other direction.
///
/// The population moved rather than shrank: the surviving script path parses a
/// section spelling in exactly two places, and `../effects/src/kernel.rs` — the
/// `read(path, section=…)` boundary — JOINS the list here, having been reachable
/// on this path all along behind the CLI lane's own copy.
#[test]
fn read_sel_parse_is_the_only_section_parser_on_the_script_path() {
    // Relative to crates/mrd/, which is CARGO_MANIFEST_DIR for this target.
    const SECTION_PARSING_SOURCES: [&str; 2] = [
        "../effects/src/script_edit.rs", // section_segments — the arm side
        "../effects/src/kernel.rs",      // the read builtin's `section=` boundary
    ];
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for rel in SECTION_PARSING_SOURCES {
        let src = std::fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
        assert!(
            src.contains("ReadSel::parse"),
            "{rel} handles a section spelling and must do it through the one door"
        );
        for (n, line) in src.lines().enumerate() {
            // Prose may NAME the thing it forbids; only code can commit it.
            let code = line.trim_start();
            if code.starts_with("//") || code.starts_with('*') {
                continue;
            }
            assert!(
                !line.contains(r"split('/')"),
                "{rel}:{}: a raw `split('/')` is a SECOND section parser — \
                 route it through ReadSel::parse\n{line}",
                n + 1
            );
        }
    }
}

// ── Retired against named twins ──────────────────────────────────────────────
//
// These rows instrumented the LOCAL transaction's own dialogue — the reads it
// lowered, the tokens it threaded, the splice it issued. That dialogue is the
// daemon's now, so the rows have no subject on this side. None is dropped
// silently; each names the daemon-side test that owns the behavior today. Line
// numbers are `grep -n` at the head this PR branches from.
//
// THE SEQUENCE (reads, threading, commit)
// * `the_recorded_toc_face_carries_the_wire_word_count` — the read face and its
//   `words_total`. Served in-process by `registry::script_op::EntryWorldHost`
//   now. Twin: `crates/registry/tests/script_op.rs:149`
//   § `a_read_only_program_answers_a_no_effect_trace_at_the_entry_fingerprint`
//   (the trace's read block over a live daemon). **Named gap:** no daemon-side
//   row asserts `words_total` specifically — the count is carried by the same
//   face the twin renders, but nothing pins the number. Recorded here rather
//   than left to be discovered.
// * `a_property_row_carries_the_file_rev_the_script_read`,
//   `an_append_row_carries_the_section_rev_from_the_same_read` — per-row CAS
//   threading at both grains. Twin: `crates/registry/src/script_op.rs:1526`
//   § `threading_values_from_the_entry_world_without_a_license` (the entry-rev
//   law, both grains, license-free).
// * `a_write_to_an_unread_page_mints_its_token_at_commit_and_lands` — the CAS
//   relaxation for an unread target. Twins:
//   `crates/registry/tests/script_op.rs:277` § `a_props_write_with_no_read_commits_on_an_unmoved_world`
//   and `:306` § `an_append_with_no_read_commits_on_an_unmoved_world`.
// * `a_section_read_threads_its_rev_in_every_legal_spelling_of_one_address` —
//   spelling-independent threading. Twins: the threading twin above, plus
//   `crates/registry/tests/script_op.rs:1006` § `a_slash_bearing_section_is_readable_through_the_segment_form`
//   for the spelling half.
// * `a_zero_armed_run_issues_no_splice_at_all` — the read-class exit. Twin:
//   `crates/registry/tests/script_op.rs:149`
//   § `a_read_only_program_answers_a_no_effect_trace_at_the_entry_fingerprint`.
// * `a_commit_time_refusal_is_refused_and_carries_the_engines_own_words` — the
//   commit refusal's wording. The derivation is `registry::script_op::refusal_of`
//   (`crates/registry/src/script_op.rs:1059`); the crossing is pinned above by
//   § `the_typed_triple_crosses_the_trace_boundary_and_absence_stays_absence`.
//   **Named gap:** the daemon's `refusal_of` has no dedicated precedence test of
//   its own (frame class over table class), which the deleted CLI rows did carry.
//
// THE CALLER GUARD
// * `a_stale_caller_guard_performs_zero_reads_and_zero_splices` — Twin:
//   `crates/registry/tests/script_op.rs:464`
//   § `a_stale_caller_guard_refuses_pre_eval_with_zero_reads`.
// * `a_malformed_caller_pin_refuses_as_input_never_as_a_moved_world`,
//   `only_a_pre_eval_guard_refusal_carries_the_pinned_expected_token` — Twin:
//   `crates/registry/tests/script_op.rs:496`
//   § `a_malformed_caller_pin_refuses_as_input_on_the_wire_lane`, which asserts
//   `fault.recovery == fix` (`:518`) and the ABSENCE of `guard_expected`
//   (`:525`) — the same discriminator, on the lane that now owns it.
//
// THE REHEARSAL AND THE CLOCK
// * `a_dry_run_returns_the_full_effect_set_with_no_fingerprint_after` — the
//   rehearsal's effect set. `--dry` still rides the wire (asserted above by
//   § `every_parsed_input_rides_the_one_request_and_nothing_is_invented`); what
//   it PRODUCES is the daemon's. Twin: `crates/registry/tests/script_op.rs:830`
//   § `a_dry_run_rehearses_and_lands_nothing`.
// * `a_run_whose_clock_elapses_during_evaluation_refuses_before_the_commit`,
//   `an_engine_minted_refusal_names_its_own_class_and_no_code` (its clock half),
//   and the `StallingFake` harness — the entry wall clock. It binds in the
//   daemon now, at the same three layers. Twin:
//   `crates/registry/src/script_op.rs:2032`
//   § `the_wall_clock_binds_at_the_read_builtin`, plus the pre-commit wall site
//   at `crates/registry/src/script_op.rs:358`.
//
// THE INERT INPUTS
// * `files_bind_in_call_order_and_args_is_a_json_object` — split. The REQUEST
//   half is pinned above (§ `every_parsed_input_rides_the_one_request_and_nothing_is_invented`);
//   the BINDING half is the daemon's. Twins:
//   `crates/registry/tests/script_op.rs:755`
//   § `files_bind_in_call_order_so_the_edit_lands_on_the_typed_first_member`
//   and `:794` § `the_trace_opens_with_one_bound_row_per_files_member`.

// ── Retired against named twins — PR 2, the deletion ─────────────────────────
//
// PR 1 made the local transaction unreachable; PR 2 deleted it. Gone from
// `src/script/cmd.rs`: `run_local`, `guarded`, `mint_for`, `mint_toc`,
// `file_rev_of`, `section_rev_of`, `fingerprint`, `commit`, `lost_answer`,
// `is_mismatch`, `refusal_of`, `refusal_reason`. Gone from
// `src/script/wire_host.rs`: `WireHost` (the `effects::ScriptHost` read
// lowering), `sec_ref`, `toc_entry`. `Door`, `SocketDoor` and `Frame` stay —
// the write verbs dial the same door.
//
// **The POPULATION, counted before the cut** (PR 1's verdict, correction 1: a
// census published at the grain the author happens to notice is a floor, not a
// population). `grep -c` for `#[test]` on the two modules at the base commit:
// **4 in `cmd.rs` + 8 in `wire_host.rs` = 12**. Nine are deleted and every one
// is named below; three survive, and the count closes — 9 + 3 = 12, re-derive
// with the same `grep -c` if it ever stops closing.
//
// The three survivors, with the subject that kept them:
// * `a_files_pattern_forwards_the_attempt_to_the_daemon` — `forward()`, the
//   lane that stayed.
// * `a_socket_that_never_answers_fails_the_round_trip_instead_of_parking` and
//   `a_foreign_build_daemon_is_refused_at_connect_and_both_builds_are_named` —
//   the DIAL, which every write verb shares.
//
// Line numbers below are `grep -n` at THIS PR's head, re-verified after its own
// edits moved them.
//
// `src/script/cmd.rs` — the CAS-threading trio, all three subjects deleted
// * `a_rows_rev_is_looked_up_by_its_own_path_not_by_read_order` — **RE-HOMED,
//   not retired.** It made an accident law: `guarded()` looked a token up BY
//   `arm.path`, so a program touching two files threads each row from the file
//   THAT ROW targets. `thread_entry` keys the same way and nothing pinned it on
//   this side — the nearest twin arms two rows on ONE file, where a path-blind
//   lookup passes. Written in this PR: `crates/registry/src/script_op.rs:1603`
//   § `a_rows_entry_rev_is_looked_up_by_its_own_path_not_by_arm_order`.
// * `a_row_targeting_an_unread_path_mints_its_own_token` — the CAS relaxation
//   for an unread target. The mint is gone with the lane (the daemon threads
//   from the pinned entry world, so there is no trip to spend). Twins:
//   `crates/registry/src/script_op.rs:1526`
//   § `threading_values_from_the_entry_world_without_a_license` (license-free,
//   both grains) and `crates/registry/tests/script_op.rs:277` / `:306`
//   § `a_props_write_with_no_read_commits_on_an_unmoved_world` /
//   § `an_append_with_no_read_commits_on_an_unmoved_world` (end to end).
// * `a_refused_mint_leaves_the_row_untokened` — **class eliminated, not moved.**
//   It pinned degrade-loud on a REFUSED mint trip; in-process there is no mint
//   and no trip, so no refusal of one exists to test. The property it protected
//   — a row with no entry facts is never given a guessed token — is now
//   structural (`thread_entry` maps over `entry_toc(..) -> Option`) and the
//   commit says it out loud: an armed path absent at entry premises ABSENCE
//   (§5.6). Pinned by `crates/registry/src/script_op.rs:1726`
//   § `touch_premises_cover_reads_expansions_and_arms_at_entry_values`.
//
// `src/script/wire_host.rs` — the read lowering's six
// * `a_whole_file_read_is_two_trips_whatever_the_frontmatter_costs` — the
//   fm-costs-no-round-trips law. Round trips do not exist in-process, so the
//   law's SUBJECT is gone; its two delivered facts survive on the face and are
//   pinned together at `crates/registry/tests/script_op.rs:149`
//   § `a_read_only_program_answers_a_no_effect_trace_at_the_entry_fingerprint`
//   — decoded `fm` off the face, and `words` (**PR 1's NAMED GAP 1, closed by
//   this PR**: the count is now asserted as a number, not merely carried).
// * `a_composition_spanning_two_revisions_refuses_instead_of_being_assembled` —
//   the `file_rev` bracket. Same elimination, one layer stronger: the daemon
//   serves every read of one attempt from ONE pinned entry world, so two
//   observations cannot disagree. Twin: `crates/registry/src/script_op.rs:1465`
//   § `a_foreign_edit_after_entry_is_invisible_to_reads_and_refuses_the_commit`
//   (invisible to the reads, caught at the commit by the touch set). The
//   tombstone in `script_golden_live.rs` § the composed read's bracket says the
//   same thing about this test's live sibling.
// * `the_wall_clock_is_checked_before_every_round_trip_not_once_per_read` — the
//   per-TRIP-not-per-READ distinction. It is a statement ABOUT round trips and
//   dies with them. What survives is that the clock binds at the read site at
//   all: `crates/registry/src/script_op.rs:2032`
//   § `the_wall_clock_binds_at_the_read_builtin`, plus the pre-commit wall at
//   `crates/registry/src/script_op.rs:358`.
// * `the_deadline_refusal_is_pinned_and_claims_nothing_the_host_cannot_see` —
//   the refusal wording, pinned verbatim. The wording was `WireHost`'s own and
//   goes with it; the daemon mints its own text and names its own budget, which
//   § `the_wall_clock_binds_at_the_read_builtin` asserts. **Named gap:** no
//   daemon-side row pins that refusal string VERBATIM the way this one did — it
//   asserts the refusal names the clock, not its exact bytes.
// * `an_armed_run_that_meets_the_clock_answers_a_face_that_does_not_deny_the_arm`
//   — the honesty law: a refusal rendered under an armed row must not deny the
//   arm. The law is the TRACE's, not the host's, and it is pinned doorless at
//   `crates/mrd/tests/script_trace.rs:177`
//   § `a_fault_keeps_its_armed_entries_flagged_not_committed` (armed rows render
//   `[not committed]` under a fault, and the fault's own reason is asserted).
// * `a_runtime_fault_face_is_labelled_and_names_its_line` — exact twin, same
//   three assertions (the `runtime fault at line N — ` opener, the kernel's own
//   message after it, and the ABSENCE of the rules-plane `rule 'script'`
//   framing), doorless, in the same test:
//   `crates/mrd/tests/script_trace.rs:177`
//   § `a_fault_keeps_its_armed_entries_flagged_not_committed`.
//
// **Successor-less rows: one, named** — the verbatim deadline-refusal string
// above. Everything else lands on a daemon-side or trace-side row named here by
// file, line and § name.
