//! The realise engine (S1, U3.5a) — observe → check → apply per claim on the
//! shipped run plane (d2 §3; §5.4).
//!
//! Owns the convergence loop and the terminal-state classifier over a set of
//! [`Claim`]s: observe and check (a pure read), then on drift apply the
//! claim's program through [`run::runner::run`] until the check converges or
//! the retry budget is spent. Never defines what a check MEANS (the [`Check`]
//! trait is the caller's), invents a second write path, or carries a
//! per-consumer branch — the storm (U3.3) is an ordinary effect page realised
//! through this loop.
//!
//! The A4 mechanical classifier (d2 §2.1):
//! - check failed ∧ no apply-capable claim → [`ClaimState::PendingAgent`]
//!   (one board card, guarded create);
//! - apply declared ∧ retry budget exhausted → [`ClaimState::NonConvergent`];
//! - check converged → [`ClaimState::Converged`].
//!
//! Laws held here:
//! - No apply lands unrecorded (d2 §3): every apply runs with a receipt
//!   address; [`RealiseError::UnrecordedApply`] surfaces a violation.
//! - Caps = exactly the union of the claims' declared caps; the verb adds
//!   none (d2 §3): [`RealiseReport::caps_union`]. Enforcement stays per-apply
//!   at the executor choke point.
//! - Board card idempotency by claim selector (§5.4): the card path derives
//!   from the selector, so a re-realise hits the `if_absent` CAS — already
//!   scheduled, never a second card.
//! - Card vocabulary is the user's (docs/laws.md § Amendment — no hard-coded
//!   flow): a claim's [`Claim::card_template`] page supplies the card body
//!   through the one template mechanism (`crates/preset`); the engine fills
//!   only the slots it owns. The baked [`render_card`] body mints only when
//!   no template is declared.
//! - §9 identity/time: the engine mints no clock and no identity; everything
//!   is caller-supplied on [`RealiseSpec`].

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use effects::EvalLimits;
use run::caps::{Cap, CapSet};
use run::executor::{Applied, ReceiptAddr};

/// Empty run-birth fields for the realise lane (no fields ride it).
static EMPTY_RUN_FIELDS: BTreeMap<String, String> = BTreeMap::new();
use run::runner::{self, RunSpec, TaskOutcome};

/// The receipt file every realise apply appends to (workspace-relative). One
/// file per realise plane, mirroring the run plane's `receipts/run.md` (address
/// policy is the caller's — this is the realise convention).
const REALISE_RECEIPT_PATH: &str = "receipts/realise.md";

/// The verdict of observing and checking one claim against the current tree —
/// a pure read, no write, no cap (d2 §5.4).
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
/// detection: an implementation reads the workspace and returns a
/// [`CheckOutcome`]; it never writes and needs no capability.
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
/// (d2 §5.4). Converged iff the field is present with exactly `expected`;
/// drifted otherwise, naming the observed value. Reads disk only — no cap.
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

/// Read one frontmatter field's current value off a parsed document, PUBLISHED
/// through the one value owner ([`model::fm_doc_publish`], wire-contract
/// § A.6.1 + § A.6.1a). This is the OBSERVED half of a value comparison; the
/// declared half (`realise.expected`, read at the page edge) publishes through
/// the same owner, because a decode on one side alone moves the mismatch
/// instead of closing it.
///
/// `field` is whatever a page declares as `realise.field`, so a block scalar is
/// authorable under it and no naming convention bounds the class — the same
/// arbitrary-key reason [`preset`](../preset)'s checker publishes through the
/// seam (card `scalar-text-trims-config-key-block-scalars`). Trimming here
/// would report DRIFTED against a page that has already converged, which is the
/// worse half of the failure: a realise loop then applies a change the world
/// did not need.
fn fm_value(doc: &model::Document, field: &str) -> Option<String> {
    model::fm_doc_publish(doc, field)
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
    /// The id of the rule this claim realises (`policy::RuleId` grammar), when
    /// the caller declared one. A minted board card carries it as a reference
    /// — the rule lives at its own page and the card never copies its body
    /// (verdict 18.1). `None` ⇒ no rule key on the card.
    pub rule: Option<String>,
    /// Observe + check: reads the current tree, returns convergence. Pure.
    pub check: Box<dyn Check>,
    /// The apply program. `None` ⇒ the claim is NOT apply-capable: a drifted
    /// check classifies `pending-agent` (A4).
    pub apply: Option<ApplyBinding>,
    /// Max apply attempts before `non-convergent` (A4). Ignored when `apply`
    /// is `None`.
    pub retry_budget: u32,
    /// The user-supplied template page (workspace-relative) whose `^template`
    /// block supplies this claim's pending-agent card body — the card's whole
    /// vocabulary is the USER's, the engine fills only the slots it owns
    /// (`{{selector}}`, `{{rule}}`, `{{detail}}`, `{{now}}`, `{{actor}}`;
    /// docs/laws.md § Amendment — no hard-coded flow). Declared but
    /// unresolvable REFUSES the mint loud — a silent fallback would let a
    /// typo'd path resurrect the baked vocabulary invisibly. `None` ⇒ the
    /// built-in [`render_card`] body.
    pub card_template: Option<String>,
}

/// The terminal state the A4 mechanical classifier assigns a claim (d2 §2.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimState {
    /// The check holds (the green terminal) — reached at observe time or after
    /// an apply converged it.
    Converged,
    /// Check failed and no apply-capable claim is declared in scope: a human /
    /// agent is needed. One board card was minted (or already present —
    /// idempotent).
    PendingAgent {
        /// The born card's workspace-relative path when THIS run minted it;
        /// `None` when the card already existed (already scheduled).
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
    /// Base invocation id; each apply derives `<invocation_id>~<n>` for the run
    /// plane and `r-<invocation_id>-<n>` for its receipt anchor. The anchor
    /// carries the base id because `receipts/realise.md` is shared across
    /// invocations (§6.6), so the id must be unique per invocation and inside
    /// the block-id charset (`[A-Za-z0-9-]`, §2.4) — a violation refuses with
    /// [`RealiseError::BadInvocationId`] before any apply runs.
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
    /// The caller's invocation id cannot carry an apply receipt anchor: the id
    /// the anchor derives from bears a char outside the block-id charset
    /// (`[A-Za-z0-9-]`, §2.4). Refused before any apply runs — an anchor no
    /// strict door can address is a receipt published unusable (§6.6).
    BadInvocationId {
        /// The offending caller-supplied id.
        invocation_id: String,
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
            RealiseError::BadInvocationId { invocation_id } => write!(
                f,
                "invocation id '{invocation_id}' is outside the block-id charset [A-Za-z0-9-] \
                 (§2.4) — an apply receipt anchor derives from it and would be unaddressable"
            ),
        }
    }
}

impl std::error::Error for RealiseError {}

/// Realise a set of claims: observe → check → apply each, classify with the A4
/// mechanical classifier, and mint a board card for every pending-agent claim.
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
    // once up front.
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
                    mint_board_card(root, claim, &detail, spec)?
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
        anchor: apply_anchor(&spec.invocation_id, attempt)?,
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
        scratch: &spec.scratch,
        timeout: spec.timeout,
        declaring_root: spec.declaring_root.as_deref(),
        limits: spec.limits,
        actor: None,
        step_cwd: None,
        delta: None, // realise lane: delta status unchanged by the § A.8 ruling
        // Realise addresses its own declared blocks; no birth fields ride
        // this lane and no ring is in reach.
        fields: &EMPTY_RUN_FIELDS,
        birth_seq: None,
        ambient: None,
        // Realise drives the plane wherever it is hosted; it holds no resident
        // cache handle, so its observations keep the drawer instrument.
        observations: run::dispatch_bash::ObservationSource::Drawer,
    };
    let mut sink = io::sink();
    let report = runner::run(root, &run_spec, &[], &mut sink).map_err(|e| RealiseError::Apply {
        selector: selector.to_owned(),
        reason: e.to_string(),
    })?;
    Ok(applied_of(report.outcome))
}

/// Mint one apply receipt's anchor: `r-<invocation-id>-<attempt>`, unique
/// within the SHARED `receipts/realise.md` across invocations (contract §6.6 —
/// the anchor is the caller's to mint, and a mint that collides publishes a
/// receipt no strict door can address). A per-invocation counter is unique only
/// inside its own process, so from invocation 2 on it re-mints invocation 1's
/// ids; the caller's invocation id is what makes the id file-scoped.
///
/// The mint routes through the block-id door (`model::Ref::anchor`, §2.4) so a
/// caller-supplied id outside `[A-Za-z0-9-]` refuses here rather than
/// publishing an unaddressable anchor.
fn apply_anchor(invocation_id: &str, attempt: u64) -> Result<String, RealiseError> {
    let id = format!("r-{invocation_id}-{attempt:06}");
    match model::Ref::anchor(id.clone()) {
        Ok(_) => Ok(id),
        Err(_) => Err(RealiseError::BadInvocationId {
            invocation_id: invocation_id.to_owned(),
        }),
    }
}

/// Extract the executor's commit from a run-plane report — the md.* apply of
/// generation 0, on either dispatch path.
fn applied_of(outcome: TaskOutcome) -> Option<Applied> {
    match outcome {
        TaskOutcome::Starlark(o) => o.applied,
        TaskOutcome::Bash(o) => match o.phase2 {
            run::dispatch_bash::Phase2::Applied { applied } => Some(applied),
            _ => None,
        },
    }
}

/// Mint one pending-agent board card through the U2.6 guarded create (§5.4
/// emit): CAS `if_absent`, gate seam (`&[]` ⇒ bare commit). Returns the born
/// card's path. Idempotent by claim selector — a card that already exists is
/// "already scheduled", returned as `Ok(None)`, never a second card and never an
/// error.
fn mint_board_card(
    root: &fs::WorkspaceRoot,
    claim: &Claim,
    detail: &str,
    spec: &RealiseSpec,
) -> Result<Option<String>, RealiseError> {
    let selector = claim.selector.as_str();
    let rule = claim.rule.as_deref();
    let path = format!(
        "{}/{}.md",
        spec.board_dir.trim_end_matches('/'),
        card_slug(selector)
    );
    // `created:` is RFC3339 or nothing (verdict 15.7): a malformed caller
    // clock is refused loud, never stamped onto a governed page.
    if let Some(now) = spec.now.as_deref()
        && !wire::now_is_rfc3339(now)
    {
        return Err(RealiseError::CardMint {
            selector: selector.to_owned(),
            reason: format!("`now` is not RFC3339: {now:?}"),
        });
    }
    let body = match claim.card_template.as_deref() {
        Some(template) => render_card_from_template(root, template, selector, rule, detail, spec)?,
        None => render_card(selector, rule, detail, spec.now.as_deref()),
    };
    let args = wire_serve::write::CreateArgs {
        id: None,
        path: wire::Path(path.clone()),
        body,
        actor: Some(spec.actor.clone()),
        now: spec.now.clone(),
        if_root: None,
        dry: false,
        fields: BTreeMap::default(),
        // The card's frontmatter rides its rendered body (D6 props= is the
        // starlark birth lane's door argument, not this one).
        props: BTreeMap::default(),
    };
    match wire_serve::write::create(root, None, &args, &[]) {
        Ok(_) => Ok(Some(path)),
        // The card already exists — the CAS refusal IS the idempotency.
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

/// Render the card body from the claim's user-supplied template page
/// (docs/laws.md § Amendment — no hard-coded flow): the page's `^template`
/// block IS the card — folder names, the key that spells state, status words,
/// and prose are all the user's — and the engine substitutes only the slot
/// values it owns: `{{selector}}`, `{{rule}}` (empty when the claim declares
/// none), `{{detail}}`, `{{now}}` (empty when the caller passed none),
/// `{{actor}}`.
///
/// Extraction and fill are `crates/preset`'s — the ONE template mechanism —
/// so a frontmatter substitution rides the § A.6.3a encoder (a `detail`
/// carrying `: ` cannot mint a shadow key line) and a newline into the
/// frontmatter block refuses rather than rewrites.
///
/// Every failure is a LOUD [`RealiseError::CardMint`] naming the template
/// page: an unreadable page, a page with no `^template` block, a newline into
/// frontmatter. Never a silent fallback — that would let a typo'd path
/// resurrect the baked vocabulary invisibly.
fn render_card_from_template(
    root: &fs::WorkspaceRoot,
    template_page: &str,
    selector: &str,
    rule: Option<&str>,
    detail: &str,
    spec: &RealiseSpec,
) -> Result<String, RealiseError> {
    let err = |reason: String| RealiseError::CardMint {
        selector: selector.to_owned(),
        reason,
    };
    let doc = fs::load(root, Path::new(template_page))
        .map_err(|e| err(format!("card template {template_page}: {e}")))?;
    let template = preset::template_of(&doc.raw).ok_or_else(|| {
        err(format!(
            "card template {template_page} declares no ^template block — the card body is the \
             fenced record inside the section whose heading LINE carries the `^template` anchor \
             (`# Template ^template`); a bare `# Template` heading declares nothing"
        ))
    })?;
    let vars = [
        ("{{selector}}", selector),
        ("{{rule}}", rule.unwrap_or("")),
        ("{{detail}}", detail),
        ("{{now}}", spec.now.as_deref().unwrap_or("")),
        ("{{actor}}", spec.actor.as_str()),
    ];
    preset::fill_slots(&template, &vars).map_err(|e| {
        let key = if e.key.is_empty() {
            e.placeholder.as_str()
        } else {
            e.key.as_str()
        };
        err(format!(
            "card template {template_page}: {} — the card mint filled {} there, and the engine \
             stamps the observed drift exactly as given (§9), so it refuses rather than \
             rewrite it",
            policy::defs::multi_line_value_refusal(key),
            e.placeholder
        ))
    })
}

/// Render the BUILT-IN pending-agent board card — the body minted only when
/// the claim declares no [`Claim::card_template`]: a governed markdown page an
/// agent pulls from the board and works through the same doors as any editor
/// (§5.4).
///
/// A card references its rule, it never embeds it (verdict 18.1): the card
/// carries the rule id in `rule:` plus one wikilink, so the law has exactly
/// one home. `created:` is RFC3339 (verdict 15.7) from the caller's `now` —
/// this function reads no clock, so a fixed `now` renders a byte-identical
/// card.
fn render_card(selector: &str, rule: Option<&str>, detail: &str, now: Option<&str>) -> String {
    let created = now.map_or_else(String::new, |now| format!("created: {now}\n"));
    let (rule_key, rule_ref) = rule.map_or_else(
        || (String::new(), String::new()),
        |id| {
            (
                format!("rule: {id}\n"),
                format!("Rule: [[{id}]] — read the law there; this card carries the id, not the body.\n\n"),
            )
        },
    );
    format!(
        "---\ntype: board-card\nstate: pending-agent\nclaim: {selector}\n{rule_key}{created}---\n\n# pending-agent: {selector}\n\nCheck drifted with no apply-capable claim in scope.\n\n{rule_ref}{detail}\n"
    )
}

/// Resolve one apply binding's declared caps through the run plane's own
/// authority resolution (explicit frontmatter > the root's `MERIDIAN.md`
/// convention > deny) — the same resolution `runner::run` applies, read here
/// without running the block.
///
/// A bash binding contributes [`CapSet::none()`]: capabilities do not apply
/// to it (`docs/laws.md` § Amendment), so `caps_union` under-describes a bash
/// claim by construction — it is a union of DECLARED caps, not a boundedness
/// claim.
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
