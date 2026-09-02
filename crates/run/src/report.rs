//! Run report (U9) — the ONE renderer of a [`RunReport`] into the human-
//! facing summary. Owns presentation only; never decides outcomes.
//!
//! Renders caps, effects, exec status, detection verdict, and refusals in a
//! stable order so CLI and tests share one surface. Out-of-band delta lines
//! come from [`Detection`], not from the block's own effects.

use serde::Serialize;

use effects::{Domain, Effect};

use crate::caps::{Authority, CapResolution, CapSource};
use crate::dispatch_bash::Phase2;
use crate::exec::ExecStatus;
use crate::record::StdoutRecord;
use crate::runner::{RunReport, TaskOutcome};

/// The run's overall report state (plan §4 U9; S2). Closed set — a run is
/// exactly one of these. Serialized kebab-case (`unexecuted-no-capability`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReportState {
    /// Generation-0 md.* committed through the choke point (nothing withheld,
    /// nothing interrupted).
    Applied,
    /// The block emitted only effects with no local executor
    /// (`daemon.*`/`proto.*`) — surfaced, never run.
    UnexecutedNoCapability,
    /// The cascade reached the depth cap; the kernel suppressed further md.*
    /// (the generic cap-reached fact, never a per-effect listing).
    WithheldDepthCap,
    /// Bash two-phase: phase 1 committed (the pre-exec receipt stands) but
    /// phase 2 refused (nonzero exit / choke refusal) — a partial
    /// run (S2).
    Partial,
    /// The step was interrupted — a wall-clock timeout or a signal (S2/#21),
    /// a DISTINCT state from a clean nonzero exit.
    Interrupted,
}

impl ReportState {
    /// The stable wire/human spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ReportState::Applied => "applied",
            ReportState::UnexecutedNoCapability => "unexecuted-no-capability",
            ReportState::WithheldDepthCap => "withheld-depth-cap",
            ReportState::Partial => "partial",
            ReportState::Interrupted => "interrupted",
        }
    }
}

/// One effect named for the report — its namespaced kind and the domain that
/// decides whether a LOCAL executor exists for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EffectLine {
    /// The namespaced effect kind (`md.set_field`).
    pub kind: String,
    /// The effect's domain (`md` / `daemon` / `proto`).
    pub domain: String,
}

impl EffectLine {
    fn of(effect: &Effect) -> Self {
        Self {
            kind: effect.kind.as_str().to_owned(),
            domain: domain_str(effect.kind.domain()).to_owned(),
        }
    }
}

/// The phase-2 refusal facts (card cli-run-birth-silent-loss): the typed
/// executor refusal, named on both faces — a refused completion commit must
/// never render success-shaped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RefusedReport {
    /// The executor's refusal, in its own words.
    pub error: String,
}

/// The resolved-capability facts the report renders: the effective set, where
/// the grant came from, and every cap a ceiling narrowed away (#23 gate b).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CapsReport {
    /// The caps the block's effects were validated against (md-local only in
    /// S1) — canonical strings.
    pub effective: Vec<String>,
    /// Where the grant came from (`explicit` / `convention:<pattern>` /
    /// `deny-default`).
    pub source: String,
    /// Granted caps a ceiling tightened or dropped — never silent.
    pub narrowed: Vec<String>,
}

impl CapsReport {
    fn of(caps: &CapResolution) -> Self {
        Self {
            effective: caps
                .effective
                .0
                .iter()
                .map(crate::caps::Cap::as_string)
                .collect(),
            source: match &caps.source {
                CapSource::Explicit => "explicit".to_owned(),
                CapSource::Convention(pattern) => format!("convention:{pattern}"),
                CapSource::DenyDefault => "deny-default".to_owned(),
            },
            narrowed: caps
                .narrowed
                .iter()
                .map(crate::caps::Cap::as_string)
                .collect(),
        }
    }
}

/// The whole run report as ONE object — the `--json` payload and the source of
/// the human text (the two never diverge).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    /// The resolved task name.
    pub task: String,
    /// The addressed block's procedure-hash (`node_rev` at address time).
    pub task_rev: String,
    /// The block's guarantee class (`hermetic` / `unsandboxed`). Text renders
    /// it only where positive; `--json` always carries it.
    pub guarantee: String,
    /// The overall report state.
    pub state: ReportState,
    /// md.* effects the choke point APPLIED (kind + domain). A refused
    /// batch's effects are never listed here — they ride [`Report::refused`].
    pub applied: Vec<EffectLine>,
    /// Bash phase 2: the executor refused the completion commit — the
    /// refusal, named (card cli-run-birth-silent-loss). Absent when nothing
    /// was refused.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refused: Option<RefusedReport>,
    /// `daemon.*`/`proto.*` effects with no local executor — surfaced,
    /// never run (`unexecuted-no-capability`).
    pub unexecuted: Vec<EffectLine>,
    /// The capability partition (effective + source + narrowed) — ABSENT for
    /// an unsandboxed bash run, where `null` or `[]` would still be an answer
    /// to a question the engine cannot answer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caps: Option<CapsReport>,
    /// Bash only: `undeclared`. The block's effects are whatever the shell
    /// did, and the engine describes rather than claims.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effects: Option<String>,
    /// The cascade reached the depth cap (the generic cap-reached fact).
    pub cap_reached: bool,
    /// The pre-exec divergence line (bash only; card run-preexec-severity):
    /// a foreign write between the phase-1 commit and the bracket opening,
    /// reported with the same trust posture as the in-window delta. `"none"`
    /// when the observed pre-exec tree folded to the computed root; absent
    /// on the starlark path (no exec window, no pre-exec gap).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pre_exec_delta: Option<String>,
    /// The exec-window out-of-band delta line (S1 hermetic: no exec window).
    pub out_of_band_delta: String,
    /// Bash exec facts — ADDITIVE, `RunReport`-sourced, NEVER receipt-sourced
    /// (a failed run has no completion receipt, but its exec facts must
    /// still surface). Absent (`None`, omitted from `--json`) on the
    /// starlark path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exec: Option<ExecReport>,
}

/// One bash step's exec facts for the report surface (u9-gate forward note):
/// how the step ended plus the out-of-tree stdout record address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExecReport {
    /// How the step ended: `exited` / `signaled` / `timed-out` (#21 keeps
    /// timeout distinct).
    pub status: String,
    /// The exit code (`exited` only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// The signal number (`signaled` only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<i32>,
    /// The enforced ceiling in seconds (`timed-out` only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    /// The sealed stdout record (invocation id, log address, sha256, size).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout: Option<StdoutRecord>,
    /// The typed record failure when the log could not seal — the run still
    /// reports; the orphan lint reconciles.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout_error: Option<String>,
}

impl ExecReport {
    fn of(outcome: &TaskOutcome) -> Option<Self> {
        let TaskOutcome::Bash(o) = outcome else {
            return None;
        };
        let (status, exit_code, signal, timeout_secs) = match &o.status {
            ExecStatus::Exited { code } => ("exited", Some(*code), None, None),
            ExecStatus::Signaled { signal } => ("signaled", None, Some(*signal), None),
            ExecStatus::TimedOut { limit } => ("timed-out", None, None, Some(limit.as_secs())),
        };
        let (stdout, stdout_error) = match &o.stdout {
            Ok(record) => (Some(record.clone()), None),
            Err(e) => (None, Some(e.to_string())),
        };
        Some(Self {
            status: status.to_owned(),
            exit_code,
            signal,
            timeout_secs,
            stdout,
            stdout_error,
        })
    }
}

/// Build the report from a runner [`RunReport`] — the one place the renderer
/// reads the outcome; text and json come off the same struct.
#[must_use]
pub fn render(report: &RunReport) -> Report {
    let (applied, unexecuted) = partition_effects(report);
    Report {
        task: report.task.clone(),
        task_rev: report.task_rev.clone(),
        guarantee: report.guarantee.as_str().to_owned(),
        state: classify(report, &applied, &unexecuted),
        applied,
        refused: refused_report(report),
        unexecuted,
        caps: report.authority.capabilities().map(CapsReport::of),
        effects: match report.authority {
            Authority::Capabilities(_) => None,
            Authority::Unsandboxed => Some(crate::caps::UNDECLARED_EFFECTS.to_owned()),
        },
        cap_reached: report.cap_reached,
        pre_exec_delta: pre_exec_delta(report),
        out_of_band_delta: out_of_band_delta(report),
        exec: ExecReport::of(&report.outcome),
    }
}

impl Report {
    /// The `--json` one-object payload.
    ///
    /// # Errors
    /// [`serde_json::Error`] only on an unserializable value (unreachable for
    /// this all-owned struct).
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// The human report — the SAME facts as [`Report::to_json`], one per line.
    #[must_use]
    pub fn to_text(&self) -> String {
        use std::fmt::Write as _;
        let mut s = String::new();
        // A guarantee word renders only where it is POSITIVE (`hermetic`).
        // There is no sandbox, so `(unsandboxed)` names an alternative that
        // does not exist (`docs/laws.md` § Amendment); `effects: undeclared` below
        // carries the bash fact. The `--json` `guarantee` field is unchanged.
        if self.guarantee == crate::fence::GuaranteeClass::Unsandboxed.as_str() {
            let _ = writeln!(s, "task: {}", self.task);
        } else {
            let _ = writeln!(s, "task: {} ({})", self.task, self.guarantee);
        }
        let _ = writeln!(s, "procedure: {}", self.task_rev);
        let _ = writeln!(s, "state: {}", self.state.as_str());
        let _ = writeln!(s, "applied: {} md descriptor(s)", self.applied.len());
        for e in &self.applied {
            let _ = writeln!(s, "  - {}", e.kind);
        }
        if let Some(refused) = &self.refused {
            let _ = writeln!(s, "refused: {}", refused.error);
        }
        if !self.unexecuted.is_empty() {
            let _ = writeln!(
                s,
                "unexecuted (no local capability): {}",
                self.unexecuted.len()
            );
            for e in &self.unexecuted {
                let _ = writeln!(s, "  - {} [{}]", e.kind, e.domain);
            }
        }
        if let Some(caps) = &self.caps {
            let _ = writeln!(
                s,
                "caps: {} (source: {})",
                if caps.effective.is_empty() {
                    "none (read-only)".to_owned()
                } else {
                    caps.effective.join(", ")
                },
                caps.source
            );
            if !caps.narrowed.is_empty() {
                let _ = writeln!(s, "narrowed by ceiling: {}", caps.narrowed.join(", "));
            }
        }
        if let Some(effects) = &self.effects {
            let _ = writeln!(s, "effects: {effects}");
        }
        if self.cap_reached {
            let _ = writeln!(s, "cascade: depth cap reached — capped md.* not applied");
        }
        if let Some(exec) = &self.exec {
            let ended = match (&exec.exit_code, &exec.signal, &exec.timeout_secs) {
                (Some(code), _, _) => format!("exited {code}"),
                (_, Some(signal), _) => format!("signaled {signal}"),
                (_, _, Some(secs)) => format!("timed out after {secs}s"),
                _ => exec.status.clone(),
            };
            let _ = writeln!(s, "exec: {ended}");
            match (&exec.stdout, &exec.stdout_error) {
                (Some(record), _) => {
                    let _ = writeln!(
                        s,
                        "stdout: {} ({} byte(s), sha256 {})",
                        record.log, record.bytes, record.sha256
                    );
                }
                (None, Some(error)) => {
                    let _ = writeln!(s, "stdout: record failed — {error}");
                }
                (None, None) => {}
            }
        }
        if let Some(pre) = &self.pre_exec_delta {
            let _ = writeln!(s, "pre-exec delta: {pre}");
        }
        let _ = writeln!(s, "out-of-band delta: {}", self.out_of_band_delta);
        s
    }
}

/// Split the outcome's (and cascade's) effects into md.* applied vs the
/// daemon.*/proto.* surfaced-unexecuted set. The report reads the outcome's
/// effect lists directly — the executor's own count is not the surface here.
fn partition_effects(report: &RunReport) -> (Vec<EffectLine>, Vec<EffectLine>) {
    let mut applied = Vec::new();
    let mut unexecuted = Vec::new();
    let all = outcome_effects(&report.outcome)
        .iter()
        .chain(report.cascade.iter().flat_map(|g| g.effects.iter()));
    for effect in all {
        if effect.kind.domain() == Domain::Md {
            applied.push(EffectLine::of(effect));
        } else {
            unexecuted.push(EffectLine::of(effect));
        }
    }
    (applied, unexecuted)
}

/// Generation 0's LANDED effects, whole. Bash has no effect channel — its
/// slice is empty on every phase-2 state; the report state carries any
/// refusal.
fn outcome_effects(outcome: &TaskOutcome) -> &[Effect] {
    match outcome {
        TaskOutcome::Starlark(o) => &o.effects,
        TaskOutcome::Bash(_) => &[],
    }
}

/// The phase-2 refusal half of the split: the executor's refusal, for
/// [`Report::refused`]. `None` everywhere except [`Phase2::RefusedExec`].
fn refused_report(report: &RunReport) -> Option<RefusedReport> {
    let TaskOutcome::Bash(o) = &report.outcome else {
        return None;
    };
    let Phase2::RefusedExec { error } = &o.phase2 else {
        return None;
    };
    Some(RefusedReport {
        error: error.to_string(),
    })
}

/// Classify the run into its single [`ReportState`] — the bash failure matrix
/// (S2/#21) outranks the depth cap, which outranks the applied/unexecuted
/// split; a run that only surfaced non-md effects is `unexecuted-no-capability`.
fn classify(report: &RunReport, applied: &[EffectLine], unexecuted: &[EffectLine]) -> ReportState {
    if let TaskOutcome::Bash(o) = &report.outcome {
        match &o.phase2 {
            Phase2::RefusedTimeout | Phase2::RefusedSignaled => {
                return ReportState::Interrupted;
            }
            Phase2::RefusedExecFailed { .. }
            | Phase2::RefusedDetection
            | Phase2::RefusedExec { .. } => {
                // Phase 1 committed (the pre-exec receipt stands) → partial;
                // without a pre-exec receipt there is no committed phase-1
                // state, so the run is interrupted before anything landed.
                // A detection refusal is the same shape: the exec ran, the
                // completion commit was withheld — the out-of-band-delta
                // line names it.
                return if o.pre_receipt_line.is_some() {
                    ReportState::Partial
                } else {
                    ReportState::Interrupted
                };
            }
            Phase2::Applied { .. } => {}
        }
    }
    if report.cap_reached {
        return ReportState::WithheldDepthCap;
    }
    if applied.is_empty() && !unexecuted.is_empty() {
        return ReportState::UnexecutedNoCapability;
    }
    ReportState::Applied
}

/// The pre-exec divergence line — the report half of the fatal-opt-in split
/// (card run-preexec-severity). Reads [`dispatch_bash::BashOutcome::pre_exec`];
/// the fatal path never reaches a report (the dispatch refuses pre-exec).
fn pre_exec_delta(report: &RunReport) -> Option<String> {
    match &report.outcome {
        TaskOutcome::Starlark(_) => None,
        TaskOutcome::Bash(o) => Some(match &o.pre_exec {
            None => "none".to_owned(),
            Some(div) => div.to_string(),
        }),
    }
}

/// The exec-window out-of-band-delta line. The bracket is U6b's and rides the
/// bash path; a hermetic run has NO exec window, so the delta is vacuously
/// none. (When the `detected` label activates, this reads the bracket verdict.)
fn out_of_band_delta(report: &RunReport) -> String {
    match &report.outcome {
        TaskOutcome::Starlark(_) => "none (hermetic — no exec window)".to_owned(),
        // The U6b bracket verdict rides EVERY bash outcome (orthogonal to how
        // the exec ended): a clean window is no delta; anything else names it.
        TaskOutcome::Bash(o) => {
            if o.detection.is_clean() {
                "none".to_owned()
            } else {
                o.detection.to_string()
            }
        }
    }
}

/// The report spelling of a [`Domain`] — the same tokens the caps namespace
/// uses (`md` / `daemon` / `proto`).
fn domain_str(domain: Domain) -> &'static str {
    match domain {
        Domain::Md => "md",
        Domain::Daemon => "daemon",
        Domain::Proto => "proto",
    }
}
