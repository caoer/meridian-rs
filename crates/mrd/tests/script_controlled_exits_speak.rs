//! **No controlled failure exit of `mrd script` is silent past the point where
//! the trace has a premise** (`docs/run-plane.md` § A controlled failure exit
//! SPEAKS).
//!
//! The property under test is not "these five messages are right". It is that a
//! consumer reading a nonzero exit can tell a deliberate refusal from a process
//! killed mid-write — and it cannot, if the engine's own deliberate refusals
//! arrive as an absent trace, because that is exactly what a killed process
//! leaves behind. The remedies are opposite: fix your request, versus never
//! resend this one.
//!
//! Every test drives the verb through its `Door` seam, so what the engine SAYS on
//! a broken commit is assertable without breaking a real daemon.
//!
//! ⭐ THE SWEEP AT THE BOTTOM CARRIES ITS OWN POSITIVE CONTROL. A test that
//! walks the premise-holding doors and finds all of them speaking proves nothing
//! unless the same walk is shown going RED on a door that does not speak. The
//! control arm walks the doors BEFORE the premise exists — where the contract
//! deliberately keeps them silent — and asserts the sweep's own predicate fails
//! there. Without it this file would certify rather than check.

use std::io;

use mrd::script::cmd::attempt;
use mrd::script::{Door, FaultClass, ScriptOutcome, ScriptTrace};
use serde_json::{Value, json};
use wire::Recovery;

/// The entry fingerprint the fake daemon reports (§4.7).
const ENTRY: &str = "b3:a90f13c7ba0e1d4f5c6b7a8990112233445566778899aabbccddeeff00112233";

/// The card the script reads and claims.
const CARD: &str = "tasks/0011-token-audit.md";

/// A script that ARMS something, so the run reaches the commit. A read-class
/// script would never issue a splice and could not exercise a commit door.
const CLAIM: &str = r#"
card = read("tasks/0011-token-audit.md")
if card.fm["owner"] == "":
    put("tasks/0011-token-audit.md", props={"owner": me(), "status": "doing"})
"#;

/// How the fake daemon behaves when the armed splice arrives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OnSplice {
    /// The call itself fails: the request went out, no answer came back.
    NeverAnswers,
    /// Bytes that are not a frame at all.
    Unparseable,
    /// `ok: true` with no body — success asserted, nothing to describe it with.
    OkWithNoBody,
    /// `ok: false` with no error body — a refusal that will not say why.
    RefusedWithNoErrorBody,
}

impl OnSplice {
    /// Every premise-holding commit door reachable through this seam, in one
    /// place, so the sweep below cannot silently stop covering one.
    ///
    /// The fifth door — an armed set writing more than one content path — is
    /// unreachable here by construction: the arm-time single-write-file law
    /// refuses it before any splice is issued, which is why the sweep names four.
    const ALL: [Self; 4] = [
        Self::NeverAnswers,
        Self::Unparseable,
        Self::OkWithNoBody,
        Self::RefusedWithNoErrorBody,
    ];

    /// Does this door KNOW what happened to the workspace?
    ///
    /// Three of the four do not: the request was issued and its fate is not
    /// readable from here. The refusal DOES — `ok: false` means the daemon
    /// declined, so nothing landed; it simply did not say why.
    fn is_indeterminate(self) -> bool {
        match self {
            Self::NeverAnswers | Self::Unparseable | Self::OkWithNoBody => true,
            Self::RefusedWithNoErrorBody => false,
        }
    }
}

/// A fake daemon that answers the read ops honestly and breaks exactly one way at
/// the commit.
struct Fake {
    on_splice: OnSplice,
    /// Set when a `splice` request was actually put on the door — the fact the
    /// indeterminacy claim rests on.
    splice_issued: bool,
}

impl Fake {
    fn breaking(on_splice: OnSplice) -> Self {
        Self {
            on_splice,
            splice_issued: false,
        }
    }
}

impl Door for Fake {
    fn call(&mut self, request: &Value) -> io::Result<String> {
        let op = request["op"].as_str().expect("every request names an op");
        if op == "splice" {
            self.splice_issued = true;
            return match self.on_splice {
                OnSplice::NeverAnswers => Err(io::Error::other("connection reset by peer")),
                OnSplice::Unparseable => Ok("<html>502 Bad Gateway</html>".to_owned()),
                OnSplice::OkWithNoBody => Ok(json!({"ok": true}).to_string()),
                OnSplice::RefusedWithNoErrorBody => Ok(json!({"ok": false}).to_string()),
            };
        }
        Ok(match op {
            "fingerprint" => format!(r#"{{"ok":true,"body":{{"fingerprint":"{ENTRY}","seq":2}}}}"#),
            "toc" => json!({"ok": true, "body": {
                "path": CARD,
                "file_rev": "7c40e1a8b2f9d356",
                "fingerprint": ENTRY,
                "nodes": [
                    {"kind": "frontmatter", "span": [0, 32], "node_rev": "26796ebec5d0bf1a",
                     "text_prefix_16b": "---\nowner:\n", "keys": ["owner", "status"]},
                ],
            }})
            .to_string(),
            "read" => json!({"ok": true, "body": {
                "path": CARD,
                "file_rev": "7c40e1a8b2f9d356",
                "root": ENTRY,
                "words_total": 41,
                "toc": [],
                "anchors": [],
                "rendered_text": "",
                "props": [
                    {"key": "owner", "value": "", "span": [4, 11], "prop_rev": "33d5b0e1"},
                    {"key": "status", "value": "todo", "span": [12, 25], "prop_rev": "41f643f0"},
                ],
            }})
            .to_string(),
            "cat" => json!({"ok": true, "body": {
                "span": [4, 12], "node_rev": "33d5b0e1b27cb48b", "content": "owner:\n"
            }})
            .to_string(),
            other => panic!("the script entry asked for an op it must not know: {other}"),
        })
    }
}

/// A daemon that breaks BEFORE the premise exists: `fingerprint` refuses, so no
/// entry fingerprint is ever minted.
///
/// This is the control arm's door. The contract deliberately keeps this path
/// silent — a trace's first field is the premise, and synthesizing one would mint
/// a fact — so it is the honest negative the sweep is measured against.
struct NoPremise;

impl Door for NoPremise {
    fn call(&mut self, request: &Value) -> io::Result<String> {
        let op = request["op"].as_str().expect("every request names an op");
        assert_eq!(op, "fingerprint", "the run must not get past the premise");
        Ok(json!({"ok": false, "error": {"code": "bad_request"}}).to_string())
    }
}

/// Run the claim script with `flags` against `door`, returning whatever the verb
/// gives back — a trace, or the diagnostic of an exit that said nothing.
fn run(door: &mut dyn Door, flags: &[&str]) -> Result<ScriptTrace, String> {
    let argv: Vec<String> = flags.iter().map(|flag| (*flag).to_owned()).collect();
    attempt(&argv, CLAIM, door)
}

/// The sweep's predicate, named once so the control arm can apply the SAME one:
/// this door produced a trace, and that trace speaks about the commit.
fn speaks(door: &mut dyn Door, flags: &[&str]) -> Result<(), String> {
    let trace = run(door, flags)?;
    if trace.fault.is_none() {
        return Err("a trace with no fault says nothing about why the commit failed".to_owned());
    }
    Ok(())
}

// ── the four premise-holding doors, one at a time ─────────────────────────────

/// The load-bearing door, and the one the consumer's own source names: the splice
/// went out and no answer came. The engine does not know whether it landed, and
/// `resync` is the class that says "re-read, never resend" to a machine.
#[test]
fn a_splice_whose_answer_never_came_declares_resync_and_an_unknown_commit() {
    let mut door = Fake::breaking(OnSplice::NeverAnswers);
    let trace =
        run(&mut door, &["--actor", "8ab41c02"]).expect("the door SPEAKS, it does not exit");

    assert!(door.splice_issued, "the premise of the whole claim");
    assert_eq!(trace.outcome, ScriptOutcome::Refused);
    assert!(
        trace.commit_unknown,
        "the splice was issued and never answered for, so the trace must not let \
         `refused` be read as `nothing was applied`"
    );
    let fault = trace.fault.expect("a controlled exit says why");
    assert_eq!(fault.class, FaultClass::Refused);
    assert_eq!(fault.recovery, Some(Recovery::Resync));
    assert_eq!(
        fault.code, None,
        "no frame minted a §8 code, and inventing one puts a value on the wire \
         surface no daemon can answer with"
    );
    assert!(
        fault.reason.contains("UNKNOWN"),
        "the operator-facing half states it too: {}",
        fault.reason
    );
}

/// The same door under `--dry`. A rehearsal runs everything except disk, so it
/// provably committed nothing — declaring `resync` would tell a caller their file
/// might have changed when it could not. The consumer's killed-engine face splits
/// on exactly this, and the engine must not disagree with it.
#[test]
fn a_dry_run_that_lost_its_answer_declares_retry_because_a_rehearsal_writes_nothing() {
    let mut door = Fake::breaking(OnSplice::NeverAnswers);
    let trace = run(&mut door, &["--actor", "8ab41c02", "--dry"]).expect("the door SPEAKS");

    let fault = trace.fault.expect("a controlled exit says why");
    assert_eq!(
        fault.recovery,
        Some(Recovery::Retry),
        "a dry run's lost answer is retry, never resync"
    );
    assert!(
        fault.reason.contains("DRY"),
        "and it says which fact makes the class safe: {}",
        fault.reason
    );
}

/// An answer that is not a frame is the same indeterminacy as no answer: the
/// daemon may have applied the splice before replying with something unreadable.
#[test]
fn an_unparseable_answer_is_the_same_indeterminacy_as_no_answer() {
    let mut door = Fake::breaking(OnSplice::Unparseable);
    let trace = run(&mut door, &["--actor", "8ab41c02"]).expect("the door SPEAKS");

    assert_eq!(trace.outcome, ScriptOutcome::Refused);
    assert!(trace.commit_unknown);
    assert_eq!(
        trace.fault.expect("says why").recovery,
        Some(Recovery::Resync)
    );
}

/// `ok: true` with no body: the daemon asserts success and hands nothing to carry
/// as the commit fact. The trace cannot embed a leg it never received, so the
/// honest answer is the workspace, not this trace.
#[test]
fn an_ok_frame_with_no_body_carries_no_commit_fact_so_it_declares_unknown() {
    let mut door = Fake::breaking(OnSplice::OkWithNoBody);
    let trace = run(&mut door, &["--actor", "8ab41c02"]).expect("the door SPEAKS");

    assert_eq!(trace.outcome, ScriptOutcome::Refused);
    assert!(trace.commit_unknown);
    assert!(
        trace.commit.is_none(),
        "there were no bytes to embed — absence, never a synthesized leg"
    );
}

/// `ok: false` with no error body is DETERMINATE: the daemon declined, so nothing
/// landed. It gets a plain refusal — `commit_unknown` would be a false alarm — and
/// the class is `respawn`, because a frame that violates §8's own shape is a
/// broken channel rather than a request the caller can repair.
#[test]
fn a_refusal_with_no_error_body_states_that_nothing_landed_and_blames_the_channel() {
    let mut door = Fake::breaking(OnSplice::RefusedWithNoErrorBody);
    let trace = run(&mut door, &["--actor", "8ab41c02"]).expect("the door SPEAKS");

    assert_eq!(trace.outcome, ScriptOutcome::Refused);
    assert!(
        !trace.commit_unknown,
        "the daemon REFUSED, so nothing landed — claiming indeterminacy here would \
         send a caller to re-read a file that provably did not change"
    );
    assert_eq!(
        trace.fault.expect("says why").recovery,
        Some(Recovery::Respawn)
    );
}

// ── the law, and the control that makes it a measurement ─────────────────────

/// **THE LAW.** No premise-holding commit door exits without speaking.
///
/// It walks every door in `OnSplice::ALL` rather than trusting the four tests
/// above to stay in step with the code: a door added to `commit()` and wired into
/// this enum is covered by construction, and one added without it is the gap this
/// sweep cannot see — named in the deliverable as such.
#[test]
fn no_premise_holding_commit_door_exits_silently() {
    for door in OnSplice::ALL {
        let mut fake = Fake::breaking(door);
        speaks(&mut fake, &["--actor", "8ab41c02"])
            .unwrap_or_else(|why| panic!("{door:?} exited without speaking: {why}"));
        assert!(fake.splice_issued, "{door:?} never reached the commit");

        // The indeterminacy marker is not decoration: it is the only thing that
        // stops `refused` from being read as "nothing was applied".
        let mut fake = Fake::breaking(door);
        let trace = run(&mut fake, &["--actor", "8ab41c02"]).expect("speaks");
        assert_eq!(
            trace.commit_unknown,
            door.is_indeterminate(),
            "{door:?} must declare exactly what it knows about the workspace"
        );
    }
}

/// ⭐ **THE POSITIVE CONTROL for the sweep above.** The same predicate, applied
/// to a door the contract deliberately keeps SILENT, must go red.
///
/// A path that fails before the entry fingerprint exists has no premise to state,
/// and a synthesized premise would mint a fact. So it exits through
/// `mrd::run` — exit 2, empty stdout — and `speaks()` fails on it. That failure
/// is what proves the sweep is capable of failing at all: without this arm,
/// `no_premise_holding_commit_door_exits_silently` would pass just as happily
/// against an engine that emitted a trace unconditionally, or against a predicate
/// that asserted nothing.
///
/// It also pins the OTHER half of the absence contract. This door's silence is
/// the contract's guarantee — nothing armed, nothing sent, the workspace
/// unchanged — and the guarantee is only true because the doors above stopped
/// sharing this exit with it.
#[test]
fn the_sweeps_predicate_goes_red_on_a_path_that_deliberately_stays_silent() {
    let mut door = NoPremise;
    let outcome = speaks(&mut door, &["--actor", "8ab41c02"]);

    let why = outcome.expect_err(
        "a pre-premise failure must NOT produce a trace — if it does, either the \
         engine is synthesizing a premise it does not have, or this control has \
         stopped being able to fail",
    );
    assert!(
        why.contains("fingerprint"),
        "and it names the door it died at, on stderr, for an operator: {why}"
    );
}

// ── additive migration, proved rather than asserted ──────────────────────────

/// A trace minted before this change carries no `commit_unknown`. It must still
/// deserialize, and the missing field must read as `false` — not as a decode
/// error, and not as an alarm.
#[test]
fn a_pre_change_trace_json_still_deserializes_and_reads_as_a_known_commit() {
    const PRE_CHANGE: &str = r#"{
      "entry_fingerprint": "b3:a90f13c7",
      "outcome": "refused",
      "trace": [],
      "fault": {"class": "refused", "reason": "foreign_edit: someone else wrote it"},
      "telemetry": {"fuel_used": 12, "mem_used": 34, "reads_used": 1, "wall_ms": 5}
    }"#;

    let trace: ScriptTrace = serde_json::from_str(PRE_CHANGE).expect("the old shape still decodes");
    assert_eq!(trace.outcome, ScriptOutcome::Refused);
    assert!(
        !trace.commit_unknown,
        "an absent marker is a KNOWN commit — a trace written before the field \
         existed described a determinate refusal, and must not start reading as \
         an alarm"
    );
}

/// And the field is absent from the wire on every ordinary trace, so a consumer
/// reading a determinate run sees byte-identical output to before.
#[test]
fn the_marker_is_absent_from_the_json_of_a_determinate_refusal() {
    let mut door = Fake::breaking(OnSplice::RefusedWithNoErrorBody);
    let trace = run(&mut door, &["--actor", "8ab41c02"]).expect("the door SPEAKS");

    let json = serde_json::to_string(&trace).expect("the trace serializes");
    assert!(
        !json.contains("commit_unknown"),
        "a marker present on every trace is noise a consumer must read past: {json}"
    );
}
