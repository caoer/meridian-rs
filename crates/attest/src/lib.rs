//! U2.5 — **attest**: `attest = realised-gate + pin + receipt` (d2 §2.5 A7/E1,
//! settled team-wide; rulings check-amendment site 2; checkpoint-1 gap 1).
//!
//! Attest is a pure COMPOSITION over the landed pin ([`pin::pin`], U2.4) — it
//! never forks pin's write path. It adds ONE precondition ahead of the pin: the
//! **realised-gate**. The complete refuse-vs-record predicate the plan ratifies:
//!
//! attest refuses iff
//! - **(a) resolution fails** — an unresolvable / ambiguous / dangling ref
//!   (pin's atomic fail-closed resolve, rows 1-3);
//! - **(b) the realised-gate is unmet** — a claim in the pinner's own compose
//!   closure is `non-convergent` / `pending-agent` / `foreign_edit` (each would
//!   stamp a receipt over unconverged content); the refusal names the claim, its
//!   state, and its last receipt (row 5, `realised_gate{claim,state}`);
//! - **(c) a declared `check:` predicate is false** at the pinned rev — a false
//!   claim is not staleness, so re-pinning cannot fix it: refuse-class, never
//!   record-class (pin's atomic `check_claim`, row 4).
//!
//! Everything else RECORDS. **Drifted pins never refuse** — re-pinning drift is
//! attest's very purpose (rev currency is what pin refreshes, never a
//! precondition). **Grey inputs never refuse** — the first pin is how grey turns
//! green. A **refused attest writes nothing** (no receipt, no rev, no journal
//! row): class (b) refuses BEFORE pin runs, classes (a)/(c) ride pin's own
//! atomicity. `dry` never refuses; it reports `would_refuse` with the same named
//! reasons.
//!
//! **Scope law** (d2 §2.5): the gate covers the pinner's OWN compose closure
//! only — an upstream page's unrealised claims are that page's refusal, rendered
//! through the walk, never this pin's. The built-in [`board::BoardGate`] reads
//! the realise engine's pending-agent board cards (its recorded terminal state),
//! scoping to the target by the card's claim-selector page.

pub mod board;

use serde::Serialize;

/// The §8 code a realised-gate refusal mints (taxonomy row 5,
/// `realised_gate{claim,state}`).
const REALISED_GATE_CODE: &str = "realised_gate";
/// The §8 recovery class row 5 binds to: `retry` — converge the claim, then
/// re-attest (distinct from class (c)'s `fix`, which re-pinning cannot resolve).
const REALISED_GATE_RECOVERY: &str = "retry";

/// The states that FAIL the realised-gate (d2 §2.5): a claim in any of these
/// would stamp a receipt over unconverged content, so attest refuses (row 5). A
/// `converged` / `realised` claim (or no board card at all) passes the gate.
pub const UNMET_STATES: [&str; 3] = ["pending-agent", "non-convergent", "foreign_edit"];

/// Options for an attest run — the same `dry` / recorded-actor / recorded-time
/// surface pin takes (§9: actor and time are recorded exactly as given, never
/// invented).
#[derive(Debug, Clone, Default)]
pub struct AttestOptions {
    /// Report fully and write nothing (a `dry` refusal is a `would_refuse`).
    pub dry: bool,
    /// The recorded actor threaded onto the pin receipt.
    pub actor: Option<String>,
    /// The recorded timestamp threaded onto the pin receipt.
    pub now: Option<String>,
}

/// The realised-gate — attest's (b) precondition. Observe the pinner's own
/// compose-closure claims (a PURE read: no write, no capability) and report
/// whether they are all realised. The engine is general over this trait; the
/// built-in [`board::BoardGate`] is one instance (the realise engine's board).
pub trait RealisedGate {
    /// Evaluate the gate for `target`.
    ///
    /// # Errors
    /// [`GateError`] when the observation itself fails (board read / parse) —
    /// distinct from a clean [`GateOutcome::Unmet`], which is a verdict, not an
    /// error.
    fn evaluate(&self, root: &fs::WorkspaceRoot, target: &str) -> Result<GateOutcome, GateError>;
}

/// The realised-gate's verdict for a target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateOutcome {
    /// Every claim in the pinner's compose closure is realised — the gate passes
    /// and the pin may proceed.
    Realised,
    /// A claim is not realised — attest refuses class (b), naming it.
    Unmet(UnmetClaim),
}

/// The unrealised claim a failed gate names (d2 §2.5: claim + state + last
/// receipt).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnmetClaim {
    /// The claim selector — the board-card key and the pending-agent name.
    pub claim: String,
    /// The claim's terminal state — one of [`UNMET_STATES`].
    pub state: String,
    /// The claim's last receipt anchor, for the operator to locate its history;
    /// `None` when the claim minted none (a pure pending-agent has no apply).
    pub last_receipt: Option<String>,
}

/// Why a realised-gate observation could not be made (distinct from a clean
/// [`GateOutcome::Unmet`] verdict) — a tool failure (exit 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateError {
    /// The underlying reason (board read / parse).
    pub reason: String,
}

impl std::fmt::Display for GateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "realised-gate observation failed: {}", self.reason)
    }
}

impl std::error::Error for GateError {}

/// A tool failure — attest could not run to a verdict (exit 2). Distinct from a
/// refusal, which IS a verdict (attest ran and refused, exit 1).
#[derive(Debug)]
pub enum AttestError {
    /// The workspace could not be read or parsed.
    Corpus(String),
    /// The target page is not in the corpus.
    TargetNotFound(String),
    /// The realised-gate observation faulted.
    Gate(String),
    /// The CAS-guarded pin write refused or failed (the page drifted, or I/O).
    Write(String),
}

impl std::fmt::Display for AttestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AttestError::Corpus(m) => write!(f, "cannot read the workspace: {m}"),
            AttestError::TargetNotFound(p) => write!(f, "target page not in the corpus: {p}"),
            AttestError::Gate(m) => write!(f, "realised-gate faulted: {m}"),
            AttestError::Write(m) => write!(f, "attest write failed: {m}"),
        }
    }
}

impl std::error::Error for AttestError {}

/// The outcome of an attest run — attest either attested (pinned / re-pinned /
/// idempotent no-op) or refused atomically. Both are verdicts; only an
/// [`AttestError`] is a tool failure.
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AttestResult {
    /// The realised-gate held and the pin landed (re-pinned drift, greened grey,
    /// or was a byte-identical idempotent no-op) — exit 0.
    Attested(AttestReport),
    /// Attest refused — class (a)/(c) via the pin, or class (b) the realised
    /// -gate. Nothing landed. On a `dry` run this is a `would_refuse` (exit 1
    /// all the same).
    Refused(AttestRefusal),
}

/// The report of a successful (or idempotent) attest — the underlying pin
/// report, carried verbatim (attest adds no render of its own on the record
/// path; the receipt IS the pin's).
#[derive(Debug, Serialize)]
pub struct AttestReport {
    /// The attested page path.
    pub target: String,
    /// The pin that landed — per-item colors / revs, `wrote`, `dry`, `receipt`.
    pub pin: pin::PinReport,
}

/// An atomic attest refusal. Class (b) is the realised-gate; classes (a)/(c) are
/// the pin's own per-item refusal, carried verbatim (rows 1-4).
#[derive(Debug, Serialize)]
#[serde(tag = "class", rename_all = "snake_case")]
pub enum AttestRefusal {
    /// Class (b) — the realised-gate is unmet (row 5, `realised_gate{claim,state}`).
    RealisedGate(GateRefusal),
    /// Class (a)/(c) — the pin refused atomically (rows 1-4); nothing landed.
    Pin(pin::PinRefusal),
}

/// A realised-gate refusal (class (b), taxonomy row 5): the named claim, its
/// state, its last receipt, and the `(code, recovery)` pair.
#[derive(Debug, Serialize)]
pub struct GateRefusal {
    /// The attested page path.
    pub target: String,
    /// The unrealised claim selector.
    pub claim: String,
    /// The claim's terminal state — one of [`UNMET_STATES`].
    pub state: String,
    /// The claim's last receipt anchor; `None` when it minted none.
    pub last_receipt: Option<String>,
    /// The §8 code — `realised_gate` (row 5).
    pub code: &'static str,
    /// The §8 recovery class — `retry` (converge, then re-attest).
    pub recovery: &'static str,
    /// `true` when this is a `dry` rehearsal (`would_refuse`), false for a real
    /// refusal — the render differs, the atomicity does not.
    pub would_refuse: bool,
}

impl AttestRefusal {
    /// Whether this refusal is a `dry` rehearsal (`would_refuse`) rather than a
    /// real refusal — uniform over both classes for the CLI render.
    #[must_use]
    pub fn would_refuse(&self) -> bool {
        match self {
            AttestRefusal::RealisedGate(g) => g.would_refuse,
            AttestRefusal::Pin(p) => p.would_refuse,
        }
    }

    /// The attested page path — uniform over both classes.
    #[must_use]
    pub fn target(&self) -> &str {
        match self {
            AttestRefusal::RealisedGate(g) => &g.target,
            AttestRefusal::Pin(p) => &p.target,
        }
    }
}

/// **Attest the target page** (d2 §2.5 A7/E1): evaluate the realised-gate, then —
/// only if it holds — pin. The gate runs FIRST and refuses class (b) before any
/// bytes move, so a failed gate writes nothing; classes (a)/(c) ride pin's own
/// atomic fail-closed resolve + `check_claim`. Drifted pins re-pin (never refuse),
/// grey greens, and a landed pin mints ONE receipt through pin's single write
/// path. See the module docs for the full contract.
///
/// # Errors
/// [`AttestError`] on a tool failure — an unreadable workspace, a missing target
/// page, a faulting gate observation, or a refused/failed CAS write. An attest
/// that RAN and refused is `Ok(AttestResult::Refused)`, not an error.
pub fn attest(
    root: &fs::WorkspaceRoot,
    target: &str,
    gate: &dyn RealisedGate,
    opts: &AttestOptions,
) -> Result<AttestResult, AttestError> {
    // (b) The realised-gate — a PURE read, evaluated FIRST. An unmet gate refuses
    // and writes nothing (before the pin runs, so no re-pin lands over
    // unconverged content). `dry` renders it as a `would_refuse`.
    match gate
        .evaluate(root, target)
        .map_err(|e| AttestError::Gate(e.reason))?
    {
        GateOutcome::Unmet(u) => {
            return Ok(AttestResult::Refused(AttestRefusal::RealisedGate(
                GateRefusal {
                    target: target.to_string(),
                    claim: u.claim,
                    state: u.state,
                    last_receipt: u.last_receipt,
                    code: REALISED_GATE_CODE,
                    recovery: REALISED_GATE_RECOVERY,
                    would_refuse: opts.dry,
                },
            )));
        }
        GateOutcome::Realised => {}
    }

    // (a)/(c) + record: the pin resolves atomically (rows 1-3), evaluates any
    // declared check_claim (row 4), re-pins drift, greens grey, and mints one
    // receipt through the ONE write path. attest composes it — never a fork.
    let pin_opts = pin::PinOptions {
        dry: opts.dry,
        actor: opts.actor.clone(),
        now: opts.now.clone(),
    };
    match pin::pin(root, target, &pin_opts).map_err(map_pin_error)? {
        pin::PinResult::Pinned(report) => Ok(AttestResult::Attested(AttestReport {
            target: target.to_string(),
            pin: report,
        })),
        pin::PinResult::Refused(refusal) => Ok(AttestResult::Refused(AttestRefusal::Pin(refusal))),
    }
}

/// Map a [`pin::PinError`] tool failure onto the attest surface (same three
/// classes — attest adds no write path, so it adds no failure mode).
fn map_pin_error(e: pin::PinError) -> AttestError {
    match e {
        pin::PinError::Corpus(m) => AttestError::Corpus(m),
        pin::PinError::TargetNotFound(p) => AttestError::TargetNotFound(p),
        pin::PinError::Write(m) => AttestError::Write(m),
    }
}
