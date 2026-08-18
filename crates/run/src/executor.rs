//! Shared executor — the ONE write path both dispatch paths converge on
//! (decision #4; verdict ruling 2). md.* descriptors → cap validation at the
//! choke point → one atomic splice batch → receipt in the same commit →
//! apply→event synthesis with real post-apply fingerprints.
//!
//! # Laws
//! - **Choke point (#13):** every descriptor validated against the block's
//!   [`CapSet`] before any I/O; one violation refuses the whole batch.
//! - **One batch (ruling 2):** all edits + receipt ride one
//!   `validate_batch` → `apply_batch`; no rollback path.
//! - **No guard on this door** (no-guard-on-effects ruling, 2026-08-15;
//!   `docs/run-plane.md` § the no-guard amendment): no world pin, no CAS
//!   premise, no per-target pin-and-verify. The former `if_root` self-pin
//!   (#19) and the foreign-edit law (#26) are RETIRED — a foreign advance
//!   re-derives and proceeds, and no refusal on this door is a premise
//!   refusal. [`ApplyRequest::observed_root`] is receipt provenance only.
//! - **Self-guards (#9):** every edit carries `if_node_rev` planned from the
//!   same locked load the batch validates against; load→validate→commit
//!   under workspace flock ([`WorkspaceLock`], `LOCK_NB`).
//! - **§9:** `invocation_id` and `now` are caller-supplied; no clock here.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::time::{Duration, Instant};

use effects::{ArgValue, ChangeEvent, Domain, Effect, EffectKind, EventFacts, Provenance};
use model::{
    Document, Edit, EditKind, HpathSeg, MerkleRoot, NodeKind, NodeRev, PutAt, ReceiptAppend, Ref,
    SpliceRequest, SpliceVerdict, delta,
};
use serde::{Deserialize, Serialize};

use crate::caps::{Authority, Cap};
use crate::record::{ExecRecord, ExecRecordSink};

/// The run plane's receipt file — ONE convention, whichever door invoked the
/// plane (the CLI entry and the § A.8 wire arm both append here). Promoted
/// from the CLI (2026-08-13, § A.8) so the convention has one home.
pub const RECEIPT_FILE: &str = "receipts/run.md";

/// Where the run receipt lands: a workspace-relative file (appended) and the
/// pre-minted block anchor for the line. Address policy is the CALLER's (U5
/// convention); the executor renders and folds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptAddr {
    /// Workspace-relative receipt file path (created on first append).
    pub path: String,
    /// The line's block anchor id (e.g. `r-000042`), caller-minted.
    pub anchor: String,
}

/// One apply request: the md.* descriptors of one generation, the authority
/// governing them, and the root pins. See the module docs for the laws each
/// field serves.
#[derive(Debug)]
pub struct ApplyRequest<'a> {
    /// The page the effects apply to (workspace-relative).
    pub page: &'a str,
    /// The task name — the receipt actor is `run:<task>`.
    pub task: &'a str,
    /// The addressed task block's `node_rev` at eval/address time — the
    /// procedure-hash the receipt attests (WHICH code ran, not just the
    /// mutable task NAME). The caller threads it from the resolved block.
    pub task_rev: &'a str,
    /// Caller-supplied invocation id (§9).
    pub invocation_id: &'a str,
    /// Caller-supplied time fact (§9); absent stays absent, never invented.
    pub now: Option<&'a str>,
    /// The md.* effect descriptors to apply, in emission order.
    pub effects: &'a [Effect],
    /// The block's resolved authority — the choke point validates against
    /// exactly this. An unsandboxed shell admits every descriptor, because
    /// there is no gate to apply, not because everything was granted.
    pub authority: &'a Authority,
    /// The corpus root the effects were produced against (root-at-eval, or
    /// the window baseline on the bash path) — RECEIPT PROVENANCE ONLY
    /// (observation honesty). Never validated: this door holds no world pin
    /// (no-guard-on-effects ruling, 2026-08-15).
    pub observed_root: &'a MerkleRoot,
    /// Receipt address; `None` skips the receipt (dispatch paths always
    /// pass one).
    pub receipt: Option<ReceiptAddr>,
    /// The bash step's exec facts (U13), threaded at render so the COMMITTED
    /// receipt line carries them — `render_receipt` commits internally, so a
    /// post-hoc `fill_exec` cannot reach the committed line. `None` on the
    /// hermetic path, on phase-1 pre-exec receipts (no child has run yet),
    /// and on cascade generations. The receipt field stays skip-if-none
    /// (#27 freeze clock — no new required wire field).
    pub exec: Option<&'a ExecRecord>,
    /// Caller-supplied identity (§9, § A.8): threads into the receipt's
    /// `actor` fact. `None` keeps the plane's `run:<task>` self-label, so
    /// every CLI receipt byte stands unchanged.
    pub actor: Option<&'a str>,
    /// The cascade generation of the effects being applied (`0` for the run
    /// itself); the synthesized event carries `depth + 1`.
    pub depth: u32,
    /// Delta honesty (§ A.8 run-delta ruling): the host's frame mint,
    /// offered the facts of each committed batch UNDER the workspace flock.
    /// `None` is the CLI entry — a separate process with no ring in reach —
    /// whose commits stay external change by the flock ruling.
    pub delta: Option<&'a dyn DeltaSink>,
    /// § A.2.1 opaque passthrough for `md.create` births: delivered verbatim
    /// to the create door as `ctx.fields` (armed middleware reads them; the
    /// run plane interprets NO key). Empty on the CLI entry today; the § A.8
    /// wire arm threads the frame's optional `fields` (cap `run.fields`).
    pub fields: &'a BTreeMap<String, String>,
    /// The workspace ring for door-committed births (`wire_serve::seq`), so
    /// a daemon-side birth is numbered like any door write. `None` on the
    /// CLI entry — its commits stay external change, exactly as `delta`.
    pub birth_seq: Option<&'a dyn wire_serve::seq::SeqSink>,
}

/// The host seam of the run-delta ruling (§ A.8 Delta honesty): the daemon
/// hands one of these down the dispatch chain, and the executor offers it the
/// facts of every committed batch so the host can mint the batch's Delta
/// frame and advance its ring — inside the same flock as the commit, which is
/// what closes the detector-misattribution window.
///
/// The trait lives in the plane (model-grain facts only, no wire types) so
/// the plane stays free of the wire graph; the one production implementor is
/// the registry's ring sink.
pub trait DeltaSink: std::fmt::Debug {
    /// One committed batch's facts. Called after `fs::apply_batch` returned,
    /// with the caller's workspace flock still held. Infallible by contract:
    /// a mint failure is the host's to degrade (its detector will still
    /// reconcile), never a reason to misreport a landed commit.
    fn committed(&self, root: &fs::WorkspaceRoot, facts: &CommitFacts<'_>);
}

/// What one committed batch changed — the before tense held in memory by the
/// executor, the after tense as the exact candidate `fs` landed (D8/U31: the
/// dry bytes ARE the committed bytes), and the workspace root the batch
/// advanced from, folded under the flock immediately before the commit.
#[derive(Debug)]
pub struct CommitFacts<'a> {
    /// The content page the batch spliced (workspace-relative).
    pub page: &'a str,
    /// The page before the batch (the step-2 load under the lock).
    pub before: &'a Document,
    /// The page after the batch — the committed candidate, no re-read.
    pub after: &'a Document,
    /// The receipt file the batch appended to (`None`: receipt-less apply).
    pub receipt_path: Option<&'a str>,
    /// The receipt file before the batch (`None`: first append creates it).
    pub receipt_before: Option<&'a Document>,
    /// The workspace root before the commit, folded under the flock.
    pub root_before: &'a MerkleRoot,
    /// The frame's identity fact, resolved by the receipt's own law (§9):
    /// the supplied actor verbatim, else the plane's `run:<task>` self-label
    /// — a governed run is never unattributable.
    pub actor: &'a str,
    /// The caller's time fact; absent stays absent, never invented.
    pub now: Option<&'a str>,
}

/// Why the canonical intent → executor adapter refused (R13 ruling § normative
/// mapping).
///
/// Every variant is LOUD by law: a payload missing a required key for its action, or
/// carrying a key the action does not use, refuses the whole generation — never a
/// defaulted or partial write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdapterError {
    /// The intent's `action` is not an `md.*` executor descriptor kind. `proto.*`
    /// intents never pass through the Markdown adapter, so the production slice-1
    /// allowlist is untouched by this seam.
    NonMdAction {
        /// The rule whose hook emitted the intent.
        rule_id: String,
        /// The action the intent declared.
        action: String,
    },
    /// The action requires a key the intent did not carry.
    MissingKey {
        /// The rule whose hook emitted the intent.
        rule_id: String,
        /// The action whose shape is incomplete.
        action: String,
        /// The `Intent` field that must be present for this action.
        key: &'static str,
    },
    /// The intent carried a key the action does not use.
    UnusedKey {
        /// The rule whose hook emitted the intent.
        rule_id: String,
        /// The action that has no use for the key.
        action: String,
        /// The `Intent` field that must be absent for this action.
        key: &'static str,
    },
}

impl std::fmt::Display for AdapterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AdapterError::NonMdAction { rule_id, action } => write!(
                f,
                "intent from `{rule_id}` names action '{action}', which is not an md.* descriptor — the Markdown adapter carries md.set_field and md.append_section only"
            ),
            AdapterError::MissingKey {
                rule_id,
                action,
                key,
            } => write!(
                f,
                "intent from `{rule_id}` for action '{action}' is missing `{key}` — the adapter defaults nothing"
            ),
            AdapterError::UnusedKey {
                rule_id,
                action,
                key,
            } => write!(
                f,
                "intent from `{rule_id}` for action '{action}' carries `{key}`, which that action does not use"
            ),
        }
    }
}

impl std::error::Error for AdapterError {}

/// One generation of validated [`policy::Intent`]s, adapted to the exact
/// [`ApplyRequest`] production executes (R13 ruling §1–§2) — the one seam
/// between the WHEN plane's canonical intent and the HOW plane's descriptor
/// batch (`policy` stays I/O-free; `run` owns HOW).
///
/// The mapping is mechanical and 1:1:
///
/// | [`policy::Intent`] | executor descriptor |
/// |---|---|
/// | `action` | the descriptor kind — `md.set_field` / `md.append_section` |
/// | `target` | the addressed node — planner `field` / `section` |
/// | `payload` | what lands there — planner `value` / `content` |
/// | receipt address | this request's [`ReceiptAddr`] — one call site, no second plumbing |
///
/// The adapter **re-validates nothing** — canonical action, receipt and
/// argument surface were already checked by `policy::intent_from_effect`;
/// only shape-completeness happens here. [`Provenance`] and cascade depth are
/// carried through from the emitting event by the caller.
#[derive(Debug, Clone, PartialEq)]
pub struct IntentApply {
    effects: Vec<Effect>,
    receipt: ReceiptAddr,
}

impl IntentApply {
    /// Adapt one generation of validated intents plus the receipt address the same
    /// [`ApplyRequest`] will carry.
    ///
    /// # Errors
    /// [`AdapterError`] — an action outside the `md.*` descriptor surface, or an
    /// intent whose key shape does not complete its action. Nothing is adapted in
    /// either case: one generation is one batch, so one bad intent refuses all of it.
    pub fn from_intents(
        intents: &[policy::Intent],
        receipt: ReceiptAddr,
        provenance: &Provenance,
        depth: u32,
    ) -> Result<Self, AdapterError> {
        let effects = intents
            .iter()
            .map(|intent| effect_from_intent(intent, provenance, depth))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { effects, receipt })
    }

    /// The adapted descriptors, in emission order.
    #[must_use]
    pub fn effects(&self) -> &[Effect] {
        &self.effects
    }

    /// The [`ApplyRequest`] production executes for this generation: `base` supplies
    /// the run facts and this generation supplies exactly TWO of them — the
    /// descriptors and the receipt address they ride with.
    ///
    /// Every other field is threaded explicitly rather than through `..base`, so a
    /// new [`ApplyRequest`] field is a compile error here: a fact the adapter must
    /// consciously route, never one it silently defaults.
    #[must_use]
    pub fn request<'a>(&'a self, base: &ApplyRequest<'a>) -> ApplyRequest<'a> {
        ApplyRequest {
            page: base.page,
            task: base.task,
            task_rev: base.task_rev,
            invocation_id: base.invocation_id,
            now: base.now,
            effects: &self.effects,
            authority: base.authority,
            observed_root: base.observed_root,
            receipt: Some(self.receipt.clone()),
            exec: base.exec,
            actor: base.actor,
            depth: base.depth,
            delta: base.delta,
            fields: base.fields,
            birth_seq: base.birth_seq,
        }
    }
}

/// The mechanical [`policy::Intent`] → [`Effect`] mapping (R13 ruling §2).
fn effect_from_intent(
    intent: &policy::Intent,
    provenance: &Provenance,
    depth: u32,
) -> Result<Effect, AdapterError> {
    let kind = effects::action_kind(&intent.action)
        .filter(|kind| kind.domain() == Domain::Md)
        .ok_or_else(|| AdapterError::NonMdAction {
            rule_id: intent.rule_id.clone(),
            action: intent.action.clone(),
        })?;
    // `Intent.action` selects the descriptor kind 1:1, and the kind selects which
    // planner keys `target` and `payload` become.
    let (node_key, content_key) = match kind {
        EffectKind::SetField => ("field", "value"),
        EffectKind::AppendSection => ("section", "content"),
        _ => unreachable!("md.* kinds are SetField | AppendSection"),
    };
    let missing = |key| AdapterError::MissingKey {
        rule_id: intent.rule_id.clone(),
        action: intent.action.clone(),
        key,
    };
    let node = intent.target.clone().ok_or_else(|| missing("target"))?;
    let content = intent.payload.clone().ok_or_else(|| missing("payload"))?;
    // `severity` is a proto-plane classification. An md.* descriptor has nowhere to
    // put it, and silently dropping it would be the defaulted write the ruling forbids.
    if intent.severity.is_some() {
        return Err(AdapterError::UnusedKey {
            rule_id: intent.rule_id.clone(),
            action: intent.action.clone(),
            key: "severity",
        });
    }
    let mut args = BTreeMap::new();
    args.insert(node_key.to_owned(), ArgValue::Str(node));
    args.insert(content_key.to_owned(), ArgValue::Str(content));
    Ok(Effect {
        kind,
        rule_id: intent.rule_id.clone(),
        seq: intent.seq,
        depth,
        provenance: provenance.clone(),
        args,
    })
}

/// A committed apply: what landed and the facts the runner reports.
#[derive(Debug, Clone, PartialEq)]
pub struct Applied {
    /// How many descriptors were applied (all of them — the batch is atomic).
    pub applied: usize,
    /// The apply→event synthesis: the semantic change this batch caused, with
    /// REAL post-apply fingerprints (`None` when the batch was a no-op).
    pub event: Option<ChangeEvent>,
    /// The receipt line that rode the commit (`None` without an address).
    pub receipt_line: Option<String>,
    /// The page's post-apply file rev.
    pub file_rev_after: String,
}

/// Why the executor refused. Every variant applied NOTHING — the batch is
/// atomic and there is no partial state and no rollback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecError {
    /// A non-md descriptor reached the executor — a dispatch bug, refused loud.
    NonMdEffect { kind: String },
    /// A descriptor's kind/target is not admitted by the block's caps.
    CapDenied {
        kind: String,
        target: String,
        /// The ceiling that took a cap which WOULD have admitted this
        /// descriptor, when one did. `None` is not "unknown" — it is the
        /// measured absence of a ceiling cause (deny-default, or a grant that
        /// never held the cap), and the refusal then names the measured
        /// grants and the `task.<name>.caps` declaration that grants.
        ceiling: Option<String>,
        /// The effective grants the resolution measured — the deny-default
        /// arm's teachable facts (dogfood r3 gap 6b). Empty for a task that
        /// declares no caps.
        declared: Vec<String>,
    },
    /// A descriptor argument is missing or wrongly shaped (kernel constructors
    /// make this unreachable; hand-built descriptors fault here).
    BadDescriptor { kind: String, reason: String },
    /// `md.append_section` names a section absent from the page.
    SectionNotFound { section: String },
    /// `md.append_section` names a heading appearing more than once.
    SectionAmbiguous { section: String, count: usize },
    /// The create door refused an `md.create` birth (occupied path, armed
    /// refusal, bad path, …) — the door's own error, carried verbatim. The
    /// whole generation refuses; births realized EARLIER in the same
    /// generation stay landed (no-rollback, decision #14) and are attested
    /// by the door's own frames.
    BirthRefused { path: String, detail: String },
    /// Another run holds the workspace lock (decision #9: `LOCK_NB` — a fast
    /// typed refusal, never a wait; a hung holder can never make callers hang).
    WorkspaceBusy,
    /// Any other typed validation refusal (CAS, no-match, would-corrupt,
    /// overlap, …) — carried as the verdict's debug shape.
    Refused { verdict: String },
    /// Page load failure (missing, non-UTF-8, I/O).
    Page { path: String, reason: String },
    /// Lock, receipt-scan, or commit I/O failure.
    Io { reason: String },
    /// The armed change plane REFUSED this apply (U4.2): the workspace's own
    /// attested law blocked the change the run plane produced, before any byte
    /// landed. The run plane lands bytes through `fs::apply_batch` (not the wire
    /// choke-point), so it mounts the SAME gate — byte-landing parity. `detail`
    /// names the rule and cites the legal path, or renders the armed-law fault in
    /// `policy::ArmedFault`'s own words.
    ArmedRefusal { detail: String },
    /// The apply would put an `@fp` decoration token in a claim-link position
    /// (R32 (3)): a fingerprint claim nobody minted. Tokens the batch's
    /// own payloads carry are STRIPPED, silently and by law — this variant is
    /// what is left when the strip cannot place a claim, which is never a write
    /// the engine may guess at. `cause` names which case.
    FpClaim { page: String, cause: String },
    /// The apply would change the page's `meridian-lock` bytes (R25):
    /// the run plane mints no pin, so any change to the attestation artifact is
    /// a claim nobody computed. The wire choke-point carries the same guard —
    /// this door bypasses `splice`, so the guard is mounted here too.
    LockArtifact { page: String },
}

impl std::fmt::Display for ExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecError::NonMdEffect { kind } => {
                write!(f, "executor applies md.* only, got '{kind}'")
            }
            ExecError::CapDenied {
                kind,
                target,
                ceiling,
                declared,
            } => {
                write!(f, "capability denied: {kind} on '{target}'")?;
                // run-plane § capabilities: a block whose own frontmatter
                // declares the cap reads this refusal as the engine ignoring a
                // grant that is plainly on the page, and derives a remedy
                // already in place. Naming the ceiling is the only remedy that
                // repairs THIS denial — and it is taught only where the
                // resolution measured one.
                if let Some(ceiling) = ceiling {
                    return write!(
                        f,
                        " — the grant was narrowed away by {ceiling}. Conventions narrow only, \
                         never widen, so declaring the cap again cannot lift it. Fix: widen or \
                         remove that ceiling entry, or aim the effect inside what it leaves."
                    );
                }
                // Deny-by-default (dogfood r3 gap 6b): name WHY, the grants
                // the resolution measured, and the declaration that grants —
                // the caller repairs the page, not the engine's source.
                write!(
                    f,
                    " — only declared capabilities are granted, and no declared cap \
                     covers this effect: "
                )?;
                if declared.is_empty() {
                    write!(f, "the task declares no md.* capabilities")?;
                } else {
                    write!(f, "the task's grants are [{}]", declared.join(", "))?;
                }
                write!(
                    f,
                    ". Fix: add `{kind}:{target}` (or an untargeted `{kind}`) to the \
                     task's `task.<name>.caps` list in the page's frontmatter."
                )
            }
            ExecError::BadDescriptor { kind, reason } => {
                write!(f, "bad {kind} descriptor: {reason}")
            }
            ExecError::SectionNotFound { section } => write!(f, "no section '{section}'"),
            ExecError::SectionAmbiguous { section, count } => {
                write!(f, "section '{section}' appears {count} times (ambiguous)")
            }
            ExecError::BirthRefused { path, detail } => {
                write!(f, "birth refused at {path}: {detail}")
            }
            ExecError::WorkspaceBusy => write!(
                f,
                "workspace busy: another run holds the lock — retry when it exits"
            ),
            ExecError::Refused { verdict } => write!(f, "batch refused: {verdict}"),
            ExecError::Page { path, reason } => write!(f, "page {path}: {reason}"),
            ExecError::Io { reason } => write!(f, "io: {reason}"),
            ExecError::ArmedRefusal { detail } => {
                write!(f, "armed change refused: {detail}")
            }
            ExecError::FpClaim { page, cause } => write!(
                f,
                "@fp refused in {page}: {cause}. `@green.…` after a block ref is a render-face decoration the engine mints on read, never storable content (S10) — write the plain `[[page#^id]]` address and let the tone and digest be computed. Nothing applied"
            ),
            ExecError::LockArtifact { page } => write!(
                f,
                "meridian-lock refused: the lock block in {page} is the engine's attestation artifact (#8 §3) and the run plane mints no pin — a pin is minted by `mrd pin`, which fingerprints the target behind the read-mint gate. Nothing applied"
            ),
        }
    }
}

impl std::error::Error for ExecError {}

/// The workspace run lock (decision #9): an exclusive advisory `flock(2)` on
/// `.meridian/run.lock`, held from page load through the atomic rename, so two
/// local runs cannot interleave read→rename (the lost-update TOCTOU intra-
/// process CAS guards cannot see). `LOCK_NB` acquire — a held lock is
/// [`io::ErrorKind::WouldBlock`], surfaced as the fast typed
/// [`ExecError::WorkspaceBusy`] refusal; it never waits, so a hung holder can
/// never make callers hang (review C4). Released on drop — by an EXPLICIT
/// unlock, not by the fd close (see the [`Drop`] impl: relying on the close
/// leaks the lock into any concurrently forking subprocess, and the run plane
/// forks a child per dispatched task).
#[derive(Debug)]
pub struct WorkspaceLock {
    // Held open for its fd; released by the explicit `flock(LOCK_UN)` in Drop.
    file: File,
}

/// Release the lock EXPLICITLY, before the fd closes: a `flock` lock belongs
/// to the open file DESCRIPTION, and a concurrent fork in this process holds a
/// copy of the fd between fork and exec (CLOEXEC acts at exec, not fork), so
/// closing our fd would NOT release the lock. `LOCK_UN` acts on the
/// description itself — one unlock releases it however many fd copies exist.
impl Drop for WorkspaceLock {
    fn drop(&mut self) {
        // SAFETY: flock on a valid open fd we own; the fd outlives the call.
        unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
    }
}

impl WorkspaceLock {
    /// Try to acquire the exclusive workspace run lock, creating `.meridian/`
    /// and the lockfile on first use. Never blocks: a held lock returns
    /// [`io::ErrorKind::WouldBlock`] immediately (decision #9).
    ///
    /// # Errors
    /// [`io::ErrorKind::WouldBlock`] when another run holds the lock; any
    /// other I/O failure creating or locking the lockfile.
    pub fn acquire(workspace_root: &Path) -> io::Result<Self> {
        let dir = workspace_root.join(".meridian");
        std::fs::create_dir_all(&dir)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join("run.lock"))?;
        // SAFETY: flock on a valid open fd; the fd outlives the call.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { file })
    }
}

/// The machine-re-readable body of one run receipt line. Serialized as one
/// compact JSON object inside the markdown line (`- run {json} ^anchor`);
/// the format is run-plane-local (review S10), not the wire receipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptFacts {
    /// The page the batch applied to.
    pub page: String,
    /// The task that ran.
    pub task: String,
    /// Caller-supplied invocation id.
    pub invocation: String,
    /// `run:<task>`.
    pub actor: String,
    /// Caller-supplied time fact; absent stays absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub now: Option<String>,
    /// The corpus root the effects were produced against — an OBSERVATION
    /// the receipt attests, never a validated pin (no-guard-on-effects
    /// ruling, 2026-08-15). The JSON key keeps its historical spelling so
    /// every receipt byte stands.
    pub root_pin: String,
    /// The addressed task block's `node_rev` — the procedure-hash (attestation
    /// roadmap): the receipt names WHICH code ran, not just the mutable task
    /// NAME. Stamped at eval/address time, frozen into the receipt here.
    pub task_rev: String,
    /// Per-edit facts: target identity + rev transition.
    pub edits: Vec<ReceiptEdit>,
    /// The bash step's exec facts (U8/U13): threaded into the committed line
    /// at render via [`ApplyRequest::exec`]; absent on the hermetic path (no
    /// child process). The [`ExecRecordSink`] seam remains for composition
    /// outside the commit (parse-back, tests).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub exec: Option<ExecRecord>,
}

/// The U8→U4 record edge (review S10): the receipt owner adopts the sealed
/// exec facts by taking them into its optional `exec` field. Defined here so
/// the bash path can fill the record without editing this file's construction.
impl ExecRecordSink for ReceiptFacts {
    fn fill_exec(&mut self, exec: ExecRecord) {
        self.exec = Some(exec);
    }
}

/// One edit's receipt fact: which node, and its rev transition — attested
/// history, compared by nothing (the former decision-#26 foreign-edit scan
/// is RETIRED with the no-guard-on-effects ruling, 2026-08-15).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptEdit {
    /// The target's identity.
    pub target: EditTarget,
    /// The node rev the edit was validated against.
    pub before: String,
    /// The node rev after the commit.
    pub after: String,
}

/// A run-plane edit target identity — structured, join-string-free (mirrors
/// the wire's no-join-string law).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EditTarget {
    /// A frontmatter key.
    #[serde(rename = "fm")]
    FmKey(String),
    /// A section heading chain, root → governing heading.
    #[serde(rename = "sec")]
    Section(Vec<String>),
    /// A file born through the create door (`md.create`) — the value is the
    /// workspace-relative path. Its receipt row's `before` is empty (no
    /// prior node exists) and `after` is the born whole-file rev.
    #[serde(rename = "born")]
    Born(String),
}

impl EditTarget {
    /// This target with its identifier strings rendered for a receipt line.
    /// A field name or heading chain arrives as arbitrary bytes, and
    /// `serde_json` escapes neither `[` nor `@` — an undecorated pass-through
    /// would land a claim link in a claim-link position in the receipt file.
    fn rendered(&self) -> Self {
        match self {
            EditTarget::FmKey(k) => EditTarget::FmKey(receipt::render_field(k).into_owned()),
            EditTarget::Section(segs) => EditTarget::Section(
                segs.iter()
                    .map(|s| receipt::render_field(s).into_owned())
                    .collect(),
            ),
            EditTarget::Born(path) => EditTarget::Born(receipt::render_field(path).into_owned()),
        }
    }
}

/// One planned edit: the model edit plus the identity/target facts the receipt,
/// the foreign-edit check and the `@fp` strip need. `before` is the target as
/// the model resolved it at load — its rev is the foreign-edit anchor, its SPAN
/// is what attributes a candidate token back to the effect that supplied it.
struct PlannedEdit {
    edit: Edit,
    identity: EditTarget,
    before: model::Target,
}

/// Apply one generation of md.* effects to `page` as ONE atomic batch. See the
/// module docs for the full law set; the flow is: choke-point cap validation →
/// flock → load → plan edits (self-guarded) → validate → receipt → the single
/// `fs::apply_batch` commit → apply→event synthesis.
///
/// # Errors
/// [`ExecError`] — in every case NOTHING was applied.
pub fn apply(root: &fs::WorkspaceRoot, req: &ApplyRequest<'_>) -> Result<Applied, ExecError> {
    // Serialize local runs (decision #9: LOCK_NB — busy is a fast typed
    // refusal, never a wait).
    let lock = WorkspaceLock::acquire(&root.0).map_err(|e| {
        if e.kind() == io::ErrorKind::WouldBlock {
            ExecError::WorkspaceBusy
        } else {
            ExecError::Io {
                reason: format!("workspace lock: {e}"),
            }
        }
    })?;
    apply_under(&lock, root, req)
}

/// THE CHOKE POINT, one owner for both tenses: md.* only, each effect
/// admitted by the block's authority (kind + target, so target-scoped caps
/// bind for real). The apply path runs it before any I/O; the dry rehearsal
/// (`runner::rehearse`) runs the SAME admission over the same md.* partition,
/// so a rehearsal cannot pass what the apply would refuse (dogfood r2 F2).
/// An unsandboxed shell passes every descriptor: gating it would only move
/// the same write to `sed -i`, off the attested path.
///
/// # Errors
/// [`ExecError`] — the first denied or malformed descriptor, in effect order.
pub fn admit(effects: &[Effect], authority: &Authority) -> Result<(), ExecError> {
    for effect in effects {
        let (kind, target) = descriptor_surface(effect)?;
        if !authority.admits(kind, Some(&target)) {
            let ceiling = authority
                .capabilities()
                .and_then(|caps| caps.ceiling_denying(kind, Some(&target)))
                .map(ToString::to_string);
            let declared = authority
                .capabilities()
                .map(|caps| caps.effective.0.iter().map(Cap::as_string).collect())
                .unwrap_or_default();
            return Err(ExecError::CapDenied {
                kind: kind.to_owned(),
                target,
                ceiling,
                declared,
            });
        }
    }
    Ok(())
}

/// [`apply`] under a CALLER-held [`WorkspaceLock`] — the U6a two-phase seam
/// (u4-gate addendum on #19): the bash dispatcher must commit phase 1 and
/// compute `root_after_phase1` inside ONE locked window, so it holds the lock
/// across both and threads it here. The lock parameter is the proof-of-lock —
/// flock cannot re-acquire on a second fd, so a self-locking call under a
/// held lock would refuse itself as busy.
///
/// The birth lane (`md.create`, the declared-task birth cap — ruled
/// 2026-08-18): every birth goes through the CREATE DOOR, so occupied-path
/// refusal (`cas_mismatch`), armed middleware stamps (`ctx.fields` =
/// `req.fields` verbatim) and checks are the door's own, never
/// re-implemented here. Births realize BEFORE the declaring-page batch,
/// sequentially in emission order; the first refusal refuses the generation
/// (earlier landed births stay — no rollback, decision #14 — attested by the
/// door's own frames). Lock order holds: the caller holds run.lock and the
/// door takes write.lock, the one legal order.
fn realize_births(
    root: &fs::WorkspaceRoot,
    req: &ApplyRequest<'_>,
) -> Result<Vec<ReceiptEdit>, ExecError> {
    let mut births = Vec::new();
    for effect in req.effects.iter().filter(|e| e.kind == EffectKind::Create) {
        let path = str_arg(effect, "path")?;
        let body = str_arg(effect, "body")?;
        let args = wire_serve::write::CreateArgs {
            id: None,
            path: wire::Path(path.clone()),
            body,
            // §9: the receipt's own actor law — supplied verbatim, else the
            // plane's self-label — so the door's frame and the run receipt
            // name one identity.
            actor: Some(
                req.actor
                    .map_or_else(|| format!("run:{}", req.task), str::to_owned),
            ),
            now: req.now.map(str::to_owned),
            // No world pin on this door (no-guard-on-effects ruling).
            if_root: None,
            dry: false,
            fields: req.fields.clone(),
        };
        let out = wire_serve::write::create(root, req.birth_seq, &args, &[]).map_err(|e| {
            ExecError::BirthRefused {
                path: path.clone(),
                // The door's typed frame, carried whole (it has no Display).
                detail: serde_json::to_string(e.as_ref()).unwrap_or_else(|_| format!("{e:?}")),
            }
        })?;
        births.push(ReceiptEdit {
            target: EditTarget::Born(path),
            before: String::new(),
            after: out.file_rev_after.0.clone(),
        });
    }
    Ok(births)
}

/// # Errors
/// [`ExecError`] — in every case NOTHING was applied.
pub fn apply_under(
    _lock: &WorkspaceLock,
    root: &fs::WorkspaceRoot,
    req: &ApplyRequest<'_>,
) -> Result<Applied, ExecError> {
    // 1. THE CHOKE POINT — before any I/O.
    admit(req.effects, req.authority)?;

    // 1b. THE BIRTH LANE — see [`realize_births`].
    let births = realize_births(root, req)?;
    let page_effects: Vec<&Effect> = req
        .effects
        .iter()
        .filter(|e| e.kind != EffectKind::Create)
        .collect();

    // 2. Load under the lock.
    let doc = fs::load(root, Path::new(req.page)).map_err(|e| ExecError::Page {
        path: req.page.to_owned(),
        reason: e.to_string(),
    })?;

    // 3. Plan edits, self-guarded with load-time revs.
    let mut planned = Vec::with_capacity(page_effects.len());
    for effect in &page_effects {
        planned.push(plan_edit(&doc, effect)?);
    }

    // 4-6. Seal the batch and the bytes it will write, `@fp`-stripped.
    let (batch, after_doc) = seal_stripped_candidate(&doc, &planned, req)?;

    // Each target's post-apply rev, read off the (post-strip) reparse.
    let after_revs: Vec<NodeRev> = planned
        .iter()
        .map(|p| after_rev(after_doc.document(), &p.edit.target))
        .collect::<Result<_, _>>()?;

    // 6b. THE LOCK ARTIFACT GUARD (R25), over the same candidate the gate
    // below reads and ordered before it: a forged claim is not a policy question.
    guard_lock_artifact(&doc, after_doc.document(), req.page)?;

    // 6c. THE ARMED-PLANE GATE (U4.2) — byte-landing parity: the run plane
    // lands bytes through `fs::apply_batch`, not the wire choke-point, so it
    // mounts the SAME `policy::gate` at this write's path before the commit.
    // A never-armed workspace is a bit-for-bit no-op.
    if let Some(detail) = crate::gate::refuse_reason(
        root,
        &doc,
        after_doc.document(),
        req.page,
        &batch.edits,
        &format!("run:{}", req.task),
    ) {
        return Err(ExecError::ArmedRefusal { detail });
    }

    // 7. Receipt (rides the same sealed commit — §6.1). Birth rows lead the
    // line, in realization order, before the page-edit rows.
    let receipt = match &req.receipt {
        Some(addr) => Some(render_receipt(
            root,
            addr,
            req,
            &planned,
            &after_revs,
            &births,
        )?),
        None => None,
    };
    let receipt_line = receipt.as_ref().map(|(_, _, line)| line.clone());
    let sealed = match model::validate_batch(
        &doc,
        None,
        &batch,
        receipt.as_ref().map(|(_, append, _)| append.clone()),
    ) {
        SpliceVerdict::Validated(b) => b,
        refused => {
            // Same inputs as step 5 plus an EOF append — unreachable refusal.
            return Err(ExecError::Refused {
                verdict: format!("{refused:?}"),
            });
        }
    };

    // 7b. The delta-mint bracket (§ A.8 run-delta ruling), only when the
    // host armed a sink: the commit and its frame mint hold the workspace
    // WRITE flock — the detector's and the wire choke-point's own
    // serialization point. The run lock alone cannot exclude the detector
    // (run applies and wire splices do not otherwise serialize), so without
    // this flock a detect cycle can classify the half-landed commit as
    // external, actorless change — the misattribution this ruling exists to
    // kill. Pre-facts fold under the same flock; nothing has landed yet, so
    // a failure here refuses cleanly.
    let _delta_bracket = match req.delta {
        Some(_) => Some(acquire_write_flock(root)?),
        None => None,
    };
    let delta_pre = delta_pre_facts(root, req)?;

    // 8. THE commit — the only write, atomic, two files (§6.5 crash window
    // accepted: content-without-receipt recovers by re-derive + lint).
    fs::apply_batch(
        root,
        Path::new(req.page),
        receipt
            .as_ref()
            .map(|(path, _, _)| Path::new(path.as_str())),
        &sealed,
        // The validated pre-image (D8): the sealed spans index `doc.raw` (the
        // step-2 load under the lock); fs splices exactly these bytes and
        // verifies the live page still carries them before the rename.
        doc.raw.as_bytes(),
        // U31: the SAME candidate the strip, the artifact guard and the armed
        // gate above all read — `fs` refuses any other.
        &after_doc,
    )
    .map_err(|e| ExecError::Io {
        reason: e.to_string(),
    })?;

    // 8b. Offer the committed batch to the host's frame mint — still under
    // the flock, so the ring advances before any detect cycle can observe
    // the moved root as unaccounted external change.
    offer_committed(root, req, &doc, after_doc.document(), delta_pre.as_ref());

    let file_rev_after = after_doc.document().root.node_rev.0.clone();

    // 9. Apply→event synthesis with REAL post-apply fingerprints (the dry
    // bytes ARE the committed bytes — no re-read).
    let event = synthesize_event(req.page, &doc, after_doc.document(), req.depth);
    Ok(Applied {
        applied: req.effects.len(),
        event,
        receipt_line,
        file_rev_after,
    })
}

/// The delta-mint bracket's flock (step 7b): the workspace WRITE lock,
/// bounded-wait — its holders (a wire splice's critical section, a detect
/// cycle) are short, so a brief retry converts the NB acquire into a bounded
/// wait; past the deadline the apply refuses the plane's own typed busy
/// word. Lock order is run.lock → write.lock on this path and no path takes
/// them the other way.
fn acquire_write_flock(root: &fs::WorkspaceRoot) -> Result<fs::WriteLock, ExecError> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match fs::WriteLock::acquire(root) {
            Ok(lock) => return Ok(lock),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err(ExecError::WorkspaceBusy);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => {
                return Err(ExecError::Io {
                    reason: format!("write flock for the delta mint: {e}"),
                });
            }
        }
    }
}

/// The frame mint's pre-commit facts (step 7b): the receipt file's before
/// tense and the workspace root the batch advances from, both under the
/// flock. `None` when no sink is armed — the CLI pays no fold.
fn delta_pre_facts(
    root: &fs::WorkspaceRoot,
    req: &ApplyRequest<'_>,
) -> Result<Option<(Option<Document>, MerkleRoot)>, ExecError> {
    if req.delta.is_none() {
        return Ok(None);
    }
    let receipt_before = match &req.receipt {
        Some(addr) => load_receipt_before(root, &addr.path)?,
        None => None,
    };
    let root_before = fs::domain_snapshot(root)
        .map(|(_, r)| r)
        .map_err(|e| ExecError::Io {
            reason: format!("pre-commit root fold: {e}"),
        })?;
    Ok(Some((receipt_before, root_before)))
}

/// Step 8b: hand the committed batch's facts to the armed sink, actor
/// resolved by the receipt's own §9 law (supplied verbatim, else the plane's
/// `run:<task>` self-label). A no-op without a sink.
fn offer_committed(
    root: &fs::WorkspaceRoot,
    req: &ApplyRequest<'_>,
    before: &Document,
    after: &Document,
    pre: Option<&(Option<Document>, MerkleRoot)>,
) {
    let (Some(sink), Some((receipt_before, root_before))) = (req.delta, pre) else {
        return;
    };
    let actor = req
        .actor
        .map_or_else(|| format!("run:{}", req.task), str::to_owned);
    sink.committed(
        root,
        &CommitFacts {
            page: req.page,
            before,
            after,
            receipt_path: req.receipt.as_ref().map(|a| a.path.as_str()),
            receipt_before: receipt_before.as_ref(),
            root_before,
            actor: &actor,
            now: req.now,
        },
    );
}

/// The receipt file's before tense for the frame mint — absence is a legal
/// state (the first append creates the file); any other read fault refuses
/// the apply before anything lands.
fn load_receipt_before(root: &fs::WorkspaceRoot, rel: &str) -> Result<Option<Document>, ExecError> {
    match fs::load(root, Path::new(rel)) {
        Ok(doc) => Ok(Some(doc)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(ExecError::Io {
            reason: format!("receipt before tense: {e}"),
        }),
    }
}

/// Steps 4-6: mint the sealed batch and the candidate document it will write,
/// with the `@fp` strip already applied — the request batch, its seal, and the
/// bytes, all three agreeing.
///
/// The batch carries NO `if_root`: this door holds no world pin (no-guard
/// ruling). The candidate is dry-applied in memory (the SAME bytes `fs` will
/// write) and the `@fp` strip (R32 (3)) rewrites the REQUEST batch: this door
/// bypasses the wire choke-point, so it carries the choke-point's law itself, and
/// every judgment after it — the post-apply revs the receipt commits, the lock
/// artifact guard, the armed gate — reads the bytes that actually land.
///
/// # Errors
/// [`ExecError::Refused`] from validation, or [`ExecError::FpClaim`] from a
/// claim token the strip cannot place. Nothing has been applied at this point
/// in any case.
fn seal_stripped_candidate(
    doc: &Document,
    planned: &[PlannedEdit],
    req: &ApplyRequest<'_>,
) -> Result<(SpliceRequest, model::CandidateDocument), ExecError> {
    let mut batch = SpliceRequest {
        if_root: None,
        edits: planned.iter().map(|p| p.edit.clone()).collect(),
        engine: None,
    };
    let mut sealed = match model::validate_batch(doc, None, &batch, None) {
        SpliceVerdict::Validated(b) => b,
        refused => {
            return Err(ExecError::Refused {
                verdict: format!("{refused:?}"),
            });
        }
    };
    let mut after_doc = crate::fp::candidate(req.page, doc, &sealed);
    let before_facts: Vec<model::Target> = planned.iter().map(|p| p.before.clone()).collect();
    crate::fp::strip_candidate(
        doc,
        &before_facts,
        req.page,
        &mut batch,
        &mut sealed,
        &mut after_doc,
    )?;
    Ok((batch, after_doc))
}

/// **The lock ARTIFACT guard** (R25) — guard the artifact, not the verb.
///
/// The read-mint gate guards `splice.pin`; this door bypasses `splice` entirely
/// and mints no pin, so ANY change to the page's `meridian-lock` bytes is an
/// attestation nobody computed. Byte-identity over `lock::block_texts`, which is
/// the one owner of "which bytes are the lock" — re-deriving that here would be a
/// second spelling of the grammar.
///
/// # Errors
/// [`ExecError::LockArtifact`] — nothing was applied.
fn guard_lock_artifact(before: &Document, after: &Document, page: &str) -> Result<(), ExecError> {
    if lock::block_texts(after) == lock::block_texts(before) {
        return Ok(());
    }
    Err(ExecError::LockArtifact {
        page: page.to_owned(),
    })
}

/// A descriptor's choke-point surface: its namespaced kind string and its
/// capability target (the field / section it touches).
fn descriptor_surface(effect: &Effect) -> Result<(&'static str, String), ExecError> {
    if effect.kind.domain() != Domain::Md {
        return Err(ExecError::NonMdEffect {
            kind: effect.kind.as_str().to_owned(),
        });
    }
    let target = match effect.kind {
        EffectKind::SetField => str_arg(effect, "field")?,
        EffectKind::AppendSection => str_arg(effect, "section")?,
        // The birth cap's capability grain is the path's FIRST segment
        // (`tasks/foo.md` → `tasks`), so `md.create:tasks` scopes a
        // directory with the cap grammar's existing exact-match semantics
        // (the target charset admits no `/`). A root-level birth's grain is
        // the path itself.
        EffectKind::Create => {
            let path = str_arg(effect, "path")?;
            path.split_once('/')
                .map_or(path.clone(), |(d, _)| d.to_owned())
        }
        _ => unreachable!("md.* kinds are SetField | AppendSection | Create"),
    };
    Ok((effect.kind.as_str(), target))
}

/// A required scalar string argument off a descriptor.
fn str_arg(effect: &Effect, key: &str) -> Result<String, ExecError> {
    match effect.args.get(key) {
        Some(ArgValue::Str(s)) => Ok(s.clone()),
        _ => Err(ExecError::BadDescriptor {
            kind: effect.kind.as_str().to_owned(),
            reason: format!("missing scalar '{key}'"),
        }),
    }
}

/// Plan one md.* descriptor as a self-guarded model edit.
fn plan_edit(doc: &Document, effect: &Effect) -> Result<PlannedEdit, ExecError> {
    match effect.kind {
        EffectKind::SetField => {
            let field = str_arg(effect, "field")?;
            let value = str_arg(effect, "value")?;
            let before = model::fm_upsert_before(doc, &field);
            // ⑤-F2 (2026-08-17): PRESERVATION ONLY. A value-identical write —
            // the stored spelling already decodes (§ A.6.1) to exactly the
            // caller's string — keeps the STORED spelling, quotes included,
            // so its bytes, and with them the field hash and `prop_rev`, do
            // not move. The raw re-composition this replaces dropped the
            // quotes the wire door had written: the parsed value
            // round-tripped while every value-identical `md.set_field` moved
            // the line's bytes, defeating idempotence-on-hash and littering
            // git with no-op diffs.
            //
            // Everything else still lands VERBATIM, deliberately — this
            // plane's contract is raw-grain ("whole-value grains that must
            // land as sent", the wire door's own words), and
            // s2fix_run_plane_fp pins both raw edges: a pre-quoted claim-link
            // value lands as sent, and the multi-line fence-close window
            // stays genuinely open (the fp strip's COMPOSED arm guards the
            // forgery through it). Encoding fresh values here (the wire
            // door's yaml_safe_value arm) would break both pins; the ONE
            // preservation predicate is shared instead
            // (policy::defs::fm_spelling_preserves), with the same
            // one-leading-space colon split fm_index reads a key line by.
            let stored_value = (before.span.start < before.span.end)
                .then(|| doc.raw[before.span.clone()].to_string())
                .and_then(|line| {
                    let colon = line.find(':')?;
                    let rest = &line[colon + 1..];
                    Some(rest.strip_prefix(' ').unwrap_or(rest).to_string())
                });
            let text = match &stored_value {
                Some(stored) if policy::defs::fm_spelling_preserves(stored, &value) => {
                    stored.trim().to_string()
                }
                _ => value,
            };
            Ok(PlannedEdit {
                edit: Edit {
                    target: Ref::FmKey(field.clone()),
                    edit: EditKind::Put {
                        at: PutAt::Upsert,
                        text,
                    },
                    if_node_rev: Some(before.node_rev.clone()),
                },
                identity: EditTarget::FmKey(field),
                before,
            })
        }
        EffectKind::AppendSection => {
            let section = str_arg(effect, "section")?;
            let content = str_arg(effect, "content")?;
            let (segs, span_end_byte, before) = find_section(doc, &section)?;
            // Append as a LINE: exactly one trailing newline; a leading one
            // only when the section's last byte is not already a terminator.
            let mut text = String::new();
            if span_end_byte != Some(b'\n') {
                text.push('\n');
            }
            text.push_str(content.trim_end_matches('\n'));
            text.push('\n');
            Ok(PlannedEdit {
                edit: Edit {
                    target: Ref::Hpath(
                        segs.iter()
                            .map(|h| HpathSeg {
                                h: h.clone(),
                                n: None,
                            })
                            .collect(),
                    ),
                    edit: EditKind::Put {
                        at: PutAt::End,
                        text,
                    },
                    if_node_rev: Some(before.node_rev.clone()),
                },
                identity: EditTarget::Section(segs),
                before,
            })
        }
        _ => Err(ExecError::NonMdEffect {
            kind: effect.kind.as_str().to_owned(),
        }),
    }
}

/// Find the UNIQUE section whose governing heading text is `heading`; returns
/// its full hpath chain, its last raw byte, and its load-time target (span +
/// rev — the same span `model::resolve` hands `validate_batch`). Zero → not
/// found; two-plus → ambiguous (the mint plane never silently picks).
fn find_section(
    doc: &Document,
    heading: &str,
) -> Result<(Vec<String>, Option<u8>, model::Target), ExecError> {
    fn collect<'a>(node: &'a model::Node, heading: &str, out: &mut Vec<&'a model::Node>) {
        if matches!(&node.kind, NodeKind::Section { heading_text, .. } if heading_text == heading) {
            out.push(node);
        }
        for c in &node.children {
            collect(c, heading, out);
        }
    }
    let mut hits: Vec<&model::Node> = Vec::new();
    collect(&doc.root, heading, &mut hits);
    match hits.as_slice() {
        [] => Err(ExecError::SectionNotFound {
            section: heading.to_owned(),
        }),
        [only] => {
            let segs = only
                .hpath
                .clone()
                .unwrap_or_else(|| vec![heading.to_owned()]);
            let last_byte = doc
                .raw
                .as_bytes()
                .get(only.span.end.wrapping_sub(1))
                .copied();
            Ok((
                segs,
                last_byte,
                model::Target {
                    span: only.span.clone(),
                    node_rev: only.node_rev.clone(),
                },
            ))
        }
        many => Err(ExecError::SectionAmbiguous {
            section: heading.to_owned(),
            count: many.len(),
        }),
    }
}

/// Render the receipt line and its EOF append for this batch.
fn render_receipt(
    root: &fs::WorkspaceRoot,
    addr: &ReceiptAddr,
    req: &ApplyRequest<'_>,
    planned: &[PlannedEdit],
    after_revs: &[NodeRev],
    births: &[ReceiptEdit],
) -> Result<(String, ReceiptAppend, String), ExecError> {
    let io_err = |e: io::Error| ExecError::Io {
        reason: format!("receipt: {e}"),
    };
    // Free-text fields go through the receipt crate's field law. `task` is
    // already an identifier (`address::declared` refuses any other shape) and
    // `invocation`/`now`/roots/revs are minted or engine hex — `page` and the
    // edit targets are the two that arrive as arbitrary bytes.
    let facts = ReceiptFacts {
        page: receipt::render_field(req.page).into_owned(),
        task: req.task.to_owned(),
        invocation: req.invocation_id.to_owned(),
        // §9 / § A.8: a supplied actor is the caller's identity; absent
        // keeps the plane's self-label and the CLI's receipt bytes.
        actor: req
            .actor
            .map_or_else(|| format!("run:{}", req.task), str::to_owned),
        now: req.now.map(str::to_owned),
        root_pin: req.observed_root.0.clone(),
        task_rev: req.task_rev.to_owned(),
        edits: births
            .iter()
            .map(|b| ReceiptEdit {
                target: b.target.rendered(),
                before: b.before.clone(),
                after: b.after.clone(),
            })
            .chain(
                planned
                    .iter()
                    .zip(after_revs)
                    .map(|(p, after)| ReceiptEdit {
                        target: p.identity.rendered(),
                        before: p.before.node_rev.0.clone(),
                        after: after.0.clone(),
                    }),
            )
            .collect(),
        // U13: the sealed exec facts enter the COMMITTED line here — the only
        // point that can put them there (this render commits internally).
        // S8 holds at this site by construction: the record exists only after
        // its log sealed (`ExecRecord.stdout` presence proves the fsync ran).
        exec: req.exec.cloned(),
    };
    let json = serde_json::to_string(&facts).map_err(|e| ExecError::Io {
        reason: format!("receipt encode: {e}"),
    })?;
    let line = format!("- run {json} ^{}", addr.anchor);
    let abs = root.0.join(&addr.path);
    if let Some(parent) = abs.parent() {
        std::fs::create_dir_all(parent).map_err(io_err)?;
    }
    let len = match std::fs::read(&abs) {
        Ok(bytes) => bytes.len(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => 0,
        Err(e) => return Err(io_err(e)),
    };
    Ok((
        addr.path.clone(),
        ReceiptAppend {
            span: len..len,
            text: format!("{line}\n"),
        },
        line,
    ))
}

/// A planned target's post-apply rev, read off the reparsed after-document.
fn after_rev(after_doc: &Document, target: &Ref) -> Result<NodeRev, ExecError> {
    match target {
        // The upserted key exists after the batch by construction.
        Ref::FmKey(key) => model::resolve(after_doc, &Ref::FmKey(key.clone()))
            .map(|t| t.node_rev)
            .map_err(|e| ExecError::Refused {
                verdict: format!("post-apply resolve of fm:{key} failed: {e:?}"),
            }),
        other => model::resolve(after_doc, other)
            .map(|t| t.node_rev)
            .map_err(|e| ExecError::Refused {
                verdict: format!("post-apply resolve failed: {e:?}"),
            }),
    }
}

/// The apply→event synthesis (decision #4): the semantic change THIS batch
/// caused, from the real before/after documents — the primitive the phase-2
/// resident cascade adopts. Deterministic, duplicate-free change sets;
/// fingerprints are the real file revs; `depth` is the applied generation
/// plus one.
#[must_use]
pub fn synthesize_event(
    page: &str,
    before: &Document,
    after: &Document,
    applied_depth: u32,
) -> Option<ChangeEvent> {
    let fd = delta::file_delta(Some(before), Some(after))?;
    let mut sections = Vec::new();
    let mut fields = Vec::new();
    for nd in &fd.nodes {
        match &nd.target {
            Ref::Hpath(segs) => sections.push(render_hpath(segs)),
            Ref::Anchor(id) => sections.push(format!("^{id}")),
            Ref::FmKey(key) => fields.push(key.clone()),
        }
    }
    sections.sort();
    sections.dedup();
    fields.sort();
    fields.dedup();
    Some(ChangeEvent {
        file: page.to_owned(),
        sections_changed: sections,
        fields_changed: fields,
        // `changes`/`facts` need values and frontmatter that only a
        // `policy::Change`'s DocFacts carry; this path synthesizes from
        // `model::delta` node entries (identities + revs, never values).
        // Empty is fail-closed: a reaction reading `event.changes` sees
        // nothing and does not fire. Note `fd.nodes` is empty whenever one
        // splice touches frontmatter AND a body section (the changed range has
        // no addressable container), so these fields are already silent there.
        changes: Vec::new(),
        facts: EventFacts::default(),
        fingerprint_before: fd.file_rev_before.map(|r| r.0).unwrap_or_default(),
        fingerprint_after: fd.file_rev_after.map(|r| r.0).unwrap_or_default(),
        depth: applied_depth + 1,
    })
}

/// Render an hpath for the event payload (`A#B`, `%n` occurrence) — the
/// mint-plane heading-path spelling.
fn render_hpath(segs: &[HpathSeg]) -> String {
    segs.iter()
        .map(|s| match s.n {
            Some(n) => format!("{}%{n}", s.h),
            None => s.h.clone(),
        })
        .collect::<Vec<_>>()
        .join("#")
}
