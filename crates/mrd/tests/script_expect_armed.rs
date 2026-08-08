//! `mrd script --expect-armed` — the armed-set expectation
//! (`docs/run-plane.md` § Sub-amendment (the armed-set expectation,
//! `--expect-armed`)).
//!
//! The arm/commit split gates the ARM's rows and then runs a SECOND child to
//! commit. The amendment argues the two children arm identically and then relies
//! on the argument; this flag turns it into a per-call measurement.
//!
//! **The negative proof is the deliverable, and it has a specific shape.** A
//! mismatch must refuse BEFORE the splice is issued — detection after landing is
//! not refusal. That claim is unfalsifiable against a live daemon, which can only
//! show what changed and never what was said, so these tests drive the verb
//! through its `Door` seam and assert on the **socket census**: the ops actually
//! put on the wire. "No `splice` frame exists in the census" is the assertion
//! that means pre-splice; anything weaker would pass on a run that spliced and
//! then complained.

use std::io;

use mrd::script::cmd::attempt;
use mrd::script::{ArmedRow, Door, ScriptOutcome};
use serde_json::{Value, json};
use wire::PlanEdit;

/// The entry fingerprint the fake daemon reports (§4.7).
const ENTRY: &str = "b3:a90f13c7ba0e1d4f5c6b7a8990112233445566778899aabbccddeeff00112233";

/// The one page the fake daemon serves.
const CARD: &str = "tasks/0011-token-audit.md";

/// A SECOND page, byte-identical in everything the fake answers — same revs,
/// same frontmatter, same section map. Only the address differs, which is what
/// makes it the control for the target dimension: a digest that comes out
/// different for these two runs can only have read the path.
const OTHER_CARD: &str = "tasks/0012-token-audit.md";

/// Read the card, claim it if nobody holds it — golden scenario 1's shape.
const CLAIM: &str = r#"
card = read("tasks/0011-token-audit.md")
if card.fm["owner"] == "":
    put("tasks/0011-token-audit.md", props={"owner": me(), "status": "doing"})
"#;

/// A well-formed digest that is not the one this script produces. Well-formed
/// on purpose, **domain tag included**: a mismatch must be refused for being the
/// WRONG SET, never for being an unparseable string or an untagged one, and
/// either would let this suite pass for the wrong reason.
const FOREIGN: &str =
    "armed-set-path-edit:sha256:0000000000000000000000000000000000000000000000000000000000000000";

/// A fake daemon behind the one door, recording every op it is asked for — the
/// census these tests are built on.
struct Fake {
    ops: Vec<String>,
    requests: Vec<Value>,
    /// The page this daemon answers for. Every other fact it serves is fixed, so
    /// two `Fake`s differing only here differ only in the target.
    card: String,
}

impl Fake {
    fn new() -> Self {
        Self {
            ops: Vec::new(),
            requests: Vec::new(),
            card: CARD.to_owned(),
        }
    }

    /// The same daemon answering for a different page.
    fn serving(card: &str) -> Self {
        Self {
            card: card.to_owned(),
            ..Self::new()
        }
    }

    fn asked(&self, op: &str) -> bool {
        self.ops.iter().any(|seen| seen == op)
    }

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
                "path": &self.card,
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
            "read" => json!({"ok": true, "body": {
                "path": &self.card,
                "file_rev": "7c40e1a8b2f9d356",
                "root": ENTRY,
                "words_total": 41,
                "toc": [],
                "anchors": [],
                "rendered_text": "",
                // § A.6-decoded frontmatter, as the real composed read serves it.
                "props": [
                    {"key": "owner", "value": "", "span": [4, 11], "prop_rev": "33d5b0e1"},
                    {"key": "status", "value": "todo", "span": [12, 25], "prop_rev": "41f643f0"},
                ],
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
            "splice" => format!(
                r#"{{"ok":true,"body":{{"armed":{{"edits":2}},"fingerprint_before":"{ENTRY}","fingerprint_after":"b3:c4e91d02","seq":3,"verdicts":[]}}}}"#
            ),
            other => panic!("the script entry asked for an op it must not know: {other}"),
        })
    }
}

fn claim(door: &mut Fake, flags: &[&str]) -> mrd::script::ScriptTrace {
    let argv: Vec<String> = flags.iter().map(|flag| (*flag).to_owned()).collect();
    attempt(&argv, CLAIM, door).expect("the attempt runs")
}

/// The digest the ARM leg publishes — what a host copies into the commit child.
fn arm_digest() -> String {
    let mut door = Fake::new();
    let trace = claim(&mut door, &["--actor", "8ab41c02", "--dry"]);
    assert_eq!(
        trace.outcome,
        ScriptOutcome::NoEffect,
        "the arm is a rehearsal: nothing lands"
    );
    trace
        .armed_digest
        .expect("an arm that armed rows publishes their digest")
}

// ── the courier property ─────────────────────────────────────────────────────

/// **The property that makes the host a courier and not a second
/// implementation.** The arm and the commit are separate processes evaluating
/// the same source against the same responses; they must publish the same
/// digest, or the host has nothing it can forward.
///
/// This is also what rules out the failure the sub-amendment names: if the two
/// legs disagreed, every gated call would refuse, and the natural "fix" is a
/// host-side recomputation — a second canonicalization, whose agreement with the
/// engine would then be luck.
#[test]
fn the_arm_and_the_commit_publish_the_same_digest() {
    let mut door = Fake::new();
    let commit = claim(&mut door, &["--actor", "8ab41c02"]);
    assert_eq!(commit.outcome, ScriptOutcome::Committed);
    assert_eq!(
        commit.armed_digest.as_deref(),
        Some(arm_digest().as_str()),
        "the arm leg and the commit leg hash the same armed set"
    );
}

/// The digest describes the rows the commit actually sends — **and the file it
/// sends them to.** Asserted against the REQUEST rather than against the trace,
/// because the trace is what this feature publishes and the request is what the
/// daemon acts on; a digest that described the trace while the request carried
/// something else would be exactly the vacuous pass this flag exists to prevent.
///
/// The reconstruction goes through the wire's own `path` and `plan_edits[]`,
/// which is the whole point of P3-1's fix: `plan_edits[]` alone cannot say where
/// the bytes land, so a digest reproducible from it alone was blind to the
/// target.
#[test]
fn the_published_digest_is_over_the_path_and_plan_edits_the_commit_sends() {
    let mut door = Fake::new();
    let trace = claim(&mut door, &["--actor", "8ab41c02"]);

    let splice = door.request("splice");
    let path = splice["path"].as_str().expect("the splice names its target");
    let sent: Vec<PlanEdit> =
        serde_json::from_value(splice["plan_edits"].clone()).expect("plan_edits is an array");
    assert!(!sent.is_empty(), "the run really did arm something");

    let rows: Vec<ArmedRow<'_>> = sent.iter().map(|edit| ArmedRow { edit, path }).collect();
    assert_eq!(
        trace.armed_digest,
        Some(mrd::script::armed_digest(&rows)),
        "hashing the wire's own path + plan_edits[] reproduces the published digest"
    );
}

// ── the capability layer: the domain tag ─────────────────────────────────────

/// **The tag rides the live trace**, which is what makes it assertable by a host
/// that parses nothing. The value a host copies into `--expect-armed` is this
/// string, tag included.
#[test]
fn the_published_digest_carries_the_domain_tag() {
    let published = arm_digest();
    assert!(
        published.starts_with(mrd::script::DOMAIN_TAG),
        "a host asserts this literal prefix and nothing else: {published}"
    );
    assert!(
        !published.starts_with("sha256:"),
        "and the pre-tag spelling must not be reachable, or the assertion admits an \
         engine whose digest is blind to the target: {published}"
    );
}

/// **The wire-observable capability probe.** A caller with nothing but the entry
/// and a door can establish that THIS engine's digest covers the target: run the
/// same edits at two different paths and compare the published digests. This is
/// the observation itself, not a version constant standing in for it.
///
/// It is also P3-1's acceptance at entry level rather than unit level — the
/// defect lived in the trace assembly, so the proof belongs where a host reads.
#[test]
fn identical_edits_to_two_targets_publish_different_digests() {
    let here = {
        let mut door = Fake::new();
        claim(&mut door, &["--actor", "8ab41c02", "--dry"])
            .armed_digest
            .expect("rows were armed")
    };
    let there = {
        let mut door = Fake::serving(OTHER_CARD);
        attempt(
            &["--actor".to_owned(), "8ab41c02".to_owned(), "--dry".to_owned()],
            &CLAIM.replace(CARD, OTHER_CARD),
            &mut door,
        )
        .expect("the attempt runs")
        .armed_digest
        .expect("rows were armed")
    };
    assert_ne!(
        here, there,
        "the same edits at two different targets must publish two digests — equal here \
         means the host can gate one file and the commit child land in another"
    );
}

/// The ADMIT half of the same probe: the target is the ONLY thing that moved
/// above. Two runs of the same script against the same target publish one
/// digest, so an ordinary arm/commit pair still agrees.
#[test]
fn the_same_target_publishes_one_digest_across_runs() {
    assert_eq!(arm_digest(), arm_digest(), "the courier property, live");
}

// ── acceptance 1: a matching digest commits ──────────────────────────────────

/// **THE ADMIT ARM FOR THE WHOLE CARD, end to end on the tagged engine.** Arm,
/// forward the published string verbatim the way a host does, commit — and the
/// bytes go out.
///
/// It exists because both refusal arms of this change are green in the failure
/// it guards against. Widening the domain, or adding the tag where the digest is
/// PUBLISHED but not where it is RECOMPUTED for comparison, makes every ordinary
/// commit false-refuse while an old engine is still rejected and two targets
/// still hash apart. A gate with only refusal arms cannot tell "correctly
/// strict" from "entirely dead", so the census here asserts the splice was
/// ISSUED — the mirror of the negative proof's assertion that it was not.
#[test]
fn an_ordinary_commit_still_commits_on_the_tagged_engine() {
    let expected = arm_digest();
    assert!(
        expected.starts_with(mrd::script::DOMAIN_TAG),
        "the value forwarded is the tagged one — otherwise this proves nothing about \
         the tag: {expected}"
    );

    let mut door = Fake::new();
    let trace = claim(
        &mut door,
        &["--actor", "8ab41c02", "--expect-armed", &expected],
    );

    assert_eq!(
        trace.outcome,
        ScriptOutcome::Committed,
        "a tagged pin from this engine's own arm must COMMIT, not refuse: {:?}",
        trace.fault
    );
    assert!(
        door.asked("splice"),
        "census: the splice was issued — census: {:?}",
        door.ops
    );
    assert!(
        !door.request("splice")["plan_edits"]
            .as_array()
            .expect("an array")
            .is_empty(),
        "and it carried the armed rows"
    );
}

/// A commit child handed the digest its own arm produced proceeds exactly as it
/// would without the flag.
#[test]
fn a_matching_digest_commits_normally() {
    let expected = arm_digest();
    let mut door = Fake::new();
    let trace = claim(
        &mut door,
        &["--actor", "8ab41c02", "--expect-armed", &expected],
    );

    assert_eq!(trace.outcome, ScriptOutcome::Committed, "{:?}", trace.fault);
    assert!(door.asked("splice"), "census: {:?}", door.ops);
    assert_eq!(door.request("splice")["if_fingerprint"], json!(ENTRY));
}

// ── acceptance 2: the negative proof ─────────────────────────────────────────

/// **The load-bearing test.** A planted mismatch refuses, and the refusal is
/// PRE-SPLICE: the census carries the read ops and NO `splice` frame at all.
///
/// The assertion is deliberately on the absence of the op rather than on the
/// outcome word. An implementation that spliced and then reported `refused`
/// would satisfy every other assertion in this file while landing bytes — which
/// is the difference between refusing a write and noticing one.
#[test]
fn a_planted_mismatch_refuses_before_the_splice_is_issued() {
    let mut door = Fake::new();
    let trace = claim(
        &mut door,
        &["--actor", "8ab41c02", "--expect-armed", FOREIGN],
    );

    assert_eq!(trace.outcome, ScriptOutcome::Refused, "{:?}", trace.fault);
    assert!(
        !door.asked("splice"),
        "a mismatch must refuse BEFORE the splice is issued — census: {:?}",
        door.ops
    );

    // The census is not vacuous: this run really did reach the point where a
    // commit was the next thing to do. Without these, a run that faulted at
    // parse time would pass the assertion above for the wrong reason.
    assert!(
        door.asked("fingerprint") && door.asked("toc"),
        "{:?}",
        door.ops
    );
    assert!(
        trace.armed_digest.is_some(),
        "rows were armed — the refusal is about WHICH rows, not about arming none"
    );

    // The refusal names both values, so an operator can see which set was
    // authorized and which one ran.
    let reason = &trace.fault.expect("a refusal carries its reason").reason;
    assert!(reason.contains("expect_armed_mismatch"), "{reason}");
    assert!(
        reason.contains(FOREIGN),
        "names what the caller pinned: {reason}"
    );
    assert!(
        reason.contains(trace.armed_digest.as_deref().unwrap_or_default()),
        "names what this run armed: {reason}"
    );
}

/// A refusal is `refused`, never `conflict`. The two grep apart by law (r4/F7),
/// and a mismatched armed set is not the world moving — a host that retried it
/// as a conflict would re-run a script whose set it had already declined.
#[test]
fn a_mismatch_is_a_refusal_not_a_conflict() {
    let mut door = Fake::new();
    let trace = claim(
        &mut door,
        &["--actor", "8ab41c02", "--expect-armed", FOREIGN],
    );

    assert_eq!(trace.outcome, ScriptOutcome::Refused);
    assert_ne!(trace.outcome, ScriptOutcome::Conflict);
    assert_eq!(
        trace.fault.expect("a refusal carries a fault").class,
        mrd::script::FaultClass::Refused
    );
    assert!(
        trace.commit.is_none(),
        "no splice was issued, so there is no commit leg to embed"
    );
}

// ── acceptance 5: the receipt exclusion ──────────────────────────────────────

/// **The collision note, pinned.** The receipt rides `request.receipt`, never
/// `eval.armed`, so it is outside the digest by construction — the same armed
/// set hashes identically with and without `--receipt`.
///
/// This is a fact worth a test rather than a sentence, because it is exactly the
/// kind of thing a reader assumes the other way: a host that believed
/// `--expect-armed` covered the receipt would think a write was gated that this
/// flag never sees. The receipt is gated on its own pre-spawn path.
#[test]
fn the_receipt_is_outside_the_armed_digest() {
    let bare = {
        let mut door = Fake::new();
        claim(&mut door, &["--actor", "8ab41c02"]).armed_digest
    };

    let mut door = Fake::new();
    let with_receipt = claim(
        &mut door,
        &[
            "--actor",
            "8ab41c02",
            "--receipt",
            "receipts/2026-08-07.md#r-000118",
        ],
    );

    assert_eq!(
        with_receipt.armed_digest, bare,
        "the receipt is not an armed row, so it moves no digest"
    );
    // And it really did ride the request — otherwise the equality above would be
    // true because the receipt went nowhere.
    assert_eq!(
        door.request("splice")["receipt"],
        json!({"path": "receipts/2026-08-07.md", "anchor": "r-000118"}),
        "the receipt rides the splice as its own field, beside plan_edits[]"
    );
    assert!(
        door.request("splice")["plan_edits"]
            .as_array()
            .expect("an array")
            .iter()
            .all(|row| row.get("receipt").is_none()),
        "and never as a member of plan_edits[]"
    );
}

// ── the flag is optional ─────────────────────────────────────────────────────

/// Absent the flag the entry behaves exactly as before — an operator running
/// `mrd script` directly is unaffected, which is what keeps this a consumer-plane
/// addition rather than a new law on the CLI entry.
#[test]
fn the_entry_is_unchanged_when_the_flag_is_absent() {
    let mut door = Fake::new();
    let trace = claim(&mut door, &["--actor", "8ab41c02"]);
    assert_eq!(trace.outcome, ScriptOutcome::Committed);
    assert!(door.asked("splice"));
}

/// Zero wire delta: the guarded run speaks the same closed op set as the
/// unguarded one, and the `--expect-armed` value appears in NO request. It is a
/// consumer-plane comparison; a host that saw it on the wire would be looking at
/// a schema change.
#[test]
fn the_flag_puts_nothing_on_the_wire() {
    const DECLARED: [&str; 6] = ["hello", "fingerprint", "toc", "cat", "read", "splice"];

    let expected = arm_digest();
    let mut door = Fake::new();
    claim(
        &mut door,
        &["--actor", "8ab41c02", "--expect-armed", &expected],
    );

    for op in &door.ops {
        assert!(
            DECLARED.contains(&op.as_str()),
            "`{op}` is not on the wire — census: {:?}",
            door.ops
        );
    }
    for request in &door.requests {
        let text = request.to_string();
        assert!(
            !text.contains(&expected) && !text.contains("expect_armed"),
            "the expectation is consumer-plane and never crosses the wire: {text}"
        );
    }
}

/// `--expect-armed` without a value is a bad invocation, not a silently ignored
/// flag. A guard that disappears when its argument is forgotten is worse than no
/// guard: the call proceeds ungated and nothing says so.
#[test]
fn the_flag_demands_its_value() {
    let mut door = Fake::new();
    let argv = vec!["--expect-armed".to_owned()];
    let error = attempt(&argv, CLAIM, &mut door).expect_err("a bad invocation");
    assert!(error.contains("--expect-armed"), "{error}");
    assert!(
        door.ops.is_empty(),
        "a bad invocation touches no socket: {:?}",
        door.ops
    );
}
