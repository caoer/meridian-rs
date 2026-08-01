//! The realise engine (S1, U3.5a) — **observe → check → apply per claim** on
//! the shipped run plane (plan §4 Block 3; d2 §3 realise; §5.4 the agentic
//! bridge).
//!
//! # Charter
//! **Owns:** the convergence loop and the terminal-state classifier over a set
//! of [`Claim`]s. Per claim: observe the current tree and CHECK it (a pure
//! read — "check = rev compare", §5.4), then, on drift, APPLY the claim's
//! program through the shipped run plane ([`run::runner::run`], the ordinary
//! loop) until the check converges or the retry budget is spent. The engine
//! classifies each claim's outcome with the **A4 mechanical classifier**
//! (d2 §2.1) and never judges beyond it.
//!
//! **Never does:** define what a claim's check MEANS (the [`Check`] trait is
//! the caller's — the built-in [`FieldEquals`] is one instance, the storm's
//! census is another), invent a second write path (apply rides the run plane's
//! one write path, [`run::executor`]), or carry any per-consumer path. The
//! storm (U3.3) is an ordinary effect page realised through THIS loop — there
//! is no storm branch here.
//!
//! # The A4 mechanical classifier (d2 §2.1, verbatim)
//! - check failed ∧ **no apply-capable claim** declared in scope →
//!   [`ClaimState::PendingAgent`] (and one board card, born through the U2.6
//!   guarded create — §5.4 emit).
//! - **apply declared** ∧ retry budget exhausted →
//!   [`ClaimState::NonConvergent`].
//! - check converged → [`ClaimState::Converged`] (the green terminal).
//!
//! The core never judges: the engine reads the [`CheckOutcome`] the claim
//! reports and applies the mechanical rule above — it does not decide WHETHER a
//! divergence is legitimate.
//!
//! # Laws held here
//! - **No apply lands unrecorded** (d2 §3): every apply runs with a receipt
//!   address, so the run plane's one write path mints a receipt line for every
//!   committed batch; the engine surfaces [`RealiseError::UnrecordedApply`] if
//!   a committed apply ever lacked a receipt (unreachable by construction — the
//!   assertion keeps it honest).
//! - **Caps = exactly the union of the claims' declared caps; the verb adds
//!   none** (d2 §3): [`RealiseReport::caps_union`] is the fold of every
//!   apply-capable claim's resolved [`run::caps::CapSet`] — the realise verb's
//!   total authority. Enforcement stays per-apply at the executor choke point;
//!   this is the declared surface (and the `--dry-run` blast radius).
//! - **Board card idempotency by claim selector** (§5.4): the card path derives
//!   from the claim selector, so a re-realise of the same pending-agent claim
//!   hits the guarded create's `if_absent` CAS and is treated as already
//!   scheduled — never a second card, never an error.
//! - **§9 identity/time**: the engine mints no clock and no identity; the base
//!   invocation id, `now`, actor, and scratch dir are all caller-supplied on
//!   [`RealiseSpec`]. Per-apply invocation ids and receipt anchors derive
//!   deterministically from the base plus a monotonic attempt counter.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use effects::EvalLimits;
use run::caps::{Cap, CapSet};
use run::executor::{Applied, ReceiptAddr};
use run::runner::{self, RunSpec, TaskOutcome};

/// The receipt file every realise apply appends to (workspace-relative). One
/// file per realise plane, mirroring the run plane's `receipts/run.md` (address
/// policy is the caller's — this is the realise convention).
const REALISE_RECEIPT_PATH: &str = "receipts/realise.md";

/// The verdict of observing and checking one claim against the current tree —
/// a pure read, no write, no cap (d2 §5.4: "detect (core: check = rev
/// compare)"). A [`Check`] returns exactly this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckOutcome {
    /// The claim holds: the observed state matches what the claim asserts.
    Converged,
    /// The claim does not hold — with a human-facing reason carried onto the
    /// board card when the claim is not apply-capable.
    Drifted {
        /// Why the check failed (observed vs expected), for the board card.
        detail: String,
    },
}

/// How to observe the current tree and decide a claim's convergence. Pure
/// detection: an implementation READS the workspace and returns a
/// [`CheckOutcome`]; it never writes and needs no capability. The engine is
/// general over this trait — the built-in [`FieldEquals`] is one instance, the
/// storm's corpus census is another (added additively, never a branch here).
pub trait Check {
    /// Observe the current tree and check the claim.
    ///
    /// # Errors
    /// [`CheckError`] when the observation itself fails (page load, I/O) —
    /// distinct from a clean [`CheckOutcome::Drifted`], which is not an error.
    fn observe(&self, root: &fs::WorkspaceRoot) -> Result<CheckOutcome, CheckError>;
}

/// Why an observation could not be made (distinct from a clean drift verdict).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckError {
    /// The claim being observed.
    pub selector: String,
    /// The underlying reason (page load, I/O).
    pub reason: String,
}

impl std::fmt::Display for CheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "observe '{}' failed: {}", self.selector, self.reason)
    }
}

impl std::error::Error for CheckError {}

/// The built-in check: a frontmatter field's current value equals `expected`
/// (d2 §5.4 "check = rev compare", at value grain). Converged iff the field is
/// present with exactly `expected`; drifted otherwise, naming the observed
/// value. Reads disk only — no cap.
#[derive(Debug, Clone)]
pub struct FieldEquals {
    /// The page carrying the field (workspace-relative).
    pub page: String,
    /// The frontmatter field to observe.
    pub field: String,
    /// The value that means converged.
    pub expected: String,
}

impl Check for FieldEquals {
    fn observe(&self, root: &fs::WorkspaceRoot) -> Result<CheckOutcome, CheckError> {
        let err = |reason: String| CheckError {
            selector: format!("{}#{}", self.page, self.field),
            reason,
        };
        let doc = fs::load(root, Path::new(&self.page)).map_err(|e| err(e.to_string()))?;
        let observed = fm_value(&doc, &self.field);
        Ok(match observed.as_deref() {
            Some(v) if v == self.expected => CheckOutcome::Converged,
            Some(v) => CheckOutcome::Drifted {
                detail: format!(
                    "{}: '{}' is '{v}', expected '{}'",
                    self.page, self.field, self.expected
                ),
            },
            None => CheckOutcome::Drifted {
                detail: format!(
                    "{}: '{}' is unset, expected '{}'",
                    self.page, self.field, self.expected
                ),
            },
        })
    }
}

/// Read one frontmatter field's current value off a parsed document, using only
/// the public model surface (the frontmatter node's [`model::YamlMap`]).
fn fm_value(doc: &model::Document, field: &str) -> Option<String> {
    fn find(node: &model::Node) -> Option<&model::YamlMap> {
        if let model::NodeKind::Frontmatter { map } = &node.kind {
            return Some(map);
        }
        node.children.iter().find_map(find)
    }
    find(&doc.root)
        .and_then(|m| m.0.iter().find(|(k, _)| k == field))
        .map(|(_, v)| v.clone())
}

/// A claim's apply program — a run-plane task binding. Running it drives the
/// world toward convergence through the shipped run plane (`runner::run`); its
/// declared caps live in the page frontmatter (`task.<task>.caps`) and are the
/// only distinction the engine sees (deny-by-default).
#[derive(Debug, Clone)]
pub struct ApplyBinding {
    /// The effect page carrying the apply task (workspace-relative).
    pub page: String,
    /// The apply task name to address on that page.
    pub task: String,
    /// Positional args the apply task's contract requires.
    pub args: Vec<String>,
    /// Declared env the apply task's contract requires.
    pub env: BTreeMap<String, String>,
}

/// One claim the engine realises: its identity, how to check it, and — if it is
/// apply-capable — the program that converges it plus the retry budget.
pub struct Claim {
    /// The claim selector — the board-card key and the pending-agent name.
    /// Card idempotency is keyed on this (§5.4).
    pub selector: String,
    /// Observe + check: reads the current tree, returns convergence. Pure.
    pub check: Box<dyn Check>,
    /// The apply program. `None` ⇒ the claim is NOT apply-capable: a drifted
    /// check classifies `pending-agent` (A4).
    pub apply: Option<ApplyBinding>,
    /// Max apply attempts before `non-convergent` (A4). Ignored when `apply`
    /// is `None`.
    pub retry_budget: u32,
}

/// The terminal state the A4 mechanical classifier assigns a claim (d2 §2.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimState {
    /// The check holds (the green terminal) — reached at observe time or after
    /// an apply converged it.
    Converged,
    /// Check failed and no apply-capable claim is declared in scope: a human /
    /// agent is needed. One board card was minted (or already present —
    /// idempotent), its journal anchor carried here when this run minted it.
    PendingAgent {
        /// The guarded-create journal anchor when THIS run minted the card;
        /// `None` when the card already existed (idempotent — already
        /// scheduled).
        card: Option<String>,
    },
    /// Apply was declared but the retry budget was exhausted with the check
    /// still drifted.
    NonConvergent,
}

/// What the engine did with one claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimResult {
    /// The claim selector.
    pub selector: String,
    /// The A4 terminal state.
    pub state: ClaimState,
    /// How many apply attempts ran (each = one run-plane invocation).
    pub applies: u32,
    /// The receipt line of every apply that COMMITTED, in order — the proof
    /// that no apply landed unrecorded.
    pub receipts: Vec<String>,
}

/// The caller-supplied envelope for one realise run (§9 — the engine mints no
/// clock and no identity).
#[derive(Debug, Clone)]
pub struct RealiseSpec {
    /// Base invocation id; each apply derives `<invocation_id>~<n>`.
    pub invocation_id: String,
    /// Caller-supplied time fact stamped onto every apply receipt and board
    /// card; absent stays absent, never invented.
    pub now: Option<String>,
    /// The actor recorded on a minted board card (guarded create).
    pub actor: String,
    /// The workspace-relative directory pending-agent board cards are born in
    /// (one file per claim selector).
    pub board_dir: String,
    /// A caller-created scratch directory for bash apply steps (unused on the
    /// starlark path; the run plane requires it).
    pub scratch: PathBuf,
    /// `--dry-run` (d2 §3): observe + check + classify PROJECTED states with
    /// ZERO caps — no apply runs and no card is minted; the report's
    /// `projected_applies` is the blast radius.
    pub dry_run: bool,
    /// Kernel eval limits threaded into every apply run.
    pub limits: EvalLimits,
    /// Bash wall-clock ceiling threaded into every apply run.
    pub timeout: Duration,
    /// The root whose `MERIDIAN.md` declares the caps convention ceiling, or
    /// `None` when the ladder answered nothing. Threaded in beside `timeout`
    /// for the same reason: only the caller holds the ladder's answer.
    pub declaring_root: Option<PathBuf>,
}

/// The whole realise run's outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealiseReport {
    /// Per-claim results, in input order.
    pub claims: Vec<ClaimResult>,
    /// The union of every apply-capable claim's declared caps — the realise
    /// verb's total authority (the verb adds none). Empty on a corpus with no
    /// apply-capable claim.
    pub caps_union: CapSet,
    /// The `--dry-run` blast radius: the selectors that WOULD apply (drifted ∧
    /// apply-capable). Empty on a live run.
    pub projected_applies: Vec<String>,
}

/// Why a realise run faulted. A fault stops the run — prior claims' committed
/// applies stand (the run plane never rolls back).
#[derive(Debug)]
pub enum RealiseError {
    /// A claim's observation failed (not a clean drift — see [`CheckError`]).
    Check(CheckError),
    /// Resolving a claim's declared caps failed (page load, malformed caps).
    Caps {
        /// The claim whose caps failed to resolve.
        selector: String,
        /// The underlying reason.
        reason: String,
    },
    /// An apply run refused or faulted on the run plane.
    Apply {
        /// The claim whose apply faulted.
        selector: String,
        /// The typed run-plane failure.
        reason: String,
    },
    /// A committed apply lacked a receipt — the no-apply-unrecorded invariant
    /// was violated (unreachable by construction; surfaced, never swallowed).
    UnrecordedApply {
        /// The claim whose apply committed without a receipt.
        selector: String,
    },
    /// Minting the pending-agent board card through the guarded create failed
    /// for a reason other than the idempotent `if_absent` CAS.
    CardMint {
        /// The claim whose card mint failed.
        selector: String,
        /// The underlying refusal.
        reason: String,
    },
}

impl std::fmt::Display for RealiseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RealiseError::Check(e) => write!(f, "{e}"),
            RealiseError::Caps { selector, reason } => {
                write!(f, "caps for '{selector}': {reason}")
            }
            RealiseError::Apply { selector, reason } => {
                write!(f, "apply '{selector}': {reason}")
            }
            RealiseError::UnrecordedApply { selector } => write!(
                f,
                "apply '{selector}' committed without a receipt — no-apply-unrecorded violated"
            ),
            RealiseError::CardMint { selector, reason } => {
                write!(f, "board card for '{selector}': {reason}")
            }
        }
    }
}

impl std::error::Error for RealiseError {}

/// Realise a set of claims: observe → check → apply each, classify with the A4
/// mechanical classifier, and mint a board card for every pending-agent claim.
///
/// The cutover order (walk → pin → attest → realise → status) is law elsewhere
/// (d2 §6.2); this is the ENGINE the storm (U3.3) and every later consumer
/// drives — general, no per-consumer branch.
///
/// # Errors
/// [`RealiseError`] on the first faulting claim; prior claims' committed applies
/// stand (no rollback). A clean [`ClaimState::PendingAgent`] /
/// [`ClaimState::NonConvergent`] is NOT an error — it is the classifier's
/// verdict, carried in the report.
pub fn realise(
    root: &fs::WorkspaceRoot,
    claims: &[Claim],
    spec: &RealiseSpec,
) -> Result<RealiseReport, RealiseError> {
    // Caps = the union of every apply-capable claim's declared caps, resolved
    // once up front (the verb's authority + the dry-run blast-radius input).
    let mut caps_union = CapSet::none();
    for claim in claims {
        if let Some(binding) = &claim.apply {
            let caps = resolve_binding_caps(
                root,
                spec.declaring_root.as_deref(),
                &claim.selector,
                binding,
            )?;
            caps_union = union(&caps_union, &caps);
        }
    }

    let mut results = Vec::with_capacity(claims.len());
    let mut projected_applies = Vec::new();
    // Monotonic across the whole run — keeps apply receipt anchors and
    // invocation ids unique (§9: derived from the caller's base id).
    let mut attempt_seq: u64 = 0;

    for claim in claims {
        let outcome = claim.check.observe(root).map_err(RealiseError::Check)?;
        let CheckOutcome::Drifted { detail } = outcome else {
            results.push(ClaimResult {
                selector: claim.selector.clone(),
                state: ClaimState::Converged,
                applies: 0,
                receipts: Vec::new(),
            });
            continue;
        };

        match &claim.apply {
            // Not apply-capable + drifted → pending-agent (A4).
            None => {
                let card = if spec.dry_run {
                    None
                } else {
                    mint_board_card(root, &claim.selector, &detail, spec)?
                };
                results.push(ClaimResult {
                    selector: claim.selector.clone(),
                    state: ClaimState::PendingAgent { card },
                    applies: 0,
                    receipts: Vec::new(),
                });
            }
            // Apply-capable + drifted. Dry-run projects the blast radius and
            // stops (zero caps). Live drives the convergence loop.
            Some(binding) => {
                if spec.dry_run {
                    projected_applies.push(claim.selector.clone());
                    results.push(ClaimResult {
                        selector: claim.selector.clone(),
                        state: ClaimState::NonConvergent,
                        applies: 0,
                        receipts: Vec::new(),
                    });
                    continue;
                }
                let result = converge(root, claim, binding, spec, &mut attempt_seq)?;
                results.push(result);
            }
        }
    }

    Ok(RealiseReport {
        claims: results,
        caps_union,
        projected_applies,
    })
}

/// Drive one apply-capable claim's convergence loop: apply through the run
/// plane, re-check, up to the retry budget. Converged at any point wins;
/// budget exhausted with the check still drifted is `non-convergent` (A4).
fn converge(
    root: &fs::WorkspaceRoot,
    claim: &Claim,
    binding: &ApplyBinding,
    spec: &RealiseSpec,
    attempt_seq: &mut u64,
) -> Result<ClaimResult, RealiseError> {
    let mut receipts = Vec::new();
    let mut applies = 0;
    for _ in 0..claim.retry_budget {
        *attempt_seq += 1;
        let committed = run_apply(root, &claim.selector, binding, spec, *attempt_seq)?;
        applies += 1;
        // No apply lands unrecorded: a committed batch MUST carry a receipt.
        if let Some(a) = committed {
            let line = a
                .receipt_line
                .ok_or_else(|| RealiseError::UnrecordedApply {
                    selector: claim.selector.clone(),
                })?;
            receipts.push(line);
        }
        let outcome = claim.check.observe(root).map_err(RealiseError::Check)?;
        if outcome == CheckOutcome::Converged {
            return Ok(ClaimResult {
                selector: claim.selector.clone(),
                state: ClaimState::Converged,
                applies,
                receipts,
            });
        }
    }
    Ok(ClaimResult {
        selector: claim.selector.clone(),
        state: ClaimState::NonConvergent,
        applies,
        receipts,
    })
}

/// Run one apply attempt through the shipped run plane. Every attempt carries a
/// receipt address (unique anchor per attempt), so the executor mints a receipt
/// for every committed batch. Returns the executor's [`Applied`] when the block
/// committed md.* effects, `None` when it emitted none (nothing landed).
fn run_apply(
    root: &fs::WorkspaceRoot,
    selector: &str,
    binding: &ApplyBinding,
    spec: &RealiseSpec,
    attempt: u64,
) -> Result<Option<Applied>, RealiseError> {
    let receipt = ReceiptAddr {
        path: REALISE_RECEIPT_PATH.to_owned(),
        anchor: format!("r-{attempt:06}"),
    };
    let invocation = format!("{}~{attempt}", spec.invocation_id);
    let run_spec = RunSpec {
        page: &binding.page,
        task: Some(binding.task.as_str()),
        args: binding.args.clone(),
        env: binding.env.clone(),
        invocation_id: &invocation,
        now: spec.now.as_deref(),
        receipt: Some(receipt),
        pre_receipt: None,
        takeover: false,
        scratch: &spec.scratch,
        timeout: spec.timeout,
        declaring_root: spec.declaring_root.as_deref(),
        limits: spec.limits,
    };
    let mut sink = io::sink();
    let report = runner::run(root, &run_spec, &[], &mut sink).map_err(|e| RealiseError::Apply {
        selector: selector.to_owned(),
        reason: e.to_string(),
    })?;
    Ok(applied_of(report.outcome))
}

/// Extract the executor's commit from a run-plane report — the md.* apply of
/// generation 0, on either dispatch path.
fn applied_of(outcome: TaskOutcome) -> Option<Applied> {
    match outcome {
        TaskOutcome::Starlark(o) => o.applied,
        TaskOutcome::Bash(o) => match o.phase2 {
            run::dispatch_bash::Phase2::Applied { applied, .. } => applied,
            _ => None,
        },
    }
}

/// Mint one pending-agent board card through the U2.6 guarded create (§5.4
/// emit): CAS `if_absent`, journaled birth, gate seam (`&[]` ⇒ bare commit).
/// Idempotent by claim selector — a card that already exists is "already
/// scheduled", returned as `Ok(None)`, never a second card and never an error.
fn mint_board_card(
    root: &fs::WorkspaceRoot,
    selector: &str,
    detail: &str,
    spec: &RealiseSpec,
) -> Result<Option<String>, RealiseError> {
    let path = format!(
        "{}/{}.md",
        spec.board_dir.trim_end_matches('/'),
        card_slug(selector)
    );
    let body = render_card(selector, detail, spec.now.as_deref());
    let args = wire_serve::write::CreateArgs {
        id: None,
        path: wire::Path(path),
        body,
        actor: Some(spec.actor.clone()),
        now: spec.now.clone(),
        if_root: None,
        dry: false,
    };
    match wire_serve::write::create(root, 0, &args, &[]) {
        Ok(out) => Ok(out.journal_anchor),
        // The card already exists: the guarded create refuses the occupied path
        // with a CAS mismatch — that IS the idempotency (already scheduled).
        Err(e) if is_cas_mismatch(&e) => Ok(None),
        Err(e) => Err(RealiseError::CardMint {
            selector: selector.to_owned(),
            reason: format!("{e:?}"),
        }),
    }
}

/// Whether a guarded-create refusal is the `if_absent` CAS mismatch (the card
/// already exists) — the one refusal the engine treats as idempotent success.
fn is_cas_mismatch(err: &wire::ErrorBody) -> bool {
    err.code == wire::ErrorCode::CasMismatch
}

/// The board-card file stem for a claim selector: path-safe, deterministic,
/// one card per selector (the idempotency key).
fn card_slug(selector: &str) -> String {
    selector
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Render a pending-agent board card: a governed markdown page an agent pulls
/// from the board and works through the same doors as any editor (§5.4).
fn render_card(selector: &str, detail: &str, now: Option<&str>) -> String {
    let created = now.unwrap_or("");
    format!(
        "---\ntype: board-card\nstate: pending-agent\nclaim: {selector}\ncreated: {created}\n---\n\n# pending-agent: {selector}\n\nCheck drifted with no apply-capable claim in scope.\n\n{detail}\n"
    )
}

/// Resolve one apply binding's declared caps through the run plane's own
/// authority resolution (explicit frontmatter > the root's own `MERIDIAN.md`
/// `run.caps.<pattern>` convention > deny) — the same resolution `runner::run`
/// applies, read here for the union and the dry-run blast radius without
/// running the block.
///
/// A BASH binding contributes [`CapSet::none()`]: it declares no capability,
/// because capabilities do not apply to it (`docs/laws.md` § Amendment). That
/// is the honest fold for a union OF DECLARED CAPS — and it is not a claim
/// that the binding is bounded. `caps_union` under-describes a bash claim by
/// construction; naming a shell's reach is a question this fold cannot answer.
fn resolve_binding_caps(
    root: &fs::WorkspaceRoot,
    declaring_root: Option<&Path>,
    selector: &str,
    binding: &ApplyBinding,
) -> Result<CapSet, RealiseError> {
    let err = |reason: String| RealiseError::Caps {
        selector: selector.to_owned(),
        reason,
    };
    let doc =
        run::address::load_page(root, Path::new(&binding.page)).map_err(|e| err(e.to_string()))?;
    let task =
        run::address::resolve_task(&doc, Some(&binding.task)).map_err(|e| err(e.to_string()))?;
    let (conventions, _source) =
        run::caps::load_conventions(declaring_root).map_err(|e| err(e.to_string()))?;
    let authority =
        run::caps::resolve_authority(&doc, &binding.task, task.block.lang, &conventions)
            .map_err(|e| err(e.to_string()))?;
    Ok(authority
        .capabilities()
        .map_or_else(CapSet::none, |resolution| resolution.effective.clone()))
}

/// The union of two cap sets — the widening the realise verb performs over its
/// claims (deny-by-default `narrow` is the executor's; the verb's declared
/// authority is the union of what its claims declare).
fn union(a: &CapSet, b: &CapSet) -> CapSet {
    let merged: std::collections::BTreeSet<Cap> = a.0.union(&b.0).cloned().collect();
    CapSet(merged)
}
