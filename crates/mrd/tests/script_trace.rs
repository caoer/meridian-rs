//! U3 gates — `ScriptTrace` assembly.
//!
//! The load-bearing property is negative: the trace MINTS no commit fact. The
//! commit leg is the §4.4 splice response embedded verbatim, so these gates
//! assert against the raw response BYTES, never against a re-serialization.

use std::collections::BTreeMap;

use effects::{
    ArmedEdit, EvalError, ReadFace, ReadPosition, ReadRecord, ScriptEval, ScriptFacts,
    ScriptRecording, ScriptTelemetry, SecFacts, TocEntry, TocFacts,
};
use mrd::script::{CommitLeg, FaultClass, ScriptOutcome, ScriptTrace};
use serde_json::value::RawValue;
use wire::{HpathSeg, PlanEdit};

/// A splice response whose bytes a re-serialization could not reproduce: keys
/// out of alphabetical order, irregular whitespace, a nested object. If the
/// trace re-typed it, this is what would drift.
const SPLICE_RESPONSE: &str = r#"{"fingerprint_after":"b3:c4e91d02","fingerprint_before":"b3:a90f13c7",  "seq": 118,
  "edits": [{"path":"tasks/0011-token-audit.md","node_rev":"7c40e1a8"}],
  "receipt": {"path":"receipts/2026-08.md","anchor":"^r-000118"},
  "verdicts": []}"#;

fn telemetry() -> ScriptTelemetry {
    ScriptTelemetry {
        fuel_used: 4_211,
        mem_used: 65_536,
        reads_used: 1,
        wall_ms: 12,
    }
}

fn toc_read(line: u32, position: ReadPosition) -> ReadRecord {
    ReadRecord {
        path: "tasks/0011-token-audit.md".to_owned(),
        section: None,
        line,
        position,
        face: ReadFace::Toc(TocFacts {
            rev: "7c40e1a8b2f9d356".to_owned(),
            fm: BTreeMap::from([
                ("owner".to_owned(), String::new()),
                ("status".to_owned(), "todo".to_owned()),
            ]),
            toc: vec![TocEntry {
                section: "Notes".to_owned(),
                anchor: None,
                rev: "3b62f9c8".to_owned(),
            }],
            words: 41,
        }),
    }
}

fn armed_props(line: u32) -> ArmedEdit {
    ArmedEdit {
        path: "tasks/0011-token-audit.md".to_owned(),
        edit: PlanEdit::SetProperty {
            key: "owner".to_owned(),
            value: "8ab41c02".to_owned(),
            rev: None,
        },
        line,
        depth: 0,
    }
}

fn armed_append(line: u32) -> ArmedEdit {
    ArmedEdit {
        path: "tasks/0011-token-audit.md".to_owned(),
        edit: PlanEdit::Append {
            hpath: vec![HpathSeg {
                h: "Close".to_owned(),
                n: None,
            }],
            body: "- done\n".to_owned(),
            rev: None,
        },
        line,
        depth: 1,
    }
}

fn ok_eval(armed: Vec<ArmedEdit>, reads: Vec<ReadRecord>) -> ScriptEval {
    ScriptEval {
        outcome: Ok(ScriptFacts {
            bindings: BTreeMap::from([("card".to_owned(), "<toc>".to_owned())]),
        }),
        armed,
        recording: ScriptRecording {
            actor: "8ab41c02".to_owned(),
            reads,
        },
        telemetry: telemetry(),
    }
}

fn failed_eval(error: EvalError, armed: Vec<ArmedEdit>, reads: Vec<ReadRecord>) -> ScriptEval {
    ScriptEval {
        outcome: Err(error),
        armed,
        recording: ScriptRecording {
            actor: "8ab41c02".to_owned(),
            reads,
        },
        telemetry: telemetry(),
    }
}

fn response() -> Box<RawValue> {
    RawValue::from_string(SPLICE_RESPONSE.to_owned()).expect("the fixture is valid JSON")
}

/// Pull the `commit` member back out of the emitted trace WITHOUT re-parsing it
/// into a typed shape — `RawValue` hands back the exact bytes that were emitted.
fn emitted_commit(json: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Probe {
        commit: Option<Box<RawValue>>,
    }
    let probe: Probe = serde_json::from_str(json).expect("the trace emits valid JSON");
    probe.commit.map(|raw| raw.get().to_owned())
}

/// Gate 1 — the commit leg round-trips byte-identical to the raw splice body.
/// Asserted against the BYTES: a `serde_json::Value` would sort the keys and
/// normalize the whitespace, which is exactly the second commit-fact shape the
/// design forbids.
#[test]
fn the_commit_leg_round_trips_byte_identical_to_the_splice_response() {
    let eval = ok_eval(vec![armed_props(3)], vec![toc_read(1, ReadPosition::Echo)]);
    let trace = ScriptTrace::assemble("b3:a90f13c7", &eval, CommitLeg::Response(response()));

    let json = serde_json::to_string(&trace).expect("the trace serializes");
    let commit = emitted_commit(&json).expect("a committed trace carries its commit leg");

    assert_eq!(
        commit.as_bytes(),
        SPLICE_RESPONSE.as_bytes(),
        "the commit leg must be the splice response byte-for-byte"
    );
    assert_eq!(trace.outcome, ScriptOutcome::Committed);
}

/// Gate 2 — a zero-armed script is read-class: `no_effect`, no commit leg, and
/// telemetry still present.
#[test]
fn a_zero_armed_script_is_no_effect_with_telemetry_and_no_commit() {
    let eval = ok_eval(Vec::new(), vec![toc_read(1, ReadPosition::Echo)]);
    let trace = ScriptTrace::assemble("b3:b84e0d63", &eval, CommitLeg::NotIssued);

    assert_eq!(trace.outcome, ScriptOutcome::NoEffect);
    assert!(trace.commit.is_none());
    assert!(trace.fault.is_none());
    assert_eq!(trace.telemetry.wall_ms, 12);

    let json = serde_json::to_string(&trace).expect("the trace serializes");
    assert!(
        !json.contains("\"commit\""),
        "an absent commit leg is absent from the JSON, never null: {json}"
    );
    assert!(json.contains("\"telemetry\""), "telemetry is unconditional");
}

/// Gate 3 — a fault keeps the arms that landed, flagged not-committed, and
/// carries no commit leg. This is golden scenario 4's face.
#[test]
fn a_fault_keeps_its_armed_entries_flagged_not_committed() {
    let eval = failed_eval(
        EvalError::Runtime {
            rule_id: "script".to_owned(),
            reason: "no frontmatter key \"report_path\"".to_owned(),
            line: Some(4),
        },
        vec![armed_props(2)],
        vec![toc_read(1, ReadPosition::Echo)],
    );
    let trace = ScriptTrace::assemble("b3:02be6d91", &eval, CommitLeg::NotIssued);

    assert_eq!(trace.outcome, ScriptOutcome::Fault);
    assert!(trace.commit.is_none());
    let fault = trace
        .fault
        .as_ref()
        .expect("a fault outcome carries a fault");
    assert_eq!(fault.class, FaultClass::Runtime);
    assert!(fault.reason.contains("report_path"));
    // The harmonized wording (defect-ledger DIV-1): the entry is not a rule,
    // so the face labels the fault class and names the faulting line instead
    // of the rules-plane framing.
    assert_eq!(
        fault.line,
        Some(4),
        "a runtime fault names its faulting line"
    );
    assert!(
        fault.reason.starts_with("runtime fault at line 4 — "),
        "the face opens with the fault label and line: {}",
        fault.reason
    );
    assert!(
        !fault.reason.contains("rule 'script'"),
        "the script entry is not a rule; the rules-plane framing is not this \
         face's: {}",
        fault.reason
    );

    let armed: Vec<_> = trace.armed_entries().collect();
    assert_eq!(armed.len(), 1, "the arm that landed is still traced");
    assert!(
        armed.iter().all(|entry| !entry.committed),
        "nothing committed, so every armed line renders [not committed]"
    );
}

/// A refusal is not a fault — the two must grep apart (r4/F7). The class enum
/// is CLOSED at parse|runtime|budget|refused. The refusal exemplar is a
/// commit-leg refusal (the arm-time `multi_file_write_set` refusal is retired
/// — an armed set spanning files commits as the §4.4 set form).
#[test]
fn a_refusal_greps_apart_from_a_fault() {
    let refused = ok_eval(vec![armed_props(5)], Vec::new());
    let trace = ScriptTrace::assemble(
        "b3:77d20e19",
        &refused,
        CommitLeg::Refused(mrd::script::Refusal::minted(
            wire::Recovery::Fix,
            "expect_armed_mismatch: this run armed a set the caller never pinned",
        )),
    );
    assert_eq!(trace.outcome, ScriptOutcome::Refused);
    let fault = trace.fault.as_ref().expect("a refusal carries its reason");
    assert_eq!(fault.class, FaultClass::Refused);
    assert!(fault.reason.contains("expect_armed_mismatch"));

    let budget = failed_eval(
        EvalError::Budget {
            fuel: 1_000_000,
            mem: 67_108_864,
        },
        Vec::new(),
        Vec::new(),
    );
    let trace = ScriptTrace::assemble("b3:77d20e19", &budget, CommitLeg::NotIssued);
    assert_eq!(trace.outcome, ScriptOutcome::Fault);
    assert_eq!(
        trace.fault.expect("a fault reason").class,
        FaultClass::Budget
    );

    let armed_budget = failed_eval(
        EvalError::ArmedBudget {
            rule_id: "script".to_owned(),
            line: 9,
            limit: 64,
        },
        Vec::new(),
        Vec::new(),
    );
    let trace = ScriptTrace::assemble("b3:77d20e19", &armed_budget, CommitLeg::NotIssued);
    let fault = trace.fault.expect("a ceiling names itself");
    assert_eq!(fault.class, FaultClass::Budget);
    assert_eq!(fault.line, Some(9));
}

/// Gate 4 — serde round-trip is stable and the string `attempts` appears
/// nowhere. `attempts` is a HOST fact stamped on the composed face; finding it
/// here would mean the retry loop leaked into the entry.
#[test]
fn the_trace_round_trips_and_never_says_attempts() {
    let eval = ok_eval(
        vec![armed_props(3), armed_append(4)],
        vec![
            toc_read(1, ReadPosition::Echo),
            ReadRecord {
                path: "tasks/0012-cache-sweep.md".to_owned(),
                section: Some("Close".to_owned()),
                line: 2,
                position: ReadPosition::Quiet,
                face: ReadFace::Section(SecFacts {
                    text: "- one\n".to_owned(),
                    rev: "9e02c5b7".to_owned(),
                }),
            },
        ],
    );
    let trace = ScriptTrace::assemble("b3:a90f13c7", &eval, CommitLeg::Response(response()));

    let json = serde_json::to_string(&trace).expect("the trace serializes");
    assert!(
        !json.contains("attempts"),
        "attempts is a host fact, never a field of the entry's trace: {json}"
    );

    let back: ScriptTrace = serde_json::from_str(&json).expect("the trace round-trips");
    let again = serde_json::to_string(&back).expect("the trace re-serializes");
    assert_eq!(json, again, "serde round-trip is byte-stable");
    assert_eq!(
        emitted_commit(&again).as_deref(),
        Some(SPLICE_RESPONSE),
        "the commit leg survives a full round-trip verbatim"
    );
}

/// The statement-position rule reaches the trace: a top-level-statement read is
/// the `echo` kind, every other position is `read`. Armed entries follow, in arm
/// order, whatever their depth — depth never suppresses.
#[test]
fn echo_and_quiet_reads_carry_distinct_kinds_and_armed_follows_in_arm_order() {
    let eval = ok_eval(
        vec![armed_props(3), armed_append(4)],
        vec![
            toc_read(1, ReadPosition::Echo),
            toc_read(2, ReadPosition::Quiet),
        ],
    );
    let trace = ScriptTrace::assemble("b3:a90f13c7", &eval, CommitLeg::Response(response()));

    let json = serde_json::to_value(&trace).expect("the trace serializes");
    let kinds: Vec<&str> = json["trace"]
        .as_array()
        .expect("trace is an array")
        .iter()
        .map(|entry| entry["kind"].as_str().expect("every entry is kinded"))
        .collect();
    assert_eq!(kinds, ["echo", "read", "armed", "armed"]);

    let armed: Vec<_> = trace.armed_entries().collect();
    assert_eq!(
        armed.iter().map(|entry| entry.line).collect::<Vec<_>>(),
        [3, 4],
        "arm order is execution order"
    );
    assert_eq!(armed[1].depth, 1, "depth is recorded and never suppresses");
    assert!(
        armed.iter().all(|entry| entry.committed),
        "a committed splice zips every armed descriptor to committed"
    );
}

/// The result-echo ruling (2026-08-13, F-S1+F-S3): a successful evaluation's
/// top-level bindings ride the trace as `bindings` — name → Starlark repr —
/// so the face can render the values the run computed. The kernel captured
/// them all along (`ScriptFacts::bindings`); the assembler used to drop them,
/// which is what made learning a value cost a committed write.
#[test]
fn a_successful_evals_bindings_ride_the_trace_and_round_trip() {
    let eval = ok_eval(Vec::new(), vec![toc_read(1, ReadPosition::Echo)]);
    let trace = ScriptTrace::assemble("b3:a90f13c7", &eval, CommitLeg::NotIssued);

    let json = serde_json::to_value(&trace).expect("the trace serializes");
    assert_eq!(
        json["bindings"]["card"], "<toc>",
        "the bindings the eval produced ride the trace: {json}"
    );

    let text = serde_json::to_string(&trace).expect("the trace serializes");
    let back: ScriptTrace = serde_json::from_str(&text).expect("the trace round-trips");
    let again = serde_json::to_string(&back).expect("the trace re-serializes");
    assert_eq!(text, again, "bindings survive the round-trip byte-stable");
}

/// Absence stays absence: a run with nothing to carry emits no `bindings`
/// member at all — never `{}`. Three shapes owe that silence: a script that
/// bound nothing, a FAILED evaluation (its namespace is not a result), and
/// the guard refusal (zero evaluation).
#[test]
fn empty_or_failed_bindings_are_absent_never_an_empty_object() {
    let mut nothing_bound = ok_eval(Vec::new(), Vec::new());
    nothing_bound.outcome = Ok(ScriptFacts {
        bindings: BTreeMap::new(),
    });
    let trace = ScriptTrace::assemble("b3:a90f13c7", &nothing_bound, CommitLeg::NotIssued);
    let json = serde_json::to_string(&trace).expect("the trace serializes");
    assert!(
        !json.contains("\"bindings\""),
        "nothing bound is absence: {json}"
    );

    let failed = failed_eval(
        EvalError::Budget { fuel: 1, mem: 1 },
        Vec::new(),
        Vec::new(),
    );
    let trace = ScriptTrace::assemble("b3:a90f13c7", &failed, CommitLeg::NotIssued);
    let json = serde_json::to_string(&trace).expect("the trace serializes");
    assert!(
        !json.contains("\"bindings\""),
        "a failed eval's namespace is not a result: {json}"
    );

    let refused = ScriptTrace::guard_refused("b3:a90f13c7", "b3:51e7c0d2");
    let json = serde_json::to_string(&refused).expect("the trace serializes");
    assert!(
        !json.contains("\"bindings\""),
        "the guard refusal evaluated nothing: {json}"
    );
}

/// The serialized toc face carries `words` under exactly that key: the face
/// renderer lives in another repo and decodes this JSON, so the key IS the
/// cross-repo seam. A renamed or dropped key renders `words:0` on a live face
/// while the goldens render the true count, and both sides still compile.
#[test]
fn the_serialized_toc_face_publishes_the_word_count_under_words() {
    let eval = ok_eval(Vec::new(), vec![toc_read(1, ReadPosition::Echo)]);
    let trace = ScriptTrace::assemble("b3:a90f13c7", &eval, CommitLeg::NotIssued);

    let json = serde_json::to_value(&trace).expect("the trace serializes");
    assert_eq!(
        json["trace"][0]["face"]["Toc"]["words"], 41,
        "the toc face publishes `words`: {}",
        json["trace"][0]
    );
}

/// A conflict embeds the §5.1 mismatch verbatim too — one commit-fact shape, so
/// `fingerprint_mismatch{expected, actual, changed}` needs no re-typing either.
#[test]
fn a_conflict_embeds_the_mismatch_verbatim_and_commits_nothing() {
    const MISMATCH: &str = r#"{"error":"fingerprint_mismatch","expected":"b3:51e7c0d2","actual":"b3:88aa1f47","changed":["tasks/0011-token-audit.md"],"recovery":"resync"}"#;
    let eval = ok_eval(vec![armed_props(3)], vec![toc_read(1, ReadPosition::Echo)]);
    let trace = ScriptTrace::assemble(
        "b3:51e7c0d2",
        &eval,
        CommitLeg::Conflict(RawValue::from_string(MISMATCH.to_owned()).expect("valid JSON")),
    );

    assert_eq!(trace.outcome, ScriptOutcome::Conflict);
    assert!(trace.fault.is_none(), "a conflict is not a fault");
    let json = serde_json::to_string(&trace).expect("the trace serializes");
    assert_eq!(emitted_commit(&json).as_deref(), Some(MISMATCH));
    assert!(
        trace.armed_entries().all(|entry| !entry.committed),
        "a refused commit leaves every arm not-committed"
    );
}
