//! `ScriptTrace` — the run-plane boundary contract between the entry and the
//! face that renders it (`mrd script --json` is that contract;
//! `run-script-contract-delta` § Q9 is its shape).
//!
//! Two laws shape the whole module, and both are negative:
//!
//! - **The commit leg IS the §4.4 splice response, embedded unmodified.** It is
//!   carried as a [`RawValue`] — the exact bytes the daemon answered, never a
//!   re-typed copy. A second commit-fact shape would drift the moment §4.4 grows
//!   a field, and the face's wrote-lines would stop rendering from `armed[]` the
//!   way put faces already do. `verdicts` (rules-as-data), the receipt fact, the
//!   rev transitions and `fingerprint_before/after` all ride it for free.
//! - **There is no `attempts` field.** Retry is the HOST's loop, so `attempts:N`
//!   is a host fact stamped on the composed face (r3 ruling). A trace that
//!   carried it would mean the retry loop had leaked into the entry.
//!
//! Telemetry is unconditional — present on faults and refusals too, the
//! `RuleTelemetry` precedent.

use effects::{ArmedEdit, EvalError, ReadFace, ReadPosition, ScriptEval, ScriptTelemetry};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use wire::PlanEdit;

/// What the whole run amounted to. Read-class outcomes (`no_effect`) are
/// first-class success — the `Ok(vec![])` precedent — and a refusal is
/// deliberately NOT a fault: the two must grep apart (r4/F7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptOutcome {
    /// The one guarded splice landed.
    Committed,
    /// Nothing landed — no receipt, no fingerprint advance, the workspace
    /// unchanged. Reached two ways: nothing was armed (no splice at all), or the
    /// splice was a rehearsal ([`CommitLeg::Rehearsal`], which carries the
    /// daemon's own `dry: true`).
    NoEffect,
    /// The commit met a moved world: `fingerprint_mismatch`, recovery `resync`.
    Conflict,
    /// Evaluation failed — parse, runtime, or a budget ceiling.
    Fault,
    /// An arm-time law refused the script. Nothing was applied.
    Refused,
}

/// The fault taxonomy, CLOSED at four words (r4/F7). Never a §8 wire code — a
/// script fault does not cross the wire, so the wire's closed taxonomy stays
/// closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaultClass {
    /// The source would not parse as Starlark.
    Parse,
    /// Parsed, then faulted at eval.
    Runtime,
    /// A ceiling was reached — fuel, memory, source bytes, reads, armed edits.
    Budget,
    /// An arm-time law refused (single-write-file). Semantically not a fault.
    Refused,
}

/// Why the run produced nothing. `line` is present exactly when the kernel could
/// attribute one — the arm-time refusals name their `put()` call; a parse or
/// budget fault carries the position inside `reason` when the evaluator knew it.
/// Absence is absence, never a synthesized line 0.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptFault {
    /// 1-based source line, when the kernel attributed one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// Which family of failure this is.
    pub class: FaultClass,
    /// The kernel's own wording, carried verbatim — the face renders it after
    /// its `SCRIPT:` opener.
    pub reason: String,
}

/// One traced read: the recorded response, carried verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadEntry {
    /// 1-based source line of the `read()` call.
    pub line: u32,
    /// The path read.
    pub path: String,
    /// The section, when the cat face was asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    /// The host's response, exactly as recorded — this is what replay serves.
    pub face: ReadFace,
}

/// One traced arm: the wire plan-edit shape plus whether the commit carried it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArmedEntry {
    /// 1-based source line of the `put()` call that armed it.
    pub line: u32,
    /// The content path this edit writes.
    pub path: String,
    /// The `splice.plan_edits[]` item, carried verbatim (ruling B′).
    pub edit: PlanEdit,
    /// Nesting depth at arm time; 0 at module top level. A trace fact only —
    /// **depth never suppresses**: an applied effect renders at any depth.
    pub depth: u32,
    /// The post-commit zip: `true` only when the one guarded splice landed. The
    /// face renders `wrote` when true and `armed … [not committed]` when false,
    /// so wrote-lines come from `armed[]` exactly as put faces do today.
    pub committed: bool,
}

/// One line of the decision trace, in the order the face renders it: reads in
/// call order, then the armed block.
///
/// The read/echo split IS the statement-position rule (`run-plane.md` § The
/// statement-position rule): a read whose call is the whole right-hand side of a
/// top-level assignment — or a top-level expression statement — is `echo`, and
/// the face renders it; every other position is `read` and stays quiet. Both are
/// recorded either way, so the split costs the record nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TraceEntry {
    /// A quiet read — comprehension, condition, loop body, function body.
    Read(ReadEntry),
    /// A top-level-statement read: `card = read(…)` binds and echoes.
    Echo(ReadEntry),
    /// An armed edit. Renders unsuppressably — the honesty law.
    Armed(ArmedEntry),
}

/// The commit leg the consumer plane hands the assembler: the daemon's own
/// answer, never re-typed.
///
/// The host issues a splice **exactly when something was armed**, so
/// [`CommitLeg::NotIssued`] is the read-class path — zero-armed, or an
/// evaluation that never reached the commit.
#[derive(Debug)]
pub enum CommitLeg {
    /// No splice was issued.
    NotIssued,
    /// The §4.4 splice response, verbatim.
    Response(Box<RawValue>),
    /// A §5.1 `fingerprint_mismatch{expected, actual, changed}`, verbatim. It is
    /// the splice's own response, so it embeds through the same one leg — the
    /// mismatch extras need no second shape either.
    Conflict(Box<RawValue>),
    /// The §4.4 response to a REHEARSAL (`dry: true`), verbatim: the batch ran
    /// everything except disk, so the effect set is real and nothing was
    /// applied.
    ///
    /// The outcome is therefore `no_effect` — the workspace is unchanged, no
    /// receipt was written and the fingerprint did not advance — and every armed
    /// entry stays `[not committed]`, which is what plan v1.2 states for the dry
    /// face. The rehearsal is still distinguishable from a zero-armed run
    /// without a second outcome word: this leg carries the response, whose own
    /// `dry: true` and `fingerprint_after: null` are the daemon's, not ours.
    Rehearsal(Box<RawValue>),
    /// A commit-time refusal that is not the world moving: `foreign_edit`,
    /// `bad_request{overlap}`, `would_corrupt`. The engine's own message rides
    /// verbatim into [`ScriptFault::reason`] and the outcome is `refused`, so a
    /// refusal and a fault still grep apart.
    Refused(String),
}

/// The entry's whole result: what the script did, what the commit answered, and
/// what it cost.
// `PartialEq` is absent because `RawValue` does not implement it — the commit
// leg is bytes carried verbatim, and widening a foreign type's traits to please
// a consumer is exactly the drift this arrangement exists to prevent. Compare
// two traces through their JSON, which is the contract anyway.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptTrace {
    /// The fingerprint pinned at the door (§4.7) — the premise the whole run is
    /// consistent with, and the value the commit's `if_fingerprint` carried.
    pub entry_fingerprint: String,
    /// What the run amounted to.
    pub outcome: ScriptOutcome,
    /// The decision trace: reads in call order, then the armed block.
    pub trace: Vec<TraceEntry>,
    /// The §4.4 splice response, embedded VERBATIM. Absent when no splice was
    /// issued — absence, never a null.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<Box<RawValue>>,
    /// Why the run produced nothing. Absent on `committed` / `no_effect` /
    /// `conflict` — a conflict is the world moving, not a fault.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fault: Option<ScriptFault>,
    /// The caller's own `--if-fingerprint`, present EXACTLY when the pre-eval
    /// guard refused the run — the `expected` half of the face's extras line,
    /// whose `actual` is [`ScriptTrace::entry_fingerprint`].
    ///
    /// It is here because the face renders from this trace and nothing else.
    /// Every other terminal in the conflict family carries both tokens in band
    /// (the commit-time mismatch embeds the daemon's own `{expected, actual,
    /// changed}`), and a renderer that had to reach past the contract for one
    /// line of one terminal would be a second source of the same fact.
    ///
    /// It is also the discriminator: `conflict` + no commit leg + this field is
    /// a guard refusal, which is sharper than an absent leg alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard_expected: Option<String>,
    /// Always present, whatever the outcome.
    pub telemetry: ScriptTelemetry,
}

impl ScriptTrace {
    /// Assemble the trace from the eval facts plus the commit leg.
    ///
    /// The eval facts come from `effects::eval_script`; the commit leg is the
    /// daemon's answer to the one guarded splice. Nothing here mints a fact:
    /// reads are the recording, arms are the kernel's armed list, and the commit
    /// is bytes.
    ///
    /// A failed evaluation never commits, so a commit leg supplied alongside one
    /// is dropped — the arms it would describe were never applied.
    #[must_use]
    pub fn assemble(
        entry_fingerprint: impl Into<String>,
        eval: &ScriptEval,
        commit: CommitLeg,
    ) -> Self {
        let mut fault = eval.outcome.as_ref().err().map(fault_of);
        let (outcome, commit) = match (&fault, commit) {
            (Some(fault), _) => (
                match fault.class {
                    FaultClass::Refused => ScriptOutcome::Refused,
                    _ => ScriptOutcome::Fault,
                },
                None,
            ),
            (None, CommitLeg::NotIssued) => {
                debug_assert!(
                    eval.armed.is_empty(),
                    "a splice is issued exactly when something is armed"
                );
                (ScriptOutcome::NoEffect, None)
            }
            (None, CommitLeg::Response(body)) => (ScriptOutcome::Committed, Some(body)),
            (None, CommitLeg::Conflict(body)) => (ScriptOutcome::Conflict, Some(body)),
            (None, CommitLeg::Rehearsal(body)) => (ScriptOutcome::NoEffect, Some(body)),
            (None, CommitLeg::Refused(reason)) => {
                fault = Some(ScriptFault {
                    line: None,
                    class: FaultClass::Refused,
                    reason,
                });
                (ScriptOutcome::Refused, None)
            }
        };
        let committed = outcome == ScriptOutcome::Committed;

        let mut trace: Vec<TraceEntry> = eval
            .recording
            .reads
            .iter()
            .map(|read| {
                let entry = ReadEntry {
                    line: read.line,
                    path: read.path.clone(),
                    section: read.section.clone(),
                    face: read.face.clone(),
                };
                match read.position {
                    ReadPosition::Echo => TraceEntry::Echo(entry),
                    ReadPosition::Quiet => TraceEntry::Read(entry),
                }
            })
            .collect();
        trace.extend(eval.armed.iter().map(|armed| {
            let ArmedEdit {
                path,
                edit,
                line,
                depth,
            } = armed;
            TraceEntry::Armed(ArmedEntry {
                line: *line,
                path: path.clone(),
                edit: edit.clone(),
                depth: *depth,
                committed,
            })
        }));

        Self {
            entry_fingerprint: entry_fingerprint.into(),
            outcome,
            trace,
            commit,
            fault,
            guard_expected: None,
            telemetry: eval.telemetry,
        }
    }

    /// The trace of a caller guard that did not match: the run refused at the
    /// door, with **zero evaluation** — no reads, nothing armed, no splice.
    ///
    /// It mints no §5.1 body and carries no commit leg, because no splice was
    /// issued: `expected` is `pinned`, `actual` IS
    /// [`ScriptTrace::entry_fingerprint`], and `changed` is absent because
    /// nothing asked the wire what moved — absence stays absence. Telemetry is
    /// zero and unconditional: nothing ran.
    #[must_use]
    pub fn guard_refused(entry_fingerprint: impl Into<String>, pinned: impl Into<String>) -> Self {
        Self {
            entry_fingerprint: entry_fingerprint.into(),
            outcome: ScriptOutcome::Conflict,
            trace: Vec::new(),
            commit: None,
            fault: None,
            guard_expected: Some(pinned.into()),
            telemetry: ScriptTelemetry {
                fuel_used: 0,
                mem_used: 0,
                reads_used: 0,
                wall_ms: 0,
            },
        }
    }

    /// The armed entries, in arm order — the face's armed block.
    pub fn armed_entries(&self) -> impl Iterator<Item = &ArmedEntry> {
        self.trace.iter().filter_map(|entry| match entry {
            TraceEntry::Armed(armed) => Some(armed),
            TraceEntry::Read(_) | TraceEntry::Echo(_) => None,
        })
    }
}

/// Classify a kernel error into the closed fault taxonomy. `reason` is the
/// kernel's own `Display`, carried verbatim: the refusal wordings are already
/// the face's (golden scenarios 2a and 4), so re-phrasing them here would fork
/// the text in two places.
fn fault_of(error: &EvalError) -> ScriptFault {
    let (class, line) = match error {
        EvalError::Parse { .. } => (FaultClass::Parse, None),
        // The script entry has no hook, so it never raises `MissingEntry`; if a
        // future entry does, it is an evaluation failure like any other.
        EvalError::Runtime { .. } | EvalError::MissingEntry { .. } => (FaultClass::Runtime, None),
        EvalError::Budget { .. }
        | EvalError::SourceTooLarge { .. }
        | EvalError::ReadBudget { .. } => (FaultClass::Budget, None),
        EvalError::ArmedBudget { line, .. } => (FaultClass::Budget, Some(*line)),
        EvalError::MultiFileWriteSet { line, .. } => (FaultClass::Refused, Some(*line)),
    };
    ScriptFault {
        line,
        class,
        reason: error.to_string(),
    }
}
