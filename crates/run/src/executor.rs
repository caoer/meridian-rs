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

use crate::caps::{self, Authority, Cap};
use crate::record::{ExecRecord, ExecRecordSink};

/// The run plane's receipt file — ONE convention, whichever door invoked the
/// plane (the CLI entry and the § A.8 wire arm both append here). Promoted
/// from the CLI (§ A.8) so the convention has one home.
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
    /// The task name — the plane's self-label is `run:<task>`, the fallback
    /// half of the §9 actor law ([`ApplyRequest::actor`]).
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
    /// The caller's ambient directory, workspace-relative (md-create-ambient-
    /// paths law, shape (c)): the DEFAULT resolution base a
    /// baseless `md.create` path lands under ([`resolve_birth_targets`]), and
    /// the coordinate frame [`admit`] judges page edits in (the boundary is
    /// data). `None` (the CLI entry, hosts
    /// predating cap `run.ambient`) keeps the bare-door law:
    /// workspace-root-relative.
    pub ambient: Option<&'a str>,
}

impl ApplyRequest<'_> {
    /// **The §9 actor law, in one spelling: supplied verbatim, else the
    /// plane's `run:<task>` self-label.**
    ///
    /// A function rather than the expression written out at each site, for the
    /// same reason [`seal_candidate`] is one function: a second spelling of an
    /// identity law is how two legs of ONE apply come to disagree about who is
    /// writing. That is not hypothetical here — the CHECK leg (6c) spelled it
    /// `format!("run:{}", req.task)` unconditionally while the middleware leg,
    /// the birth door, the delta sink and the receipt all resolved the
    /// supplied actor, so a fire by `agent:x` presented `agent:x` to a
    /// middleware rule and `run:<task>` to a check rule in the same write.
    /// Every leg now reads this.
    fn actor(&self) -> String {
        self.actor
            .map_or_else(|| format!("run:{}", self.task), str::to_owned)
    }
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
    /// The put frame's opaque § A.2.1 `fields` map, verbatim
    /// ([`ApplyRequest::fields`]) — the same frame the armed middleware leg
    /// saw as `ctx.fields` when it evaluated this write ([`crate::gate`]).
    ///
    /// This is a NOTIFICATION lane: a sink mints a frame from it, no rule
    /// reads it. The frame a RULE evaluates on is the middleware ctx, which
    /// the gate mount carries — the two are different mechanisms and the
    /// second is what design § 6 step 6 promises. Carried here so a sink
    /// can attribute a fire's splice the way it attributes a put, rather
    /// than having to re-derive the caller's frame from the actor alone.
    pub fields: &'a BTreeMap<String, String>,
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
            ambient: base.ambient,
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

/// Why the executor refused.
///
/// **The batch is atomic on the PAGE, and sequential on the BIRTH lane** —
/// and this doc used to claim the first about both. It read *"every variant
/// applied NOTHING"*, while [`realize_births`] calls the create door once per
/// birth in emission order and `?`-propagates on the first refusal, so births
/// realized EARLIER are on disk and stay there (decision #14, no rollback —
/// [`ExecError::BirthRefused`]'s own doc said so, three hundred lines below a
/// header that denied it). Two doc comments contradicting each other is how
/// PR 195's first review reached the wrong conclusion about `applied[]`.
///
/// What is true, and what a row builder may rely on:
///
/// - births realize FIRST, in emission order, and stop at the first refusal;
/// - the page splice (load → plan → seal → armed gate → commit) runs AFTER
///   them, so any refusal from step 2 onward — including an armed-middleware
///   veto — means NO edit landed, while births before it did;
/// - therefore [`ExecError::descriptor_index`] is enough to say which
///   descriptors landed: creates before it did, the one at it did not, and
///   nothing after it ran.
///
/// No variant carries partial PAGE state: an edit either committed with the
/// whole splice or did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecError {
    /// A non-md descriptor reached the executor — a dispatch bug, refused loud.
    NonMdEffect {
        /// The descriptor kind that should never have arrived.
        kind: String,
        /// WHICH descriptor, by index into the batch (see [`ExecError::at`]).
        index: Option<u32>,
    },
    /// A descriptor's verb/path is not admitted by the block's caps.
    CapDenied {
        /// The cap verb the descriptor authorizes under (`md.create` /
        /// `md.edit`).
        kind: String,
        /// The SESSION-ROOT-RELATIVE landing path — the coordinate cap globs
        /// match (caps-redesign ruling, 2026-08-19).
        target: String,
        /// The full workspace-relative resolved path, when a session prefix
        /// was stripped to produce `target` (`None` when they coincide).
        resolved: Option<String>,
        /// The ceiling that took a cap which WOULD have admitted this
        /// descriptor, when one did. `None` is not "unknown" — it is the
        /// measured absence of a ceiling cause (deny-default, or a grant that
        /// never held the cap), and the refusal then names the measured
        /// grants and the `task.<name>.caps` declaration that grants.
        ceiling: Option<String>,
        /// The effective grants the resolution measured — the deny-default
        /// arm's teachable facts (dogfood r3 gap 6b). Empty for a task that
        /// declares no caps.
        declared: Box<[String]>,
        /// WHICH descriptor, by index into the batch.
        index: Option<u32>,
    },
    /// A descriptor argument is missing or wrongly shaped (kernel constructors
    /// make this unreachable; hand-built descriptors fault here).
    BadDescriptor {
        /// The descriptor kind.
        kind: String,
        /// What is wrong with it.
        reason: String,
        /// WHICH descriptor, by index into the batch.
        index: Option<u32>,
    },
    /// `md.append_section` names a section absent from the page.
    SectionNotFound {
        /// The heading nobody could find.
        section: String,
        /// WHICH descriptor, by index into the batch.
        index: Option<u32>,
    },
    /// `md.append_section` names a heading appearing more than once.
    SectionAmbiguous {
        /// The heading that appears more than once.
        section: String,
        /// How many times.
        count: usize,
        /// WHICH descriptor, by index into the batch.
        index: Option<u32>,
    },
    /// The create door refused an `md.create` birth (occupied path, armed
    /// refusal, bad path, …) — the door's own error, carried verbatim. The
    /// whole generation refuses; births realized EARLIER in the same
    /// generation stay landed (no-rollback, decision #14) and are attested
    /// by the door's own frames.
    BirthRefused {
        /// The landing path the door refused.
        path: String,
        /// The door's typed frame, carried whole.
        detail: String,
        /// WHICH descriptor, by index into the batch.
        index: Option<u32>,
    },
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
    /// An armed MIDDLEWARE row emitted something this lane cannot land, so the
    /// apply refuses whole rather than dropping it (V1 limit, mirroring the
    /// birth door's own: *"the birth door admits refuse, this-file `set_field`,
    /// and send only"*).
    ///
    /// The run plane's page splice is ONE atomic batch on ONE page committed
    /// through `fs::apply_batch`; it compiles no sealed SET, so a cross-file
    /// `set_field`, a `create`, and a `send` (whose intents a fire row has no
    /// channel to carry) have nowhere to go. Silently ignoring them would let
    /// a rule believe it stamped a file it never touched — the failure mode a
    /// loud refusal exists to prevent.
    MiddlewareEmit {
        /// The armed middleware id that emitted it.
        rule: String,
        /// What it emitted and why this lane cannot land it.
        detail: String,
    },
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

impl ExecError {
    /// Stamp WHICH descriptor of the batch this refusal is about.
    ///
    /// The A8 rule: the refusal must say WHICH descriptor the door refused
    /// (index + reason token); adding the index to the refusal variant is the
    /// whole executor change and the atomicity story is unchanged. It is a
    /// LOCATOR, not new state: it
    /// records WHICH descriptor a refusal is about and changes nothing about
    /// what landed. It does **not** say "nothing landed" — the enum header
    /// above states what actually holds (no EDIT landed; births before the
    /// refused index did, and stay).
    /// It exists so a row builder names the refused descriptor by identity
    /// rather than by matching coordinates, which cannot tell two descriptors
    /// sharing a path and a verb apart.
    #[must_use]
    pub fn at(self, at: usize) -> Self {
        let at = u32::try_from(at).unwrap_or(u32::MAX);
        let stamp = |index: Option<u32>| index.or(Some(at));
        match self {
            ExecError::NonMdEffect { kind, index } => ExecError::NonMdEffect {
                kind,
                index: stamp(index),
            },
            ExecError::BadDescriptor {
                kind,
                reason,
                index,
            } => ExecError::BadDescriptor {
                kind,
                reason,
                index: stamp(index),
            },
            ExecError::SectionNotFound { section, index } => ExecError::SectionNotFound {
                section,
                index: stamp(index),
            },
            ExecError::SectionAmbiguous {
                section,
                count,
                index,
            } => ExecError::SectionAmbiguous {
                section,
                count,
                index: stamp(index),
            },
            ExecError::BirthRefused {
                path,
                detail,
                index,
            } => ExecError::BirthRefused {
                path,
                detail,
                index: stamp(index),
            },
            ExecError::CapDenied {
                kind,
                target,
                resolved,
                ceiling,
                declared,
                index,
            } => ExecError::CapDenied {
                kind,
                target,
                resolved,
                ceiling,
                declared,
                index: stamp(index),
            },
            other => other,
        }
    }

    /// WHICH descriptor this refusal is about, when it is about one.
    ///
    /// `None` is not "unknown": it is a refusal that names no single
    /// descriptor — the workspace lock, I/O, a page that will not load — and a
    /// caller must then treat the whole batch as refused rather than guess.
    #[must_use]
    pub fn descriptor_index(&self) -> Option<usize> {
        match self {
            ExecError::NonMdEffect { index, .. }
            | ExecError::BadDescriptor { index, .. }
            | ExecError::SectionNotFound { index, .. }
            | ExecError::SectionAmbiguous { index, .. }
            | ExecError::BirthRefused { index, .. }
            | ExecError::CapDenied { index, .. } => index.map(|i| i as usize),
            _ => None,
        }
    }
}

impl std::fmt::Display for ExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecError::NonMdEffect { kind, .. } => {
                write!(f, "executor applies md.* only, got '{kind}'")
            }
            ExecError::CapDenied {
                kind,
                target,
                resolved,
                ceiling,
                declared,
                ..
            } => {
                write!(f, "capability denied: {kind} on '{target}'")?;
                // The matching coordinate is the ADDRESSED one — when the
                // page was judged relative to the
                // caller's ambient, name its workspace path too, so the
                // refusal reads against the bytes the caller can see.
                if let Some(resolved) = resolved {
                    write!(
                        f,
                        " (the page's workspace path is {resolved}; cap globs match the \
                         coordinate it was addressed by)"
                    )?;
                }
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
                         never widen: a scoped grant survives only where its scope NESTS inside \
                         the ceiling's (overlap is not nesting), so declaring the cap again \
                         cannot lift it. Fix: respell the grant's scope inside that ceiling, \
                         widen or remove the ceiling entry, or aim the effect inside what it \
                         leaves."
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
                    write!(f, "the task declares no md.* capabilities.")?;
                } else {
                    write!(f, "the task's grants are [{}].", declared.join(", "))?;
                }
                // The migration teach (caps-redesign ruling): a declared
                // same-verb GLOBLESS scope that would cover this landing as
                // `<scope>/*.md` is the retired partition spelling — name the
                // respell exactly, or the caller reads a grant plainly on the
                // page as ignored.
                for legacy in declared
                    .iter()
                    .filter_map(|cap| cap.strip_prefix(&format!("{kind}:")))
                    .filter(|scope| !scope.contains('/') && !scope.contains('*'))
                {
                    if policy::glob_match(&format!("{legacy}/*.md"), target) {
                        write!(
                            f,
                            " The declared `{kind}:{legacy}` is a literal glob matching \
                             only the path `{legacy}` — partition grain is retired; spell \
                             it `{kind}:{legacy}/*.md` to cover this landing."
                        )?;
                    }
                }
                // THE LEGALITY CHECK (card cap-refusals-teach-legally). Both
                // suggested spellings are SYNTHESIZED from the denied
                // coordinate, and a coordinate the cap grammar cannot express
                // — a rooted `root:rel` spelling, a segment outside the scope
                // charset — yields a `Fix:` that `Cap::parse` refuses. Probed
                // on `ad547a7c2`: a rooted denial suggested
                // `md.create:probe-root:tasks/*.md`, so following the refusal
                // produced a different refusal. Every printed suggestion
                // round-trips first; when none is legal the refusal says so
                // and names the grant that DOES work, rather than a spelling
                // that dies at parse.
                //
                // The retirement teach above needs no check: its `legacy`
                // scope already parsed, and `{legacy}/*.md` keeps every
                // segment inside the charset by construction.
                let legal = |cap: &str| Cap::parse(cap).is_ok();
                let exact = format!("{kind}:{target}");
                let parent_glob = target
                    .rsplit_once('/')
                    .map(|(parent, _)| format!("{kind}:{parent}/*.md"))
                    .filter(|glob| legal(glob));
                let tail = " to the task's `task.<name>.caps` list in the page's \
                            frontmatter; globs match the block's DECLARED relative paths, \
                            wherever a base or ambient lands them.";
                if legal(&exact) {
                    write!(f, " Fix: add `{exact}` or a glob covering it")?;
                    if let Some(glob) = &parent_glob {
                        write!(f, " (e.g. `{glob}`)")?;
                    }
                    write!(f, "{tail}")
                } else if let Some(glob) = &parent_glob {
                    // The exact coordinate is unnameable but its parent glob
                    // is legal — the shape when only the leaf carries an
                    // out-of-charset byte.
                    write!(f, " Fix: add `{glob}`{tail}")
                } else {
                    let why = match Cap::parse(&exact) {
                        Err(caps::CapsError::BadGlob { reason, .. }) => format!(" ({reason})"),
                        _ => String::new(),
                    };
                    // The unparseable spelling is deliberately NOT printed:
                    // printing it is the defect this arm exists to close.
                    write!(
                        f,
                        " Fix: no cap SCOPE can name `{target}`{why} — a scope is a \
                         workspace-relative path glob, so scoping this verb to that \
                         coordinate refuses at parse and following it would only refuse \
                         again. Grant the unscoped `{kind}`{tail}"
                    )
                }
            }
            ExecError::BadDescriptor { kind, reason, .. } => {
                write!(f, "bad {kind} descriptor: {reason}")
            }
            ExecError::SectionNotFound { section, .. } => write!(f, "no section '{section}'"),
            ExecError::SectionAmbiguous { section, count, .. } => {
                write!(f, "section '{section}' appears {count} times (ambiguous)")
            }
            ExecError::BirthRefused { path, detail, .. } => {
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
            ExecError::MiddlewareEmit { rule, detail } => {
                write!(f, "armed middleware `{rule}` emission refused: {detail}")
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
    /// The identity fact this receipt attests, resolved by its own §9 law:
    /// the supplied actor verbatim, else the plane's `run:<task>` self-label.
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
/// [`ExecError`] — no EDIT was applied in any case (the page splice is one
/// atomic batch). Births are the exception the enum's own header now states:
/// they realize first, one door call each in emission order, so a refusal at
/// descriptor `i` leaves every birth before `i` committed.
/// [`ExecError::descriptor_index`] names `i`.
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

/// The coordinate an EDIT authorizes in (the
/// boundary is DATA, never a layout pattern): the page path relative to the
/// caller's ambient directory when the page lies under it — literal prefix
/// arithmetic on the ambient the frame carried — else the page's
/// workspace-relative path unchanged (the bare-door degenerate case). This is
/// what makes one `md.edit:tasks/*.md` grant read identically on every
/// session's board and on the root board.
fn edit_coordinate<'a>(page: &'a str, ambient: Option<&str>) -> &'a str {
    ambient
        .and_then(|dir| page.strip_prefix(dir))
        .and_then(|rest| rest.strip_prefix('/'))
        .filter(|rest| !rest.is_empty())
        .unwrap_or(page)
}

/// THE CHOKE POINT, one owner for both tenses: md.* only, each effect
/// admitted by the block's authority (verb + path, so scoped caps bind for
/// real). It judges DECLARED coordinates: an
/// `md.create` matches the `path` argument exactly as the block declared it —
/// the resolution base (`base` arg / `ambient`) is a separate axis this gate
/// never joins in — and an `md.edit` matches the page in the coordinates it
/// was addressed by ([`edit_coordinate`]). All targeting lanes therefore
/// present ONE string to one glob: `md.create:tasks/*.md` covers the ambient
/// board, a `base`-targeted board, and the root board alike. It runs BEFORE
/// birth-target resolution — admission needs nothing resolved, and a denial
/// deliberately outranks a resolution fault. The apply path runs it before
/// any I/O; the dry rehearsal (`runner::rehearse`) runs the SAME admission
/// over the same md.* partition, so a rehearsal cannot pass what the apply
/// would refuse (dogfood r2 F2). An unsandboxed shell passes every
/// descriptor: gating it would only move the same write to `sed -i`, off the
/// attested path.
///
/// # Errors
/// [`ExecError`] — the first denied or malformed descriptor, in effect order.
pub fn admit(
    page: &str,
    ambient: Option<&str>,
    effects: &[Effect],
    authority: &Authority,
) -> Result<(), ExecError> {
    let page_coordinate = edit_coordinate(page, ambient);
    for (index, effect) in effects.iter().enumerate() {
        let (verb, coordinate) = descriptor_surface(page_coordinate, effect)?;
        if !authority.admits(verb, Some(&coordinate)) {
            let ceiling = authority
                .capabilities()
                .and_then(|caps| caps.ceiling_denying(verb, Some(&coordinate)))
                .map(ToString::to_string);
            let declared: Box<[String]> = authority
                .capabilities()
                .map(|caps| caps.effective.0.iter().map(Cap::as_string).collect())
                .unwrap_or_default();
            return Err(ExecError::CapDenied {
                kind: verb.to_owned(),
                target: coordinate,
                resolved: (verb == caps::VERB_EDIT && page_coordinate != page)
                    .then(|| page.to_owned()),
                ceiling,
                declared,
                index: Some(u32::try_from(index).unwrap_or(u32::MAX)),
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
/// The birth lane (`md.create`, the declared-task birth cap): every birth
/// goes through the CREATE DOOR, so occupied-path
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
    effects: &[Effect],
) -> Result<Vec<ReceiptEdit>, ExecError> {
    let mut births = Vec::new();
    for (index, effect) in effects
        .iter()
        .enumerate()
        .filter(|(_, e)| e.kind == EffectKind::Create)
    {
        // The index is the descriptor's position in the WHOLE batch, not
        // among the births, because that is what an `applied[]` row is keyed
        // by. Births realize in emission order and stop at the first refusal,
        // so this index alone tells a row builder which births are already on
        // disk (decision #14: no rollback) and which never ran.
        let path = str_arg(effect, "path").map_err(|e| e.at(index))?;
        let body = str_arg(effect, "body").map_err(|e| e.at(index))?;
        let props = props_arg(effect).map_err(|e| e.at(index))?;
        let args = wire_serve::write::CreateArgs {
            id: None,
            path: wire::Path(path.clone()),
            body,
            // §9: the receipt's own actor law, so the door's frame and the run
            // receipt name one identity.
            actor: Some(req.actor()),
            now: req.now.map(str::to_owned),
            // No world pin on this door (no-guard-on-effects ruling).
            if_root: None,
            dry: false,
            fields: req.fields.clone(),
            // D6: carried as DATA to the door, which serializes it.
            props,
        };
        let out = create_waiting_out_busy(root, req.birth_seq, &args).map_err(|e| {
            ExecError::BirthRefused {
                path: path.clone(),
                index: Some(u32::try_from(index).unwrap_or(u32::MAX)),
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

/// The birth lane's door call, under the same waiting policy the delta-mint
/// bracket takes ([`busy_wait_budget`]): a `workspace_busy` frame is retried
/// every 10 ms until the budget is spent, then carried whole to the caller as
/// the door minted it.
///
/// Retrying the DOOR (not just the flock) is what makes the birth lane wait at
/// all — `create` owns its own `LOCK_NB` acquire, and it takes that lock
/// before it reads a byte or writes one, so a busy refusal leaves the corpus
/// bit-identical and a retry is the same call, never a second write path. No
/// other refusal code is retried: they are verdicts about the request, and
/// repeating the request cannot change them.
fn create_waiting_out_busy(
    root: &fs::WorkspaceRoot,
    seq: Option<&dyn wire_serve::seq::SeqSink>,
    args: &wire_serve::write::CreateArgs,
) -> Result<wire_serve::write::CreateOutcome, Box<wire::ErrorBody>> {
    let deadline = Instant::now() + busy_wait_budget();
    loop {
        match wire_serve::write::create(root, seq, args, &[]) {
            Err(e) if e.code == wire::ErrorCode::WorkspaceBusy && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            outcome => return outcome,
        }
    }
}

/// Resolve every `md.create` birth target in `effects` to the workspace-
/// relative path it will LAND at — the face path law carried onto the birth
/// lane, with the boundary carried as DATA: the
/// declared `path` composes under the descriptor's own `base` when one rides
/// it, under the caller's ambient directory otherwise
/// (md-create-ambient-paths, shape (c)), and stays workspace-root-relative
/// under neither — the bare-door law. The one grammar opinion is
/// [`wire_serve::write::resolve_birth_target`].
///
/// Returns `None` when no effect is a birth — the caller keeps its slice —
/// and the rewritten list otherwise, so the birth lane, the receipt, and the
/// dry row all judge the RESOLVED landing. The capability grain does NOT:
/// [`admit`] judges the DECLARED path and runs BEFORE this resolution in
/// both tenses ([`apply_under`] and `runner::rehearse` — dogfood r2 F2:
/// dry-green predicts live-green).
///
/// # Errors
/// [`ExecError::BirthRefused`] — the resolver's typed `bad_path` frame
/// carried whole, exactly as a door refusal rides. Nothing was applied.
pub fn resolve_birth_targets(
    root: &fs::WorkspaceRoot,
    ambient: Option<&str>,
    effects: &[Effect],
) -> Result<Option<Vec<Effect>>, ExecError> {
    if !effects.iter().any(|e| e.kind == EffectKind::Create) {
        return Ok(None);
    }
    let mut resolved = effects.to_vec();
    for (index, effect) in resolved.iter_mut().enumerate() {
        if effect.kind != EffectKind::Create {
            continue;
        }
        let path = str_arg(effect, "path").map_err(|e| e.at(index))?;
        let base = opt_str_arg(effect, "base").map_err(|e| e.at(index))?;
        let landed = wire_serve::write::resolve_birth_target(root, &path, base.as_deref(), ambient)
            .map_err(|e| {
                ExecError::BirthRefused {
                    path: path.clone(),
                    index: Some(u32::try_from(index).unwrap_or(u32::MAX)),
                    // The resolver's typed frame, carried whole (no Display).
                    detail: serde_json::to_string(e.as_ref()).unwrap_or_else(|_| format!("{e:?}")),
                }
            })?;
        effect.args.insert("path".to_owned(), ArgValue::Str(landed));
    }
    Ok(Some(resolved))
}

/// # Errors
/// [`ExecError`] — no EDIT was applied in any case (the page splice is one
/// atomic batch). Births are the exception the enum's own header now states:
/// they realize first, one door call each in emission order, so a refusal at
/// descriptor `i` leaves every birth before `i` committed.
/// [`ExecError::descriptor_index`] names `i`.
pub fn apply_under(
    _lock: &WorkspaceLock,
    root: &fs::WorkspaceRoot,
    req: &ApplyRequest<'_>,
) -> Result<Applied, ExecError> {
    // 0. THE CHOKE POINT — before any I/O, judging DECLARED coordinates (ZT
    // ruling 2026-08-19 #2): the cap glob reads the path as the block wrote
    // it; the base axis never joins in. Deliberately BEFORE resolution — a
    // denial outranks a resolution fault.
    admit(req.page, req.ambient, req.effects, req.authority)?;

    // 1. THE BIRTH-TARGET RESOLUTION — see [`resolve_birth_targets`]: the
    // declared path composes under its base (descriptor `base`, else the
    // caller's ambient), so the door and the receipt judge where bytes land.
    let resolved = resolve_birth_targets(root, req.ambient, req.effects)?;
    let effects: &[Effect] = resolved.as_deref().unwrap_or(req.effects);

    // 1b. THE BIRTH LANE — see [`realize_births`].
    let births = realize_births(root, req, effects)?;
    let page_effects: Vec<&Effect> = effects
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
        // The descriptor's index in the WHOLE batch, so a `section_not_found`
        // names the same row a caller sees.
        let index = effects
            .iter()
            .position(|e| std::ptr::eq(e, *effect))
            .unwrap_or(0);
        planned.push(plan_edit(&doc, effect).map_err(|e| e.at(index))?);
    }

    // 3a′. ONE APPLY, ONE LAW. The armed law is resolved from disk exactly
    // ONCE here and shared by BOTH armed legs below — the middleware door at
    // 3b and the check gate at 6c. They used to resolve independently, and
    // `run.lock` does not exclude wire writers (`write.lock` is taken only at
    // the delta bracket, 7b), so a concurrent splice rewriting
    // `meridian/armed-rules.md` between the two reads had the two legs of ONE
    // write evaluating DIFFERENT law. Resolved before 3b so the snapshot
    // predates every transform this apply makes.
    let law = crate::gate::resolve_at(root, req.page);

    // 3b. THE MIDDLEWARE DOOR (armed-plane Part A2, § A.2.1) — the armed
    // plane's OTHER leg, mounted for the same byte-landing-parity reason as
    // the check gate at 6c. This is where the put frame's `fields` reaches a
    // fire's splice write as `ctx.fields` (design § 6 step 6). Ordered like
    // the wire door's: after planning, BEFORE the `@fp` strip — so a
    // transform's payload is stripped like any other, and everything below
    // (the strip, the lock guard, the check gate, the receipt) reads the
    // bytes middleware left, and a middleware cannot smuggle bytes past an
    // armed check.
    mount_middleware(root, &law, &doc, req, &mut planned)?;

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
    //
    // `law` is the SAME snapshot 3b judged against (resolved at 3a′), not a
    // second read of disk.
    //
    // The actor is `req.actor()` — the §9 law, NOT `run:<task>` unconditional.
    // This leg dropped the supplied identity, so one apply presented two
    // different `change.actor` values to the same workspace's law depending on
    // which leg was evaluating. A CHECK keyed on actor — the shipped
    // `reviewer-not-owner` shape in `crate::gate` is exactly one — could then
    // never fire on a fire: it compared the caller's identity against the
    // plane's self-label and silently passed. An armed rule reading as a pass
    // for the wrong reason is the failure class the parity mounts exist to end.
    if let Some(detail) = crate::gate::refuse_reason(
        &law,
        &doc,
        after_doc.document(),
        req.page,
        &batch.edits,
        &req.actor(),
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

/// How long a run-plane acquire of `.meridian/write.lock` waits out a
/// `workspace_busy` refusal before surfacing it — the CLI lane's own waiting
/// policy, which the wire contract assigns to the caller ("refused in ≤0.1 ms,
/// no engine retry and no queue, so waiting is entirely the caller's policy").
///
/// Default 10 s, sized off measurement rather than taste: on a 37 878-file
/// corpus with a resident daemon the competing holds are the daemon's own
/// commits at 1.1–1.5 s each (`wall = hold`), recurring every few seconds, so
/// the old 2 s bound was inside the noise. `MERIDIAN_BUSY_WAIT_MS` overrides
/// it — 0 restores the pure `LOCK_NB` refusal, which is how the tests assert
/// the refusal still exists without waiting the default out.
fn busy_wait_budget() -> Duration {
    match std::env::var("MERIDIAN_BUSY_WAIT_MS") {
        Ok(raw) => Duration::from_millis(raw.trim().parse().unwrap_or(10_000)),
        Err(_) => Duration::from_secs(10),
    }
}

/// The delta-mint bracket's flock (step 7b): the workspace WRITE lock,
/// bounded-wait — a holder is one wire splice's critical section or a detect
/// cycle, so the retry converts the NB acquire into a bounded wait; past the
/// deadline the apply refuses the plane's own typed busy word. Lock order is
/// run.lock → write.lock on this path and no path takes them the other way.
fn acquire_write_flock(root: &fs::WorkspaceRoot) -> Result<fs::WriteLock, ExecError> {
    let deadline = Instant::now() + busy_wait_budget();
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
    let actor = req.actor();
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
            fields: req.fields,
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
    let (mut batch, mut sealed, mut after_doc) = seal_candidate(req.page, doc, planned)?;
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

/// Steps 4-5 alone: the sealed batch and the candidate document, WITHOUT the
/// `@fp` strip — the pending state the middleware door reads between rows
/// ([`mount_middleware`]), and the first half of [`seal_stripped_candidate`].
///
/// Split out rather than duplicated because a second spelling of "seal this
/// batch" is how the bytes middleware judges would come to differ from the
/// bytes the gate judges.
///
/// **This refusal became reachable in a NEW way when the middleware door was
/// mounted, and its text is not yet equal to that.** Measured by reviewer
/// `36637e1a` on PR 214 (finding 2): when a middleware stamps a frontmatter
/// key the caller's own effect already targets, validation refuses the whole
/// apply with the raw verdict Debug — e.g.
/// `Overlap { edits: [0, 1], spans: [23..35, 23..35] }`. Fail-closed and
/// byte-clean, but "edit 1" is the MIDDLEWARE's, which the caller never sent
/// and cannot see, and nothing in the string says "middleware" or names the
/// rule. The caller is handed an index into a batch that is not theirs.
///
/// The sibling arm — the key being NEW rather than existing — used to LAND
/// THE KEY TWICE rather than refuse, because both upserts planned the same
/// zero-width insert and the region grain read them disjoint. Closed at
/// engine grain (`model::validate_batch` rung 3a, one key one upsert), so
/// both arms now refuse the same way through every lane that calls it — this
/// one, the wire put door's re-seal, and the wire create door's own
/// `mw_upsert` self-transform. What remains open is the TEXT quoted above:
/// naming which leg is the middleware's is card
/// `actor-asymmetry-check-leg-vs-mw-leg`.
///
/// # Errors
/// [`ExecError::Refused`] from validation. Nothing has been applied.
fn seal_candidate(
    page: &str,
    doc: &Document,
    planned: &[PlannedEdit],
) -> Result<
    (
        SpliceRequest,
        model::ValidatedBatch,
        model::CandidateDocument,
    ),
    ExecError,
> {
    let batch = SpliceRequest {
        if_root: None,
        edits: planned.iter().map(|p| p.edit.clone()).collect(),
        engine: None,
    };
    let sealed = match model::validate_batch(doc, None, &batch, None) {
        SpliceVerdict::Validated(b) => b,
        refused => {
            return Err(ExecError::Refused {
                verdict: format!("{refused:?}"),
            });
        }
    };
    let after_doc = crate::fp::candidate(page, doc, &sealed);
    Ok((batch, sealed, after_doc))
}

/// Step 3b: run every armed in-scope middleware row over the pending splice,
/// `id` ascending, folding this-file transforms into `planned`.
///
/// **What this closes.** `ctx.fields` is a middleware-ctx surface
/// ([`policy::MwCtxInput`]) and exists nowhere else — not on a CHECK change,
/// not on [`CommitFacts`]. Until this mount, a fire's `set_field` /
/// `append_section` evaluated NO middleware at all: the put lane and the fire
/// lane governed the same page under different halves of the same armed law.
/// Design § 6 step 6: *armed middleware evaluates on those writes as on any
/// put, with the frame the put face would have given it.*
///
/// **Row *n* reads what row *n-1* left**, exactly as the wire door does: the
/// candidate is re-derived between rows, so a stamp by an earlier rule is
/// visible to a later one as `ctx.after`.
///
/// **What this lane admits, and what it refuses LOUD** (V1 limit, mirroring
/// the birth door's): `refuse` and a this-file `set_field` are honored; a
/// cross-file `set_field`, a `create`, and a `send` refuse the apply
/// ([`ExecError::MiddlewareEmit`]). The page splice is one atomic batch on one
/// page and compiles no sealed set, so those emissions have nowhere to land —
/// and a rule that believes it stamped a file it never touched is worse than
/// a refusal that says so.
///
/// A never-armed workspace, or one with no middleware row in scope, costs one
/// marker `stat` and returns — the mount is a bit-for-bit no-op there.
///
/// # Errors
/// [`ExecError::ArmedRefusal`] — a middleware `refuse`, or an evaluation fault
/// (fail-closed: a law that cannot complete never reads as a pass).
/// [`ExecError::MiddlewareEmit`] — an emission this lane cannot land.
/// [`ExecError::Refused`] — the transformed batch does not validate.
/// Nothing has been applied in any case.
fn mount_middleware(
    root: &fs::WorkspaceRoot,
    law: &policy::ArmedLaw,
    doc: &Document,
    req: &ApplyRequest<'_>,
    planned: &mut Vec<PlannedEdit>,
) -> Result<(), ExecError> {
    // The apply's own snapshot (resolved at 3a′), never a second disk read:
    // the check gate at 6c judges against this same law.
    let rows = crate::gate::middleware_rows(law, req.page);
    if rows.is_empty() {
        return Ok(());
    }
    // §9: the receipt's own actor law, so `ctx.actor` on this lane reads what
    // the receipt attests, what the create door was told, and — since the
    // CHECK leg was brought onto the same law — what a check rule sees.
    let actor = req.actor();
    for row in &rows {
        let (batch, _sealed, after_doc) = seal_candidate(req.page, doc, planned)?;
        let emits = crate::gate::middleware_emits(
            root,
            row,
            &crate::gate::PendingSplice {
                page: req.page,
                before: doc,
                after: after_doc.document(),
                edits: &batch.edits,
                actor: &actor,
                fields: req.fields,
            },
        )
        .map_err(|detail| ExecError::ArmedRefusal { detail })?;
        for emit in emits {
            let unsupported = |what: String| ExecError::MiddlewareEmit {
                rule: row.id().as_str().to_owned(),
                detail: format!(
                    "{what} — a fire's page splice is ONE atomic batch on `{}` and compiles no \
                     sealed set, so this lane admits refuse and this-file set_field only (V1 \
                     limit); route cross-file work through a put",
                    req.page
                ),
            };
            match emit {
                policy::MwEmit::SetField { path, key, value } if path == req.page => {
                    planned.push(plan_edit(doc, &mw_set_field(req, row, &key, value))?);
                }
                policy::MwEmit::SetField { path, .. } => {
                    return Err(unsupported(format!(
                        "middleware `{}` emits set_field to `{path}`, not the page under fire",
                        row.id()
                    )));
                }
                policy::MwEmit::Create { path, .. } => {
                    return Err(unsupported(format!(
                        "middleware `{}` emits create `{path}` from the splice door",
                        row.id()
                    )));
                }
                policy::MwEmit::Send { to, .. } => {
                    return Err(unsupported(format!(
                        "middleware `{}` emits send to [{}], and a fire row carries no intent \
                         channel to hand it to a host",
                        row.id(),
                        to.join(", ")
                    )));
                }
            }
        }
    }
    Ok(())
}

/// One middleware self-transform as the descriptor [`plan_edit`] plans: a
/// frontmatter upsert, planned against the SAME base document the caller's
/// own edits were planned against (the wire door plans its `mw_upsert` rows
/// the same way), so both carry load-time self-guards.
///
/// The provenance is the run's own — this edit happened inside this
/// invocation — and `rule_id` names the middleware **on the descriptor**.
///
/// **It does NOT reach the receipt, and this doc used to claim it did.**
/// Measured by reviewer `36637e1a` on PR 214 (finding 3): the row that lands
/// is `{"target":{"fm":"<key>"},"before":…,"after":…}` and nothing else —
/// [`render_receipt`] renders target/before/after per planned edit and never
/// looks at `rule_id`, which rides only this transient [`Effect`]. So a
/// middleware stamp is presently **indistinguishable in the receipt from a
/// caller edit**.
///
/// Left as-is rather than fixed here: threading the id would change the
/// receipt's committed line shape, which is a receipt-grain decision and one
/// the wire door's own armed-edit rows share. Recorded here so the next
/// reader inherits the fact rather than the wish.
fn mw_set_field(
    req: &ApplyRequest<'_>,
    row: &policy::ArmedRule,
    key: &str,
    value: String,
) -> Effect {
    Effect {
        kind: EffectKind::SetField,
        rule_id: row.id().as_str().to_owned(),
        seq: 0,
        depth: req.depth,
        provenance: Provenance::Run {
            invocation_id: req.invocation_id.to_owned(),
            root_at_eval: req.observed_root.0.clone(),
        },
        args: BTreeMap::from([
            ("field".to_owned(), ArgValue::Str(key.to_owned())),
            ("value".to_owned(), ArgValue::Str(value)),
        ]),
    }
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

/// A descriptor's choke-point surface: the cap VERB it authorizes under and
/// the DECLARED coordinate the cap glob judges (caps-redesign ruling,
/// 2026-08-19).
///
/// The verb map is the descriptor→cap fold: `Create` births under
/// [`caps::VERB_CREATE`] at its declared `path` argument, verbatim — the
/// resolution base is a separate axis ([`resolve_birth_targets`]) the cap
/// never reads; `SetField` and `AppendSection` both CHANGE the declaring
/// page, so they authorize under [`caps::VERB_EDIT`] at `page_coordinate`
/// (the page in the coordinates it was addressed by — [`edit_coordinate`]).
/// [`caps::VERB_DELETE`] is reserved: no descriptor maps to it until a retire
/// descriptor exists. Field/section grain left the cap grammar with the
/// ruling — a field-grain guard belongs inside the block.
fn descriptor_surface(
    page_coordinate: &str,
    effect: &Effect,
) -> Result<(&'static str, String), ExecError> {
    if effect.kind.domain() != Domain::Md {
        return Err(ExecError::NonMdEffect {
            kind: effect.kind.as_str().to_owned(),
            index: None,
        });
    }
    match effect.kind {
        EffectKind::SetField | EffectKind::AppendSection => {
            Ok((caps::VERB_EDIT, page_coordinate.to_owned()))
        }
        EffectKind::Create => Ok((caps::VERB_CREATE, str_arg(effect, "path")?)),
        _ => unreachable!("md.* kinds are SetField | AppendSection | Create"),
    }
}

/// A required scalar string argument off a descriptor.
fn str_arg(effect: &Effect, key: &str) -> Result<String, ExecError> {
    match effect.args.get(key) {
        Some(ArgValue::Str(s)) => Ok(s.clone()),
        _ => Err(ExecError::BadDescriptor {
            kind: effect.kind.as_str().to_owned(),
            reason: format!("missing scalar '{key}'"),
            index: None,
        }),
    }
}

/// The `create(props = {…})` map off a descriptor, lowered to the door's own
/// shape (D6, card 17). Absent is an empty map — the shipped birth, where
/// `body` is the whole document. Present-but-not-a-map, or a map value that is
/// neither scalar nor list, is the same loud fault as a missing required arg:
/// the kernel constructor already refused those shapes, so reaching here means
/// a descriptor was built by hand out of contract.
fn props_arg(effect: &Effect) -> Result<BTreeMap<String, wire_serve::write::PropValue>, ExecError> {
    let bad = |reason: String| ExecError::BadDescriptor {
        kind: effect.kind.as_str().to_owned(),
        reason,
        index: None,
    };
    let map = match effect.args.get("props") {
        None => return Ok(BTreeMap::new()),
        Some(ArgValue::Map(map)) => map,
        Some(_) => return Err(bad("non-map 'props'".to_owned())),
    };
    let mut props = BTreeMap::new();
    for (key, value) in map {
        let value = match value {
            ArgValue::Str(s) => wire_serve::write::PropValue::Scalar(s.clone()),
            ArgValue::List(items) => wire_serve::write::PropValue::List(items.clone()),
            ArgValue::Map(_) => {
                return Err(bad(format!(
                    "props value for '{key}' is a map — frontmatter values are scalars or \
                     one-level lists"
                )));
            }
        };
        props.insert(key.clone(), value);
    }
    Ok(props)
}

/// An optional scalar string argument off a descriptor: absent is `None`,
/// present-but-non-scalar is the same loud fault as a missing required arg.
fn opt_str_arg(effect: &Effect, key: &str) -> Result<Option<String>, ExecError> {
    match effect.args.get(key) {
        None => Ok(None),
        Some(ArgValue::Str(s)) => Ok(Some(s.clone())),
        Some(_) => Err(ExecError::BadDescriptor {
            kind: effect.kind.as_str().to_owned(),
            reason: format!("non-scalar '{key}'"),
            index: None,
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
            // ⑤-F2: PRESERVATION ONLY. A value-identical write —
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
            index: None,
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
            index: None,
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
            index: None,
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
        actor: req.actor(),
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
