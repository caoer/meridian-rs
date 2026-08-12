//! Trace assembly — the consumer-plane half of [`wire::ScriptTrace`].
//!
//! The TYPE lives in `wire` (§ A.7: the trace is a wire response body, and
//! `mrd script --json` is the same shape — Law 2 plus the one-shape law). The
//! ASSEMBLY lives here because it takes `effects::ScriptEval`, which `wire`
//! must never name: the commit leg is a §4.4 splice response, and the serve
//! organ is where splice responses exist.
//!
//! Two laws shape the module, both negative (moved verbatim from
//! `mrd::script::trace`, which now re-exports this):
//!
//! - **The commit leg IS the §4.4 splice response, embedded unmodified** — a
//!   [`RawValue`], the exact bytes a v3 client sees, never a re-typed copy.
//! - **There is no `attempts` field.** Retry is the caller's loop.

use effects::{ArmedEdit, EvalError, ReadPosition, ScriptEval};
use serde_json::value::RawValue;
use wire::{
    ArmedEntry, FaultClass, ReadEntry, Recovery, ScriptFault, ScriptOutcome, ScriptTelemetry,
    ScriptTrace, TraceEntry,
};

/// The commit leg the consumer plane hands the assembler: the engine's own
/// answer, never re-typed.
///
/// A splice is issued **exactly when something was armed**, so
/// [`CommitLeg::NotIssued`] is the read-class path — zero-armed, or an
/// evaluation that never reached the commit.
#[derive(Debug)]
pub enum CommitLeg {
    /// No splice was issued.
    NotIssued,
    /// The §4.4 splice response, verbatim (v3 vocabulary).
    Response(Box<RawValue>),
    /// A §5.1 `fingerprint_mismatch{expected, actual}`, verbatim — the
    /// splice's own response, so it embeds through the same one leg.
    Conflict(Box<RawValue>),
    /// The §4.4 response to a REHEARSAL (`dry: true`), verbatim: the batch ran
    /// everything except disk, so the effect set is real and nothing was
    /// applied. The outcome is `no_effect`; the response's own `dry: true`
    /// and `fingerprint_after: null` are the engine's, not ours.
    Rehearsal(Box<RawValue>),
    /// A commit-time refusal that is not the world moving. The engine's own
    /// message rides verbatim into [`ScriptFault::reason`] and the outcome is
    /// `refused`, so a refusal and a fault still grep apart.
    Refused(Refusal),
    /// **The splice was ISSUED and its outcome is not known.** A separate leg
    /// from [`CommitLeg::Refused`] because every other leg STATES what
    /// happened to the workspace and this one cannot — the trace carries
    /// [`wire::ScriptTrace::commit_unknown`] beside the refusal instead.
    Unknown(Refusal),
}

/// The wire's refusal triple, carried across the script boundary TYPED.
///
/// The engine's frame carries `recovery` first-class; flattening it into
/// prose would destroy a class no host-side change can recover. One engine,
/// one refusal vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// The §8 code, verbatim — absent on an engine-minted refusal.
    pub code: Option<String>,
    /// The §8 recovery class, from [`wire::Recovery`] and nowhere else.
    pub recovery: Option<Recovery>,
    /// The rendered wording, exactly as the face has always received it.
    pub reason: String,
}

impl Refusal {
    /// A refusal minted at the entry itself: no frame crossed a door, so
    /// there is no `code`, and the class is named at the site that mints it.
    #[must_use]
    pub fn minted(recovery: Recovery, reason: impl Into<String>) -> Self {
        Self {
            code: None,
            recovery: Some(recovery),
            reason: reason.into(),
        }
    }
}

/// Assemble the trace from the eval facts plus the commit leg.
///
/// Nothing here mints a fact: reads are the recording, arms are the kernel's
/// armed list, and the commit is bytes. A failed evaluation never commits, so
/// a commit leg supplied alongside one is dropped — the arms it would
/// describe were never applied.
#[must_use]
pub fn assemble(
    entry_fingerprint: impl Into<String>,
    eval: &ScriptEval,
    commit: CommitLeg,
) -> ScriptTrace {
    let mut fault = eval.outcome.as_ref().err().map(fault_of);
    let mut commit_unknown = false;
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
        (None, CommitLeg::Refused(refusal)) => {
            fault = Some(ScriptFault {
                line: None,
                class: FaultClass::Refused,
                code: refusal.code,
                recovery: refusal.recovery,
                reason: refusal.reason,
            });
            (ScriptOutcome::Refused, None)
        }
        // The splice went out and its outcome is not known. The fault is
        // shaped exactly like a refusal — one vocabulary — and the marker
        // beside it is what stops `refused` from being read as "nothing was
        // applied". No commit leg: there is no answer to embed.
        (None, CommitLeg::Unknown(refusal)) => {
            commit_unknown = true;
            fault = Some(ScriptFault {
                line: None,
                class: FaultClass::Refused,
                code: refusal.code,
                recovery: refusal.recovery,
                reason: refusal.reason,
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

    ScriptTrace {
        entry_fingerprint: entry_fingerprint.into(),
        outcome,
        trace,
        commit,
        fault,
        guard_expected: None,
        // Over the armed rows as they stand here — after the consumer plane
        // threaded each row's CAS token, which is the same list the commit
        // sends as `plan_edits[]`. Hashing the pre-threading rows would
        // publish a digest for a set that never goes on any wire.
        armed_digest: (!eval.armed.is_empty())
            .then(|| super::digest::armed_digest(&super::digest::ArmedRow::of_all(&eval.armed))),
        commit_unknown,
        telemetry: eval.telemetry,
    }
}

/// The trace of a caller guard that did not match: the run refused at the
/// door, with **zero evaluation** — no reads, nothing armed, no splice.
#[must_use]
pub fn guard_refused(
    entry_fingerprint: impl Into<String>,
    pinned: impl Into<String>,
) -> ScriptTrace {
    ScriptTrace {
        entry_fingerprint: entry_fingerprint.into(),
        outcome: ScriptOutcome::Conflict,
        trace: Vec::new(),
        commit: None,
        fault: None,
        guard_expected: Some(pinned.into()),
        // Nothing was armed — the guard refused before evaluation — so there
        // is no armed set to describe. Absence, never a digest of `[]`.
        armed_digest: None,
        // The guard refused at the door: no splice was issued, so nothing is
        // unknown about one.
        commit_unknown: false,
        telemetry: ScriptTelemetry {
            fuel_used: 0,
            mem_used: 0,
            reads_used: 0,
            wall_ms: 0,
        },
    }
}

/// Classify a kernel error into the closed fault taxonomy. `reason` is the
/// kernel's own `Display`, carried verbatim; the runtime fault re-frames its
/// opener because the script entry is not a rule (defect-ledger DIV-1).
#[must_use]
pub fn fault_of(error: &EvalError) -> ScriptFault {
    if let EvalError::Runtime { reason, line, .. } = error {
        return ScriptFault {
            line: *line,
            class: FaultClass::Runtime,
            // A fault is not a refusal: the §8 triple belongs to the refused
            // class alone, and a fault carries neither half of it.
            code: None,
            recovery: None,
            reason: match line {
                Some(line) => format!("runtime fault at line {line} — {reason}"),
                None => format!("runtime fault — {reason}"),
            },
        };
    }
    let (class, line) = match error {
        EvalError::Parse { .. } => (FaultClass::Parse, None),
        // Returned above; the arm stays for exhaustiveness.
        EvalError::Runtime { .. } => unreachable!("handled above"),
        // The script entry has no hook, so it never raises `MissingEntry`; if
        // a future entry does, it is an evaluation failure like any other.
        EvalError::MissingEntry { .. } => (FaultClass::Runtime, None),
        EvalError::Budget { .. }
        | EvalError::SourceTooLarge { .. }
        | EvalError::ReadBudget { .. } => (FaultClass::Budget, None),
        EvalError::ArmedBudget { line, .. } => (FaultClass::Budget, Some(*line)),
        EvalError::MultiFileWriteSet { line, .. } => (FaultClass::Refused, Some(*line)),
    };
    ScriptFault {
        line,
        class,
        // No frame crossed a door for an arm-time refusal, so there is no §8
        // code to carry — but the class is knowable at the site: the script
        // armed a set no splice may carry, and only the script can change that.
        code: None,
        recovery: match class {
            FaultClass::Refused => Some(Recovery::Fix),
            FaultClass::Parse | FaultClass::Runtime | FaultClass::Budget => None,
        },
        reason: error.to_string(),
    }
}
