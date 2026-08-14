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
//!
//! ⭐ AND THE SWEEP'S POPULATION IS ANSWERABLE TO THE CODE. A sweep that walks a
//! hand-maintained list covers only the doors someone remembered; the census at
//! the bottom derives `commit()`'s arms from its own source and refuses any arm
//! that is neither swept nor recorded unreachable with the law that keeps it so.
//! A gate that cannot notice a missing door is the defect it was built to remove.

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
if card["fm"]["owner"] == "":
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
    /// `ok: false` carrying a §5.1 `fingerprint_mismatch` — the world moved
    /// under the guard. It speaks as a CONFLICT, not as a fault.
    ConflictOnMismatch,
    /// `ok: false` with a readable §8 error body that is not the world moving.
    RefusedWithErrorBody,
    /// `ok: false` with no error body — a refusal that will not say why.
    RefusedWithNoErrorBody,
}

/// What SPEAKING means for a door.
///
/// The law is one law — a premise-holding commit door does not exit silently —
/// but the doors do not all speak the same way, and a single predicate that only
/// knew `fault` would have to drop the conflict door from the population to stay
/// green. Dropping a door to keep a sweep green is the defect this file exists to
/// remove, so the door DECLARES its speech and the sweep applies it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Speech {
    /// A `fault` states why the commit failed.
    Fault,
    /// `outcome: conflict` carrying the daemon's own mismatch body. There is no
    /// fault: a moved world is an answer, not a failure to answer.
    Conflict,
}

impl OnSplice {
    /// Every premise-holding commit FAILURE door reachable through this seam.
    ///
    /// This list is no longer trusted to be the population: the census in
    /// `every_commit_door_is_either_swept_or_recorded_unreachable` derives the
    /// arms from `commit()`'s own source and refuses a door that is not here.
    const ALL: [Self; 6] = [
        Self::NeverAnswers,
        Self::Unparseable,
        Self::OkWithNoBody,
        Self::ConflictOnMismatch,
        Self::RefusedWithErrorBody,
        Self::RefusedWithNoErrorBody,
    ];

    /// How this door speaks — see [`Speech`].
    fn speech(self) -> Speech {
        match self {
            Self::ConflictOnMismatch => Speech::Conflict,
            Self::NeverAnswers
            | Self::Unparseable
            | Self::OkWithNoBody
            | Self::RefusedWithErrorBody
            | Self::RefusedWithNoErrorBody => Speech::Fault,
        }
    }

    /// Does this door KNOW what happened to the workspace?
    ///
    /// Three of the six do not: the request was issued and its fate is not
    /// readable from here. The three `ok: false` doors DO — the daemon declined,
    /// so nothing landed; they differ only in how much they say about why.
    fn is_indeterminate(self) -> bool {
        match self {
            Self::NeverAnswers | Self::Unparseable | Self::OkWithNoBody => true,
            Self::ConflictOnMismatch
            | Self::RefusedWithErrorBody
            | Self::RefusedWithNoErrorBody => false,
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
    /// The last `splice` request's bytes, verbatim — what the set-form pin
    /// asserts its shape on.
    last_splice: Option<Value>,
}

impl Fake {
    fn breaking(on_splice: OnSplice) -> Self {
        Self {
            on_splice,
            splice_issued: false,
            last_splice: None,
        }
    }
}

impl Door for Fake {
    fn call(&mut self, request: &Value) -> io::Result<String> {
        let op = request["op"].as_str().expect("every request names an op");
        if op == "splice" {
            self.splice_issued = true;
            self.last_splice = Some(request.clone());
            return match self.on_splice {
                OnSplice::NeverAnswers => Err(io::Error::other("connection reset by peer")),
                OnSplice::Unparseable => Ok("<html>502 Bad Gateway</html>".to_owned()),
                OnSplice::OkWithNoBody => Ok(json!({"ok": true}).to_string()),
                OnSplice::ConflictOnMismatch => Ok(json!({"ok": false, "error": {
                    "code": "fingerprint_mismatch",
                    "expected": ENTRY,
                    "actual": "b3:00ff11ee22dd33cc44bb55aa66997788",
                }})
                .to_string()),
                // No `recovery` field: the class comes from the §8 frozen table's
                // binding for the code, which is the precedence the engine states.
                OnSplice::RefusedWithErrorBody => Ok(json!({"ok": false, "error": {
                    "code": "would_corrupt",
                    "message": "the heading identity does not survive the reparse",
                }})
                .to_string()),
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
/// this door produced a trace, and that trace speaks about the commit in the way
/// the door declares.
fn speaks(door: &mut dyn Door, flags: &[&str], speech: Speech) -> Result<(), String> {
    let trace = run(door, flags)?;
    match speech {
        Speech::Fault => {
            if trace.fault.is_none() {
                return Err(
                    "a trace with no fault says nothing about why the commit failed".to_owned(),
                );
            }
        }
        Speech::Conflict => {
            if trace.outcome != ScriptOutcome::Conflict {
                return Err(format!(
                    "a moved world must arrive as `conflict`, not {:?}",
                    trace.outcome
                ));
            }
            if trace.commit.is_none() {
                return Err(
                    "a conflict with no commit leg drops the daemon's own mismatch body, which \
                     is the whole answer here"
                        .to_owned(),
                );
            }
        }
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

/// A `fingerprint_mismatch` is the world moving, and it is an ANSWER: the daemon
/// read the guard, declined, and said exactly what changed. It speaks as
/// `conflict` with the daemon's own body embedded — no fault, because nothing
/// failed to be understood.
///
/// It is in the sweep because the census puts it there: it is one of `commit()`'s
/// arms, so a change that made it silent would be caught by the same law as the
/// rest.
#[test]
fn a_moved_world_speaks_as_a_conflict_carrying_the_daemons_own_body() {
    let mut door = Fake::breaking(OnSplice::ConflictOnMismatch);
    let trace = run(&mut door, &["--actor", "8ab41c02"]).expect("the door SPEAKS");

    assert_eq!(trace.outcome, ScriptOutcome::Conflict);
    assert!(
        !trace.commit_unknown,
        "the daemon answered the guard, so the workspace is known: nothing landed"
    );
    let commit = trace
        .commit
        .as_ref()
        .expect("the mismatch body rides the leg");
    let body: Value = serde_json::from_str(commit.get()).expect("the leg is the daemon's bytes");
    assert_eq!(body["code"], "fingerprint_mismatch");
    assert!(
        body.get("changed").is_none(),
        "`changed` is STRUCK (§18 row 2, ruled 2026-08-10) — nothing mints it, \
         and the extras that DO exist ride verbatim: {body}"
    );
    assert!(
        trace.fault.is_none(),
        "a moved world is not a fault — a consumer greps the two apart"
    );
}

/// A readable §8 refusal that is not the world moving: the engine's own wording
/// rides into the reason, and the class comes from the frozen table's binding for
/// the code, because this frame carried no `recovery` of its own.
#[test]
fn a_refusal_with_a_readable_error_body_carries_its_code_and_the_tables_class() {
    let mut door = Fake::breaking(OnSplice::RefusedWithErrorBody);
    let trace = run(&mut door, &["--actor", "8ab41c02"]).expect("the door SPEAKS");

    assert_eq!(trace.outcome, ScriptOutcome::Refused);
    assert!(
        !trace.commit_unknown,
        "the daemon refused with a readable body, so nothing landed and the run knows it"
    );
    let fault = trace.fault.expect("a controlled exit says why");
    assert_eq!(fault.class, FaultClass::Refused);
    assert_eq!(fault.code.as_deref(), Some("would_corrupt"));
    assert_eq!(
        fault.recovery,
        Some(Recovery::Fix),
        "no `recovery` on the frame, so the §8 table's binding for the code decides — \
         one source read a second way, never a second table"
    );
    assert!(
        fault.reason.contains("does not survive the reparse"),
        "the daemon's own wording travels verbatim: {}",
        fault.reason
    );
}

// ── the law, and the control that makes it a measurement ─────────────────────

/// **THE LAW.** No premise-holding commit door exits without speaking.
///
/// It walks every door in `OnSplice::ALL` rather than trusting the tests above to
/// stay in step with the code. `ALL` is a maintained list, so on its own it makes
/// coverage true only for doors someone remembered; the census below is what
/// makes the list answerable to `commit()`'s real arms.
#[test]
fn no_premise_holding_commit_door_exits_silently() {
    for door in OnSplice::ALL {
        let mut fake = Fake::breaking(door);
        speaks(&mut fake, &["--actor", "8ab41c02"], door.speech())
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
    let outcome = speaks(&mut door, &["--actor", "8ab41c02"], Speech::Fault);

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

// ── the census: the population, DERIVED from the code it claims to cover ─────

/// `commit()`'s own source. The sweep above walks a hand-maintained list, and a
/// hand-maintained list is a literal wearing a derived costume: a door added to
/// `commit()` and never wired into `OnSplice` is invisible to it, the suite stays
/// green, and nothing says the coverage shrank.
///
/// Reading the source is the cheap instrument that makes the list ANSWERABLE.
/// The exhaustiveness cannot be a compile-time fact here without the door
/// taxonomy living in the engine's own types, and it must not: three of these
/// arms build the SAME `CommitLeg::Unknown` variant, so a variant-exhaustive
/// match would certify a population it cannot see — the defect one level up.
/// Precedent for reading a sibling module's source in a test:
/// `crates/mrd/tests/rules_cli.rs` § `the_cli_layer_holds_no_second_resolver`.
const COMMIT_SOURCE: &str = include_str!("../src/script/cmd.rs");

/// What the census says about one arm of `commit()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Coverage {
    /// A failure door the sweep drives through the `Door` seam.
    Swept(OnSplice),
    /// Not a failure exit: the daemon answered and the leg carries its answer.
    Success,
}

/// Every `CommitLeg` construction inside `fn commit`, in source order, as
/// `(1-based line, variant)`.
///
/// Comment lines are dropped before matching, so prose naming a leg can neither
/// satisfy nor break a claim about code — the same discipline the rules-CLI
/// structural test uses.
fn commit_arms() -> Vec<(usize, String)> {
    let lines: Vec<(usize, &str)> = COMMIT_SOURCE
        .lines()
        .enumerate()
        .map(|(i, l)| (i + 1, l))
        .collect();
    let start = lines
        .iter()
        .position(|(_, line)| line.starts_with("fn commit("))
        .expect("`fn commit` is the function this census is about");
    let end = lines[start..]
        .iter()
        .position(|(_, line)| *line == "}")
        .expect("the function closes at column zero")
        + start;

    let mut arms = Vec::new();
    for (number, line) in &lines[start..=end] {
        if line.trim_start().starts_with("//") {
            continue;
        }
        let mut rest = *line;
        while let Some(at) = rest.find("CommitLeg::") {
            rest = &rest[at + "CommitLeg::".len()..];
            let variant: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            arms.push((*number, variant));
        }
    }
    arms
}

/// ⭐ **THE POPULATION GATE.** Every arm of `commit()` is either swept by the law
/// above or recorded here as unreachable WITH the law that makes it so.
///
/// This is the gate the sweep could not be: the sweep asks "does each door I know
/// about speak?", and this asks "do I know about every door?". A door added to
/// `commit()` and not wired in fails HERE, loudly, naming its line — instead of
/// passing silently as a coverage gap nobody minted a signal for.
#[test]
fn every_commit_door_is_either_swept_or_recorded_unreachable() {
    /// The census, in `commit()`'s own source order. Adding a door to `commit()`
    /// means adding its row here and either wiring it into `OnSplice` or stating
    /// the law that keeps it unreachable.
    const CENSUS: [(&str, Coverage); 8] = [
        ("Unknown", Coverage::Swept(OnSplice::NeverAnswers)),
        ("Unknown", Coverage::Swept(OnSplice::Unparseable)),
        // `dry` — a rehearsal that ran everything except disk.
        ("Rehearsal", Coverage::Success),
        ("Response", Coverage::Success),
        ("Unknown", Coverage::Swept(OnSplice::OkWithNoBody)),
        ("Conflict", Coverage::Swept(OnSplice::ConflictOnMismatch)),
        ("Refused", Coverage::Swept(OnSplice::RefusedWithErrorBody)),
        ("Refused", Coverage::Swept(OnSplice::RefusedWithNoErrorBody)),
    ];

    let arms = commit_arms();
    let rendered = arms
        .iter()
        .map(|(line, variant)| format!("  cmd.rs:{line} CommitLeg::{variant}"))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        arms.len(),
        CENSUS.len(),
        "`commit()` has {} arms and the census names {}. A door added to `commit()` is \
         invisible to the sweep until it is wired into `OnSplice` (or recorded here with the \
         law that keeps it unreachable). The arms as read from source:\n{rendered}",
        arms.len(),
        CENSUS.len(),
    );
    for (index, ((line, variant), (expected, _))) in arms.iter().zip(CENSUS).enumerate() {
        assert_eq!(
            variant, expected,
            "census row {index} says `CommitLeg::{expected}` and cmd.rs:{line} builds \
             `CommitLeg::{variant}` — the population moved under the sweep"
        );
    }

    // And the other direction: no door sits in `OnSplice` without a census row,
    // so the enum cannot drift away from the code either.
    let mut swept: Vec<OnSplice> = CENSUS
        .iter()
        .filter_map(|(_, coverage)| match coverage {
            Coverage::Swept(door) => Some(*door),
            _ => None,
        })
        .collect();
    for door in OnSplice::ALL {
        let at = swept
            .iter()
            .position(|candidate| *candidate == door)
            .unwrap_or_else(|| {
                panic!(
                    "{door:?} is in the sweep but names no arm of `commit()` — it tests a \
                        door the engine no longer has"
                )
            });
        swept.remove(at);
    }
    assert!(
        swept.is_empty(),
        "the census claims these doors are swept and `OnSplice::ALL` does not walk them: \
         {swept:?}"
    );
}

/// **THE SET-FORM LOWERING PIN** (replaces the retired door-367 pin: the
/// arm-time `multi_file_write_set` law and `commit()`'s single-path door both
/// retired with the §4.4 set form — run-plane.md § One COMMIT per attempt).
///
/// A two-file armed set issues exactly ONE splice, and that splice is the SET
/// form: `files[]` carrying every armed path's plan group in first-arm order,
/// under the entry guard — never two splices, never a bare `path` form.
#[test]
fn a_multi_file_armed_set_issues_one_set_splice() {
    const TWO_FILES: &str = r#"
card = read("tasks/0011-token-audit.md")
put("tasks/0011-token-audit.md", props={"owner": me()})
put("tasks/0012-second-file.md", props={"owner": me()})
"#;

    let mut door = Fake::breaking(OnSplice::NeverAnswers);
    let argv = ["--actor".to_owned(), "8ab41c02".to_owned()];
    let trace = attempt(&argv, TWO_FILES, &mut door).expect("a lost answer still SPEAKS");

    assert!(
        door.splice_issued,
        "a two-file armed set commits — one set splice goes out"
    );
    let request = door.last_splice.expect("the door recorded the request");
    assert!(
        request.get("path").is_none(),
        "the set form carries no top-level `path`: {request}"
    );
    let files = request["files"]
        .as_array()
        .expect("the set form carries `files[]`");
    assert_eq!(files.len(), 2, "one member per armed content path");
    assert_eq!(files[0]["path"], "tasks/0011-token-audit.md");
    assert_eq!(files[1]["path"], "tasks/0012-second-file.md");
    assert!(
        files
            .iter()
            .all(|f| f["plan_edits"].as_array().is_some_and(|e| e.len() == 1)),
        "each member carries its own plan group: {request}"
    );
    assert!(
        request.get("if_fingerprint").is_some(),
        "the set commit rides the entry guard"
    );
    // The daemon never answered, and the request DID go out — the leg is the
    // same indeterminacy speech as the single form's (a lost answer renders
    // the refused class and says the outcome is NOT KNOWN).
    assert_eq!(trace.outcome, ScriptOutcome::Refused);
    assert!(
        trace.commit_unknown,
        "a lost set answer carries the commit-unknown flag, like the single form"
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
