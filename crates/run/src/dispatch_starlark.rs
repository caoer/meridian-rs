//! The hermetic dispatch path (U5, decision #16): fence=`starlark` →
//! `run(ctx)` in the sealed kernel → md.* into the executor's one batch,
//! `daemon.*`/`proto.*` back to the caller unexecuted (the report's business,
//! never faked, never dropped). Guarantee class: **hermetic** — proof by
//! construction, the kernel cannot reach I/O at all.

use std::collections::BTreeMap;

use effects::{Domain, Effect, EvalError, EvalLimits, Provenance, Rule, RunCtx, eval_run};
use model::MerkleRoot;

use crate::caps::Authority;
use crate::executor::{self, Applied, ApplyRequest, ExecError, ReceiptAddr};
use crate::fence::GuaranteeClass;

/// One starlark block dispatch: the addressed block's facts plus the
/// caller-supplied identity (§9 — nothing here mints or reads a clock). It
/// carries no root: the corpus observation is this module's own, taken after
/// the eval and only when there is something to stamp
/// ([`observe_if_emitted`]).
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
    /// The block's resolved authority (the executor's choke input) — always a
    /// real capability grant on this path.
    pub authority: &'a Authority,
    /// Receipt address for the commit.
    pub receipt: Option<ReceiptAddr>,
    /// Kernel eval limits.
    pub limits: EvalLimits,
    /// Caller-supplied identity (§9, § A.8): threads into the receipt actor.
    pub actor: Option<&'a str>,
    /// The host's frame mint for committed batches (§ A.8 Delta honesty);
    /// `None` on the CLI entry and on evaluate-only callers.
    pub delta: Option<&'a dyn executor::DeltaSink>,
    /// § A.2.1 passthrough for `md.create` births (`ctx.fields`, verbatim).
    pub fields: &'a BTreeMap<String, String>,
    /// The workspace ring for door-committed births; `None` on the CLI entry.
    pub birth_seq: Option<&'a dyn wire_serve::seq::SeqSink>,
    /// The caller's ambient directory for bare birth targets
    /// (md-create-ambient-paths, shape (c)); `None` on the CLI entry.
    pub ambient: Option<&'a str>,
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
    /// The post-eval corpus fold failed. Only reachable when the block
    /// emitted — an effect-free block takes no fold and cannot fail here.
    Root {
        /// The underlying I/O failure, rendered.
        reason: String,
    },
}

impl std::fmt::Display for DispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DispatchError::Eval(e) => write!(f, "eval: {e}"),
            DispatchError::Exec(e) => write!(f, "apply: {e}"),
            DispatchError::Root { reason } => write!(f, "corpus root: {reason}"),
        }
    }
}

impl std::error::Error for DispatchError {}

/// Evaluate the block hermetically — the pure half of the dispatch, shared by
/// `--dry` (full descriptor truth, nothing applied) and the real run.
///
/// **The effects come back UNOBSERVED**: `Provenance::Run.root_at_eval` is
/// empty on every one of them, because the fold that fills it is lazy and has
/// not run yet (`run-plane.md` § The run plane). A caller that reports or
/// applies these effects must pass them through [`observe_if_emitted`] first;
/// [`dispatch`] does it for you.
///
/// # Errors
/// [`EvalError`] — the typed kernel surface.
pub fn evaluate(d: &StarlarkDispatch<'_>) -> Result<Vec<Effect>, EvalError> {
    // Measured here, not at the call site, so `--dry` reports the same `eval`
    // phase the live run does — one seam, both tenses. A faulted eval reports
    // nothing: the `?` below abandons the span (`timing` has no `Drop`).
    let eval = timing::phase("eval");
    let ctx = RunCtx {
        page: d.page.to_owned(),
        task: d.task.to_owned(),
        args: d.args.clone(),
        env: d.env.clone(),
        invocation_id: d.invocation_id.to_owned(),
        // Unobserved: the domain is folded after eval, and only if this
        // returns something to stamp. The sandbox cannot read the field
        // (`effects::RunCtx` — page/task/args/env only), so an eval cannot
        // tell the difference and no output can depend on it.
        root_at_eval: String::new(),
    };
    let effects = eval_run(&Rule::new(d.task, d.source), &ctx, d.limits)?;
    eval.stop();
    Ok(effects)
}

/// Stamp `token` onto every effect's `Provenance::Run.root_at_eval`,
/// overwriting the unobserved placeholder [`evaluate`] left there.
fn restamp_run_root(effects: &mut [Effect], token: &MerkleRoot) {
    for e in effects {
        if let Provenance::Run { root_at_eval, .. } = &mut e.provenance {
            root_at_eval.clone_from(&token.0);
        }
    }
}

/// The lazy observation, one owner for both tenses: fold the hash domain and
/// stamp the emitted effects with its root — **only if the eval emitted**.
///
/// An effect-free block returns `Ok(None)` having walked nothing: there is no
/// provenance to carry the token, no md.\* batch to hand it to as
/// `observed_root`, and no receipt to attest it, so the fold would be
/// observation nobody observes. That is the whole saving (`run-plane.md`
/// § The run plane: 99.5% of an effect-free run on a 37 800-member root).
///
/// Folding AFTER the eval names the same domain folding before it would have:
/// this entry is hermetic by construction — the eval reaches no I/O, so it
/// cannot move the corpus underneath itself.
///
/// # Errors
/// I/O from [`fs::domain_snapshot`] — loading the domain config, walking the
/// root, or reading a member.
pub fn observe_if_emitted(
    root: &fs::WorkspaceRoot,
    effects: &mut [Effect],
) -> std::io::Result<Option<MerkleRoot>> {
    if effects.is_empty() {
        return Ok(None);
    }
    let (_, folded) = fs::domain_snapshot(root)?;
    restamp_run_root(effects, &folded);
    Ok(Some(folded))
}

/// The full hermetic dispatch: evaluate, observe if it emitted, split by
/// domain, apply the md.* subset through the executor's one batch. No pin is
/// compared — a foreign advance across the eval window re-derives and
/// proceeds (no-guard ruling); the receipt attests `root_at_eval` as the
/// observation the effects were produced against.
///
/// # Errors
/// [`DispatchError`] — eval fault, post-eval fold failure, or executor
/// refusal.
pub fn dispatch(
    root: &fs::WorkspaceRoot,
    d: &StarlarkDispatch<'_>,
) -> Result<DispatchOutcome, DispatchError> {
    let mut effects = evaluate(d).map_err(DispatchError::Eval)?;
    // The lazy fold, before the partition: `md` is cloned out of `effects`,
    // so the stamp has to land while there is still one copy.
    let observed = observe_if_emitted(root, &mut effects).map_err(|e| DispatchError::Root {
        reason: e.to_string(),
    })?;
    let (md, unexecuted): (Vec<Effect>, Vec<Effect>) = effects
        .iter()
        .cloned()
        .partition(|e| e.kind.domain() == Domain::Md);

    let applied = if md.is_empty() {
        None
    } else {
        // A block that emitted no md.* effect emits no `apply` line either:
        // there was no batch, and a zero-microsecond line would claim there
        // was one. A REFUSED batch is likewise silent — the `?` abandons the
        // span, and a refusal is not a completed apply.
        let apply = timing::phase("apply");
        // md.* effects are effects, so the fold above ran. Stated as a
        // refusal rather than an unwrap: an apply may not invent a root.
        let observed_root = observed.as_ref().ok_or_else(|| DispatchError::Root {
            reason: "md.* effects with no post-eval observation".to_owned(),
        })?;
        let committed = executor::apply(
            root,
            &ApplyRequest {
                page: d.page,
                task: d.task,
                task_rev: d.task_rev,
                invocation_id: d.invocation_id,
                now: d.now,
                effects: &md,
                authority: d.authority,
                observed_root,
                receipt: d.receipt.clone(),
                exec: None, // hermetic: no child process
                actor: d.actor,
                depth: 0,
                delta: d.delta,
                fields: d.fields,
                birth_seq: d.birth_seq,
                ambient: d.ambient,
            },
        )
        .map_err(DispatchError::Exec)?;
        apply.stop();
        Some(committed)
    };

    Ok(DispatchOutcome {
        guarantee: GuaranteeClass::Hermetic,
        effects,
        applied,
        unexecuted,
    })
}
