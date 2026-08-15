//! Runner (U7) — composition + cascade loop control only.
//! Resolves the addressed task, contracts, and authority; dispatches on fence
//! language; threads the shared executor; records generation outcomes.
//! Does not own write logic, process supervision, or report formatting.
//!
//! `declaring_root` and `timeout` are injected by the caller (the only holder
//! of the convention ladder and wall-clock ceiling). `declaring_root` is
//! `None` when no convention ceiling is in force.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use effects::{ChangeEvent, Domain, Effect, EvalError, EvalLimits, Rule};

use crate::address::{self, AddressError};
use crate::caps::{self, Authority, CapsError};
use crate::contracts::{self, ContractError, ContractViolation};
use crate::dispatch_bash::{self, BashDispatch, BashError, BashOutcome};
use crate::dispatch_starlark::{self, DispatchError, DispatchOutcome, StarlarkDispatch};
use crate::executor::{self, Applied, ApplyRequest, ExecError, ReceiptAddr};
use crate::fence::{GuaranteeClass, TaskLanguage};

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
    /// Fatal opt-in for the pre-exec divergence (card run-preexec-severity):
    /// `false` reports a foreign write landing between the phase-1 commit
    /// and the bracket opening (the exec runs, stdout stands); `true`
    /// restores the hard refusal. Unused on the starlark path.
    pub fatal_preexec: bool,
    /// Bash artifact scratch dir (caller-created; NOT the cwd since U16;
    /// unused on the starlark path).
    pub scratch: &'a Path,
    /// Bash wall-clock ceiling (#21; unused on the starlark path).
    pub timeout: Duration,
    /// The root whose `MERIDIAN.md` declares the convention ceiling, or `None`
    /// when the ladder answered nothing (`CwdDefault`) — then no ceiling is in
    /// force and deny-by-default is the whole chain.
    ///
    /// Injected, not derived from the [`fs::WorkspaceRoot`] above: one is where
    /// files are read, the other is whether anything is entitled to declare
    /// policy — only the caller holds the ladder's answer.
    pub declaring_root: Option<&'a Path>,
    /// Kernel eval limits — `max_depth` is ALSO the cascade depth cap.
    pub limits: EvalLimits,
    /// Caller-supplied identity (§9, § A.8): threads into the receipt's
    /// `actor` fact. `None` keeps the plane's `run:<task>` self-label — the
    /// CLI entry, its own host, passes `None` and its receipt bytes stand.
    pub actor: Option<&'a str>,
    /// The bash step's working directory. `None` is U16 as written — the
    /// step runs where the process runs (the CLI entry). The wire arm
    /// (§ A.8 amendment to U16) passes the bound workspace root: a daemon
    /// has no meaningful cwd. Unused on the starlark path.
    pub step_cwd: Option<&'a Path>,
    /// The host's frame mint for committed batches (§ A.8 Delta honesty):
    /// threaded through both dispatch paths and the cascade, so every commit
    /// of the run mints its Delta on the host's ring. `None` on the CLI
    /// entry — a separate process with no ring in reach.
    pub delta: Option<&'a dyn executor::DeltaSink>,
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
    /// What the engine may claim about the block's effects — a real capability
    /// resolution on the starlark path, `Unsandboxed` on the bash path.
    pub authority: Authority,
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
            CascadeError::Eval { depth, error } => {
                write!(f, "cascade eval (depth {depth}): {error}")
            }
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
            RunnerError::Root { reason } => write!(f, "corpus root: {reason}"),
            RunnerError::Starlark(e) => write!(f, "dispatch: {e}"),
            RunnerError::Bash(e) => write!(f, "dispatch: {e}"),
            RunnerError::Cascade(e) => write!(f, "cascade: {e}"),
        }
    }
}

impl std::error::Error for RunnerError {}

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
    let (task, authority) = pre_eval(
        root,
        spec.page,
        spec.task,
        &spec.args,
        &spec.env,
        spec.declaring_root,
    )?;
    let name = task.binding.name.clone();

    // 2. Dispatch by fence language (decision #13).
    let outcome = dispatch(root, spec, &task, &name, &authority, live)?;

    // 3. The guarantee label: `hermetic` is the sealed kernel's proof by
    // construction; bash is `unsandboxed` (`docs/laws.md` § Amendment) — the
    // bracket verdict rides the report's out-of-band-delta line instead.
    let guarantee = match &outcome {
        TaskOutcome::Starlark(_) => GuaranteeClass::Hermetic,
        TaskOutcome::Bash(_) => GuaranteeClass::Unsandboxed,
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
        &authority,
        first_event,
    )
    .map_err(RunnerError::Cascade)?;

    Ok(RunReport {
        task: name,
        task_rev: task.task_rev,
        guarantee,
        authority,
        outcome,
        cascade,
        cap_reached,
    })
}

/// The pre-eval chain (U2), ONE owner for both tenses: address → contract →
/// caps, in the order the live run enforces them. [`rehearse`] composes the
/// SAME chain, so a rehearsal refuses exactly where a run would — dry-green
/// predicts live-green by construction (dogfood r2 F2).
fn pre_eval(
    root: &fs::WorkspaceRoot,
    page: &str,
    task: Option<&str>,
    args: &[String],
    env: &BTreeMap<String, String>,
    declaring_root: Option<&Path>,
) -> Result<(address::ResolvedTask, Authority), RunnerError> {
    let doc = address::load_page(root, Path::new(page)).map_err(RunnerError::Address)?;
    let task = address::resolve_task(&doc, task).map_err(RunnerError::Address)?;
    let name = task.binding.name.clone();

    let contract = contracts::contract_for(&doc, &name).map_err(RunnerError::Contract)?;
    contracts::validate(&name, &contract, args, env).map_err(RunnerError::Violation)?;

    let (conventions, _source) =
        caps::load_conventions(declaring_root).map_err(RunnerError::Caps)?;
    let authority = caps::resolve_authority(&doc, &name, task.block.lang, &conventions)
        .map_err(RunnerError::Caps)?;
    Ok((task, authority))
}

/// One dry rehearsal request: the [`RunSpec`] fields a rehearsal consumes —
/// the addressed page/task, its contract inputs, and the §9 identity. No
/// receipt, scratch, timeout, or cwd exists here: nothing executes, nothing
/// lands.
#[derive(Debug)]
pub struct RehearseSpec<'a> {
    /// The page the task lives on (workspace-relative).
    pub page: &'a str,
    /// The task to rehearse; `None` addresses the page's only binding.
    pub task: Option<&'a str>,
    /// Supplied positional args (validated against the declared contract).
    pub args: Vec<String>,
    /// Supplied env (validated against the declared contract).
    pub env: BTreeMap<String, String>,
    /// Caller-supplied invocation id (rides eval provenance only).
    pub invocation_id: &'a str,
    /// Caller-supplied time fact.
    pub now: Option<&'a str>,
    /// The convention-declaring root — same semantics as [`RunSpec`]'s field.
    pub declaring_root: Option<&'a Path>,
    /// Kernel eval limits.
    pub limits: EvalLimits,
    /// Caller-supplied identity (§9, § A.8).
    pub actor: Option<&'a str>,
}

/// What one rehearsal proved: the resolved task facts plus the language leg.
#[derive(Debug)]
pub struct Rehearsal {
    /// The resolved task name.
    pub task: String,
    /// The addressed block's procedure-hash (`node_rev` at address time).
    pub task_rev: String,
    /// The language-split outcome.
    pub outcome: Rehearsed,
}

/// The rehearsal's language legs.
#[derive(Debug)]
pub enum Rehearsed {
    /// Starlark, every gate green: the full declared effect set, its md.*
    /// partition admitted by the block's authority through the executor's own
    /// choke point ([`executor::admit`]). Nothing applied.
    Starlark {
        /// Every effect the block emitted, in emission order — never filtered.
        effects: Vec<Effect>,
    },
    /// Bash rehearses by showing the block — no eval, no descriptors: its
    /// effects only exist by running it, and inventing them would be fiction.
    Bash {
        /// The fence's inner source, verbatim.
        source: String,
    },
}

/// Rehearse one addressed task: run EVERY pre-apply gate the live run
/// enforces — the [`pre_eval`] chain (address → contract → caps), then on the
/// starlark leg the U5 evaluation and the executor's own choke-point
/// admission over the md.* partition — and apply nothing.
///
/// The refusals are the live run's own [`RunnerError`] values, so both doors'
/// existing mappings render a rehearsed refusal byte-identical to the live
/// one: one gate, one grammar, both tenses (dogfood r2 F2).
///
/// # Errors
/// [`RunnerError`] — each stage's typed refusal, exactly as [`run`] answers
/// it. Nothing was applied in any case.
pub fn rehearse(
    root: &fs::WorkspaceRoot,
    spec: &RehearseSpec<'_>,
) -> Result<Rehearsal, RunnerError> {
    let (task, authority) = pre_eval(
        root,
        spec.page,
        spec.task,
        &spec.args,
        &spec.env,
        spec.declaring_root,
    )?;
    let name = task.binding.name.clone();
    match task.block.lang {
        TaskLanguage::Starlark => {
            let root_at_eval =
                fs::domain_snapshot(root)
                    .map(|(_, r)| r)
                    .map_err(|e| RunnerError::Root {
                        reason: e.to_string(),
                    })?;
            let effects = dispatch_starlark::evaluate(&StarlarkDispatch {
                page: spec.page,
                task: &name,
                task_rev: &task.task_rev,
                source: &task.block.source,
                args: spec.args.clone(),
                env: spec.env.clone(),
                invocation_id: spec.invocation_id,
                now: spec.now,
                root_at_eval: &root_at_eval,
                authority: &authority,
                receipt: None,
                takeover: false,
                limits: spec.limits,
                actor: spec.actor,
                // A rehearsal never commits, so no Delta can honestly exist.
                delta: None,
            })
            .map_err(|e| RunnerError::Starlark(DispatchError::Eval(e)))?;
            // The choke point, rehearsal tense: the SAME admission the apply
            // enforces, over the same md.* partition.
            let md: Vec<Effect> = effects
                .iter()
                .filter(|e| e.kind.domain() == Domain::Md)
                .cloned()
                .collect();
            executor::admit(&md, &authority)
                .map_err(|e| RunnerError::Starlark(DispatchError::Exec(e)))?;
            Ok(Rehearsal {
                task: name,
                task_rev: task.task_rev,
                outcome: Rehearsed::Starlark { effects },
            })
        }
        TaskLanguage::Bash => Ok(Rehearsal {
            task: name,
            task_rev: task.task_rev,
            outcome: Rehearsed::Bash {
                source: task.block.source.clone(),
            },
        }),
    }
}

/// Dispatch the addressed block by its fence language (decision #13): the
/// hermetic starlark path or the two-phase bash path, each carried whole.
fn dispatch(
    root: &fs::WorkspaceRoot,
    spec: &RunSpec<'_>,
    task: &address::ResolvedTask,
    name: &str,
    authority: &Authority,
    live: &mut (dyn Write + Send),
) -> Result<TaskOutcome, RunnerError> {
    match task.block.lang {
        TaskLanguage::Starlark => {
            let root_at_eval =
                fs::domain_snapshot(root)
                    .map(|(_, r)| r)
                    .map_err(|e| RunnerError::Root {
                        reason: e.to_string(),
                    })?;
            Ok(TaskOutcome::Starlark(Box::new(
                dispatch_starlark::dispatch(
                    root,
                    &StarlarkDispatch {
                        page: spec.page,
                        task: name,
                        task_rev: &task.task_rev,
                        source: &task.block.source,
                        args: spec.args.clone(),
                        env: spec.env.clone(),
                        invocation_id: spec.invocation_id,
                        now: spec.now,
                        root_at_eval: &root_at_eval,
                        authority,
                        receipt: spec.receipt.clone(),
                        takeover: spec.takeover,
                        limits: spec.limits,
                        actor: spec.actor,
                        delta: spec.delta,
                    },
                )
                .map_err(RunnerError::Starlark)?,
            )))
        }
        TaskLanguage::Bash => Ok(TaskOutcome::Bash(Box::new(
            dispatch_bash::run(
                root,
                &BashDispatch {
                    page: spec.page,
                    task: name,
                    task_rev: &task.task_rev,
                    source: &task.block.source,
                    args: spec.args.clone(),
                    env: spec.env.clone(),
                    invocation_id: spec.invocation_id,
                    now: spec.now,
                    pre_receipt: spec.pre_receipt.clone(),
                    receipt: spec.receipt.clone(),
                    takeover: spec.takeover,
                    fatal_preexec: spec.fatal_preexec,
                    scratch: spec.scratch,
                    timeout: spec.timeout,
                    actor: spec.actor,
                    step_cwd: spec.step_cwd,
                    delta: spec.delta,
                },
                live,
            )
            .map_err(RunnerError::Bash)?,
        ))),
    }
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
    authority: &Authority,
    mut event: Option<ChangeEvent>,
) -> Result<(Vec<Generation>, bool), CascadeError> {
    let mut generations = Vec::new();
    let mut cap_reached = false;
    if rules.is_empty() {
        return Ok((generations, cap_reached));
    }
    while let Some(ev) = event.take() {
        let effects = effects::eval_with_limits(rules, &ev, spec.limits).map_err(|error| {
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
            let live =
                fs::domain_snapshot(root)
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
                        authority,
                        pin_root: &live,
                        live_root: &live,
                        receipt: None,
                        takeover: false,
                        exec: None, // cascade generation: no child exec
                        actor: spec.actor,
                        depth: ev.depth,
                        delta: spec.delta,
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
