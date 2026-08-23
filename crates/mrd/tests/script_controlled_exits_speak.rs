//! **No controlled failure exit of `mrd script` is silent about what it does
//! not know** (`docs/run-plane.md` § A controlled failure exit SPEAKS).
//!
//! The property under test is not "these five messages are right". It is that a
//! consumer reading a nonzero exit can tell a deliberate refusal from a process
//! killed mid-write — and it cannot, if the engine's own deliberate refusals
//! arrive saying nothing about whether bytes landed, because that is exactly
//! what a killed process leaves behind. The remedies are opposite: fix your
//! request, versus never resend this one.
//!
//! ⭐ **THE SUBJECT MOVED, AND SO DID THE LAW'S REACH.** Until card
//! `script-door-commit-premise-world-grain-vs-touch-set`, this file swept
//! `commit()` — the local transaction's own splice doors — and every one of them
//! held a PREMISE: the CLI had minted the entry fingerprint itself, so it could
//! always speak a trace. `mrd script` is now ONE lane: the whole attempt is the
//! wire `script` op, the entry fingerprint is the DAEMON's, and it reaches this
//! process only inside a trace that arrived. So the doors this file sweeps are
//! `forward()`'s, and the premise is no longer this engine's to state.
//!
//! **The recorded exception, stated rather than papered over.** On the exits
//! where no trace arrived, this lane CANNOT speak in band: a `ScriptTrace`'s
//! first field is the premise, and synthesizing one would mint a fact — the very
//! thing [`NoPremise`]'s control arm exists to forbid. Those exits carry the
//! indeterminacy in PROSE instead, and the prose is what this suite pins: the
//! daemon did not answer, a commit MAY have landed daemon-side, verify with a
//! fresh read before retrying. An honest "premise unknown" spelling in
//! `ScriptTrace` is a trace-contract change, carded as
//! `script-trace-premise-unknown-spelling`. **Until it lands, THE PROSE IS THE
//! OPERATING SURFACE.**
//!
//! Every test drives the verb through its `Door` seam, so what the engine SAYS
//! on a broken answer is assertable without breaking a real daemon. The seven
//! injected pathologies of the pre-card suite all survive — they land on the
//! `script` response instead of the `splice` response.
//!
//! ⭐ AND THE SWEEP'S POPULATION IS ANSWERABLE TO THE CODE. A sweep that walks a
//! hand-maintained list covers only the doors someone remembered; the census at
//! the bottom derives `forward()`'s exits from its own source and refuses any
//! exit that is neither swept nor recorded unreachable with the law that keeps
//! it so. A gate that cannot notice a missing door is the defect it was built to
//! remove.

use std::io;

use mrd::script::cmd::attempt;
use mrd::script::{Door, ScriptOutcome, ScriptTrace};
use serde_json::{Value, json};

/// The entry fingerprint the fake daemon reports inside its trace (§4.7). It is
/// the DAEMON's value now: this engine never mints one.
const ENTRY: &str = "b3:a90f13c7ba0e1d4f5c6b7a8990112233445566778899aabbccddeeff00112233";

/// The card the script reads and claims.
const CARD: &str = "tasks/0011-token-audit.md";

/// A script that ARMS something. Under one lane the source never runs in this
/// process — the daemon evaluates it — but the request still carries it, and a
/// read-class script would be a different fixture than the one these doors are
/// about.
const CLAIM: &str = r#"
card = read("tasks/0011-token-audit.md")
if card["fm"]["owner"] == "":
    put("tasks/0011-token-audit.md", props={"owner": me(), "status": "doing"})
"#;

/// A whole `ScriptTrace` as the daemon serves it — `forward()` deserializes the
/// body as one, so a fake answering `script` must produce a complete trace, not
/// a fragment.
fn committed_trace() -> Value {
    json!({
        "entry_fingerprint": ENTRY,
        "outcome": "committed",
        "trace": [],
        "commit": {"armed": {"edits": 1}, "fingerprint_before": ENTRY,
                   "fingerprint_after": "b3:c4e91d02", "seq": 3, "verdicts": []},
        "telemetry": {"fuel_used": 812, "mem_used": 4096, "reads_used": 1, "wall_ms": 3},
    })
}

/// The same shape, ending as the daemon's own `conflict` — the touch-set guard
/// refusing with the mismatch extras in band.
fn conflict_trace() -> Value {
    json!({
        "entry_fingerprint": ENTRY,
        "outcome": "conflict",
        "trace": [],
        "commit": {"code": "fingerprint_mismatch", "recovery": "resync",
                   "expected": ENTRY, "actual": "b3:00ff11ee22dd33cc",
                   "scope": CARD},
        "telemetry": {"fuel_used": 812, "mem_used": 4096, "reads_used": 1, "wall_ms": 3},
    })
}

/// The same shape, ending `refused` with a determinate fault — used to pin that
/// the indeterminacy marker stays absent from an ordinary trace.
fn refused_trace() -> Value {
    json!({
        "entry_fingerprint": ENTRY,
        "outcome": "refused",
        "trace": [],
        "fault": {"class": "refused", "code": "would_corrupt", "recovery": "fix",
                  "reason": "would_corrupt: the heading identity does not survive the reparse"},
        "telemetry": {"fuel_used": 812, "mem_used": 4096, "reads_used": 1, "wall_ms": 3},
    })
}

/// How the fake daemon answers the one `script` op the whole attempt now is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OnScript {
    /// The call itself fails: the request went out, no answer came back.
    NeverAnswers,
    /// Bytes that are not a frame at all.
    Unparseable,
    /// `ok: true` with no body — success asserted, nothing to describe it with.
    OkWithNoBody,
    /// `ok: true` with a body that is not a `ScriptTrace` this build can read —
    /// engine and CLI disagreeing on the trace shape.
    UnreadableTrace,
    /// `ok: false` carrying a readable §8 error body. **The daemon's `script`
    /// door emits this ONLY from before the entry exists** (see [`NoPremise`]),
    /// which is what makes it the determinate arm.
    RefusedBeforeEntry,
    /// `ok: false` with no error body — a refusal that will not say why.
    RefusedWithNoErrorBody,
    /// `ok: true` with a whole readable trace — the ONE arm that produces a
    /// trace, and the census's positive control against a suite of failures.
    ATrace,
}

/// What SPEAKING means for an exit of `forward()`.
///
/// The law is one law — an exit past the send point does not leave a consumer
/// unable to tell a refusal from a killed process — but the exits do not all
/// know the same thing, and a single predicate that only knew "prose mentioning
/// doubt" would have to drop the determinate door from the population to stay
/// green. Dropping a door to keep a sweep green is the defect this file exists
/// to remove, so the door DECLARES what it knows and the sweep applies it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Knows {
    /// The answer never arrived, or arrived unreadable. A commit MAY have
    /// landed daemon-side and this engine cannot tell — so it says so.
    Unknown,
    /// The daemon said itself that nothing was attempted. Determinate, and the
    /// exit must invent no doubt.
    NothingLanded,
    /// A trace came back. The trace is the speech.
    Trace,
}

impl OnScript {
    /// Every door reachable through this seam.
    ///
    /// This list is not trusted to be the population: the census in
    /// [`every_forward_door_is_either_swept_or_recorded_unreachable`] derives
    /// the exits from `forward()`'s own source and refuses a door that is not
    /// here.
    const ALL: [Self; 7] = [
        Self::NeverAnswers,
        Self::Unparseable,
        Self::OkWithNoBody,
        Self::UnreadableTrace,
        Self::RefusedBeforeEntry,
        Self::RefusedWithNoErrorBody,
        Self::ATrace,
    ];

    /// What this door knows about the workspace — see [`Knows`].
    fn knows(self) -> Knows {
        match self {
            Self::NeverAnswers
            | Self::Unparseable
            | Self::OkWithNoBody
            | Self::UnreadableTrace
            | Self::RefusedWithNoErrorBody => Knows::Unknown,
            Self::RefusedBeforeEntry => Knows::NothingLanded,
            Self::ATrace => Knows::Trace,
        }
    }

    /// The bytes this door puts back on the wire.
    fn answer(self) -> io::Result<String> {
        Ok(match self {
            Self::NeverAnswers => return Err(io::Error::other("connection reset by peer")),
            Self::Unparseable => "<html>502 Bad Gateway</html>".to_owned(),
            Self::OkWithNoBody => json!({"id": null, "ok": true}).to_string(),
            Self::UnreadableTrace => {
                json!({"id": null, "ok": true, "body": {"not_a": "trace"}}).to_string()
            }
            // The daemon's own pre-entry refusal, verbatim in shape: the cold
            // gate answers `corpus_warming` before the entry world is pinned
            // (`registry::server::cold_gate_wire`).
            Self::RefusedBeforeEntry => json!({"id": null, "ok": false, "error": {
                "code": "corpus_warming",
                "recovery": "retry",
                "message": "the drawer is still warming: this workspace has no resident corpus \
                            engine yet (cold start)",
            }})
            .to_string(),
            Self::RefusedWithNoErrorBody => json!({"id": null, "ok": false}).to_string(),
            Self::ATrace => json!({"id": null, "ok": true, "body": committed_trace()}).to_string(),
        })
    }
}

/// A fake daemon that breaks exactly one way at the one op this verb speaks.
///
/// It answers NOTHING else: under one lane a `fingerprint`, `toc`, `cat` or
/// `splice` trip is a law violation, and the assertion below is what makes that
/// a measurement rather than a claim.
struct Fake {
    on_script: OnScript,
    /// Set when the `script` request was actually put on the door — the fact the
    /// indeterminacy claim rests on.
    sent: bool,
    /// The last `script` request's bytes, verbatim.
    last: Option<Value>,
}

impl Fake {
    fn breaking(on_script: OnScript) -> Self {
        Self {
            on_script,
            sent: false,
            last: None,
        }
    }
}

impl Door for Fake {
    fn call(&mut self, request: &Value) -> io::Result<String> {
        assert_eq!(
            request["op"], "script",
            "one lane: the whole attempt IS the wire `script` op, and it spends \
             no other trip: {request}"
        );
        self.sent = true;
        self.last = Some(request.clone());
        self.on_script.answer()
    }
}

/// A daemon that refuses BEFORE the premise exists.
///
/// **This is not a hypothetical.** `registry::script_op::serve` has exactly
/// three `Err(ErrorBody)` exits and every one of them stands ABOVE
/// `let entry = world.at_fingerprint.0.clone()`: the cold gate
/// (`corpus_warming`), the entry pass (`io_error`), and the warm→pin race
/// (`corpus_race`). `serve_line` adds four more, all further above still — a
/// decode refusal, `unknown_op` on a non-v3 session, `bad_request` for an
/// unbound workspace, and the internal routing arm. So a §8 error frame from
/// this door NEVER carries an entry fingerprint, and a trace built here would
/// have to fabricate its first field.
///
/// That is why this door's silence is the CONTRACT rather than a gap, and why
/// the sweep's predicate must go red on it.
struct NoPremise;

impl Door for NoPremise {
    fn call(&mut self, request: &Value) -> io::Result<String> {
        assert_eq!(request["op"], "script", "the one op");
        OnScript::RefusedBeforeEntry.answer()
    }
}

/// Run the claim script with `flags` against `door`, returning whatever the verb
/// gives back — a trace, or the diagnostic of an exit that had no premise.
fn run(door: &mut dyn Door, flags: &[&str]) -> Result<ScriptTrace, String> {
    let argv: Vec<String> = flags.iter().map(|flag| (*flag).to_owned()).collect();
    attempt(&argv, CLAIM, door)
}

/// The three facts an indeterminate exit owes its caller. They are asserted as
/// SUBSTRINGS of the diagnostic, because the diagnostic is the operating
/// surface until `script-trace-premise-unknown-spelling` lands.
const OWED: [&str; 3] = [
    // 1. the answer did not arrive (or did not survive)
    "`script`",
    // 2. a commit may have landed anyway
    "UNKNOWN",
    // 3. the remedy, and the one thing that must not be done
    "never resend",
];

/// The sweep's predicate, named once so the control arm can apply the SAME one.
fn speaks(door: &mut dyn Door, flags: &[&str], knows: Knows) -> Result<(), String> {
    let outcome = run(door, flags);
    match (knows, outcome) {
        (Knows::Trace, Ok(_)) => Ok(()),
        (Knows::Trace, Err(why)) => Err(format!("a readable trace must come back, not: {why}")),
        (Knows::Unknown | Knows::NothingLanded, Ok(trace)) => Err(format!(
            "no trace arrived, so none may be spoken — this one names {}",
            trace.entry_fingerprint
        )),
        (Knows::Unknown, Err(why)) => {
            for owed in OWED {
                if !why.contains(owed) {
                    return Err(format!(
                        "an exit that does not know whether the commit landed must say so, \
                         and this one never says {owed:?}: {why}"
                    ));
                }
            }
            Ok(())
        }
        (Knows::NothingLanded, Err(why)) => {
            if !why.contains("nothing landed") {
                return Err(format!(
                    "a determinate refusal states that nothing landed: {why}"
                ));
            }
            if why.contains("UNKNOWN") || why.contains("never resend") {
                return Err(format!(
                    "the daemon said nothing was attempted, so this exit must invent \
                     no doubt: {why}"
                ));
            }
            Ok(())
        }
    }
}

// ── the doors, one at a time ─────────────────────────────────────────────────

/// The load-bearing door: the program went out and no answer came. This engine
/// does not know whether it landed, and — sharper than the retired local
/// lane's — a `script` op that got no answer may have COMMITTED daemon-side,
/// because the daemon owns the whole attempt now.
#[test]
fn a_program_whose_answer_never_came_states_all_three_facts_it_owes() {
    let mut door = Fake::breaking(OnScript::NeverAnswers);
    let why = run(&mut door, &["--actor", "8ab41c02"])
        .expect_err("no trace arrived, so no trace may be spoken");

    assert!(door.sent, "the premise of the whole claim");
    assert!(
        why.contains("the daemon did not answer `script`"),
        "fact 1 — the answer never came: {why}"
    );
    assert!(
        why.contains("SENT") && why.contains("UNKNOWN"),
        "fact 2 — a commit MAY have landed daemon-side: {why}"
    );
    assert!(
        why.contains("fresh read") && why.contains("never resend"),
        "fact 3 — verify before retrying, and a resend writes twice: {why}"
    );
}

/// An answer that is not a frame is the same indeterminacy as no answer: the
/// daemon may have committed before replying with something unreadable.
#[test]
fn an_unparseable_answer_is_the_same_indeterminacy_as_no_answer() {
    let mut door = Fake::breaking(OnScript::Unparseable);
    let why = run(&mut door, &["--actor", "8ab41c02"]).expect_err("nothing readable arrived");

    assert!(
        why.contains("bytes this engine cannot read"),
        "it names the door it died at: {why}"
    );
    for owed in OWED {
        assert!(why.contains(owed), "{owed:?} missing: {why}");
    }
}

/// A trace shape this build cannot read is the THIRD indeterminate exit, and it
/// is the one a version skew produces: the daemon committed, and the answer
/// describing it will not decode here.
#[test]
fn a_trace_this_build_cannot_read_still_states_the_indeterminacy() {
    let mut door = Fake::breaking(OnScript::UnreadableTrace);
    let why = run(&mut door, &["--actor", "8ab41c02"]).expect_err("the trace would not decode");

    assert!(
        why.contains("align their versions"),
        "it names the remedy for a skew rather than blaming the script: {why}"
    );
    for owed in OWED {
        assert!(why.contains(owed), "{owed:?} missing: {why}");
    }
}

/// `ok: true` with no body, and `ok: false` with no error, both violate §8's own
/// frame shape. The daemon may have done anything, so both are indeterminate.
#[test]
fn a_frame_that_violates_the_eight_shape_is_indeterminate_in_both_directions() {
    for door in [OnScript::OkWithNoBody, OnScript::RefusedWithNoErrorBody] {
        let mut fake = Fake::breaking(door);
        let why = run(&mut fake, &["--actor", "8ab41c02"])
            .expect_err("a frame with neither body nor error cannot produce a trace");
        assert!(
            why.contains("violates the §8 frame shape"),
            "{door:?} names the shape it wanted: {why}"
        );
        for owed in OWED {
            assert!(why.contains(owed), "{door:?}: {owed:?} missing: {why}");
        }
    }
}

/// ⭐ **THE DETERMINATE ARM, and the §8 ruling it answers to.**
///
/// The canonical line (afe34e1a and e57663f7, 2026-08-23): *a readable §8
/// refusal speaks as `CommitLeg::Refused` IFF every trace fact is
/// daemon-supplied — the refusal frame's own premise token, code, and words,
/// nothing minted.*
///
/// **VERIFIED, and the branch is unreachable.** `wire::ErrorBody` has no slot
/// that can carry an entry premise for these codes (`expected`/`actual` are the
/// §8 `cas_mismatch`/`root_mismatch` node tokens, `new_fingerprint` is the
/// `cas_mismatch` resend token, `scope` is the scoped-premise spelling), and
/// every §8 exit of the `script` door stands above the entry pin — see
/// [`NoPremise`] for the enumeration. So there is no premise token to speak
/// with, the arm correctly stays `Err`, and what it must do instead is state the
/// determinacy WITHOUT inventing doubt.
#[test]
fn a_refusal_that_never_reached_an_entry_says_nothing_landed_and_invents_no_doubt() {
    let mut door = Fake::breaking(OnScript::RefusedBeforeEntry);
    let why = run(&mut door, &["--actor", "8ab41c02"]).expect_err("no entry, so no trace");

    assert!(
        why.contains("refused before any entry existed"),
        "it names WHY there is no trace: {why}"
    );
    assert!(
        why.contains("nothing was evaluated and nothing landed"),
        "the determinate half — the daemon said so itself: {why}"
    );
    assert!(
        !why.contains("UNKNOWN") && !why.contains("never resend"),
        "a determinate refusal must not borrow the indeterminate arm's doubt — \
         it would send a caller to re-read a file that provably did not change: {why}"
    );
    assert!(
        why.contains("corpus_warming"),
        "and the daemon's own error body rides verbatim: {why}"
    );
}

/// The success arm: a whole readable trace comes back and is carried VERBATIM.
/// The daemon owns every fact in it — this lane re-types nothing.
#[test]
fn a_readable_trace_comes_back_verbatim_and_this_lane_mints_nothing() {
    let mut door = Fake::breaking(OnScript::ATrace);
    let trace = run(&mut door, &["--actor", "8ab41c02"]).expect("the trace arrives");

    assert_eq!(
        trace.entry_fingerprint, ENTRY,
        "the premise is the DAEMON's — this engine no longer mints one"
    );
    assert_eq!(trace.outcome, ScriptOutcome::Committed);
    let commit: Value = serde_json::from_str(trace.commit.as_ref().expect("the leg").get())
        .expect("the leg is the daemon's own bytes");
    assert_eq!(
        commit["fingerprint_after"], "b3:c4e91d02",
        "the §4.4 response is embedded, not re-typed: {commit}"
    );
    let sent = door.last.expect("the door recorded the request");
    assert_eq!(sent["source"], json!(CLAIM), "the program rode verbatim");
    assert_eq!(sent["actor"], json!("8ab41c02"));
}

/// A moved TOUCH SET arrives as the daemon's own `conflict` INSIDE the trace —
/// not as an error frame, and not as anything this lane classifies. The mismatch
/// extras ride the commit leg verbatim, `scope` included, which is the §5.7
/// spelling the touch-set guard uses.
#[test]
fn a_moved_touch_set_arrives_as_the_daemons_own_conflict_in_band() {
    struct Conflicting;
    impl Door for Conflicting {
        fn call(&mut self, request: &Value) -> io::Result<String> {
            assert_eq!(request["op"], "script");
            Ok(json!({"id": null, "ok": true, "body": conflict_trace()}).to_string())
        }
    }

    let trace = run(&mut Conflicting, &["--actor", "8ab41c02"]).expect("a conflict is an ANSWER");

    assert_eq!(trace.outcome, ScriptOutcome::Conflict);
    assert!(
        trace.fault.is_none(),
        "a moved world is not a fault — a consumer greps the two apart"
    );
    let commit: Value = serde_json::from_str(trace.commit.as_ref().expect("the leg").get())
        .expect("the leg is the daemon's bytes");
    assert_eq!(commit["code"], "fingerprint_mismatch");
    assert_eq!(
        commit["scope"], CARD,
        "the touch-set refusal names the moved premise's SCOPE (§5.7) — the \
         world-grain refusal this card deleted named none: {commit}"
    );
    assert!(
        commit.get("changed").is_none(),
        "`changed` is STRUCK (§18 row 2, ruled 2026-08-10): {commit}"
    );
}

// ── the law, and the control that makes it a measurement ─────────────────────

/// **THE LAW.** No exit of `forward()` leaves a consumer unable to tell a
/// refusal from a killed process.
///
/// It walks every door in `OnScript::ALL` rather than trusting the tests above
/// to stay in step with the code. `ALL` is a maintained list, so on its own it
/// makes coverage true only for doors someone remembered; the census below is
/// what makes the list answerable to `forward()`'s real exits.
#[test]
fn no_forward_door_leaves_the_caller_unable_to_tell_what_happened() {
    for door in OnScript::ALL {
        let mut fake = Fake::breaking(door);
        speaks(&mut fake, &["--actor", "8ab41c02"], door.knows())
            .unwrap_or_else(|why| panic!("{door:?} did not speak: {why}"));
        assert!(
            fake.sent,
            "{door:?} never reached the wire, so it swept nothing"
        );
    }
}

/// ⭐ **THE POSITIVE CONTROL for the sweep above.** The same predicate, applied
/// with the WRONG expectation to the door the contract deliberately keeps silent
/// about doubt, must go red.
///
/// [`NoPremise`] fails before the entry fingerprint exists, so it has no premise
/// to state and a synthesized one would mint a fact. Asking `speaks()` to find
/// the indeterminate prose there must FAIL — and that failure is what proves the
/// sweep is capable of failing at all. Without this arm,
/// [`no_forward_door_leaves_the_caller_unable_to_tell_what_happened`] would pass
/// just as happily against an engine that appended the doubt paragraph to every
/// diagnostic, or against a predicate that asserted nothing.
///
/// It also pins the OTHER half of the absence contract: this door's silence is
/// the contract's guarantee — nothing evaluated, the workspace unchanged — and
/// the guarantee is only true because the indeterminate doors above stopped
/// sharing this exit with it.
#[test]
fn the_sweeps_predicate_goes_red_on_the_door_that_deliberately_states_no_doubt() {
    let mut door = NoPremise;
    let outcome = speaks(&mut door, &["--actor", "8ab41c02"], Knows::Unknown);

    let why = outcome.expect_err(
        "a pre-entry refusal must NOT carry the indeterminacy prose — if it does, \
         either the engine is inventing doubt it does not have, or this control has \
         stopped being able to fail",
    );
    assert!(
        why.contains("must say so"),
        "the control fails on the predicate's own words, not by accident: {why}"
    );

    // And the same door under its TRUE expectation passes — otherwise the arm
    // above would be red for the trivial reason that this door speaks nothing.
    speaks(
        &mut NoPremise,
        &["--actor", "8ab41c02"],
        Knows::NothingLanded,
    )
    .expect("its own class is determinate, and it states that");
}

// ── the census: the population, DERIVED from the code it claims to cover ─────

/// `forward()`'s own source. The sweep above walks a hand-maintained list, and a
/// hand-maintained list is a literal wearing a derived costume: an exit added to
/// `forward()` and never wired into `OnScript` is invisible to it, the suite
/// stays green, and nothing says the coverage shrank.
///
/// Reading the source is the cheap instrument that makes the list ANSWERABLE.
/// The exhaustiveness cannot be a compile-time fact here — `forward()`'s exits
/// are five constructions of ONE type (`Fail::tool`), so a variant-exhaustive
/// match would certify a population it cannot see. Precedent for reading a
/// sibling module's source in a test:
/// `crates/mrd/tests/rules_cli.rs` § `the_cli_layer_holds_no_second_resolver`.
const FORWARD_SOURCE: &str = include_str!("../src/script/cmd.rs");

/// Every `Fail::tool(` exit inside `fn forward`, in source order, as
/// `(1-based line, what it claims to know)`.
///
/// The claim is READ FROM THE CODE, not declared: an exit whose message carries
/// the `MAY_HAVE_LANDED` clause states the indeterminacy, and one that does not
/// declares the outcome determinate. That is the law itself, so the census
/// measures the law rather than a label beside it.
///
/// Comment lines are dropped before matching, so prose naming an exit can
/// neither satisfy nor break a claim about code — the same discipline the
/// rules-CLI structural test uses.
fn forward_exits() -> Vec<(usize, Knows)> {
    let lines: Vec<(usize, &str)> = FORWARD_SOURCE
        .lines()
        .enumerate()
        .map(|(i, l)| (i + 1, l))
        .collect();
    let start = lines
        .iter()
        .position(|(_, line)| line.starts_with("fn forward("))
        .expect("`fn forward` is the function this census is about");
    let end = lines[start..]
        .iter()
        .position(|(_, line)| *line == "}")
        .expect("the function closes at column zero")
        + start;

    let mut exits: Vec<(usize, String)> = Vec::new();
    for (number, raw) in &lines[start..=end] {
        let line: &str = raw;
        if line.trim_start().starts_with("//") {
            continue;
        }
        if let Some(at) = line.find("Fail::tool(") {
            exits.push((*number, line[at..].to_owned()));
        } else if let Some((_, text)) = exits.last_mut() {
            text.push('\n');
            text.push_str(line);
        }
    }
    exits
        .into_iter()
        .map(|(number, text)| {
            let knows = if text.contains("MAY_HAVE_LANDED") {
                Knows::Unknown
            } else {
                Knows::NothingLanded
            };
            (number, knows)
        })
        .collect()
}

/// ⭐ **THE POPULATION GATE.** Every exit of `forward()` is either swept by the
/// law above or recorded here as unreachable WITH the law that makes it so.
///
/// This is the gate the sweep could not be: the sweep asks "does each door I
/// know about speak?", and this asks "do I know about every door?". An exit
/// added to `forward()` and not wired in fails HERE, loudly, naming its line —
/// instead of passing silently as a coverage gap nobody minted a signal for.
///
/// **This census replaces the one over `commit()`.** `commit()` is the retired
/// local transaction, deleted in PR 2 of card
/// `script-door-commit-premise-world-grain-vs-touch-set`; sweeping it would be a
/// gate over code no caller can reach, which is a gate that measures nothing.
#[test]
fn every_forward_door_is_either_swept_or_recorded_unreachable() {
    /// The census, in `forward()`'s own source order. Adding an exit to
    /// `forward()` means adding its row here and either wiring it into
    /// `OnScript` or stating the law that keeps it unreachable.
    const CENSUS: [(Knows, &[OnScript]); 5] = [
        // the door call failed — the answer never came
        (Knows::Unknown, &[OnScript::NeverAnswers]),
        // the line would not parse as a frame
        (Knows::Unknown, &[OnScript::Unparseable]),
        // a trace this build cannot read
        (Knows::Unknown, &[OnScript::UnreadableTrace]),
        // a §8 error frame — DETERMINATE, and the only one (see `NoPremise`)
        (Knows::NothingLanded, &[OnScript::RefusedBeforeEntry]),
        // the §8 shape violation, reached from both directions
        (
            Knows::Unknown,
            &[OnScript::OkWithNoBody, OnScript::RefusedWithNoErrorBody],
        ),
    ];

    let exits = forward_exits();
    let rendered = exits
        .iter()
        .map(|(line, knows)| format!("  cmd.rs:{line} {knows:?}"))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        exits.len(),
        CENSUS.len(),
        "`forward()` has {} exits and the census names {}. An exit added to `forward()` is \
         invisible to the sweep until it is wired into `OnScript` (or recorded here with the \
         law that keeps it unreachable). The exits as read from source:\n{rendered}",
        exits.len(),
        CENSUS.len(),
    );
    for (index, ((line, knows), (expected, _))) in exits.iter().zip(CENSUS).enumerate() {
        assert_eq!(
            *knows, expected,
            "census row {index} says {expected:?} and cmd.rs:{line} reads as {knows:?} — \
             the population moved under the sweep. Every exit past the send point must \
             carry MAY_HAVE_LANDED; the pre-entry refusal must not"
        );
    }

    // The success arm is not a `Fail::tool` exit, so it has no census row — but
    // it must exist, or every row above would be a failure mode of a function
    // that can no longer succeed.
    assert!(
        FORWARD_SOURCE.contains("from_str::<ScriptTrace>"),
        "`forward()` must still have the ONE arm that yields a trace"
    );

    // And the other direction: no door sits in `OnScript` without a census row,
    // so the enum cannot drift away from the code either.
    let mut swept: Vec<OnScript> = CENSUS
        .iter()
        .flat_map(|(_, doors)| doors.iter().copied())
        .collect();
    for door in OnScript::ALL {
        // `ATrace` is the success arm's door: swept by the law, censused by the
        // assertion above rather than by a row, because it is not an exit.
        if door == OnScript::ATrace {
            continue;
        }
        let at = swept
            .iter()
            .position(|candidate| *candidate == door)
            .unwrap_or_else(|| {
                panic!(
                    "{door:?} is in the sweep but names no exit of `forward()` — it tests a \
                     door the engine no longer has"
                )
            });
        swept.remove(at);
    }
    assert!(
        swept.is_empty(),
        "the census claims these doors are swept and `OnScript::ALL` does not walk them: \
         {swept:?}"
    );
}

// ── retired against named twins ──────────────────────────────────────────────
//
// Two rows of this suite lost their subject when the local transaction stopped
// being a lane. Neither is dropped silently; each names the daemon-side test
// that now owns the behavior:
//
// * `a_dry_run_that_lost_its_answer_declares_retry_because_a_rehearsal_writes_nothing`
//   — the dry/live split of a lost commit's recovery class. The splitting
//   function is now `registry::script_op::lost_commit`, and its twin is
//   `registry/src/script_op.rs` § `a_panicked_splice_speaks_commit_unknown_never_a_plain_refusal`,
//   which builds both legs (`lost_commit(false)` and `lost_commit(true)`) and
//   asserts the two classes apart. `forward()` cannot split on `--dry`: it has
//   no trace to carry a class in, which is exactly what
//   `script-trace-premise-unknown-spelling` is carded to fix.
//
// * `a_multi_file_armed_set_issues_one_set_splice` — the §4.4 set-form
//   lowering. The CLI issues no splice at all now; the twins are
//   `registry/tests/script_op.rs` § `an_armed_target_reads_back_its_own_armed_content_and_commits_once`
//   (one commit per attempt, over a live daemon) and § `a_set_member_that_cannot_validate_refuses_the_whole_set`
//   (the set form's all-or-nothing law).

// ── additive migration, proved rather than asserted ──────────────────────────

/// A trace minted before the `commit_unknown` field existed carries none. It
/// must still deserialize, and the missing field must read as `false` — not as a
/// decode error, and not as an alarm.
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
    struct Refusing;
    impl Door for Refusing {
        fn call(&mut self, request: &Value) -> io::Result<String> {
            assert_eq!(request["op"], "script");
            Ok(json!({"id": null, "ok": true, "body": refused_trace()}).to_string())
        }
    }

    let trace = run(&mut Refusing, &["--actor", "8ab41c02"]).expect("the trace arrives");
    assert_eq!(trace.outcome, ScriptOutcome::Refused);
    assert!(!trace.commit_unknown, "the daemon declared it determinate");

    let json = serde_json::to_string(&trace).expect("the trace serializes");
    assert!(
        !json.contains("commit_unknown"),
        "a marker present on every trace is noise a consumer must read past: {json}"
    );
}
