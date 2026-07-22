//! The runner (U7) — composition + cascade loop control, NOTHING else
//! (plan §4 U7; decisions #5/#16/#23). The pre-eval chain is U2's, the
//! dispatch paths are U5/U6a's, synthesis and the one write path are U4's;
//! this module only strings them together and drives the generation loop:
//!
//! ```text
//! load_page → resolve_task → contract validate → caps resolve
//!   → GUARANTEE LABEL (#23 gate — refuse before anything commits)
//!   → dispatch by fence (starlark hermetic | bash two-phase)
//!   → cascade: event → eval(&[Rule]) → md.* through the executor → event…
//! ```
//!
//! # Cascade (decision #5)
//! The runner takes `&[Rule]` — EMPTY in S1, so generation 0 is the
//! degenerate whole run (vacuous stop: no rules, no generation records).
//! Tests inject synthetic `on_change` rules to exercise the loop against the
//! REAL executor path. Depth semantics are the kernel's: at
//! `event.depth >= limits.max_depth` the kernel suppresses cascading `md.*`
//! (terminal `daemon.*`/`proto.*` survive) — the runner records the
//! cap-reached fact for the report's generic cap line (U9) and stops.
//! Cascade generations ride the block's resolved caps (the run's granted
//! authority is the ceiling — a cascade can never exceed it) and carry no
//! receipt (resident-cascade receipt policy is phase-2 scope).
//!
//! # The guarantee labeler (#23, SHARPENED)
//! Ruling 4: every block's claim ships scoped, per block. `hermetic` is
//! starlark's by construction. `detected` may be emitted ONLY when BOTH
//! hold: (a) the U6b detection bracket is wired through `dispatch_bash`
//! (`BashOutcome.detection` gates phase 2), and (b) the U9 report renders
//! the `narrowed[]` caps facts. Until both land, the labeler emits
//! hermetic-or-refuses — a bash block REFUSES AT THE LABEL, before any
//! receipt or commit, rather than run under a guarantee nobody can state.
//! Activation is the one-line [`DETECTED_LABEL_ACTIVE`] flip, landed LAST.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use rules::{ChangeEvent, Domain, Effect, EvalError, EvalLimits, Rule};

use crate::address::{self, AddressError};
use crate::caps::{self, CapResolution, CapSet, CapsError};
use crate::contracts::{self, ContractError, ContractViolation};
use crate::dispatch_bash::{self, BashDispatch, BashError, BashOutcome};
use crate::dispatch_starlark::{self, DispatchError, DispatchOutcome, StarlarkDispatch};
use crate::executor::{self, Applied, ApplyRequest, ExecError, ReceiptAddr};
use crate::fence::{GuaranteeClass, TaskLanguage};

/// The #23 activation switch. `false` until BOTH gate conditions are landed
/// and verified (see the module docs) — flipping it is the LAST commit of
/// the label chain, never a side effect of another change.
pub const DETECTED_LABEL_ACTIVE: bool = false;

/// One run request: the addressed page/task plus the caller-supplied
/// identity, receipts, and bash bracket inputs (§9 — nothing here mints or
/// reads a clock).
#[derive(Debug)]
pub struct RunSpec<'a> {
    /// The page the task lives on (workspace-relative).
    pub page: &'a str,
    /// The task to run; `None` addresses the page's only binding.
    pub task: Option<&'a str>,
    /// Supplied positional args (validated against the declared contract).
    pub args: Vec<String>,
    /// Supplied env (validated against the declared contract).
    pub env: BTreeMap<String, String>,
    /// Caller-supplied invocation id.
    pub invocation_id: &'a str,
    /// Caller-supplied time fact.
    pub now: Option<&'a str>,
    /// Generation-0 completion receipt address.
    pub receipt: Option<ReceiptAddr>,
    /// Bash pre-exec receipt address (S2; unused on the starlark path).
    pub pre_receipt: Option<ReceiptAddr>,
    /// Decision-#26 explicit foreign-edit takeover.
    pub takeover: bool,
    /// Bash scratch cwd (caller-created; unused on the starlark path).
    pub scratch: &'a Path,
    /// Bash wall-clock ceiling (#21; unused on the starlark path).
    pub timeout: Duration,
    /// Kernel eval limits — `max_depth` is ALSO the cascade depth cap.
    pub limits: EvalLimits,
}

/// What one full run produced — the report's (U9) single input.
#[derive(Debug)]
pub struct RunReport {
    /// The resolved task name.
    pub task: String,
    /// The addressed block's procedure-hash (`node_rev` at address time).
    pub task_rev: String,
    /// The block's guarantee class (#23-gated).
    pub guarantee: GuaranteeClass,
    /// The caps resolution — source and `narrowed[]` facts for the report.
    pub caps: CapResolution,
    /// Generation 0: the dispatch outcome, whole.
    pub outcome: TaskOutcome,
    /// Cascade generations 1.. (empty in S1 — `&[Rule]` is empty).
    pub cascade: Vec<Generation>,
    /// The cascade stopped at the depth cap (the kernel suppressed md.*) —
    /// the report's generic cap-reached line.
    pub cap_reached: bool,
}

/// Generation 0 by dispatch path, carried whole (exec facts are never
/// flattened away — the report reads them directly).
#[derive(Debug)]
pub enum TaskOutcome {
    /// The hermetic path (U5). Boxed like its sibling — both outcomes carry
    /// bulky exec/effect facts, and the report reads them by reference.
    Starlark(Box<DispatchOutcome>),
    /// The two-phase bash path (U6a).
    Bash(Box<BashOutcome>),
}

/// One cascade generation: what the rules emitted over the previous
/// generation's event, and what the executor committed of it.
#[derive(Debug)]
pub struct Generation {
    /// The depth of the event this generation evaluated.
    pub depth: u32,
    /// Every effect the rules emitted, in emission order — never filtered
    /// (md.* already kernel-suppressed at the cap).
    pub effects: Vec<Effect>,
    /// The executor's commit of the md.* subset (`None` when none).
    pub applied: Option<Applied>,
    /// `daemon.*` / `proto.*` — no local executor, surfaced unexecuted.
    pub unexecuted: Vec<Effect>,
}

/// The #23 label refusal — the block does not run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabelRefusal {
    /// A bash block while the `detected` label is not active: the detection
    /// bracket and the narrowed-caps report are not both landed, and the
    /// runner will not execute under a guarantee nobody can state.
    DetectionNotActive {
        /// The refused task.
        task: String,
    },
}

impl std::fmt::Display for LabelRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LabelRefusal::DetectionNotActive { task } => write!(
                f,
                "task '{task}' is a bash block, and the 'detected' guarantee label is not active yet (U6b bracket + U9 narrowed-caps rendering must both land) — refusing to run unlabeled"
            ),
        }
    }
}

impl std::error::Error for LabelRefusal {}

/// A cascade fault at a specific generation. Prior generations stand
/// committed (ruling 2: no rollback); the report names the depth.
#[derive(Debug)]
pub enum CascadeError {
    /// The kernel refused or a rule faulted.
    Eval {
        /// The event depth being evaluated.
        depth: u32,
        /// The typed kernel fault.
        error: EvalError,
    },
    /// The executor refused the generation's md.* batch.
    Apply {
        /// The event depth being applied.
        depth: u32,
        /// The typed executor refusal.
        error: ExecError,
    },
    /// Computing the generation's corpus root failed.
    Root {
        /// The event depth.
        depth: u32,
        /// The underlying failure.
        reason: String,
    },
}

impl std::fmt::Display for CascadeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CascadeError::Eval { depth, error } => write!(f, "cascade eval (depth {depth}): {error}"),
            CascadeError::Apply { depth, error } => {
                write!(f, "cascade apply (depth {depth}): {error}")
            }
            CascadeError::Root { depth, reason } => {
                write!(f, "cascade root (depth {depth}): {reason}")
            }
        }
    }
}

impl std::error::Error for CascadeError {}

/// Why the run refused or faulted — each stage's typed error, distinct
/// (exit-code mapping is the CLI's).
#[derive(Debug)]
pub enum RunnerError {
    /// Addressing refused (page load, binding, anchor, fence class).
    Address(AddressError),
    /// The declared contract itself is malformed.
    Contract(ContractError),
    /// The supplied args/env do not satisfy the declared contract.
    Violation(ContractViolation),
    /// Capability resolution refused.
    Caps(CapsError),
    /// The #23 label gate refused — nothing ran, nothing committed.
    Label(LabelRefusal),
    /// Computing the pre-dispatch corpus root failed.
    Root {
        /// The underlying failure.
        reason: String,
    },
    /// The hermetic dispatch failed (typed eval/exec fault).
    Starlark(DispatchError),
    /// The bash dispatch failed before exec facts existed.
    Bash(BashError),
    /// A cascade generation faulted; generation 0 (and any prior cascade
    /// generation) stands committed.
    Cascade(CascadeError),
}

impl std::fmt::Display for RunnerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunnerError::Address(e) => write!(f, "address: {e}"),
            RunnerError::Contract(e) => write!(f, "contract: {e}"),
            RunnerError::Violation(e) => write!(f, "contract: {e}"),
            RunnerError::Caps(e) => write!(f, "caps: {e}"),
            RunnerError::Label(e) => write!(f, "label: {e}"),
            RunnerError::Root { reason } => write!(f, "corpus root: {reason}"),
            RunnerError::Starlark(e) => write!(f, "dispatch: {e}"),
            RunnerError::Bash(e) => write!(f, "dispatch: {e}"),
            RunnerError::Cascade(e) => write!(f, "cascade: {e}"),
        }
    }
}

impl std::error::Error for RunnerError {}

/// The #23-gated guarantee labeler. Starlark is hermetic by construction;
/// bash labels `detected` ONLY once [`DETECTED_LABEL_ACTIVE`] — until then a
/// bash block refuses HERE, before any receipt or commit.
///
/// # Errors
/// [`LabelRefusal`] — the block must not run.
pub fn label(task: &str, lang: TaskLanguage) -> Result<GuaranteeClass, LabelRefusal> {
    match lang {
        TaskLanguage::Starlark => Ok(GuaranteeClass::Hermetic),
        TaskLanguage::Bash if DETECTED_LABEL_ACTIVE => Ok(GuaranteeClass::Detected),
        TaskLanguage::Bash => Err(LabelRefusal::DetectionNotActive {
            task: task.to_owned(),
        }),
    }
}

/// Run one addressed task end to end: compose the pre-eval chain, dispatch
/// by fence language, then drive the cascade over `rules` (EMPTY in S1 —
/// the vacuous gen-0 stop). `live` receives the bash stdout stream (ignored
/// on the starlark path).
///
/// # Errors
/// [`RunnerError`] — each stage's typed refusal. Pre-dispatch errors
/// committed nothing; a cascade error leaves prior generations standing
/// (ruling 2 — no rollback).
pub fn run(
    root: &fs::WorkspaceRoot,
    spec: &RunSpec<'_>,
    rules: &[Rule],
    live: &mut (dyn Write + Send),
) -> Result<RunReport, RunnerError> {
    // 1. Pre-eval resolution (U2): address → contract → caps.
    let doc = address::load_page(root, Path::new(spec.page)).map_err(RunnerError::Address)?;
    let task = address::resolve_task(&doc, spec.task).map_err(RunnerError::Address)?;
    let name = task.binding.name.clone();

    let contract = contracts::contract_for(&doc, &name).map_err(RunnerError::Contract)?;
    contracts::validate(&name, &contract, &spec.args, &spec.env)
        .map_err(RunnerError::Violation)?;

    let explicit = caps::explicit_caps(&doc, &name).map_err(RunnerError::Caps)?;
    let conventions = caps::load_conventions(&root.0).map_err(RunnerError::Caps)?;
    let resolution = caps::resolve_caps(&name, task.block.lang, explicit.as_ref(), &conventions)
        .map_err(RunnerError::Caps)?;

    // 2. The label gate (#23) — BEFORE anything can commit.
    let guarantee = label(&name, task.block.lang).map_err(RunnerError::Label)?;

    // 3. Dispatch by fence language (decision #13).
    let outcome = match task.block.lang {
        TaskLanguage::Starlark => {
            let root_at_eval = fs::domain_snapshot(root)
                .map(|(_, r)| r)
                .map_err(|e| RunnerError::Root {
                    reason: e.to_string(),
                })?;
            TaskOutcome::Starlark(Box::new(
                dispatch_starlark::dispatch(
                    root,
                    &StarlarkDispatch {
                        page: spec.page,
                        task: &name,
                        task_rev: &task.task_rev,
                        source: &task.block.source,
                        args: spec.args.clone(),
                        env: spec.env.clone(),
                        invocation_id: spec.invocation_id,
                        now: spec.now,
                        root_at_eval: &root_at_eval,
                        caps: &resolution.effective,
                        receipt: spec.receipt.clone(),
                        takeover: spec.takeover,
                        limits: spec.limits,
                    },
                )
                .map_err(RunnerError::Starlark)?,
            ))
        }
        TaskLanguage::Bash => TaskOutcome::Bash(Box::new(
            dispatch_bash::run(
                root,
                &BashDispatch {
                    page: spec.page,
                    task: &name,
                    task_rev: &task.task_rev,
                    source: &task.block.source,
                    args: spec.args.clone(),
                    env: spec.env.clone(),
                    invocation_id: spec.invocation_id,
                    now: spec.now,
                    caps: &resolution.effective,
                    pre_receipt: spec.pre_receipt.clone(),
                    receipt: spec.receipt.clone(),
                    takeover: spec.takeover,
                    scratch: spec.scratch,
                    timeout: spec.timeout,
                },
                live,
            )
            .map_err(RunnerError::Bash)?,
        )),
    };

    // 4. The cascade loop over generation 0's event.
    let first_event = match &outcome {
        TaskOutcome::Starlark(o) => o.applied.as_ref().and_then(|a| a.event.clone()),
        TaskOutcome::Bash(o) => match &o.phase2 {
            dispatch_bash::Phase2::Applied { applied, .. } => {
                applied.as_ref().and_then(|a| a.event.clone())
            }
            _ => None,
        },
    };
    let (cascade, cap_reached) = cascade(
        root,
        spec,
        rules,
        &name,
        &task.task_rev,
        &resolution.effective,
        first_event,
    )
    .map_err(RunnerError::Cascade)?;

    Ok(RunReport {
        task: name,
        task_rev: task.task_rev,
        guarantee,
        caps: resolution,
        outcome,
        cascade,
        cap_reached,
    })
}

/// Drive the cascade: evaluate `rules` over each generation's event, apply
/// the md.* subset through the executor's one batch, and follow the
/// synthesized event until the chain goes quiet or the depth cap suppresses
/// it. Empty `rules` is the S1 vacuous stop — zero generations, no eval.
fn cascade(
    root: &fs::WorkspaceRoot,
    spec: &RunSpec<'_>,
    rules: &[Rule],
    task: &str,
    task_rev: &str,
    caps: &CapSet,
    mut event: Option<ChangeEvent>,
) -> Result<(Vec<Generation>, bool), CascadeError> {
    let mut generations = Vec::new();
    let mut cap_reached = false;
    if rules.is_empty() {
        return Ok((generations, cap_reached));
    }
    while let Some(ev) = event.take() {
        let effects = rules::eval_with_limits(rules, &ev, spec.limits).map_err(|error| {
            CascadeError::Eval {
                depth: ev.depth,
                error,
            }
        })?;
        if ev.depth >= spec.limits.max_depth {
            // The kernel suppressed this generation's cascading md.* —
            // terminal daemon/proto survive below; the loop cannot continue.
            cap_reached = true;
        }
        if effects.is_empty() {
            break;
        }
        let (md, unexecuted): (Vec<Effect>, Vec<Effect>) = effects
            .iter()
            .cloned()
            .partition(|e| e.kind.domain() == Domain::Md);

        let applied = if md.is_empty() {
            None
        } else {
            let live = fs::domain_snapshot(root)
                .map(|(_, r)| r)
                .map_err(|e| CascadeError::Root {
                    depth: ev.depth,
                    reason: e.to_string(),
                })?;
            Some(
                executor::apply(
                    root,
                    &ApplyRequest {
                        page: spec.page,
                        task,
                        task_rev,
                        invocation_id: spec.invocation_id,
                        now: spec.now,
                        effects: &md,
                        caps,
                        pin_root: &live,
                        live_root: &live,
                        receipt: None,
                        takeover: false,
                        depth: ev.depth,
                    },
                )
                .map_err(|error| CascadeError::Apply {
                    depth: ev.depth,
                    error,
                })?,
            )
        };

        event = applied.as_ref().and_then(|a| a.event.clone());
        generations.push(Generation {
            depth: ev.depth,
            effects,
            applied,
            unexecuted,
        });
    }
    Ok((generations, cap_reached))
}
