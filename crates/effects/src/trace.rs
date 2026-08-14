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

use std::collections::BTreeMap;

use crate::{ArmedEdit, EvalError, ReadFace, ReadPosition, ScriptEval, ScriptTelemetry};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use wire::{PlanEdit, Recovery};

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
    /// A LIVE program (effects mode, script-effects ruling 2026-08-13) ran to
    /// completion. Its acts already landed as they happened — the trace's
    /// `wrote`/`ran` entries are the record; there is no commit leg and no
    /// rollback. The vocabulary's one addition; the pure five keep their
    /// meanings.
    Effects,
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
/// attribute one — the arm-time refusals name their `put()` call, and a runtime
/// fault carries the line of its Starlark span; a parse or budget fault carries
/// the position inside `reason` when the evaluator knew it. Absence is absence,
/// never a synthesized line 0.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptFault {
    /// 1-based source line, when the kernel attributed one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// Which family of failure this is.
    pub class: FaultClass,
    /// The §8 error code of a refusal that crossed the wire, verbatim. Absent
    /// on every other class, and on a refusal the engine minted itself — no
    /// frame minted a code for those, and inventing one would put a value on
    /// the §8 surface no daemon can answer with. Carried as a string because an
    /// unrecognized code must not fail the trace: §8 clients dispatch on
    /// `recovery` alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// The refusal's recovery class — transient-vs-permanent, TYPED, from the
    /// one source the wire field's vocabulary comes from ([`wire::Recovery`],
    /// the closed six-class §8 enum).
    ///
    /// It is a field and not a fifth [`FaultClass`] on purpose: recovery is a
    /// PROPERTY of a refusal, not a KIND of fault, and a fifth variant would
    /// conflate the two axes and break every consumer matching `refused`.
    /// Absent when nothing could name it honestly — a code this engine cannot
    /// parse with no `recovery` beside it. Absence stays absence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<Recovery>,
    /// The kernel's own wording, carried verbatim — the face renders it after
    /// its `SCRIPT:` opener. A RENDERING of the refusal, never the carrier of
    /// its class: a consumer that needs the class reads `recovery`.
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
    /// A § A.7 `files[]` pattern expansion at entry — one row per pattern
    /// member, rendered before the reads (expansion happens before
    /// evaluation). Present so the binding is reconstructible and the
    /// zero-match case observable; a typo'd pattern is visible here.
    Expanded(crate::ExpansionRecord),
    /// A quiet read — comprehension, condition, loop body, function body.
    Read(ReadEntry),
    /// A top-level-statement read: `card = read(…)` binds and echoes.
    Echo(ReadEntry),
    /// An armed edit. Renders unsuppressably — the honesty law.
    Armed(ArmedEntry),
    /// Effects mode: a live `put()` that ALREADY applied (no rev, no CAS —
    /// the ruling's model). Renders unsuppressably — same honesty law.
    Wrote(WroteEntry),
    /// Effects mode: a live `run()` that ALREADY executed; the § A.8 row is
    /// the record and was the program's own return value.
    Ran(RanEntry),
}

/// One live `put()`'s record: where, how many edits, and the world the splice
/// answered (informational — nothing guards on it).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WroteEntry {
    /// The content path written.
    pub path: String,
    /// 1-based source line of the `put()` call.
    pub line: u32,
    /// How many plan edits the call carried.
    pub edits: usize,
    /// The splice's own after-fingerprint, when the door answered one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint_after: Option<String>,
}

/// One live `run()`'s record: the § A.8 per-target row, verbatim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RanEntry {
    /// The page the task lives on.
    pub page: String,
    /// 1-based source line of the `run()` call.
    pub line: u32,
    /// The § A.8 row — report or refusal — exactly as returned in-program.
    pub row: serde_json::Value,
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
    /// A §5.1 `fingerprint_mismatch{expected, actual}`, verbatim. It is
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
    Refused(Refusal),
    /// **The splice was ISSUED and its outcome is not known** — the answer never
    /// came, or came in bytes this engine cannot read
    /// (`docs/run-plane.md` § A controlled failure exit SPEAKS).
    ///
    /// It is a separate leg from [`CommitLeg::Refused`] because every other leg
    /// STATES what happened to the workspace, and this one cannot. Assembling it
    /// as a plain refusal would tell a caller nothing was applied, which is a
    /// fabrication about their own file — so the trace carries
    /// [`ScriptTrace::commit_unknown`] beside the refusal instead.
    Unknown(Refusal),
}

/// The wire's refusal triple, carried across the script boundary TYPED.
///
/// The daemon frame carries `recovery` first-class and the put door reads it;
/// flattening it into prose here would destroy a class no host-side change can
/// recover, leaving a face to match strings. One engine, one refusal vocabulary.
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
    /// A refusal the ENGINE minted: no frame crossed the wire, so there is no
    /// `code`, and the class is named at the site that mints it rather than
    /// derived from a table that never saw this refusal.
    #[must_use]
    pub fn minted(recovery: Recovery, reason: impl Into<String>) -> Self {
        Self {
            code: None,
            recovery: Some(recovery),
            reason: reason.into(),
        }
    }
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
    /// The digest of the armed set — `crate::digest::armed_digest` over the rows
    /// the armed block publishes. Present exactly when something was armed.
    ///
    /// **This field is what makes the host a courier instead of a second
    /// implementation.** The arm/commit split needs the commit child to prove it
    /// armed the set the host gated; the host takes this string from the arm's
    /// trace and hands it back as `--expect-armed`, never canonicalizing
    /// anything itself. A host that re-derived the digest from the armed rows
    /// below would be a second spelling of the serialization, and two spellings
    /// agree only by luck — the vacuous-pass shape the sub-amendment exists to
    /// prevent (`docs/run-plane.md` § Sub-amendment (the armed-set expectation)).
    ///
    /// It describes the armed block and nothing else: the receipt rides
    /// `request.receipt`, never `plan_edits[]`, so it is outside this value by
    /// construction and gated on its own pre-spawn path.
    ///
    /// It carries the digest's domain tag (`crate::digest::DOMAIN_TAG`), so a
    /// host reading this field can tell an engine that hashes `(path, edit)`
    /// pairs from one that hashed payloads alone — by literal prefix, with no
    /// parsing. That is a capability assertion, not a canonicalization, so the
    /// courier property above survives it intact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub armed_digest: Option<String>,
    /// The module's top-level bindings after a SUCCESSFUL evaluation — name →
    /// Starlark repr, in name order (the map's own order, not statement
    /// order). This is the result-echo half the ruling restored (2026-08-13,
    /// F-S1+F-S3): the face renders values from here, which is what makes the
    /// kernel's own teaching — "bind it to a name to echo it" — true.
    ///
    /// Three names never appear (the kernel's capture law,
    /// `kernel::ScriptEntry::capture_bindings`): the inert inputs
    /// (`args`, `files`), function bindings (a `def` is not a value the run
    /// computed), and a name whose LAST top-level assignment is a bare
    /// `read()` call — that value already rides the trace as the read's own
    /// `echo` entry, and one fact gets one carrier.
    ///
    /// Absent exactly when there is nothing to carry: a script that bound
    /// nothing, a failed or refused evaluation (its namespace is not a
    /// result), or the guard refusal (zero evaluation). Absence, never `{}`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bindings: BTreeMap<String, String>,
    /// Present and `true` EXACTLY when a splice was issued and its outcome is
    /// not known: the answer never arrived, or arrived unreadable
    /// (`docs/run-plane.md` § A controlled failure exit SPEAKS).
    ///
    /// **It exists because [`ScriptTrace::commit`]'s absence is already spoken
    /// for.** An absent leg means no splice was issued — the read-class path — so
    /// a lost answer that merely omitted the leg would read as a run that never
    /// tried to write. The one thing a consumer must not conclude is the one
    /// thing it would conclude.
    ///
    /// It is a FIELD and not a sixth [`ScriptOutcome`], on the same reasoning one
    /// axis over from why `recovery` is not a fifth [`FaultClass`]:
    /// committed-or-not-known is a PROPERTY of a run, not a KIND of outcome. A
    /// sixth word would break every consumer matching the closed five.
    ///
    /// The actionable class rides `fault.recovery` — `resync`, because a splice
    /// on the wire is the daemon's to finish, so re-read and never resend; or
    /// `retry` under `--dry`, because a rehearsal writes nothing and could not
    /// have committed.
    #[serde(default, skip_serializing_if = "is_false")]
    pub commit_unknown: bool,
    /// Always present, whatever the outcome.
    pub telemetry: ScriptTelemetry,
}

/// Omit `commit_unknown` unless it is true: the field is a marker, and a `false`
/// on every ordinary trace would be noise a consumer has to read past.
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde predicate signature"
)]
fn is_false(value: &bool) -> bool {
    !*value
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

        // Expansion rows open the trace: they happened at entry, before the
        // first read, and the face renders them in that order.
        let mut trace: Vec<TraceEntry> = eval
            .recording
            .expansions
            .iter()
            .cloned()
            .map(TraceEntry::Expanded)
            .collect();
        trace.extend(eval.recording.reads.iter().map(|read| {
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
            }));
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
            // Only a SUCCESSFUL evaluation's namespace is a result; a failed
            // one carries no facts to begin with (`ScriptFacts` exists on `Ok`
            // alone).
            bindings: eval
                .outcome
                .as_ref()
                .map(|facts| facts.bindings.clone())
                .unwrap_or_default(),
            // Over the armed rows as they stand here — after the consumer plane
            // threaded each row's CAS token, which is the same list the commit
            // sends as `plan_edits[]`. Hashing the pre-threading rows would
            // publish a digest for a set that never goes on any wire.
            armed_digest: (!eval.armed.is_empty()).then(|| {
                crate::digest::armed_digest(&crate::digest::ArmedRow::of_all(&eval.armed))
            }),
            commit_unknown,
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
            // Zero evaluation bound zero names.
            bindings: BTreeMap::new(),
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

    /// The armed entries, in arm order — the face's armed block. Live-mode
    /// `wrote`/`ran` entries are acts, not arms — never here.
    pub fn armed_entries(&self) -> impl Iterator<Item = &ArmedEntry> {
        self.trace.iter().filter_map(|entry| match entry {
            TraceEntry::Armed(armed) => Some(armed),
            TraceEntry::Expanded(_)
            | TraceEntry::Read(_)
            | TraceEntry::Echo(_)
            | TraceEntry::Wrote(_)
            | TraceEntry::Ran(_) => None,
        })
    }
}

/// Classify a kernel error into the closed fault taxonomy. `reason` is the
/// kernel's own `Display`, carried verbatim: the refusal wordings are already
/// the face's (golden scenario 2a), so re-phrasing them here would fork the
/// text in two places. The one exception is the runtime fault (golden
/// scenario 4): the shared `Display` frames it as a rules-plane fact
/// (`rule 'script' evaluation error`), but the script entry is not a rule —
/// its face opens with the fault label and the faulting line, with the
/// kernel's message verbatim after the label (defect-ledger DIV-1).
fn fault_of(error: &EvalError) -> ScriptFault {
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
        // The script entry has no hook, so it never raises `MissingEntry`; if a
        // future entry does, it is an evaluation failure like any other.
        EvalError::MissingEntry { .. } => (FaultClass::Runtime, None),
        EvalError::Budget { .. }
        | EvalError::SourceTooLarge { .. }
        | EvalError::ReadBudget { .. }
        | EvalError::RunBudget { .. } => (FaultClass::Budget, None),
        EvalError::ArmedBudget { line, .. } => (FaultClass::Budget, Some(*line)),
    };
    ScriptFault {
        line,
        class,
        // No frame crossed the wire for an arm-time refusal, so there is no §8
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
