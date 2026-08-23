//! `mrd script --expect-armed` — the armed-set expectation
//! (`docs/run-plane.md` § Sub-amendment (the armed-set expectation,
//! `--expect-armed`)).
//!
//! The arm/commit split gates the ARM's rows and then runs a SECOND child to
//! commit. The amendment argues the two children arm identically and then relies
//! on the argument; this flag turns it into a per-call measurement.
//!
//! ⭐ **THE SUBJECT SPLIT IN TWO** (card
//! `script-door-commit-premise-world-grain-vs-touch-set`). `mrd script` is now
//! ONE lane — the whole attempt is the wire `script` op — so the digest is
//! computed and the gate is applied by the DAEMON. What stays here is exactly
//! the two halves that are still this side's:
//!
//! * **the digest function itself**, which is pure and shared
//!   (`effects::digest::armed_digest`, re-exported as `mrd::script::armed_digest`
//!   — one definition, reached by both lanes). Its properties are asserted
//!   directly, with no daemon and no door: a function is not better tested
//!   through a socket.
//! * **the courier property**, which is what a host actually depends on: this
//!   lane carries the published digest out and the caller's pinned digest in,
//!   both byte for byte, canonicalizing nothing. A lane that re-derived either
//!   value would be a second spelling of the serialization, and two spellings
//!   agree only by luck — the vacuous-pass shape the sub-amendment exists to
//!   prevent.
//!
//! The gate's own negative proof — a mismatch refuses BEFORE anything is issued
//! — is the daemon's, and § Retired against named twins names the test that owns
//! it.

use std::io;

use mrd::script::cmd::attempt;
use mrd::script::{ArmedRow, Door, ScriptOutcome, ScriptTrace};
use serde_json::{Value, json};
use wire::{HpathSeg, PlanEdit};

/// The entry fingerprint the fake daemon reports inside its trace (§4.7).
const ENTRY: &str = "b3:a90f13c7ba0e1d4f5c6b7a8990112233445566778899aabbccddeeff00112233";

/// The one page the fake daemon serves.
const CARD: &str = "tasks/0011-token-audit.md";

/// A SECOND page. Only the address differs, which is what makes it the control
/// for the target dimension: a digest that comes out different for two otherwise
/// identical row sets can only have read the path.
const OTHER_CARD: &str = "tasks/0012-token-audit.md";

/// Read the card, claim it if nobody holds it — golden scenario 1's shape.
const CLAIM: &str = r#"
card = read("tasks/0011-token-audit.md")
if card["fm"]["owner"] == "":
    put("tasks/0011-token-audit.md", props={"owner": me(), "status": "doing"})
"#;

/// A well-formed digest that is not the one the fixture publishes. Well-formed
/// on purpose, **domain tag included**: a mismatch must be refused for being the
/// WRONG SET, never for being an unparseable string or an untagged one, and
/// either would let this suite pass for the wrong reason.
const FOREIGN: &str =
    "armed-set-path-edit:sha256:0000000000000000000000000000000000000000000000000000000000000000";

// ── the digest function: pure, shared, and asserted without a socket ─────────

/// The two rows the claim script arms, as the wire spells them.
fn claim_rows() -> Vec<PlanEdit> {
    vec![
        PlanEdit::SetProperty {
            key: "owner".to_owned(),
            value: "8ab41c02".to_owned(),
            rev: Some("7c40e1a8b2f9d356".to_owned()),
        },
        PlanEdit::SetProperty {
            key: "status".to_owned(),
            value: "doing".to_owned(),
            rev: Some("7c40e1a8b2f9d356".to_owned()),
        },
    ]
}

/// The digest of `rows` armed at `path`.
fn digest_at(path: &str, rows: &[PlanEdit]) -> String {
    let rows: Vec<ArmedRow<'_>> = rows.iter().map(|edit| ArmedRow { edit, path }).collect();
    mrd::script::armed_digest(&rows)
}

/// **The tag is a capability assertion a host reads by literal prefix**, with no
/// parsing: it tells an engine that hashes `(path, edit)` pairs from one that
/// hashed payloads alone. The value a host copies into `--expect-armed` is this
/// string, tag included.
#[test]
fn the_published_digest_carries_the_domain_tag() {
    let published = digest_at(CARD, &claim_rows());
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

/// **P3-1's acceptance.** The same edits at two different targets publish two
/// digests — equal here would mean a host can gate one file while the commit
/// child lands in another.
#[test]
fn identical_edits_to_two_targets_publish_different_digests() {
    assert_ne!(
        digest_at(CARD, &claim_rows()),
        digest_at(OTHER_CARD, &claim_rows()),
        "the digest covers the TARGET, not the payload alone"
    );
}

/// The ADMIT half of the same probe: the target is the ONLY thing that moved
/// above. The same rows at the same target hash the same way every time, so an
/// ordinary arm/commit pair still agrees — a digest that did not would refuse
/// every gated call, and the natural "fix" for that is a host-side
/// recomputation, which is the second canonicalization this design forbids.
#[test]
fn the_same_target_publishes_one_digest_across_runs() {
    assert_eq!(
        digest_at(CARD, &claim_rows()),
        digest_at(CARD, &claim_rows()),
        "the courier property, at the function that has to hold it"
    );
}

/// The digest describes the armed rows and their target, and NOTHING else. A
/// row's own grain moves it — an `append` into `Goals` is not a `set_property`
/// on the same file — which is what makes "the set that was authorized" mean the
/// set rather than the file.
#[test]
fn a_different_armed_grain_at_one_target_publishes_a_different_digest() {
    let append = vec![PlanEdit::Append {
        hpath: vec![HpathSeg {
            h: "Goals".to_owned(),
            n: None,
        }],
        body: "- done\n".to_owned(),
        rev: Some("a6665baff294bd04".to_owned()),
    }];
    assert_ne!(
        digest_at(CARD, &claim_rows()),
        digest_at(CARD, &append),
        "two different armed sets at one target must not hash alike"
    );
}

// ── the courier property, at the boundary this lane still owns ───────────────

/// A whole `ScriptTrace` as the daemon serves it. `forward()` deserializes the
/// body as one, so a fake answering `script` produces a complete trace.
fn trace_body(outcome: &str, digest: Option<&str>, fault: Option<Value>) -> Value {
    let mut body = json!({
        "entry_fingerprint": ENTRY,
        "outcome": outcome,
        "trace": [],
        "telemetry": {"fuel_used": 812, "mem_used": 4096, "reads_used": 1, "wall_ms": 3},
    });
    if let Some(digest) = digest {
        body["armed_digest"] = json!(digest);
    }
    if let Some(fault) = fault {
        body["fault"] = fault;
    }
    if outcome == "committed" {
        body["commit"] = json!({"armed": {"edits": 2}, "fingerprint_before": ENTRY,
                                "fingerprint_after": "b3:c4e91d02", "seq": 3, "verdicts": []});
    }
    body
}

/// A fake daemon behind the one door, recording every op and every request — the
/// census these tests are built on.
struct Fake {
    ops: Vec<String>,
    requests: Vec<Value>,
    body: Value,
}

impl Fake {
    fn new() -> Self {
        Self {
            ops: Vec::new(),
            requests: Vec::new(),
            body: trace_body("committed", Some(&digest_at(CARD, &claim_rows())), None),
        }
    }

    fn answering(mut self, body: Value) -> Self {
        self.body = body;
        self
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
        assert_eq!(
            op, "script",
            "ONE LANE: the gate is applied daemon-side, so the flag rides the one \
             `script` op and this verb spends no other trip: {request}"
        );
        Ok(json!({"id": null, "ok": true, "body": self.body.clone()}).to_string())
    }
}

fn claim(door: &mut Fake, flags: &[&str]) -> ScriptTrace {
    let argv: Vec<String> = flags.iter().map(|flag| (*flag).to_owned()).collect();
    attempt(&argv, CLAIM, door).expect("the attempt runs")
}

/// ⭐ **THE FLIP. This test used to be named `the_flag_puts_nothing_on_the_wire`,
/// and it asserted the opposite of what is now true.**
///
/// It was right while the CLI held its own transaction: the flag was gated HERE,
/// so a host that saw `expect_armed` on the socket was looking at a schema
/// change. With one lane the gate is the daemon's
/// (`registry::script_op::serve` — after rev threading, before anything is
/// issued), so the value MUST cross, and a lane that swallowed it would leave
/// every gated call ungated with nothing to say so.
///
/// **Zero wire delta survives the flip intact**, and that is the assertion that
/// replaces the old one: `expect_armed` is a field the §A.7 `script` op already
/// declares. No new op, no new request shape — the op census is still the closed
/// two-name set.
#[test]
fn the_flag_rides_the_wire_as_the_script_ops_own_declared_field() {
    const DECLARED: [&str; 2] = ["hello", "script"];

    let expected = digest_at(CARD, &claim_rows());
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
    assert_eq!(
        door.request("script")["expect_armed"],
        json!(expected),
        "the caller's pinned digest crosses verbatim — canonicalized nowhere on \
         the way"
    );
}

/// Absent the flag, absent the field. The pair is the whole property: a lane
/// that always sent something would gate a set the caller never authorized, and
/// one that never sent it would gate nothing.
#[test]
fn the_entry_sends_no_expectation_when_the_caller_pinned_none() {
    let mut door = Fake::new();
    let trace = claim(&mut door, &["--actor", "8ab41c02"]);

    assert!(
        door.request("script").get("expect_armed").is_none(),
        "an operator running `mrd script` directly is unaffected: {:?}",
        door.requests
    );
    assert_eq!(trace.outcome, ScriptOutcome::Committed);
}

/// **The courier property, measured at this boundary.** The digest a host copies
/// is the one the engine published, byte for byte — this lane carries the string
/// out of the trace and never re-derives it. A lane that recomputed would be the
/// second canonicalization the sub-amendment forbids, and the fixture proves the
/// difference: the daemon's published value here is the digest of the rows, so a
/// re-derivation over an EMPTY armed block (this trace carries none) would come
/// out different and this row would go red.
#[test]
fn the_published_digest_is_carried_out_of_the_trace_never_re_derived() {
    let published = digest_at(CARD, &claim_rows());
    let mut door = Fake::new();
    let trace = claim(&mut door, &["--actor", "8ab41c02"]);

    assert_eq!(
        trace.armed_digest.as_deref(),
        Some(published.as_str()),
        "the value a host forwards is the engine's own string"
    );
}

/// A planted mismatch: the daemon refuses, and this lane carries the refusal
/// whole — both values named, so an operator can see which set was authorized
/// and which one ran. The PRE-SPLICE position of that refusal is the daemon's
/// to prove (§ Retired against named twins); what this side owes is to deliver
/// the caller's pin unaltered and the answer unflattened.
#[test]
fn a_planted_mismatch_is_delivered_pinned_and_carried_back_whole() {
    let armed = digest_at(CARD, &claim_rows());
    let reason = format!(
        "expect_armed_mismatch: this run armed {armed}, the caller pinned {FOREIGN}. \
         The armed set is not the one that was authorized, so NO splice was issued — \
         nothing was sent, nothing landed, no fingerprint advanced. re-arm: run the arm \
         leg again and gate the set it publishes"
    );
    let mut door = Fake::new().answering(trace_body(
        "refused",
        Some(&armed),
        Some(json!({"class": "refused", "recovery": "fix", "reason": reason})),
    ));
    let trace = claim(
        &mut door,
        &["--actor", "8ab41c02", "--expect-armed", FOREIGN],
    );

    assert_eq!(
        door.request("script")["expect_armed"],
        json!(FOREIGN),
        "the pin the caller typed is what the gate saw"
    );
    assert_eq!(trace.outcome, ScriptOutcome::Refused, "{:?}", trace.fault);
    assert_ne!(
        trace.outcome,
        ScriptOutcome::Conflict,
        "a mismatched armed set is not the world moving — a host that retried it \
         as a conflict would re-run a script whose set it had already declined"
    );
    assert!(
        trace.commit.is_none(),
        "no splice was issued, so there is no commit leg to embed"
    );
    let carried = &trace.fault.expect("a refusal carries its reason").reason;
    assert!(carried.contains("expect_armed_mismatch"), "{carried}");
    assert!(carried.contains(FOREIGN), "names what the caller pinned");
    assert!(carried.contains(&armed), "names what this run armed");
}

/// **The collision note, pinned.** The receipt rides `request.receipt`, never
/// the armed rows, so it is outside the digest by construction — and by
/// construction is measurable: the digest is a function of `(path, edit)` pairs
/// alone, and no receipt is representable as one. This row pins the other half,
/// which is this lane's: the receipt reaches the wire as its OWN field, beside
/// the program, and never inside `expect_armed`.
///
/// It is a fact worth a test rather than a sentence, because it is exactly the
/// kind of thing a reader assumes the other way: a host that believed
/// `--expect-armed` covered the receipt would think a write was gated that this
/// flag never sees.
#[test]
fn the_receipt_rides_its_own_field_and_is_outside_the_armed_digest() {
    let expected = digest_at(CARD, &claim_rows());
    let mut door = Fake::new();
    claim(
        &mut door,
        &[
            "--actor",
            "8ab41c02",
            "--expect-armed",
            &expected,
            "--receipt",
            "receipts/2026-08-07.md#r-000118",
        ],
    );

    let sent = door.request("script");
    assert_eq!(
        sent["receipt"],
        json!({"path": "receipts/2026-08-07.md", "anchor": "r-000118"}),
        "the receipt rides as its own field: {sent}"
    );
    assert_eq!(
        sent["expect_armed"],
        json!(expected),
        "and the expectation is unchanged by its presence — the receipt moves no \
         digest: {sent}"
    );
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

// ── Retired against named twins ──────────────────────────────────────────────
//
// These rows asserted the GATE, which is the daemon's now. The negative proof
// they carried is not weakened — it is asserted where the gate lives, against a
// real daemon. Line numbers are `grep -n` at the head this PR branches from.
//
// * `a_planted_mismatch_refuses_before_the_splice_is_issued` — the load-bearing
//   negative proof, whose assertion was the socket census ("no `splice` frame
//   exists"). Twin: `crates/registry/tests/script_op.rs:540`
//   § `an_expect_armed_mismatch_refuses_pre_splice`, which runs a real daemon
//   over a real corpus and asserts the refusal AND that the workspace is
//   unchanged — a stronger statement than the census, because it measures the
//   disk rather than the dialogue.
// * `a_mismatch_is_a_refusal_not_a_conflict` — folded into
//   § `a_planted_mismatch_is_delivered_pinned_and_carried_back_whole` above,
//   which asserts both directions on the carried trace.
// * `the_arm_and_the_commit_publish_the_same_digest`,
//   `the_published_digest_is_over_the_path_and_plan_edits_the_commit_sends` —
//   two processes evaluating one source publish one digest. Both legs are the
//   daemon's evaluation now, and the property is the function's:
//   § `the_same_target_publishes_one_digest_across_runs` and
//   § `identical_edits_to_two_targets_publish_different_digests` assert it
//   directly, with no socket in the way. The wire-shape half — that the digest
//   is over what the COMMIT sends — is the daemon's: twin
//   `crates/registry/tests/script_op.rs:204`
//   § `an_armed_target_reads_back_its_own_armed_content_and_commits_once`.
// * `an_ordinary_commit_still_commits_on_the_tagged_engine`,
//   `a_matching_digest_commits_normally`,
//   `the_entry_is_unchanged_when_the_flag_is_absent` — the ADMIT arm, which
//   existed because a gate with only refusal arms cannot tell "correctly strict"
//   from "entirely dead". It is kept on this side in the form this side can
//   still hold (§ `the_entry_sends_no_expectation_when_the_caller_pinned_none`
//   and § `the_flag_rides_the_wire_as_the_script_ops_own_declared_field`, which
//   both end `committed`); the end-to-end admit is the daemon's, twin
//   `crates/registry/tests/script_op.rs:204` as above.
