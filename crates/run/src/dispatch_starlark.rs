//! The hermetic dispatch path (U5, decision #16): fence=`starlark` →
//! `run(ctx)` in the sealed kernel → md.* into the executor's one batch,
//! `daemon.*`/`proto.*` back to the caller unexecuted (the report's business,
//! never faked, never dropped). Guarantee class: **hermetic** — proof by
//! construction, the kernel cannot reach I/O at all.

use std::collections::BTreeMap;

use effects::{Domain, Effect, EvalError, EvalLimits, Rule, RunCtx, eval_run};
use model::MerkleRoot;

use crate::caps::Authority;
use crate::executor::{self, Applied, ApplyRequest, ExecError, ReceiptAddr};
use crate::fence::GuaranteeClass;

/// One starlark block dispatch: the addressed block's facts plus the
/// caller-supplied identity/roots (§9 — nothing here mints or reads a clock).
#[derive(Debug)]
pub struct StarlarkDispatch<'a> {
    /// The page the task lives on (workspace-relative).
    pub page: &'a str,
    /// The task name (rule id, effect provenance, receipt actor).
    pub task: &'a str,
    /// The addressed block's `node_rev` — the procedure-hash the receipt
    /// attests (from the resolved task; §9 — nothing here mints it).
    pub task_rev: &'a str,
    /// The fence's inner source (a `def run(ctx):` definition).
    pub source: &'a str,
    /// Contract-validated positional args.
    pub args: Vec<String>,
    /// Contract-validated env.
    pub env: BTreeMap<String, String>,
    /// Caller-supplied invocation id.
    pub invocation_id: &'a str,
    /// Caller-supplied time fact.
    pub now: Option<&'a str>,
    /// The corpus root the caller computed at eval time — stamped as Run
    /// provenance AND pinned as the batch's `if_root`.
    pub root_at_eval: &'a MerkleRoot,
    /// The block's resolved authority (the executor's choke input) — always a
    /// real capability grant on this path.
    pub authority: &'a Authority,
    /// Receipt address for the commit.
    pub receipt: Option<ReceiptAddr>,
    /// Decision-#26 explicit foreign-edit takeover.
    pub takeover: bool,
    /// Kernel eval limits.
    pub limits: EvalLimits,
}

/// What one block dispatch produced: the FULL deterministic effect set (the
/// truth `--dry`/`--json` report), what was applied, what the local runner
/// has no executor for, and the block's guarantee class.
#[derive(Debug)]
pub struct DispatchOutcome {
    /// The block's guarantee class (hermetic on this path).
    pub guarantee: GuaranteeClass,
    /// Every effect the block emitted, in emission order — never filtered.
    pub effects: Vec<Effect>,
    /// The executor's commit result for the md.* subset (`None` when the
    /// block emitted no md.* effect — nothing to apply is not a fault).
    pub applied: Option<Applied>,
    /// `daemon.*` / `proto.*` effects — no local executor; the report names
    /// them unexecuted (U9), they are never silently dropped.
    pub unexecuted: Vec<Effect>,
}

/// Why a dispatch failed. Eval faults and executor refusals stay typed and
/// distinct — exit-code mapping is the CLI's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchError {
    /// The kernel refused or the source faulted (typed [`EvalError`]).
    Eval(EvalError),
    /// The executor refused the md.* batch (typed [`ExecError`]) — nothing
    /// was applied.
    Exec(ExecError),
    /// Computing the live corpus root for the apply failed.
    Root { reason: String },
}

impl std::fmt::Display for DispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DispatchError::Eval(e) => write!(f, "eval: {e}"),
            DispatchError::Exec(e) => write!(f, "apply: {e}"),
            DispatchError::Root { reason } => write!(f, "live root: {reason}"),
        }
    }
}

impl std::error::Error for DispatchError {}

/// Evaluate the block hermetically — the pure half of the dispatch, shared by
/// `--dry` (full descriptor truth, nothing applied) and the real run.
///
/// # Errors
/// [`EvalError`] — the typed kernel surface.
pub fn evaluate(d: &StarlarkDispatch<'_>) -> Result<Vec<Effect>, EvalError> {
    let ctx = RunCtx {
        page: d.page.to_owned(),
        task: d.task.to_owned(),
        args: d.args.clone(),
        env: d.env.clone(),
        invocation_id: d.invocation_id.to_owned(),
        root_at_eval: d.root_at_eval.0.clone(),
    };
    eval_run(&Rule::new(d.task, d.source), &ctx, d.limits)
}

/// The full hermetic dispatch: evaluate, split by domain, apply the md.*
/// subset through the executor's one batch. The live root is computed HERE,
/// immediately before the apply — the pin (`root_at_eval`) against it detects
/// out-of-band change across the eval window.
///
/// # Errors
/// [`DispatchError`] — eval fault, executor refusal, or root computation.
pub fn dispatch(
    root: &fs::WorkspaceRoot,
    d: &StarlarkDispatch<'_>,
) -> Result<DispatchOutcome, DispatchError> {
    let effects = evaluate(d).map_err(DispatchError::Eval)?;
    let (md, unexecuted): (Vec<Effect>, Vec<Effect>) = effects
        .iter()
        .cloned()
        .partition(|e| e.kind.domain() == Domain::Md);

    let applied = if md.is_empty() {
        None
    } else {
        let live = fs::domain_snapshot(root)
            .map(|(_, r)| r)
            .map_err(|e| DispatchError::Root {
                reason: e.to_string(),
            })?;
        Some(
            executor::apply(
                root,
                &ApplyRequest {
                    page: d.page,
                    task: d.task,
                    task_rev: d.task_rev,
                    invocation_id: d.invocation_id,
                    now: d.now,
                    effects: &md,
                    authority: d.authority,
                    pin_root: d.root_at_eval,
                    live_root: &live,
                    receipt: d.receipt.clone(),
                    takeover: d.takeover,
                    exec: None, // hermetic: no child process
                    depth: 0,
                },
            )
            .map_err(DispatchError::Exec)?,
        )
    };

    Ok(DispatchOutcome {
        guarantee: GuaranteeClass::Hermetic,
        effects,
        applied,
        unexecuted,
    })
}
