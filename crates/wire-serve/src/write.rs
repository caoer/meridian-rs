//! The shared write choke-point — `splice → commit` — used by the resident
//! registry daemon (the one wire door, §3.3) and the in-process callers.
//!
//! # The single choke-point
//! [`splice`] is the one function the write path flows through: flock → load →
//! §5.1 world guard → validate → build the post-batch doc once → I4
//! def-conformance (refuse) → evaluate verdicts → armed gate (refuse) → dry
//! short-circuit → (real) render the receipt + [`commit_batch`] (validate →
//! `fs::apply_batch` → one Delta). Every rung from the flock down reads the
//! same loaded pre-image and the same post-batch doc, which is what makes a
//! verdict binding on the bytes it authorized.
//!
//! # Verdicts
//! [`evaluate_verdicts`] runs whatever admitted `policy::CompiledRuleset`s the
//! caller hands in over the post-batch doc — the resident daemon hands `&[]`.
//! Empty rulesets ⇒ `verdicts: []`.
//!
//! # The delta ring lives with the sink
//! [`commit_batch`] assembles one `DeltaFrame` at the single §7.3 constructor
//! ([`assemble_delta`]) and returns it; it does not hold or advance a ring.
//! The frame reaches the resident daemon's per-workspace ring through the
//! sink's committed hook ([`crate::seq::SeqSink::committed`]), invoked while
//! the write flock is still held — so a detect cycle can never take the flock
//! and re-tell the just-committed change as external. The returned frame is
//! data for the caller's response; a ringless in-process caller discards it.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::io::ErrorKind;
use std::path::{Path as FsPath, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

// The uniform § A.6.3a refusal sentence — one owner beside the `MultiLineValue`
// it renders, so this door, `set_property` and the preset birth door cannot
// drift into three dialects of one law.
use policy::defs::multi_line_value_refusal;
use wire::{
    Armed, ArmedEdit, Delta, DeltaFile, DeltaFrame, Edit, EditShape, ErrorBody, ErrorCode,
    HpathSeg, MwIntent, NodeRev, Path, PutAt, ReceiptAddr, ReceiptFact, Referrer, ReferrerKind,
    ResponseBody, Root, SecRef, Severity, Span, Verdict, WouldCorruptFamily,
};

use crate::read::{ambiguous, to_model_ref};
use crate::{bad_request, load_doc};

// ---------------------------------------------------------------------------
// The resident-tree wrapper seam (merkle-spec §6.1, merged plan §4.1/§6 step 3)
// ---------------------------------------------------------------------------
//
// Every root a write door serves flows through here — never through a
// flock-held full-corpus read. `root_before` is a LIVE observation through
// the workspace's resident `fs::DomainCache` (the member set and every
// member's identity are checked now; only moved members are read).
// `root_after` is the commit's own overlay: the door replaces exactly the
// leaves it wrote and refolds — NEVER a second corpus read; a foreign write
// racing the commit never silently enters the folded baseline
// (`DomainLeaves::overlay`'s doc law, carried by `DomainCache::overlay_leaf`).
//
// Interim served-token law (merged plan §6 step 3): every value minted here
// stays an OLD-law (law-1, flat-encoding) token — `DomainCache` serves
// `model::merkle_root_of_leaves` over its current leaves, recomputed only
// when the tree advances (lane C, 12.1 ms measured). No law-2 value reaches
// a `Root` before the cutover.
//
// Lock discipline: each helper takes its workspace's cache mutex for one
// short scope. The scopes compose soundly because every door touches the
// cache only INSIDE the D9 write flock — cooperating writers serialize
// before their first cache touch, so the tree cannot move between a door's
// observation and its overlay. (Cache mutex nests inside the flock; nothing
// outside this seam locks it.)

/// The `DomainCache` a write door rides. The daemon passes
/// `Registry::domain_cache` so the write plane and the feed patch one tree.
/// In-process callers omit it and fall back to [`WRITE_CACHES`].
pub type WriteCache = Arc<Mutex<fs::DomainCache>>;

/// Fallback only: in-process callers with no registry (CLI, tests, library
/// doors). Daemon writes must not land here — they pass [`WriteCache`] so
/// event loss and the dirty set reach the cache `observed_root` locks.
static WRITE_CACHES: LazyLock<Mutex<HashMap<PathBuf, WriteCache>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The daemon's write-door currency instrument (card
/// bug-trusted-overlay-unvouched; merkle-spec §6.1/§6.4): the registry memo
/// the write plane shares with the feed, PLUS the observation that makes
/// its overlay servable. The two travel together by construction — a
/// resident door cannot exist without the vouch, because a drained dirty
/// set alone never authorizes the overlay.
pub struct ResidentDoor<'a> {
    /// `Registry::domain_cache` — the same memo the feed patches; the
    /// commit's own-write overlay lands here.
    pub cache: &'a WriteCache,
    /// The door-entry observation on that SAME memo, run inside the door's
    /// flock at `observed_root` time (`Registry::door_observation`): cookie
    /// barrier, take-and-apply, overlay only on a vouched memo — otherwise
    /// the live-fold floor.
    pub observe: &'a dyn Fn() -> std::io::Result<model::MerkleRoot>,
}

/// One door call's cache: either the caller-supplied daemon memo (with its
/// vouched observation) or the process-local fallback.
struct WriteCacheHandle<'a> {
    cache: WriteCache,
    /// Present when the caller supplied the registry memo; the door-entry
    /// observation is then the registry's. The fallback has no feed, so it
    /// always live-observes.
    observe: Option<&'a dyn Fn() -> std::io::Result<model::MerkleRoot>>,
}

/// The workspace's write-plane fallback cache, created on first use.
fn write_cache(root: &fs::WorkspaceRoot) -> WriteCache {
    let key = std::fs::canonicalize(&root.0).unwrap_or_else(|_| root.0.clone());
    let mut map = WRITE_CACHES.lock().unwrap_or_else(PoisonError::into_inner);
    Arc::clone(map.entry(key).or_default())
}

fn door_cache<'a>(
    root: &fs::WorkspaceRoot,
    supplied: Option<ResidentDoor<'a>>,
) -> WriteCacheHandle<'a> {
    match supplied {
        Some(door) => WriteCacheHandle {
            cache: Arc::clone(door.cache),
            observe: Some(door.observe),
        },
        None => WriteCacheHandle {
            cache: write_cache(root),
            observe: None,
        },
    }
}

/// `root_before` through the wrapper seam.
///
/// A supplied (daemon) door observes through the registry's currency
/// instrument: the §6.4 cookie vouches the overlay, and any miss degrades
/// to the full observation that absorbs the loss — on the door's own memo
/// either way. The in-process fallback always live-observes: nothing
/// fences its gap.
///
/// # Errors
/// Wire `io_error` when the domain config or the observation fails — the
/// same envelope the retired corpus read refused with.
fn observed_root(
    root: &fs::WorkspaceRoot,
    door: &WriteCacheHandle<'_>,
) -> Result<Root, Box<ErrorBody>> {
    let result = if let Some(observe) = door.observe {
        observe()
            .map(|folded| Root(folded.0))
            .map_err(|e| door_memo_refusal(&e))
    } else {
        // The in-process fallback needs no bound: `WRITE_CACHES` is reached
        // only by callers with no registry, and they touch it only inside the
        // flock — for THIS memo the module's lock discipline holds, so it
        // cannot be contended. The resident memo is the one the read plane
        // also locks, from outside the flock, which is why that arm is bounded.
        let mut cache = door.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache
            .root(root)
            .map(|folded| Root(folded.0))
            .map_err(|e| io_refusal(e.to_string()))
    };
    #[cfg(test)]
    if result.is_ok() {
        AFTER_DOOR_OBSERVE.with(|slot| {
            if let Some(hook) = slot.borrow_mut().take() {
                hook();
            }
        });
    }
    result
}

/// The door-entry observation's typed refusal (card
/// `engine-splice-timeout-hits-rotation-seals`).
///
/// A [`ErrorKind::WouldBlock`] means the shared domain memo stayed held for
/// the door's whole budget — the SAME contention shape the workspace flock
/// already refuses with, so it gets the same code: `workspace_busy`,
/// transient, retry. Callers need no new vocabulary.
///
/// The message states what happened to the bytes, and can: `observed_root`
/// runs before any byte moves. That statement is the point of the refusal —
/// it replaces a client-side deadline firing with nothing to show for it,
/// which leaves a caller unable to tell a lost write from a landed one. Any
/// other I/O failure is the observation itself failing, and stays `io_error`.
#[must_use]
pub fn door_memo_refusal(e: &std::io::Error) -> Box<ErrorBody> {
    if e.kind() != ErrorKind::WouldBlock {
        return io_refusal(e.to_string());
    }
    let mut w = ErrorBody::new(ErrorCode::WorkspaceBusy);
    w.message = Some(
        "the workspace's shared domain memo stayed held for this door's whole budget — \
         nothing was committed; transient, retry"
            .into(),
    );
    Box::new(w)
}

/// `root_after` from the commit's own overlay — no walk, no stat, no byte
/// read; a lane-C refold only when the tree advanced.
///
/// # Errors
/// Wire `io_error` — only reachable as a caller-order defect (an overlay
/// before any observation), refused rather than guessed around.
fn overlaid_root(cache: &WriteCache) -> Result<Root, Box<ErrorBody>> {
    let mut cache = cache.lock().unwrap_or_else(PoisonError::into_inner);
    cache
        .overlay_root()
        .map(|folded| Root(folded.0))
        .map_err(|e| io_refusal(e.to_string()))
}

#[cfg(test)]
thread_local! {
    static AFTER_DOOR_OBSERVE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

/// Install a one-shot hook that runs after the next successful door-entry
/// observation on this thread — the race-fixture seam (card
/// bug-overlay-reread). Production has no hook.
#[cfg(test)]
pub fn after_door_observe(hook: impl FnOnce() + 'static) {
    AFTER_DOOR_OBSERVE.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

/// Apply one landed write to the resident tree (own-write overlay, insert
/// half): the commit knows the exact bytes it wrote. Returns whether the
/// tree advanced. A path outside the observed domain is a no-op (`false`).
///
/// # Errors
/// As [`overlaid_root`] (an overlay needs an observed baseline).
fn overlay_written(cache: &WriteCache, rel: &str, bytes: &[u8]) -> Result<bool, Box<ErrorBody>> {
    let mut cache = cache.lock().unwrap_or_else(PoisonError::into_inner);
    cache
        .overlay_leaf(FsPath::new(rel), model::leaf_digest(bytes))
        .map_err(|e| io_refusal(e.to_string()))
}

/// Does a write at `rel` move the DOMAIN itself?
fn touches_domain_config(rel: &str) -> bool {
    rel == fs::domain::DOMAIN_CONFIG_PATH || rel == fs::domain::CONFIG_FILE_NAME
}

/// Parse the Domain the commit just wrote. `None` when `rel` is not a
/// domain-config surface.
fn domain_from_own_bytes(rel: &str, bytes: &str) -> Option<fs::domain::Domain> {
    if rel == fs::domain::DOMAIN_CONFIG_PATH {
        Some(fs::domain::Domain::from_markdown(bytes))
    } else if rel == fs::domain::CONFIG_FILE_NAME {
        Some(fs::domain::Domain::from_config(bytes))
    } else {
        None
    }
}

/// Apply the commit's own domain-config bytes as a membership overlay
/// ([`fs::DomainCache::overlay_membership`]): new Domain from those bytes,
/// imposed on the overlay's current leaves. Never a second observation.
fn overlay_membership_from(
    cache: &WriteCache,
    rel: &str,
    bytes: &str,
) -> Result<bool, Box<ErrorBody>> {
    let Some(domain) = domain_from_own_bytes(rel, bytes) else {
        return Ok(false);
    };
    let mut cache = cache.lock().unwrap_or_else(PoisonError::into_inner);
    cache
        .overlay_membership(domain)
        .map_err(|e| io_refusal(e.to_string()))
}

/// Own-write overlay, removal half.
fn overlay_unlinked(cache: &WriteCache, rel: &str) -> Result<bool, Box<ErrorBody>> {
    let mut cache = cache.lock().unwrap_or_else(PoisonError::into_inner);
    cache
        .overlay_remove(FsPath::new(rel))
        .map_err(|e| io_refusal(e.to_string()))
}

/// Engine-composed receipt bytes: the pre-image plus the sealed append.
/// Never a post-apply reload.
fn compose_receipt(before: Option<&model::Document>, append: &model::ReceiptAppend) -> String {
    let old = before.map_or("", |d| d.raw.as_str());
    let start = append.span.start.min(old.len());
    let end = append.span.end.min(old.len());
    let mut out = String::with_capacity(old.len() + append.text.len());
    out.push_str(&old[..start]);
    out.push_str(&append.text);
    out.push_str(&old[end..]);
    out
}

/// One splice request's decoded fields. Both hosts build this from the decoded
/// `wire::Op::Splice`, then call [`splice`].
#[derive(Debug, Clone)]
pub struct SpliceArgs {
    /// The frame correlation token — recorded into the receipt line (§6.1); no
    /// other field reads it.
    pub id: Option<u64>,
    /// The content file the batch edits.
    pub path: Path,
    /// Which door this splice arrived through, stated by the caller and never
    /// sniffed. Every `Wire` door enforces fingerprint-or-force; `InProcess` is
    /// not a wire door, so the rule does not reach it.
    pub origin: crate::guard::Origin,
    /// The recorded actor (§9: recorded exactly as given, never invented).
    pub actor: Option<String>,
    /// The recorded timestamp (§9: recorded exactly as given, never invented).
    pub now: Option<String>,
    /// The optional receipt address — its append rides the same sealed batch.
    pub receipt: Option<ReceiptAddr>,
    /// The optional §5.1 world guard: refuse if the ambient root differs.
    pub if_root: Option<Root>,
    /// The §5.4 premise list (`guards[]` + the desugared scoped sugar):
    /// scoped and root premises beyond `if_root`, checked widest-first
    /// against the resident tree, counted by §5.5 coverage. The wire door
    /// fills this from [`crate::guard::lower_premises`]; in-process callers
    /// (the script door's touch set, tests) construct them directly.
    pub premises: Vec<crate::guard::Premise>,
    /// Dry run — everything except disk (no receipt, no root advance, no Delta).
    pub dry: bool,
    /// `--force`: escape an armed binding-break / block refusal. The skip is
    /// rendered as a `forced:`-marked verdict naming the bypassed rule. The
    /// index-integrity floor is not escaped.
    pub force: bool,
    /// The requested edits, 1:1 with the armed edits in the response.
    pub edits: Vec<Edit>,
    /// `splice.plan_edits`: the plan-level batch (mutually exclusive with
    /// `edits`, decode-enforced). Lowered to native edits at the intake below
    /// (`crate::plan::lower`); armed facts align 1:1 with the lowered edits,
    /// and a `create` row's fact names the BORN section, not the parent the
    /// lowering appends under (§ A.3 create door). Empty = the native form.
    pub plan_edits: Vec<wire::PlanEdit>,
    /// `splice.pin`: the pin riding this splice. `args.path` is the pinning
    /// page — the page whose `meridian-lock` block records the claim, so the
    /// lock write is a content edit on this splice's own file and lands in the
    /// same [`commit_batch`] rename. A pin-only splice carries no `edits`. The
    /// pin's actor is `self.actor` and nothing else.
    pub pin: Option<wire::PinSpec>,
    /// § A.2.1 opaque passthrough, delivered to middleware verbatim as
    /// `ctx.fields`. The engine interprets NO key.
    pub fields: BTreeMap<String, String>,
}

/// The outcome of the write choke-point: the wire `Splice` response body plus,
/// on a real commit, the one emitted `DeltaFrame`. `committed` is `None` on a
/// dry run.
#[derive(Debug)]
pub struct SpliceOutcome {
    /// The `wire::ResponseBody::Splice` body to return to the client.
    pub body: ResponseBody,
    /// The emitted delta, present only on a real commit (absent on dry).
    pub committed: Option<DeltaFrame>,
    /// Rehearsal only: the candidate document's whole bytes, so an in-process
    /// `--dry` preview diffs against the bytes the real commit would write.
    /// `None` on a real commit. A Rust-side field, never a wire field — no
    /// remote caller gains a way to pull a whole document out of a rehearsal.
    pub candidate: Option<String>,
}

/// The single `splice → commit` choke-point. Strict-decoded edits →
/// §5.1-ordered validation → the commit seam ([`commit_batch`]: validate →
/// `fs::apply_batch` → Delta emission) — one exchange, one reparse, one root
/// advance, one Delta. `dry: true` runs everything except disk: same response
/// shape, `root_after: null`, no receipt written, no ring frame, no mkdir.
///
/// `seq` is a [`crate::seq::SeqSink`], not a number: it is called inside this
/// function's write flock, at the instant the frame is assembled, so the
/// allocation cannot race a second producer. `None` is the in-process caller
/// (no ring, no subscribers, `seq` stays `0`). `rulesets` are the admitted
/// packs whose §11.1 findings ride the `verdicts` field; `&[]` ⇒
/// `verdicts: []`.
///
/// Receipt pairing rides `CommitRequest` (fs re-checks fail-loud), the receipt
/// line renders via `crates/receipt` and folds in pre-validation (§6.1), and
/// the receipt parent dir is created on real commits only (fs does not mkdir).
///
/// # Errors
/// A typed validation refusal (§5.2 failure split) mapped to its wire frame, an
/// ambient-root/domain failure, or an I/O error — in every error case no Delta
/// exists and nothing was committed, with one exception that is a disk fault
/// rather than a refusal: a pin's anchor promotion is a second rename, so an
/// I/O failure in the commit after that rename lands can leave the marker
/// behind. It is fingerprint-neutral and idempotently reused by the next pin.
/// Every refusal rung, including all of the pin's, runs before that rename.
#[allow(clippy::too_many_lines)]
pub fn splice(
    root: &fs::WorkspaceRoot,
    seq: Option<&dyn crate::seq::SeqSink>,
    args: &SpliceArgs,
    rulesets: &[policy::CompiledRuleset],
    supplied: Option<ResidentDoor<'_>>,
) -> Result<SpliceOutcome, Box<ErrorBody>> {
    // Workspace-root confinement: `Path::join` with an absolute path discards
    // the root, so an absolute or `..`-bearing splice path would read and write
    // outside the workspace, invisible to the ledger. Checked before the flock
    // and before `load_doc` — a refusal must not depend on having already
    // touched the path it refuses.
    path_confined(root, &args.path)?;

    // The cross-process write flock, held across the whole critical section —
    // read#1 below, validate, gate, the commit's read#2 → verify → renames —
    // so cooperating meridian writers serialize instead of interleaving
    // read→rename. Dry runs take it too: a rehearsal refuses `workspace_busy`
    // exactly where the real write would. Released on drop.
    let flock = acquire_write_lock(root)?;
    let door = door_cache(root, supplied);

    let mut doc = load_doc(root, &args.path)?;
    let root_before = observed_root(root, &door)?;

    // §5.1 order: the world guard first — so a stale plan refuses before any
    // per-target resolution can answer for it, and before this splice's own
    // promotion can advance the root it guards on.
    world_guard(args.if_root.as_ref(), &root_before)?;

    // §6.6 pre-flight: the requested receipt anchor is resolved against the
    // receipt file BEFORE any byte moves, so a collision refuses the batch
    // whole instead of being detected after the commit that minted it.
    preflight_receipt_anchor(root, args.receipt.as_ref())?;

    // The pin prologue, ordered gate → fingerprint + blob → anchor promotion.
    // It runs inside the flock this splice already holds, so the receipt's
    // rev-recheck reads the same pre-image the batch will validate against,
    // and the promotion needs no second flock.
    let mut pin = match &args.pin {
        Some(spec) => Some(mint_pin(root, spec, args.actor.as_deref(), args.force)?),
        None => None,
    };
    // The promotion's own gate ran at mint time — it must refuse before any
    // byte is written, on the dry path too. Its advisory findings and forced
    // skips ride this response, merged below with the batch's own.
    let mut pin_gate = pin
        .as_mut()
        .map(|p| std::mem::take(&mut p.gate))
        .unwrap_or_default();
    // The promotion lands far below, after the last rung that can refuse: it is
    // the one write that does not ride the batch (two inodes are two renames),
    // so ordering it last is what makes a refused pin leave every file
    // byte-unchanged.
    //
    // Nothing has moved on disk yet, so `root_before` and the pinning page's
    // pre-image both still stand — except when the promotion's target IS the
    // pinning page: those promoted bytes are the pre-image this batch must be
    // composed against, because they are what disk carries when `commit_batch`
    // reads it back.
    if let Some(p) = pin.as_ref().and_then(|p| p.promotion.as_ref())
        && same_physical_file(&p.root, &p.target, root, &args.path)
    {
        doc = build_doc(&args.path, p.candidate.raw());
    }
    // The lock block is composed against the post-promotion pinning page and
    // rides the batch as the one engine-minted span edit (`model::EngineEdit`):
    // a fenced block is unaddressable by the §2.1 ref grammar, and the engine
    // is its sole writer (#8 §3). Riding here puts content+lock in one
    // `commit_batch` — a second flocked `lock_write` call would self-refuse
    // `workspace_busy` (the flock is non-reentrant per open-file-description).
    // `pin_block` is the canonical block this call minted — the one byte form
    // the artifact guard below admits as a lock change.
    let (pin_engine, pin_block) = match &pin {
        Some(p) => {
            let (edit, block) = lock_engine_edit(&doc, &args.path, p)?;
            (Some(edit), Some(block))
        }
        None => (None, None),
    };

    // The plan-lowering intake — plan_edits become native edits here, under the
    // flock, against the just-loaded pre-batch doc; the path below runs
    // unchanged on the lowered batch. Payloads ride through verbatim: the `@fp`
    // strip runs once, at document grain, over the candidate
    // ([`strip_fp_candidate`] below). `born` rides index-aligned with the
    // lowered edits: `Some(title)` exactly on `create` rows, whose armed fact
    // names the BORN section (§ A.3 create door); the native face has no
    // birth shape, so its annotations are all `None`.
    let (mut effective_edits, mut born) = if args.plan_edits.is_empty() {
        let n = args.edits.len();
        (args.edits.clone(), vec![None; n])
    } else {
        let lowered = crate::plan::lower(&doc, &args.plan_edits)?;
        (lowered.edits, lowered.born)
    };

    // Fingerprint-or-force, mounted here and nowhere else — post-lowering is
    // the one point both write faces reach, so native `edits` cannot walk
    // around it. Per-edit, so an empty batch (`mrd pin`) has nothing to demand.
    // The §5.1 amended order, phase by phase: §5.5 coverage at admission
    // (phase 1) → every supplied premise, widest-first (§5.4; the root
    // premise sugar `if_root` was honored at door entry, byte-identical v2)
    // → the per-row validity rung (phase 2), with per-edit `if_node_rev`
    // following in the model validate below. A supplied premise is checked
    // under `force` too — force bypasses requiredness, never a claim the
    // caller made (the world guard's own precedent). See `crate::guard`.
    let demands = crate::guard::coverage_gate(
        args.origin,
        args.force,
        &doc,
        &args.path,
        &args.plan_edits,
        &effective_edits,
        &args.premises,
        args.if_root.is_some(),
    )?;
    premise_guard(&door, &args.premises, &root_before)?;
    let bypassed =
        crate::guard::validity_gate(args.force, &args.path, demands, &mut effective_edits)?;

    let (model_edits, mut before_facts) =
        model_edits_and_before_facts(&doc, &effective_edits, &args.path)?;
    let mut batch = model::SpliceRequest {
        // The client's world guard was honored above, against the root it
        // actually pinned. The batch re-guards on the CURRENT root: a pin's
        // rev-neutral promotion advances the root, and re-comparing the
        // client's pre-promotion token would self-refuse `root_mismatch` on
        // this splice's own write. Nothing else can move the root under the
        // flock, and an unguarded request stays unguarded.
        if_root: args
            .if_root
            .as_ref()
            .map(|_| model::MerkleRoot(root_before.0.clone())),
        edits: model_edits,
        engine: pin_engine,
    };

    // Validate + simulate the after state in memory: armed AFTER facts come
    // from a real parse of the simulated bytes, never arithmetic-shifted.
    let sealed = match model::validate_batch(
        &doc,
        Some(&model::MerkleRoot(root_before.0.clone())),
        &batch,
        None,
    ) {
        model::SpliceVerdict::Validated(b) => b,
        refused => {
            return Err(verdict_to_wire(
                &refused,
                &effective_edits,
                &doc,
                &args.path,
            ));
        }
    };

    // Build the post-batch document state once, shared by the armed AFTER facts
    // and the verdicts, on both the dry and real paths (§4.4 one-reparse law) —
    // the real commit writes exactly these bytes, so evaluating this simulated
    // doc is evaluating the committed doc.
    let mut after_doc = build_after_doc(&doc, &sealed, &args.path);

    // The middleware door (armed-plane Part A2, § A.2.1): one eval per armed
    // in-scope middleware page, id ascending, over the pending set — after
    // CAS/validation, before bytes land. Self transforms join THIS batch and
    // re-run everything below (strip, translation, guards, I4, the check
    // gate), so a middleware cannot smuggle bytes past an armed check;
    // cross-file transforms and births become sealed-set members; sends
    // become response intents the host realizes.
    let mut sealed = sealed;
    let mw = run_door_middleware(
        root,
        &args.path,
        args.actor.as_deref(),
        args.force,
        &args.fields,
        &doc,
        &root_before,
        &mut effective_edits,
        &mut born,
        &mut batch,
        &mut sealed,
        &mut after_doc,
        &mut before_facts,
    )?;

    // The `@fp` strip runs over the candidate. It rewrites the batch's payloads
    // (so `commit_batch`'s re-validation lands the same bytes judged here) and
    // leaves a document-grain assertion behind it: any token still standing in
    // a claim-link position refuses loud instead of landing silently.
    strip_fp_candidate(
        &doc,
        &root_before,
        &args.path,
        &before_facts,
        &mut batch,
        &mut sealed,
        &mut after_doc,
    )?;

    // U12 — the stored-form translation, at the candidate (D9). Ordered after
    // the `@fp` strip: the strip has already removed every decoration this
    // write introduces, so an address reaching the stored plane with one still
    // attached is refused there rather than carried into a URI here. Like the
    // strip it rewrites payloads, re-validates and re-builds.
    translate_stored_candidate(
        &doc,
        &root_before,
        &args.path,
        &before_facts,
        &mut batch,
        &mut sealed,
        &mut after_doc,
    )?;

    // Guard the artifact, not the verb: the `meridian-lock` bytes the read-mint
    // gate protects are ordinary page text every put shape can reach. This rung
    // refuses any lock-byte change that is not exactly the block THIS call
    // minted, so an actor with no receipt cannot write a pin through native
    // `edits`, a lowered `plan_edits` batch, or any put shape added later.
    // Ordered before the ladder, the advisory verdicts, and the dry
    // short-circuit, so a rehearsal refuses exactly where the real write does.
    lock_artifact_guard(&doc, after_doc.document(), pin_block.as_deref(), &args.path)?;

    let armed_edits = simulate_armed_edits(
        after_doc.document(),
        &effective_edits,
        &before_facts,
        &born,
        &sealed,
    )?;

    // The I4 def-conformance verdict that AUTHORIZES bytes: inside the flock,
    // over the `after_doc` this splice is about to write, against the `doc` the
    // flock loaded — so a foreign writer cannot land between a host's
    // `check_write` pre-flight and its apply. Ordered before the armed gate and
    // before the dry short-circuit. Repairs/`forced` stay the standalone op's
    // channel; this run only gates, it never mutates the sealed batch.
    if let Some(refusal) = crate::check_write::verdict(
        &doc,
        after_doc.document(),
        &conformance_target(root, &args.path),
        args.actor.as_deref().unwrap_or_default(),
        args.now.as_deref().unwrap_or_default(),
    )
    .refuse
    {
        return Err(conformance_to_wire(&refusal, &args.path));
    }

    // Advisory §11.1 verdicts from caller packs — they do not gate; only the
    // armed law below does.
    let mut verdicts = evaluate_verdicts(rulesets, after_doc.document());

    // The armed-plane gate: after CAS, before bytes land, on both writer paths.
    // Reads the workspace's own armed law (never caller packs) and refuses here
    // before the dry short-circuit; never-armed is a no-op. `args.force`
    // escapes a binding-break / block refusal (rendered as a `forced:`-marked
    // verdict); the index-integrity floor never escapes.
    let gate_pass = crate::gate::gate_write(
        root,
        &doc,
        after_doc.document(),
        &batch.edits,
        policy::ChangeOp::Splice,
        args.actor.as_deref(),
        args.force,
        after_doc.document(),
    )?;
    verdicts.extend(gate_pass.verdicts);
    verdicts.extend(std::mem::take(&mut pin_gate.verdicts));
    // A forced write names the planes it wrote past on the rendered surface.
    // Empty for every unforced write.
    verdicts.extend(crate::guard::bypass_verdicts(&bypassed, &doc, &args.path));

    // Middleware members run the SAME single-form pipeline at their own paths
    // (validate → strip → translate → guards → I4 → check gate), and births
    // gate as births — any fault refuses the whole request with nothing
    // landed (validate-all-then-apply, § A.2.1).
    let mut mw_entries: Vec<SetEntryState> = Vec::new();
    if !mw.members.is_empty() {
        let member_args = SpliceSetArgs {
            id: args.id,
            files: mw.members.clone(),
            origin: crate::guard::Origin::InProcess,
            actor: args.actor.clone(),
            now: args.now.clone(),
            receipt: None,
            if_root: None,
            premises: Vec::new(),
            dry: args.dry,
            force: false,
        };
        for file in &member_args.files {
            let entry =
                validate_set_member(root, &member_args, file, &root_before, &[], &mut verdicts)
                    .map_err(|e| mw_member_refusal(&file.path, e))?;
            mw_entries.push(entry);
        }
    }
    for (path, candidate) in &mw.births {
        verdicts.extend(
            crate::gate::gate_write(
                root,
                &crate::gate::absent_doc(path),
                candidate.document(),
                &[],
                policy::ChangeOp::Create,
                args.actor.as_deref(),
                false,
                candidate.document(),
            )
            .map_err(|e| mw_member_refusal(path, e))?
            .verdicts,
        );
    }

    // Dry short-circuit (§4.4 batch law): everything except disk — no receipt,
    // no root advance, no Delta, no mkdir.
    if args.dry {
        let candidate = after_doc.document().raw.clone();
        return Ok(SpliceOutcome {
            candidate: Some(candidate),
            body: ResponseBody::Splice {
                armed: Armed {
                    path: args.path.clone(),
                    // Dry writes nothing, so there is no post-write file rev
                    // (mirrors `root_after: None` at file grain).
                    file_rev_after: None,
                    edits: armed_edits,
                    effects: Vec::new(),
                    // § A.2.1: intents/set ride non-dry successes only.
                    intents: None,
                    set: None,
                },
                receipt: None,
                root_before,
                root_after: None,
                seq: None,
                dry: Some(true),
                verdicts,
                // A dry pin reports the plan it rehearsed: `promoted` reads as
                // what a real run would do.
                pin: pin.map(|p| Box::new(p.fact)),
            },
            committed: None,
        });
    }

    // The promotion lands here, last: everything above can still refuse, below
    // there is only the commit's own I/O. "A refused pin leaves the target
    // byte-unchanged" holds by ordering, not by cleanup. The residual (G3) is a
    // crash between this rename and the commit's: a fingerprint-neutral marker
    // the next pin reuses.
    //
    // The write is rev-neutral — norm-v2 removes the marker line whole, so the
    // target's fingerprint cannot move and no other page pinning that target
    // reddens. That is what permits promoting into a possibly-unowned target at
    // all (D14), and it is asserted in `s2fix_promotion`.
    let mut promotion_row: Option<CommitPromotion> = None;
    if let Some(minted) = pin.as_mut()
        && let Some(p) = minted.promotion.take()
    {
        fs::replace_file(&p.root, FsPath::new(&p.target.0), &p.candidate)
            .map_err(|e| io_to_wire(&e))?;
        // No receipt refresh is owed here (the old D16 debt): the promotion
        // moves the section's `sec_rev` but not its fingerprint (the marker
        // line is anchor-removed from the token), and the § A.3 proof compare
        // is on the fingerprint — so the caller's next pin passes on the same
        // token its read served, with no server-side state to keep current.
        //
        // The promotion moved the corpus root — this splice's own write. The
        // batch re-guards on the advanced value (re-comparing the client's
        // pre-promotion token would self-refuse `root_mismatch` on our own
        // write; the client's guard was already honored above). `root_before`
        // itself keeps the call's pre-state: the response, the receipt and the
        // frame name that one value (§6.4 — same facts, one set), and the
        // frame tells the promotion as its own row instead of folding it
        // silently under a moved baseline (r8 D4: five physical mints, zero
        // sub rows).
        //
        // Only when the marker landed under the PINNING workspace: a
        // cross-root promotion moves the TARGET root's cursor, and this
        // workspace's `if_root` world stays exactly what the client guarded
        // (cross-root design D-B point 6 — the re-guard dance is not needed
        // there). The row follows the same physical line: it rides only where
        // this workspace's world moved — a cross-root mint is the target
        // root's story, told on that root's own plane. The predicate is
        // physical containment, so a target root nested inside the pinning
        // workspace still re-guards correctly.
        if promotion_under(root, &p) {
            // The promotion is this splice's own write: overlay exactly the
            // leaf it replaced, and serve the advanced root from the overlay
            // (merkle-spec §6.1 — never a corpus re-read). A target with no
            // spelling under this root (degenerate canonicalization) leaves
            // the row untold; re-guard against the door-entry overlay rather
            // than a second live observe.
            let frame_path = promotion_frame_path(root, &p);
            let advanced = match frame_path.as_deref() {
                Some(rel) => {
                    overlay_written(&door.cache, rel, p.candidate.raw().as_bytes())?;
                    overlay_membership_from(&door.cache, rel, p.candidate.raw())?;
                    overlaid_root(&door.cache)?
                }
                None => overlaid_root(&door.cache)?,
            };
            if advanced != root_before
                && let Some(path) = frame_path
            {
                promotion_row = Some(CommitPromotion {
                    path,
                    after: p.candidate.into_document(),
                    before: p.before,
                    root_before: root_before.clone(),
                });
            }
            batch.if_root = args
                .if_root
                .as_ref()
                .map(|_| model::MerkleRoot(advanced.0.clone()));
        }
    }

    // Real commit: render the receipt line (§6.1), fold the append, honor the
    // parent-dir obligation, then drive the commit seam.
    let receipt_input = match &args.receipt {
        Some(addr) => Some(receipt_input(
            root,
            args,
            &effective_edits,
            &root_before,
            &armed_edits,
            &born,
            addr,
        )?),
        None => None,
    };
    // The commit: the plain single-file batch when middleware compiled no
    // members, or ONE sealed set carrying the caller's batch, every
    // middleware member, and every birth (§ A.2.1 — one root advance, one
    // Delta of every file, receipt last).
    let frame = if mw_entries.is_empty() && mw.births.is_empty() {
        commit_batch(
            seq,
            // The workspace rides the flock, so root/flock cannot disagree.
            &flock,
            &CommitRequest {
                content_path: args.path.0.clone(),
                batch,
                receipt: receipt_input,
                actor: args.actor.clone(),
                now: args.now.clone(),
                promotion: promotion_row,
            },
            &door.cache,
        )
        .map_err(|e| match e {
            CommitError::Refused(v) => verdict_to_wire(&v, &effective_edits, &doc, &args.path),
            CommitError::Env(err) => err,
            CommitError::Io(err) => commit_io_to_wire(&err, &args.path),
        })?
    } else {
        let mut entries: Vec<CommitSetEntry> = Vec::with_capacity(1 + mw_entries.len());
        entries.push(CommitSetEntry::Edit {
            content_path: args.path.0.clone(),
            batch,
        });
        entries.extend(mw_entries.iter().map(|e| CommitSetEntry::Edit {
            content_path: e.path.0.clone(),
            batch: e.batch.clone(),
        }));
        entries.extend(mw.births.iter().map(|(p, c)| CommitSetEntry::Birth {
            content_path: p.0.clone(),
            body: c.raw().to_string(),
        }));
        commit_set(
            seq,
            &flock,
            &CommitSetRequest {
                entries,
                receipt: receipt_input,
                actor: args.actor.clone(),
                now: args.now.clone(),
                promotion: promotion_row,
            },
            &door.cache,
        )
        .map_err(|e| match e {
            CommitSetError::Refused { index, verdict } => {
                // Entry 0 is the caller's own batch; later indexes are the
                // middleware members, in member order (births never validate
                // here — their occupancy check is the fs verify wall).
                if index == 0 {
                    verdict_to_wire(&verdict, &effective_edits, &doc, &args.path)
                } else if let Some(entry) = mw_entries.get(index - 1) {
                    mw_member_refusal(
                        &entry.path,
                        verdict_to_wire(&verdict, &entry.effective_edits, &entry.doc, &entry.path),
                    )
                } else {
                    bad_request(format!("set commit refused at entry {index}: {verdict:?}"))
                }
            }
            CommitSetError::Env(err) => err,
            CommitSetError::Io(err) => commit_io_to_wire(&err, &args.path),
        })?
    };

    // The receipt FACT from the true post-state (host-block-leaf grain).
    let receipt_fact = resolve_receipt_fact(root, args.receipt.as_ref())?;

    // The frame lands on the sink's ring HERE, under the flock — after the
    // last fallible step, so a post-commit failure leaves the change to the
    // detector (external, degraded but honest) instead of telling a frame the
    // caller never received. Closes the detector double-emission window.
    crate::seq::committed(seq, &flock, &frame);

    Ok(SpliceOutcome {
        body: ResponseBody::Splice {
            armed: Armed {
                path: args.path.clone(),
                // The post-write whole-file rev, read from the same simulated
                // after-doc as the armed edits: the real commit writes exactly
                // these bytes, so this equals the committed file's rev and a
                // subsequent `toc`'s `file_rev`. Latency only; correctness
                // stays `root_after`.
                file_rev_after: Some(NodeRev(after_doc.document().root.node_rev.0.clone())),
                edits: armed_edits,
                // The put-path HOOK feed is retired (§ A.2.1): no reaction
                // envelopes ride a write response. What this write armed on
                // the middleware plane rides `intents` — never who gets
                // notified, never that anything was delivered.
                effects: Vec::new(),
                set: Some(armed_set_members(&mw, &frame.delta)),
                intents: Some(mw.intents),
            },
            receipt: receipt_fact,
            root_before: frame.delta.root_before.clone(),
            root_after: Some(frame.delta.root_after.clone()),
            seq: Some(frame.delta.seq),
            dry: None,
            verdicts,
            pin: pin.map(|p| Box::new(p.fact)),
        },
        committed: Some(frame),
        candidate: None,
    })
}

// ---------------------------------------------------------------------------
// The middleware door (armed-plane Part A2, wire-contract § A.2.1)
// ---------------------------------------------------------------------------

/// What one write's middleware evaluation compiled: cross-file members,
/// births (each already stripped, translated, guarded), and `send` intents.
/// Self transforms are not here — they joined the caller's own batch.
#[derive(Default)]
struct MwEmitted {
    members: Vec<wire::SpliceFile>,
    births: Vec<(Path, model::CandidateDocument)>,
    intents: Vec<MwIntent>,
    /// § A.2.1 `armed.set` attribution: member/birth path → the middleware
    /// id(s) whose emits compiled it, first-touch order, deduped per path.
    rules_by_path: BTreeMap<String, Vec<String>>,
}

impl MwEmitted {
    /// Record that `rule_id`'s emit touched `path` (member edit or birth).
    fn attribute(&mut self, path: &str, rule_id: &str) {
        let rules = self.rules_by_path.entry(path.to_string()).or_default();
        if !rules.iter().any(|r| r == rule_id) {
            rules.push(rule_id.to_string());
        }
    }
}

/// § A.2.1 `armed.set` rows: the sealed set's OTHER files — member edits,
/// then births, in emit order — each repeated from the commit's own Delta
/// row (a committed fact, never re-derived). The per-kind fallback is
/// unreachable while `commit_set` emits one Delta carrying every member;
/// it exists so a response is still shaped if that invariant ever breaks.
fn armed_set_members(mw: &MwEmitted, delta: &Delta) -> Vec<wire::ArmedSetMember> {
    let row_for = |path: &str, fallback: wire::FileChange| {
        let row = delta.files.iter().find(|f| f.path.0 == path);
        wire::ArmedSetMember {
            path: Path(path.to_string()),
            change: row.map_or(fallback, |f| f.change),
            file_rev_after: row.and_then(|f| f.file_rev_after.clone()),
            rules: mw.rules_by_path.get(path).cloned().unwrap_or_default(),
        }
    };
    let mut rows = Vec::with_capacity(mw.members.len() + mw.births.len());
    for m in &mw.members {
        rows.push(row_for(&m.path.0, wire::FileChange::Modified));
    }
    for (p, _) in &mw.births {
        rows.push(row_for(&p.0, wire::FileChange::Created));
    }
    rows
}

/// The armed in-scope middleware rows at `path`, `id` ascending. Armed-law
/// FAULTS are not consulted here — the check gate downstream refuses them
/// (a red/unloadable/unevaluable middleware row is `block`-mode and fails
/// closed there), so a fault can never read as "no middleware".
///
/// Public because the RUN plane mounts the same door over its own pending
/// splice (`run::gate`, U4.2 byte-landing parity). WHICH rows fire at a path
/// is one law; a second spelling in `run` is how the two doors would come to
/// disagree about what is armed.
#[must_use]
pub fn middleware_rows(root: &fs::WorkspaceRoot, path: &str) -> Vec<policy::ArmedRule> {
    middleware_rows_of(&crate::armed_disk::resolve_at(root, path), path)
}

/// [`middleware_rows`] over a law the caller ALREADY resolved — the row
/// selection alone, with the disk read lifted out.
///
/// Split out for the run plane, which mounts two armed legs over ONE apply
/// (middleware at 3b, CHECK at 6c) and must judge both against ONE snapshot:
/// `run.lock` does not exclude wire writers, so two resolves from disk let a
/// concurrent splice rewriting `meridian/armed-rules.md` between them have the
/// two legs of one write evaluating DIFFERENT law. The run plane resolves once
/// and feeds that snapshot here.
///
/// The selection itself stays in this module on purpose — WHICH rows fire at a
/// path is one law, and a second spelling in `run` is how the two doors would
/// come to disagree about what is armed. Only the READ moved.
#[must_use]
pub fn middleware_rows_of(law: &policy::ArmedLaw, path: &str) -> Vec<policy::ArmedRule> {
    let mut rows: Vec<policy::ArmedRule> = law
        .rules()
        .iter()
        .filter(|armed| {
            armed.mode().fires()
                && armed.rule().middleware_source().is_some()
                && armed.rule().matches_path(path)
        })
        .cloned()
        .collect();
    rows.sort_by(|a, b| a.id().as_str().cmp(b.id().as_str()));
    rows
}

/// Stamp a middleware-member refusal with the whole-request clause.
fn mw_member_refusal(path: &Path, mut e: Box<ErrorBody>) -> Box<ErrorBody> {
    if e.path.is_none() {
        e.path = Some(path.clone());
    }
    let clause = format!(
        "The write refused whole at middleware member `{}`: a middleware-compiled set \
         validates every member before any byte moves, so nothing landed.",
        path.0
    );
    e.message = Some(match e.message.take() {
        Some(msg) => format!("{msg} {clause}"),
        None => clause,
    });
    e
}

/// A middleware program refusal → the wire refusal (the check gate's own
/// `convention_fault` shape, naming each rule and citing its passing case).
fn mw_refusal_to_wire(rule_id: &str, refusals: Vec<policy::Refusal>) -> Box<ErrorBody> {
    crate::gate::gate_refusal_to_wire(policy::GateRefusal::Blocked {
        violations: refusals
            .into_iter()
            .map(|r| policy::GateViolation {
                rule: rule_id.to_string(),
                message: r.message,
                passing_scenario: r.passing_scenario,
            })
            .collect(),
    })
}

/// A middleware evaluation fault → the fail-closed wire refusal, in the one
/// armed-fault voice (`Unevaluable` — a law that cannot complete never reads
/// as a pass).
fn mw_fault_to_wire(row: &policy::ArmedRule, detail: String) -> Box<ErrorBody> {
    crate::gate::gate_refusal_to_wire(policy::GateRefusal::ArmedLawFault {
        faults: vec![policy::ArmedFault::Unevaluable {
            row: row.row().clone(),
            detail,
        }],
    })
}

/// The ONE birth mint on the middleware plane (U12 census site): a newborn's
/// candidate from its full body bytes. Reached from `run_door_middleware`
/// (where its stored-form guard discharges) and from `commit_set`'s birth
/// arm, whose unified per-entry discharge re-checks it at the seam.
fn birth_candidate(content_path: &str, body: String) -> model::CandidateDocument {
    model::candidate_of_body(content_path, body)
}

/// The ONE overlay re-seal mint on the middleware plane (U12 census site,
/// class ReadOnly): the pending after-state of a member or a transformed
/// birth, fed to the `ctx.sql`/`ctx.read` world or back into the birth door's
/// own pipeline. On the member path the value is read and dropped — the
/// LANDING member bytes are re-minted at `commit_set` from read#2.
fn member_overlay_candidate(
    content_path: &str,
    raw: &str,
    sealed: &model::ValidatedBatch,
) -> model::CandidateDocument {
    model::candidate_of_batch(content_path, raw, sealed)
}

/// One wire frontmatter upsert — the shape every middleware `set_field`
/// compiles to, on this file and on members alike.
fn mw_upsert(key: String, value: String) -> Edit {
    Edit {
        target: SecRef::FmKey { fm_key: key },
        edit: EditShape::Put {
            at: PutAt::Upsert,
            text: value,
        },
        if_node_rev: None,
    }
}

/// Run the middleware door over one pending single-form splice (armed-plane
/// Part A2): evaluate armed in-scope middleware `id` ascending; apply self
/// transforms into the caller's batch (re-validating and rebuilding the
/// candidate so the NEXT row reads the world as this one left it); compile
/// cross-file transforms and births; collect `send` intents.
///
/// The overlay world `ctx.sql` / `ctx.read` query is maintained here: this
/// file's pending bytes, every member's pending bytes, every birth.
///
/// # Errors
/// A middleware `refuse` (`convention_fault` naming the rule), an evaluation
/// fault (fail-closed), a member/birth validation refusal — in every case
/// the write refuses whole and nothing lands.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn run_door_middleware(
    root: &fs::WorkspaceRoot,
    path: &Path,
    actor: Option<&str>,
    force: bool,
    fields: &BTreeMap<String, String>,
    doc: &model::Document,
    root_before: &Root,
    effective_edits: &mut Vec<Edit>,
    born: &mut Vec<Option<crate::plan::Born>>,
    batch: &mut model::SpliceRequest,
    sealed: &mut model::ValidatedBatch,
    after_doc: &mut model::CandidateDocument,
    before_facts: &mut Vec<model::Target>,
) -> Result<MwEmitted, Box<ErrorBody>> {
    let rows = middleware_rows(root, &path.0);
    let mut out = MwEmitted::default();
    if rows.is_empty() {
        return Ok(out);
    }

    // The overlay world: pending bytes shadowing disk.
    let mut overlay: BTreeMap<String, String> = BTreeMap::new();
    overlay.insert(path.0.clone(), after_doc.raw().to_string());
    // Member accumulation: disk base doc + accumulated edits, first-touch order.
    let mut member_order: Vec<String> = Vec::new();
    let mut member_state: BTreeMap<String, (model::Document, Vec<Edit>)> = BTreeMap::new();

    for row in &rows {
        let source = row
            .rule()
            .middleware_source()
            .expect("middleware_rows filtered on the leg");
        let change = policy::derive_change(
            doc,
            after_doc.document(),
            &batch.edits,
            policy::Invocation {
                op: policy::ChangeOp::Splice,
                actor,
                force,
            },
            &[],
            &crate::gate::no_edges,
        );
        let world = crate::middleware::DoorWorld {
            root,
            overlay: &overlay,
        };
        let outcome = policy::run_middleware(
            source,
            &policy::MwCtxInput {
                change: &change,
                fields,
            },
            &world,
            row.rule().limits(),
        )
        .map_err(|e| mw_fault_to_wire(row, e.to_string()))?;
        if !outcome.refusals.is_empty() {
            return Err(mw_refusal_to_wire(row.id().as_str(), outcome.refusals));
        }

        let mut self_added = false;
        let mut members_touched: Vec<String> = Vec::new();
        for emit in outcome.emits {
            match emit {
                policy::MwEmit::SetField {
                    path: p,
                    key,
                    value,
                } => {
                    if p == path.0 {
                        effective_edits.push(mw_upsert(key, value));
                        born.push(None);
                        self_added = true;
                    } else if out.births.iter().any(|(b, _)| b.0 == p) {
                        return Err(bad_request(format!(
                            "middleware `{}` edits `{p}`, a file birthed in this same set — \
                             fold the value into the create's body instead (V1 limit)",
                            row.id()
                        )));
                    } else {
                        if !member_state.contains_key(&p) {
                            // Member paths come from a Starlark program —
                            // confine them exactly as caller paths are.
                            path_confined(root, &Path(p.clone()))?;
                            let base = load_doc(root, &Path(p.clone()))
                                .map_err(|e| mw_member_refusal(&Path(p.clone()), e))?;
                            member_state.insert(p.clone(), (base, Vec::new()));
                            member_order.push(p.clone());
                        }
                        if let Some((_, edits)) = member_state.get_mut(&p) {
                            edits.push(mw_upsert(key, value));
                        }
                        out.attribute(&p, row.id().as_str());
                        if !members_touched.contains(&p) {
                            members_touched.push(p);
                        }
                    }
                }
                policy::MwEmit::Create { path: p, body } => {
                    let birth_path = Path(p.clone());
                    path_confined(root, &birth_path)?;
                    if p == path.0
                        || member_state.contains_key(&p)
                        || out.births.iter().any(|(b, _)| b.0 == p)
                    {
                        return Err(bad_request(format!(
                            "middleware `{}` births `{p}`, which this set already writes — \
                             one path, one member",
                            row.id()
                        )));
                    }
                    if let Some(actual) = occupant_rev(root, &birth_path)? {
                        return Err(mw_member_refusal(
                            &birth_path,
                            cas_mismatch(&absent_rev(), &actual),
                        ));
                    }
                    // The create door's own body pipeline: document-grain
                    // strip → stored-form translation → guards.
                    let body = syntax::strip_fp(&body);
                    let body = translate_stored_body(body, &birth_path)
                        .map_err(|e| mw_member_refusal(&birth_path, e))?;
                    let candidate = birth_candidate(&birth_path.0, body.into_owned());
                    stored_form_guard_lazy(None, &candidate, &birth_path)
                        .map_err(|e| mw_member_refusal(&birth_path, e))?;
                    if !syntax::fp_removals(candidate.raw()).is_empty() {
                        return Err(bad_request(format!(
                            "refused: an @fp claim token survived the document-grain strip in \
                             {p} — the middleware birth was refused rather than landing a \
                             fingerprint claim the engine never minted"
                        )));
                    }
                    lock_artifact_guard(
                        &crate::gate::absent_doc(&birth_path),
                        candidate.document(),
                        None,
                        &birth_path,
                    )
                    .map_err(|e| mw_member_refusal(&birth_path, e))?;
                    overlay.insert(p.clone(), candidate.raw().to_string());
                    out.attribute(&p, row.id().as_str());
                    out.births.push((birth_path, candidate));
                }
                policy::MwEmit::Send { to, body } => {
                    out.intents.push(MwIntent {
                        kind: "send".to_string(),
                        to,
                        body,
                        rule_id: row.id().as_str().to_string(),
                    });
                }
            }
        }

        // Re-seal the caller's batch so the NEXT row's ctx.after and the
        // overlay carry this row's self transforms.
        if self_added {
            let (model_edits, facts) = model_edits_and_before_facts(doc, effective_edits, path)?;
            batch.edits = model_edits;
            *before_facts = facts;
            *sealed = match model::validate_batch(
                doc,
                Some(&model::MerkleRoot(root_before.0.clone())),
                batch,
                None,
            ) {
                model::SpliceVerdict::Validated(b) => b,
                refused => {
                    return Err(verdict_to_wire(&refused, effective_edits, doc, path));
                }
            };
            *after_doc = build_after_doc(doc, sealed, path);
            overlay.insert(path.0.clone(), after_doc.raw().to_string());
        }

        // Re-seal each touched member's pending bytes for the overlay.
        for p in members_touched {
            let member_path = Path(p.clone());
            let Some((base, edits)) = member_state.get(&p) else {
                continue;
            };
            let (model_edits, _) = model_edits_and_before_facts(base, edits, &member_path)
                .map_err(|e| mw_member_refusal(&member_path, e))?;
            let member_batch = model::SpliceRequest {
                if_root: None,
                edits: model_edits,
                engine: None,
            };
            let member_sealed = match model::validate_batch(base, None, &member_batch, None) {
                model::SpliceVerdict::Validated(b) => b,
                refused => {
                    return Err(mw_member_refusal(
                        &member_path,
                        verdict_to_wire(&refused, edits, base, &member_path),
                    ));
                }
            };
            let candidate = member_overlay_candidate(&p, &base.raw, &member_sealed);
            overlay.insert(p.clone(), candidate.raw().to_string());
        }
    }

    out.members = member_order
        .into_iter()
        .filter_map(|p| {
            member_state.remove(&p).map(|(_, edits)| wire::SpliceFile {
                path: Path(p),
                edits,
                plan_edits: Vec::new(),
            })
        })
        .collect();
    Ok(out)
}

/// The birth door's middleware run (create): self transforms + intents only —
/// a middleware firing cross-file edits or births from a birth refuses loud
/// (armed-plane Part A2, V1 limit).
///
/// # Errors
/// As [`run_door_middleware`], plus the V1 cross-file refusal.
fn run_birth_middleware(
    root: &fs::WorkspaceRoot,
    path: &Path,
    actor: Option<&str>,
    fields: &BTreeMap<String, String>,
    mut candidate: model::CandidateDocument,
) -> Result<(model::CandidateDocument, Vec<MwIntent>), Box<ErrorBody>> {
    let rows = middleware_rows(root, &path.0);
    let mut intents = Vec::new();
    if rows.is_empty() {
        return Ok((candidate, intents));
    }
    let absent = crate::gate::absent_doc(path);
    let mut overlay: BTreeMap<String, String> = BTreeMap::new();
    overlay.insert(path.0.clone(), candidate.raw().to_string());

    for row in &rows {
        let source = row
            .rule()
            .middleware_source()
            .expect("middleware_rows filtered on the leg");
        let change = policy::derive_change(
            &absent,
            candidate.document(),
            &[],
            policy::Invocation {
                op: policy::ChangeOp::Create,
                actor,
                force: false,
            },
            &[],
            &crate::gate::no_edges,
        );
        let world = crate::middleware::DoorWorld {
            root,
            overlay: &overlay,
        };
        let outcome = policy::run_middleware(
            source,
            &policy::MwCtxInput {
                change: &change,
                fields,
            },
            &world,
            row.rule().limits(),
        )
        .map_err(|e| mw_fault_to_wire(row, e.to_string()))?;
        if !outcome.refusals.is_empty() {
            return Err(mw_refusal_to_wire(row.id().as_str(), outcome.refusals));
        }
        let mut self_edits: Vec<Edit> = Vec::new();
        for emit in outcome.emits {
            match emit {
                policy::MwEmit::SetField {
                    path: p,
                    key,
                    value,
                } if p == path.0 => self_edits.push(mw_upsert(key, value)),
                policy::MwEmit::Send { to, body } => intents.push(MwIntent {
                    kind: "send".to_string(),
                    to,
                    body,
                    rule_id: row.id().as_str().to_string(),
                }),
                policy::MwEmit::SetField { path: p, .. }
                | policy::MwEmit::Create { path: p, .. } => {
                    return Err(bad_request(format!(
                        "middleware `{}` emits to `{p}` from the create door — the birth door \
                         admits refuse, this-file set_field, and send only (V1 limit); route \
                         cross-file work through a put on an existing record",
                        row.id()
                    )));
                }
            }
        }
        if !self_edits.is_empty() {
            let base = build_doc(path, candidate.raw());
            let (model_edits, _) = model_edits_and_before_facts(&base, &self_edits, path)?;
            let mw_batch = model::SpliceRequest {
                if_root: None,
                edits: model_edits,
                engine: None,
            };
            let mw_sealed = match model::validate_batch(&base, None, &mw_batch, None) {
                model::SpliceVerdict::Validated(b) => b,
                refused => {
                    return Err(verdict_to_wire(&refused, &self_edits, &base, path));
                }
            };
            candidate = model::candidate_of_batch(&path.0, &base.raw, &mw_sealed);
            // The transformed candidate is re-guarded HERE (its own U12
            // discharge site): a middleware value could smuggle a stored-form
            // violation into the born bytes between the door's pre-middleware
            // guard and the landing.
            stored_form_guard_lazy(None, &candidate, path)?;
            overlay.insert(path.0.clone(), candidate.raw().to_string());
        }
    }
    Ok((candidate, intents))
}

// ---------------------------------------------------------------------------
// The §4.4 SET form (dotted cap `splice.set`) — N files, one sealed commit
// ---------------------------------------------------------------------------

/// One decoded §4.4 set request. The member list is [`wire::SpliceFile`]
/// verbatim; everything request-level (guard, actor, now, receipt, dry,
/// force) covers the whole set. No `pin` — the pin rides the single form.
#[derive(Debug, Clone)]
pub struct SpliceSetArgs {
    /// Frame correlation token — recorded into the receipt line (§6.1).
    pub id: Option<u64>,
    /// The set members: two or more, paths pairwise distinct.
    pub files: Vec<wire::SpliceFile>,
    /// Which door this set arrived through (fingerprint-or-force reach).
    pub origin: crate::guard::Origin,
    pub actor: Option<String>,
    pub now: Option<String>,
    /// One receipt entry rides the sealed set and names every file.
    pub receipt: Option<ReceiptAddr>,
    /// The §5.1 world guard — world-grain, so it covers every member.
    pub if_root: Option<Root>,
    /// The §5.4 premise list — §5.5's set-form natural cover is each target
    /// file's own leaf token, one premise per file. Checked once, widest
    /// first; coverage judged per member. The wire door fills this from
    /// [`crate::guard::lower_premises`].
    pub premises: Vec<crate::guard::Premise>,
    pub dry: bool,
    pub force: bool,
}

/// Everything one validated member carries between the validation loop and
/// the commit/response assembly.
struct SetEntryState {
    path: Path,
    doc: model::Document,
    batch: model::SpliceRequest,
    after_doc: model::CandidateDocument,
    armed_edits: Vec<ArmedEdit>,
    effective_edits: Vec<Edit>,
    born: Vec<Option<crate::plan::Born>>,
}

/// The §4.4 SET choke-point: the single-form pipeline per member —
/// load → lower → guard → validate → one reparse → strip/translate → lock
/// guard → I4 verdict → gate — with EVERY member validated before the first
/// byte moves (validate-all-then-apply), then one sealed commit: one
/// fingerprint advance, one receipt entry naming every file, one Delta of
/// N+1 files, one `seq`. A refusal anywhere answers for the whole request
/// with nothing landed, naming the member that measured it.
///
/// No pin machinery reaches this door, and effects mode is untouched — the
/// set transaction is defined for the sealed batch model only.
///
/// # Errors
/// Any single-form refusal, stamped with the measuring member's path and the
/// whole-set clause; in every error case nothing was committed and no Delta
/// exists.
pub fn splice_set(
    root: &fs::WorkspaceRoot,
    seq: Option<&dyn crate::seq::SeqSink>,
    args: &SpliceSetArgs,
    rulesets: &[policy::CompiledRuleset],
) -> Result<SpliceOutcome, Box<ErrorBody>> {
    splice_set_with_cache(root, seq, args, rulesets, None)
}

/// [`splice_set`] riding a caller-supplied [`ResidentDoor`] (the daemon's
/// `Registry::domain_cache` plus its vouched observation).
///
/// # Errors
/// As [`splice_set`].
#[allow(clippy::too_many_lines)]
pub fn splice_set_with_cache(
    root: &fs::WorkspaceRoot,
    seq: Option<&dyn crate::seq::SeqSink>,
    args: &SpliceSetArgs,
    rulesets: &[policy::CompiledRuleset],
    supplied: Option<ResidentDoor<'_>>,
) -> Result<SpliceOutcome, Box<ErrorBody>> {
    // The set walls, enforced here as well as at decode — in-process callers
    // reach this door without the decode pass.
    if args.files.len() < 2 {
        return Err(bad_request(
            "a splice set takes two or more members (§4.4 set form) — a one-file write is \
             the single `path` form",
        ));
    }
    for (i, file) in args.files.iter().enumerate() {
        path_confined(root, &file.path)?;
        if args.files[..i].iter().any(|f| f.path == file.path) {
            return Err(bad_request(format!(
                "set member paths must be pairwise distinct: `{}` appears twice — merge its \
                 edits into one member",
                file.path.0
            )));
        }
        if let Some(addr) = &args.receipt
            && addr.path == file.path
        {
            return Err(bad_request(format!(
                "the receipt file `{}` is also a set member: the receipt rename would \
                 clobber the member's own commit (§6.5) — receipt into a file outside \
                 the set",
                addr.path.0
            )));
        }
    }

    // One flock spans the whole critical section: every read#1, every
    // validation, and the commit's read#2 → verify → renames (§3 one bracket).
    let flock = acquire_write_lock(root)?;
    let door = door_cache(root, supplied);

    let root_before = observed_root(root, &door)?;
    // §5.1 order: the world guard first, once — world-grain covers every
    // member (if any domain file moved, the fingerprint moved).
    world_guard(args.if_root.as_ref(), &root_before)?;
    // §5.4 premises next, once, widest-first — wider than every member's
    // per-edit CAS, so a failing premise skips all member work. Per-member
    // §5.5 coverage rides each member's guard step below.
    premise_guard(&door, &args.premises, &root_before)?;
    // §6.6 pre-flight, once per set: the anchor is resolved before any byte.
    preflight_receipt_anchor(root, args.receipt.as_ref())?;

    let mut entries: Vec<SetEntryState> = Vec::with_capacity(args.files.len());
    let mut verdicts: Vec<Verdict> = Vec::new();
    for (i, file) in args.files.iter().enumerate() {
        let entry = validate_set_member(root, args, file, &root_before, rulesets, &mut verdicts)
            .map_err(|e| set_member_refusal(i, &file.path, e))?;
        entries.push(entry);
    }

    // Dry short-circuit (§4.4 batch law): everything except disk.
    if args.dry {
        return Ok(SpliceOutcome {
            candidate: None,
            body: ResponseBody::SpliceSet {
                armed: entries
                    .into_iter()
                    .map(|e| Armed {
                        path: e.path,
                        file_rev_after: None,
                        edits: e.armed_edits,
                        effects: Vec::new(),
                        intents: None,
                        set: None,
                    })
                    .collect(),
                receipt: None,
                root_before,
                root_after: None,
                seq: None,
                dry: Some(true),
                verdicts,
            },
            committed: None,
        });
    }

    // The one receipt entry, rendered from every member's armed facts.
    let receipt_input = match &args.receipt {
        Some(addr) => Some(receipt_set_input(root, args, &entries, &root_before, addr)?),
        None => None,
    };

    // What the reaction feeder and the response need after the batches move
    // into the commit seam.
    let commit_entries: Vec<CommitSetEntry> = entries
        .iter()
        .map(|e| CommitSetEntry::Edit {
            content_path: e.path.0.clone(),
            batch: e.batch.clone(),
        })
        .collect();

    let frame = commit_set(
        seq,
        &flock,
        &CommitSetRequest {
            entries: commit_entries,
            receipt: receipt_input,
            actor: args.actor.clone(),
            now: args.now.clone(),
            promotion: None,
        },
        &door.cache,
    )
    .map_err(|e| match e {
        CommitSetError::Refused { index, verdict } => {
            let entry = &entries[index];
            set_member_refusal(
                index,
                &entry.path,
                verdict_to_wire(&verdict, &entry.effective_edits, &entry.doc, &entry.path),
            )
        }
        CommitSetError::Env(err) => err,
        CommitSetError::Io(err) => {
            // The path in the io frame names the set's first member; the fs
            // error text itself names the file that measured the failure.
            commit_io_to_wire(&err, &entries[0].path)
        }
    })?;

    // The put-path HOOK feed is retired (§ A.2.1): no reaction envelopes on
    // write responses — send is middleware-intent vocabulary now, and the set
    // door evaluates no middleware in V1, so its groups carry none of either.
    let mut armed_groups: Vec<Armed> = Vec::with_capacity(entries.len());
    for e in entries {
        armed_groups.push(Armed {
            file_rev_after: Some(NodeRev(e.after_doc.document().root.node_rev.0.clone())),
            path: e.path,
            edits: e.armed_edits,
            effects: Vec::new(),
            intents: None,
            set: None,
        });
    }

    let receipt_fact = resolve_receipt_fact(root, args.receipt.as_ref())?;

    // Under the flock, after the last fallible step — see `splice`.
    crate::seq::committed(seq, &flock, &frame);

    Ok(SpliceOutcome {
        body: ResponseBody::SpliceSet {
            armed: armed_groups,
            receipt: receipt_fact,
            root_before: frame.delta.root_before.clone(),
            root_after: Some(frame.delta.root_after.clone()),
            seq: Some(frame.delta.seq),
            dry: None,
            verdicts,
        },
        committed: Some(frame),
        candidate: None,
    })
}

/// One member through the single-form validation pipeline (the §4.4 batch
/// laws per file): lower → fingerprint-or-force guard → resolve + validate →
/// one reparse → `@fp` strip → stored-form translation → lock artifact guard
/// → I4 def-conformance → advisory verdicts → the armed gate. Pushes this
/// member's verdicts; returns its carried state.
#[allow(clippy::too_many_lines)] // the single-form pipeline, per member — splitting adds indirection
fn validate_set_member(
    root: &fs::WorkspaceRoot,
    args: &SpliceSetArgs,
    file: &wire::SpliceFile,
    root_before: &Root,
    rulesets: &[policy::CompiledRuleset],
    verdicts: &mut Vec<Verdict>,
) -> Result<SetEntryState, Box<ErrorBody>> {
    let doc = load_doc(root, &file.path)?;
    let (mut effective_edits, born) = if file.plan_edits.is_empty() {
        let n = file.edits.len();
        (file.edits.clone(), vec![None; n])
    } else {
        let lowered = crate::plan::lower(&doc, &file.plan_edits)?;
        (lowered.edits, lowered.born)
    };
    // Set-form order note: the batch premises were checked ONCE before the
    // member loop (wider than any member's rows); within each member the
    // §5.1 phases hold — coverage, then the validity rung.
    let demands = crate::guard::coverage_gate(
        args.origin,
        args.force,
        &doc,
        &file.path,
        &file.plan_edits,
        &effective_edits,
        &args.premises,
        args.if_root.is_some(),
    )?;
    let bypassed =
        crate::guard::validity_gate(args.force, &file.path, demands, &mut effective_edits)?;
    let (model_edits, before_facts) =
        model_edits_and_before_facts(&doc, &effective_edits, &file.path)?;
    let mut batch = model::SpliceRequest {
        if_root: args
            .if_root
            .as_ref()
            .map(|_| model::MerkleRoot(root_before.0.clone())),
        edits: model_edits,
        engine: None,
    };
    let sealed = match model::validate_batch(
        &doc,
        Some(&model::MerkleRoot(root_before.0.clone())),
        &batch,
        None,
    ) {
        model::SpliceVerdict::Validated(b) => b,
        refused => {
            return Err(verdict_to_wire(
                &refused,
                &effective_edits,
                &doc,
                &file.path,
            ));
        }
    };
    let mut after_doc = build_after_doc(&doc, &sealed, &file.path);
    let mut sealed = sealed;
    strip_fp_candidate(
        &doc,
        root_before,
        &file.path,
        &before_facts,
        &mut batch,
        &mut sealed,
        &mut after_doc,
    )?;
    translate_stored_candidate(
        &doc,
        root_before,
        &file.path,
        &before_facts,
        &mut batch,
        &mut sealed,
        &mut after_doc,
    )?;
    // No pin reaches the set door, so the only legal lock-byte state is
    // "unchanged".
    lock_artifact_guard(&doc, after_doc.document(), None, &file.path)?;
    let armed_edits = simulate_armed_edits(
        after_doc.document(),
        &effective_edits,
        &before_facts,
        &born,
        &sealed,
    )?;
    if let Some(refusal) = crate::check_write::verdict(
        &doc,
        after_doc.document(),
        &conformance_target(root, &file.path),
        args.actor.as_deref().unwrap_or_default(),
        args.now.as_deref().unwrap_or_default(),
    )
    .refuse
    {
        return Err(conformance_to_wire(&refusal, &file.path));
    }
    verdicts.extend(evaluate_verdicts(rulesets, after_doc.document()));
    let gate_pass = crate::gate::gate_write(
        root,
        &doc,
        after_doc.document(),
        &batch.edits,
        policy::ChangeOp::Splice,
        args.actor.as_deref(),
        args.force,
        after_doc.document(),
    )?;
    verdicts.extend(gate_pass.verdicts);
    verdicts.extend(crate::guard::bypass_verdicts(&bypassed, &doc, &file.path));
    Ok(SetEntryState {
        path: file.path.clone(),
        doc,
        batch,
        after_doc,
        armed_edits,
        effective_edits,
        born,
    })
}

/// Stamp a member's refusal with the whole-set clause: the entry that
/// measured it, and the fact that nothing landed (Draft A: "the refusal
/// names the entry and row that measured it").
fn set_member_refusal(index: usize, path: &Path, mut e: Box<ErrorBody>) -> Box<ErrorBody> {
    if e.path.is_none() {
        e.path = Some(path.clone());
    }
    let clause = format!(
        "The set refused whole at files[{index}] (`{}`): a set commit validates every member \
         against the entry state before any byte moves, so nothing landed and no fingerprint \
         advanced.",
        path.0
    );
    e.message = Some(match e.message.take() {
        Some(msg) => format!("{msg} {clause}"),
        None => clause,
    });
    e
}

/// The set receipt entry: ONE line, op token `splice.set`, every member named
/// with its own edit rows, one anchor (§6.6 checked once for the whole set).
fn receipt_set_input(
    root: &fs::WorkspaceRoot,
    args: &SpliceSetArgs,
    entries: &[SetEntryState],
    root_before: &Root,
    addr: &ReceiptAddr,
) -> Result<(String, model::ReceiptAppend), Box<ErrorBody>> {
    let io_err = |e: std::io::Error| {
        let mut err = ErrorBody::new(ErrorCode::IoError);
        err.cause = Some(e.to_string());
        Box::new(err)
    };
    let files: Vec<receipt::FileFacts<'_>> = entries
        .iter()
        .map(|e| receipt::FileFacts {
            path: &e.path,
            edits: e
                .effective_edits
                .iter()
                .zip(&e.armed_edits)
                .enumerate()
                .map(|(i, (req, armed))| receipt::EditFact {
                    target: &armed.target,
                    op: if e.born.get(i).is_some_and(Option::is_some) {
                        receipt::OpFact::Create
                    } else {
                        receipt::OpFact::Edit(&req.edit)
                    },
                    before: &armed.node_rev_before,
                    after: &armed.node_rev_after,
                })
                .collect(),
        })
        .collect();
    let facts = receipt::SetArmedFacts {
        id: args.id,
        actor: args.actor.as_deref(),
        now: args.now.as_deref(),
        root_before,
        anchor: &addr.anchor,
        files,
    };
    let line = receipt::render_set_line(&facts);
    let receipt_abs = root.0.join(&addr.path.0);
    let receipt_len = match std::fs::read(&receipt_abs) {
        Ok(bytes) => bytes.len(),
        Err(e) if e.kind() == ErrorKind::NotFound => 0,
        Err(e) => return Err(io_err(e)),
    };
    if let Some(parent) = receipt_abs.parent() {
        std::fs::create_dir_all(parent).map_err(io_err)?;
    }
    Ok((
        addr.path.0.clone(),
        model::ReceiptAppend {
            span: receipt_len..receipt_len,
            text: format!("{line}\n"),
        },
    ))
}

/// One set member at the commit seam: an edit (path + model batch) or a
/// birth (path + full body bytes, post-strip/translate — armed-plane Part A2).
#[derive(Debug, Clone)]
pub enum CommitSetEntry {
    /// Splice this member's sealed batch over its read#2 pre-image.
    Edit {
        content_path: String,
        batch: model::SpliceRequest,
    },
    /// Birth this member — the destination must be absent at commit.
    Birth { content_path: String, body: String },
}

impl CommitSetEntry {
    fn content_path(&self) -> &str {
        match self {
            CommitSetEntry::Edit { content_path, .. }
            | CommitSetEntry::Birth { content_path, .. } => content_path,
        }
    }
}

/// One set commit's inputs — the per-member batches plus the set-level
/// receipt and the §9 envelope facts.
#[derive(Debug, Clone)]
pub struct CommitSetRequest {
    pub entries: Vec<CommitSetEntry>,
    pub receipt: Option<(String, model::ReceiptAppend)>,
    pub actor: Option<String>,
    pub now: Option<String>,
    /// A pin's anchor promotion this call already landed under the same
    /// flock — exactly [`CommitRequest::promotion`]'s law, on the set seam
    /// (a middleware-augmented single splice may carry both a pin and
    /// members).
    pub promotion: Option<CommitPromotion>,
}

/// A set commit that did not emit — [`CommitError`]'s shape with the
/// measuring member's index on the refusal arm.
#[derive(Debug)]
pub enum CommitSetError {
    /// Member `index`'s re-validation refused — the set never reached `fs`.
    Refused {
        index: usize,
        verdict: model::SpliceVerdict,
    },
    /// Ambient-root/domain failure, already in the wire envelope shape.
    Env(Box<ErrorBody>),
    /// The atomic set write failed or the seam contract refused. `fs` names
    /// the file and the rollback outcome in the error text.
    Io(std::io::Error),
}

/// Read#2 + re-validate every set entry before any byte moves (the commit
/// seam's own pass). A birth's read#2 is the occupancy check (verified again
/// at the fs verify wall); its candidate is built from the body bytes the
/// driver already gated. ONE stored-form guard discharge per entry, edit and
/// birth alike (U12: this is the seam's single mint + discharge site).
#[allow(clippy::type_complexity)]
fn validate_set_entries(
    root: &fs::WorkspaceRoot,
    entries: &[CommitSetEntry],
    root_before: &Root,
) -> Result<
    (
        Vec<Option<model::Document>>,
        Vec<(Option<model::ValidatedBatch>, model::CandidateDocument)>,
    ),
    CommitSetError,
> {
    let mut befores: Vec<Option<model::Document>> = Vec::with_capacity(entries.len());
    let mut owned: Vec<(Option<model::ValidatedBatch>, model::CandidateDocument)> =
        Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let (before, sealed, candidate) = match entry {
            CommitSetEntry::Edit {
                content_path,
                batch,
            } => {
                let before =
                    fs::load(root, FsPath::new(content_path)).map_err(CommitSetError::Io)?;
                let sealed = match model::validate_batch(
                    &before,
                    Some(&model::MerkleRoot(root_before.0.clone())),
                    batch,
                    None,
                ) {
                    model::SpliceVerdict::Validated(batch) => batch,
                    refused => {
                        return Err(CommitSetError::Refused {
                            index,
                            verdict: refused,
                        });
                    }
                };
                let candidate = model::candidate_of_batch(content_path, &before.raw, &sealed);
                (Some(before), Some(sealed), candidate)
            }
            CommitSetEntry::Birth { content_path, body } => {
                (None, None, birth_candidate(content_path, body.clone()))
            }
        };
        // ONE discharge per entry, edit and birth alike (U12: one guard call
        // site in this seam) — a birth's pre-image is absence, so its arm is
        // the birth-door shape `stored_form_guard_lazy(None, …)`.
        stored_form_guard_lazy(
            before.as_ref(),
            &candidate,
            &Path(entry.content_path().to_string()),
        )
        .map_err(CommitSetError::Env)?;
        befores.push(before);
        owned.push((sealed, candidate));
    }
    Ok((befores, owned))
}

/// Commit one SET and return its one Delta (§7.1 generalized: one Delta =
/// one sealed set = one root advance, `files[]` carrying every member plus
/// the receipt — cardinality is data). The `commit_batch` discipline per
/// member: read#2 from disk, re-validate, seal, candidate-tie; then ONE
/// `fs::apply_set` (verify-all → rename member order, receipt last,
/// in-memory rollback on failure), one `seq` allocation under the caller's
/// flock.
///
/// # Errors
/// [`CommitSetError`] — in every error case nothing was emitted and, short
/// of the stated crash window, nothing stays landed (fs rolls a partial
/// rename sequence back from held pre-images).
pub fn commit_set(
    seq: Option<&dyn crate::seq::SeqSink>,
    flock: &fs::WriteLock,
    req: &CommitSetRequest,
    cache: &WriteCache,
) -> Result<DeltaFrame, CommitSetError> {
    let root = flock.root();
    // Door-entry baseline still sitting in the cache — never a second live
    // observe (merkle-spec §6.1).
    let root_before = overlaid_root(cache).map_err(CommitSetError::Env)?;

    // Read#2 + re-validate every member before any byte moves.
    let (befores, owned) = validate_set_entries(root, &req.entries, &root_before)?;
    let before_receipt = match &req.receipt {
        Some((rp, _)) => load_optional_set(root, rp)?,
        None => None,
    };

    // The one sealed apply: verify-all-then-rename, receipt last, in-memory
    // rollback on a mid-sequence failure (no journal — ruling 2026-08-14).
    let members: Vec<fs::SetMember<'_>> = req
        .entries
        .iter()
        .zip(&befores)
        .zip(&owned)
        .map(|((entry, before), (sealed, candidate))| fs::SetMember {
            content_path: FsPath::new(entry.content_path()),
            payload: match (sealed, before) {
                (Some(batch), Some(before)) => fs::SetPayload::Edit {
                    batch,
                    expected_content: before.raw.as_bytes(),
                    candidate,
                },
                _ => fs::SetPayload::Birth { candidate },
            },
        })
        .collect();
    fs::apply_set(
        root,
        &members,
        req.receipt
            .as_ref()
            .map(|(rp, append)| (FsPath::new(rp.as_str()), append)),
    )
    .map_err(CommitSetError::Io)?;

    // The set's own overlay (§6.1): every member's landed leaf is its
    // candidate's own bytes; the receipt leaf is the engine-composed
    // append, never a reload. Membership overlay when a member is a
    // domain-config surface. One advanced root serves the whole set.
    let mut files: Vec<DeltaFile> = Vec::new();
    for (entry, (before, (_, candidate))) in req.entries.iter().zip(befores.iter().zip(&owned)) {
        overlay_written(cache, entry.content_path(), candidate.raw().as_bytes())
            .map_err(CommitSetError::Env)?;
        overlay_membership_from(cache, entry.content_path(), candidate.raw())
            .map_err(CommitSetError::Env)?;
        if let Some(fd) = model::delta::file_delta(before.as_ref(), Some(candidate.document())) {
            files.push(wire_map::project_file_delta(entry.content_path(), &fd));
        }
    }
    if let Some((rp, append)) = &req.receipt {
        let composed = compose_receipt(before_receipt.as_ref(), append);
        overlay_written(cache, rp, composed.as_bytes()).map_err(CommitSetError::Env)?;
        overlay_membership_from(cache, rp, &composed).map_err(CommitSetError::Env)?;
        let after_receipt = build_doc(&Path(rp.clone()), &composed);
        if let Some(fd) = model::delta::file_delta(before_receipt.as_ref(), Some(&after_receipt)) {
            files.push(wire_map::project_file_delta(rp, &fd));
        }
    }
    // A promotion into a file not already told above rides as its own row,
    // and the frame spans from before the FIRST of this call's writes —
    // [`commit_batch`]'s exact law, on the set seam.
    if let Some(p) = &req.promotion
        && !req.entries.iter().any(|e| e.content_path() == p.path)
        && req.receipt.as_ref().is_none_or(|(rp, _)| *rp != p.path)
        && let Some(fd) = model::delta::file_delta(Some(&p.before), Some(&p.after))
    {
        files.push(wire_map::project_file_delta(&p.path, &fd));
    }
    let root_after = overlaid_root(cache).map_err(CommitSetError::Env)?;
    let root_before = req
        .promotion
        .as_ref()
        .map_or(root_before, |p| p.root_before.clone());

    let seq = crate::seq::allocate(seq, flock, &root_before, &root_after, &files);
    Ok(assemble_delta(
        seq,
        root_before,
        root_after,
        req.actor.clone(),
        req.now.clone(),
        files,
    ))
}

/// [`load_optional`]'s twin on the set seam's error type.
fn load_optional_set(
    root: &fs::WorkspaceRoot,
    rel: &str,
) -> Result<Option<model::Document>, CommitSetError> {
    match fs::load(root, FsPath::new(rel)) {
        Ok(doc) => Ok(Some(doc)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(CommitSetError::Io(e)),
    }
}

// ---------------------------------------------------------------------------
// Guarded create / remove — file birth and death (d2 §2.5 C3, U2.6)
// ---------------------------------------------------------------------------
//
// `create` runs under CAS `if_absent` + workspace-root; `remove` under CAS on
// the file's read rev (remove-what-you-read) + workspace-root. Both expose the
// change surface a gate evaluates at birth/death — `before = absent` (create) /
// `after = absent` (remove). The verdict seam runs whatever rulesets the caller
// hands in (`&[]` ⇒ the bare commit), exactly like `splice`.

/// One `create` request's fields — one `new` spec (a single path + body, never
/// a batch). `actor`/`now` are recorded exactly as given (§9), `if_root` is the
/// optional §5.1 world guard, `dry` runs everything but disk.
#[derive(Debug, Clone)]
pub struct CreateArgs {
    /// Frame correlation token — recorded only, no field reads it.
    pub id: Option<u64>,
    /// The path the new file is born at (workspace-confined).
    pub path: Path,
    /// The new file's full bytes.
    pub body: String,
    pub actor: Option<String>,
    pub now: Option<String>,
    /// The optional §5.1 world guard: refuse if the ambient root differs.
    pub if_root: Option<Root>,
    /// Dry run — everything except disk (no file, no root advance).
    pub dry: bool,
    /// § A.2.1 opaque passthrough, delivered to middleware verbatim as
    /// `ctx.fields`. The engine interprets NO key.
    pub fields: BTreeMap<String, String>,
    /// D6 — the newborn's frontmatter AS DATA: keys to scalars or lists, which
    /// THIS DOOR serializes ([`compose_props`]). Empty is the shipped
    /// behaviour: `body` is the whole document, frontmatter included.
    ///
    /// The point is the quoting: a caller that composes its own frontmatter
    /// string must escape every value or forge keys with one `:` — an
    /// injection class that reappeared in every record-birthing block. Handing
    /// the door the map closes it at one place for all of them.
    pub props: BTreeMap<String, PropValue>,
}

/// A `props` value: the two shapes a v1 frontmatter value can take. Both are
/// single-line at the door — a list spells as one-line flow (`[a, b]`), the
/// spelling the corpus already carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropValue {
    /// A scalar value — encoded by `policy::defs::yaml_safe_value`.
    Scalar(String),
    /// A list value — encoded by `policy::defs::yaml_safe_flow`.
    List(Vec<String>),
}

/// **The create door's frontmatter serializer** (D6, card 17): `props` as data
/// → the newborn's frontmatter block, prepended to `body`.
///
/// Keys go through `yaml_safe_key` and values through the value plane's own
/// encoders (`yaml_safe_value` / `yaml_safe_flow`), so this door quotes
/// exactly what every other write door quotes — one law, one owner. A hostile
/// value cannot end its own line, open a second key, or nest a collection: it
/// quotes, or (the D11 newline case) the birth REFUSES. Keys land in sorted
/// order — the map carries no order, and a birth must be replayable byte for
/// byte.
///
/// Ordered AFTER the `@fp` strip and the stored-form translation deliberately:
/// those two rewrite caller bytes, and a props value must land byte-identical
/// or refuse. A value carrying an `@fp` token or a standing agent-plane
/// address therefore meets the guards below as a loud refusal instead of a
/// silent rewrite.
///
/// # Errors
/// `bad_request` — a key outside the property-key grammar, a value carrying a
/// newline (D11), or a `body` that already opens its own frontmatter fence
/// while `props` is inhabited (two spellings of one block: pass one).
fn compose_props<'a>(
    body: std::borrow::Cow<'a, str>,
    path: &Path,
    props: &BTreeMap<String, PropValue>,
) -> Result<std::borrow::Cow<'a, str>, Box<ErrorBody>> {
    if props.is_empty() {
        return Ok(body);
    }
    // The frontmatter predicate is `syntax`'s own (`parse`: a metadata block
    // counts only at offset 0 of a body opening `---\n`), cited rather than
    // re-derived.
    if body.starts_with("---\n") {
        return Err(bad_request(format!(
            "refused: {} was born with props= AND a body that already opens its own frontmatter \
             fence — two spellings of one block. Pass the keys as props= (the door quotes them) \
             or write the whole document as body=, never both",
            path.0
        )));
    }
    let mut block = String::from("---\n");
    for (key, value) in props {
        let key = policy::defs::yaml_safe_key(key)
            .map_err(|_| bad_request(policy::defs::invalid_property_key_refusal(key)))?;
        let encoded = match value {
            PropValue::Scalar(v) => policy::defs::yaml_safe_scalar(v),
            PropValue::List(items) => policy::defs::yaml_safe_flow(items),
        }
        .map_err(|_| bad_request(multi_line_value_refusal(key.as_str())))?;
        block.push_str(key.as_str());
        block.push_str(": ");
        block.push_str(&encoded);
        block.push('\n');
    }
    block.push_str("---\n");
    block.push_str(&body);
    Ok(std::borrow::Cow::Owned(block))
}

/// The outcome of a guarded `create` (birth). `committed`/`root_after` are absent
/// on a dry run — nothing landed. `verdicts` is the
/// gate seam's output over the birth's after-state: empty for the BARE commit,
/// inhabited once U4.2 mounts `gate()` at this site.
#[derive(Debug)]
pub struct CreateOutcome {
    pub root_before: Root,
    /// `None` on a dry run (nothing written, so no advanced root).
    pub root_after: Option<Root>,
    /// The born file's whole-file rev — computed from the body, so present even
    /// on a dry run (a fact about the spec, not the disk).
    pub file_rev_after: NodeRev,
    /// The birth Delta (`created`, `file_rev_before` absent); `None` on dry.
    pub committed: Option<DeltaFrame>,
    pub verdicts: Vec<Verdict>,
    pub dry: bool,
    /// § A.2.1 middleware intents the birth armed — `Some` (possibly empty)
    /// on every non-dry landed birth, `None` on dry.
    pub intents: Option<Vec<MwIntent>>,
}

/// Project a landed birth into its wire response body — one implementation both
/// hosts render through, so the `create` frame cannot drift between them.
///
/// `seq` rides from the emitted Delta, so it is absent on a dry run for the same
/// reason `root_after` is. `dry` is `Some(true)` only on a rehearsal — an
/// ordinary birth serializes no `dry` key, exactly like `splice`.
#[must_use]
pub fn create_response(path: Path, out: &CreateOutcome) -> ResponseBody {
    ResponseBody::Create {
        path,
        file_rev_after: out.file_rev_after.clone(),
        root_before: out.root_before.clone(),
        root_after: out.root_after.clone(),
        seq: out.committed.as_ref().map(|frame| frame.delta.seq),
        dry: out.dry.then_some(true),
        verdicts: out.verdicts.clone(),
        intents: out.intents.clone(),
    }
}

/// The death reply body (§ A.3 remove door): what died — its confirmed rev —
/// and the root transition. The birth reply's mirror.
#[must_use]
pub fn remove_response(path: Path, out: &RemoveOutcome) -> ResponseBody {
    ResponseBody::Remove {
        path,
        file_rev_before: out.file_rev_before.clone(),
        root_before: out.root_before.clone(),
        root_after: out.root_after.clone(),
        seq: out.committed.as_ref().map(|frame| frame.delta.seq),
        dry: out.dry.then_some(true),
        verdicts: out.verdicts.clone(),
    }
}

/// One `remove` request's fields. `if_file_rev` is the rev the caller read —
/// remove-what-you-read: the live file must still carry it, or the death
/// refuses citing the drift. Schema-optional (§ A.1: a rev-less frame still
/// decodes) and semantically mandatory from EVERY origin — deletion has no
/// recovery, so absence refuses `guard_required` after decode, in-process
/// callers included (§ A.3 remove door). `if_root`/`dry` mirror `create`.
#[derive(Debug, Clone)]
pub struct RemoveArgs {
    pub id: Option<u64>,
    /// The path whose file is removed (workspace-confined).
    pub path: Path,
    /// The whole-file rev the caller read — the remove-what-you-read guard.
    /// `None` decodes and refuses `guard_required` (never a frame rejection).
    pub if_file_rev: Option<NodeRev>,
    pub actor: Option<String>,
    pub now: Option<String>,
    pub if_root: Option<Root>,
    pub dry: bool,
}

/// The outcome of a guarded `remove` (death). Absences mirror [`CreateOutcome`].
#[derive(Debug)]
pub struct RemoveOutcome {
    pub root_before: Root,
    pub root_after: Option<Root>,
    /// The removed file's whole-file rev (the read rev, re-confirmed live).
    pub file_rev_before: NodeRev,
    /// The death Delta (`deleted`, `file_rev_after` absent); `None` on dry.
    pub committed: Option<DeltaFrame>,
    pub verdicts: Vec<Verdict>,
    pub dry: bool,
}

/// **Guarded `create`**: birth one file under CAS `if_absent` +
/// workspace-root, and emit the `created` change surface.
///
/// Order: path confinement → the machinery floor
/// ([`machinery_contained`]) → world guard (§5.1) → the gate seam over the
/// birth's after-state → the `if_absent` CAS at the disk edge
/// ([`fs::create_file`], the single source of the guard) → root advance → birth
/// Delta. `dry: true` runs everything except disk and still refuses a would-be
/// clobber.
///
/// # Errors
/// `bad_path` (escapes the workspace, or the landing carries an engine
/// machinery segment — [`MACHINERY_DIRS`]), `root_mismatch` (stale world guard),
/// `cas_mismatch` (the path is occupied — taxonomy row 13, recovery `refresh`),
/// or an I/O failure. In every error case nothing was created.
pub fn create(
    root: &fs::WorkspaceRoot,
    seq: Option<&dyn crate::seq::SeqSink>,
    args: &CreateArgs,
    rulesets: &[policy::CompiledRuleset],
) -> Result<CreateOutcome, Box<ErrorBody>> {
    create_with_cache(root, seq, args, rulesets, None)
}

/// [`create`] riding a caller-supplied [`ResidentDoor`] (the daemon's
/// `Registry::domain_cache` plus its vouched observation).
///
/// # Errors
/// As [`create`].
#[allow(clippy::too_many_lines)]
pub fn create_with_cache(
    root: &fs::WorkspaceRoot,
    seq: Option<&dyn crate::seq::SeqSink>,
    args: &CreateArgs,
    rulesets: &[policy::CompiledRuleset],
    supplied: Option<ResidentDoor<'_>>,
) -> Result<CreateOutcome, Box<ErrorBody>> {
    let fs_path = FsPath::new(&args.path.0);
    path_confined(root, &args.path)?;
    // The machinery floor: the landing may be confined and still be substrate
    // (`.git/`, `.meridian/`, `meridian/`, `receipts/`). Caps judge the
    // DECLARED coordinate, so this is the only place the landing is judged.
    machinery_contained(&args.path)?;

    // D9: births serialize on the same write flock as every meridian writer —
    // this also closes the `if_absent` check→rename window for cooperators.
    let flock = acquire_write_lock(root)?;
    let door = door_cache(root, supplied);

    let root_before = observed_root(root, &door)?;
    world_guard(args.if_root.as_ref(), &root_before)?;

    // The payload IS the candidate document here, so `strip_fp` over the whole
    // body runs at document grain — the same grammar `strip_fp_candidate`
    // applies to a splice. The rev the birth reports is therefore the rev of
    // the bytes that land, never of a decorated draft.
    let body = syntax::strip_fp(&args.body);

    // U12 — the stored-form translation at the BIRTH door (see
    // [`translate_stored_body`]).
    let body = translate_stored_body(body, &args.path)?;

    // D6 — the frontmatter serializer (see [`compose_props`]): the door owns
    // quoting, so no record-birthing block hand-rolls a YAML escaper. Last of
    // the three body steps: the two above rewrite caller bytes, and a props
    // value lands byte-identical or refuses at the guards below.
    let body = compose_props(body, &args.path, &args.props)?;

    // The birth's after-state, built once from the body (path-stamped so the
    // gate sees it). Its whole-file rev is the born file's rev.
    let after_doc = model::candidate_of_body(&args.path.0, body.into_owned());

    // The middleware door on the birth (armed-plane Part A2): self
    // transforms land IN the born bytes — one receipt, no unstamped birth —
    // and `send` intents ride the reply. Every guard below judges the FINAL
    // candidate, so a middleware cannot smuggle bytes past them.
    let (after_doc, mw_intents) = run_birth_middleware(
        root,
        &args.path,
        args.actor.as_deref(),
        &args.fields,
        after_doc,
    )?;

    // The artifact guard on the birth path: an agent-plane cross-root address
    // still standing refuses instead of landing bytes no reader can follow. A
    // birth has no pre-image, so `None`.
    stored_form_guard_lazy(None, &after_doc, &args.path)?;
    let file_rev_after = NodeRev(after_doc.document().root.node_rev.0.clone());

    // A token still standing in a claim-link position refuses instead of
    // landing. A birth has no pre-image, so "introduced" and "present" are the
    // same set here.
    if !syntax::fp_removals(after_doc.raw()).is_empty() {
        return Err(bad_request(format!(
            "refused: an @fp claim token survived the document-grain strip in {} — the birth was \
             refused rather than landing a fingerprint claim the engine never minted",
            args.path.0
        )));
    }

    // The lock artifact guard at the birth door: a newborn page has no
    // pre-image and this op mints no pin, so any `meridian-lock` bytes in the
    // body are a claim nobody computed.
    lock_artifact_guard(
        &crate::gate::absent_doc(&args.path),
        after_doc.document(),
        None,
        &args.path,
    )?;

    // Advisory §11.1 findings from any caller packs (never a decision).
    let mut verdicts = evaluate_verdicts(rulesets, after_doc.document());

    // The armed-plane gate over the birth's after-state — before=absent. Blocks
    // an armed refusal before the file is born; a no-op on a never-armed
    // workspace. Guarded create carries no `--force`: there is no forced-birth
    // path, and the wire `create` op declares no `force` field.
    verdicts.extend(
        crate::gate::gate_write(
            root,
            &crate::gate::absent_doc(&args.path),
            after_doc.document(),
            &[],
            policy::ChangeOp::Create,
            args.actor.as_deref(),
            false,
            after_doc.document(),
        )?
        .verdicts,
    );

    if args.dry {
        // A dry birth honors if_absent too — a rehearsal of a clobber refuses.
        if let Some(actual) = occupant_rev(root, &args.path)? {
            return Err(cas_mismatch(&absent_rev(), &actual));
        }
        return Ok(CreateOutcome {
            root_before,
            root_after: None,
            file_rev_after,
            committed: None,
            verdicts,
            dry: true,
            // § A.2.1: intents ride non-dry successes only.
            intents: None,
        });
    }

    // The if_absent CAS lives at the disk edge (`fs::create_file`): an occupied
    // path is `AlreadyExists`, mapped to `cas_mismatch{expected:absent,
    // actual:occupant-rev}` (row 13, recovery refresh).
    if let Err(e) = fs::create_file(root, fs_path, &after_doc) {
        return Err(match e.kind() {
            ErrorKind::AlreadyExists => cas_mismatch(
                &absent_rev(),
                &occupant_rev(root, &args.path)?.unwrap_or_else(absent_rev),
            ),
            _ => io_to_wire(&e),
        });
    }
    // The birth is this commit's own write: overlay the born leaf, and if
    // it is a domain config, impose that membership on current leaves.
    overlay_written(&door.cache, &args.path.0, after_doc.raw().as_bytes())?;
    overlay_membership_from(&door.cache, &args.path.0, after_doc.raw())?;
    let root_after = overlaid_root(&door.cache)?;

    let committed = birth_death_delta(
        seq,
        &flock,
        &args.path,
        &root_before,
        &root_after,
        args.actor.clone(),
        args.now.clone(),
        model::delta::file_delta(None, Some(after_doc.document())).as_ref(),
    );
    // Under the flock, after the last fallible step — see `splice`.
    crate::seq::committed(seq, &flock, &committed);
    Ok(CreateOutcome {
        root_before,
        root_after: Some(root_after),
        file_rev_after,
        committed: Some(committed),
        verdicts,
        dry: false,
        intents: Some(mw_intents),
    })
}

/// **Guarded `remove`** (§ A.3 remove door): death of one file — the write
/// model's third mutation, completing birth (`create`) and edit (`splice`) —
/// under remove-what-you-read + the referential check, emitting the `deleted`
/// change surface.
///
/// Order: path confinement → the write flock (D9) → door-entry observation
/// (`root_before` / world guard) → world guard (§5.1) → load the live file
/// (absent ⇒ `file_not_found`) → the `if_file_rev` demand (absent ⇒
/// `guard_required`; deletion has no recovery, so the token is a
/// precondition from EVERY origin) → the remove-what-you-read CAS → the
/// referential check (`query::backlinks` + `query::lock_pin_referrers` over
/// hash-domain file bytes; never `domain_snapshot`) → any inbound
/// wikilink/embed/ambient-pin ⇒ `remove_refused{referrers}` → the gate seam
/// over the death's before-state → unlink → `overlay_remove` +
/// `overlay_root` → death Delta. Check and unlink share the flock: a
/// cooperating writer cannot land a link between them.
///
/// # Errors
/// `bad_path`, `root_mismatch`, `file_not_found` (nothing to remove),
/// `guard_required` (no `if_file_rev` — there is no force on this door),
/// `cas_mismatch` (the file drifted from the read rev — taxonomy row 14,
/// recovery `refresh`), `remove_refused` (inbound references exist; the
/// refusal names every referrer), or an I/O failure. In every error case
/// nothing was removed.
pub fn remove(
    root: &fs::WorkspaceRoot,
    seq: Option<&dyn crate::seq::SeqSink>,
    args: &RemoveArgs,
    rulesets: &[policy::CompiledRuleset],
) -> Result<RemoveOutcome, Box<ErrorBody>> {
    remove_with_cache(root, seq, args, rulesets, None)
}

/// [`remove`] riding a caller-supplied [`ResidentDoor`] (the daemon's
/// `Registry::domain_cache` plus its vouched observation).
///
/// # Errors
/// As [`remove`].
#[allow(clippy::too_many_lines)]
pub fn remove_with_cache(
    root: &fs::WorkspaceRoot,
    seq: Option<&dyn crate::seq::SeqSink>,
    args: &RemoveArgs,
    rulesets: &[policy::CompiledRuleset],
    supplied: Option<ResidentDoor<'_>>,
) -> Result<RemoveOutcome, Box<ErrorBody>> {
    let fs_path = FsPath::new(&args.path.0);
    path_confined(root, &args.path)?;

    // D9: deaths serialize on the same write flock (read-rev CAS →
    // referential check → unlink is ONE critical section like any other
    // write).
    let flock = acquire_write_lock(root)?;
    let door = door_cache(root, supplied);

    // Door-entry observation seeds the cache and answers root_before / the
    // world guard. The referential check is a different read (query
    // instruments over hash-domain bytes), never a second fold.
    let root_before = observed_root(root, &door)?;
    world_guard(args.if_root.as_ref(), &root_before)?;

    // Load what is there — you cannot remove nothing (`file_not_found`, env).
    let before_doc = load_doc(root, &args.path)?;
    let current = NodeRev(before_doc.root.node_rev.0.clone());

    // Remove-what-you-read is a precondition of the op (§ A.3): deletion is
    // the one unrecoverable write, so a rev-less remove refuses from every
    // origin — in-process callers included, and no force alternative exists.
    let Some(if_file_rev) = args.if_file_rev.as_ref() else {
        return Err(remove_guard_required(&args.path));
    };

    // remove-what-you-read CAS (row 14, recovery refresh): the live rev must
    // still equal the rev the caller read. Drift refuses citing rev read
    // (`expected`) vs found (`actual`).
    if *if_file_rev != current {
        return Err(cas_mismatch(if_file_rev, &current));
    }

    // The referential check, inside the same critical section as the unlink
    // (§ A.3: checking outside the lock is a TOCTOU hole — a link landed
    // between check and unlink would be stranded by a door that just
    // certified nothing pointed at the file). Existing instruments:
    // query::backlinks and query::lock_pin_referrers. Their input is
    // hash-domain bytes, not a domain_snapshot fold.
    let referrers = inbound_referrers(&args.path.0, referential_files(root)?);
    if !referrers.is_empty() {
        return Err(remove_refused(&args.path, referrers));
    }

    // Advisory §11.1 findings from any caller packs (never a decision).
    let mut verdicts = evaluate_verdicts(rulesets, &before_doc);

    // The armed-plane gate over the death — after=absent; `before_doc` carries
    // what is being removed. Blocks an armed refusal before the unlink; the
    // index-integrity floor refuses a remove of the INDEX or the once-armed
    // marker here. No-op on never-armed.
    verdicts.extend(
        crate::gate::gate_write(
            root,
            &before_doc,
            &crate::gate::absent_doc(&args.path),
            &[],
            policy::ChangeOp::Remove,
            args.actor.as_deref(),
            false,
            &before_doc,
        )?
        .verdicts,
    );

    if args.dry {
        return Ok(RemoveOutcome {
            root_before,
            root_after: None,
            file_rev_before: current,
            committed: None,
            verdicts,
            dry: true,
        });
    }

    fs::remove_file(root, fs_path).map_err(|e| io_to_wire(&e))?;
    // Death lands through overlay_remove, never a second observe. Removing
    // the domain config reverts membership to the default Domain against
    // current leaves.
    if touches_domain_config(&args.path.0) {
        let mut cache = door.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache
            .overlay_membership(fs::domain::Domain::new())
            .map_err(|e| io_refusal(e.to_string()))?;
    }
    overlay_unlinked(&door.cache, &args.path.0)?;
    let root_after = overlaid_root(&door.cache)?;

    let committed = birth_death_delta(
        seq,
        &flock,
        &args.path,
        &root_before,
        &root_after,
        args.actor.clone(),
        args.now.clone(),
        model::delta::file_delta(Some(&before_doc), None).as_ref(),
    );
    // Under the flock, after the last fallible step — see `splice`.
    crate::seq::committed(seq, &flock, &committed);
    Ok(RemoveOutcome {
        root_before,
        root_after: Some(root_after),
        file_rev_before: current,
        committed: Some(committed),
        verdicts,
        dry: false,
    })
}

/// Hash-domain file bytes for the referential check — list via
/// [`fs::hash_domain`], read each UTF-8-named member, do not digest or fold.
///
/// The inbound instruments are [`query::backlinks`] and
/// [`query::lock_pin_referrers`]; this is only their corpus input. A new
/// reverse-link index is a different card. Not counted by [`fs::fold_count`].
fn referential_files(root: &fs::WorkspaceRoot) -> Result<fs::DomainFiles, Box<ErrorBody>> {
    let domain = fs::domain::Domain::load(root).map_err(|e| io_refusal(e.to_string()))?;
    let rels = fs::hash_domain(root, &domain).map_err(|e| io_refusal(e.to_string()))?;
    let mut files = Vec::with_capacity(rels.len());
    for rel in rels {
        let Some(rel_str) = rel.to_str() else {
            continue;
        };
        let bytes = std::fs::read(root.0.join(&rel)).map_err(|e| io_refusal(e.to_string()))?;
        files.push((rel_str.to_owned(), bytes));
    }
    Ok(files)
}

/// Every inbound reference to `target` in the corpus, aggregated to the
/// refusal's `referrers` rows: (referring file, edge kind, count), path-lex
/// then kind order (§ A.3 remove door).
///
/// Both planes read through `query` — the corpus-reads owner: wikilinks and
/// embeds via [`query::backlinks`] (link-plane resolution, walk stage 1),
/// ambient `meridian-lock` pins via [`query::lock_pin_referrers`] (the walk
/// plane's Down predicate at corpus grain; cross-root inbound is that plane's
/// stated limit, § A.3). Self-edges are excluded: a record cannot hold itself
/// alive.
fn inbound_referrers(target: &str, files: fs::DomainFiles) -> Vec<Referrer> {
    let (index, docs, _unserved) = fs::build_corpus(files);
    let mut counts: BTreeMap<(String, ReferrerKind), u64> = BTreeMap::new();

    for b in query::backlinks(&index, &docs, target) {
        if b.path == target {
            continue;
        }
        let kind = match b.kind {
            query::BacklinkKind::Wikilink => ReferrerKind::Wikilink,
            query::BacklinkKind::Embed => ReferrerKind::Embed,
        };
        *counts.entry((b.path, kind)).or_insert(0) += 1;
    }

    for src in query::lock_pin_referrers(&index, &docs, target) {
        if src == target {
            continue;
        }
        *counts.entry((src, ReferrerKind::Pin)).or_insert(0) += 1;
    }

    counts
        .into_iter()
        .map(|((path, kind), count)| Referrer { path, kind, count })
        .collect()
}

/// The `remove_refused` refusal (§ A.3): the record still has inbound
/// references, named one by one — reason first, then the fitted remedy.
fn remove_refused(path: &Path, referrers: Vec<Referrer>) -> Box<ErrorBody> {
    let edges: u64 = referrers.iter().map(|r| r.count).sum();
    // Rows aggregate per (path, kind): one file referring by two kinds spans
    // two rows, so the files figure dedups paths rather than counting rows.
    let files = referrers
        .iter()
        .map(|r| r.path.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let mut e = ErrorBody::new(ErrorCode::RemoveRefused);
    e.path = Some(path.clone());
    e.message = Some(format!(
        "refused: {} still has {edges} inbound reference{} from {files} file{} — removing it \
         would strand them dangling. The referrers list names each referring file, its edge \
         kind (wikilink / embed / pin), and its edge count: unlink or retarget those edges \
         (re-read each referring file first, then edit it through the write door), then resend \
         the remove. There is no force on this door.",
        path.0,
        if edges == 1 { "" } else { "s" },
        if files == 1 { "" } else { "s" },
    ));
    e.referrers = Some(referrers);
    Box::new(e)
}

/// The remove door's `guard_required` (§ A.3): no `if_file_rev` on the one
/// unrecoverable write — demanded from every origin, no force alternative.
fn remove_guard_required(path: &Path) -> Box<ErrorBody> {
    let mut e = ErrorBody::new(ErrorCode::GuardRequired);
    e.path = Some(path.clone());
    e.message = Some(format!(
        "refused: this remove carries no `if_file_rev` — deletion is the one write with no \
         recovery, so remove-what-you-read is a precondition of the op: read {} (a toc or cat \
         serves its whole-file rev), then resend with `if_file_rev` set to the rev you read. \
         There is no force on this door.",
        path.0
    ));
    Box::new(e)
}

// ---------------------------------------------------------------------------
// Guarded `meridian-lock` write (decision #8)
// ---------------------------------------------------------------------------
//
// The lock is a machine-owned lockfile in the page: a fenced `meridian-lock`
// block (versioned root object, `objects:`/`pins:` planes). The format —
// types, strict parse, canonical render, locate — lives in `crates/lock`; this
// is the engine-sole-writer path (#8 §3): the one place lock bytes reach disk.
// Callers hand in the typed `lock::Lock`, never raw block bytes, so a
// hand-forged block cannot enter through this door. Lock-is-content (#8 §5):
// the block sits inside the page span, so the page's fingerprint covers its
// lock and the write is one atomic file replace.

/// One guarded `meridian-lock` write request: upsert the page's one lock block
/// from a typed [`lock::Lock`]. `if_file_rev` is the page's whole-file rev the
/// caller read (write-what-you-read CAS); `if_root` the §5.1 world guard; `dry`
/// runs everything except disk.
#[derive(Debug, Clone)]
pub struct LockWriteArgs {
    /// Frame correlation token — recorded only.
    pub id: Option<u64>,
    /// The pinning page the `meridian-lock` block lives in (workspace-confined).
    pub path: Path,
    /// The typed lock object — the sole input form (engine-sole-writer #8 §3:
    /// raw block bytes never cross this seam; rendering is `lock::render`'s).
    pub lock: lock::Lock,
    pub actor: Option<String>,
    pub now: Option<String>,
    /// The optional §5.1 world guard: refuse if the ambient root differs.
    pub if_root: Option<Root>,
    /// The page's whole-file rev the caller read — write-what-you-read CAS.
    pub if_file_rev: NodeRev,
    /// Dry run — everything except disk (no bytes, no advance).
    pub dry: bool,
}

/// The outcome of a guarded lock write. Absences mirror [`CreateOutcome`]:
/// `root_after`/`committed` are `None` on a dry run.
#[derive(Debug)]
pub struct LockWriteOutcome {
    pub root_before: Root,
    pub root_after: Option<Root>,
    /// The page's whole-file rev before the write (the CAS-confirmed rev).
    pub file_rev_before: NodeRev,
    /// The page's whole-file rev after the lock landed (computed on dry too —
    /// a fact about the spec, not the disk).
    pub file_rev_after: NodeRev,
    pub committed: Option<DeltaFrame>,
    /// `true` when the write birthed the block (EOF append — no lock existed);
    /// `false` when it replaced the existing block in place.
    pub created: bool,
    pub dry: bool,
}

/// **Guarded `meridian-lock` write** (decision #8): land the page's one lock
/// block — replace it in place when present, birth it at EOF when absent —
/// under CAS write-what-you-read + workspace-root + the D9 write flock, and
/// emit the `modified` change surface.
///
/// Order: path confinement → the write flock (D9) → load the page → world guard
/// (§5.1) → the write-what-you-read CAS → locate the block (`lock::find` —
/// multiple blocks refuse loud: sole-writer mints exactly one) → render via
/// `lock::render` (canonical bytes; terminators are this path's) → in-memory
/// splice → [`fs::replace_file`] (atomic) → root advance → Delta. `dry: true`
/// runs everything except disk.
///
/// # Placement law (fresh lock)
/// A birthed block lands as file preamble — after the frontmatter, before the
/// first heading, where no section claims its bytes — separated from the body
/// by exactly one blank line. A replaced block keeps its exact span
/// (fence-to-fence), wherever it sits ([`lock_block_splice`]).
///
/// # Errors
/// `bad_path`, `bad_request` (a malformed/duplicated existing lock block —
/// surfaced, never silently adopted), `workspace_busy` (D9), `file_not_found`
/// (the page must exist — a lock pins content), `root_mismatch`,
/// `cas_mismatch`, or an I/O failure. In every error case nothing was written.
pub fn lock_write(
    root: &fs::WorkspaceRoot,
    seq: Option<&dyn crate::seq::SeqSink>,
    args: &LockWriteArgs,
) -> Result<LockWriteOutcome, Box<ErrorBody>> {
    lock_write_with_cache(root, seq, args, None)
}

/// [`lock_write`] riding a caller-supplied [`ResidentDoor`] (the daemon's
/// `Registry::domain_cache` plus its vouched observation).
///
/// # Errors
/// As [`lock_write`].
#[allow(clippy::too_many_lines)]
pub fn lock_write_with_cache(
    root: &fs::WorkspaceRoot,
    seq: Option<&dyn crate::seq::SeqSink>,
    args: &LockWriteArgs,
    supplied: Option<ResidentDoor<'_>>,
) -> Result<LockWriteOutcome, Box<ErrorBody>> {
    let fs_path = FsPath::new(&args.path.0);
    path_confined(root, &args.path)?;

    // D9: the lock write serializes on the same write flock as every writer.
    let flock = acquire_write_lock(root)?;
    let door = door_cache(root, supplied);

    let before_doc = load_doc(root, &args.path)?;
    let file_rev_before = NodeRev(before_doc.root.node_rev.0.clone());

    let root_before = observed_root(root, &door)?;
    world_guard(args.if_root.as_ref(), &root_before)?;

    // write-what-you-read CAS: the page must still carry the rev the caller
    // read (drift means the lock's facts were computed against stale bytes).
    if args.if_file_rev != file_rev_before {
        return Err(cas_mismatch(&args.if_file_rev, &file_rev_before));
    }

    // Locate the one block (or the EOF birth point). `lock::find` fails loud on
    // duplicates and malformed YAML — adopting that state would launder
    // corruption. Bytes and placement come from `lock_block_splice`, the one
    // owner the pin path shares.
    let raw = &before_doc.raw;
    let (edit, created) = lock_block_splice(&before_doc, locate_lock(&before_doc)?, &args.lock);
    let mut new_raw = String::with_capacity(raw.len() + edit.text.len());
    new_raw.push_str(&raw[..edit.span.start]);
    new_raw.push_str(&edit.text);
    new_raw.push_str(&raw[edit.span.end..]);
    let after_doc = model::candidate_of_body(&args.path.0, new_raw);
    // The artifact guard at the lock door. The lock block's own `ref:` and
    // `objects:` keys are positions 3 and 4, where the translation is the
    // identity — they stay in the canonical `root:` form, never the URI. This
    // rung asserts the other half: engine-composed lock bytes introduce no
    // agent-plane address into positions 1 or 2.
    stored_form_guard_lazy(Some(&before_doc), &after_doc, &args.path)?;
    let file_rev_after = NodeRev(after_doc.document().root.node_rev.0.clone());

    if args.dry {
        return Ok(LockWriteOutcome {
            root_before,
            root_after: None,
            file_rev_before,
            file_rev_after,
            committed: None,
            created,
            dry: true,
        });
    }

    fs::replace_file(root, fs_path, &after_doc).map_err(|e| io_to_wire(&e))?;
    overlay_written(&door.cache, &args.path.0, after_doc.raw().as_bytes())?;
    overlay_membership_from(&door.cache, &args.path.0, after_doc.raw())?;
    let root_after = overlaid_root(&door.cache)?;

    let files = model::delta::file_delta(Some(&before_doc), Some(after_doc.document()))
        .map(|fd| vec![wire_map::project_file_delta(&args.path.0, &fd)])
        .unwrap_or_default();
    // Allocate inside the flock this fn already holds, not at the caller before
    // it. See `crate::seq`. The sink outlives the shadowing `seq` number: the
    // committed offer below still needs it.
    let sink = seq;
    let seq = crate::seq::allocate(seq, &flock, &root_before, &root_after, &files);
    let committed = assemble_delta(
        seq,
        root_before.clone(),
        root_after.clone(),
        args.actor.clone(),
        args.now.clone(),
        files,
    );
    // Under the flock, after the last fallible step — see `splice`.
    crate::seq::committed(sink, &flock, &committed);
    Ok(LockWriteOutcome {
        root_before,
        root_after: Some(root_after),
        file_rev_before,
        file_rev_after,
        committed: Some(committed),
        created,
        dry: false,
    })
}

// ---------------------------------------------------------------------------
// The pin prologue (D7/D13/D14/D15/D16)
// ---------------------------------------------------------------------------
//
// A pin is a Splice-sibling field, never its own op: the splice's `path` is the
// pinning page, so the lock write is a content edit on that page and rides the
// same `commit_batch` rename. What lives here is everything that must happen
// under the flock before the batch is sealed — the read-mint gate, the
// fingerprint + blob, and the anchor promotion — plus the lock composition.
//
// # The grain: why the lock's `ref` is the canonical selector
// A pin's fingerprint is minted over exactly the span its `ref` resolves to,
// because that is the span the verify plane recomputes
// (`model::selector::resolve_selector` → `fingerprint::verify_content`). So the
// `ref` carries the canonical selector the read receipt was keyed on — a
// `/`-joined sanitized heading path resolving to the SECTION, or `^id` for a
// block-anchor row. The promoted `^slug` is deliberately NOT the `ref`: an
// anchor node's model span is its host line (`model::build`'s
// `anchor_host_span`), so an `^id` ref over a promoted heading would silently
// narrow a section pin to its heading text and every body edit would read as
// green. The slug is the stable handle (D15) a claim link decorates and a later
// rename-heal relocates by.

/// The R4 lock row a pin will land, in the schema's own types — minted beside
/// the wire fact and never derived from it.
///
/// - `object` — the wiki link's inner text: the target's vault-relative path
///   with its `.md` suffix removed. That is the form
///   `model::CorpusIndex::resolve_ref` matches by whole subpath suffix, so it
///   cannot collide with a same-named file in another folder the way a bare
///   basename can.
/// - `hash` — the target file's git blob oid, never optional.
/// - `selector` — `path` XOR `properties`, arrays only.
#[derive(Debug)]
struct PinRow {
    object: String,
    hash: String,
    selector: lock::Selector,
}

/// What a pin minted, plus what it still owes to disk. Nothing here has been
/// written: the prologue computes, the caller lands (see [`PendingPromotion`]).
struct PinMint {
    /// The wire fact returned to the client.
    fact: wire::PinFact,
    /// The R4 lock row's structural fields, built from the target's own read
    /// facts — never re-derived by splitting a joined address spelling.
    row: PinRow,
    /// The pinned selector's span in the target — the exact bytes the
    /// fingerprint covers, in the post-promotion document (the promotion widens
    /// the selector's node by the marker line).
    span: std::ops::Range<usize>,
    /// The anchor promotion this pin decided on, or `None` when the stable
    /// handle already existed and nothing needs writing.
    promotion: Option<PendingPromotion>,
    /// The promotion's armed-gate pass — advisory verdicts and forced skips the
    /// caller merges into its own (the gate itself already ran; a refusal never
    /// reached here).
    gate: crate::gate::GatePass,
    /// Cross-root only: the TARGET root's write flock. Taken before the
    /// target's bytes were read and held until this mint's splice returns, so
    /// the read, the gate's rev-recheck, the promotion landing and the commit
    /// form one critical section against target-root writers — the same span
    /// the pinning flock covers same-root. `LOCK_NB` is the deadlock
    /// discipline: the acquire never blocks while the pinning flock is held,
    /// so no hold-and-wait cycle can form.
    #[allow(dead_code)] // held for Drop — the lock IS the use
    target_flock: Option<fs::WriteLock>,
}

/// An anchor promotion that has been decided and not written: the exact bytes,
/// the root and page they belong to, and the receipt refresh the write owes.
///
/// The promotion touches a different file from the one the request names, so a
/// rung refusing after it would leave bytes in a page the caller never asked to
/// change. Held here, it lands after the last such rung.
#[derive(Debug)]
struct PendingPromotion {
    /// The workspace the marker lands in — the TARGET root's for a cross-root
    /// pin, the pinning root otherwise.
    root: fs::WorkspaceRoot,
    /// The page the marker lands in, relative to `root` — the pin's target,
    /// which may be the pinning page itself.
    target: Path,
    /// The target as it was before the marker — the before tense of the
    /// Delta row this write owes the commit frame (r8 D4: an untold mint is
    /// a write sub's history denies).
    before: model::Document,
    /// The sealed candidate to write — its bytes are the exact bytes that land,
    /// and also the pinning page's pre-image when the target IS the pinning
    /// page.
    candidate: model::CandidateDocument,
}

/// The pin prologue: resolve the target, gate it against the request's own
/// proof (§ A.3 proof law — the caller's read served the token it carries),
/// decide the stable anchor, and mint the fingerprint + blob oid over the
/// bytes the promotion will land.
///
/// **This function writes nothing.** The promotion travels back as a
/// [`PendingPromotion`] and the caller lands it after its last refusal rung.
///
/// - The fingerprint, the ref and the blob oid are all computed over the
///   post-promotion bytes, on the dry path exactly as on the real one — a
///   rehearsal reports what a real run mints (§4.4), and the fingerprint agrees
///   either way only because the promotion is rev-neutral.
/// - The promotion's armed gate runs here: the marker is a change to a page like
///   any other, so it passes [`crate::gate::gate_write`] before it can be handed
///   back as pending.
///
/// # Errors
/// `bad_path` / `bad_request` (the target escapes the workspace, or its slug id
/// is taken), `pin_target_missing` (no such page or selector),
/// `ambiguous_ref` (the selector matches more than one node — no door may pin
/// an occurrence the caller did not name; A.3 door symmetry),
/// `pin_proof_required` (a session actor pinning with no proof, or a proof
/// that does not match the live bytes), `write_conflict` (the supplied
/// `sec_rev` is stale — the world moved since the caller's read), a
/// `convention_fault` / `armed_drift` / `index_integrity` gate refusal on the
/// promotion, `io_error`.
#[allow(clippy::too_many_lines)]
fn mint_pin(
    root: &fs::WorkspaceRoot,
    spec: &wire::PinSpec,
    actor: Option<&str>,
    force: bool,
) -> Result<PinMint, Box<ErrorBody>> {
    let target = resolve_pin_target(root, &spec.target)?;
    // The spelling every fact, lock row and refusal names: the ruled
    // `name:rel` form for a genuinely foreign target (D-A — carried verbatim
    // into the lock's `object`, minus `.md`), the bare rel otherwise. A rooted
    // spelling that names the pinning root itself has already normalized to
    // its bare form in the resolver — one name per thing.
    let spelled: Path = match &target.mount {
        Some(name) => Path(format!("{name}:{}", target.rel.0)),
        None => target.rel.clone(),
    };

    // D-B (round 2, one-call): the TARGET root's write flock, taken BEFORE its
    // bytes are read, so the read, the receipt rev-recheck and the promotion
    // landing are serialized against target-root writers by construction. The
    // acquire is LOCK_NB — it never blocks while the pinning flock is held, so
    // no hold-and-wait cycle can form; a busy target refuses `workspace_busy`
    // naming which root is busy.
    let target_flock = match &target.mount {
        Some(name) => Some(acquire_write_lock(&target.root).map_err(|mut e| {
            if e.code == ErrorCode::WorkspaceBusy {
                e.message = Some(format!(
                    "another meridian writer holds the target root {name}'s \
                     .meridian/write.lock — transient; retry"
                ));
            }
            e
        })?),
        None => None,
    };
    let mut target_doc = load_doc(&target.root, &target.rel).map_err(|e| {
        if e.code == ErrorCode::FileNotFound {
            pin_target_missing(&spelled, format!("no page at {} to pin", spelled.0))
        } else {
            e
        }
    })?;
    // The armed gate scopes its rules by the document's path, and `fs::load`
    // leaves that empty — an unstamped pre-image is a page no path-scoped
    // convention can see. The stamp is the ROOT-RELATIVE path: the gate runs
    // under the target root's own convention scope (D-B point 3).
    stamp_path(&mut target_doc, &target.rel);

    // `spec.selector` arrives tagged — the conversion from a human string
    // happens in the caller's own coat (`mrd pin`), never here, so this door
    // holds no address grammar at all.
    let asked = &spec.selector;
    let facts = wire_map::facts::read_facts(
        &wire_map::project_toc(&target_doc),
        target_doc.raw.as_bytes(),
    );
    let fact = match wire_map::facts::selector_matches(&facts, asked).as_slice() {
        &[fact] => fact,
        [] => {
            return Err(pin_target_missing(
                &spelled,
                format!(
                    "no section addressed by \"{}\" in {}. Nothing was written — the pin's \
                     page is byte-untouched. {}",
                    asked.display(),
                    spelled.0,
                    crate::section_recovery(&asked.display(), Some(spelled.0.as_str()))
                ),
            ));
        }
        many => {
            // No door may pin an occurrence the caller did not name (A.3, door
            // symmetry): the refusal is the same `ambiguous_ref` the read and
            // write doors mint, in the same voice.
            let mut e = match asked {
                wire::ReadSel::Anchor { anchor } => {
                    let mut e = ErrorBody::new(ErrorCode::AmbiguousRef);
                    e.candidates = Some(Vec::new());
                    e.message = Some(model::selector::render_anchor_ambiguity(
                        &format!("^{anchor}"),
                        many.len(),
                    ));
                    e
                }
                wire::ReadSel::Hpath { hpath } => {
                    // The same duplicates the facts plane matched, as resolver
                    // targets — the shared renderer names each by node index
                    // and `^block`, identical to the splice door's refusal.
                    let sec_ref = SecRef::Hpath {
                        hpath: hpath.clone(),
                    };
                    let targets: Vec<model::Target> = many
                        .iter()
                        .map(|f| model::Target {
                            span: span_range(f.span),
                            node_rev: model::NodeRev(f.sec_rev.clone()),
                        })
                        .collect();
                    ambiguous(&sec_ref, &target_doc, &targets)
                }
                wire::ReadSel::Dewey { .. } => {
                    unreachable!("a dewey selector matches at most one row")
                }
            };
            e.path = Some(spelled.clone());
            return Err(Box::new(e));
        }
    };
    // The canonical selector: what the caller asked resolved to, in the read
    // face's own tagged grammar — never the caller's spelling, and never a
    // dewey ordinal (an ordinal is positional and a pin must outlive the next
    // edit). This is the receipt key, the same key the mint side minted under
    // (one owner: `wire_map::facts::canonical_sel`).
    let selector = wire_map::facts::canonical_sel(fact);
    // Refusal messages still need a spelling to name back at a human.
    let selector_text = selector.display();
    // Captured before the promotion re-resolve borrows the doc again: anchor
    // rows carry a block id (heading rows do not), and the raw title is what the
    // D15 slug derives from.
    let fact_anchor = fact.anchor.clone();
    let title = fact.title.clone();
    // The raw segment array the lock's `path` array is built from.
    let fact_segments = fact.hpath.clone();

    let fact_span = span_range(fact.span);
    // The § A.3 proof gate: recompute the section's live `fp1.…` token over
    // the bytes on disk right now — under the flock this splice already
    // holds (the TARGET root's for a cross-root pin, so the compare runs
    // against the resolved target's own bytes, never an ambient same-named
    // file: the §8.2 bypass class) — and compare it to the token the
    // request carries. The pre-promotion span serves because the token is
    // anchor-removal-normalized: the promotion below cannot move it.
    let live_fp = mint_fingerprint(&target_doc, &fact_span, &spelled, &selector_text)?;
    pin_proof_gate(
        spec,
        actor,
        &spelled,
        &selector_text,
        &live_fp,
        &fact.sec_rev,
    )?;
    let slot = promotion_slot(&target_doc.raw, fact_span.start);
    // The occurrence ordinal of the pinned node itself — the canonical
    // selector's LEAF segment carries `n` exactly when siblings collide
    // (`raw_addresses` publishes minimal addresses), and that ordinal is what
    // de-collides the minted slug (r8 D2).
    let leaf_n = match &selector {
        wire::ReadSel::Hpath { hpath } => hpath.last().and_then(|s| s.n),
        _ => None,
    };
    let (anchor, promote) = decide_anchor(
        &target_doc,
        &spelled,
        fact_anchor.as_deref(),
        slot,
        &title,
        leaf_n,
        &selector_text,
    )?;

    // Compose the promotion in memory and mint from those bytes: the blob oid
    // is the whole file's content id, so taking it from the pre-promotion bytes
    // would record an oid for a state that ceases to exist the moment the
    // marker lands (and `--vibe` would eagerly write that unreachable blob).
    // The fingerprint agrees either way, because the promotion is rev-neutral.
    let mut gate = crate::gate::GatePass::default();
    let promoted = if promote {
        // The promotion is a write to the TARGET's root: its artifact guard,
        // stored-form guard and armed gate all run under THAT root's own law
        // (D-B point 3 — `gate_write` reads the workspace it is handed).
        let (candidate, pass) = plan_promotion(
            &target.root,
            &target.rel,
            &target_doc,
            slot,
            &anchor,
            actor,
            force,
        )?;
        gate = pass;
        Some(candidate)
    } else {
        None
    };
    let pinned_doc: &model::Document = promoted.as_ref().map_or(&target_doc, |c| c.document());

    let (span, segments) = if promote {
        post_promotion_facts(pinned_doc, &spelled, &selector)?
    } else {
        (fact_span, fact_segments)
    };

    let fingerprint = mint_fingerprint(pinned_doc, &span, &spelled, &selector_text)?;
    // D-D: the blob is asked of — and, under `--vibe`, written into — the
    // TARGET root's object store: drift/repair diff from the target's git
    // history, so the blob must live where that history lives.
    let blob = blob_oid(
        &target.root,
        &target.rel,
        promoted.as_ref().map(model::CandidateDocument::raw),
        spec.vibe.unwrap_or(false),
    )?;
    refuse_unrepresentable_heading(pinned_doc, &span, fact_anchor.as_deref(), &selector_text)?;
    let row = pin_row(&spelled, fact_anchor.as_deref(), &segments, blob.as_deref())?;

    Ok(PinMint {
        fact: wire::PinFact {
            target: spelled,
            selector,
            fingerprint,
            blob,
            anchor,
            promoted: promote,
        },
        row,
        span,
        promotion: promoted.map(|candidate| PendingPromotion {
            root: target.root.clone(),
            target: target.rel.clone(),
            before: target_doc,
            candidate,
        }),
        gate,
        target_flock,
    })
}

/// Re-resolve the pinned selector against the post-promotion bytes: the span the
/// fingerprint will cover, and the raw segment array. A promotion widens the
/// selector's node by the marker line, so the pre-promotion span would hash
/// bytes that are no longer the selector's.
///
/// Both come from one fact, so "the lock row describes the bytes that were
/// hashed" holds by construction.
///
/// # Errors
/// `pin_target_missing` when the selector no longer resolves after promotion.
fn post_promotion_facts(
    pinned_doc: &model::Document,
    target: &Path,
    selector: &wire::ReadSel,
) -> Result<(std::ops::Range<usize>, Vec<HpathSeg>), Box<ErrorBody>> {
    let facts = wire_map::facts::read_facts(
        &wire_map::project_toc(pinned_doc),
        pinned_doc.raw.as_bytes(),
    );
    let Some(fresh) = wire_map::facts::resolve_selector(&facts, selector) else {
        return Err(pin_target_missing(
            target,
            format!(
                "\"{}\" no longer resolves after promotion",
                selector.display()
            ),
        ));
    };
    Ok((span_range(fresh.span), fresh.hpath.clone()))
}

/// Mint the R4 lock row's structural fields — **the one-time conversion door**.
///
/// Everything the lock plane needs is derived here, from the target's own read
/// facts, and travels onward as [`PinRow`]. No later stage re-derives an address
/// by re-splitting a joined address spelling: `sanitize_heading` is many-to-one
/// and `/` is legal in a heading, so a split would be a guess.
///
/// **The anchor arm is the `path` arm.** R4 spells a block-anchor pin as a path
/// array whose sole element is the `^id` (`path: ["^findings"]`). It is a
/// block-grain claim and is never widened to the host section.
///
/// # Errors
/// - `bad_request` — a mixed array: heading segments and a `^id` element
///   together. That form has no ruled grain, so it is refused rather than
///   assigned a meaning here. Reachable two ways: an anchor fact arriving with a
///   heading chain, and a heading whose raw text begins with `^`.
/// - `io_error` — no blob oid. The hash is a field of the R4 claim, so a target
///   git cannot answer for refuses the pin rather than shipping a row that
///   cannot mean what the schema says it means.
fn pin_row(
    target: &Path,
    fact_anchor: Option<&str>,
    segments: &[HpathSeg],
    blob: Option<&str>,
) -> Result<PinRow, Box<ErrorBody>> {
    let elements = match fact_anchor {
        // Block grain, sole element, no promotion — R4's anchor form verbatim.
        Some(id) if segments.is_empty() => vec![format!("^{id}")],
        Some(id) => {
            return Err(bad_request(format!(
                "refused: the anchor ^{id} resolved with a heading chain as well, \
                 and R4 spells an anchor pin as a path array whose SOLE element is \
                 the ^id. A mixed array — headings AND an anchor — has no ruled \
                 meaning, so the engine will not invent one. Nothing was written."
            )));
        }
        // Heading grain: segments verbatim, each rendered through the R4
        // occurrence spelling — `n` rides the stored segment (`"Dup#2"`), so a
        // pin on a duplicate sibling is not born ambiguous (r8 D3). The read
        // side of the spelling is `view::walk::model_selector` via
        // `lock::parse_occurrence`; segments without `n` stay bare, and
        // `refuse_unrepresentable_heading` has already refused `#`-bearing
        // heading TEXT before this door, so the spelling cannot collide.
        None => segments
            .iter()
            .map(|s| lock::render_occurrence(&s.h, s.n))
            .collect(),
    };
    if fact_anchor.is_none()
        && let Some(bad) = elements.iter().find(|h| h.starts_with('^'))
    {
        return Err(bad_request(format!(
            "refused: the heading \"{bad}\" begins with `^`, so writing it into a \
             path array would be indistinguishable from R4's anchor form (a sole \
             ^id element) and the pin's GRAIN would become unreadable — block or \
             section, with no way to tell. Nothing was written. Give that section \
             its own ^id and pin that instead."
        )));
    }
    let Some(hash) = blob else {
        return Err(io_refusal(format!(
            "refused: git could not give a blob oid for {}, and an R4 pin has no \
             form without one — the hash IS the target's explicit meaning, so a \
             row missing it would claim less than it appears to. Nothing was \
             written. Run this inside a git work tree with git on PATH.",
            target.0
        )));
    };
    Ok(PinRow {
        // The vault-relative path minus `.md` — the link spelling
        // `model::CorpusIndex::resolve_ref` matches by whole subpath suffix, so
        // it addresses THIS file and not a same-named file in another folder.
        object: target.0.trim_end_matches(".md").to_string(),
        hash: hash.to_string(),
        // `HpathSeg::n` used to be dropped here on the theory that an n-less
        // address refuses loudly if it turns ambiguous later — but a pin minted
        // ON a duplicate sibling was then born ambiguous and greyed in the very
        // session that minted it (r8 D3, a broken attestation). The stored
        // selector is now the RESOLVED one. If a sibling is later deleted or
        // inserted, the ordinal resolves elsewhere and the FINGERPRINT is what
        // refuses — measured red, never a silent green.
        selector: lock::Selector::Path(elements),
    })
}

/// An `io_error` refusal carrying its cause — the shape [`blob_oid`]'s `--vibe`
/// arm already refuses with, reused so the two git-cannot-answer doors speak
/// with one voice.
fn io_refusal(cause: String) -> Box<ErrorBody> {
    let mut err = ErrorBody::new(ErrorCode::IoError);
    err.cause = Some(cause);
    Box::new(err)
}

/// The pin door's one representability refusal: a heading whose raw text
/// carries a `#`.
///
/// `#` is a live delimiter in the ingress grammars that address a pin —
/// wikilink block refs (`[[page#^id]]`), the CLI's `path#Fragment` split — and
/// a `#`-bearing heading is not representable end-to-end through them. A
/// `/`-bearing heading is: an R4 `path` array carries `["A/B", "leaf"]`
/// unambiguously.
///
/// # Errors
/// `bad_request` when no heading chain sits at the span, or when a heading in
/// the chain carries a `#`. The remedy is the node's own `^id`.
fn refuse_unrepresentable_heading(
    doc: &model::Document,
    span: &std::ops::Range<usize>,
    anchor_row: Option<&str>,
    selector: &str,
) -> Result<(), Box<ErrorBody>> {
    if anchor_row.is_some() {
        return Ok(()); // a block pin addresses by id; no heading text is involved
    }
    let Some(chain) = section_hpath_at(&doc.root, span.start) else {
        return Err(bad_request(format!(
            "cannot address \"{selector}\" as a section — no heading chain at that span"
        )));
    };
    if let Some(bad) = chain.iter().find(|h| h.contains('#')) {
        return Err(bad_request(format!(
            "the heading \"{bad}\" carries a `#`, which is still a live delimiter in the \
             grammars that address a pin from outside the engine (wikilink block refs, \
             the CLI's `path#fragment` split) — give that section an explicit ^id and \
             pin that instead"
        )));
    }
    Ok(())
}

/// A wire `Span` as a byte range. Every span this engine mints comes from a
/// `usize` file offset, so the narrowing is lossless in practice; saturating
/// beats panicking on a hypothetical 32-bit target, and an out-of-range span is
/// refused downstream by `model`'s char-alignment guarantor either way.
fn span_range(span: Span) -> std::ops::Range<usize> {
    let lo = usize::try_from(span.0).unwrap_or(usize::MAX);
    let hi = usize::try_from(span.1).unwrap_or(usize::MAX);
    lo..hi
}

/// The D15 stable handle for a pinned selector, and whether it must be promoted.
///
/// An id already in the selector's promotion slot is reused verbatim, which is
/// what makes a re-pin idempotent instead of growing one marker per pin. The
/// selector may also be a block anchor; either way nothing needs writing.
///
/// # Errors
/// `bad_request` when the title yields no id ([`slug_id`]), or when the id this
/// pin would mint ([`occurrence_slug`]) is already taken by another node —
/// refused rather than uniquified further, so the id stays a function of the
/// address alone (D15 as amended by r8 D2: title + occurrence ordinal). The
/// refusal's remedy is executable against the target as it stands — the r8
/// remedy named "that node's own ^id", which a duplicate sibling does not have.
fn decide_anchor(
    target_doc: &model::Document,
    target: &Path,
    fact_anchor: Option<&str>,
    slot: usize,
    title: &str,
    leaf_n: Option<u32>,
    selector: &str,
) -> Result<(String, bool), Box<ErrorBody>> {
    if let Some(id) = fact_anchor
        .map(ToOwned::to_owned)
        .or_else(|| anchor_on_line(target_doc, slot))
    {
        return Ok((id, false));
    }
    let slug = occurrence_slug(title, leaf_n)?;
    if !matches!(
        model::resolve(target_doc, &model::Ref::Anchor(slug.clone())),
        Err(model::ResolveError::NotFound)
    ) {
        return Err(bad_request(format!(
            "the slug id ^{slug} this pin would mint for \"{selector}\" is already \
             taken by another node in {file}, so the mint cannot give this section \
             a stable handle. Nothing was written. Fix in two round trips: append \
             a block id line of your own under that heading (a put op:append at \
             this same selector, body \"^your-id\"), read \"^your-id\" back (the \
             selector-exact read mints the receipt a session pin needs), then \
             pin at \"^your-id\" — or rename one of the colliding nodes.",
            file = target.0
        )));
    }
    Ok((slug, true))
}

/// The id a pin mints for a heading occurrence — the D15 slug of the title,
/// suffixed with the occurrence ordinal for a second-or-later same-named
/// sibling (`Dup` #2 → `dup-2`, r8 D2). Order-independent by construction: the
/// ordinal comes from the ADDRESS, not from which sibling pinned first, so the
/// id stays a pure function of (title, occurrence) and a claim-link decoration
/// can recompute it from the lock row alone.
///
/// The first occurrence keeps the bare slug — `n: Some(1)` and `n: None` mint
/// the same id, so pinning a heading that later gains a duplicate does not
/// strand its anchor.
///
/// # Errors
/// `bad_request` when the title yields no id characters ([`slug_id`]).
pub(crate) fn occurrence_slug(title: &str, n: Option<u32>) -> Result<String, Box<ErrorBody>> {
    let slug = slug_id(title)?;
    Ok(match n {
        Some(n) if n >= 2 => format!("{slug}-{n}"),
        _ => slug,
    })
}

/// Compose one anchor promotion and put it through the two rungs a target write
/// owes, without writing it: the promoted document, its exact bytes, and the
/// armed gate's pass (whose verdicts and forced skips the caller merges).
///
/// The promotion is the one write in a pin that does not ride `commit_batch`, so
/// both rungs live here rather than at the write site, while a refusal still
/// costs nothing:
///
/// - **the artifact guard**: a marker line must be lock-neutral, or this door
///   reaches the attestation bytes the batch door refuses to.
/// - **the armed gate**: the same `gate::gate_write` mount every other target
///   write passes, over this promotion's own before/after states. Rev-neutral is
///   not ungated — it is still a write to a page this actor may not own.
///
/// # Errors
/// The artifact guard's `bad_request`, or a `convention_fault` / `armed_drift` /
/// `binding_break` / `index_integrity` gate refusal.
fn plan_promotion(
    root: &fs::WorkspaceRoot,
    target: &Path,
    target_doc: &model::Document,
    slot: usize,
    anchor: &str,
    actor: Option<&str>,
    force: bool,
) -> Result<(model::CandidateDocument, crate::gate::GatePass), Box<ErrorBody>> {
    // The promotion's bytes and the document both rungs judge are one sealed
    // candidate — the same object `fs::replace_file` demands at the write site,
    // so this door cannot land bytes it never gated.
    let promoted =
        model::candidate_of_body(&target.0, promote_anchor(&target_doc.raw, slot, anchor));
    lock_artifact_guard(target_doc, promoted.document(), None, target)?;
    // The artifact guard at the promotion door: an anchor promotion inserts
    // `^slug` and nothing else, so it introduces no address — asserted rather
    // than assumed, because this door lands a second inode and would otherwise
    // be the one candidate no rung of the address plane ever reads.
    stored_form_guard_lazy(Some(target_doc), &promoted, target)?;
    let gate = crate::gate::gate_write(
        root,
        target_doc,
        promoted.document(),
        &[],
        policy::ChangeOp::Splice,
        actor,
        force,
        promoted.document(),
    )?;
    Ok((promoted, gate))
}

/// The raw heading chain of the section starting at `start` — `model` carries it
/// per node in delimiter-free array form, so nothing here re-derives an address.
fn section_hpath_at(node: &model::Node, start: usize) -> Option<Vec<String>> {
    if matches!(node.kind, model::NodeKind::Section { .. }) && node.span.start == start {
        return node.hpath.clone();
    }
    node.children
        .iter()
        .find_map(|c| section_hpath_at(c, start))
}

/// A real session identity, or `None` for the trusted door.
///
/// The bare CLI sends no actor and is local-operator-trusted — as `mrd put`
/// skips the host's authz — so proof is not REQUIRED of it. A blank actor is
/// absent too: an empty string is not an identity.
fn session_actor(actor: Option<&str>) -> Option<&str> {
    actor.map(str::trim).filter(|a| !a.is_empty())
}

/// The pin-proof gate (§ A.3 proof law), the whole refusal ladder in one
/// place. The caller proves it read the pinned content by carrying the
/// section's own `fp1.…` token from its read; `live_fp` is the engine's
/// recompute over the bytes on disk right now, under the caller's flock.
///
/// `actor == None` (or blank) is the bare CLI: local-operator-trusted, so an
/// ABSENT proof passes — but a supplied token is still compared (trust
/// excuses absence, never a wrong token). A real session actor must carry
/// `fingerprint`.
///
/// # Errors
/// `pin_proof_required` (a session actor carried no proof, or the token does
/// not match the live bytes and no stale `sec_rev` tells a moved world
/// apart), `write_conflict` (the supplied `sec_rev` is stale — the world
/// moved since the caller's read; § A.7's register: bad input is never
/// spoken as a moved world, and a moved world never as bad input).
fn pin_proof_gate(
    spec: &wire::PinSpec,
    actor: Option<&str>,
    target: &Path,
    selector_text: &str,
    live_fp: &str,
    live_sec_rev: &str,
) -> Result<(), Box<ErrorBody>> {
    let Some(proof) = spec.fingerprint.as_deref() else {
        if session_actor(actor).is_none() {
            return Ok(());
        }
        return Err(pin_proof_required(
            target,
            format!(
                "pin of \"{selector_text}\" in {} refused: the request carries no proof of \
                 read — you cannot attest content that was never in your context. Fix in one \
                 round trip: read \"{selector_text}\" in {} (a sections read, that exact \
                 selector), carry the `fingerprint` the read serves on that section into the \
                 pin, then send again.",
                target.0, target.0
            ),
        ));
    };
    if proof == live_fp {
        return Ok(());
    }
    // The mismatch split: a supplied stale `sec_rev` proves the world moved
    // since the caller's read — say that, with both revs. Without that
    // evidence the gate cannot tell a moved world from a wrong token, so the
    // refusal names both causes honestly and one remedy serves either.
    if let Some(read_rev) = spec.sec_rev.as_deref()
        && read_rev != live_sec_rev
    {
        let mut e = ErrorBody::new(ErrorCode::WriteConflict);
        e.path = Some(target.clone());
        e.expected = Some(NodeRev(read_rev.to_owned()));
        e.actual = Some(NodeRev(live_sec_rev.to_owned()));
        e.message = Some(format!(
            "pin of \"{selector_text}\" in {} refused: the section moved since your read — \
             your read served rev {read_rev} and it now carries {live_sec_rev}, so the \
             content your proof covers is no longer what disk holds. Re-read the selector \
             (the fresh read serves the fresh `fingerprint`) and pin again.",
            target.0
        ));
        return Err(Box::new(e));
    }
    // With a MATCHING `sec_rev` the raw bytes are provably what the read
    // served, so only the token can be at fault; with none supplied the two
    // causes are indistinguishable and the refusal names both honestly.
    let cause = if spec.sec_rev.as_deref() == Some(live_sec_rev) {
        "the section's raw bytes are exactly what your read served, so the token is not \
         from a read of this section"
    } else {
        "either the content moved since your read, or the token is not from a read of \
         this section"
    };
    Err(pin_proof_required(
        target,
        format!(
            "pin of \"{selector_text}\" in {} refused: the carried proof does not match the \
             section's live content — {cause}. Fix, either way, in one round trip: re-read \
             \"{selector_text}\" in {} (a sections read, that exact selector) and pin again \
             with the `fingerprint` that read serves.",
            target.0, target.0
        ),
    ))
}

/// `pin_proof_required` (§ A.3 proof law): a pin whose request carries no
/// usable proof of read. Fix class — read the exact selector, carry the
/// served `fingerprint`, then pin.
fn pin_proof_required(target: &Path, message: String) -> Box<ErrorBody> {
    let mut e = ErrorBody::new(ErrorCode::PinProofRequired);
    e.path = Some(target.clone());
    e.message = Some(message);
    Box::new(e)
}

/// Mint the pin's fingerprint over the bytes the promotion will land —
/// discharging [`model::fingerprint::fingerprint_span`]'s fallible owner.
///
/// This refusal is not the load-bearing guard: every ref form whose normalized
/// span can be empty is already refused at an earlier rung, so the rung is
/// measured-unreachable today (`tests/s2fix_empty_span_mint.rs` asserts each by
/// name). The guard that bites is on the verdict side
/// (`model::fingerprint::ContentVerdict::EmptySpan`), because the class arrives
/// through hand- or tool-authored `meridian-lock` blocks. It exists here so a
/// future read-face change that projects such a fact refuses instead of minting
/// a token that matches every document.
///
/// # Errors
/// `pin_target_missing` when the selector addresses no content to fingerprint.
fn mint_fingerprint(
    doc: &model::Document,
    span: &model::ByteSpan,
    target: &Path,
    selector: &str,
) -> Result<String, Box<ErrorBody>> {
    let removals = syntax::anchor_removals(&doc.raw);
    model::fingerprint::fingerprint_span(doc, span, &removals)
        .map(model::fingerprint::Fingerprint::into_string)
        .map_err(|model::fingerprint::EmptySpan| {
            pin_target_missing(
                target,
                format!(
                    "\"{selector}\" in {} addresses no content to fingerprint — its bytes \
                     canonicalize to nothing, and a fingerprint over nothing would match \
                     every document instead of this one. Pin a section or block that has \
                     content.",
                    target.0
                ),
            )
        })
}

/// `pin_target_missing`: the pin's page or selector does not resolve, so there
/// is nothing to fingerprint. Refusing at mint time beats writing a claim the
/// drift plane could only ever render `red(dangling)`.
fn pin_target_missing(target: &Path, message: String) -> Box<ErrorBody> {
    let mut e = ErrorBody::new(ErrorCode::PinTargetMissing);
    e.path = Some(target.clone());
    e.message = Some(message);
    Box::new(e)
}

/// The block id of an anchor whose host line starts at `line_start`, if the line
/// carries one. The idempotence probe against the promotion slot: the slot
/// either already bears a stable id (reuse it, promote nothing) or it does not
/// (mint the slug).
fn anchor_on_line(doc: &model::Document, line_start: usize) -> Option<String> {
    // A RAW-line probe since F-R4: the promoted marker's anchor NODE keys the
    // block it attaches to (the heading above the slot), so node spans no
    // longer witness the marker's own line — the line's bytes do.
    let bytes = doc.raw.as_bytes();
    if line_start >= bytes.len() {
        return None;
    }
    let end = bytes[line_start..]
        .iter()
        .position(|&b| b == b'\n')
        .map_or(bytes.len(), |p| line_start + p);
    let line = doc.raw[line_start..end].trim();
    let rest = line.strip_prefix('^')?;
    (!rest.is_empty() && rest.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
        .then(|| rest.to_string())
}

/// The D15 slug: a deterministic block id derived from the target's own heading
/// title (`"Leader's Guideline"` → `leaders-guideline`). Determinism makes a
/// re-pin recompute the same id, so promotion is idempotent.
///
/// Apostrophes are dropped rather than separating (`Leader's` is one word);
/// every other run outside the block-id charset (`[A-Za-z0-9-]`, §2.4)
/// collapses to a single `-`. A slug that collides with an id already on the
/// page is refused (see the caller) rather than uniquified, so the id stays a
/// function of the title alone.
///
/// # Errors
/// `bad_request` when the title yields no id characters at all (e.g. a wholly
/// non-ASCII heading) — the caller's remedy is to give the target node its own
/// `^id` and pin that.
pub(crate) fn slug_id(title: &str) -> Result<String, Box<ErrorBody>> {
    let mut out = String::new();
    let mut pending_dash = false;
    for ch in title.chars() {
        if ch == '\'' || ch == '\u{2019}' {
            continue;
        }
        if ch.is_ascii_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(ch.to_ascii_lowercase());
        } else {
            pending_dash = true;
        }
    }
    if out.is_empty() {
        return Err(bad_request(format!(
            "cannot derive a block id from the title \"{title}\" (nothing in the \
             [A-Za-z0-9-] charset, §2.4) — give the target node an explicit ^id and pin that"
        )));
    }
    Ok(out)
}

/// The **promotion slot**: the byte immediately after the terminator of the line
/// starting at `line_start` — i.e. the start of the selector's second line.
///
/// A promoted marker goes on its own line there, never at the heading line's
/// tail, and that placement is load-bearing twice over:
///
/// - **Address-neutral.** A heading's text is everything after its `#` run,
///   trimmed, so a tail marker would become part of the heading text and dangle
///   every existing pin and reader address for it.
/// - **Fingerprint-neutral (D14).** norm-v2's R2 removal takes an own-line
///   anchor's entire line including its terminator, so the section's canonical
///   bytes are identical before and after — which is what makes promoting into
///   a target this actor may not own honest.
fn promotion_slot(raw: &str, line_start: usize) -> usize {
    raw.as_bytes()[line_start..]
        .iter()
        .position(|&b| b == b'\n')
        .map_or(raw.len(), |p| line_start + p + 1)
}

/// Write `^id` on its own line at `slot` (see [`promotion_slot`]), matching the
/// file's own line terminator so a CRLF page stays CRLF.
///
/// # The file's EOF terminator state is preserved
/// Rev-neutrality is a claim about every pinned span in the target, so the
/// promotion may not move a byte outside the marker line. A marker landing at an
/// unterminated EOF therefore stays unterminated, and norm-v2's R2b takes it: an
/// own-line anchor with no terminator of its own is removed together with the
/// terminator before it — exactly the one this function added to give the marker
/// its own line. Held by
/// `promoting_at_eof_leaves_another_pages_pinned_fingerprint_identical`.
fn promote_anchor(raw: &str, slot: usize, id: &str) -> String {
    let head = &raw[..slot];
    let tail = &raw[slot..];
    let nl = if head.ends_with("\r\n") { "\r\n" } else { "\n" };
    let mut out = String::with_capacity(raw.len() + id.len() + 4);
    out.push_str(head);
    // An unterminated last line (the heading is the file's final line) needs its
    // own terminator before the marker line can start. norm-v2 R2b removes it
    // again with the marker, so it is inside the mask, not outside it.
    if !head.is_empty() && !head.ends_with('\n') {
        out.push_str(nl);
    }
    out.push('^');
    out.push_str(id);
    if tail.is_empty() {
        // At EOF: terminate the marker line only if the file was terminated.
        // Adding a terminator to a file that had none is a byte OUTSIDE the
        // marker line and outside norm-v2's mask (see this function's contract).
        if raw.ends_with('\n') {
            out.push_str(nl);
        }
    } else {
        out.push_str(nl);
        out.push_str(tail);
    }
    out
}

/// The target file's git blob oid — R4's per-pin `hash` (D5: git owns
/// content-addressing, so shell out). `vibe` additionally WRITES the blob into
/// the object store, so the pin is retrievable before any commit references it.
///
/// `pending` is the bytes the caller has DECIDED to write to `target` and not
/// written yet (an anchor promotion) — the oid must describe the state the file
/// will carry, so those bytes are hashed as if they were already there. `None`
/// asks about the file on disk, which is the same thing when nothing is pending.
///
/// `None` when git cannot answer (no repo, no git on PATH) — a fabricated sha
/// would be worse than no sha, so this function never invents one. Under v1 that
/// `None` merely dropped an `objects:` row and the claim still landed; under R4
/// the hash is a FIELD of the claim, so [`pin_row`] turns the same `None` into a
/// refusal. `--vibe` refuses here regardless: its entire purpose is the eager
/// write, so silently not writing would be a lie.
///
/// # Errors
/// `io_error{cause}` when `vibe` was asked for and git could not do it.
fn blob_oid(
    root: &fs::WorkspaceRoot,
    target: &Path,
    pending: Option<&str>,
    vibe: bool,
) -> Result<Option<String>, Box<ErrorBody>> {
    let repo = git::Repo::at(root.0.clone());
    let abs = root.0.join(&target.0);
    let ask = |write: bool| match pending {
        Some(bytes) => repo.blob_oid_of_bytes(&abs, bytes.as_bytes(), write),
        None if write => repo.write_blob(&abs),
        None => repo.blob_oid(&abs),
    };
    if vibe {
        return ask(true).map(Some).map_err(|e| {
            let mut err = ErrorBody::new(ErrorCode::IoError);
            err.cause = Some(format!(
                "--vibe asked for an eager blob write of {} and git refused: {e}",
                target.0
            ));
            Box::new(err)
        });
    }
    Ok(ask(false).ok())
}

/// Compose the pinning page's `meridian-lock` block as the batch's one
/// engine-minted span edit: union the pin into the page's existing lock
/// (`upsert_pin` — position-preserving, so a re-pin never drops or reorders a
/// sibling claim), render the canonical bytes, and hand back the span they
/// replace plus the minted block itself — the one byte form
/// [`lock_artifact_guard`] admits as a lock change.
///
/// # Errors
/// `bad_request` when the page's existing lock state is corrupt (malformed, or
/// more than one block — the sole writer mints exactly one, so adopting either
/// would launder corruption).
fn lock_engine_edit(
    doc: &model::Document,
    pinning_path: &Path,
    pin: &PinMint,
) -> Result<(model::EngineEdit, String), Box<ErrorBody>> {
    let found = find_lock(doc)?;
    let mut lock = found
        .as_ref()
        .map_or_else(lock::Lock::new, |f| f.lock.clone());
    // No ingress here: the R4 row arrived already structural from `mint_pin`
    // ([`pin_row`]), so this door parses nothing and splits nothing. The hash
    // rides the pin row it was minted for, so it cannot outlive the claim.
    lock.upsert_pin(lock::PinEntry::new(
        &pin.row.object,
        &pin.row.hash,
        pin.row.selector.clone(),
        &pin.fact.fingerprint,
    ));
    let edit = lock_block_splice(doc, found.map(|f| f.span), &lock).0;
    // Lock-is-content (#8 §5): the block sits inside the page's own span, so a
    // page pinning a span of itself that CONTAINS the lock edit pins bytes
    // this write is about to change — the claim could never be green. Refuse
    // rather than mint a permanently-red pin. Strict overlap only: a preamble
    // birth is a HEAD-side insert, which shifts a following span without
    // absorbing into it (the old EOF birth needed touching-counts because a
    // section running to EOF absorbed a tail insert). The rung still bites on
    // a legacy-placed block — replaced in place inside the very section being
    // pinned.
    if pin.fact.target.0 == pinning_path.0
        && !pin.span.is_empty()
        && edit.span.start < pin.span.end
        && edit.span.end > pin.span.start
    {
        return Err(bad_request(format!(
            "refused: this page's meridian-lock block sits INSIDE \"{}\", the very \
             section being pinned, so the pin would fingerprint bytes its own lock \
             write is about to change — the claim could never verify green \
             (lock-is-content, #8 §5). A fresh block births in the file preamble \
             (before the first heading), but an existing block is replaced in place, \
             so this one still sits where an older engine left it. By case: when \
             another page can hold the claim, pin from there; when this page should \
             carry it, move the meridian-lock block by hand to the file preamble \
             (after the frontmatter, before the first heading) and re-issue the pin. \
             Nothing was written",
            pin.fact.selector.display()
        )));
    }
    Ok((edit, lock::render(&lock)))
}

/// The `meridian-lock` block's byte form and its placement law, in one place —
/// shared by the pin path and [`lock_write`] so the two cannot drift.
///
/// A fresh block is birthed as FILE PREAMBLE — immediately after the
/// frontmatter (terminator-inclusive span; byte zero without one), before the
/// first heading — separated from following content by exactly one blank line.
/// The preamble belongs to no section (dogfood r3 F3): an EOF birth landed
/// inside the page's LAST section, inflating its word count with machinery,
/// serving YAML plumbing on its read face, and firing a `edited §Last` feed
/// row for a write the receipt never aimed there. Machinery also sits beside
/// machinery: frontmatter, then lock, then prose.
///
/// An existing block is replaced across its exact fence-to-fence span,
/// WHEREVER it sits — a legacy EOF block is not relocated, because relocation
/// would rewrite a region the caller never aimed at and would stale a self-pin
/// fingerprint minted before this edit composes. `lock::render` emits no
/// trailing newline, so terminators are this caller's. Returns the edit plus
/// whether it birthed.
fn lock_block_splice(
    doc: &model::Document,
    existing: Option<std::ops::Range<usize>>,
    lock: &lock::Lock,
) -> (model::EngineEdit, bool) {
    let raw = &doc.raw;
    let block = lock::render(lock);
    if let Some(span) = existing {
        return (model::EngineEdit { span, text: block }, false);
    }
    let at = doc
        .root
        .children
        .iter()
        .find(|c| matches!(c.kind, model::NodeKind::Frontmatter { .. }))
        .map_or(0, |fm| fm.span.end);
    // A frontmatter whose closing fence lacks its terminator gets one, so the
    // block always opens on its own line; the tail keeps (or gains) exactly
    // one blank line between the block and the body, and a tail-less file
    // ends with the block's one terminator.
    let lead = if at > 0 && !raw[..at].ends_with('\n') {
        "\n"
    } else {
        ""
    };
    let tail = &raw[at..];
    let sep = if tail.is_empty() || tail.starts_with('\n') {
        ""
    } else {
        "\n"
    };
    (
        model::EngineEdit {
            span: at..at,
            text: format!("{lead}{block}\n{sep}"),
        },
        true,
    )
}

/// The page's `meridian-lock` block, parsed, or `None` when it has none — the
/// one `crates/lock` read adapter. A present-but-broken lock is an error, never
/// "absent": adopting it would launder corruption.
///
/// # Errors
/// `bad_request` naming the `LockError` (malformed, unsupported version, or
/// multiple blocks).
fn find_lock(doc: &model::Document) -> Result<Option<lock::Found>, Box<ErrorBody>> {
    lock::find(doc).map_err(|e| {
        bad_request(format!(
            "the page's meridian-lock state is corrupt ({e:?}) — the engine is the sole \
             writer (#8 §3); repair the block by hand-removing it, then re-mint"
        ))
    })
}

/// The one `crates/lock` locate adapter: the page's existing block span
/// (fence-to-fence, terminator-exclusive), `None` when the page has no lock, or
/// a `bad_request` when the lock state is corrupt (multiple blocks or
/// unparseable YAML) — a hand-edited lock must be repaired deliberately, never
/// silently rewritten over.
fn locate_lock(doc: &model::Document) -> Result<Option<std::ops::Range<usize>>, Box<ErrorBody>> {
    Ok(find_lock(doc)?.map(|found| found.span))
}

/// Acquire the workspace write flock (D9) with the typed error split: a held
/// lock (`WouldBlock`, `LOCK_NB`) is the fast `workspace_busy` refusal
/// (transient — retry); any other lock-file I/O failure maps to
/// `io_error{cause}`.
fn acquire_write_lock(root: &fs::WorkspaceRoot) -> Result<fs::WriteLock, Box<ErrorBody>> {
    fs::WriteLock::acquire(root).map_err(|e| {
        if e.kind() == ErrorKind::WouldBlock {
            let mut w = ErrorBody::new(ErrorCode::WorkspaceBusy);
            w.message = Some(
                "another meridian writer holds .meridian/write.lock — transient; retry".into(),
            );
            Box::new(w)
        } else {
            let mut w = ErrorBody::new(ErrorCode::IoError);
            w.cause = Some(format!("write lock: {e}"));
            Box::new(w)
        }
    })
}

/// Workspace-root confinement: the same §1 path law the strict decode enforces
/// — no absolute path, no `.`/`..`/empty segment, and no root separator in the
/// head — so a write door can never escape the root via `root.join`. A violation
/// is `bad_path`, echoing the offending path.
///
/// The predicate lives in `addr::confined`, so the write doors here and the
/// resolver in `model` ask one implementation.
///
/// The head-colon arm is part of confinement because a `root:` prefix selects
/// WHICH tree a path is joined onto: a `root:`-bearing spelling at a write door
/// is an address, never a corpus path, and is refused rather than creating a
/// document no address can name (§4.2, D11).
fn path_confined(root: &fs::WorkspaceRoot, path: &Path) -> Result<(), Box<ErrorBody>> {
    if !addr::confined(&path.0) {
        let mut e = ErrorBody::new(ErrorCode::BadPath);
        e.path = Some(path.clone());
        let mut m = format!(
            "{} is not a workspace-relative path — the write doors admit only \
             workspace-relative spellings (§1 path law: no absolute path, no \
             `.`/`..`/empty segment, no `root:` prefix in the head). Nothing was written.",
            path.0
        );
        if let Some(rel) = relative_respelling(root, &path.0) {
            let _ = write!(
                m,
                " This path lies inside this workspace — respell it as `{rel}`."
            );
        }
        e.message = Some(m);
        return Err(Box::new(e));
    }
    Ok(())
}

/// The engine machinery directories a birth may never land under — engine
/// substrate rather than markdown-record homes (`docs/run-plane.md` § the
/// machinery floor, 2026-08-20): `.git` is the git directory, `.meridian` the
/// engine's stable state and run logs, `meridian` the attestation tree
/// (`meridian/armed-rules.md`, `meridian/attested`), and `receipts` the
/// receipt ledger.
///
/// Public so a cross-crate test can assert the family's membership, not just
/// each member's spelling — the [`fs::domain::RESERVED_PATHS`] precedent.
pub const MACHINERY_DIRS: &[&str] = &[".git", ".meridian", "meridian", "receipts"];

/// The machinery floor at the create door: refuse a birth whose landing
/// carries a [`MACHINERY_DIRS`] name as a path segment.
///
/// This is the ONE owner of birth containment, and it judges the RESOLVED
/// landing — deliberately the axis capabilities do not judge (caps read the
/// DECLARED coordinate's shape, never where the bytes land), so a grant as
/// narrow as `md.create:tasks/*.md` could otherwise land `tasks/x.md` under
/// `.git/` through the descriptor's own `base`. Every birth lane converges
/// here: the run-plane lane (starlark `create()`), the wire `create` op, the
/// birth preset, the realise card mint.
///
/// **At any depth**, not just the head segment: a nested root's machinery is
/// machinery too, and `results/ws/.git/x.md` corrupts a repository exactly as
/// `.git/x.md` does. Measured over the live sessions corpus before this
/// landed — every non-root occurrence of these four names was a nested root's
/// OWN machinery, never content — so the depth rule refuses no legitimate
/// birth.
///
/// **ASCII-case-insensitively**, because a case-insensitive filesystem
/// (macOS's default) lands `.GIT/x.md` inside `.git/`, and a guard a spelling
/// defeats is not a guard. Every occurrence in the live corpus is exactly
/// lowercase, so the wider match costs nothing.
///
/// The engine's own writes to these directories do not pass this door: the
/// armed artifact is written by [`crate::armed_disk`], the receipt rides the
/// batch commit ([`commit_set`]), and run logs use plain I/O. The ONE
/// exception is the hash-domain config — see [`is_domain_config`].
fn machinery_contained(path: &Path) -> Result<(), Box<ErrorBody>> {
    if is_domain_config(&path.0) {
        return Ok(());
    }
    let Some(segment) = path.0.split('/').find(|seg| {
        MACHINERY_DIRS
            .iter()
            .any(|dir| seg.eq_ignore_ascii_case(dir))
    }) else {
        return Ok(());
    };
    let mut e = ErrorBody::new(ErrorCode::BadPath);
    e.path = Some(path.clone());
    e.message = Some(format!(
        "the birth landing {} carries `{segment}` as a path segment — an engine machinery \
         directory the create door never births into, whatever the capabilities admit: \
         `.git` is the git directory, `.meridian` the engine's stable state, `meridian` the \
         attestation tree and `receipts` the receipt ledger, and none of them is a \
         markdown-record home. A capability scope judges the DECLARED path's shape; this \
         door judges the RESOLVED landing — re-aim the birth's `base` (or the caller's \
         ambient directory) at a content directory. Nothing was written.",
        path.0
    ));
    Err(Box::new(e))
}

/// Is `path` a workspace's own hash-domain config
/// ([`fs::domain::DOMAIN_CONFIG_PATH`], `meridian/domain.md`) — the one page
/// under a machinery directory that the floor must NOT refuse?
///
/// It sits beside the attestation artifacts but is not one of them: it is
/// AUTHORED content that declares the ignore list, and it is deliberately
/// inside its own hash domain, so it is born through this door like any other
/// page. Measured, not reasoned: the floor's first CI run refused it and took
/// down `domain_config_write_overlays_membership`,
/// `root_after_ignores_a_foreign_racer_on_every_door` and
/// `a_guarded_write_runs_zero_full_corpus_reads` — the resident write path
/// births it.
///
/// Matched at any depth, mirroring the deny side: a nested root's domain
/// config is that root's config. The spelling comes from `fs` so the
/// exemption cannot drift from the constant it exempts.
///
/// **Stated limit.** This is a hole in the floor, and a narrow one: a run
/// block granted a matching `md.create` scope can reach `meridian/domain.md`
/// through its own `base` and reshape which files the workspace attests. The
/// door cannot tell that block from a human authoring the same page — the
/// `actor` is caller-supplied — so closing it needs a policy axis this guard
/// does not have. Named here rather than left implied.
fn is_domain_config(path: &str) -> bool {
    match path.strip_suffix(fs::domain::DOMAIN_CONFIG_PATH) {
        Some("") => true,
        Some(prefix) => prefix.ends_with('/'),
        None => false,
    }
}

/// A pin target resolved to the root that serves it (cross-root design D-A):
/// a bare spelling stays in the pinning root; a `name:rel` spelling resolves
/// through the machine's mount table to that root's bound workspace.
struct PinTarget {
    /// The workspace the target lives in — the pinning root itself unless the
    /// spelling named another mounted root.
    root: fs::WorkspaceRoot,
    /// The target's path INSIDE `root` — the load path, the ledger key, and
    /// the promotion's landing path.
    rel: Path,
    /// The canonical mount name for a genuinely foreign target. `None` for a
    /// bare spelling AND for a rooted spelling that resolves to the pinning
    /// root itself (normalized to the bare form — one name per thing).
    mount: Option<addr::MountName>,
}

/// Resolve `pin.target` against the pinning root and the machine's mount
/// table. The ONE place the pin door reads the `[root:]path` grammar — the
/// same §4.1 head-colon law `addr::Addr::parse` holds, minus the fragment
/// arm (the target is a path position; `#` is an ordinary path byte there,
/// and the selector rides its own field).
///
/// Order is parse → confinement → table, so a malformed spelling refuses
/// without the mount table ever being read.
///
/// # Errors
/// `bad_path` — a malformed head (bad name, two colons, empty rel), an
/// escaping rel, an unreadable mount table, or a root the table does not
/// bind. Each refusal teaches its own remedy.
fn resolve_pin_target(
    root: &fs::WorkspaceRoot,
    target: &Path,
) -> Result<PinTarget, Box<ErrorBody>> {
    if !addr::head_carries_root_separator(&target.0) {
        path_confined(root, target)?;
        return Ok(PinTarget {
            root: root.clone(),
            rel: target.clone(),
            mount: None,
        });
    }
    let rooted = resolve_rooted_spelling(target, "the pin target")?;
    // A rooted spelling of the PINNING root itself is the same-root pin under
    // one of its names: normalize to the bare form. This also keeps the flock
    // plane single — a second LOCK_NB acquire on the already-held pinning
    // flock would refuse `workspace_busy` against ourselves.
    let pinning = std::fs::canonicalize(&root.0).unwrap_or_else(|_| root.0.clone());
    if rooted.workspace == pinning {
        return Ok(PinTarget {
            root: root.clone(),
            rel: Path(rooted.rel),
            mount: None,
        });
    }
    Ok(PinTarget {
        root: fs::WorkspaceRoot(rooted.workspace),
        rel: Path(rooted.rel),
        mount: Some(rooted.name),
    })
}

/// A rooted `root:rel` spelling, resolved through the machine's mount table
/// — grammar and table only, BEFORE any same-root normalization (that policy
/// is each door's own).
struct RootedSpelling {
    /// The MOUNT's canonical name — never the alias the caller spelled
    /// (`address-grammar.md` § 4.6a). This field reaches STORED BYTES: it
    /// becomes the `meridian-lock` object's `root:` prefix and the wire pin
    /// response's target, so an alias here would put one machine's private
    /// mapping into portable shared content, where it resolves to nothing.
    name: addr::MountName,
    /// The root's canonical bound path.
    workspace: PathBuf,
    /// The root-relative half, confined.
    rel: String,
}

/// The ONE engine-side reading of a rooted `root:rel` target at a write-door
/// position — the §4.1 head-colon law minus the fragment arm (a target is a
/// path position; `#` is an ordinary path byte there). Shared by the pin
/// door and the run plane's birth lane so two doors cannot hold two opinions
/// of one spelling. `what` is the caller's own noun for its refusal texts
/// ("the pin target", "the birth target").
///
/// Order is parse → confinement → table, so a malformed spelling refuses
/// without the mount table ever being read.
///
/// # Errors
/// `bad_path` — a malformed head (bad name, two colons, empty rel), an
/// escaping rel, an unreadable mount table, or a root the table does not
/// bind. Each refusal teaches its own remedy.
fn resolve_rooted_spelling(target: &Path, what: &str) -> Result<RootedSpelling, Box<ErrorBody>> {
    let head = target.0.split('/').next().unwrap_or(&target.0);
    let refuse = |message: String| -> Box<ErrorBody> {
        let mut e = ErrorBody::new(ErrorCode::BadPath);
        e.path = Some(target.clone());
        e.message = Some(message);
        Box::new(e)
    };
    if head.match_indices(':').count() > 1 {
        return Err(refuse(format!(
            "{} carries more than one `:` before the first `/` — exactly one colon may \
             separate a root from its path (§4.1). Nothing was written.",
            target.0
        )));
    }
    let colon = head.find(':').unwrap_or(0);
    let name = addr::MountName::parse(&head[..colon]).map_err(|e| refuse(format!("{e}")))?;
    let rel = &target.0[colon + 1..];
    if rel.is_empty() {
        return Err(refuse(format!(
            "{} names a root and no path — a root alone addresses nothing. Nothing was written.",
            target.0
        )));
    }
    if !addr::confined(rel) {
        return Err(refuse(format!(
            "{rel} is not a root-relative path — the rel half of a rooted target obeys \
             the same §1 path law as any write path (no absolute path, no `.`/`..`/empty \
             segment, no second `root:` prefix). Nothing was written.",
        )));
    }
    // The table, read fresh per call (the same currency law the resolver
    // holds): a missing or invalid ~/MERIDIAN.md means the name cannot be
    // resolved HERE, which is a loud refusal at a write door — never a grey.
    let Some(table) = machine_mount_table() else {
        return Err(refuse(format!(
            "{what} {} names root `{name}`, but this machine's mount table \
             (~/MERIDIAN.md) cannot be read, so the name resolves to no workspace. Declare \
             the root's mount there and retry. Nothing was written.",
            target.0
        )));
    };
    // Name first, then alias (`meridian-md-schema.md` §5.1b) — the pin door
    // shares the ONE lookup order, so a `sessions:` target names the same tree
    // here that it names at every read door.
    let bound = table
        .by_name_or_alias(name.as_str())
        .filter(|m| !m.state().refuses());
    let Some(mount) = bound else {
        let names: Vec<String> = table
            .mounts()
            .iter()
            .filter(|m| !m.state().refuses())
            .map(|m| match m.alias() {
                Some(alias) => format!("{} (alias {alias})", m.name()),
                None => m.name().to_owned(),
            })
            .collect();
        return Err(refuse(format!(
            "{what} {} names root `{name}`, which this machine does not bind \
             (bound roots: {}). A claim on an unbound root could never be walked or \
             checked from here. Declare the name in the target root's own MERIDIAN.md and \
             bind it in ~/MERIDIAN.md, or declare `alias: {name}` on the mount that holds \
             that tree — a root is looked up by name first and only then by alias, so the \
             tree may already be here under another name. Nothing was written.",
            target.0,
            if names.is_empty() {
                "none".to_string()
            } else {
                names.join(", ")
            }
        )));
    };
    let Some(target_root) = mount.canonical_path() else {
        return Err(refuse(format!(
            "{what} {} names root `{name}`, whose mount carries no canonical path. \
             Nothing was written.",
            target.0
        )));
    };
    // The MOUNT's name, never the caller's spelling: this name reaches the lock
    // object and the wire target, and an alias is a lookup spelling that means
    // nothing on the next machine to read those bytes (§ 4.6a).
    let name = addr::MountName::parse(mount.name()).map_err(|e| {
        refuse(format!(
            "{what} {} resolves to a mount named `{}`, which is not a canonical root name \
             ({e}). Nothing was written.",
            target.0,
            mount.name()
        ))
    })?;
    Ok(RootedSpelling {
        name,
        workspace: target_root.to_path_buf(),
        rel: rel.to_string(),
    })
}

/// Resolve one `md.create` birth target for the run plane — the face path
/// law carried onto the birth lane, with the boundary carried as DATA (ZT
/// ruling 2026-08-19 #2, superseding both the pre-joined `root:rel` path
/// spelling and any layout pattern):
///
/// - `path` is the birth's RELATIVE landing coordinate as the block declared
///   it — the same string the capability glob judges. It admits no rooted
///   spelling and no unconfined shape: the base axis exists so the two facts
///   never ride one glued string.
/// - `base` is the optional resolution base the descriptor carried (the
///   block's `--target` lane): a rooted `root:rel` ref resolves through the
///   one rooted lane ([`resolve_rooted_spelling`]) and must name the run's
///   own bound workspace — the run's births ride that workspace's ring,
///   locks, and armed law, so a foreign-root base refuses with a teaching —
///   or a confined workspace-relative directory.
/// - absent `base`, the caller's `ambient` directory is the default base
///   (md-create-ambient-paths, shape (c)); absent both, the path lands
///   workspace-root-relative (the documented bare-door behavior).
///
/// Returns the workspace-relative path the birth lands at. The create door
/// still runs its own confinement on it — this seam exists so the birth
/// lane, the receipt, and the dry row see the RESOLVED landing.
///
/// # Errors
/// `bad_path` — a rooted or unconfined `path`, a malformed or foreign
/// `base`/`ambient`, or an unconfined resolved landing. Nothing was written.
pub fn resolve_birth_target(
    root: &fs::WorkspaceRoot,
    path: &str,
    base: Option<&str>,
    ambient: Option<&str>,
) -> Result<String, Box<ErrorBody>> {
    let refuse = |message: String| -> Box<ErrorBody> {
        let mut e = ErrorBody::new(ErrorCode::BadPath);
        e.path = Some(Path(path.to_owned()));
        e.message = Some(message);
        Box::new(e)
    };
    if addr::head_carries_root_separator(path) {
        return Err(refuse(format!(
            "the birth path {path} carries a `root:` spelling — the path argument is \
             the RELATIVE landing coordinate (the string capability globs match), and \
             targeting rides the separate `base` argument: create(path = \"tasks/x.md\", \
             base = \"<root>:<session-dir>\"). Nothing was written.",
        )));
    }
    if !addr::confined(path) {
        return Err(refuse(format!(
            "the birth path {path} is not a confined relative path (no absolute path, \
             no `.`/`..`/empty segment). Nothing was written.",
        )));
    }
    let base_rel = match (base, ambient) {
        (Some(dir), _) if addr::head_carries_root_separator(dir) => {
            let rooted = resolve_rooted_spelling(&Path(dir.to_owned()), "the birth base")?;
            let bound = std::fs::canonicalize(&root.0).unwrap_or_else(|_| root.0.clone());
            if rooted.workspace != bound {
                return Err(refuse(format!(
                    "the birth base {dir} names root `{}`, which is not this run's \
                     bound workspace — a run-plane birth rides the bound workspace's \
                     ring, locks, and armed law, so it cannot land in a foreign tree. \
                     Run the page bound to that root instead. Nothing was written.",
                    rooted.name
                )));
            }
            Some(rooted.rel)
        }
        (Some(dir), _) => {
            if dir.is_empty() || !addr::confined(dir) {
                return Err(refuse(format!(
                    "the birth base `{dir}` is not a confined workspace-relative \
                     directory path (no absolute path, no `.`/`..`/empty segment) and \
                     not a rooted `root:rel` ref, so the birth path {path} cannot \
                     resolve under it. Nothing was written.",
                )));
            }
            Some(dir.to_owned())
        }
        (None, Some(dir)) => {
            if dir.is_empty() || !addr::confined(dir) {
                return Err(refuse(format!(
                    "ambient `{dir}` is not a confined workspace-relative directory \
                     path (no absolute path, no `.`/`..`/empty segment), so the bare \
                     birth path {path} cannot resolve under it. Nothing was written.",
                )));
            }
            Some(dir.to_owned())
        }
        (None, None) => None,
    };
    let resolved = match base_rel {
        Some(dir) => format!("{dir}/{path}"),
        None => path.to_owned(),
    };
    if !addr::confined(&resolved) {
        return Err(refuse(format!(
            "{resolved} is not a confined workspace-relative path (no absolute path, \
             no `.`/`..`/empty segment, no `root:` prefix past the head). Nothing was \
             written.",
        )));
    }
    Ok(resolved)
}

/// The machine's bound mount table, or `None` when no config resolves or the
/// table refuses to bind — the pin door's refusal texts own what that means.
fn machine_mount_table() -> Option<config::mount::MountTable> {
    let env = config::Env::from_process();
    let resolution = config::resolve(&env).ok()?;
    resolution.bind(&env).ok()
}

/// Same physical file across two (root, rel) spellings — the cross-root
/// generalization of [`same_file`]: a promotion landing in another root can
/// still be the pinning page when roots nest, and composing the batch against
/// the pre-promotion bytes would then splice at offsets the file no longer
/// has.
fn same_physical_file(
    root_a: &fs::WorkspaceRoot,
    a: &Path,
    root_b: &fs::WorkspaceRoot,
    b: &Path,
) -> bool {
    if root_a.0 == root_b.0 {
        return same_file(root_a, a, b);
    }
    match (
        std::fs::canonicalize(root_a.0.join(&a.0)),
        std::fs::canonicalize(root_b.0.join(&b.0)),
    ) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// Did this promotion land under the PINNING workspace? Physical containment,
/// so a same-root promotion answers yes, a disjoint cross-root one no, and a
/// target root nested inside the pinning workspace yes — exactly the set of
/// landings that move the pinning root's corpus cursor.
fn promotion_under(root: &fs::WorkspaceRoot, p: &PendingPromotion) -> bool {
    if p.root.0 == root.0 {
        return true;
    }
    let landed = std::fs::canonicalize(p.root.0.join(&p.target.0));
    let pinning = std::fs::canonicalize(&root.0).unwrap_or_else(|_| root.0.clone());
    matches!(landed, Ok(path) if path.starts_with(&pinning))
}

/// The committing workspace's spelling of a promotion target that landed
/// under it: the target's own rel when the roots coincide, else the physical
/// path re-based under the pinning root (a target root nested inside it).
/// `None` when no spelling exists — the caller leaves the row untold rather
/// than fabricating a path (degrade to re-derive, never to wrong data: the
/// watcher's next reconcile still names the change, actor-absent).
fn promotion_frame_path(root: &fs::WorkspaceRoot, p: &PendingPromotion) -> Option<String> {
    if p.root.0 == root.0 {
        return Some(p.target.0.clone());
    }
    let landed = std::fs::canonicalize(p.root.0.join(&p.target.0)).ok()?;
    let base = std::fs::canonicalize(&root.0).unwrap_or_else(|_| root.0.clone());
    landed
        .strip_prefix(&base)
        .ok()
        .map(|rel| rel.to_string_lossy().into_owned())
}

/// The workspace-relative respelling of an ABSOLUTE spelling that lies inside
/// `root`, or `None` when no respelling exists (relative violations, paths
/// outside the root). Teaching only — admission stays lexical (`addr::confined`).
///
/// Public because both doors teach it: the write door's [`path_confined`]
/// here, and the read door's `bad_path` face at the CLI (dogfood NEW-A —
/// one computation, so the two doors cannot train opposite habits). The
/// computation itself is [`fs::workspace_relative`] — the same one the run
/// doors key §2.1 receipts with, so a taught spelling and a receipted
/// spelling cannot drift; this wrapper only keeps the teaching's
/// absolute-spellings-only admission.
#[must_use]
pub fn relative_respelling(root: &fs::WorkspaceRoot, path: &str) -> Option<String> {
    if !path.starts_with('/') {
        return None;
    }
    fs::workspace_relative(root, path)
}

/// The §5.1 world guard, shared by `create`/`remove`: refuse `root_mismatch` if
/// a supplied `if_root` no longer matches the ambient root (the plan is stale).
/// A value that is not a premise token at all refuses `bad_request` first
/// (§5.7's malformed arm) — comparing it would claim the world moved.
fn world_guard(if_root: Option<&Root>, root_before: &Root) -> Result<(), Box<ErrorBody>> {
    if let Some(expected) = if_root {
        if let Some(refusal) = malformed_value(None, &expected.0) {
            return Err(refusal);
        }
        if *expected != *root_before {
            let mut e = ErrorBody::new(ErrorCode::RootMismatch);
            e.expected = Some(NodeRev(expected.0.clone()));
            e.actual = Some(NodeRev(root_before.0.clone()));
            return Err(Box::new(e));
        }
    }
    Ok(())
}

/// The §5.4 premise checks, widest-first (merkle-spec §7 ordering: root
/// premises, then folders shallowest-first, then file leaves) — the first
/// failing premise refuses the batch whole, and a failing wider premise
/// skips narrower work. Before any compare, every supplied value passes
/// §5.7's grammar wall ([`malformed_value`]) — input faults answer first,
/// like the §5.4 pair faults. Root premises (`scope: None`) compare against
/// `root_before` and refuse in the v2 shape (no `scope` field); scoped
/// premises resolve through the door's resident tree ([`fs::DomainCache::
/// scope_token`]) and their refusal carries `scope` (§5.7).
fn premise_guard(
    door: &WriteCacheHandle<'_>,
    premises: &[crate::guard::Premise],
    root_before: &Root,
) -> Result<(), Box<ErrorBody>> {
    if premises.is_empty() {
        return Ok(());
    }
    let mut ordered: Vec<&crate::guard::Premise> = premises.iter().collect();
    ordered.sort_by_key(|p| p.scope.as_ref().map_or(0, |s| 1 + s.components().count()));
    // §5.7's malformed arm, whole-list first: an ungrammatical premise VALUE
    // is an input fault — like the §5.4 pair faults it refuses before any
    // fold is compared, or token inequality would tell a moved-world story
    // about a damaged spelling (one leading space renders invisible in the
    // expected/live pair — dogfood break #7).
    for premise in &ordered {
        if let crate::guard::PremiseValue::Token(t) = &premise.value {
            let scope = premise.scope.as_ref().map(|s| s.to_string_lossy());
            if let Some(refusal) = malformed_value(scope.as_deref(), t) {
                return Err(refusal);
            }
        }
    }
    for premise in ordered {
        match &premise.scope {
            // The root premise as a list entry: the v2 world guard verbatim.
            None => match &premise.value {
                crate::guard::PremiseValue::Token(t) => {
                    world_guard(Some(&Root(t.clone())), root_before)?;
                }
                // `absent` at the root can never hold — the workspace tree
                // root always exists (merkle-spec §4.2.3). Honest mismatch:
                // the premise names a value the world does not hold.
                crate::guard::PremiseValue::Absent => {
                    let mut e = ErrorBody::new(ErrorCode::RootMismatch);
                    e.expected = Some(NodeRev("absent".to_owned()));
                    e.actual = Some(NodeRev(root_before.0.clone()));
                    return Err(Box::new(e));
                }
            },
            Some(scope) => {
                let scope_str = scope.to_string_lossy().into_owned();
                // Address-layer confinement, the premise analog of
                // `path_confined`: a scope that escapes the workspace cannot
                // hold a token — `scope_unresolved` (§5.6), never a probe
                // outside the root.
                if scope.is_absolute()
                    || scope
                        .components()
                        .any(|c| matches!(c, std::path::Component::ParentDir))
                {
                    return Err(scope_unresolved(&scope_str));
                }
                let outcome = {
                    let mut cache = door.cache.lock().unwrap_or_else(PoisonError::into_inner);
                    cache.scope_token(scope)
                };
                match outcome {
                    Ok(fs::ScopeToken::Token(live)) => match &premise.value {
                        crate::guard::PremiseValue::Token(t) if *t == live.0 => {}
                        crate::guard::PremiseValue::Token(t) => {
                            return Err(scoped_mismatch(&scope_str, t, &live.0));
                        }
                        crate::guard::PremiseValue::Absent => {
                            return Err(scoped_mismatch(&scope_str, "absent", &live.0));
                        }
                    },
                    Ok(fs::ScopeToken::Absent) => match &premise.value {
                        crate::guard::PremiseValue::Absent => {}
                        // §5.7's amended arm (dogfood break #6): a token
                        // premise at a node-less scope is bad input, never a
                        // narrated deletion — `(token, absent)` cannot
                        // distinguish a post-mint removal from a pairing that
                        // never held, and a `resync` here re-reads a path
                        // that serves nothing.
                        crate::guard::PremiseValue::Token(_) => {
                            return Err(token_at_absent_scope(&scope_str));
                        }
                    },
                    Err(fs::ScopeTokenError::Unresolved(refusal)) => {
                        return Err(scope_unresolved(&refusal.path));
                    }
                    Err(fs::ScopeTokenError::NoBaseline) => {
                        return Err(io_refusal(
                            "premise check before any observation — caller-order defect".to_owned(),
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

/// §5.7's malformed arm: `Some(bad_request)` when a premise VALUE is neither
/// the reserved `absent` (§5.6) nor a grammatical `Root`-family token
/// ([`model::parse_root`]). The teaching debug-quotes the raw bytes so
/// invisible damage — a leading space, the measured case — shows; version
/// families are untouched (a grammatical retired/future token parses).
fn malformed_value(scope: Option<&str>, raw: &str) -> Option<Box<ErrorBody>> {
    if raw == "absent" || model::parse_root(raw).is_some() {
        return None;
    }
    Some(bad_request(wire::malformed_premise_value_teaching(
        scope, raw,
    )))
}

/// A scoped `fingerprint_mismatch` (§5.7): expected/actual plus the premise's
/// `scope`, with the §8.2 register teaching. The root premise never mints
/// this shape — its refusal stays byte-identical to v2 (§5.1).
fn scoped_mismatch(scope: &str, expected: &str, actual: &str) -> Box<ErrorBody> {
    let mut e = ErrorBody::new(ErrorCode::RootMismatch);
    e.expected = Some(NodeRev(expected.to_owned()));
    e.actual = Some(NodeRev(actual.to_owned()));
    e.scope = Some(scope.to_owned());
    e.message = Some(wire::scoped_mismatch_teaching(scope, expected, actual));
    Box::new(e)
}

/// The §5.7 amended-arm refusal (dogfood break #6): a token premise at a
/// node-less scope — `scope_does_not_cover`, recovery `fix`, carrying `scope`
/// alone (`uncovered` stays §5.5's target-set extra; this mint home names no
/// target set). Never `fingerprint_mismatch`: the engine cannot know whether
/// the node was removed or never existed, and the retired absent-actual text
/// stated the first as fact.
fn token_at_absent_scope(scope: &str) -> Box<ErrorBody> {
    let mut e = ErrorBody::new(ErrorCode::ScopeDoesNotCover);
    e.scope = Some(scope.to_owned());
    e.message = Some(wire::token_at_absent_scope_teaching(scope));
    Box::new(e)
}

/// A `scope_unresolved` refusal (§5.6/§5.7) with its §8.2 register teaching.
fn scope_unresolved(scope: &str) -> Box<ErrorBody> {
    let mut e = ErrorBody::new(ErrorCode::ScopeUnresolved);
    e.scope = Some(scope.to_owned());
    e.message = Some(wire::scope_unresolved_teaching(scope));
    Box::new(e)
}

/// Mint the current §5.4 premise token at `scope` through the write plane's
/// own cache — the one mint home every premise family accepts (§4.7's scoped
/// arm serves through this). Observes the domain first (the same currency the
/// door's own entry pays), then folds the scope. `None` scope mints the root.
///
/// # Errors
/// Wire `io_error` on observation failure; `scope_unresolved` per §5.6.
pub fn scope_token(
    root: &fs::WorkspaceRoot,
    supplied: Option<&WriteCache>,
    scope: Option<&std::path::Path>,
) -> Result<Option<String>, Box<ErrorBody>> {
    // A bare cache, no vouch: the mint below live-observes unconditionally,
    // so the overlay-serve law the [`ResidentDoor`] carries has no arm here.
    let cache = supplied.map_or_else(|| write_cache(root), Arc::clone);
    let mut cache = cache.lock().unwrap_or_else(PoisonError::into_inner);
    // The observation both refreshes the resident tree and establishes the
    // baseline `scope_token` demands.
    let world = cache.root(root).map_err(|e| io_refusal(e.to_string()))?;
    match scope {
        None => Ok(Some(world.0)),
        Some(s) => {
            let scope_str = s.to_string_lossy().into_owned();
            if s.is_absolute()
                || s.components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                return Err(scope_unresolved(&scope_str));
            }
            match cache.scope_token(s) {
                Ok(fs::ScopeToken::Token(t)) => Ok(Some(t.0)),
                Ok(fs::ScopeToken::Absent) => Ok(None),
                Err(fs::ScopeTokenError::Unresolved(r)) => Err(scope_unresolved(&r.path)),
                Err(fs::ScopeTokenError::NoBaseline) => Err(io_refusal(
                    "scope mint before any observation — caller-order defect".to_owned(),
                )),
            }
        }
    }
}

/// Build a path-stamped `model::Document` from raw body bytes — the birth/death
/// state the gate seam and the Delta read (`model::build` is I/O-free and leaves
/// the path empty, so stamp it, mirroring `build_after_doc`).
fn build_doc(path: &Path, body: &str) -> model::Document {
    let mut doc = model::build(body.to_string(), syntax::parse(body));
    stamp_path(&mut doc, path);
    doc
}

/// Stamp a document's own path (`model::build` is I/O-free and leaves it empty).
/// The one writer of that field in this crate: the armed gate scopes its rules
/// by this value, so an unstamped pre-image is a page no path-scoped convention
/// can see.
fn stamp_path(doc: &mut model::Document, path: &Path) {
    if let model::NodeKind::Document { path: p, .. } = &mut doc.root.kind {
        p.clone_from(&path.0);
    }
}

/// Same file, not the same spelling. A pin whose target is the pinning page
/// reached through a different spelling of that path still writes the page the
/// batch is composed against, and composing against the pre-promotion bytes
/// would splice the lock block at an offset the file no longer has.
///
/// String equality answers the common case with no I/O; when the spellings
/// differ the filesystem answers.
fn same_file(root: &fs::WorkspaceRoot, a: &Path, b: &Path) -> bool {
    if a.0 == b.0 {
        return true;
    }
    let resolved = |p: &Path| std::fs::canonicalize(root.0.join(&p.0)).ok();
    match (resolved(a), resolved(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

/// The occupant's whole-file rev when `path` is occupied, else `None`. Occupancy
/// is `symlink_metadata` (a symlink or dangling link counts), so a birth cannot
/// dodge the `if_absent` CAS by aiming at a link.
fn occupant_rev(root: &fs::WorkspaceRoot, path: &Path) -> Result<Option<NodeRev>, Box<ErrorBody>> {
    if std::fs::symlink_metadata(root.0.join(&path.0)).is_ok() {
        Ok(Some(NodeRev(load_doc(root, path)?.root.node_rev.0)))
    } else {
        Ok(None)
    }
}

/// The "absent" file rev sentinel — the whole-file rev of empty content, used
/// as a create-CAS refusal's `expected` (an absent file is bytewise nothing).
fn absent_rev() -> NodeRev {
    NodeRev(
        model::build(String::new(), syntax::parse(""))
            .root
            .node_rev
            .0,
    )
}

/// A `cas_mismatch` frame (recovery `refresh` by the §8 binding) carrying the
/// pinned-vs-found revs — the create-CAS (`expected=absent`) and remove-CAS
/// (`expected=rev-read`) refusals both mint through here.
fn cas_mismatch(expected: &NodeRev, actual: &NodeRev) -> Box<ErrorBody> {
    let mut e = ErrorBody::new(ErrorCode::CasMismatch);
    e.expected = Some(expected.clone());
    e.actual = Some(actual.clone());
    Box::new(e)
}

/// Map a commit-path I/O error onto its wire envelope: the typed fs
/// write-conflict (D8 — live bytes drifted from the validated pre-image
/// between validate and rename) becomes `write_conflict` (refresh: re-read,
/// re-plan) carrying the drifted path; everything else is `io_error{cause}`.
fn commit_io_to_wire(err: &std::io::Error, path: &Path) -> Box<ErrorBody> {
    if fs::is_write_conflict(err) {
        let mut w = ErrorBody::new(ErrorCode::WriteConflict);
        w.path = Some(path.clone());
        w.message = Some(err.to_string());
        return Box::new(w);
    }
    let mut w = ErrorBody::new(ErrorCode::IoError);
    w.cause = Some(err.to_string());
    Box::new(w)
}

/// The I4 def-layer discovery anchor and refusal label: the target file's
/// absolute spelling — what a host passes the standalone `check_write` op.
/// `policy::defs` walks upward from it for `defs/` layers, so a
/// workspace-relative path would anchor the ladder at the process cwd instead of
/// the workspace.
fn conformance_target(root: &fs::WorkspaceRoot, path: &Path) -> String {
    root.0.join(&path.0).display().to_string()
}

/// Map an I4 conformance refusal onto its wire envelope: a `bad_request` frame
/// (recovery `fix`) carrying the ladder's `CODE: message — remedy` render
/// verbatim, plus the refused path. No new §8 reason is minted, so the frozen v2
/// error surface keeps its shape.
fn conformance_to_wire(refusal: &policy::defs::BodyError, path: &Path) -> Box<ErrorBody> {
    let mut e = bad_request(refusal.render());
    e.path = Some(path.clone());
    e
}

/// Map an `fs` I/O error onto its wire envelope: `NotFound` ⇒ `file_not_found`
/// (env), otherwise `io_error{cause}`.
fn io_to_wire(e: &std::io::Error) -> Box<ErrorBody> {
    if e.kind() == ErrorKind::NotFound {
        return Box::new(ErrorBody::new(ErrorCode::FileNotFound));
    }
    let mut w = ErrorBody::new(ErrorCode::IoError);
    w.cause = Some(e.to_string());
    Box::new(w)
}

/// Assemble the birth/death Delta at the one production constructor
/// ([`assemble_delta`]): a `created`/`deleted` file (absent-tense per §7.1 — no
/// `file_rev_before` on birth, no `file_rev_after` on death). `fd` is `None`
/// only if nothing changed, which a real create/remove never is. `flock` is the
/// caller's write lock, carried purely as the [`crate::seq::allocate`] witness.
#[expect(
    clippy::too_many_arguments,
    reason = "the 8th is the flock witness; bundling it into a struct would let a caller \
              build the struct without holding the lock, which is the one thing this \
              parameter exists to prevent"
)]
fn birth_death_delta(
    seq: Option<&dyn crate::seq::SeqSink>,
    flock: &fs::WriteLock,
    path: &Path,
    root_before: &Root,
    root_after: &Root,
    actor: Option<String>,
    now: Option<String>,
    fd: Option<&model::delta::FileDelta>,
) -> DeltaFrame {
    let files = fd
        .map(|fd| vec![wire_map::project_file_delta(&path.0, fd)])
        .unwrap_or_default();
    let seq = crate::seq::allocate(seq, flock, root_before, root_after, &files);
    assemble_delta(
        seq,
        root_before.clone(),
        root_after.clone(),
        actor,
        now,
        files,
    )
}

/// The §6.6 pre-flight: the requested receipt anchor passes the block-id
/// mint-guard and does not already stand in the receipt file — both answered
/// before any byte moves.
///
/// The engine reads the receipt file in the same act that appends to it, so a
/// collision with the requested anchor is visible up front. Answering it
/// afterwards is what made a refusal write: the append minted a second block
/// under one id, [`resolve_receipt_fact`] then found two and reported a corrupt
/// receipt over two already-committed files, and the `fix` it taught appended
/// the caller's content twice.
///
/// A receipt file that does not exist yet cannot collide — [`receipt_input`]
/// births it.
///
/// # Errors
/// `bad_request` when the anchor id fails the mint-guard, or when the receipt
/// file already carries that anchor — once, or already more than once.
fn preflight_receipt_anchor(
    root: &fs::WorkspaceRoot,
    receipt: Option<&ReceiptAddr>,
) -> Result<(), Box<ErrorBody>> {
    let Some(addr) = receipt else {
        return Ok(());
    };
    let target = model::Ref::anchor(addr.anchor.clone()).map_err(|_| {
        bad_request(format!(
            "receipt anchor ^{} is outside the block-id charset ([A-Za-z0-9-], §2.4), so no \
             door could address the receipt it would mint. No edit was applied; the batch is \
             refused whole. Fix: re-send with an anchor in that charset.",
            addr.anchor
        ))
    })?;
    if !root.0.join(&addr.path.0).exists() {
        return Ok(());
    }
    let receipt_doc = load_doc(root, &addr.path)?;
    let standing = match model::resolve(&receipt_doc, &target) {
        Err(model::ResolveError::NotFound) => return Ok(()),
        Ok(_) => 1,
        Err(model::ResolveError::Ambiguous(hits)) => hits.len(),
    };
    let mut e = bad_request(format!(
        "receipt anchor ^{} already stands in {} ({standing} block{} carr{} it), and an anchor \
         MUST be unique within the receipt file it names (§6.6): appending a second block under \
         that id publishes a receipt no strict door can address. No edit was applied; the batch \
         is refused whole. Fix: re-send with an anchor no block in {} carries — an id derived \
         from the invocation (`r-<invocation-id>`) never collides, a counter that restarts does.",
        addr.anchor,
        addr.path.0,
        if standing == 1 { "" } else { "s" },
        if standing == 1 { "ies" } else { "y" },
        addr.path.0,
    ));
    e.path = Some(addr.path.clone());
    Err(e)
}

/// The post-commit receipt fact: resolve the anchor in the just-committed
/// receipt file (host-block-leaf grain — the true after-state, §6.1). `None`
/// when the splice carried no receipt.
///
/// # Errors
/// The anchor id fails the mint-guard, or the committed anchor does not resolve
/// (a corrupt receipt) — both `bad_request`.
fn resolve_receipt_fact(
    root: &fs::WorkspaceRoot,
    receipt: Option<&ReceiptAddr>,
) -> Result<Option<ReceiptFact>, Box<ErrorBody>> {
    let Some(addr) = receipt else {
        return Ok(None);
    };
    let receipt_doc = load_doc(root, &addr.path)?;
    let target = model::Ref::anchor(addr.anchor.clone())
        .map_err(|_| bad_request("receipt anchor failed the mint-guard"))?;
    let resolved = model::resolve(&receipt_doc, &target)
        .map_err(|_| bad_request("committed receipt anchor did not resolve — receipt corrupt"))?;
    Ok(Some(ReceiptFact {
        path: addr.path.clone(),
        anchor: addr.anchor.clone(),
        node_rev: NodeRev(resolved.node_rev.0),
        span_after: Span(resolved.span.start as u64, resolved.span.end as u64),
    }))
}

// ---------------------------------------------------------------------------
// The `@fp` strip at document grain + the lock artifact guard
// ---------------------------------------------------------------------------
//
// Both rungs read the same candidate `after_doc` the ladder, the armed gate and
// the commit read, because both answer a question about bytes, not about a verb:
// identify tokens in the candidate, remove them from the payloads that carry
// them, and refuse what is left. A guard on the verb is not a guard on the file
// — the `meridian-lock` bytes the read-mint gate protects are ordinary page
// text, so every put shape is a door to them.

/// One `@fp` token run in the candidate, classified by who put it there.
enum FpOrigin {
    /// Bytes this batch supplies: request edit `edit`, at `local` inside its
    /// payload. Removable — this is the strip.
    Introduced {
        edit: usize,
        local: std::ops::Range<usize>,
    },
    /// A token already on disk, retained verbatim by this batch. Not this
    /// write's to remove: deleting bytes the batch never addressed would move
    /// the fingerprint of a node this write does not own, reddening pins that
    /// have nothing to do with it.
    Retained,
}

/// Classify every `@fp` token run in `after` — the one identification, shared by
/// the strip and by the assertion that follows it.
///
/// # Errors
/// `bad_request` when a token can be attributed to no single payload: the batch
/// composed it out of retained bytes plus its own, or two request edits contest
/// the same region.
fn classify_fp(
    doc: &model::Document,
    after: &model::Document,
    sealed: &model::ValidatedBatch,
    before_facts: &[model::Target],
    path: &Path,
) -> Result<Vec<FpOrigin>, Box<ErrorBody>> {
    let removals = syntax::fp_removals(&after.raw);
    if removals.is_empty() {
        return Ok(Vec::new());
    }
    let (inserted, retained) = splice_index(doc, sealed);
    let pre_existing = syntax::fp_removals(&doc.raw);

    let mut out = Vec::with_capacity(removals.len());
    for r in removals {
        if let Some((after_range, region)) = inserted
            .iter()
            .find(|(a, _)| a.start <= r.start && r.end <= a.end)
        {
            let edit = attribute_region(region, before_facts).ok_or_else(|| {
                bad_request(format!(
                    "refused: an @fp decoration token in {} cannot be attributed to any edit \
                     in this batch — the engine will not remove a claim token it cannot place",
                    path.0
                ))
            })?;
            out.push(FpOrigin::Introduced {
                edit,
                local: r.start - after_range.start..r.end - after_range.start,
            });
            continue;
        }
        let was_already_there = retained
            .iter()
            .find(|(a, _)| a.start <= r.start && r.end <= a.end)
            .is_some_and(|(after_range, pre_start)| {
                let start = pre_start + (r.start - after_range.start);
                pre_existing.contains(&(start..start + (r.end - r.start)))
            });
        if was_already_there {
            out.push(FpOrigin::Retained);
        } else {
            return Err(bad_request(format!(
                "refused: this write would COMPOSE an @fp claim token in {} out of bytes it \
                 does not supply — `@green.…` after a block ref is a render-face decoration the \
                 engine mints on read, never storable content (S10). Write the plain \
                 `[[page#^id]]` address; the tone and digest are computed, never authored",
                path.0
            )));
        }
    }
    Ok(out)
}

/// Each insertion in after coordinates, with the pre-image region that produced
/// it — [`splice_index`]'s first half.
type Inserted = Vec<(std::ops::Range<usize>, std::ops::Range<usize>)>;

/// Each surviving run in after coordinates, with the pre-image offset it came
/// from — [`splice_index`]'s second half.
type Retained = Vec<(std::ops::Range<usize>, usize)>;

/// **The after image, walked once — the one attribution index.**
///
/// The sealed spans index the pre-image and are sorted and disjoint, so a single
/// forward scan places every inserted text and every surviving run in after
/// coordinates, with no shift arithmetic. Returns `(inserted, retained)`:
/// `inserted` carries each insertion with the pre-image region that produced it;
/// `retained` carries each surviving run with the pre-image offset it came from.
///
/// Shared by the `@fp` strip ([`classify_fp`]) and U12's stored-form translation
/// ([`classify_cross_root`]), which ask the same question of the same candidate.
fn splice_index(doc: &model::Document, sealed: &model::ValidatedBatch) -> (Inserted, Retained) {
    let mut inserted: Inserted = Vec::with_capacity(sealed.edits.len());
    let mut retained: Retained = Vec::with_capacity(sealed.edits.len() + 1);
    let mut after_pos = 0usize;
    let mut pre_pos = 0usize;
    for e in &sealed.edits {
        let gap = e.span.start.saturating_sub(pre_pos);
        if gap > 0 {
            retained.push((after_pos..after_pos + gap, pre_pos));
            after_pos += gap;
        }
        inserted.push((after_pos..after_pos + e.text.len(), e.span.clone()));
        after_pos += e.text.len();
        pre_pos = e.span.end;
    }
    let tail = doc.raw.len().saturating_sub(pre_pos);
    if tail > 0 {
        retained.push((after_pos..after_pos + tail, pre_pos));
    }
    (inserted, retained)
}

/// Which request edit produced the sealed region — by the target span the model
/// itself resolved, never by text similarity. Disjointness is region-grain
/// (§4.4), so TARGET spans may nest: a non-empty region can sit inside two
/// nested targets, and a region contested past the boundary rule below is
/// `None` (refuse, never guess) — the callers turn that into a loud
/// `bad_request` rather than attributing an `@fp`/cross-root payload blind.
///
/// # The boundary rule, which containment alone cannot decide
/// Sections are contiguous: a section's span ends on the byte where its next
/// sibling's span begins. An `md.append_section` plans `put{at:"end"}`, whose
/// replaced region is empty and sits exactly on that shared byte (§4.4), so both
/// siblings contain it. The one the model planned it from is the one that ends
/// there. Empty regions are the only ones that can land on a shared byte, and
/// `put{at:"end"}` is the only shape that produces one (a `match` needle is
/// non-empty by validation), so this decides every case it applies to.
fn attribute_region(
    region: &std::ops::Range<usize>,
    before_facts: &[model::Target],
) -> Option<usize> {
    let containers: Vec<usize> = before_facts
        .iter()
        .enumerate()
        .filter(|(_, t)| t.span.start <= region.start && region.end <= t.span.end)
        .map(|(i, _)| i)
        .collect();
    if let [only] = containers.as_slice() {
        return Some(*only);
    }
    if !region.is_empty() {
        return None;
    }
    let mut hit = None;
    for &i in &containers {
        if before_facts[i].span.end == region.end {
            if hit.is_some() {
                return None;
            }
            hit = Some(i);
        }
    }
    hit
}

/// `text` with `ranges` (payload-local, non-overlapping, ascending) removed.
fn remove_ranges(text: &str, ranges: &[std::ops::Range<usize>]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for r in ranges {
        if r.start < cursor || r.end > text.len() {
            continue;
        }
        out.push_str(&text[cursor..r.start]);
        cursor = r.end;
    }
    out.push_str(&text[cursor..]);
    out
}

/// **The `@fp` strip, at document grain**: remove every token this batch
/// introduces from the payload that carries it, re-seal so the commit lands
/// exactly the judged bytes, and assert the candidate introduces none.
///
/// The batch is rewritten rather than the sealed copy because [`commit_batch`]
/// re-validates the request: a strip applied only to the sealed batch would
/// judge bytes the commit does not write.
///
/// # Errors
/// `bad_request` — an unattributable token (see [`classify_fp`]), a token in a
/// composed frontmatter line (unreachable: frontmatter is not a claim-link
/// position), a re-validation refusal, or a token still standing after the strip.
fn strip_fp_candidate(
    doc: &model::Document,
    root_before: &Root,
    path: &Path,
    before_facts: &[model::Target],
    batch: &mut model::SpliceRequest,
    sealed: &mut model::ValidatedBatch,
    after_doc: &mut model::CandidateDocument,
) -> Result<(), Box<ErrorBody>> {
    let mut per_edit: Vec<Vec<std::ops::Range<usize>>> = vec![Vec::new(); batch.edits.len()];
    let mut introduced = 0usize;
    for origin in classify_fp(doc, after_doc.document(), sealed, before_facts, path)? {
        if let FpOrigin::Introduced { edit, local } = origin {
            introduced += 1;
            per_edit
                .get_mut(edit)
                .map(|v| v.push(local))
                .ok_or_else(|| {
                    bad_request(format!(
                        "refused: an @fp token in {} attributes to the engine's own minted span, \
                     which composes no claim link",
                        path.0
                    ))
                })?;
        }
    }
    if introduced == 0 {
        return Ok(());
    }

    for (i, ranges) in per_edit.iter().enumerate() {
        if ranges.is_empty() {
            continue;
        }
        let payload = match &mut batch.edits[i].edit {
            // The composed `{key}: {value}` frontmatter line: its payload offsets
            // are not the sealed line's, and frontmatter carries no claim-link
            // position in the one grammar — so a token attributed here means the
            // grammar moved under this code. Refuse rather than splice blind.
            model::EditKind::Put {
                at: model::PutAt::Upsert,
                ..
            } => {
                return Err(bad_request(format!(
                    "refused: an @fp token attributed to a frontmatter property line in {} — \
                     frontmatter is not a claim-link position (S10/R22); the strip cannot place it",
                    path.0
                )));
            }
            model::EditKind::Put { text, .. } => text,
            model::EditKind::Match { new, .. } => new,
            // `remove` carries no text, so no scan can attribute a token to it
            // and the loop's empty-ranges guard has already skipped it. Reaching
            // here means the scan found a range in an edit with no bytes — the
            // grammar moved under this code.
            model::EditKind::Remove => {
                return Err(bad_request(format!(
                    "refused: an @fp token attributed to a `remove` edit in {} — it carries no \
                     text to strip",
                    path.0
                )));
            }
        };
        *payload = remove_ranges(payload, ranges);
    }

    *sealed = match model::validate_batch(
        doc,
        Some(&model::MerkleRoot(root_before.0.clone())),
        batch,
        None,
    ) {
        model::SpliceVerdict::Validated(b) => b,
        refused => {
            return Err(bad_request(format!(
                "refused: the batch no longer validates after its @fp decoration tokens were \
                 stripped ({refused:?}) — nothing was written"
            )));
        }
    };
    *after_doc = build_after_doc(doc, sealed, path);

    // The candidate introduces no token. Live on every write path, dry and real
    // alike — a door that reaches these bytes without passing the strip refuses
    // here instead of landing silently.
    if classify_fp(doc, after_doc.document(), sealed, before_facts, path)?
        .iter()
        .any(|o| matches!(o, FpOrigin::Introduced { .. }))
    {
        return Err(bad_request(format!(
            "refused: an @fp claim token survived the document-grain strip in {} — the write \
             was refused rather than landing a fingerprint claim the engine never minted",
            path.0
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// U12 — the stored-form translation, guarded at the artifact (D9)
// ---------------------------------------------------------------------------
//
// `put` translates an agent-plane `root:` address into the `obsidian://` stored
// form; `read` translates back (`crate::read`). The grammar and the positional
// law are `crate::positions`'; what lives here is where it lands:
//
// - the transform rewrites the payload that introduced an address, never the
//   assembled bytes. `fs::apply_batch` re-splices the sealed spans onto the disk
//   pre-image and refuses a candidate that is not that splice's own result, so a
//   transform applied to the assembled candidate would land bytes the primitive
//   rejects. The payload is also exactly what this write introduces: rewriting a
//   retained address would change bytes this batch never addressed and redden
//   unrelated pins.
// - the guard reads the candidate, on every door, dry and real alike. A door
//   that reaches these bytes without passing the transform refuses instead of
//   landing an agent-plane spelling on disk.

/// Every agent-plane cross-root address in the candidate, classified by who put
/// it there — the [`classify_fp`] shape over U12's grammar, sharing
/// [`splice_index`]'s one attribution law.
///
/// # Errors
/// `bad_request` when an address can be attributed to no single payload: the
/// batch composed it out of retained bytes plus its own, or two request edits
/// contest the same region (refuse, never transform blind).
fn classify_cross_root(
    doc: &model::Document,
    after: &model::Document,
    sealed: &model::ValidatedBatch,
    before_facts: &[model::Target],
    path: &Path,
    mounts: &addr::MountSet,
) -> Result<Vec<FpOrigin>, Box<ErrorBody>> {
    let occupants = crate::positions::agent_plane_occupants(&after.raw, mounts);
    if occupants.is_empty() {
        return Ok(Vec::new());
    }
    let (inserted, retained) = splice_index(doc, sealed);
    let pre_existing: Vec<std::ops::Range<usize>> =
        crate::positions::agent_plane_occupants(&doc.raw, mounts)
            .into_iter()
            .map(|o| o.span)
            .collect();

    let mut out = Vec::with_capacity(occupants.len());
    for o in &occupants {
        let r = &o.span;
        if let Some((after_range, region)) = inserted
            .iter()
            .find(|(a, _)| a.start <= r.start && r.end <= a.end)
        {
            let edit = attribute_region(region, before_facts).ok_or_else(|| {
                bad_request(format!(
                    "refused: the cross-root address '{}' in {} cannot be attributed to any edit \
                     in this batch — the engine will not translate an address it cannot place",
                    o.addr.target(),
                    path.0
                ))
            })?;
            out.push(FpOrigin::Introduced {
                edit,
                local: r.start - after_range.start..r.end - after_range.start,
            });
            continue;
        }
        let was_already_there = retained
            .iter()
            .find(|(a, _)| a.start <= r.start && r.end <= a.end)
            .is_some_and(|(after_range, pre_start)| {
                let start = pre_start + (r.start - after_range.start);
                pre_existing.contains(&(start..start + (r.end - r.start)))
            });
        if was_already_there {
            // Not this write's to translate: rewriting bytes the batch never
            // addressed would move the fingerprint of a node this write does not
            // own. The same rule the `@fp` strip follows.
            out.push(FpOrigin::Retained);
        } else {
            return Err(bad_request(format!(
                "refused: this write would COMPOSE the cross-root address '{}' in {} out of bytes \
                 it does not supply — the engine translates an address into its `obsidian://` \
                 stored form from the payload that carries it, and will not assemble one across a \
                 payload boundary. Write the whole address in one edit",
                o.addr.target(),
                path.0
            )));
        }
    }
    Ok(out)
}

/// **The stored-form translation, at the candidate** (D9): rewrite every
/// cross-root address this batch introduces into its `obsidian://` stored form,
/// re-seal so the commit lands exactly the judged bytes, and leave the artifact
/// guard behind it.
///
/// Mirrors [`strip_fp_candidate`] rung for rung — identify in the candidate,
/// attribute to the payload, rewrite the payload, re-validate, rebuild, assert.
/// The batch is rewritten rather than the sealed copy because [`commit_batch`]
/// re-validates the request.
///
/// # Errors
/// `bad_request` — an unattributable address ([`classify_cross_root`]), an
/// address with no stored form ([`crate::positions::TranslateError`]), a
/// re-validation refusal, or an agent-plane address still standing afterwards.
fn translate_stored_candidate(
    doc: &model::Document,
    root_before: &Root,
    path: &Path,
    before_facts: &[model::Target],
    batch: &mut model::SpliceRequest,
    sealed: &mut model::ValidatedBatch,
    after_doc: &mut model::CandidateDocument,
) -> Result<(), Box<ErrorBody>> {
    // The lazy gate: an ordinary single-root candidate never loads a mount
    // table, and never pays for one.
    if !crate::positions::may_carry_cross_root(after_doc.raw()) {
        return Ok(());
    }
    let mounts = crate::positions::machine_mounts();

    let mut per_edit: Vec<Vec<std::ops::Range<usize>>> = vec![Vec::new(); batch.edits.len()];
    let mut introduced = 0usize;
    for origin in classify_cross_root(
        doc,
        after_doc.document(),
        sealed,
        before_facts,
        path,
        &mounts,
    )? {
        if let FpOrigin::Introduced { edit, local } = origin {
            introduced += 1;
            per_edit
                .get_mut(edit)
                .map(|v| v.push(local))
                .ok_or_else(|| {
                    bad_request(format!(
                        "refused: a cross-root address in {} attributes to the engine's own \
                         minted span, which composes no link",
                        path.0
                    ))
                })?;
        }
    }
    if introduced == 0 {
        return Ok(());
    }

    for (i, ranges) in per_edit.iter().enumerate() {
        if ranges.is_empty() {
            continue;
        }
        let payload = match &mut batch.edits[i].edit {
            // Frontmatter is not an address position (§9.2 A-1): `root:` is a
            // live YAML key in the shipped preset/def grammar. Refuse rather
            // than translate blind — a blanket rewrite there would corrupt the
            // def and invalidate every pin whose fingerprint covers the line.
            model::EditKind::Put {
                at: model::PutAt::Upsert,
                ..
            } => {
                return Err(bad_request(format!(
                    "refused: a cross-root address attributed to a frontmatter property line in \
                     {} — frontmatter is not an address position (S10/R22, address-grammar § 9.2); \
                     the translation cannot place it",
                    path.0
                )));
            }
            model::EditKind::Put { text, .. } => text,
            model::EditKind::Match { new, .. } => new,
            // `remove` carries no text, so no scan can attribute an address to
            // it and the loop's empty-ranges guard has already skipped it.
            // Reaching here means the scan found a range in an edit with no
            // bytes — the grammar moved under this code.
            model::EditKind::Remove => {
                return Err(bad_request(format!(
                    "refused: a cross-root address attributed to a `remove` edit in {} — it \
                     carries no text to translate",
                    path.0
                )));
            }
        };
        *payload = crate::positions::to_stored(payload, &mounts)
            .map_err(|e| bad_request(format!("{e} (in {})", path.0)))?;
    }

    *sealed = match model::validate_batch(
        doc,
        Some(&model::MerkleRoot(root_before.0.clone())),
        batch,
        None,
    ) {
        model::SpliceVerdict::Validated(b) => b,
        refused => {
            return Err(bad_request(format!(
                "refused: the batch no longer validates after its cross-root addresses were \
                 translated to their stored form ({refused:?}) — nothing was written"
            )));
        }
    };
    *after_doc = build_after_doc(doc, sealed, path);

    // The closing rung goes through the same door-facing guard every other door
    // calls, so "which doors are guarded" is answerable by counting one name.
    stored_form_guard_lazy(Some(doc), after_doc, path)
}

/// **The stored-form translation at a whole-body door** (D9): the birth door
/// supplies its entire document, so the whole body is this write's payload and
/// "introduced" and "present" are the same set — no attribution walk is needed.
///
/// Ordered after the `@fp` strip like the splice door, and lazy for the same
/// reason: a body with no cross-root position never loads a mount table.
///
/// # Errors
/// `bad_request` — an address with no stored form
/// ([`crate::positions::TranslateError`]), naming the address and the fix.
fn translate_stored_body<'a>(
    body: std::borrow::Cow<'a, str>,
    path: &Path,
) -> Result<std::borrow::Cow<'a, str>, Box<ErrorBody>> {
    if !crate::positions::may_carry_cross_root(&body) {
        return Ok(body);
    }
    let mounts = crate::positions::machine_mounts();
    Ok(std::borrow::Cow::Owned(
        crate::positions::to_stored(&body, &mounts)
            .map_err(|e| bad_request(format!("{e} (in {})", path.0)))?,
    ))
}

/// **The artifact guard** (D9): the candidate introduces no agent-plane
/// cross-root address in an owned position.
///
/// Live on every write door in this module, dry and real alike. `before` is the
/// pre-image, or `None` for a birth — where "introduced" and "present" are the
/// same set. The comparison is introduce-scoped for the reason
/// [`classify_cross_root`] gives: an address a document already carried is not
/// this write's to move.
///
/// A door that reaches these bytes without passing the translation refuses here
/// instead of landing an agent-plane spelling on disk.
fn stored_form_guard(
    before: Option<&model::Document>,
    candidate: &model::CandidateDocument,
    path: &Path,
    mounts: &addr::MountSet,
) -> Result<(), Box<ErrorBody>> {
    let after = crate::positions::agent_plane_occupants(candidate.raw(), mounts);
    if after.is_empty() {
        return Ok(());
    }
    let mut standing: Vec<String> = after.iter().map(|o| o.addr.target()).collect();
    if let Some(before) = before {
        for carried in crate::positions::agent_plane_occupants(&before.raw, mounts) {
            let spelling = carried.addr.target();
            if let Some(i) = standing.iter().position(|s| *s == spelling) {
                standing.remove(i);
            }
        }
    }
    let Some(offender) = standing.first() else {
        return Ok(());
    };
    Err(bad_request(format!(
        "refused: the cross-root address '{offender}' survived the stored-form translation in {} \
         — a `root:` address is the AGENT plane's spelling and is unresolvable garbage to \
         Obsidian on disk; the stored form is an `obsidian://` URI carrying the vault name. The \
         write was refused rather than landing a link no reader can follow",
        path.0
    )))
}

/// The one door-facing entry to the artifact guard. Every byte-landing door in
/// this module discharges it, and it loads the mount table only if the candidate
/// can carry a cross-root position at all, so an ordinary single-root write
/// never pays for one. `tests/u12_door_enumeration.rs` counts the doors by this
/// one name.
fn stored_form_guard_lazy(
    before: Option<&model::Document>,
    candidate: &model::CandidateDocument,
    path: &Path,
) -> Result<(), Box<ErrorBody>> {
    if !crate::positions::may_carry_cross_root(candidate.raw()) {
        return Ok(());
    }
    stored_form_guard(before, candidate, path, &crate::positions::machine_mounts())
}

/// **The lock artifact guard**: the `meridian-lock` bytes change only as the pin
/// this call minted.
///
/// `minted` is the canonical block this splice's pin composed, or `None` when the
/// call carries no pin. The comparison is over raw block bytes
/// ([`lock::block_texts`]) rather than parsed values, so a change from one
/// unparseable block to another is still a change, and a page whose lock is
/// already corrupt can still be edited elsewhere without laundering it.
///
/// # Errors
/// `bad_request` — the candidate's lock bytes differ from the pre-image's and are
/// not exactly the minted block.
fn lock_artifact_guard(
    before: &model::Document,
    after: &model::Document,
    minted: Option<&str>,
    path: &Path,
) -> Result<(), Box<ErrorBody>> {
    let before_blocks = lock::block_texts(before);
    let after_blocks = lock::block_texts(after);
    if after_blocks == before_blocks {
        return Ok(());
    }
    if let Some(block) = minted
        && after_blocks == vec![block]
    {
        return Ok(());
    }
    Err(bad_request(format!(
        "refused: this write changes the meridian-lock block in {} without minting it. The lock \
         is the ATTESTATION artifact and the engine is its sole writer (#8 §3) — a pin is minted \
         by `splice.pin` (mrd pin), which fingerprints the target's real bytes behind the \
         read-mint gate. Lock bytes reaching disk as ordinary page text would be a claim nobody \
         computed. WHAT THIS WOULD DESTROY: the {} attestation claim(s) already minted on this \
         page. WHAT TO DO INSTEAD: fresh blocks live in the file preamble, which no section \
         write reaches — this refusal usually means the page carries a legacy-placed block \
         inside the section being written. When you meant to keep the claims, append with `put \
         at:end` or write a section that does not hold the block; when the legacy block is in \
         the way, move it by hand to the file preamble (after the frontmatter, before the first \
         heading) and re-issue the write. Retiring a claim on purpose needs an unpin verb, \
         which does not exist yet (stage 3) — until it does, remove the block by hand and \
         re-mint",
        path.0,
        before_blocks.len()
    )))
}

/// Per-target BEFORE facts + the wire→model edit conversion, request order
/// (§4.4: armed edits align 1:1 with request edits) — resolution failures name
/// the failing target exactly.
///
/// The address half of the `@fp` law is ordered here (the payload half is
/// [`strip_fp_candidate`]'s, at document grain). `Match{old}` is a needle matched
/// against stored bytes, which never carry a token, so a needle copied from the
/// decorated render face would otherwise never match its own document: an
/// address is compared, never stored. Every native and lowered edit passes
/// through this one funnel, so no put shape can skip it.
/// A caller-facing value scope on an `fm_key` target (§ A.6.3a): the two shapes
/// whose input is a FRAGMENT of a value rather than a composed line.
enum FmValueScope<'a> {
    /// `put{at:"end"}` — the fragment appends to the stored value.
    Append(&'a str),
    /// `match` — the fragment replaces a unique run inside the stored value.
    Match { old: &'a str, new: &'a str },
}

/// Classify an edit as a caller-facing `fm_key` value scope, or not one.
///
/// `at:"all"` and `at:"content"` are deliberately absent: they are the LOWERING's
/// own line slots (§ A.6.3a′ lowers `set_property` through `at:"all"` carrying an
/// already-encoded line), so they stay raw. Encoding or refusing there would
/// break `set_property`. `at:"upsert"` already encodes on its own path.
fn fm_value_scope<'a>(
    upsert_key: Option<&String>,
    edit: &'a EditShape,
) -> Option<FmValueScope<'a>> {
    upsert_key?;
    match edit {
        EditShape::Put {
            at: PutAt::End,
            text,
        } => Some(FmValueScope::Append(text)),
        EditShape::Match { old, new } => Some(FmValueScope::Match { old, new }),
        // `remove` is the IDENTITY shape, not a value scope (§ A.6.6): it
        // carries no value, so the § A.6.3a encoder has nothing to own and the
        // door has nothing to compose.
        EditShape::Put { .. } | EditShape::Remove {} => None,
    }
}

/// Whether the kernel's CAS check will refuse this edit, in which case the door
/// composes nothing and lets the kernel answer.
///
/// The kernel tests `if_node_rev` BEFORE it resolves a match region, so a door
/// that refused `no_match` first would report a typo where the world had moved
/// — and the mismatch ladder's rung-1 recovery is bound to `cas_mismatch`.
fn cas_defers(edit: &Edit, before: &model::Target) -> bool {
    edit.if_node_rev
        .as_ref()
        .is_some_and(|r| r.0 != before.node_rev.0)
}

/// Compose, encode and lower a caller-facing `fm_key` value scope (§ A.6.3a,
/// ruling `0021-fmkey-value-grain-ruling`).
///
/// The encoder takes a WHOLE value while these doors supply a FRAGMENT, so it
/// cannot simply be routed to: encoding the fragment alone emits
/// `owner: seed"hand: x"` — broken in a new way. The door therefore DECODES the
/// stored value (§ A.6.1), composes the caller's result, encodes the whole of it
/// through the one encoder every § A.6.3a door shares, and lowers to the
/// `at:"all"` line slot. Composing below the door is forbidden — the kernel
/// stays raw-grain because the run plane's `md.set_field` writes whole-value
/// grains through it.
///
/// The stored LINE feeds § A.6.3c, so a semantic no-op through either scope
/// keeps the stored spelling and leaves `prop_rev`, `span` and `props1` unmoved.
/// The key is carried from the stored bytes, never re-spelled from the target.
///
/// # Errors
/// The uniform § A.6.3a multi-line refusal, in the same words as every other
/// value-plane door; and `match`'s own `no_match`/`not_unique` refusals, which
/// the door mints itself because the composition happens above the kernel that
/// would otherwise mint them.
fn lower_fm_value_scope(
    doc: &model::Document,
    before: &model::Target,
    key: &str,
    scope: &FmValueScope<'_>,
) -> Result<model::EditKind, Box<ErrorBody>> {
    let stored_line = &doc.raw[before.span.clone()];
    // A key line without a colon is not a mapping line; the def checker's own
    // refusal is the honest one, so leave the bytes alone rather than invent a
    // composition over them.
    let Some(colon) = stored_line.find(':') else {
        return Err(bad_request(multi_line_value_refusal(key)));
    };
    let stored_key = &stored_line[..colon];
    let rest = &stored_line[colon + 1..];
    let stored_value = rest.strip_prefix(' ').unwrap_or(rest);
    let current = model::scalar::text(stored_value);

    let composed = match scope {
        FmValueScope::Append(text) => format!("{current}{text}"),
        FmValueScope::Match { old, new } => {
            let old = syntax::strip_fp(old);
            // The kernel's `match_region` rule, applied to the VALUE: unique,
            // non-overlapping, left→right. The grain moves; the arithmetic and
            // the refusals do not.
            let hits = current.matches(old.as_ref()).count();
            if hits == 0 {
                let mut e = ErrorBody::new(ErrorCode::NoMatch);
                e.matches = Some(0);
                return Err(Box::new(e));
            }
            if hits > 1 {
                let mut e = ErrorBody::new(ErrorCode::NotUnique);
                e.matches = Some(u32::try_from(hits).unwrap_or(u32::MAX));
                return Err(Box::new(e));
            }
            current.replacen(old.as_ref(), new, 1)
        }
    };

    let encoded = policy::defs::yaml_preserve_or_encode(Some(stored_line), &composed)
        .map_err(|_| bad_request(multi_line_value_refusal(key)))?;
    Ok(model::EditKind::Put {
        at: model::PutAt::All,
        text: format!("{stored_key}: {encoded}"),
    })
}

/// The § A.7 overlay candidate: apply plan-level edits to `doc` IN MEMORY —
/// lower → convert → validate → candidate, one reparse, no flock, no disk, no
/// receipt, no gate. The read-your-own-writes serve of the in-process script
/// lane builds its overlay document here so the overlay applies edits with
/// exactly the machinery the commit will apply them with: a set that cannot
/// apply refuses here in the same laws the commit would refuse in.
///
/// Deliberately NOT the write path: no guard demand (the overlay is a read
/// serve of the program's own arms; the commit's `guard_batch` still runs at
/// the real splice), no armed-plane gate, no verdicts — those bind writes,
/// and nothing here lands.
///
/// # Errors
/// The first refusing law, as its ordinary wire error body.
pub fn overlay_candidate(
    doc: &model::Document,
    path: &Path,
    plan_edits: &[wire::PlanEdit],
) -> Result<model::Document, Box<ErrorBody>> {
    let lowered = crate::plan::lower(doc, plan_edits)?;
    let (model_edits, _before) = model_edits_and_before_facts(doc, &lowered.edits, path)?;
    let batch = model::SpliceRequest {
        if_root: None,
        edits: model_edits,
        engine: None,
    };
    match model::validate_batch(doc, None, &batch, None) {
        model::SpliceVerdict::Validated(sealed) => {
            Ok(model::candidate_of_batch(&path.0, &doc.raw, &sealed).into_document())
        }
        refused => Err(verdict_to_wire(&refused, &lowered.edits, doc, path)),
    }
}

fn model_edits_and_before_facts(
    doc: &model::Document,
    edits: &[Edit],
    path: &Path,
) -> Result<(Vec<model::Edit>, Vec<model::Target>), Box<ErrorBody>> {
    let mut model_edits = Vec::with_capacity(edits.len());
    let mut before_facts = Vec::with_capacity(edits.len());
    for edit in edits {
        let target = to_model_ref(&edit.target)?;
        // The upsert door's key, kept for the multi-line refusal below: the
        // target is moved into the model edit before that arm runs, and the
        // refusal names the offending key (§ A.6.3a — one law, one sentence, at
        // both value-plane doors).
        let upsert_key = match &target {
            model::Ref::FmKey(key) => Some(key.clone()),
            _ => None,
        };
        // `put at:upsert` is the one create-or-replace shape: the `fm_key` may
        // not exist yet, so its BEFORE fact is synthesized (`fm_upsert_before`)
        // rather than resolved; a plain `resolve` would `ref_not_found` on the
        // very key the upsert is about to create. Two guards fence the verb to
        // its domain: the target must be an `fm_key`, and the value must be
        // single-line — the server composes `{key}: {value}`, so a newline would
        // forge extra frontmatter lines.
        remove_fence(doc, edit, upsert_key.as_deref())?;
        let before = if let EditShape::Put {
            at: PutAt::Upsert,
            text,
        } = &edit.edit
        {
            let model::Ref::FmKey(key) = &target else {
                return Err(bad_request(
                    "put at:upsert is valid only on an fm_key target",
                ));
            };
            if text.contains(['\n', '\r']) {
                return Err(bad_request(multi_line_value_refusal(key)));
            }
            model::fm_upsert_before(doc, key)
        } else {
            model::resolve(doc, &target).map_err(|e| {
                Box::new(match e {
                    model::ResolveError::NotFound => {
                        crate::read::ref_not_found(&edit.target, doc, path.0.as_str())
                    }
                    model::ResolveError::Ambiguous(c) => ambiguous(&edit.target, doc, &c),
                })
            })?
        };
        before_facts.push(before);
        model_edits.push(model::Edit {
            target,
            // A stale `if_node_rev` is refused BEFORE the composition is
            // attempted: the kernel checks CAS ahead of `match_region`, so a
            // door that minted `no_match` first would answer a moved world with
            // the wrong refusal — and the ladder's rung-1 recovery hangs off
            // `cas_mismatch`. Deferring to the kernel keeps ONE ordering.
            edit: match fm_value_scope(upsert_key.as_ref(), &edit.edit)
                .filter(|_| !cas_defers(edit, &before_facts[before_facts.len() - 1]))
            {
                // § A.6.3a caller-facing value scopes on an `fm_key` — composed
                // and encoded AT the door, then lowered to the line slot.
                Some(scope) => lower_fm_value_scope(
                    doc,
                    &before_facts[before_facts.len() - 1],
                    upsert_key.as_deref().unwrap_or(""),
                    &scope,
                )?,
                None => match &edit.edit {
                    // The identity shape lowers straight through: no value, no
                    // encode, no composition. The kernel plans the region —
                    // the key's grain span PLUS its terminator, and the whole
                    // block when this was its last key (§ A.6.6).
                    EditShape::Remove {} => model::EditKind::Remove,
                    EditShape::Match { old, new } => model::EditKind::Match {
                        old: syntax::strip_fp(old).into_owned(),
                        new: new.clone(),
                    },
                    EditShape::Put { at, text } => model::EditKind::Put {
                        at: match at {
                            PutAt::All => model::PutAt::All,
                            PutAt::Content => model::PutAt::Content,
                            PutAt::End => model::PutAt::End,
                            PutAt::Upsert => model::PutAt::Upsert,
                        },
                        // `put{at:"upsert"}` is a VALUE-plane door (wire-contract
                        // § A.6.3a): the caller's `text` is a flat string, so it
                        // passes the ONE encoder `set_property` writes through and
                        // `[[x]]` lands `"[[x]]"` instead of a nested flow
                        // sequence the I4 law would refuse. An existing key's
                        // stored line (the non-empty before span) feeds § A.6.3c,
                        // so a write-back of the served value keeps the stored
                        // spelling. The model kernel below stays raw-grain — the
                        // run plane's `md.set_field` rides `plan_fm_upsert` with
                        // whole-value grains that must land as sent.
                        text: if matches!(at, PutAt::Upsert) {
                            let stored_line = before_facts
                                .last()
                                .filter(|t| t.span.start < t.span.end)
                                .map(|t| &doc.raw[t.span.clone()]);
                            policy::defs::yaml_preserve_or_encode(stored_line, text).map_err(
                                |_| {
                                    bad_request(multi_line_value_refusal(
                                        upsert_key.as_deref().unwrap_or(""),
                                    ))
                                },
                            )?
                        } else {
                            text.clone()
                        },
                    },
                },
            },
            if_node_rev: edit
                .if_node_rev
                .as_ref()
                .map(|r| model::NodeRev(r.0.clone())),
        });
    }
    Ok((model_edits, before_facts))
}

/// The post-batch document state, built once (§4.4 one-reparse law): apply the
/// sealed span edits in memory → reparse → build, stamping the document path so
/// §11.1 verdicts carry it. Both the armed AFTER facts and the verdicts read
/// this doc, on both the dry and real paths.
fn build_after_doc(
    doc: &model::Document,
    sealed: &model::ValidatedBatch,
    path: &Path,
) -> model::CandidateDocument {
    model::candidate_of_batch(&path.0, &doc.raw, sealed)
}

/// The armed AFTER facts, resolved against the shared post-batch state
/// [`build_after_doc`] built — request order (§4.4: armed edits align 1:1 with
/// request edits).
///
/// A `create` row (`born[i]` carries its title) arms the BORN section, not
/// the parent the lowering appends under (§ A.3 create door; A.6.3a′ is the
/// precedent): target = the read face's published address for the born node,
/// `node_rev_before` = the born-from-nothing token, after facts = the born
/// node's own, from the same one reparse. The born node is identified by the
/// POSITION the sealed batch placed it ([`born_section_target`]) — never by
/// counting siblings, which an earlier same-batch edit's smuggled heading
/// would shift.
fn simulate_armed_edits(
    after_doc: &model::Document,
    edits: &[Edit],
    before_facts: &[model::Target],
    born: &[Option<crate::plan::Born>],
    sealed: &model::ValidatedBatch,
) -> Result<Vec<ArmedEdit>, Box<ErrorBody>> {
    let mut armed_edits = Vec::with_capacity(edits.len());
    for (i, (edit, before)) in edits.iter().zip(before_facts).enumerate() {
        if let Some(birth) = born.get(i).and_then(Option::as_ref) {
            armed_edits.push(born_armed_edit(after_doc, sealed, i, edit, birth)?);
            continue;
        }
        // The identity shape arms its own death (§ A.6.6). The
        // `target_identity` refusal below rests on the premise "the armed
        // facts are unrepresentable" — FALSE here, and that PREMISE is what
        // exempts this shape, never its spelling: `node_rev_before` is the key
        // line's real rev, `node_rev_after` is the no-node token A.6.3a′ arms
        // on the create arm, and `span_after` is the zero-width point the line
        // vacated. Ruling `decisions/0018` forbids keying this family on the
        // `at:` scope because a scope enumeration misses cells; nothing is
        // missed here, because a removal's target has no post-batch rev that
        // could stand still.
        if matches!(edit.edit, EditShape::Remove {}) && matches!(edit.target, SecRef::FmKey { .. })
        {
            // The zero-width point the struck line vacated. A batch that could
            // not place it (an impossible negative shift) falls back to the
            // pre-batch start rather than refusing a write that DID commit —
            // the removal is a fact on disk, and a fact is never withheld for
            // an offset.
            let point = landed_offset(sealed, i).unwrap_or(before.span.start) as u64;
            armed_edits.push(ArmedEdit {
                target: edit.target.clone(),
                node_rev_before: NodeRev(before.node_rev.0.clone()),
                node_rev_after: NodeRev(model::born_before_rev().0),
                span_after: Span(point, point),
            });
            continue;
        }
        let target = to_model_ref(&edit.target)?;
        let after = model::resolve(after_doc, &target).map_err(|_| {
            // A target whose identity does not survive its own edit (e.g. a
            // heading rewritten by put at:all) has no worked armed shape in
            // the frozen text — refuse loud rather than invent one. This is
            // the §4.4 `target_identity` family: the same post-reparse death
            // as containment loss, one code, discriminated by `family`.
            let mut e = ErrorBody::new(ErrorCode::WouldCorrupt);
            e.family = Some(WouldCorruptFamily::TargetIdentity);
            e.target = Some(edit.target.clone());
            // The teaching rides the wire, not one face: this refusal has a
            // `message`, and every face prefers it, so a remedy written into
            // the CLI alone would never reach the host doors.
            e.message = Some(format!(
                "target identity does not survive the edit — \"{}\" does not resolve after this \
                 batch, so its armed facts are unrepresentable. {} Fix: re-supply the identity \
                 the slot overwrites — a section heading for `at:\"all\"`, a line-final block id \
                 for `at:\"end\"` on an anchor; to RETIRE an identity, write through the parent's \
                 content slot instead of its own.",
                target_display(&edit.target),
                crate::NO_PARTIAL_WRITE_CLAUSE
            ));
            Box::new(e)
        })?;
        armed_edits.push(ArmedEdit {
            target: edit.target.clone(),
            node_rev_before: NodeRev(before.node_rev.0.clone()),
            node_rev_after: NodeRev(after.node_rev.0.clone()),
            span_after: Span(after.span.start as u64, after.span.end as u64),
        });
    }
    Ok(armed_edits)
}

/// The armed fact of one birth: locate the born heading by the position the
/// sealed batch placed it, read its published address off the after-doc's own
/// read-facts table, and resolve that address for the after facts.
///
/// The position is exact, not a heuristic: the sealed edits are disjoint and
/// applied back-to-front, so batch edit `i`'s text lands at its own region
/// start plus the length shift of every sealed edit ordered before it —
/// same-point inserts keep request order under the seal's stable sort. The
/// born heading's offset within the lowered text is STATED by the lowering
/// ([`crate::plan::Born::heading_offset`]) — the § A.3 hygiene composition
/// derives the separators from the document, so the reader no longer assumes
/// them. The FACTS still come from the real reparse; arithmetic only picks
/// which node to read.
///
/// # Errors
/// `would_corrupt{target_identity}` when the reparse leaves no section
/// heading at the placed position, or the published address does not resolve
/// — a neighbouring edit's bytes destroyed or absorbed the birth, so its
/// armed facts are unrepresentable.
fn born_armed_edit(
    after_doc: &model::Document,
    sealed: &model::ValidatedBatch,
    batch_index: usize,
    edit: &Edit,
    birth: &crate::plan::Born,
) -> Result<ArmedEdit, Box<ErrorBody>> {
    let title = birth.title.as_str();
    let refuse = || birth_unrepresentable(edit, title);
    let pos = sealed
        .edits
        .iter()
        .position(|e| e.index == batch_index)
        .ok_or_else(refuse)?;
    let landed = landed_offset(sealed, batch_index).ok_or_else(refuse)?;
    // The stated offset must point at a heading opener in the sealed text; a
    // sealed text rewritten out of shape refuses rather than misplacing the
    // birth.
    if sealed.edits[pos].text.as_bytes().get(birth.heading_offset) != Some(&b'#') {
        return Err(refuse());
    }
    let heading_start = landed + birth.heading_offset;
    let hpath = crate::plan::published_hpath_at(after_doc, heading_start).ok_or_else(refuse)?;
    let target = SecRef::Hpath { hpath };
    let model_ref = to_model_ref(&target)?;
    let after = model::resolve(after_doc, &model_ref).map_err(|_| refuse())?;
    Ok(ArmedEdit {
        target,
        node_rev_before: NodeRev(model::born_before_rev().0),
        node_rev_after: NodeRev(after.node_rev.0.clone()),
        span_after: Span(after.span.start as u64, after.span.end as u64),
    })
}

/// The POST-batch byte offset that batch edit `batch_index`'s region start
/// lands at — the pre-batch start shifted by the net length change of every
/// sealed edit ordered before it.
///
/// Exact, not a heuristic: the sealed edits are disjoint and their order is the
/// pre-batch offset order (same-point inserts keep request order under the
/// seal's stable sort), so every earlier edit's shift is fully applied and no
/// later edit's is. The totals are kept as unsigned added/removed halves
/// because the DIFFERENCE can be negative while the final position cannot — a
/// landed offset below zero is an impossibility, which the `checked_sub` turns
/// into `None` for the caller to refuse on rather than a wrapped number.
///
/// Two readers: a birth (which heading position to read the born node from) and
/// a `remove` (the zero-width point the struck line vacated, § A.6.6).
fn landed_offset(sealed: &model::ValidatedBatch, batch_index: usize) -> Option<usize> {
    let pos = sealed.edits.iter().position(|e| e.index == batch_index)?;
    let (added, removed) = sealed.edits[..pos]
        .iter()
        .fold((0usize, 0usize), |(a, r), e| {
            (a + e.text.len(), r + e.span.len())
        });
    (sealed.edits[pos].span.start + added).checked_sub(removed)
}

/// The birth's own `target_identity` refusal: the batch commits nothing, and
/// the message names the address the caller asked to bear.
fn birth_unrepresentable(edit: &Edit, title: &str) -> Box<ErrorBody> {
    let mut e = ErrorBody::new(ErrorCode::WouldCorrupt);
    e.family = Some(WouldCorruptFamily::TargetIdentity);
    e.target = Some(edit.target.clone());
    e.message = Some(format!(
        "the born section's armed facts are unrepresentable — after this batch no section \
         heading stands where \"{}/{}\" was placed, so another edit's bytes destroyed or \
         absorbed the birth. {} Fix: land the create in its own batch, or repair the edit \
         whose text swallows the new heading.",
        target_display(&edit.target),
        title,
        crate::NO_PARTIAL_WRITE_CLAUSE
    ));
    Box::new(e)
}

/// The receipt append for a real commit: render the line (§6.1), honor the
/// parent-dir obligation (fs does not mkdir — the production caller does, real
/// commits only), and fold the append at the receipt file's EOF.
fn receipt_input(
    root: &fs::WorkspaceRoot,
    args: &SpliceArgs,
    edits: &[Edit],
    root_before: &Root,
    armed_edits: &[ArmedEdit],
    born: &[Option<crate::plan::Born>],
    addr: &ReceiptAddr,
) -> Result<(String, model::ReceiptAppend), Box<ErrorBody>> {
    let io_err = |e: std::io::Error| {
        let mut err = ErrorBody::new(ErrorCode::IoError);
        err.cause = Some(e.to_string());
        Box::new(err)
    };
    let facts = receipt::ArmedFacts {
        id: args.id,
        path: &args.path,
        actor: args.actor.as_deref(),
        now: args.now.as_deref(),
        root_before,
        anchor: &addr.anchor,
        // The armed target — identical to the request target on every edit
        // except a birth, whose fact names the born section. The receipt
        // renders the armed facts (§6.4/§6.1: same facts, one set), so the
        // two surfaces cannot split; a birth's op token is `create`, the op
        // the caller asked, not the lowering's parent-append mechanism.
        edits: edits
            .iter()
            .zip(armed_edits)
            .enumerate()
            .map(|(i, (req, armed))| receipt::EditFact {
                target: &armed.target,
                op: if born.get(i).is_some_and(Option::is_some) {
                    receipt::OpFact::Create
                } else {
                    receipt::OpFact::Edit(&req.edit)
                },
                before: &armed.node_rev_before,
                after: &armed.node_rev_after,
            })
            .collect(),
    };
    let line = receipt::render_line(&facts);
    let receipt_abs = root.0.join(&addr.path.0);
    let receipt_len = match std::fs::read(&receipt_abs) {
        Ok(bytes) => bytes.len(),
        Err(e) if e.kind() == ErrorKind::NotFound => 0,
        Err(e) => return Err(io_err(e)),
    };
    if let Some(parent) = receipt_abs.parent() {
        std::fs::create_dir_all(parent).map_err(io_err)?;
    }
    Ok((
        addr.path.0.clone(),
        model::ReceiptAppend {
            span: receipt_len..receipt_len,
            text: format!("{line}\n"),
        },
    ))
}

/// A target in the caller's own spelling, for refusal messages.
fn target_display(sec: &SecRef) -> String {
    match sec {
        SecRef::Hpath { hpath } => crate::display_hpath(hpath),
        SecRef::Anchor { anchor } => format!("^{anchor}"),
        SecRef::FmKey { fm_key } => fm_key.clone(),
    }
}

/// The two fences on the identity shape (§ A.6.6), lifted out of the lowering
/// loop so each carries its own reason.
///
/// 1. **`fm_key` targets only.** A section and an anchor already retire through
///    the parent's content slot NAMING that parent (§4.4 `target_identity`), so
///    a `remove` there would be a SECOND spelling of a capability that exists —
///    the thing this contract forbids. The refusal says which plane the target
///    is, so the caller learns the rule rather than the row.
/// 2. **Not the block's last key.** Neither downstream outcome is available and
///    both die worse: bare fences are not frontmatter to this engine (the next
///    property write synthesizes a second block above them), and a blockless
///    document is refused outright by the def plane (`unreadable frontmatter:
///    <nil>`, never forceable) — three layers down, in a message about NESTED
///    frontmatter that misdiagnoses the write that drew it.
///
/// # Errors
/// `bad_request` on either fence, naming the target or the key.
fn remove_fence(
    doc: &model::Document,
    edit: &Edit,
    upsert_key: Option<&str>,
) -> Result<(), Box<ErrorBody>> {
    if !matches!(edit.edit, EditShape::Remove {}) {
        return Ok(());
    }
    let Some(key) = upsert_key else {
        return Err(bad_request(format!(
            "`remove` is valid only on an fm_key target — `{}` is a {}, and a {} retires through \
             its parent's content slot, not through a shape of its own.",
            target_display(&edit.target),
            sec_ref_plane(&edit.target),
            sec_ref_plane(&edit.target),
        )));
    };
    if model::fm_remove_empties_block(doc, key) {
        return Err(bad_request(format!(
            "`{key}` is the only key in this record's frontmatter, and removing it would leave \
             the record with no frontmatter block — which the def plane refuses as `unreadable \
             frontmatter: <nil>`. A record's frontmatter is its identity surface, so striking its \
             last key is not a property edit. Fix: set another key first if the record should \
             live, or retire the whole record with `op:\"remove\"` if it should not."
        )));
    }
    Ok(())
}

/// The plane a target addresses, named for a refusal that has to say why the
/// target is the wrong KIND rather than the wrong name (§ A.6.6's `remove`
/// fence).
fn sec_ref_plane(sec: &SecRef) -> &'static str {
    match sec {
        SecRef::Hpath { .. } => "section",
        SecRef::Anchor { .. } => "block anchor",
        SecRef::FmKey { .. } => "frontmatter key",
    }
}

/// The frontmatter key BOTH offending edits upsert, when that is what the
/// collision is — the model's rung-3a refusal (`validate_batch`, §4.4 target
/// grain), which reuses the `Overlap` verdict. `None` for an ordinary region
/// overlap, including two `match` edits on one key line: those really do
/// rewrite overlapping bytes and the general remedy fits them.
fn duplicate_fm_upsert_key<'a>(offending: &[usize], edits: &'a [Edit]) -> Option<&'a str> {
    let [a, b] = offending else { return None };
    let upsert_key = |i: &usize| match edits.get(*i) {
        Some(Edit {
            target: SecRef::FmKey { fm_key },
            edit: EditShape::Put {
                at: PutAt::Upsert, ..
            },
            ..
        }) => Some(fm_key.as_str()),
        _ => None,
    };
    let (x, y) = (upsert_key(a)?, upsert_key(b)?);
    (x == y).then_some(x)
}

/// The §4.4 overlap refusal, loud (§8): names the offending edits by batch
/// index and target, and teaches the fix. An index one past the caller's
/// edits is the engine-minted edit (receipt append).
fn overlap_refusal(offending: &[usize], edits: &[Edit]) -> ErrorBody {
    let mut e = ErrorBody::new(ErrorCode::BadRequest);
    let name = |i: &usize| {
        edits.get(*i).map_or_else(
            || "the engine-minted receipt append".to_string(),
            |edit| format!("edits[{i}] (target \"{}\")", target_display(&edit.target)),
        )
    };
    let names: Vec<String> = offending.iter().map(name).collect();
    e.message = Some(match duplicate_fm_upsert_key(offending, edits) {
        // Model rung 3a (§4.4 target grain): one key, one upsert. The general
        // remedy below is UNFOLLOWABLE here — a frontmatter key has exactly one
        // place in the block, so "re-anchor one" cannot be done and the caller
        // resends the same batch. This arm teaches its own fix instead.
        Some(key) => format!(
            "a batch must upsert each frontmatter key at most once (§4.4): {} both \
             upsert \"{key}\" — send the key once carrying the value you want, or \
             split them into separate splice calls",
            names.join(" and ")
        ),
        None => format!(
            "batch edits must rewrite disjoint bytes (§4.4): {} rewrite overlapping \
             regions of the file — re-anchor one so they touch different bytes, or \
             split them into separate splice calls",
            names.join(" and ")
        ),
    });
    let overlapping: Vec<SecRef> = offending
        .iter()
        .filter_map(|&i| edits.get(i))
        .map(|edit| edit.target.clone())
        .collect();
    if !overlapping.is_empty() {
        e.overlap = Some(overlapping);
    }
    e
}

/// The `would_corrupt` containment-lost body, lifted verbatim out of
/// [`verdict_to_wire`]'s arm so that function stays under the workspace line
/// policy without losing an arm. Pure code motion: same fields, same order.
fn containment_lost_refusal(lost: &[Vec<String>], cause: Option<model::CorruptCause>) -> ErrorBody {
    let mut e = ErrorBody::new(ErrorCode::WouldCorrupt);
    e.family = Some(WouldCorruptFamily::ContainmentLost);
    e.cause = cause.map(|c| {
        match c {
            model::CorruptCause::HeadingDestroyed => "heading_destroyed",
            model::CorruptCause::Reparented => "reparented",
        }
        .to_owned()
    });
    e.lost = Some(
        lost.iter()
            .map(|chain| {
                chain
                    .iter()
                    .map(|h| HpathSeg {
                        h: h.clone(),
                        n: None,
                    })
                    .collect()
            })
            .collect(),
    );
    e
}

/// The `transition_unrepresentable` body, lifted verbatim out of
/// [`verdict_to_wire`]'s arm. Pure code motion: same re-projection, same
/// message, same fields.
fn transition_unrepresentable_refusal(target: &model::Ref, edits: &[Edit]) -> ErrorBody {
    // Name the offender in the CALLER's own spelling: find the request
    // edit whose ref is the model ref the guard returned, the same
    // re-projection `ref_not_found` and `ambiguous` use.
    let sec = edits
        .iter()
        .map(|e| &e.target)
        .find(|t| to_model_ref(t).is_ok_and(|r| r == *target))
        .cloned()
        .unwrap_or_else(|| SecRef::Anchor {
            anchor: String::new(),
        });
    let mut e = ErrorBody::new(ErrorCode::WouldCorrupt);
    e.family = Some(WouldCorruptFamily::TransitionUnrepresentable);
    e.message = Some(format!(
        "this edit writes past \"{}\" — some of its bytes land outside that node's own \
                 span, so the node never receives them and its `node_rev` cannot move, leaving \
                 `if_node_rev` guarding a value this write can never change. {} Fix: a leaf's \
                 span EXCLUDES its line terminator, so its extent ends there — drop the \
                 trailing separator from your text, or aim the write at the enclosing section, \
                 whose span contains the bytes you meant to add.",
        target_display(&sec),
        crate::NO_PARTIAL_WRITE_CLAUSE
    ));
    e.target = Some(sec);
    e
}

/// The §5.2 failure split, mapped: every refusal verdict to its wire frame
/// (code + required recovery + the frozen extras). `edits` is the effective
/// batch (post-lowering) — the request targets the extras echo.
fn verdict_to_wire(
    verdict: &model::SpliceVerdict,
    edits: &[Edit],
    doc: &model::Document,
    path: &Path,
) -> Box<ErrorBody> {
    let e = match verdict {
        model::SpliceVerdict::Validated(_) => {
            unreachable!("validated batches are not refusals")
        }
        model::SpliceVerdict::RootMismatch { expected, actual } => {
            let mut e = ErrorBody::new(ErrorCode::RootMismatch);
            e.expected = Some(NodeRev(expected.0.clone()));
            e.actual = Some(NodeRev(actual.0.clone()));
            e
        }
        // Normally pre-empted by the per-target resolution in
        // `model_edits_and_before_facts`; routed through the shared helper so
        // the two miss sites cannot drift.
        model::SpliceVerdict::RefNotFound => {
            let offending = edits
                .iter()
                .map(|e| &e.target)
                .find(|t| {
                    to_model_ref(t).is_ok_and(|r| {
                        matches!(model::resolve(doc, &r), Err(model::ResolveError::NotFound))
                    })
                })
                .or_else(|| edits.first().map(|e| &e.target));
            match offending {
                Some(t) => crate::read::ref_not_found(t, doc, path.0.as_str()),
                None => ErrorBody::new(ErrorCode::RefNotFound),
            }
        }
        model::SpliceVerdict::Ambiguous(candidates) => {
            // Name each duplicate by node index + ^block via the shared helper.
            // Normally pre-empted by the per-target resolution in
            // `model_edits_and_before_facts`; routing through `ambiguous` keeps
            // both refusal sites identical.
            let offending = edits.iter().map(|e| &e.target).find(|t| {
                to_model_ref(t).is_ok_and(|r| {
                    matches!(
                        model::resolve(doc, &r),
                        Err(model::ResolveError::Ambiguous(_))
                    )
                })
            });
            if let Some(sec) = offending {
                ambiguous(sec, doc, candidates)
            } else {
                let mut e = ErrorBody::new(ErrorCode::AmbiguousRef);
                e.candidates = Some(Vec::new());
                e
            }
        }
        // The ladder attaches here, on the existing code with its bound
        // `Recovery::Refresh` — the refusal carries the richest computable rung
        // so the caller never has to re-read the file.
        model::SpliceVerdict::CasMismatch { expected, actual } => {
            let mut e = ErrorBody::new(ErrorCode::CasMismatch);
            e.expected = Some(NodeRev(expected.0.clone()));
            e.actual = Some(NodeRev(actual.0.clone()));
            crate::ladder::enrich(&mut e, doc, edits, path);
            e
        }
        model::SpliceVerdict::NoMatch { matches } => {
            let mut e = ErrorBody::new(ErrorCode::NoMatch);
            e.matches = Some(u32::try_from(*matches).unwrap_or(u32::MAX));
            e
        }
        model::SpliceVerdict::NotUnique { matches } => {
            let mut e = ErrorBody::new(ErrorCode::NotUnique);
            e.matches = Some(u32::try_from(*matches).unwrap_or(u32::MAX));
            e
        }
        model::SpliceVerdict::Overlap {
            edits: offending,
            spans: _,
        } => overlap_refusal(offending, edits),
        model::SpliceVerdict::WouldCorrupt { lost, cause } => {
            containment_lost_refusal(lost, *cause)
        }
        model::SpliceVerdict::TransitionUnrepresentable { target } => {
            transition_unrepresentable_refusal(target, edits)
        }
        model::SpliceVerdict::MultibyteSplit => {
            let mut e = ErrorBody::new(ErrorCode::BadRequest);
            e.message = Some("edit region splits a multi-byte character (§1)".into());
            e
        }
    };
    Box::new(e)
}

/// One commit's inputs: the model-side batch plus the envelope facts the engine
/// records but never invents (§9). `receipt` carries the receipt file's path and
/// the pre-rendered append, folded in before validation so it rides the sealed
/// batch and the single root advance (§6.1); its presence must pair with the
/// batch's — `fs` enforces the §6.5 seam contract fail-loud before any byte
/// lands.
#[derive(Debug, Clone)]
pub struct CommitRequest {
    pub content_path: String,
    pub batch: model::SpliceRequest,
    pub receipt: Option<(String, model::ReceiptAppend)>,
    pub actor: Option<String>,
    pub now: Option<String>,
    /// A pin's anchor promotion this splice already landed under the same
    /// flock — present exactly when it moved THIS workspace's world. The
    /// frame owes it a file row and its `root_before` (r8 D4: an untold mint
    /// is a write sub's history denies).
    pub promotion: Option<CommitPromotion>,
}

/// The promotion write as the commit frame must tell it: the splice knows it
/// made the write, so the frame carries the row and spans the call's whole
/// root movement. A cross-root promotion never builds one — it is the target
/// root's story, told on that root's own plane.
#[derive(Debug, Clone)]
pub struct CommitPromotion {
    /// The promotion target, relative to the committing workspace — the row's
    /// frame path.
    pub path: String,
    /// The target's pre-promotion parse — the row's before tense (and the
    /// content or receipt row's, when the target IS that file: one changed
    /// file is one row, §7.1).
    pub before: model::Document,
    /// The target's post-promotion parse — the row's after tense.
    pub after: model::Document,
    /// The world before the promotion landed — the frame's true `root_before`.
    /// The batch validates against the advanced root (its own write moved it);
    /// the CHAIN spans from before the first of this call's writes.
    pub root_before: Root,
}

/// A commit that did not emit: no byte reached disk, no Delta exists, the
/// ring did not advance. The choke-point maps each variant to its wire frame.
#[derive(Debug)]
pub enum CommitError {
    /// A typed validation refusal (§5.2 failure split) — the batch never
    /// reached `fs`.
    Refused(model::SpliceVerdict),
    /// Ambient-root/domain failure, already in the wire envelope shape.
    Env(Box<ErrorBody>),
    /// The atomic write failed, or the §6.5 seam contract refused
    /// (`InvalidInput` before any byte).
    Io(std::io::Error),
}

/// Commit one batch and return its Delta (§7.1: one Delta = one batch = one
/// root advance). The frame's `seq` comes from the [`crate::seq::SeqSink`],
/// allocated here — after `root_after` is folded and while the caller's write
/// flock is still held — so the number and the frame are decided in one act.
/// The caller advances its own ring with the returned frame (this seam holds no
/// ring, so the resident daemon can commit without one).
///
/// The write flock is the caller's (D9) and `flock` is the proof of it: this
/// seam acquires nothing, and a caller that has dropped the lock has no witness
/// to hand over. The workspace is taken from the flock rather than passed
/// beside it, so there is no second value to disagree with.
///
/// # Errors
/// [`CommitError`] — validation refusal, environment failure, or I/O; in
/// every error case nothing was emitted (a Delta exists only for a batch that
/// actually committed).
pub fn commit_batch(
    seq: Option<&dyn crate::seq::SeqSink>,
    flock: &fs::WriteLock,
    req: &CommitRequest,
    cache: &WriteCache,
) -> Result<DeltaFrame, CommitError> {
    // The workspace comes from the lock, and that is the guard: the daemon holds
    // many workspace roots at once, so a separately-passed root could name a
    // different workspace than the lock. Deriving it here leaves no second value
    // to disagree with.
    let root = flock.root();
    // Pre-state: the documents the batch validates against + the world root.
    let before_content = fs::load(root, FsPath::new(&req.content_path)).map_err(CommitError::Io)?;
    let before_receipt = match &req.receipt {
        Some((rp, _)) => load_optional(root, rp)?,
        None => None,
    };
    // Door-entry baseline still sitting in the cache — never a second live
    // observe (merkle-spec §6.1).
    let root_before = overlaid_root(cache).map_err(CommitError::Env)?;
    // Read#2 of the content file above is CAS, not a corpus observation.

    // Validate (§5.1 order) — mints the sealed batch, the only path to fs.
    let sealed = match model::validate_batch(
        &before_content,
        Some(&model::MerkleRoot(root_before.0.clone())),
        &req.batch,
        req.receipt.as_ref().map(|(_, append)| append.clone()),
    ) {
        model::SpliceVerdict::Validated(batch) => batch,
        refused => return Err(CommitError::Refused(refused)),
    };

    // Commit: the two-file atomic write (§6.5). fs enforces the pairing contract
    // fail-loud; a refusal here means no byte landed. The splice source is
    // read#2's validated bytes (`before_content.raw` — the bytes the sealed
    // spans index), and fs verifies the live file still carries them before any
    // rename (D8): drift refuses the typed write-conflict instead of
    // blind-splicing stale spans into moved bytes. The seam mints the candidate
    // for the bytes it is about to land, and `fs` refuses a candidate that is
    // not this batch's splice result.
    let candidate = model::candidate_of_batch(&req.content_path, &before_content.raw, &sealed);
    // The artifact guard at the commit seam: this is the public door, and a
    // caller reaching it directly has passed no translation at all.
    stored_form_guard_lazy(
        Some(&before_content),
        &candidate,
        &Path(req.content_path.clone()),
    )
    .map_err(CommitError::Env)?;
    fs::apply_batch(
        root,
        FsPath::new(&req.content_path),
        req.receipt.as_ref().map(|(rp, _)| FsPath::new(rp.as_str())),
        &sealed,
        before_content.raw.as_bytes(),
        &candidate,
    )
    .map_err(CommitError::Io)?;

    // Overlay from the engine's own bytes — never a post-apply reload.
    overlay_written(cache, &req.content_path, candidate.raw().as_bytes())
        .map_err(CommitError::Env)?;
    overlay_membership_from(cache, &req.content_path, candidate.raw()).map_err(CommitError::Env)?;
    let after_receipt = match &req.receipt {
        Some((rp, append)) => {
            let composed = compose_receipt(before_receipt.as_ref(), append);
            overlay_written(cache, rp, composed.as_bytes()).map_err(CommitError::Env)?;
            overlay_membership_from(cache, rp, &composed).map_err(CommitError::Env)?;
            Some(build_doc(&Path(rp.clone()), &composed))
        }
        None => None,
    };
    let root_after = overlaid_root(cache).map_err(CommitError::Env)?;

    // Change facts → wire projection, in §7.1 print order: content file first,
    // then the receipt file, then a promotion's own row.
    let after_receipt_doc = after_receipt.as_ref();
    let files = commit_delta_files(
        &req.content_path,
        &before_content,
        candidate.document(),
        req.receipt
            .as_ref()
            .map(|(rp, _)| (rp.as_str(), before_receipt.as_ref(), after_receipt_doc)),
        req.promotion.as_ref(),
    );

    // The chain tense: a promoting splice moved the world twice (marker, then
    // batch) — one call is one Delta (§7.1), so the frame spans from before
    // the FIRST of its writes. Validation above used the ambient root: the
    // batch's own guard was re-based onto it precisely because the promotion
    // was this splice's write.
    let root_before = req
        .promotion
        .as_ref()
        .map_or(root_before, |p| p.root_before.clone());

    // Assemble at the one production site and return the frame — the caller
    // advances its ring (or, on the resident daemon, discards it).
    let seq = crate::seq::allocate(seq, flock, &root_before, &root_after, &files);
    Ok(assemble_delta(
        seq,
        root_before,
        root_after,
        req.actor.clone(),
        req.now.clone(),
        files,
    ))
}

/// One commit's change facts → wire Delta file entries, §7.1 print order:
/// content file first, then the receipt file, then a pin promotion's own row.
/// Shared by [`commit_batch`] and the run plane's registry sink (§ A.8 Delta
/// honesty), so a run frame's file grain cannot drift from a splice frame's.
///
/// A promotion into the content or receipt file folds into that file's own
/// row as its before tense — one changed file is one row (§7.1), covering
/// both of this call's writes to it. Only a promotion into a third file rides
/// as its own entry.
#[must_use]
pub fn commit_delta_files(
    content_path: &str,
    before_content: &model::Document,
    after_content: &model::Document,
    receipt: Option<(&str, Option<&model::Document>, Option<&model::Document>)>,
    promotion: Option<&CommitPromotion>,
) -> Vec<DeltaFile> {
    let promoted_into = |path: &str| promotion.filter(|p| p.path == path);
    let mut files = Vec::new();
    let content_before = promoted_into(content_path).map_or(before_content, |p| &p.before);
    if let Some(fd) = model::delta::file_delta(Some(content_before), Some(after_content)) {
        files.push(wire_map::project_file_delta(content_path, &fd));
    }
    if let Some((rp, before, after)) = receipt
        && let Some(fd) =
            model::delta::file_delta(promoted_into(rp).map(|p| &p.before).or(before), after)
    {
        files.push(wire_map::project_file_delta(rp, &fd));
    }
    if let Some(p) = promotion
        && p.path != content_path
        && receipt.is_none_or(|(rp, _, _)| rp != p.path)
        && let Some(fd) = model::delta::file_delta(Some(&p.before), Some(&p.after))
    {
        files.push(wire_map::project_file_delta(&p.path, &fd));
    }
    files
}

/// The one production `DeltaFrame` construction site (§7.3 single-constructor
/// law): the commit path and the watcher's external path both assemble here.
/// `seq` is the final number, not a base to advance — each caller allocates
/// before it builds (the write path through its `SeqSink` under the flock, the
/// watcher through its own ring), so a second producer cannot double-count.
/// Envelope facts exactly as given (§9: external deltas pass `None`/`None` —
/// absent stays absent, never invented).
#[must_use]
pub fn assemble_delta(
    seq: u64,
    root_before: Root,
    root_after: Root,
    actor: Option<String>,
    now: Option<String>,
    files: Vec<DeltaFile>,
) -> DeltaFrame {
    DeltaFrame {
        delta: Delta {
            seq,
            root_before,
            root_after,
            actor,
            now,
            files,
        },
        effects: Vec::new(),
        rescope: None,
        overflow: None,
    }
}

/// A pre/post receipt-file read where absence is a legal state (the first
/// receipt append creates the file — `fs::read_or_empty` twin at the
/// document grain).
fn load_optional(
    root: &fs::WorkspaceRoot,
    rel: &str,
) -> Result<Option<model::Document>, CommitError> {
    match fs::load(root, FsPath::new(rel)) {
        Ok(doc) => Ok(Some(doc)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(CommitError::Io(e)),
    }
}

/// The write path's one production `policy::evaluate` call site: run every
/// admitted pack over the touched doc's post-batch state and project the §11.1
/// findings to `wire::Verdict`. `corpus` is `None` — the caller hands only
/// node/file-class packs (corpus-class is refused at admission). Dry and real
/// share this call over the same simulated after-doc, so their verdict sets are
/// byte-identical by construction. Empty `rulesets` ⇒ `[]`.
#[must_use]
pub fn evaluate_verdicts(
    rulesets: &[policy::CompiledRuleset],
    after_doc: &model::Document,
) -> Vec<Verdict> {
    let docs = std::slice::from_ref(&after_doc);
    rulesets
        .iter()
        .flat_map(|rs| policy::evaluate(rs, docs, None))
        .map(violation_to_verdict)
        .collect()
}

/// Project one `policy::Violation` into a `wire::Verdict` (§11.1): hpath strings
/// become `{h, n:None}` segments (§2.1), byte span → `[u64,u64]`.
/// `wire::Severity` is a distinct enum (no wire→policy edge).
fn violation_to_verdict(v: policy::Violation) -> Verdict {
    Verdict {
        rule: v.rule,
        severity: match v.severity {
            policy::Severity::Error => Severity::Error,
            policy::Severity::Warn => Severity::Warn,
            policy::Severity::Info => Severity::Info,
        },
        path: Path(v.path),
        hpath: v
            .hpath
            .map(|segs| segs.into_iter().map(|h| HpathSeg { h, n: None }).collect()),
        span: Span(v.span.start as u64, v.span.end as u64),
        node_rev: NodeRev(v.node_rev.0),
        message: v.message,
    }
}

#[cfg(test)]
mod tests {
    use wire::{ErrorCode, Path, Recovery};

    use super::commit_io_to_wire;

    /// D8: the fs write-conflict marker maps to the typed `write_conflict`
    /// frame (refresh — re-read, re-plan) carrying the request path; ordinary
    /// commit I/O failure keeps its `io_error{cause}` shape.
    #[test]
    fn commit_io_write_conflict_maps_to_typed_frame() {
        let page = Path("notes/plan.md".into());
        let conflict = commit_io_to_wire(
            &fs::write_conflict(std::path::Path::new("notes/plan.md")),
            &page,
        );
        assert_eq!(conflict.code, ErrorCode::WriteConflict);
        assert_eq!(
            conflict.recovery,
            Recovery::Refresh,
            "write_conflict → refresh"
        );
        assert_eq!(
            conflict.path.as_ref(),
            Some(&page),
            "echoes the drifted path"
        );
        assert!(
            conflict
                .message
                .as_deref()
                .is_some_and(|m| m.contains("write conflict")),
            "teaching message survives the map: {:?}",
            conflict.message
        );

        let plain = commit_io_to_wire(&std::io::Error::other("disk on fire"), &page);
        assert_eq!(plain.code, ErrorCode::IoError, "ordinary io keeps io_error");
        assert_eq!(plain.cause.as_deref(), Some("disk on fire"));
    }
}

/// Guarded `create`/`remove` — file birth and death. The named gates:
/// create-existing-path refuses (CAS), remove-after-drift refuses citing rev,
/// both emit the `before=absent`/`after=absent` change surface, and both
/// refusals map to their taxonomy rows (`cas_mismatch` + recovery `refresh`,
/// rows 13/14).
#[cfg(test)]
mod guarded_create_remove {
    use std::collections::BTreeMap;
    use wire::{
        Edit, EditShape, ErrorCode, FileChange, HpathSeg, NodeRev, Path, Recovery, ReferrerKind,
        SecRef,
    };

    // `ambient_root` stays the tests' ORACLE on purpose: an independent
    // full-corpus disk fold the production doors no longer run, so every
    // root assertion below cross-checks the resident overlay against the
    // law-1 fold of what actually landed.
    use crate::ambient_root;

    use super::{CreateArgs, RemoveArgs, SpliceArgs, create, remove, splice};

    /// A real on-disk workspace (create/remove land bytes and re-fold the root).
    fn ws() -> (tempfile::TempDir, fs::WorkspaceRoot) {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = fs::WorkspaceRoot(dir.path().to_path_buf());
        (dir, root)
    }

    fn create_args(path: &str, body: &str) -> CreateArgs {
        CreateArgs {
            id: None,
            path: Path(path.into()),
            body: body.into(),
            actor: Some("alice".into()),
            now: None,
            if_root: None,
            dry: false,
            fields: BTreeMap::default(),
            props: BTreeMap::default(),
        }
    }

    fn remove_args(path: &str, if_file_rev: &str) -> RemoveArgs {
        RemoveArgs {
            id: None,
            path: Path(path.into()),
            if_file_rev: Some(NodeRev(if_file_rev.into())),
            actor: Some("alice".into()),
            now: None,
            if_root: None,
            dry: false,
        }
    }

    /// Birth: `create` lands the file, advances the root, emits a `created`
    /// Delta (`file_rev_before` absent — the change surface's before=absent),
    #[test]
    fn create_births_file_and_advances_the_root() {
        let (dir, root) = ws();
        let out = create(&root, None, &create_args("notes/new.md", "# New\n"), &[])
            .expect("create births the file");

        // (a) the file is on disk with exactly the body bytes.
        assert_eq!(
            std::fs::read(dir.path().join("notes/new.md")).unwrap(),
            b"# New\n"
        );
        // (b) the birth advanced the world root (an in-domain md file).
        let root_after = out.root_after.expect("real create advances the root");
        assert_ne!(out.root_before, root_after, "birth advances the root");

        // (c) the change surface is a `created` file: before=absent.
        let file = &out
            .committed
            .expect("real create emits a Delta")
            .delta
            .files[0];
        assert_eq!(file.change, FileChange::Created);
        assert_eq!(file.file_rev_before, None, "created: before=absent");
        assert_eq!(file.file_rev_after.as_ref(), Some(&out.file_rev_after));
    }

    /// Create-existing-path refuses: a second `create` at an occupied path
    /// refuses `cas_mismatch` with recovery `refresh` (row 13), and the
    /// occupant's bytes are untouched.
    #[test]
    fn create_existing_path_refuses_cas() {
        let (dir, root) = ws();
        create(&root, None, &create_args("notes/new.md", "# First\n"), &[]).expect("first create");

        let err = create(&root, None, &create_args("notes/new.md", "# Second\n"), &[])
            .expect_err("create on an existing path must refuse");
        assert_eq!(
            err.code,
            ErrorCode::CasMismatch,
            "create-CAS → cas_mismatch"
        );
        assert_eq!(
            err.recovery,
            Recovery::Refresh,
            "taxonomy row 13: recovery refresh"
        );
        // The consumers' discriminator, on the frame the engine really mints:
        // `expected` is the empty document's rev, so this refusal — and only
        // this one — reads as occupancy.
        assert_eq!(
            err.expected.as_ref().map(|r| r.0.as_str()),
            Some(wire::ABSENT_REV),
            "the create-CAS names the ABSENT rev as `expected`"
        );
        assert!(
            err.is_path_occupied(),
            "the create door's own refusal must satisfy the occupancy discriminator"
        );

        assert_eq!(
            std::fs::read(dir.path().join("notes/new.md")).unwrap(),
            b"# First\n",
            "the occupant is untouched — the birth refused before any byte"
        );
    }

    /// The published [`wire::ABSENT_REV`] and the engine's computed
    /// [`super::absent_rev`] are the same token. The constant is what the Go daemon
    /// and every out-of-process consumer compare against; the computation is
    /// what the create door actually mints. A domain-rule change that moves the
    /// empty document's rev fails HERE, loudly, instead of leaving the
    /// published constant quietly lying to its readers.
    #[test]
    fn the_published_absent_rev_is_the_computed_one() {
        assert_eq!(
            super::absent_rev().0,
            wire::ABSENT_REV,
            "wire::ABSENT_REV has drifted from model::build(\"\")'s root rev — \
             update the constant AND every mirror of it (the Go daemon's \
             wirev3.AbsentRev), because callers key occupancy on it"
        );
    }

    /// Death: `remove` (with the read rev) deletes the file, advances the root,
    /// emits a `deleted` Delta (`file_rev_after` absent — after=absent).
    #[test]
    fn remove_death_emits_after_absent() {
        let (dir, root) = ws();
        let born = create(&root, None, &create_args("notes/new.md", "# New\n"), &[]).unwrap();

        let out = remove(
            &root,
            None,
            &remove_args("notes/new.md", &born.file_rev_after.0),
            &[],
        )
        .expect("remove-what-you-read succeeds when the rev still matches");

        assert!(
            !dir.path().join("notes/new.md").exists(),
            "the file is gone from disk"
        );
        let file = &out
            .committed
            .expect("real remove emits a Delta")
            .delta
            .files[0];
        assert_eq!(file.change, FileChange::Deleted);
        assert_eq!(file.file_rev_after, None, "deleted: after=absent");
        assert_eq!(file.file_rev_before.as_ref(), Some(&out.file_rev_before));
    }

    /// Remove-after-drift refuses citing rev: after the file drifts from the
    /// read rev, `remove` refuses `cas_mismatch` (recovery `refresh`, row 14)
    /// and names the rev read (`expected`) vs found (`actual`).
    #[test]
    fn remove_after_drift_refuses_citing_rev() {
        let (dir, root) = ws();
        let born = create(&root, None, &create_args("notes/new.md", "# New\n"), &[]).unwrap();
        let read_rev = born.file_rev_after.clone();

        // The file drifts under the plan (a later edit / foreign write).
        std::fs::write(dir.path().join("notes/new.md"), "# Drifted\n").unwrap();
        let live_rev = super::occupant_rev(&root, &Path("notes/new.md".into()))
            .unwrap()
            .unwrap();
        assert_ne!(read_rev, live_rev, "the fixture actually drifted");

        let err = remove(&root, None, &remove_args("notes/new.md", &read_rev.0), &[])
            .expect_err("remove after drift must refuse");
        assert_eq!(
            err.code,
            ErrorCode::CasMismatch,
            "remove-CAS → cas_mismatch"
        );
        assert_eq!(
            err.recovery,
            Recovery::Refresh,
            "taxonomy row 14: recovery refresh"
        );
        assert_eq!(err.expected.as_ref(), Some(&read_rev), "names the rev READ");
        assert_eq!(err.actual.as_ref(), Some(&live_rev), "names the rev FOUND");
        assert_ne!(
            err.expected, err.actual,
            "the refusal cites both revs, and they differ"
        );
        // THE NEGATIVE: same code as the create-CAS, opposite meaning. A
        // consumer that keys on `code` alone reads this drift refusal — "the
        // file moved under your plan" — as a benign already-exists and reports
        // a birth that never happened. The `expected` field is what separates
        // them.
        assert!(
            !err.is_path_occupied(),
            "a remove-CAS drift refusal must NOT read as occupancy: {err:?}"
        );

        // The drift refusal wrote nothing: the file still carries the drifted
        // bytes, untouched by the refused remove.
        assert_eq!(
            std::fs::read(dir.path().join("notes/new.md")).unwrap(),
            b"# Drifted\n",
            "a refused remove leaves the file byte-untouched"
        );
    }

    /// A `remove` of a path that is not there is `file_not_found` (env) — you
    /// cannot remove nothing.
    #[test]
    fn remove_absent_is_file_not_found() {
        let (_dir, root) = ws();
        let err = remove(
            &root,
            None,
            &remove_args("notes/ghost.md", "deadbeefdeadbeef"),
            &[],
        )
        .expect_err("removing an absent file refuses");
        assert_eq!(err.code, ErrorCode::FileNotFound);
    }

    /// § A.3 referential guard, wikilink arm: a record with inbound wikilinks
    /// refuses `remove_refused` (fix) naming the referring file, the kind,
    /// and the exact edge count — and nothing is removed.
    #[test]
    fn remove_refuses_while_a_wikilink_references_the_record() {
        let (dir, root) = ws();
        let born = create(
            &root,
            None,
            &create_args("notes/victim.md", "# Victim\n"),
            &[],
        )
        .unwrap();
        create(
            &root,
            None,
            &create_args(
                "notes/fan.md",
                "# Fan\n\nsee [[victim]], and [[victim]] again\n",
            ),
            &[],
        )
        .unwrap();

        let err = remove(
            &root,
            None,
            &remove_args("notes/victim.md", &born.file_rev_after.0),
            &[],
        )
        .expect_err("a referenced record must not die");
        assert_eq!(err.code, ErrorCode::RemoveRefused);
        assert_eq!(err.recovery, Recovery::Fix, "remove_refused → fix");
        let referrers = err
            .referrers
            .as_deref()
            .expect("the refusal names the referrers");
        assert_eq!(referrers.len(), 1, "one referring file");
        assert_eq!(referrers[0].path, "notes/fan.md");
        assert_eq!(referrers[0].kind, ReferrerKind::Wikilink);
        assert_eq!(referrers[0].count, 2, "every edge counted");
        assert!(
            dir.path().join("notes/victim.md").exists(),
            "a refused remove leaves the record on disk"
        );
    }

    /// § A.3 referential guard, embed arm: `![[victim]]` blocks with its own
    /// kind — the caller's unlink worklist distinguishes an embed from a link.
    #[test]
    fn remove_refuses_an_inbound_embed_naming_the_kind() {
        let (_dir, root) = ws();
        let born = create(
            &root,
            None,
            &create_args("notes/victim.md", "# Victim\n"),
            &[],
        )
        .unwrap();
        create(
            &root,
            None,
            &create_args("notes/gallery.md", "# Gallery\n\n![[victim]]\n"),
            &[],
        )
        .unwrap();

        let err = remove(
            &root,
            None,
            &remove_args("notes/victim.md", &born.file_rev_after.0),
            &[],
        )
        .expect_err("an embedded record must not die");
        assert_eq!(err.code, ErrorCode::RemoveRefused);
        let referrers = err.referrers.as_deref().unwrap();
        assert_eq!(
            (referrers[0].path.as_str(), referrers[0].kind),
            ("notes/gallery.md", ReferrerKind::Embed)
        );
    }

    /// § A.3 referential guard, pin arm: an ambient `meridian-lock` pin on the
    /// record blocks with kind `pin` — the walk plane's Down predicate applied
    /// at the door. The pinning page is a raw fixture write: the birth door
    /// refuses lock-bearing bodies, and the guard reads DISK, so a hand-landed
    /// pin must count exactly like an engine-minted one.
    #[test]
    fn remove_refuses_while_a_lock_pin_references_the_record() {
        let (dir, root) = ws();
        let born = create(
            &root,
            None,
            &create_args("notes/victim.md", "# Victim\n"),
            &[],
        )
        .unwrap();

        // A live token of the victim's own content — the walk fixtures' shape
        // (`live_token`), so the block parses exactly as an engine-minted pin.
        let victim = model::build("# Victim\n".to_string(), syntax::parse("# Victim\n"));
        let token = model::fingerprint::fingerprint(&victim, &victim.root)
            .expect("the fixture page has content")
            .into_string();
        let mut l = lock::Lock::new();
        l.upsert_pin(lock::PinEntry::new(
            "victim",
            "9ae3f1deadbeef",
            lock::Selector::Path(Vec::new()),
            &token,
        ));
        std::fs::write(
            dir.path().join("notes/pinner.md"),
            format!("# Pinner\n\n{}\n", lock::render(&l)),
        )
        .unwrap();

        let err = remove(
            &root,
            None,
            &remove_args("notes/victim.md", &born.file_rev_after.0),
            &[],
        )
        .expect_err("a pinned record must not die");
        assert_eq!(err.code, ErrorCode::RemoveRefused);
        let referrers = err.referrers.as_deref().unwrap();
        assert_eq!(
            (
                referrers[0].path.as_str(),
                referrers[0].kind,
                referrers[0].count
            ),
            ("notes/pinner.md", ReferrerKind::Pin, 1)
        );
    }

    /// § A.3 message figures: one file referring by TWO kinds (wikilink and
    /// pin) spans two referrer rows — the message's edge figure sums every
    /// edge, and its file figure counts DISTINCT referring files, not rows.
    #[test]
    fn remove_refusal_counts_distinct_files_not_referrer_rows() {
        let (dir, root) = ws();
        let born = create(
            &root,
            None,
            &create_args("notes/victim.md", "# Victim\n"),
            &[],
        )
        .unwrap();

        // One referring file holding all three edges: two wikilinks plus an
        // ambient pin (raw fixture write — the birth door refuses lock-bearing
        // bodies, and the guard reads DISK).
        let victim = model::build("# Victim\n".to_string(), syntax::parse("# Victim\n"));
        let token = model::fingerprint::fingerprint(&victim, &victim.root)
            .expect("the fixture page has content")
            .into_string();
        let mut l = lock::Lock::new();
        l.upsert_pin(lock::PinEntry::new(
            "victim",
            "9ae3f1deadbeef",
            lock::Selector::Path(Vec::new()),
            &token,
        ));
        std::fs::write(
            dir.path().join("notes/fan.md"),
            format!(
                "# Fan\n\nsee [[victim]], and [[victim]] again\n\n{}\n",
                lock::render(&l)
            ),
        )
        .unwrap();

        let err = remove(
            &root,
            None,
            &remove_args("notes/victim.md", &born.file_rev_after.0),
            &[],
        )
        .expect_err("a referenced record must not die");
        assert_eq!(err.code, ErrorCode::RemoveRefused);
        let referrers = err.referrers.as_deref().unwrap();
        assert_eq!(referrers.len(), 2, "two rows: one per (path, kind)");
        assert!(
            referrers.iter().all(|r| r.path == "notes/fan.md"),
            "both rows name the one referring file"
        );
        assert!(
            err.message
                .as_deref()
                .is_some_and(|m| m.contains("3 inbound references from 1 file —")),
            "edges sum, files dedup distinct paths: {:?}",
            err.message
        );
    }

    /// § A.3 self-edge exclusion: a record's own links to itself do not hold
    /// it alive — the death lands and the Delta rides.
    #[test]
    fn remove_ignores_the_records_own_self_references() {
        let (dir, root) = ws();
        let born = create(
            &root,
            None,
            &create_args("notes/victim.md", "# Victim\n\nme: [[victim]]\n"),
            &[],
        )
        .unwrap();

        remove(
            &root,
            None,
            &remove_args("notes/victim.md", &born.file_rev_after.0),
            &[],
        )
        .expect("a self-referencing record still dies");
        assert!(!dir.path().join("notes/victim.md").exists());
    }

    /// § A.3: `if_file_rev` is a precondition of the op — a rev-less remove
    /// refuses `guard_required` (fix) from EVERY origin, teaching the slot,
    /// and touches nothing. There is no force alternative.
    #[test]
    fn remove_without_the_read_rev_refuses_guard_required() {
        let (dir, root) = ws();
        create(
            &root,
            None,
            &create_args("notes/victim.md", "# Victim\n"),
            &[],
        )
        .unwrap();

        let err = remove(
            &root,
            None,
            &RemoveArgs {
                if_file_rev: None,
                ..remove_args("notes/victim.md", "unused")
            },
            &[],
        )
        .expect_err("a rev-less remove refuses");
        assert_eq!(err.code, ErrorCode::GuardRequired);
        assert_eq!(err.recovery, Recovery::Fix);
        assert!(
            err.message
                .as_deref()
                .is_some_and(|m| m.contains("if_file_rev")),
            "the refusal teaches the slot: {:?}",
            err.message
        );
        assert!(dir.path().join("notes/victim.md").exists());
    }

    /// §5.1 world guard on remove: a supplied stale `if_root` refuses
    /// `root_mismatch` (resync) before anything dies — honored when present,
    /// never demanded (§ A.3).
    #[test]
    fn remove_with_a_stale_world_guard_refuses_root_mismatch() {
        let (dir, root) = ws();
        let born = create(
            &root,
            None,
            &create_args("notes/victim.md", "# Victim\n"),
            &[],
        )
        .unwrap();
        let stale = ambient_root(&root).unwrap();
        // The world moves under the plan.
        create(
            &root,
            None,
            &create_args("notes/other.md", "# Other\n"),
            &[],
        )
        .unwrap();

        let err = remove(
            &root,
            None,
            &RemoveArgs {
                if_root: Some(stale),
                ..remove_args("notes/victim.md", &born.file_rev_after.0)
            },
            &[],
        )
        .expect_err("a stale world guard refuses");
        assert_eq!(err.code, ErrorCode::RootMismatch);
        assert_eq!(err.recovery, Recovery::Resync);
        assert!(dir.path().join("notes/victim.md").exists());
    }

    /// § A.3, receipted not argued: the referential check and the unlink share
    /// the write flock. Phase 1 — while a cooperating writer holds the flock,
    /// the door refuses `workspace_busy` fast and touches nothing: no writer
    /// can interleave with a remove already inside its critical section,
    /// because the same lock excludes both directions. Phase 2 — a link that
    /// lands after the caller's read is SEEN by the check: the corpus is
    /// snapshotted after acquisition, never taken from the caller's picture,
    /// so the check-to-unlink window contains no gap a fresh write can slip
    /// through.
    #[test]
    fn remove_check_and_unlink_share_the_write_flock() {
        let (dir, root) = ws();
        let born = create(
            &root,
            None,
            &create_args("notes/victim.md", "# Victim\n"),
            &[],
        )
        .unwrap();

        // Phase 1: the flock is held elsewhere — the door refuses fast,
        // before reading or unlinking anything.
        {
            let _held = super::acquire_write_lock(&root).expect("the test takes the flock");
            let err = remove(
                &root,
                None,
                &remove_args("notes/victim.md", &born.file_rev_after.0),
                &[],
            )
            .expect_err("the door must not act while another writer holds the flock");
            assert_eq!(err.code, ErrorCode::WorkspaceBusy);
            assert!(dir.path().join("notes/victim.md").exists());
        }

        // Phase 2: a referring file lands AFTER the caller's read (any writer
        // that reached the lock first); the caller's rev is still fresh, yet
        // the in-flock check sees the new link and refuses.
        std::fs::write(
            dir.path().join("notes/late.md"),
            "# Late\n\nsee [[victim]]\n",
        )
        .unwrap();
        let err = remove(
            &root,
            None,
            &remove_args("notes/victim.md", &born.file_rev_after.0),
            &[],
        )
        .expect_err("the check reads the world as of the flock, not the caller's read");
        assert_eq!(err.code, ErrorCode::RemoveRefused);
        assert_eq!(
            err.referrers.as_deref().map(|r| r[0].path.as_str()),
            Some("notes/late.md"),
            "the late link is exactly what the refusal names"
        );

        // Unlink the referrer; the same call now lands — the door's answer
        // tracks the corpus, not the request's history.
        std::fs::remove_file(dir.path().join("notes/late.md")).unwrap();
        remove(
            &root,
            None,
            &remove_args("notes/victim.md", &born.file_rev_after.0),
            &[],
        )
        .expect("referentially empty again — the death lands");
        assert!(!dir.path().join("notes/victim.md").exists());
    }

    /// Workspace-root confinement: a `..`-escape or an absolute path refuses
    /// `bad_path` for both create and remove.
    #[test]
    fn guarded_ops_confined_to_workspace_root() {
        let (_dir, root) = ws();
        for bad in ["../outside.md", "/etc/passwd", "notes/../../escape.md"] {
            assert_eq!(
                create(&root, None, &create_args(bad, "x"), &[])
                    .unwrap_err()
                    .code,
                ErrorCode::BadPath,
                "create confined: {bad}"
            );
            assert_eq!(
                remove(&root, None, &remove_args(bad, "deadbeefdeadbeef"), &[])
                    .unwrap_err()
                    .code,
                ErrorCode::BadPath,
                "remove confined: {bad}"
            );
        }
    }

    /// Dry runs touch no disk (§4.4 batch law, applied to birth/death): a dry
    /// create writes no file; a dry remove leaves the file. Both still run the
    /// gate seam (empty ⇒ `[]`).
    #[test]
    fn dry_create_and_remove_touch_no_disk() {
        let (dir, root) = ws();

        let dry_born = create(
            &root,
            None,
            &CreateArgs {
                dry: true,
                ..create_args("notes/new.md", "# New\n")
            },
            &[],
        )
        .expect("dry create reports without landing");
        assert!(
            !dir.path().join("notes/new.md").exists(),
            "dry create writes no file"
        );
        assert!(dry_born.root_after.is_none() && dry_born.committed.is_none());
        assert!(dry_born.verdicts.is_empty());

        // A real file to dry-remove.
        let born = create(&root, None, &create_args("notes/new.md", "# New\n"), &[]).unwrap();
        let dry_dead = remove(
            &root,
            None,
            &RemoveArgs {
                dry: true,
                ..remove_args("notes/new.md", &born.file_rev_after.0)
            },
            &[],
        )
        .expect("dry remove reports without landing");
        assert!(
            dir.path().join("notes/new.md").exists(),
            "dry remove leaves the file"
        );
        assert!(dry_dead.committed.is_none());
    }

    /// A splice against a page under `root`, editing the `Alpha/Beta` section.
    fn splice_args(path: &str, old: &str, new: &str) -> SpliceArgs {
        SpliceArgs {
            premises: Vec::new(),
            id: None,
            origin: crate::guard::Origin::InProcess,
            path: Path(path.into()),
            actor: Some("alice".into()),
            now: None,
            receipt: None,
            if_root: None,
            dry: false,
            force: false,
            edits: vec![Edit {
                target: SecRef::Hpath {
                    hpath: vec![
                        HpathSeg {
                            h: "Alpha".into(),
                            n: None,
                        },
                        HpathSeg {
                            h: "Beta".into(),
                            n: None,
                        },
                    ],
                },
                edit: EditShape::Match {
                    old: old.into(),
                    new: new.into(),
                },
                if_node_rev: None,
            }],
            plan_edits: Vec::new(),
            pin: None,
            fields: BTreeMap::default(),
        }
    }

    /// §4.4 region grain: an append at the child's span-end plus an append at
    /// the parent's span-end (the `create` lowering's shape) commit as ONE
    /// batch — one Delta, one root advance — the F2 mixed-batch fix.
    #[test]
    fn nested_target_appends_commit_as_one_batch() {
        let (dir, root) = ws();
        create(
            &root,
            None,
            &create_args("notes/plan.md", "# Alpha\n\n## Beta\n\nw0\n"),
            &[],
        )
        .expect("birth");

        let mut args = splice_args("notes/plan.md", "", "");
        args.edits = vec![
            Edit {
                target: SecRef::Hpath {
                    hpath: vec![
                        HpathSeg {
                            h: "Alpha".into(),
                            n: None,
                        },
                        HpathSeg {
                            h: "Beta".into(),
                            n: None,
                        },
                    ],
                },
                edit: EditShape::Put {
                    at: wire::PutAt::End,
                    text: "appended\n".into(),
                },
                if_node_rev: None,
            },
            Edit {
                target: SecRef::Hpath {
                    hpath: vec![HpathSeg {
                        h: "Alpha".into(),
                        n: None,
                    }],
                },
                edit: EditShape::Put {
                    at: wire::PutAt::End,
                    text: "\n## Gamma\n\nborn\n".into(),
                },
                if_node_rev: None,
            },
        ];
        let out = splice(&root, None, &args, &[], None)
            .unwrap_or_else(|e| panic!("mixed nested-target batch refused: {:?}", e.message));
        out.committed.expect("one commit, one Delta");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("notes/plan.md")).unwrap(),
            "# Alpha\n\n## Beta\n\nw0\nappended\n\n## Gamma\n\nborn\n",
            "both zero-width inserts landed in one atomic batch"
        );
    }

    /// §4.4/§8: the overlap refusal names the offending edits by batch index
    /// and target, and teaches the fix — never a bare `bad_request`.
    #[test]
    fn overlap_refusal_names_the_offending_edits() {
        let (dir, root) = ws();
        create(
            &root,
            None,
            &create_args("notes/plan.md", "# Alpha\n\n## Beta\n\nship by August\n"),
            &[],
        )
        .expect("birth");

        let mut args = splice_args("notes/plan.md", "ship by August", "a");
        args.edits.push(Edit {
            target: SecRef::Hpath {
                hpath: vec![
                    HpathSeg {
                        h: "Alpha".into(),
                        n: None,
                    },
                    HpathSeg {
                        h: "Beta".into(),
                        n: None,
                    },
                ],
            },
            edit: EditShape::Match {
                old: "August".into(),
                new: "b".into(),
            },
            if_node_rev: None,
        });
        let err = splice(&root, None, &args, &[], None)
            .expect_err("overlapping matched regions must refuse");
        assert_eq!(err.code, ErrorCode::BadRequest);
        assert_eq!(
            err.message.as_deref(),
            Some(
                "batch edits must rewrite disjoint bytes (§4.4): edits[0] (target \
                 \"Alpha/Beta\") and edits[1] (target \"Alpha/Beta\") rewrite overlapping \
                 regions of the file — re-anchor one so they touch different bytes, or \
                 split them into separate splice calls"
            )
        );
        assert_eq!(
            err.overlap.as_ref().map(Vec::len),
            Some(2),
            "the overlap extra echoes both offending targets"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("notes/plan.md")).unwrap(),
            "# Alpha\n\n## Beta\n\nship by August\n",
            "refused whole — no byte landed"
        );
    }

    /// PUT LANE, model rung 3a (reviewer `36637e1a` on PR 214, finding 1):
    /// two upserts of ONE frontmatter key in one splice. The ABSENT-key arm
    /// used to land the key TWICE — both edits plan the same zero-width insert
    /// at the block's first-key offset, which the region grain reads disjoint.
    /// It now refuses, byte-clean, with a remedy a caller can actually follow:
    /// "re-anchor one" is impossible for a key that has one place.
    #[test]
    fn same_fm_key_upserted_twice_refuses_on_the_put_lane() {
        let (dir, root) = ws();
        create(
            &root,
            None,
            &create_args("notes/plan.md", "---\ntitle: c\n---\n# Alpha\n"),
            &[],
        )
        .expect("birth");
        let before = std::fs::read_to_string(dir.path().join("notes/plan.md")).unwrap();

        for (key, arm) in [
            ("nope", "absent key — two zero-width inserts at one point"),
            ("title", "existing key — two identical replace regions"),
        ] {
            let mut args = splice_args("notes/plan.md", "unused", "unused");
            args.edits = ["one", "two"]
                .into_iter()
                .map(|value| Edit {
                    target: SecRef::FmKey {
                        fm_key: key.to_string(),
                    },
                    edit: EditShape::Put {
                        at: wire::PutAt::Upsert,
                        text: value.into(),
                    },
                    if_node_rev: None,
                })
                .collect();
            let err = splice(&root, None, &args, &[], None)
                .expect_err("two upserts of one key must refuse");
            assert_eq!(err.code, ErrorCode::BadRequest, "{arm}");
            let want = format!(
                "a batch must upsert each frontmatter key at most once (§4.4): \
                 edits[0] (target \"{key}\") and edits[1] (target \"{key}\") both \
                 upsert \"{key}\" — send the key once carrying the value you want, \
                 or split them into separate splice calls"
            );
            assert_eq!(
                err.message.as_deref(),
                Some(want.as_str()),
                "{arm}: the remedy must be followable"
            );
            assert_eq!(
                std::fs::read_to_string(dir.path().join("notes/plan.md")).unwrap(),
                before,
                "{arm}: refused whole — no byte landed"
            );
        }
    }

    /// The write door's own gate: every splice moves the ambient root, and no
    /// splice leaves it where it found it.
    #[test]
    fn a_run_of_splices_advances_the_root_every_time() {
        let (_dir, root) = ws();
        create(
            &root,
            None,
            &create_args("notes/plan.md", "# Alpha\n\n## Beta\n\nw0\n"),
            &[],
        )
        .expect("birth");

        let mut seen = vec![ambient_root(&root).expect("live root")];
        for step in 1..=5 {
            let out = splice(
                &root,
                None,
                &splice_args(
                    "notes/plan.md",
                    &format!("w{}", step - 1),
                    &format!("w{step}"),
                ),
                &[],
                None,
            )
            .unwrap_or_else(|e| panic!("splice {step} refused: {e:?}"));

            let frame = out.committed.expect("a real splice commits a Delta");
            assert_eq!(
                frame.delta.root_before,
                *seen.last().expect("prior root"),
                "splice {step} guards on the root the previous write left"
            );
            assert_ne!(
                frame.delta.root_after, frame.delta.root_before,
                "splice {step} moved the tree root"
            );
            seen.push(frame.delta.root_after.clone());
        }

        assert_eq!(
            *seen.last().expect("roots"),
            ambient_root(&root).expect("live root"),
            "the last splice's root_after IS the live tree"
        );
        let unique: std::collections::BTreeSet<&str> = seen.iter().map(|r| r.0.as_str()).collect();
        assert_eq!(unique.len(), seen.len(), "every step is a distinct root");
    }

    // -----------------------------------------------------------------------
    // The guard, driven at its own level
    // -----------------------------------------------------------------------
    //
    // These call `stored_form_guard` directly with a hand-built `MountSet`: no
    // config on disk, no `machine_mounts()`, and no translation anywhere in the
    // path. The guard exists for a door that reaches bytes without passing the
    // translation, so proving it through a write whose transform refuses first
    // would prove nothing about the case it was built for.

    /// A table declaring `notes` at a path this machine cannot read, and binding
    /// `sessions` — the ordinary laptop, both arms in one table.
    fn f4_mounts() -> addr::MountSet {
        let sessions = addr::MountName::parse("sessions").expect("a canonical name");
        let notes = addr::MountName::parse("notes").expect("a canonical name");
        addr::MountSet::new([sessions.clone()])
            .with_vault(sessions, "field-notes-sessions")
            .with_unreachable(
                notes,
                "/nonexistent/notes-root",
                "No such file or directory",
            )
    }

    fn f4_guard(raw: &str) -> Result<(), Box<crate::ErrorBody>> {
        let candidate = model::candidate_of_body("page.md", raw.to_string());
        super::stored_form_guard(None, &candidate, &Path("page.md".into()), &f4_mounts())
    }

    /// The guard refuses an agent-plane address on a declared-but-unbound root,
    /// with no transform anywhere in the call.
    #[test]
    fn the_guard_sees_a_declared_but_unbound_root_with_no_transform_in_the_path() {
        for raw in [
            "# Page\n\nsee [x](notes:a.md)\n",
            "# Page\n\nsee [[notes:a.md]]\n",
        ] {
            let err = f4_guard(raw).expect_err(
                "the guard must refuse an agent-plane address on a declared-but-unbound root",
            );
            assert_eq!(err.code, ErrorCode::BadRequest);
            assert!(
                err.message
                    .as_deref()
                    .is_some_and(|m| m.contains("notes:a.md")),
                "and it names the offending address: {:?}",
                err.message,
            );
        }
    }

    /// The control that keeps the gate above from being satisfied by a guard
    /// that refuses everything. An external URI parses as a rooted address
    /// (`https://example.com` has root `https`), but nothing declares `https`,
    /// so it never reaches the guard's population.
    #[test]
    fn the_guard_leaves_undeclared_schemes_alone() {
        f4_guard(
            "# Page\n\n[ext](https://example.com) and [m](mailto:a@b.example)\n\
             and [rel](./sibling.md) and [[ambient.md]]\n",
        )
        .expect("an ordinary corpus carries no agent-plane occupant and must pass the guard");
    }

    /// A bound root's agent-plane spelling still trips the guard: reaching the
    /// guard at all means the translation was bypassed.
    #[test]
    fn the_guard_still_refuses_a_bound_roots_agent_plane_spelling() {
        let err = f4_guard("# Page\n\nsee [x](sessions:notes.md)\n")
            .expect_err("an untranslated bound-root address must still refuse");
        assert_eq!(err.code, ErrorCode::BadRequest);
    }

    /// A stored form that already translated passes — the acceptance half of the
    /// guard's own contract.
    #[test]
    fn the_guard_passes_bytes_that_already_carry_the_stored_form() {
        f4_guard(
            "# Page\n\n[sessions:notes.md](obsidian://advanced-uri\
             ?vault=field-notes-sessions&filepath=notes.md)\n",
        )
        .expect("translated bytes are what the guard exists to let through");
    }
}

/// The resident write path (merged plan §6 step 3, card write-path-overlay):
/// the guarded doors ride the resident tree through the wrapper seam,
/// `root_after` comes from the commit's own overlay, and every served token
/// stays an old-law value. `crate::ambient_root` is the tests' independent
/// oracle — a fresh full-corpus law-1 disk fold the doors no longer run.
#[cfg(test)]
mod resident_write_path {
    use std::collections::BTreeMap;
    use wire::{Edit, EditShape, ErrorCode, HpathSeg, NodeRev, Path, Root, SecRef};

    use crate::ambient_root;

    use super::{
        CreateArgs, LockWriteArgs, PoisonError, RemoveArgs, ResidentDoor, SpliceArgs,
        SpliceSetArgs, create, lock_write, remove, splice, splice_set, write_cache,
    };

    fn ws() -> (tempfile::TempDir, fs::WorkspaceRoot) {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = fs::WorkspaceRoot(dir.path().to_path_buf());
        (dir, root)
    }

    /// One in-domain page under `Alpha/Beta` whose body a Match edit can move.
    fn page_body(word: &str) -> String {
        format!("# Alpha\n\n## Beta\n\nship by {word}\n")
    }

    fn page(dir: &tempfile::TempDir, rel: &str, word: &str) {
        let abs = dir.path().join(rel);
        std::fs::create_dir_all(abs.parent().expect("parent")).expect("mkdir");
        std::fs::write(abs, page_body(word)).expect("write");
    }

    fn fold_owned(version: u32, files: &[(&str, Vec<u8>)]) -> Root {
        let leaves: Vec<(&[u8], [u8; 32])> = files
            .iter()
            .map(|(n, b)| (n.as_bytes(), model::leaf_digest(b)))
            .collect();
        Root(fs::served_root(&leaves, version).0)
    }

    fn race_foreign(dir: &std::path::Path) {
        let path = dir.join("notes/foreign.md");
        super::after_door_observe(move || {
            std::fs::write(&path, page_body("RACED")).expect("foreign racer");
        });
    }

    fn match_edit(old: &str, new: &str) -> Edit {
        Edit {
            target: SecRef::Hpath {
                hpath: vec![
                    HpathSeg {
                        h: "Alpha".into(),
                        n: None,
                    },
                    HpathSeg {
                        h: "Beta".into(),
                        n: None,
                    },
                ],
            },
            edit: EditShape::Match {
                old: old.into(),
                new: new.into(),
            },
            if_node_rev: None,
        }
    }

    fn splice_args(path: &str, old: &str, new: &str) -> SpliceArgs {
        SpliceArgs {
            premises: Vec::new(),
            id: None,
            origin: crate::guard::Origin::InProcess,
            path: Path(path.into()),
            actor: Some("alice".into()),
            now: None,
            receipt: None,
            if_root: None,
            dry: false,
            force: false,
            edits: vec![match_edit(old, new)],
            plan_edits: Vec::new(),
            pin: None,
            fields: BTreeMap::default(),
        }
    }

    /// The card's instrumented gate at member-read grain, on the cache's own
    /// monotonic counters: a warm guarded splice reads exactly the ONE member
    /// the previous write spoiled (§6.2 — reads follow change, never corpus
    /// size), and folds exactly once (lane C, only because the root advanced).
    #[test]
    fn warm_guarded_write_reads_one_spoiled_member_and_folds_once() {
        let (dir, root) = ws();
        for i in 0..8 {
            page(&dir, &format!("notes/bystander{i}.md"), "nothing");
        }
        page(&dir, "notes/plan.md", "August");
        page(&dir, "notes/other.md", "August");

        // Cold seed: the first door call observes the whole corpus once and
        // spoils exactly the member it wrote (notes/plan.md).
        let first = splice(
            &root,
            None,
            &splice_args("notes/plan.md", "August", "w1"),
            &[],
            None,
        )
        .expect("cold splice");

        let cache = write_cache(&root);
        let (reads_before, folds_before) = {
            let c = cache.lock().unwrap();
            (c.leaves_read(), c.served_folds())
        };

        // Warm guarded write on a DIFFERENT member.
        let out = splice(
            &root,
            None,
            &splice_args("notes/other.md", "August", "w1"),
            &[],
            None,
        )
        .expect("warm splice");

        let (reads_after, folds_after) = {
            let c = cache.lock().unwrap();
            (c.leaves_read(), c.served_folds())
        };
        assert_eq!(
            reads_after - reads_before,
            1,
            "the door re-reads exactly the one member the previous write \
             spoiled — never the corpus"
        );
        assert_eq!(
            folds_after - folds_before,
            1,
            "one lane-C refold, only because the root advanced"
        );

        // The chain and the oracle: the warm write guards on the root the
        // previous write left, and its root_after IS the live law-1 fold.
        let frame = out.committed.expect("real splice commits");
        assert_eq!(
            frame.delta.root_before,
            first.committed.expect("first frame").delta.root_after,
            "root_before rides the previous write's root_after"
        );
        assert_eq!(
            frame.delta.root_after,
            ambient_root(&root).expect("oracle"),
            "root_after == the independent full-corpus law-1 fold"
        );
    }

    /// A caller-supplied cache is the tree the door overlays: `Trusted`
    /// door-entry serves the overlay (no new sweep), and `root_after` lands
    /// on that same `Arc` — not on `WRITE_CACHES`.
    #[test]
    fn a_supplied_cache_is_the_tree_the_door_overlays() {
        let (dir, root) = ws();
        for i in 0..8 {
            page(&dir, &format!("notes/bystander{i}.md"), "nothing");
        }
        page(&dir, "notes/plan.md", "August");

        let cache = std::sync::Arc::new(std::sync::Mutex::new({
            let mut cache = fs::DomainCache::new();
            cache.root(&root).expect("baseline");
            cache
        }));
        assert_eq!(
            cache.lock().unwrap().guard_currency(),
            fs::stable::GuardCurrency::Trusted
        );
        let sweeps_before = cache.lock().unwrap().sweeps();

        // A registry-shaped observation double: overlay when the memo is
        // vouchable, the live-fold floor otherwise (the production decision
        // is `Registry::door_observation`, tested in the registry crate).
        let observe = || {
            let mut memo = cache.lock().unwrap_or_else(PoisonError::into_inner);
            if matches!(memo.guard_currency(), fs::stable::GuardCurrency::Trusted)
                && let Ok(folded) = memo.overlay_root()
            {
                return Ok(folded);
            }
            memo.root(&root)
        };
        let out = splice(
            &root,
            None,
            &splice_args("notes/plan.md", "August", "w1"),
            &[],
            Some(ResidentDoor {
                cache: &cache,
                observe: &observe,
            }),
        )
        .expect("supplied-cache splice");
        let frame = out.committed.expect("real splice commits");

        let (sweeps_after, overlaid) = {
            let mut cache = cache.lock().unwrap();
            (cache.sweeps(), cache.overlay_root().expect("overlay"))
        };
        assert_eq!(
            sweeps_after, sweeps_before,
            "Trusted supplied cache serves overlay — no door-entry sweep"
        );
        assert_eq!(
            overlaid.0, frame.delta.root_after.0,
            "the supplied cache carries the commit's own overlay"
        );
        assert_ne!(
            std::sync::Arc::as_ptr(&cache),
            std::sync::Arc::as_ptr(&write_cache(&root)),
            "the door did not fall back to WRITE_CACHES"
        );
    }

    /// Injected loss on a supplied cache makes the next guarded write
    /// re-observe (the `Untrusted` degrade) and absorb the loss on that
    /// same cache.
    #[test]
    fn untrusted_supplied_cache_reobserves_and_absorbs() {
        let (dir, root) = ws();
        page(&dir, "notes/plan.md", "August");
        page(&dir, "notes/other.md", "still");

        let cache = std::sync::Arc::new(std::sync::Mutex::new({
            let mut cache = fs::DomainCache::new();
            cache.root(&root).expect("baseline");
            cache
        }));
        cache
            .lock()
            .unwrap()
            .feed_gen()
            .note_loss("fixture-induced overflow");
        assert!(
            matches!(
                cache.lock().unwrap().guard_currency(),
                fs::stable::GuardCurrency::Untrusted { .. }
            ),
            "loss drops guard currency before the door"
        );
        let sweeps_before = cache.lock().unwrap().sweeps();

        // The same registry-shaped observation double as
        // `a_supplied_cache_is_the_tree_the_door_overlays`: Untrusted takes
        // the floor arm, which absorbs the loss.
        let observe = || {
            let mut memo = cache.lock().unwrap_or_else(PoisonError::into_inner);
            if matches!(memo.guard_currency(), fs::stable::GuardCurrency::Trusted)
                && let Ok(folded) = memo.overlay_root()
            {
                return Ok(folded);
            }
            memo.root(&root)
        };
        splice(
            &root,
            None,
            &splice_args("notes/plan.md", "August", "w1"),
            &[],
            Some(ResidentDoor {
                cache: &cache,
                observe: &observe,
            }),
        )
        .expect("degrade splice");

        let (sweeps_after, currency) = {
            let cache = cache.lock().unwrap();
            (cache.sweeps(), cache.guard_currency())
        };
        assert!(
            sweeps_after > sweeps_before,
            "Untrusted degrades to a full observe that absorbs the loss"
        );
        assert_eq!(currency, fs::stable::GuardCurrency::Trusted);
    }

    /// Quality gate: a warm remove of an unreferenced file on a 16+ member
    /// corpus does not increment `leaves_read` by the corpus size. Referential
    /// I/O is `hash_domain` + byte reads, not a cache-backed merkle fold.
    #[test]
    fn remove_of_unreferenced_file_does_not_reread_the_corpus_through_the_cache() {
        let (dir, root) = ws();
        for i in 0..16 {
            page(&dir, &format!("notes/bystander{i}.md"), "nothing");
        }
        page(&dir, "notes/plan.md", "August");
        page(&dir, "notes/victim.md", "gone");

        splice(
            &root,
            None,
            &splice_args("notes/plan.md", "August", "w1"),
            &[],
            None,
        )
        .expect("cold splice observes the corpus");
        // Settle the spoiled member so the measured window is a warm observe.
        let mut settle = splice_args("notes/plan.md", "w1", "w2");
        settle.dry = true;
        splice(&root, None, &settle, &[], None).expect("settling dry splice");

        let cache = write_cache(&root);
        let reads_before = cache.lock().unwrap().leaves_read();
        let rev = live_rev(&root, "notes/victim.md");
        remove(&root, None, &remove_args("notes/victim.md", rev), &[])
            .expect("unreferenced victim dies");
        let reads_after = cache.lock().unwrap().leaves_read();
        assert_eq!(
            reads_after - reads_before,
            0,
            "a warm remove must not re-read the corpus through the cache \
             (referential I/O is not leaves_read)"
        );
        assert!(
            !dir.path().join("notes/victim.md").exists(),
            "the unreferenced file is gone"
        );
    }

    fn create_args(path: &str, body: &str) -> CreateArgs {
        CreateArgs {
            id: None,
            path: Path(path.into()),
            body: body.into(),
            actor: None,
            now: None,
            if_root: None,
            dry: false,
            fields: BTreeMap::default(),
            props: BTreeMap::default(),
        }
    }

    fn remove_args(path: &str, if_file_rev: NodeRev) -> RemoveArgs {
        RemoveArgs {
            id: None,
            path: Path(path.into()),
            if_file_rev: Some(if_file_rev),
            actor: None,
            now: None,
            if_root: None,
            dry: false,
        }
    }

    fn lock_args(path: &str, if_file_rev: NodeRev) -> LockWriteArgs {
        LockWriteArgs {
            id: None,
            path: Path(path.into()),
            lock: lock::Lock::new(),
            actor: None,
            now: None,
            if_root: None,
            if_file_rev,
            dry: false,
        }
    }

    fn set_member(path: &str, old: &str, new: &str) -> wire::SpliceFile {
        wire::SpliceFile {
            path: Path(path.into()),
            edits: vec![match_edit(old, new)],
            plan_edits: Vec::new(),
        }
    }

    fn set_args(files: Vec<wire::SpliceFile>) -> SpliceSetArgs {
        SpliceSetArgs {
            premises: Vec::new(),
            id: None,
            files,
            origin: crate::guard::Origin::InProcess,
            actor: None,
            now: None,
            receipt: None,
            if_root: None,
            dry: false,
            force: false,
        }
    }

    /// The live file's whole-file rev — the CAS token a door demands.
    fn live_rev(root: &fs::WorkspaceRoot, rel: &str) -> NodeRev {
        NodeRev(
            fs::load(root, std::path::Path::new(rel))
                .expect("load")
                .root
                .node_rev
                .0
                .clone(),
        )
    }

    /// Every door serves hash-law 2. No dual-law window.
    #[test]
    fn served_tokens_are_law2_on_every_door() {
        let (dir, root) = ws();
        page(&dir, "notes/plan.md", "August");
        page(&dir, "notes/second.md", "August");

        let law2 = |label: &str, served: &Root| {
            let oracle = ambient_root(&root).expect("oracle");
            assert_eq!(*served, oracle, "{label}: served token == disk fold");
            assert!(
                served.0.starts_with("b3a:"),
                "{label}: the token is hash-law 2: {}",
                served.0
            );
        };

        let born =
            create(&root, None, &create_args("notes/new.md", "# New\n"), &[]).expect("create");
        law2("create", born.root_after.as_ref().expect("root_after"));

        let out = splice(
            &root,
            None,
            &splice_args("notes/plan.md", "August", "w1"),
            &[],
            None,
        )
        .expect("splice");
        law2("splice", &out.committed.expect("frame").delta.root_after);

        let set = splice_set(
            &root,
            None,
            &set_args(vec![
                set_member("notes/plan.md", "w1", "w2"),
                set_member("notes/second.md", "August", "w2"),
            ]),
            &[],
        )
        .expect("splice_set");
        law2(
            "splice.set",
            &set.committed.expect("set frame").delta.root_after,
        );

        let rev = live_rev(&root, "notes/plan.md");
        let locked = lock_write(&root, None, &lock_args("notes/plan.md", rev)).expect("lock_write");
        law2(
            "lock_write",
            locked.root_after.as_ref().expect("root_after"),
        );

        let dead = remove(
            &root,
            None,
            &remove_args("notes/new.md", born.file_rev_after.clone()),
            &[],
        )
        .expect("remove");
        law2("remove", dead.root_after.as_ref().expect("root_after"));
    }

    /// The observation is LIVE: a foreign write between doors moves the served
    /// world, so a stale world guard refuses and a fresh one passes — the
    /// resident tree can never serve yesterday's root.
    #[test]
    fn external_change_is_caught_by_the_next_observation() {
        let (dir, root) = ws();
        page(&dir, "notes/plan.md", "August");
        page(&dir, "notes/foreign.md", "August");

        let out = splice(
            &root,
            None,
            &splice_args("notes/plan.md", "August", "w1"),
            &[],
            None,
        )
        .expect("seed splice");
        let stale = out.committed.expect("frame").delta.root_after;

        // A foreign writer (no flock, no door) rewrites a bystander.
        std::fs::write(
            dir.path().join("notes/foreign.md"),
            "# Alpha\n\n## Beta\n\nforeign edit\n",
        )
        .expect("foreign write");

        let mut guarded = splice_args("notes/plan.md", "w1", "w2");
        guarded.if_root = Some(stale.clone());
        let err =
            splice(&root, None, &guarded, &[], None).expect_err("a stale world guard must refuse");
        assert_eq!(err.code, ErrorCode::RootMismatch);

        let live = ambient_root(&root).expect("oracle");
        assert_ne!(stale, live, "the foreign write moved the world");
        let mut fresh = splice_args("notes/plan.md", "w1", "w2");
        fresh.if_root = Some(live);
        splice(&root, None, &fresh, &[], None).expect("a fresh world guard passes");
    }

    /// A write that moves the DOMAIN itself overlays membership from the
    /// commit's own config bytes against current leaves — no second observe.
    #[test]
    fn domain_config_write_overlays_membership() {
        let (dir, root) = ws();
        page(&dir, "notes/plan.md", "August");
        page(&dir, "drafts/scratch.md", "August");

        // Seed: the resident tree holds BOTH members.
        splice(
            &root,
            None,
            &splice_args("notes/plan.md", "August", "w1"),
            &[],
            None,
        )
        .expect("seed splice");

        // Birth the domain config through the guarded door: drafts/** leaves
        // the hash domain in the same write that lands the page.
        let born = create(
            &root,
            None,
            &create_args(
                fs::domain::DOMAIN_CONFIG_PATH,
                "---\nignore:\n  - \"drafts/**\"\n---\n# Domain\n",
            ),
            &[],
        )
        .expect("config birth");
        let served = born.root_after.expect("root_after");
        assert_eq!(
            served,
            ambient_root(&root).expect("oracle"),
            "the config write's own root already excludes drafts/** and \
             includes the config page"
        );

        // And the next ordinary write still serves the new-law world.
        let out = splice(
            &root,
            None,
            &splice_args("notes/plan.md", "w1", "w2"),
            &[],
            None,
        )
        .expect("post-config splice");
        assert_eq!(
            out.committed.expect("frame").delta.root_after,
            ambient_root(&root).expect("oracle"),
        );
    }

    /// Dry runs and refusals advance nothing: no fold, no served movement —
    /// the rehearsal observes the same world the real write would.
    #[test]
    fn dry_run_folds_nothing_and_moves_nothing() {
        let (dir, root) = ws();
        page(&dir, "notes/plan.md", "August");
        splice(
            &root,
            None,
            &splice_args("notes/plan.md", "August", "w1"),
            &[],
            None,
        )
        .expect("seed splice");

        let cache = write_cache(&root);
        // The seed spoiled notes/plan.md; one observation re-reads it and
        // settles the memo before the measured window.
        let mut settle = splice_args("notes/plan.md", "w1", "w2");
        settle.dry = true;
        splice(&root, None, &settle, &[], None).expect("settling dry splice");
        let (reads_before, folds_before) = {
            let c = cache.lock().unwrap();
            (c.leaves_read(), c.served_folds())
        };

        let mut dry = splice_args("notes/plan.md", "w1", "w2");
        dry.dry = true;
        let out = splice(&root, None, &dry, &[], None).expect("dry splice");
        assert!(out.committed.is_none(), "dry commits nothing");

        let (reads_after, folds_after) = {
            let c = cache.lock().unwrap();
            (c.leaves_read(), c.served_folds())
        };
        assert_eq!(reads_after - reads_before, 0, "a dry run reads no member");
        assert_eq!(folds_after - folds_before, 0, "a dry run refolds nothing");
    }

    /// Race fixture: a foreign rewrite of B between door observe and commit
    /// must not enter `root_after`. Overlay(door-entry leaves, own writes) wins;
    /// `ambient_root` after the racer does not.
    #[allow(clippy::too_many_lines)]
    #[test]
    fn root_after_ignores_a_foreign_racer_on_every_door() {
        let b0 = page_body("bystander").into_bytes();
        let raced = page_body("RACED").into_bytes();
        assert_ne!(b0, raced, "the racer must move B's digest");

        // splice
        {
            let (dir, root) = ws();
            page(&dir, "notes/plan.md", "August");
            page(&dir, "notes/foreign.md", "bystander");
            race_foreign(dir.path());
            let out = splice(
                &root,
                None,
                &splice_args("notes/plan.md", "August", "w1"),
                &[],
                None,
            )
            .expect("splice");
            let served = out.committed.expect("frame").delta.root_after;
            let expected = fold_owned(
                0,
                &[
                    ("notes/foreign.md", b0.clone()),
                    ("notes/plan.md", page_body("w1").into_bytes()),
                ],
            );
            assert_eq!(served, expected, "splice root_after stays A1+B0");
            assert_ne!(
                served,
                ambient_root(&root).expect("oracle"),
                "splice ambient absorbed the racer"
            );
        }

        // splice.set
        {
            let (dir, root) = ws();
            page(&dir, "notes/plan.md", "August");
            page(&dir, "notes/second.md", "August");
            page(&dir, "notes/foreign.md", "bystander");
            race_foreign(dir.path());
            let set = splice_set(
                &root,
                None,
                &set_args(vec![
                    set_member("notes/plan.md", "August", "w1"),
                    set_member("notes/second.md", "August", "w1"),
                ]),
                &[],
            )
            .expect("splice_set");
            let served = set.committed.expect("set frame").delta.root_after;
            let expected = fold_owned(
                0,
                &[
                    ("notes/foreign.md", b0.clone()),
                    ("notes/plan.md", page_body("w1").into_bytes()),
                    ("notes/second.md", page_body("w1").into_bytes()),
                ],
            );
            assert_eq!(served, expected, "splice.set root_after stays own+B0");
            assert_ne!(served, ambient_root(&root).expect("oracle"));
        }

        // create
        {
            let (dir, root) = ws();
            page(&dir, "notes/plan.md", "August");
            page(&dir, "notes/foreign.md", "bystander");
            race_foreign(dir.path());
            let born =
                create(&root, None, &create_args("notes/new.md", "# New\n"), &[]).expect("create");
            let served = born.root_after.expect("root_after");
            let newborn = std::fs::read(dir.path().join("notes/new.md")).expect("new");
            let expected = fold_owned(
                0,
                &[
                    ("notes/foreign.md", b0.clone()),
                    ("notes/new.md", newborn),
                    ("notes/plan.md", page_body("August").into_bytes()),
                ],
            );
            assert_eq!(served, expected, "create root_after stays own+B0");
            assert_ne!(served, ambient_root(&root).expect("oracle"));
        }

        // lock_write
        {
            let (dir, root) = ws();
            page(&dir, "notes/plan.md", "August");
            page(&dir, "notes/foreign.md", "bystander");
            let rev = live_rev(&root, "notes/plan.md");
            race_foreign(dir.path());
            let locked =
                lock_write(&root, None, &lock_args("notes/plan.md", rev)).expect("lock_write");
            let served = locked.root_after.expect("root_after");
            let plan = std::fs::read(dir.path().join("notes/plan.md")).expect("plan after lock");
            let expected = fold_owned(
                0,
                &[("notes/foreign.md", b0.clone()), ("notes/plan.md", plan)],
            );
            assert_eq!(served, expected, "lock_write root_after stays own+B0");
            assert_ne!(served, ambient_root(&root).expect("oracle"));
        }

        // remove
        {
            let (dir, root) = ws();
            page(&dir, "notes/plan.md", "August");
            page(&dir, "notes/foreign.md", "bystander");
            let born =
                create(&root, None, &create_args("notes/new.md", "# New\n"), &[]).expect("birth");
            race_foreign(dir.path());
            let dead = remove(
                &root,
                None,
                &remove_args("notes/new.md", born.file_rev_after.clone()),
                &[],
            )
            .expect("remove");
            let served = dead.root_after.expect("root_after");
            let expected = fold_owned(
                0,
                &[
                    ("notes/foreign.md", b0.clone()),
                    ("notes/plan.md", page_body("August").into_bytes()),
                ],
            );
            assert_eq!(served, expected, "remove root_after stays remaining+B0");
            assert_ne!(served, ambient_root(&root).expect("oracle"));
        }

        // domain-config write
        {
            let (dir, root) = ws();
            page(&dir, "notes/plan.md", "August");
            page(&dir, "notes/foreign.md", "bystander");
            page(&dir, "drafts/scratch.md", "August");
            let config = "---\nignore:\n  - \"drafts/**\"\n---\n# Domain\n";
            race_foreign(dir.path());
            let born = create(
                &root,
                None,
                &create_args(fs::domain::DOMAIN_CONFIG_PATH, config),
                &[],
            )
            .expect("config birth");
            let served = born.root_after.expect("root_after");
            let domain = std::fs::read(dir.path().join(fs::domain::DOMAIN_CONFIG_PATH))
                .expect("landed config");
            let expected = fold_owned(
                0,
                &[
                    ("meridian/domain.md", domain),
                    ("notes/foreign.md", b0),
                    ("notes/plan.md", page_body("August").into_bytes()),
                ],
            );
            assert_eq!(
                served, expected,
                "config write overlays membership + B0, drops drafts"
            );
            assert_ne!(served, ambient_root(&root).expect("oracle"));
        }
    }
}

/// **The create door's frontmatter serializer** (D6, card 17): `props` is data
/// the DOOR quotes, so the quoting-injection class closes once for every
/// record-birthing block instead of once per block.
///
/// The table below is the card's own hostile-value gate: every value a program
/// could previously have smuggled a key, a comment, a collection or a second
/// line through lands as ONE scalar that reads back byte-identically — or, for
/// the one shape v1 frontmatter cannot hold (a newline), refuses the birth.
#[cfg(test)]
mod create_props_door {
    use std::collections::BTreeMap;
    use wire::{ErrorCode, Path};

    use super::{CreateArgs, PropValue, create};

    fn ws() -> (tempfile::TempDir, fs::WorkspaceRoot) {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = fs::WorkspaceRoot(dir.path().to_path_buf());
        (dir, root)
    }

    fn args(path: &str, body: &str, props: &[(&str, PropValue)]) -> CreateArgs {
        CreateArgs {
            id: None,
            path: Path(path.into()),
            body: body.into(),
            actor: Some("alice".into()),
            now: None,
            if_root: None,
            dry: false,
            fields: BTreeMap::default(),
            props: props
                .iter()
                .map(|(k, v)| ((*k).to_owned(), v.clone()))
                .collect(),
        }
    }

    fn scalar(v: &str) -> PropValue {
        PropValue::Scalar(v.to_owned())
    }

    /// The read-back: the § A.6.1 decode every read seam serves through, so
    /// "byte-identical" is asserted through the reader's own eyes, never by
    /// re-implementing the decode in the test.
    fn read_back(dir: &tempfile::TempDir, path: &str) -> policy::defs::FmMeta {
        let raw = std::fs::read_to_string(dir.path().join(path)).expect("born file");
        policy::defs::parse_meta(&raw)
            .expect("the born frontmatter parses")
            .expect("the born file carries frontmatter")
    }

    /// A refusal's sentence — the wire body carries it optionally.
    fn msg(err: &wire::ErrorBody) -> String {
        err.message.clone().unwrap_or_default()
    }

    /// THE HOSTILE-VALUE TABLE. Each value is one way a hand-rolled escaper
    /// leaked; each lands as one scalar and reads back as itself.
    #[test]
    fn hostile_values_land_as_single_line_scalars_and_read_back_identical() {
        let (dir, root) = ws();
        let hostile: Vec<(&str, &str)> = vec![
            ("dq", "a \" quote"),
            ("sq", "it's mine"),
            ("colon", "key: value"),
            ("hash", "# not a comment"),
            ("dash", "- not a list item"),
            ("space", "  padded  "),
            ("wikilink", "[[f6656ff1]]"),
            ("yaml", "{a: 1}"),
            ("fence", "--- not a fence"),
            ("empty", ""),
            ("brackets", "[a, b]"),
            ("newline_escape", "a\\nb"),
        ];
        let props: Vec<(&str, PropValue)> = hostile.iter().map(|(k, v)| (*k, scalar(v))).collect();
        create(&root, None, &args("notes/hostile.md", "# H\n", &props), &[])
            .expect("the birth lands");

        let meta = read_back(&dir, "notes/hostile.md");
        for (key, value) in &hostile {
            assert_eq!(
                meta.get(*key),
                Some(&policy::defs::FmValue::Str((*value).to_owned())),
                "{key} must read back as the caller's own string"
            );
        }
        let raw = std::fs::read_to_string(dir.path().join("notes/hostile.md")).expect("born");
        assert_eq!(
            raw.lines().filter(|l| *l == "---").count(),
            2,
            "no hostile value opened or closed a second frontmatter block: {raw}"
        );
        assert!(
            raw.ends_with("# H\n"),
            "the body follows the door's block verbatim: {raw}"
        );
    }

    /// A list value lands as a ONE-LINE flow list — the spelling the corpus
    /// carries (`tags: [type/agent]`) — and a hostile member quotes instead of
    /// ending the list early.
    #[test]
    fn list_values_land_as_a_flow_list_a_reader_projects() {
        let (dir, root) = ws();
        create(
            &root,
            None,
            &args(
                "notes/list.md",
                "# L\n",
                &[
                    ("tags", PropValue::List(vec!["type/agent".into()])),
                    (
                        "hostile",
                        PropValue::List(vec!["a, b".into(), "]".into(), "[x, y]".into()]),
                    ),
                    ("empty", PropValue::List(vec![])),
                ],
            ),
            &[],
        )
        .expect("the birth lands");

        let raw = std::fs::read_to_string(dir.path().join("notes/list.md")).expect("born");
        assert!(
            raw.contains("tags: [type/agent]\n"),
            "the plain list keeps the corpus spelling: {raw}"
        );
        let meta = read_back(&dir, "notes/list.md");
        assert_eq!(
            meta.get("tags"),
            Some(&policy::defs::FmValue::List(vec![
                policy::defs::FmValue::Str("type/agent".into())
            ])),
            "a list value reads back as a list"
        );
        let policy::defs::FmValue::List(members) =
            meta.get("hostile").expect("hostile list").clone()
        else {
            panic!("the hostile list must still be a list: {raw}");
        };
        assert_eq!(
            members,
            vec![
                policy::defs::FmValue::Str("a, b".into()),
                policy::defs::FmValue::Str("]".into()),
                policy::defs::FmValue::Str("[x, y]".into()),
            ],
            "every hostile member reads back as itself, three members not six"
        );
    }

    /// **The former named residual, now closed** (2026-08-23, card
    /// `all-digit-short-ids-read-as-int`). A props scalar whose text is a typed
    /// YAML scalar used to land verbatim and read back typed. It quotes now, at
    /// this door and at every other, because the same carve-out emitted the
    /// all-digit agent short id `19895504` as an INTEGER — ids are the fleet's
    /// join key. `props` is a STRING plane: what the caller spells as a string
    /// reads back as that string.
    ///
    /// The residual that remains, pinned so it is a decision and not an
    /// accident: this door has no numeric or boolean typed arm, so the integer
    /// 7 can no longer be authored through `props` at all (`PropValue::List`
    /// stays the one typed arm) — a def-declared `int`/`bool` property must be
    /// born in the record's own body bytes.
    #[test]
    fn a_typed_scalar_now_quotes_and_reads_back_as_a_string() {
        let (dir, root) = ws();
        create(
            &root,
            None,
            &args(
                "notes/typed.md",
                "# T\n",
                &[
                    ("n", scalar("7")),
                    ("flag", scalar("true")),
                    ("owner", scalar("19895504")),
                ],
            ),
            &[],
        )
        .expect("the birth lands");
        let raw = std::fs::read_to_string(dir.path().join("notes/typed.md")).expect("born");
        assert!(
            raw.contains("n: \"7\"\n")
                && raw.contains("flag: \"true\"\n")
                && raw.contains("owner: \"19895504\"\n"),
            "{raw}"
        );
        let meta = read_back(&dir, "notes/typed.md");
        assert_eq!(meta.get("n"), Some(&policy::defs::FmValue::Str("7".into())));
        assert_eq!(
            meta.get("flag"),
            Some(&policy::defs::FmValue::Str("true".into()))
        );
        assert_eq!(
            meta.get("owner"),
            Some(&policy::defs::FmValue::Str("19895504".into())),
            "an all-digit short id is a STRING at the create door too"
        );
    }

    /// D11 at this door too: a newline cannot ride a v1 frontmatter value, so
    /// the BIRTH refuses — the value is never sanitized into something the
    /// caller did not write, and nothing lands.
    #[test]
    fn a_newline_value_refuses_the_birth() {
        let (dir, root) = ws();
        let err = create(
            &root,
            None,
            &args("notes/nl.md", "# N\n", &[("bad", scalar("a\nb: forged"))]),
            &[],
        )
        .expect_err("a newline value refuses");
        assert_eq!(err.code, ErrorCode::BadRequest);
        assert!(
            msg(&err).contains("\"bad\"") && msg(&err).contains("newline"),
            "the refusal names the key and the law: {}",
            msg(&err)
        );
        assert!(
            !dir.path().join("notes/nl.md").exists(),
            "nothing landed on a refused birth"
        );
    }

    /// A key outside the property-key grammar refuses through the SAME sentence
    /// the patch face teaches — one law, one owner.
    #[test]
    fn a_forged_key_refuses_the_birth() {
        let (_dir, root) = ws();
        let err = create(
            &root,
            None,
            &args("notes/key.md", "# K\n", &[("a: b\nc", scalar("x"))]),
            &[],
        )
        .expect_err("a forged key refuses");
        assert_eq!(err.code, ErrorCode::BadRequest);
        assert_eq!(
            msg(&err),
            policy::defs::invalid_property_key_refusal("a: b\nc")
        );
    }

    /// `props=` AND a body that opens its own fence are two spellings of one
    /// block: the door refuses rather than pick one.
    #[test]
    fn props_beside_a_body_frontmatter_refuses() {
        let (_dir, root) = ws();
        let err = create(
            &root,
            None,
            &args(
                "notes/both.md",
                "---\nstatus: open\n---\n# B\n",
                &[("type", scalar("agent"))],
            ),
            &[],
        )
        .expect_err("two frontmatter spellings refuse");
        assert_eq!(err.code, ErrorCode::BadRequest);
        assert!(
            msg(&err).contains("two spellings of one block"),
            "the refusal teaches the choice: {}",
            msg(&err)
        );
    }

    /// Empty `props` is the shipped birth, byte for byte: the door adds no
    /// frontmatter a caller did not ask for.
    #[test]
    fn empty_props_leaves_the_body_untouched() {
        let (dir, root) = ws();
        create(&root, None, &args("notes/plain.md", "# P\n", &[]), &[]).expect("the birth lands");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("notes/plain.md")).expect("born"),
            "# P\n"
        );
    }
}
