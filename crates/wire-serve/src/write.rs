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
//! # The delta ring lives with the caller
//! [`commit_batch`] assembles one `DeltaFrame` at the single §7.3 constructor
//! ([`assemble_delta`]) and returns it; it does not hold or advance a ring.
//! The resident daemon advances its per-workspace ring with the returned
//! frame; a ringless in-process caller discards it.

use std::fmt::Write as _;
use std::io::ErrorKind;
use std::path::Path as FsPath;

use wire::{
    Armed, ArmedEdit, Delta, DeltaFile, DeltaFrame, Edit, EditShape, ErrorBody, ErrorCode,
    HpathSeg, NodeRev, Path, PutAt, ReceiptAddr, ReceiptFact, ResponseBody, Root, SecRef, Severity,
    Span, Verdict, WouldCorruptFamily,
};

use crate::read::{ambiguous, to_model_ref};
use crate::{ambient_root, bad_request, load_doc};

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
    /// (`crate::plan::lower`); armed facts align 1:1 with the lowered edits.
    /// Empty = the native form.
    pub plan_edits: Vec<wire::PlanEdit>,
    /// `splice.pin`: the pin riding this splice. `args.path` is the pinning
    /// page — the page whose `meridian-lock` block records the claim, so the
    /// lock write is a content edit on this splice's own file and lands in the
    /// same [`commit_batch`] rename. A pin-only splice carries no `edits`. The
    /// pin's actor is `self.actor` and nothing else.
    pub pin: Option<wire::PinSpec>,
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
    mints: Option<&receipt::read_mint::ReadMintStore>,
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

    let mut doc = load_doc(root, &args.path)?;
    let mut root_before = ambient_root(root)?;

    // §5.1 order: the world guard first — so a stale plan refuses before any
    // per-target resolution can answer for it, and before this splice's own
    // promotion can advance the root it guards on.
    world_guard(args.if_root.as_ref(), &root_before)?;

    // The pin prologue, ordered gate → fingerprint + blob → anchor promotion.
    // It runs inside the flock this splice already holds, so the receipt's
    // rev-recheck reads the same pre-image the batch will validate against,
    // and the promotion needs no second flock.
    let mut pin = match &args.pin {
        Some(spec) => Some(mint_pin(
            root,
            spec,
            args.actor.as_deref(),
            args.force,
            mints,
        )?),
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
        && same_file(root, &p.target, &args.path)
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
    // ([`strip_fp_candidate`] below).
    let mut effective_edits = if args.plan_edits.is_empty() {
        args.edits.clone()
    } else {
        crate::plan::lower(&doc, &args.plan_edits)?
    };

    // Fingerprint-or-force, mounted here and nowhere else — post-lowering is
    // the one point both write faces reach, so native `edits` cannot walk
    // around it. Per-edit, so an empty batch (`mrd pin`) has nothing to demand.
    // See `crate::guard`.
    let bypassed = crate::guard::guard_batch(
        args.origin,
        args.force,
        &doc,
        &args.path,
        &args.plan_edits,
        &mut effective_edits,
    )?;
    let effective_edits = &effective_edits[..];

    let (model_edits, before_facts) =
        model_edits_and_before_facts(&doc, effective_edits, &args.path)?;
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
            return Err(verdict_to_wire(&refused, effective_edits, &doc, &args.path));
        }
    };

    // Build the post-batch document state once, shared by the armed AFTER facts
    // and the verdicts, on both the dry and real paths (§4.4 one-reparse law) —
    // the real commit writes exactly these bytes, so evaluating this simulated
    // doc is evaluating the committed doc.
    let mut after_doc = build_after_doc(&doc, &sealed, &args.path);

    // The `@fp` strip runs over the candidate. It rewrites the batch's payloads
    // (so `commit_batch`'s re-validation lands the same bytes judged here) and
    // leaves a document-grain assertion behind it: any token still standing in
    // a claim-link position refuses loud instead of landing silently.
    let mut sealed = sealed;
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

    let armed_edits = simulate_armed_edits(after_doc.document(), effective_edits, &before_facts)?;

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
    if let Some(minted) = pin.as_ref()
        && let Some(p) = minted.promotion.as_ref()
    {
        fs::replace_file(root, FsPath::new(&p.target.0), &p.candidate)
            .map_err(|e| io_to_wire(&e))?;
        // D16: refresh the actor's receipt to the rev this engine write
        // created. The promotion moved the section's `sec_rev` (a rev is over
        // raw bytes) without moving one byte of what the actor read, so leaving
        // the old rev would fail the actor's own gate on its next pin. Only for
        // a receipt that already passed the gate at mint time; a foreign
        // content change still refuses.
        if let (Some(store), Some(actor)) = (mints, crate::read::mint_actor(args.actor.as_deref()))
        {
            store.mint(actor, &p.target.0, &minted.fact.selector, &p.sec_rev);
        }
        // The promotion moved the corpus root — this splice's own write.
        // Re-read it so the receipt records the root the commit reports, and
        // re-guard the batch on the current value: re-comparing the client's
        // pre-promotion token would self-refuse `root_mismatch` on our own
        // write. The client's guard was already honored above.
        root_before = ambient_root(root)?;
        batch.if_root = args
            .if_root
            .as_ref()
            .map(|_| model::MerkleRoot(root_before.0.clone()));
    }

    // Real commit: render the receipt line (§6.1), fold the append, honor the
    // parent-dir obligation, then drive the commit seam.
    let receipt_input = match &args.receipt {
        Some(addr) => Some(receipt_input(
            root,
            args,
            effective_edits,
            &root_before,
            &armed_edits,
            addr,
        )?),
        None => None,
    };
    // The batch moves into the commit seam, so the edits the reaction reports
    // on are captured while they are still here. The feeder below runs only if
    // `commit_batch` succeeds.
    let landed_edits = batch.edits.clone();
    let mut frame = commit_batch(
        seq,
        // The workspace rides the flock, so root/flock cannot disagree.
        &flock,
        &CommitRequest {
            content_path: args.path.0.clone(),
            batch,
            receipt: receipt_input,
            actor: args.actor.clone(),
            now: args.now.clone(),
        },
    )
    .map_err(|e| match e {
        CommitError::Refused(v) => verdict_to_wire(&v, effective_edits, &doc, &args.path),
        CommitError::Env(err) => err,
        CommitError::Io(err) => commit_io_to_wire(&err, &args.path),
    })?;

    // Reaction mode (C3): evaluated only after the batch landed, and it can
    // neither refuse nor mutate what landed (§4.4 — the notify path attaches at
    // reaction mode, never at the gate). The outcome rides two carriers: this
    // seq's frame, which the host flushes to subscribers, and the caller's
    // `armed` feedback below.
    //
    // A fault means "emit no reaction", never "fail the write". It is not
    // dropped either: it rides the frame as a `wire::EffectFinding::ArmedFault`.
    let armed_effects = crate::reaction::feed_landed_change(
        root,
        &doc,
        after_doc.document(),
        &landed_edits,
        policy::ChangeOp::Splice,
        args.actor.as_deref(),
    );
    frame.effects.clone_from(&armed_effects);

    // The receipt FACT from the true post-state (host-block-leaf grain).
    let receipt_fact = resolve_receipt_fact(root, args.receipt.as_ref())?;

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
                // What this write armed, stated synchronously to the caller:
                // matched rules, their intents and each canonical receipt
                // address — never who gets notified, and never that anything
                // was delivered. This response is complete before the host
                // flushes the frame to any subscriber.
                effects: armed_effects,
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
    }
}

/// One `remove` request's fields. `if_file_rev` is the rev the caller read —
/// remove-what-you-read: the live file must still carry it, or the death
/// refuses citing the drift. `if_root`/`dry` mirror `create`.
#[derive(Debug, Clone)]
pub struct RemoveArgs {
    pub id: Option<u64>,
    /// The path whose file is removed (workspace-confined).
    pub path: Path,
    /// The whole-file rev the caller read — the remove-what-you-read guard.
    pub if_file_rev: NodeRev,
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
/// Order: path confinement → world guard (§5.1) → the gate seam over the
/// birth's after-state → the `if_absent` CAS at the disk edge
/// ([`fs::create_file`], the single source of the guard) → root advance → birth
/// Delta. `dry: true` runs everything except disk and still refuses a would-be
/// clobber.
///
/// # Errors
/// `bad_path` (escapes the workspace), `root_mismatch` (stale world guard),
/// `cas_mismatch` (the path is occupied — taxonomy row 13, recovery `refresh`),
/// or an I/O failure. In every error case nothing was created.
pub fn create(
    root: &fs::WorkspaceRoot,
    seq: Option<&dyn crate::seq::SeqSink>,
    args: &CreateArgs,
    rulesets: &[policy::CompiledRuleset],
) -> Result<CreateOutcome, Box<ErrorBody>> {
    let fs_path = FsPath::new(&args.path.0);
    path_confined(root, &args.path)?;

    // D9: births serialize on the same write flock as every meridian writer —
    // this also closes the `if_absent` check→rename window for cooperators.
    let flock = acquire_write_lock(root)?;

    let root_before = ambient_root(root)?;
    world_guard(args.if_root.as_ref(), &root_before)?;

    // The payload IS the candidate document here, so `strip_fp` over the whole
    // body runs at document grain — the same grammar `strip_fp_candidate`
    // applies to a splice. The rev the birth reports is therefore the rev of
    // the bytes that land, never of a decorated draft.
    let body = syntax::strip_fp(&args.body);

    // U12 — the stored-form translation at the BIRTH door (see
    // [`translate_stored_body`]).
    let body = translate_stored_body(body, &args.path)?;

    // The birth's after-state, built once from the body (path-stamped so the
    // gate sees it). Its whole-file rev is the born file's rev.
    let after_doc = model::candidate_of_body(&args.path.0, body.into_owned());

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
    let root_after = ambient_root(root)?;

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
    Ok(CreateOutcome {
        root_before,
        root_after: Some(root_after),
        file_rev_after,
        committed: Some(committed),
        verdicts,
        dry: false,
    })
}

/// **Guarded `remove`**: death of one file under CAS remove-what-you-read +
/// workspace-root, and emit the `deleted` change surface.
///
/// Order: path confinement → world guard (§5.1) → load the live file (absent ⇒
/// `file_not_found`) → the remove-what-you-read CAS (the live rev must equal
/// `if_file_rev`, else refuse citing rev read vs found) → the gate seam over
/// the death's before-state → unlink → root advance → death Delta.
///
/// # Errors
/// `bad_path`, `root_mismatch`, `file_not_found` (nothing to remove),
/// `cas_mismatch` (the file drifted from the read rev — taxonomy row 14,
/// recovery `refresh`), or an I/O failure. In every error case nothing was
/// removed.
pub fn remove(
    root: &fs::WorkspaceRoot,
    seq: Option<&dyn crate::seq::SeqSink>,
    args: &RemoveArgs,
    rulesets: &[policy::CompiledRuleset],
) -> Result<RemoveOutcome, Box<ErrorBody>> {
    let fs_path = FsPath::new(&args.path.0);
    path_confined(root, &args.path)?;

    // D9: deaths serialize on the same write flock (read-rev CAS → unlink is
    // a critical section like any other write).
    let flock = acquire_write_lock(root)?;

    let root_before = ambient_root(root)?;
    world_guard(args.if_root.as_ref(), &root_before)?;

    // Load what is there — you cannot remove nothing (`file_not_found`, env).
    let before_doc = load_doc(root, &args.path)?;
    let current = NodeRev(before_doc.root.node_rev.0.clone());

    // remove-what-you-read CAS (row 14, recovery refresh): the live rev must
    // still equal the rev the caller read. Drift refuses citing rev read
    // (`expected`) vs found (`actual`).
    if args.if_file_rev != current {
        return Err(cas_mismatch(&args.if_file_rev, &current));
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
    let root_after = ambient_root(root)?;

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
    Ok(RemoveOutcome {
        root_before,
        root_after: Some(root_after),
        file_rev_before: current,
        committed: Some(committed),
        verdicts,
        dry: false,
    })
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
/// A birthed block appends at EOF, separated from existing content by exactly
/// one blank line, and the file ends with one terminator. A replaced block
/// keeps its exact span (fence-to-fence).
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
    let fs_path = FsPath::new(&args.path.0);
    path_confined(root, &args.path)?;

    // D9: the lock write serializes on the same write flock as every writer.
    let flock = acquire_write_lock(root)?;

    let before_doc = load_doc(root, &args.path)?;
    let file_rev_before = NodeRev(before_doc.root.node_rev.0.clone());

    let root_before = ambient_root(root)?;
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
    let root_after = ambient_root(root)?;

    let files = model::delta::file_delta(Some(&before_doc), Some(after_doc.document()))
        .map(|fd| vec![wire_map::project_file_delta(&args.path.0, &fd)])
        .unwrap_or_default();
    // Allocate inside the flock this fn already holds, not at the caller before
    // it. See `crate::seq`.
    let seq = crate::seq::allocate(seq, &flock, &root_before, &root_after, &files);
    let committed = assemble_delta(
        seq,
        root_before.clone(),
        root_after.clone(),
        args.actor.clone(),
        args.now.clone(),
        files,
    );
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
#[derive(Debug)]
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
}

/// An anchor promotion that has been decided and not written: the exact bytes,
/// the page they belong to, and the receipt refresh the write owes.
///
/// The promotion touches a different file from the one the request names, so a
/// rung refusing after it would leave bytes in a page the caller never asked to
/// change. Held here, it lands after the last such rung.
#[derive(Debug)]
struct PendingPromotion {
    /// The page the marker lands in — the pin's target, which may be the pinning
    /// page itself.
    target: Path,
    /// The sealed candidate to write — its bytes are the exact bytes that land,
    /// and also the pinning page's pre-image when the target IS the pinning
    /// page.
    candidate: model::CandidateDocument,
    /// The promoted section's `sec_rev` in those bytes — the D16 receipt refresh
    /// the write owes (a rev this engine moved, invisible to the fingerprint).
    sec_rev: String,
}

/// The pin prologue: resolve the target, gate it against the read-mint ledger,
/// decide the stable anchor, and mint the fingerprint + blob oid over the bytes
/// the promotion will land.
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
/// `read_mint_required` (D16 — a session actor pinning unread content),
/// `write_conflict` (the receipt's rev is stale), a `convention_fault` /
/// `armed_drift` / `index_integrity` gate refusal on the promotion, `io_error`.
#[allow(clippy::too_many_lines)]
fn mint_pin(
    root: &fs::WorkspaceRoot,
    spec: &wire::PinSpec,
    actor: Option<&str>,
    force: bool,
    mints: Option<&receipt::read_mint::ReadMintStore>,
) -> Result<PinMint, Box<ErrorBody>> {
    path_confined(root, &spec.target)?;

    let mut target_doc = load_doc(root, &spec.target).map_err(|e| {
        if e.code == ErrorCode::FileNotFound {
            pin_target_missing(&spec.target, format!("no page at {} to pin", spec.target.0))
        } else {
            e
        }
    })?;
    // The armed gate scopes its rules by the document's path, and `fs::load`
    // leaves that empty — an unstamped pre-image is a page no path-scoped
    // convention can see.
    stamp_path(&mut target_doc, &spec.target);

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
                &spec.target,
                format!(
                    "no section addressed by \"{}\" in {}. Nothing was written — the pin's \
                     page is byte-untouched. {}",
                    asked.display(),
                    spec.target.0,
                    crate::section_recovery(&asked.display(), Some(spec.target.0.as_str()))
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
            e.path = Some(spec.target.clone());
            return Err(Box::new(e));
        }
    };
    // The canonical selector: what the caller asked resolved to, in the read
    // face's own tagged grammar — never the caller's spelling, and never a
    // dewey ordinal (an ordinal is positional and a pin must outlive the next
    // edit). This is the receipt key, the same structure the mint side keyed on.
    let selector = canonical_selector(fact);
    // Refusal messages still need a spelling to name back at a human.
    let selector_text = selector.display();
    // Captured before the promotion re-resolve borrows the doc again: anchor
    // rows carry a block id (heading rows do not), and the raw title is what the
    // D15 slug derives from.
    let fact_anchor = fact.anchor.clone();
    let title = fact.title.clone();
    // The raw segment array the lock's `path` array is built from.
    let fact_segments = fact.hpath.clone();

    // D16: the gate, and its rev-recheck against the bytes on disk right now —
    // a receipt answers "was it read", never "is it current".
    read_mint_gate(mints, actor, &spec.target, &selector, &fact.sec_rev)?;

    let fact_span = span_range(fact.span);
    let slot = promotion_slot(&target_doc.raw, fact_span.start);
    let (anchor, promote) = decide_anchor(
        &target_doc,
        &spec.target,
        fact_anchor.as_deref(),
        slot,
        &title,
        &selector_text,
    )?;

    // Compose the promotion in memory and mint from those bytes: the blob oid
    // is the whole file's content id, so taking it from the pre-promotion bytes
    // would record an oid for a state that ceases to exist the moment the
    // marker lands (and `--vibe` would eagerly write that unreachable blob).
    // The fingerprint agrees either way, because the promotion is rev-neutral.
    let mut gate = crate::gate::GatePass::default();
    let promoted = if promote {
        let (candidate, pass) =
            plan_promotion(root, &spec.target, &target_doc, slot, &anchor, actor, force)?;
        gate = pass;
        Some(candidate)
    } else {
        None
    };
    let pinned_doc: &model::Document = promoted.as_ref().map_or(&target_doc, |c| c.document());

    let (span, promoted_sec_rev, segments) = if promote {
        post_promotion_facts(pinned_doc, &spec.target, &selector)?
    } else {
        (fact_span, String::new(), fact_segments)
    };

    let fingerprint = mint_fingerprint(pinned_doc, &span, &spec.target, &selector_text)?;
    let blob = blob_oid(
        root,
        &spec.target,
        promoted.as_ref().map(model::CandidateDocument::raw),
        spec.vibe.unwrap_or(false),
    )?;
    refuse_unrepresentable_heading(pinned_doc, &span, fact_anchor.as_deref(), &selector_text)?;
    let row = pin_row(
        &spec.target,
        fact_anchor.as_deref(),
        &segments,
        blob.as_deref(),
    )?;

    Ok(PinMint {
        fact: wire::PinFact {
            target: spec.target.clone(),
            selector,
            fingerprint,
            blob,
            anchor,
            promoted: promote,
        },
        row,
        span,
        promotion: promoted.map(|candidate| PendingPromotion {
            target: spec.target.clone(),
            candidate,
            sec_rev: promoted_sec_rev,
        }),
        gate,
    })
}

/// Re-resolve the pinned selector against the post-promotion bytes: the span the
/// fingerprint will cover, the promoted section's `sec_rev`, and the raw segment
/// array. A promotion widens the selector's node by the marker line, so the
/// pre-promotion span would hash bytes that are no longer the selector's.
///
/// All three come from one fact, so "the lock row describes the bytes that were
/// hashed" holds by construction.
///
/// # Errors
/// `pin_target_missing` when the selector no longer resolves after promotion.
fn post_promotion_facts(
    pinned_doc: &model::Document,
    target: &Path,
    selector: &wire::ReadSel,
) -> Result<(std::ops::Range<usize>, String, Vec<HpathSeg>), Box<ErrorBody>> {
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
    Ok((
        span_range(fresh.span),
        fresh.sec_rev.clone(),
        fresh.hpath.clone(),
    ))
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
        None => segments.iter().map(|s| s.h.clone()).collect(),
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
        // Segments, verbatim. `HpathSeg::n` is deliberately dropped: R4's array
        // is plain strings, and `model::selector::Selector::Heading` resolves
        // with `n: None`, which demands uniqueness — an address that turns
        // ambiguous later refuses loudly instead of silently landing on
        // whichever sibling the ordinal now points at.
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

/// The canonical read-face selector for a resolved fact: the anchor plane's id
/// when the fact is a block anchor, otherwise its structural heading address.
///
/// It is what a dewey ordinal canonicalizes to: an ordinal is positional and
/// invalidated by the next heading inserted above it, so carrying one into a
/// pin would record an address that means something else after any edit.
fn canonical_selector(fact: &wire_map::facts::ReadFact) -> wire::ReadSel {
    match &fact.anchor {
        Some(id) => wire::ReadSel::Anchor { anchor: id.clone() },
        None => wire::ReadSel::Hpath {
            hpath: fact.hpath.clone(),
        },
    }
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
/// `bad_request` when the title yields no id ([`slug_id`]), or when the derived
/// slug is already taken by another node — refused rather than uniquified, so the
/// id stays a function of the title alone (D15).
fn decide_anchor(
    target_doc: &model::Document,
    target: &Path,
    fact_anchor: Option<&str>,
    slot: usize,
    title: &str,
    selector: &str,
) -> Result<(String, bool), Box<ErrorBody>> {
    if let Some(id) = fact_anchor
        .map(ToOwned::to_owned)
        .or_else(|| anchor_on_line(target_doc, slot))
    {
        return Ok((id, false));
    }
    let slug = slug_id(title)?;
    if !matches!(
        model::resolve(target_doc, &model::Ref::Anchor(slug.clone())),
        Err(model::ResolveError::NotFound)
    ) {
        return Err(bad_request(format!(
            "the slug id ^{slug} derived from \"{selector}\" is already taken by \
             another node in {} — give that node's own ^id as the selector instead",
            target.0
        )));
    }
    Ok((slug, true))
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

/// The read-mint gate (D16 + D6), the whole refusal ladder in one place.
///
/// `actor == None` (or blank) is the bare CLI: local-operator-trusted, the gate
/// is bypassed exactly as `mrd put` bypasses the host's authz. A real session
/// actor must carry a receipt for this path and this selector — matching is
/// exact, so reading a parent section does not authorize pinning a child, and
/// only a sections-mode read mints at all. A held receipt is then re-checked
/// against the live `sec_rev` under the caller's flock: a receipt is not a
/// lease.
///
/// # Errors
/// `read_mint_required` (no covering receipt, or a host with no session layer),
/// `write_conflict` (the receipt covers a rev the target no longer carries).
fn read_mint_gate(
    store: Option<&receipt::read_mint::ReadMintStore>,
    actor: Option<&str>,
    target: &Path,
    selector: &wire::ReadSel,
    live_sec_rev: &str,
) -> Result<(), Box<ErrorBody>> {
    let Some(actor) = crate::read::mint_actor(actor) else {
        return Ok(());
    };
    let asked = selector.display();
    let Some(store) = store else {
        return Err(read_mint_required(
            target,
            format!(
                "pin of {}#{asked} refused: this host holds no read-receipt ledger, so it \
                 cannot know that actor {actor} read the content (a ledgerless in-process \
                 caller has no session — pin through the resident daemon, or from the \
                 local CLI)",
                target.0
            ),
        ));
    };
    let Some(receipt) = store.lookup(actor, &target.0, selector) else {
        // Name the cause the gate can tell apart: a receipt held under another
        // identity means the caller's session id rotated; no receipt at all
        // means the selector was never read, or the mint evaporated. Both end
        // at the same one-round-trip fix.
        let rotated = store.any_actor_read(&target.0, selector);
        let cause = if rotated {
            "this session holds a receipt for that selector under a DIFFERENT identity, so \
             yours rotated (a fork, a resume, a /clear mints under a new id)"
        } else {
            "no identity in this session has read it — either it was never read, or the mint \
             evaporated (receipts are memory-only, so a daemon restart or an idle reap clears \
             them)"
        };
        return Err(read_mint_required(
            target,
            format!(
                "pin of {}#{asked} refused: actor {actor} holds no read receipt for that \
                 selector — you cannot attest content that was never in your context. Cause: \
                 {cause}. Fix, either way, in one round trip: re-read {}#{asked} (mode \
                 sections, that exact selector) as {actor}, then pin again.",
                target.0, target.0
            ),
        ));
    };
    if receipt.sec_rev != live_sec_rev {
        let mut e = ErrorBody::new(ErrorCode::WriteConflict);
        e.path = Some(target.clone());
        e.expected = Some(NodeRev(receipt.sec_rev.clone()));
        e.actual = Some(NodeRev(live_sec_rev.to_owned()));
        e.message = Some(format!(
            "pin of {}#{asked} refused: the receipt covers rev {} but the section now carries \
             {live_sec_rev} — re-read the selector (that re-mints) and pin again",
            target.0, receipt.sec_rev
        ));
        return Err(Box::new(e));
    }
    Ok(())
}

/// `read_mint_required` (D16): a session actor pinning content no receipt in
/// this session covers. Fix class — read the exact selector, then pin.
fn read_mint_required(target: &Path, message: String) -> Box<ErrorBody> {
    let mut e = ErrorBody::new(ErrorCode::ReadMintRequired);
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
    fn walk(node: &model::Node, line_start: usize) -> Option<String> {
        if let model::NodeKind::Anchor { name } = &node.kind
            && node.span.start == line_start
        {
            return Some(name.clone());
        }
        node.children.iter().find_map(|c| walk(c, line_start))
    }
    walk(&doc.root, line_start)
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
    // page pinning a section of itself that would contain the block pins bytes
    // this write is about to change — the claim could never be green. Refuse
    // rather than mint a permanently-red pin. Touching counts, not just
    // overlap: a fresh block is an EOF insert and a section running to EOF
    // absorbs it (`edit.span.start == pin.span.end`).
    if pin.fact.target.0 == pinning_path.0
        && !pin.span.is_empty()
        && edit.span.start <= pin.span.end
        && edit.span.end >= pin.span.start
    {
        return Err(bad_request(format!(
            "refused: the meridian-lock block lands INSIDE \"{}\", the very section \
             being pinned, so the pin could never verify green (lock-is-content, #8 §5) \
             — pin a section that does not extend to the page's end, or pin from \
             another page",
            pin.fact.selector.display()
        )));
    }
    Ok((edit, lock::render(&lock)))
}

/// The `meridian-lock` block's byte form and its placement law, in one place —
/// shared by the pin path and [`lock_write`] so the two cannot drift.
///
/// An existing block is replaced across its exact fence-to-fence span. A fresh
/// block is birthed at EOF, separated from existing content by exactly one blank
/// line, and the file ends with one terminator — `lock::render` emits no
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
    let sep = if raw.is_empty() || raw.ends_with("\n\n") {
        ""
    } else if raw.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    (
        model::EngineEdit {
            span: raw.len()..raw.len(),
            text: format!("{sep}{block}\n"),
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

/// The workspace-relative respelling of an ABSOLUTE spelling that lies inside
/// `root`, or `None` when no respelling exists (relative violations, paths
/// outside the root). Teaching only — admission stays lexical (`addr::confined`).
/// Canonicalizes to survive symlinked prefixes (`/tmp` vs `/private/tmp`); a
/// missing leaf canonicalizes through its parent so a write to a not-yet-born
/// inside path still gets its respelling.
///
/// Public because both doors teach it: the write door's [`path_confined`]
/// here, and the read door's `bad_path` face at the CLI (dogfood NEW-A —
/// one computation, so the two doors cannot train opposite habits).
#[must_use]
pub fn relative_respelling(root: &fs::WorkspaceRoot, path: &str) -> Option<String> {
    if !path.starts_with('/') {
        return None;
    }
    let p = std::path::Path::new(path);
    let canonical = std::fs::canonicalize(p).ok().or_else(|| {
        let parent = std::fs::canonicalize(p.parent()?).ok()?;
        Some(parent.join(p.file_name()?))
    })?;
    let rel = canonical.strip_prefix(&root.0).ok()?.to_str()?;
    (!rel.is_empty()).then(|| rel.to_owned())
}

/// The §5.1 world guard, shared by `create`/`remove`: refuse `root_mismatch` if
/// a supplied `if_root` no longer matches the ambient root (the plan is stale).
fn world_guard(if_root: Option<&Root>, root_before: &Root) -> Result<(), Box<ErrorBody>> {
    if let Some(expected) = if_root
        && *expected != *root_before
    {
        let mut e = ErrorBody::new(ErrorCode::RootMismatch);
        e.expected = Some(NodeRev(expected.0.clone()));
        e.actual = Some(NodeRev(root_before.0.clone()));
        return Err(Box::new(e));
    }
    Ok(())
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
         page. WHAT TO DO INSTEAD: the block is birthed at the page's END, so a whole-section \
         rewrite of the LAST section deletes it — write that section with `put at:end` or an \
         append, or rewrite a section that does not hold the block, and the claims survive \
         untouched. Retiring a claim on purpose needs an unpin verb, which does not exist yet \
         (stage 3) — until it does, remove the block by hand and re-mint",
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
/// The uniform `MultiLineValue` refusal, in the words BOTH value-plane write
/// doors speak (wire-contract § A.6.3a). `set_property` already named the key
/// and taught the body-section escape while the upsert door said only that the
/// value must be single-line — one law refused in two dialects is two laws to
/// the callers who meet it, and recovery quality became a function of which
/// door the caller entered.
fn multi_line_value_refusal(key: &str) -> String {
    format!(
        "property value for \"{key}\" contains a newline — frontmatter values are \
         single-line in v1; put multi-line content in a body section"
    )
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
            edit: match &edit.edit {
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
                        policy::defs::yaml_preserve_or_encode(stored_line, text).map_err(|_| {
                            bad_request(multi_line_value_refusal(
                                upsert_key.as_deref().unwrap_or(""),
                            ))
                        })?
                    } else {
                        text.clone()
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
fn simulate_armed_edits(
    after_doc: &model::Document,
    edits: &[Edit],
    before_facts: &[model::Target],
) -> Result<Vec<ArmedEdit>, Box<ErrorBody>> {
    let mut armed_edits = Vec::with_capacity(edits.len());
    for (edit, before) in edits.iter().zip(before_facts) {
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

/// The receipt append for a real commit: render the line (§6.1), honor the
/// parent-dir obligation (fs does not mkdir — the production caller does, real
/// commits only), and fold the append at the receipt file's EOF.
fn receipt_input(
    root: &fs::WorkspaceRoot,
    args: &SpliceArgs,
    edits: &[Edit],
    root_before: &Root,
    armed_edits: &[ArmedEdit],
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
        edits: edits
            .iter()
            .zip(armed_edits)
            .map(|(req, armed)| receipt::EditFact {
                target: &req.target,
                shape: &req.edit,
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
    e.message = Some(format!(
        "batch edits must rewrite disjoint bytes (§4.4): {} rewrite overlapping \
         regions of the file — re-anchor one so they touch different bytes, or \
         split them into separate splice calls",
        names.join(" and ")
    ));
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
    let root_before = ambient(root)?;

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

    // Post-state + the advanced root.
    let after_content = fs::load(root, FsPath::new(&req.content_path)).map_err(CommitError::Io)?;
    let after_receipt = match &req.receipt {
        Some((rp, _)) => load_optional(root, rp)?,
        None => None,
    };
    let root_after = ambient(root)?;

    // Change facts → wire projection, in §7.1 print order: content file first,
    // then the receipt file.
    let mut files = Vec::new();
    if let Some(fd) = model::delta::file_delta(Some(&before_content), Some(&after_content)) {
        files.push(wire_map::project_file_delta(&req.content_path, &fd));
    }
    if let Some((rp, _)) = &req.receipt
        && let Some(fd) = model::delta::file_delta(before_receipt.as_ref(), after_receipt.as_ref())
    {
        files.push(wire_map::project_file_delta(rp, &fd));
    }

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

/// The ambient root as a [`CommitError`] (the commit seam's envelope shape).
fn ambient(root: &fs::WorkspaceRoot) -> Result<Root, CommitError> {
    ambient_root(root).map_err(CommitError::Env)
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
    use wire::{Edit, EditShape, ErrorCode, FileChange, HpathSeg, NodeRev, Path, Recovery, SecRef};

    use super::{CreateArgs, RemoveArgs, SpliceArgs, ambient_root, create, remove, splice};

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
        }
    }

    fn remove_args(path: &str, if_file_rev: &str) -> RemoveArgs {
        RemoveArgs {
            id: None,
            path: Path(path.into()),
            if_file_rev: NodeRev(if_file_rev.into()),
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

        assert_eq!(
            std::fs::read(dir.path().join("notes/new.md")).unwrap(),
            b"# First\n",
            "the occupant is untouched — the birth refused before any byte"
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
