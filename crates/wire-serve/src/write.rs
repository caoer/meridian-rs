//! The shared WRITE choke-point — `splice → commit` — lifted out of the sidecar
//! so the resident registry daemon and the per-workspace sidecar commit through
//! ONE implementation (arch map A6/W1: "lift, don't duplicate").
//!
//! # The single choke-point (decision 0002 W1)
//! [`splice`] is THE one function the write path flows through: flock → load →
//! §5.1 world guard → validate → build the post-batch doc ONCE → I4
//! def-conformance (S4a: refuse) → evaluate verdicts → armed gate (refuse) → dry
//! short-circuit → (real) render the receipt + [`commit_batch`] (the D4 seam:
//! validate → `fs::apply_batch` → one Delta). Every rung from the flock down
//! reads the SAME loaded pre-image and the SAME post-batch doc, which is what
//! makes a verdict binding on the bytes it authorized. Both hosts call `splice`;
//! a later per-session rule-evaluation hook carves in at the ONE marked verdict
//! site, never a rewrite. This unit builds NO hook, NO rule types — a BARE
//! meridian-fs commit (Advisor R3 ruling); the resident rule-engine placement is
//! reserved.
//!
//! # Verdicts are the frozen §11.1 surface, not rule machinery
//! [`evaluate_verdicts`] runs whatever admitted `policy::CompiledRuleset`s the
//! CALLER hands in over the post-batch doc — the sidecar admits packs
//! (`sidecar::admit`), the resident daemon hands `&[]` (no pack-admission surface
//! yet; that is a reserved, later unit). Empty rulesets ⇒ `verdicts: []`.
//!
//! # The delta ring lives with the caller
//! [`commit_batch`] assembles ONE `DeltaFrame` at the single §7.3 constructor
//! ([`assemble_delta`]) and RETURNS it; it does not hold or advance a ring. The
//! sidecar advances its per-epoch ring with the returned frame; the resident
//! daemon has no ring yet (P2 watcher) and discards it — the committed disk bytes
//! are the durable fact, and the next read's `warm_or_build` rebuilds from them.

use std::io::ErrorKind;
use std::path::Path as FsPath;

use wire::{
    Armed, ArmedEdit, Delta, DeltaFile, DeltaFrame, Edit, EditShape, ErrorBody, ErrorCode,
    HpathSeg, NodeRev, Path, PutAt, ReceiptAddr, ReceiptFact, ResponseBody, Root, SecRef, Severity,
    Span, Verdict,
};

use crate::read::{ambiguous, to_model_ref};
use crate::{ambient_root, bad_request, load_doc};

/// One splice request's decoded fields, bundled (the choke-point reads them as a
/// unit; `id` rides only into the receipt line — §6.1). Both hosts build this
/// from the decoded `wire::Op::Splice`, then call [`splice`].
#[derive(Debug, Clone)]
pub struct SpliceArgs {
    /// The frame correlation token — recorded into the receipt line (§6.1); no
    /// other field reads it.
    pub id: Option<u64>,
    /// The content file the batch edits.
    pub path: Path,
    /// U10: WHICH DOOR this splice arrived through, stated by the caller and
    /// never sniffed. Every `Wire` door enforces fingerprint-or-force —
    /// bookkeeping, not a trust class. `InProcess` is not a wire door, so the
    /// ruling does not reach it. No default: a door states its side or does not
    /// compile.
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
    /// U4.3 `--force`: escape an armed binding-break / block refusal. The skip is
    /// RENDERED — a `forced:`-marked verdict naming the bypassed rule (P15; the
    /// permanent force-row it also used to write died with the journal). The
    /// INDEX-integrity floor is NOT escaped (security F2). Ordinary writes: false.
    pub force: bool,
    /// The requested edits, 1:1 with the armed edits in the response.
    pub edits: Vec<Edit>,
    /// M1 U8b `splice.plan_edits`: the plan-level batch (mutually exclusive
    /// with `edits`, decode-enforced). Lowered to native edits at the intake
    /// below (`crate::plan::lower` — byte-faithful to the deleted Go arms);
    /// armed facts align 1:1 with the LOWERED edits. Empty = the native form.
    pub plan_edits: Vec<wire::PlanEdit>,
    /// Stage-2 S7 `splice.pin` (D7): the pin riding this splice. `args.path` is
    /// the PINNING page — the page whose `meridian-lock` block records the
    /// claim, so the lock write IS a content edit on this splice's own file and
    /// lands in the same [`commit_batch`] rename. A pin-only splice carries no
    /// `edits`. The pin's actor is `self.actor` and nothing else (D13).
    pub pin: Option<wire::PinSpec>,
}

/// The outcome of the write choke-point: the wire `Splice` response body plus,
/// on a REAL commit, the one emitted `DeltaFrame`. `committed` is `None` on a dry
/// run (nothing landed). The CALLER decides the frame's fate — the sidecar
/// advances its epoch ring with it; the resident daemon discards it (no ring
/// yet, P2).
#[derive(Debug)]
pub struct SpliceOutcome {
    /// The `wire::ResponseBody::Splice` body to return to the client.
    pub body: ResponseBody,
    /// The emitted delta, present only on a real commit (absent on dry).
    pub committed: Option<DeltaFrame>,
}

/// **THE single `splice → commit` choke-point** (decision 0002 W1): the whole
/// write path flows through here so a later per-session rule-evaluation hook is a
/// carve-in at the ONE verdict site, not a rewrite. Strict-decoded edits →
/// §5.1-ordered validation → the D4 commit seam ([`commit_batch`]: validate →
/// `fs::apply_batch` → Delta emission) — one exchange, one reparse, one root
/// advance, one Delta. `dry: true` runs everything except disk: same response
/// shape, `root_after: null`, no receipt written, no ring frame, no mkdir (zero
/// disk effects means zero).
///
/// `seq` is the caller's current epoch seq — the emitted frame's `seq` is
/// `seq + 1` (the sidecar passes its ring's `seq()`; the resident daemon passes
/// `0`, having no ring). `rulesets` are the admitted packs whose §11.1 findings
/// ride the `verdicts` field; `&[]` ⇒ `verdicts: []` (the BARE commit).
///
/// The production `apply_batch` caller obligations (F4 seam memo) live HERE:
/// receipt pairing rides `CommitRequest` (fs re-checks fail-loud), the receipt
/// line renders via `crates/receipt` and folds in pre-validation (§6.1 — same
/// sealed batch, ONE root advance), and the receipt parent dir is created on REAL
/// commits only (fs does not mkdir).
///
/// # Errors
/// A typed validation refusal (§5.2 failure split) mapped to its wire frame, an
/// ambient-root/domain failure, or an I/O error — in every error case no Delta
/// exists and nothing was committed, with ONE named exception that is a disk
/// fault rather than a refusal: a pin's anchor promotion is a second inode and
/// therefore a second rename (residual G3), so an I/O failure in the commit
/// AFTER the promotion's own rename lands can leave that marker behind. It is
/// fingerprint-neutral and idempotently reused by the next pin. Every REFUSAL
/// rung, including all of the pin's, runs before that rename.
// THE single write choke-point (decision 0002 W1): its length is the deliberate
// one-linear-flow this crate is built around; the U4.2 gate mount grew it past
// the 100-line lint, but splitting the flow would obscure it.
#[allow(clippy::too_many_lines)]
pub fn splice(
    root: &fs::WorkspaceRoot,
    seq: u64,
    args: &SpliceArgs,
    rulesets: &[policy::CompiledRuleset],
    mints: Option<&receipt::read_mint::ReadMintStore>,
) -> Result<SpliceOutcome, Box<ErrorBody>> {
    // U11 — WORKSPACE-ROOT CONFINEMENT, the guard this door has never had.
    // `create`, `remove`, `lock_write` and `mint_pin` all call `path_confined`;
    // `splice` — the PRIMARY write op — did not. `fs::load` joins the caller's
    // path onto the root (`root.0.join(rel_path)`), and `Path::join` with an
    // ABSOLUTE path discards the root outright, so an absolute or `..`-bearing
    // splice path read and wrote OUTSIDE the workspace. Measured before the fix:
    // both landed a real `Modified` delta on an out-of-workspace file, with
    // `root_before == root_after` — the victim is outside the hash domain, so
    // the world root never advanced and the write was invisible to the ledger.
    //
    // `mrd put` is what makes it reachable from a shell: it bypasses the strict
    // decode entirely and builds `SpliceArgs` straight from raw argv.
    //
    // FIRST, before the flock and before `load_doc`: a refusal must not depend
    // on having already touched the path it refuses.
    path_confined(&args.path)?;

    // D9 (xproc-race fix): the cross-process write flock, held across the
    // WHOLE critical section — read#1 below, validate, gate, the commit's
    // read#2 → verify → renames — so cooperating
    // meridian writers (sidecar, resident daemon, mrd) serialize instead of
    // interleaving read→rename. Dry runs take it too: a rehearsal refuses
    // `workspace_busy` exactly where the real write would. Released on drop.
    let _write_lock = acquire_write_lock(root)?;

    let mut doc = load_doc(root, &args.path)?;
    let mut root_before = ambient_root(root)?;

    // §5.1 order: the world guard FIRST — checked here so a stale plan
    // refuses before any per-target resolution can answer for it, and (S7)
    // before this splice's own promotion can advance the root it guards on.
    world_guard(args.if_root.as_ref(), &root_before)?;

    // S7 the PIN prologue (D7), ordered exactly as plan §6: gate → fingerprint
    // + blob → anchor promotion. It runs INSIDE the flock this splice already
    // holds, so the receipt's rev-recheck reads the same pre-image the batch
    // will validate against, and the promotion needs no second flock.
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
    // The promotion's own gate ran at mint time — it must refuse before any byte
    // is written, and on the dry path too (§4.4: a rehearsal refuses exactly
    // where the real write does). Its advisory findings and forced skips ride
    // this response, merged below with the batch's own.
    let mut pin_gate = pin
        .as_mut()
        .map(|p| std::mem::take(&mut p.gate))
        .unwrap_or_default();
    // The promotion is COMPUTED by the prologue and LANDS far below, after the
    // last rung that can refuse (R25, finding 12): it is the ONE write that does
    // not ride the batch (residual G3: two inodes are two renames), so ordering
    // it last is what makes a refused pin leave every file byte-unchanged.
    //
    // Nothing has moved on disk yet, so `root_before` and the pinning page's
    // pre-image both still stand — with one exception: when the promotion's
    // target IS the pinning page, those promoted bytes are the pre-image this
    // batch must be composed against, because they are what disk will carry when
    // `commit_batch` reads it back.
    if let Some(p) = pin.as_ref().and_then(|p| p.promotion.as_ref())
        && same_file(root, &p.target, &args.path)
    {
        doc = build_doc(&args.path, p.candidate.raw());
    }
    // The lock block is composed against the POST-promotion pinning page and
    // rides the batch as the one engine-minted span edit (`model::EngineEdit`):
    // a fenced block is unaddressable by the §2.1 ref grammar, and the engine is
    // its sole writer (#8 §3). Riding here is what puts content+lock in ONE
    // `commit_batch` — one flock, one rename — instead of a second flocked
    // `lock_write` call, which would self-refuse `workspace_busy` (the flock is
    // non-reentrant per open-file-description).
    // `pin_block` is the canonical block this call MINTED — the one byte form
    // the artifact guard below admits as a lock change (R25).
    let (pin_engine, pin_block) = match &pin {
        Some(p) => {
            let (edit, block) = lock_engine_edit(&doc, &args.path, p)?;
            (Some(edit), Some(block))
        }
        None => (None, None),
    };

    // M1 U8b: the plan-lowering intake — plan_edits become native edits HERE
    // (under the flock, against the just-loaded pre-batch doc), then the whole
    // path below runs unchanged on the lowered batch. Target-class refusals
    // (the deleted Go arms' teachings) fire before any per-target resolution.
    //
    // Stage-2 S10, re-grained by advisor R25: payloads ride through here
    // VERBATIM. The `@fp` strip is no longer a walk of named payload fields
    // (that list missed `create.title` and could not see a token two fields
    // compose between them) — it runs once, at DOCUMENT grain, over the
    // candidate this splice is about to commit ([`strip_fp_candidate`] below).
    let mut effective_edits = if args.plan_edits.is_empty() {
        args.edits.clone()
    } else {
        crate::plan::lower(&doc, &args.plan_edits)?
    };

    // U10 — FINGERPRINT-OR-FORCE, mounted HERE and nowhere else (P2, revised).
    // Post-lowering is the point both write faces have already reached: a guard
    // at plan lowering would be MCP-only, and native `edits` would walk around it
    // untouched (the field-rename bypass, adversarial finding 1.1/1.2). Per-edit
    // by design, so an empty batch — `mrd pin` — passes through with nothing to
    // demand. The refusal is SEMANTIC: the frame decoded fine and the WRITE is
    // refused, which is what leaves decision 007's schema half intact. See
    // `crate::guard`.
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
        // The CLIENT's world guard was honored above, against the root it
        // actually pinned. The batch re-guards on the CURRENT root instead of
        // that value: a pin's own rev-neutral promotion advances the root, and
        // re-comparing the client's pre-promotion token here would self-refuse
        // `root_mismatch` on this splice's own write. Nothing else can move the
        // root under the flock, so the guard keeps its meaning — and an
        // unguarded request stays unguarded.
        if_root: args
            .if_root
            .as_ref()
            .map(|_| model::MerkleRoot(root_before.0.clone())),
        edits: model_edits,
        engine: pin_engine,
    };

    // Validate + simulate the after state in memory (the §4.4 one-reparse
    // law's dry twin): armed AFTER facts come from a real parse of the
    // simulated bytes — computed, never arithmetic-shifted.
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
                effective_edits,
                &doc,
                &before_facts,
                &args.path,
            ));
        }
    };

    // Build the post-batch document state ONCE, shared by BOTH the armed AFTER
    // facts and the verdicts, for BOTH the dry and real paths — the single point
    // that makes the dry twin incapable of diverging from the real one (§4.4
    // one-reparse law; advisor Ruling 2). The real commit writes exactly these
    // bytes, so evaluating this simulated doc is evaluating the committed doc.
    let mut after_doc = build_after_doc(&doc, &sealed, &args.path);

    // Stage-2 S10 re-grained (advisor R25, structural fix 2): the `@fp` strip
    // runs HERE, over the CANDIDATE — one grammar, one grain. It rewrites the
    // batch's payloads (so `commit_batch`'s re-validation lands the same bytes
    // judged here) and leaves a document-grain assertion behind it: any token
    // still standing in a claim-link position refuses LOUD instead of landing
    // silently. A door that grows a new payload field is covered by
    // construction; a door that reaches these bytes another way is refused.
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

    // U12 — the STORED-FORM translation, at the candidate (D9). Ordered AFTER
    // the `@fp` strip on purpose: the strip has already removed every decoration
    // this write introduces, so an address reaching the stored plane with one
    // still attached is a token the strip could not place — refused there, not
    // silently carried into a URI here. Like the strip it rewrites payloads,
    // re-validates and re-builds, and leaves the artifact guard behind it.
    translate_stored_candidate(
        &doc,
        &root_before,
        &args.path,
        &before_facts,
        &mut batch,
        &mut sealed,
        &mut after_doc,
    )?;

    // Advisor R25, structural fix 1 — GUARD THE ARTIFACT, not the verb. The
    // read-mint gate guards the `splice.pin` door; the `meridian-lock` bytes it
    // protects are ordinary page text every put shape can reach. This rung reads
    // the SAME candidate every rung below reads and refuses any lock-byte change
    // that is not exactly the block THIS call minted — so an actor with no
    // receipt cannot write a pin through native `edits`, a lowered `plan_edits`
    // batch, or any put shape added later. Ordered before the ladder and the
    // advisory verdicts (a forged attestation is not a policy question) and above
    // the dry short-circuit, so a rehearsal refuses exactly where the real write
    // does.
    lock_artifact_guard(&doc, after_doc.document(), pin_block.as_deref(), &args.path)?;

    let armed_edits = simulate_armed_edits(after_doc.document(), effective_edits, &before_facts)?;

    // S4a/D4 (TOCTOU close): the I4 def-conformance verdict runs HERE — inside
    // the D9 flock, over the very `after_doc` this splice is about to write,
    // against the `doc` the flock loaded. The standalone `check_write` op stays
    // as the host's pre-flight, but the verdict that AUTHORIZES bytes is this
    // one: a foreign writer landing between a host's check and its apply used to
    // split the two (the check judged bytes the write no longer wrote); it can
    // no longer, because the ladder judges the same pre-image the batch
    // validated against. Ordered BEFORE the armed gate so the refusal a host
    // used to see from its pre-flight stays the first one it sees, and before
    // the dry short-circuit so a rehearsal refuses exactly where the real write
    // does. Repairs/`forced` stay the standalone op's channel — the internalized
    // run only GATES, it never mutates the sealed batch.
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

    // ADVISORY §11.1 verdicts from any caller packs (W1) — never a decision; the
    // caller-supplied packs do not gate, only the armed law below does.
    let mut verdicts = evaluate_verdicts(rulesets, after_doc.document());

    // U4.2/U4.3: the armed-plane GATE — after CAS, before bytes land, both writer
    // paths. Reads the workspace's OWN armed law (never caller packs) and REFUSES
    // here (`?`) before the dry short-circuit; never-armed is a no-op. U4.3:
    // `args.force` escapes a binding-break / block refusal (the skip is rendered
    // here as a `forced:`-marked verdict, P15); the INDEX-integrity floor never
    // escapes.
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
    // U10/P15: a forced write NAMES the planes it wrote past — on the rendered
    // surface, which is where a caller reads it (the journal is dead by ruling).
    // Empty for every unforced write, so an ordinary response is unchanged.
    verdicts.extend(crate::guard::bypass_verdicts(&bypassed, &doc, &args.path));

    // Dry short-circuit (§4.4 batch law): everything except disk — and
    // therefore no receipt, no root advance, no Delta, no mkdir.
    if args.dry {
        return Ok(SpliceOutcome {
            body: ResponseBody::Splice {
                armed: Armed {
                    path: args.path.clone(),
                    // Dry writes nothing, so there is no post-write file rev to
                    // report (mirrors `root_after: None` at file grain).
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
                // A dry pin reports the plan it rehearsed; nothing was written,
                // so `promoted` reads as what a real run WOULD do.
                pin: pin.map(|p| Box::new(p.fact)),
            },
            committed: None,
        });
    }

    // THE PROMOTION LANDS HERE — genuinely last (R25, finding 12). Everything
    // above it can still refuse; below it there is only the commit's own I/O. So
    // "a refused pin leaves the target byte-unchanged" is true by ORDERING, not
    // by cleanup: the read-mint gate, the slug collision, the ref grammar, the
    // artifact guard, the `@fp` law, def-conformance and BOTH armed gates have
    // all already answered. The unchanged residual (G3) is a crash between this
    // rename and the commit's: a fingerprint-neutral marker the next pin reuses.
    //
    // The write is rev-NEUTRAL — norm-v2 removes the marker line whole, so the
    // target's fingerprint cannot move and no other page pinning that target
    // reddens. That exactness is what permits promoting into a possibly-unowned
    // target at all (D14), and it is asserted, not assumed (`s2fix_promotion`).
    if let Some(minted) = pin.as_ref()
        && let Some(p) = minted.promotion.as_ref()
    {
        fs::replace_file(root, FsPath::new(&p.target.0), &p.candidate)
            .map_err(|e| io_to_wire(&e))?;
        // D16: refresh the actor's receipt to the rev THIS engine write created.
        // The promotion moved the section's `sec_rev` (a rev is over RAW bytes,
        // and a line was inserted) without moving one byte of what the actor
        // READ, so leaving the old rev would fail the actor's own gate on its
        // next pin. Only this path refreshes, and only for a receipt that
        // already passed the gate at mint time; a foreign content change still
        // refuses.
        if let (Some(store), Some(actor)) = (mints, crate::read::mint_actor(args.actor.as_deref()))
        {
            store.mint(actor, &p.target.0, &minted.fact.selector, &p.sec_rev);
        }
        // The promotion moved the corpus root — this splice's OWN write. Re-read
        // it so the receipt records the root the commit reports, and re-guard the
        // batch on the current value: re-comparing the client's pre-promotion
        // token would self-refuse `root_mismatch` on our own write. The client's
        // guard was already honored above, against the root it actually pinned,
        // and nothing else can move the root under the flock.
        root_before = ambient_root(root)?;
        batch.if_root = args
            .if_root
            .as_ref()
            .map(|_| model::MerkleRoot(root_before.0.clone()));
    }

    // REAL commit: render the receipt line (facts about what is being
    // ARMED — §6.1), fold the append, honor the parent-dir obligation,
    // then drive the D4 commit seam (validate → apply → emit).
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
    // The batch moves into the commit seam, so the edits the reaction reports on
    // are captured while they are still here. They describe what LANDED — the
    // feeder below runs only if `commit_batch` succeeds.
    let landed_edits = batch.edits.clone();
    let mut frame = commit_batch(
        root,
        seq,
        &CommitRequest {
            content_path: args.path.0.clone(),
            batch,
            receipt: receipt_input,
            actor: args.actor.clone(),
            now: args.now.clone(),
        },
    )
    .map_err(|e| match e {
        CommitError::Refused(v) => {
            verdict_to_wire(&v, effective_edits, &doc, &before_facts, &args.path)
        }
        CommitError::Env(err) => err,
        CommitError::Io(err) => commit_io_to_wire(&err, &args.path),
    })?;

    // C3 — REACTION MODE. Evaluated only after the batch landed, from the state
    // pair this path already holds, and it can neither refuse nor mutate what
    // landed (design §4.4: the notify path attaches at reaction mode, never at the
    // gate). The outcome rides two carriers: this seq's frame, which the host
    // flushes to subscribers, and the caller's own `armed` feedback below.
    //
    // A fault means "emit no reaction", never "fail the write" — the write is
    // already on disk, and letting a reaction turn it into an error would hand a
    // hook exactly the veto the ruling denies it. The fault is not DROPPED for that,
    // though: it rides the frame as a `wire::EffectFinding::ArmedFault`, this host's
    // channel onto the one artifact-fault surface. A `.unwrap_or_default()` stood
    // here and read every artifact fault as "nothing to react to".
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
                // The post-write whole-file rev, read from the SAME simulated
                // after-doc as the armed edits (§4.4 one-reparse law): the real
                // commit writes exactly these bytes, so this equals the
                // committed file's rev and a subsequent `toc`'s `file_rev` — no
                // drift. Latency only; correctness stays `root_after`.
                file_rev_after: Some(NodeRev(after_doc.document().root.node_rev.0.clone())),
                edits: armed_edits,
                // Constraint 8's feedback law: what this write ARMED, stated
                // synchronously to the caller that wrote it. It names matched
                // rules, their intents and each canonical receipt address — never
                // who gets notified, and never that anything was delivered. This
                // response is complete before the host flushes the frame above to
                // any subscriber, so no delivery can have happened yet.
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
    })
}

// ---------------------------------------------------------------------------
// Guarded create / remove — file birth and death (d2 §2.5 C3, U2.6)
// ---------------------------------------------------------------------------
//
// Birth and death join the strict writer as core write OPS inside the one write
// shape (design §2.5, §3): `create` under CAS `if_absent` + workspace-root;
// `remove` under CAS on the file's read rev (remove-what-you-read) +
// workspace-root. Both expose the
// change surface a gate evaluates at birth/death — `before = absent` (create) /
// `after = absent` (remove). This unit builds the OPS and that seam; the
// callers (put-class clients, the effects-domain workflow, `realise`'s apply
// plane) and the armed `gate()` mount are later units — the verdict seam here
// runs whatever rulesets the caller hands in (`&[]` ⇒ the BARE commit), exactly
// like `splice`.

/// One `create` request's fields — ONE `new` spec (a single path + body, never
/// a batch, d2 §2.5 C3). `actor`/`now` are recorded exactly as given (§9),
/// `if_root` is the optional §5.1 world guard, `dry` runs everything but disk.
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

/// Project a landed birth into its wire response body — ONE implementation both
/// hosts render through (A6 "lift, don't duplicate"), so the `create` frame
/// cannot drift between the per-workspace sidecar and the resident daemon.
///
/// `seq` rides from the emitted Delta, so it is absent on a dry run for the same
/// reason `root_after` is: a rehearsal emits no Delta.
/// `dry` is `Some(true)` only on a rehearsal — an ordinary birth serializes no
/// `dry` key, exactly like `splice`.
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

/// One `remove` request's fields (d2 §2.5 C3). `if_file_rev` is the rev the
/// caller READ — remove-what-you-read: the live file must still carry it, or the
/// death refuses citing the drift. `if_root`/`dry` mirror `create`.
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

/// **Guarded `create`** (d2 §2.5 C3): birth one file under CAS `if_absent` +
/// workspace-root, and emit the `created` change surface.
///
/// Order: path confinement → world guard (§5.1) → the
/// gate seam over the birth's after-state → the `if_absent` CAS at the disk edge
/// ([`fs::create_file`], the single source of the guard) → root advance → birth
/// Delta. `dry: true` runs everything except
/// disk and still refuses a would-be clobber.
///
/// # Errors
/// `bad_path` (escapes the workspace), `root_mismatch` (stale world guard),
/// `cas_mismatch` (the path is
/// occupied — taxonomy row 13, recovery `refresh`), or an I/O failure. In every
/// error case nothing was created.
pub fn create(
    root: &fs::WorkspaceRoot,
    seq: u64,
    args: &CreateArgs,
    rulesets: &[policy::CompiledRuleset],
) -> Result<CreateOutcome, Box<ErrorBody>> {
    let fs_path = FsPath::new(&args.path.0);
    path_confined(&args.path)?;

    // D9: births serialize on the same write flock as every meridian writer —
    // this also closes the `if_absent` check→rename window for cooperators.
    let _write_lock = acquire_write_lock(root)?;

    let root_before = ambient_root(root)?;
    world_guard(args.if_root.as_ref(), &root_before)?;

    // Stage-2 S10: a birth is a put too — and here the payload IS the candidate
    // document, so `strip_fp` over the whole body already runs at document grain
    // (advisor R25): one grammar, the same one `strip_fp_candidate` applies to a
    // splice. The rev the birth reports is therefore the rev of the bytes that
    // land, never of a decorated draft.
    let body = syntax::strip_fp(&args.body);

    // U12 — the stored-form translation at the BIRTH door (see
    // [`translate_stored_body`]).
    let body = translate_stored_body(body, &args.path)?;

    // The birth's after-state, built once from the body (path-stamped so the
    // gate sees it). Its whole-file rev is the born file's rev.
    let after_doc = model::candidate_of_body(&args.path.0, body.into_owned());

    // THE ARTIFACT GUARD (D9), live on the birth path: an agent-plane cross-root
    // address still standing refuses instead of landing bytes no reader can
    // follow. A birth has no pre-image, so `None`.
    stored_form_guard_lazy(None, &after_doc, &args.path)?;
    let file_rev_after = NodeRev(after_doc.document().root.node_rev.0.clone());

    // THE ASSERTION (R25), live on the birth path: a token still standing in a
    // claim-link position refuses instead of landing. A birth has no pre-image,
    // so "introduced" and "present" are the same set here.
    if !syntax::fp_removals(after_doc.raw()).is_empty() {
        return Err(bad_request(format!(
            "refused: an @fp claim token survived the document-grain strip in {} — the birth was \
             refused rather than landing a fingerprint claim the engine never minted",
            args.path.0
        )));
    }

    // The lock ARTIFACT guard (R25) at the birth door: a newborn page has no
    // pre-image and this op mints no pin, so ANY `meridian-lock` bytes in the
    // body are a claim nobody computed — `write::create` is one of the four
    // ungated doors the review names.
    lock_artifact_guard(
        &crate::gate::absent_doc(&args.path),
        after_doc.document(),
        None,
        &args.path,
    )?;

    // Advisory §11.1 findings from any caller packs (never a decision).
    let mut verdicts = evaluate_verdicts(rulesets, after_doc.document());

    // U4.2/U4.3: the armed-plane GATE over the birth's after-state — before=absent
    // (the `create` change surface). Blocks an armed refusal (convention or a
    // binding-break on the INDEX) before the file is born; a no-op on a
    // never-armed workspace. Guarded create carries no `--force`: there is no
    // forced-birth path, and the wire `create` op declares no `force` field for
    // exactly that reason (a key would advertise a bypass that does not exist).
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
    // actual:occupant-rev}` (row 13, recovery refresh — "re-read, it exists").
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

/// **Guarded `remove`** (d2 §2.5 C3): death of one file under CAS
/// remove-what-you-read + workspace-root, and emit the `deleted` change surface.
///
/// Order: path confinement → world guard (§5.1) → load
/// the live file (absent ⇒ `file_not_found`) → the remove-what-you-read CAS
/// (the live rev must equal `if_file_rev`, else refuse citing rev read vs found)
/// → the gate seam over the death's before-state → unlink → root advance →
/// death Delta.
///
/// # Errors
/// `bad_path`, `root_mismatch`,
/// `file_not_found` (nothing to remove), `cas_mismatch` (the file drifted from
/// the read rev — taxonomy row 14, recovery `refresh`), or an I/O failure. In
/// every error case nothing was removed.
pub fn remove(
    root: &fs::WorkspaceRoot,
    seq: u64,
    args: &RemoveArgs,
    rulesets: &[policy::CompiledRuleset],
) -> Result<RemoveOutcome, Box<ErrorBody>> {
    let fs_path = FsPath::new(&args.path.0);
    path_confined(&args.path)?;

    // D9: deaths serialize on the same write flock (read-rev CAS → unlink is
    // a critical section like any other write).
    let _write_lock = acquire_write_lock(root)?;

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

    // U4.2/U4.3: the armed-plane GATE over the death — after=absent (the `remove`
    // change surface); `before_doc` carries what is being removed. Blocks an
    // armed refusal before the unlink; the INDEX-integrity floor (U4.3) refuses a
    // remove of the INDEX or the once-armed marker here. No-op on never-armed.
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

// The old `^inputs` pin lock-write (superseded design, 22-01 module) lived
// here until M1 U12 removed it with `crates/pin`/`crates/attest`; the NEW
// lock method is the `meridian-lock` block below (U11, decision #8).

// ---------------------------------------------------------------------------
// Guarded `meridian-lock` write — the NEW lock method (M1 U11, decision #8)
// ---------------------------------------------------------------------------
//
// The lock is a machine-owned lockfile IN the page: a fenced `meridian-lock`
// block (versioned root object, `objects:`/`pins:` planes). The FORMAT —
// types, strict parse, canonical render, locate — lives in `crates/lock`;
// this is the ENGINE-SOLE-WRITER path (#8 §3): the one place lock bytes reach
// disk, mirroring the create/remove shape so the one-write-shape law holds.
// Callers hand in the TYPED `lock::Lock` — never raw block bytes — so a
// hand-forged block cannot enter through this door by construction. M1 lands
// format + this write path ONLY; the read-mint gate, drift verify-on-read,
// and vibe mode are stage 2 (nothing reads the lock to gate yet). Lock-is-
// content (#8 §5): the block sits inside the page span, so the page's
// fingerprint covers its lock and the write is one atomic file replace —
// content and lock land together or not at all.

/// One guarded `meridian-lock` write request (U11): upsert the page's ONE
/// lock block from a typed [`lock::Lock`]. `if_file_rev` is the page's
/// whole-file rev the caller read (write-what-you-read CAS); `if_root` the
/// §5.1 world guard; `dry` runs everything except disk.
#[derive(Debug, Clone)]
pub struct LockWriteArgs {
    /// Frame correlation token — recorded only.
    pub id: Option<u64>,
    /// The pinning page the `meridian-lock` block lives in (workspace-confined).
    pub path: Path,
    /// The typed lock object — the SOLE input form (engine-sole-writer #8 §3:
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
    /// `true` when the write BIRTHED the block (EOF append — no lock existed);
    /// `false` when it replaced the existing block in place.
    pub created: bool,
    pub dry: bool,
}

/// **Guarded `meridian-lock` write** (U11, decision #8): land the page's one
/// lock block — replace it in place when present, birth it at EOF when absent
/// — under CAS write-what-you-read + workspace-root + the D9 write flock, and
/// emit the `modified` change surface.
///
/// Order: path confinement → the write flock (D9) →
/// load the page → world guard (§5.1) → the write-what-you-read CAS → locate
/// the block (`lock::find` — MULTIPLE blocks refuse loud: sole-writer mints
/// exactly one, two is a hand-edit/corruption signal) → render via
/// `lock::render` (canonical bytes; terminators are THIS path's) → in-memory
/// splice → [`fs::replace_file`] (atomic; lock-is-content — one commit) →
/// root advance → Delta. `dry: true` runs everything except
/// disk.
///
/// # Placement law (fresh lock)
/// A birthed block appends at EOF — lockfile-at-bottom posture — separated
/// from existing content by exactly one blank line, and the file ends with
/// one terminator. A replaced block keeps its exact span (fence-to-fence).
///
/// # Errors
/// `bad_path`, `bad_request` (a malformed/duplicated
/// existing lock block — surfaced, never silently adopted), `workspace_busy`
/// (D9), `file_not_found` (the page must exist — a lock pins content),
/// `root_mismatch`, `cas_mismatch`, or an I/O failure. In every error case
/// nothing was written.
pub fn lock_write(
    root: &fs::WorkspaceRoot,
    seq: u64,
    args: &LockWriteArgs,
) -> Result<LockWriteOutcome, Box<ErrorBody>> {
    let fs_path = FsPath::new(&args.path.0);
    path_confined(&args.path)?;

    // D9: the lock write serializes on the same write flock as every writer.
    let _write_lock = acquire_write_lock(root)?;

    let before_doc = load_doc(root, &args.path)?;
    let file_rev_before = NodeRev(before_doc.root.node_rev.0.clone());

    let root_before = ambient_root(root)?;
    world_guard(args.if_root.as_ref(), &root_before)?;

    // write-what-you-read CAS: the page must still carry the rev the caller
    // read (drift means the lock's facts were computed against stale bytes).
    if args.if_file_rev != file_rev_before {
        return Err(cas_mismatch(&args.if_file_rev, &file_rev_before));
    }

    // Locate the ONE block (or the EOF birth point). `lock::find` fails loud
    // on duplicates and malformed YAML — a sole-writer page can only reach
    // that state by hand-editing, and adopting it would launder corruption.
    let raw = &before_doc.raw;
    // The block's bytes and its placement law, from the ONE owner the pin path
    // shares (`lock_block_splice`), then spliced in memory.
    let (edit, created) = lock_block_splice(&before_doc, locate_lock(&before_doc)?, &args.lock);
    let mut new_raw = String::with_capacity(raw.len() + edit.text.len());
    new_raw.push_str(&raw[..edit.span.start]);
    new_raw.push_str(&edit.text);
    new_raw.push_str(&raw[edit.span.end..]);
    let after_doc = model::candidate_of_body(&args.path.0, new_raw);
    // THE ARTIFACT GUARD (D9) at the lock door. The lock block's own `ref:` and
    // `objects:` keys are positions 3 and 4, where the translation is the
    // IDENTITY by ratified law — they stay in the canonical `root:` form, never
    // the URI. So this rung asserts the OTHER half: engine-composed lock bytes
    // introduce no agent-plane address into positions 1 or 2. A door proven only
    // by what it forbids would be satisfied by a door that writes nothing.
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
// The PIN prologue (stage-2 S7, D7/D13/D14/D15/D16)
// ---------------------------------------------------------------------------
//
// A pin is a Splice-SIBLING field, never its own op: the splice's `path` is the
// pinning page, so the lock write is a content edit on that page and rides the
// SAME `commit_batch` rename. What lives here is everything that must happen
// under the flock BEFORE the batch is sealed — the read-mint gate, the
// fingerprint + blob, and the anchor promotion — plus the lock composition.
//
// # The grain, and why the lock's `ref` is the canonical selector
// A pin's fingerprint is minted over EXACTLY the span its `ref` resolves to,
// because that is the span the verify plane recomputes
// (`model::selector::resolve_selector` → `fingerprint::verify_content`). So the
// `ref` carries the canonical selector the read receipt was keyed on — a
// `/`-joined sanitized heading path (`model::selector::Selector::parse`'s
// normative Heading form, resolving to the SECTION: ratified 07-22 §3 wants
// section-level pins so a change to section A reddens only A's dependents), or
// `^id` for a block-anchor row (the 07-23 leaf-selector ruling). The promoted
// `^slug` is deliberately NOT the `ref`: an anchor node's model span is its HOST
// LINE (`model::build`'s `anchor_host_span`), so an `^id` ref over a promoted
// heading would silently narrow a section pin to its heading text — every body
// edit would read as green. The slug is the STABLE HANDLE (D15) a claim link
// decorates and a later rename-heal relocates by, minted beside the claim.

/// The R4 lock row a pin will land, in the schema's own types — the STRUCTURE
/// half of a mint, minted beside the wire fact and never derived from it.
///
/// R4's three non-fingerprint fields, and the one rule each carries:
///
/// - `object` — the wiki link's INNER text. R4 demands the link resolve
///   *"EXACTLY as Obsidian does — 100% match or it is a critical trust
///   failure"*, so the engine writes the one spelling that always does: the
///   target's vault-relative path with its `.md` suffix removed. That is the
///   form `model::CorpusIndex::resolve_ref` matches by whole subpath suffix, so
///   it cannot collide with a same-named file in another folder the way a bare
///   basename can. Nothing here shortens the link for looks — a pin is a
///   machine claim, and the short form's ambiguity is exactly the trust failure
///   R4 names.
/// - `hash` — the target file's git blob oid, **never optional**. R4: *"if hash
///   is missing, we lost the explicit target meaning"*.
/// - `selector` — `path` XOR `properties`, arrays only.
#[derive(Debug)]
struct PinRow {
    object: String,
    hash: String,
    selector: lock::Selector,
}

/// What a pin minted, plus what it still OWES to disk. Nothing here has been
/// written: the prologue computes, the caller lands (see [`PendingPromotion`]).
#[derive(Debug)]
struct PinMint {
    /// The wire fact returned to the client.
    fact: wire::PinFact,
    /// The R4 lock row's own two structural fields, minted HERE and carried
    /// whole — never re-derived by splitting [`wire::PinFact::declared_ref`].
    ///
    /// `declared_ref` is the HUMAN/wire spelling and stays a `String` (criterion
    /// 7 freezes v2 byte-identity). R1.6 gives the machine surface arrays and no
    /// string address form, so the two are minted side by side from one source —
    /// the target's own read facts — and the joined form is never an input to
    /// the structural one. That is the whole reason this field exists rather
    /// than a `parse(&fact.declared_ref)` at the lock door.
    row: PinRow,
    /// The pinned selector's span in the target — the exact bytes the
    /// fingerprint covers, in the POST-promotion document (the promotion widens
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

/// An anchor promotion that has been DECIDED and not written: the exact bytes,
/// the page they belong to, and the receipt refresh the write owes.
///
/// Separating the decision from the write is the whole point (R25, finding 12).
/// The promotion touches a DIFFERENT file from the one the request names, so a
/// rung refusing after it would leave bytes in a page the caller never asked to
/// change — deterministically, and therefore unhealably. Held here, it lands
/// after the last such rung.
#[derive(Debug)]
struct PendingPromotion {
    /// The page the marker lands in — the pin's target, which may be the pinning
    /// page itself.
    target: Path,
    /// The sealed candidate to write (U31) — its bytes are the exact bytes
    /// that land, and also the pinning page's pre-image when the target IS the
    /// pinning page.
    candidate: model::CandidateDocument,
    /// The promoted section's `sec_rev` in those bytes — the D16 receipt refresh
    /// the write owes (a rev this ENGINE moved, invisible to the fingerprint).
    sec_rev: String,
}

/// The pin prologue: resolve the target, gate it against the read-mint ledger,
/// decide the stable anchor, and mint the fingerprint + blob oid over the bytes
/// the promotion WILL land.
///
/// **This function writes nothing.** It used to promote the anchor in the middle
/// of its own ladder, which put engine bytes in a page the request does not name
/// before the rungs that can still refuse had run — deterministically, so unlike
/// the accepted G3 crash orphan it never healed (R25, finding 12). The promotion
/// now travels back as a [`PendingPromotion`] and the caller lands it after its
/// last refusal rung. Two consequences worth stating:
///
/// - The fingerprint, the ref and the blob oid are all computed over the
///   POST-promotion bytes, on the dry path exactly as on the real one — a
///   rehearsal reports what a real run mints (§4.4), and the fingerprint agrees
///   either way only because the promotion is rev-neutral.
/// - The promotion's armed gate runs HERE (finding 9): the marker is a change to
///   a page like any other, so it passes [`crate::gate::gate_write`] — the same
///   mount, the same INDEX-integrity floor — before it can be handed back as
///   pending.
///
/// # Errors
/// `bad_path` / `bad_request` (the target escapes the workspace, or its slug id
/// is taken), `pin_target_missing` (no such page or
/// selector), `read_mint_required` (D16 — a session actor pinning unread
/// content), `write_conflict` (the receipt's rev is stale), a
/// `convention_fault` / `armed_drift` / `index_integrity` gate refusal on the
/// promotion, `io_error`.
fn mint_pin(
    root: &fs::WorkspaceRoot,
    spec: &wire::PinSpec,
    actor: Option<&str>,
    force: bool,
    mints: Option<&receipt::read_mint::ReadMintStore>,
) -> Result<PinMint, Box<ErrorBody>> {
    path_confined(&spec.target)?;

    let mut target_doc = load_doc(root, &spec.target).map_err(|e| {
        if e.code == ErrorCode::FileNotFound {
            pin_target_missing(&spec.target, format!("no page at {} to pin", spec.target.0))
        } else {
            e
        }
    })?;
    // The armed gate SCOPES its rules by the document's path, and `fs::load`
    // leaves that empty — an unstamped pre-image is a page no path-scoped
    // convention can see.
    stamp_path(&mut target_doc, &spec.target);

    // The CANONICAL selector, from the target's own read facts — one hpath
    // owner (`wire_map::facts` → `model::gotext::sanitize_heading`). A dewey
    // ordinal resolves here but is never carried: `fact.hpath` is what the
    // receipt was keyed on and what the lock will declare.
    let facts = wire_map::facts::read_facts(
        &wire_map::project_toc(&target_doc),
        target_doc.raw.as_bytes(),
    );
    let Some(fact) = wire_map::facts::resolve_selector(&facts, &spec.selector) else {
        return Err(pin_target_missing(
            &spec.target,
            format!(
                "no section addressed by \"{}\" in {}. Nothing was written — the pin's \
                 page is byte-untouched. {}",
                spec.selector,
                spec.target.0,
                crate::section_recovery(&spec.selector, Some(spec.target.0.as_str()))
            ),
        ));
    };
    let selector = fact.hpath.clone();
    // Captured before the promotion re-resolve borrows the doc again: anchor rows
    // carry a block id (heading rows do not), and the RAW title is what the D15
    // slug derives from.
    let fact_anchor = fact.anchor.clone();
    let title = fact.title.clone();
    // The RAW segment array — the pre-image `hpath` above is a lossy projection
    // of (`sanitize_heading` is many-to-one). The lock's `path` array is built
    // from THESE segments and never by re-splitting the joined string, so the
    // machine surface carries what round-trips (R1.6).
    let fact_hpath_raw = fact.hpath_raw.clone();

    // D16: the gate, and its rev-recheck against the bytes on disk RIGHT NOW —
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
        &selector,
    )?;

    // Compose the promotion IN MEMORY (nothing is written here — see this
    // function's own contract) and mint from those bytes. Minting from the
    // post-promotion state is not a convenience: the blob oid is the WHOLE FILE's
    // content id, so taking it from the pre-promotion bytes would record an oid
    // for a state that ceases to exist the moment the marker lands (and `--vibe`
    // would eagerly write that unreachable blob). The fingerprint agrees either
    // way, because the promotion is rev-neutral.
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

    let (span, promoted_sec_rev, hpath_raw) = if promote {
        post_promotion_facts(pinned_doc, &spec.target, &selector)?
    } else {
        (fact_span, String::new(), fact_hpath_raw)
    };

    let fingerprint = mint_fingerprint(pinned_doc, &span, &spec.target, &selector)?;
    let blob = blob_oid(
        root,
        &spec.target,
        promoted.as_ref().map(model::CandidateDocument::raw),
        spec.vibe.unwrap_or(false),
    )?;
    let declared_ref = format!(
        "{}#{}",
        spec.target.0,
        lock_ref_fragment(pinned_doc, &span, fact_anchor.as_deref(), &selector)?
    );
    let row = pin_row(
        &spec.target,
        fact_anchor.as_deref(),
        &hpath_raw,
        blob.as_deref(),
    )?;

    Ok(PinMint {
        fact: wire::PinFact {
            declared_ref,
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

/// Re-resolve the pinned selector against the POST-promotion bytes: the span the
/// fingerprint will cover, the promoted section's `sec_rev`, and the raw segment
/// array. A promotion widens the selector's node by the marker line, so the
/// pre-promotion span would hash bytes that are no longer the selector's.
///
/// All three come from ONE fact deliberately. The array is heading-text-neutral
/// under a promotion and so equals its pre-promotion value, but "the lock row
/// describes the bytes that were hashed" is a property worth holding by
/// construction rather than re-arguing each time the promotion changes.
///
/// # Errors
/// `pin_target_missing` when the selector no longer resolves after promotion.
fn post_promotion_facts(
    pinned_doc: &model::Document,
    target: &Path,
    selector: &str,
) -> Result<(std::ops::Range<usize>, String, Vec<HpathSeg>), Box<ErrorBody>> {
    let facts = wire_map::facts::read_facts(
        &wire_map::project_toc(pinned_doc),
        pinned_doc.raw.as_bytes(),
    );
    let Some(fresh) = wire_map::facts::resolve_selector(&facts, selector) else {
        return Err(pin_target_missing(
            target,
            format!("\"{selector}\" no longer resolves after promotion"),
        ));
    };
    Ok((
        span_range(fresh.span),
        fresh.sec_rev.clone(),
        fresh.hpath_raw.clone(),
    ))
}

/// Mint the R4 lock row's structural fields — **the one-time conversion door**.
///
/// Everything the lock plane needs is derived HERE, from the target's own read
/// facts, and travels onward as [`PinRow`]. No later stage re-derives an address
/// by splitting [`wire::PinFact::declared_ref`]: that string is the human/wire
/// spelling, `sanitize_heading` is many-to-one, and `/` is a legal character in
/// a heading — so a split is a guess wearing a parse's clothing (R1.6: arrays
/// for machines, no string address forms on a machine surface).
///
/// **The anchor arm is the `path` arm.** R4 spells a block-anchor pin as a path
/// array whose SOLE element is the `^id` (`path: ["^findings"]` — ZT's own typed
/// blocks, [[86449b4e]] 17:07). It is a block-grain claim and is NEVER widened to
/// the host section: that widening would silently promote what the caller
/// claimed from one block to a whole section, and would surface only much later
/// as a drift verdict over bytes nobody pinned.
///
/// # Errors
/// - `bad_request` — **a MIXED array**: heading segments and a `^id` element
///   together. That form appears nowhere in the ratified trace, so its grain is
///   unruled — refused loudly rather than assigned a meaning here. Reachable two
///   ways: an anchor fact arriving with a heading chain, and a heading whose RAW
///   text literally begins with `^` (which would be indistinguishable from an
///   anchor element once written).
/// - `io_error` — no blob oid. R4 admits no pin without one: *"if hash is
///   missing, we lost the explicit target meaning"*. [`blob_oid`] still degrades
///   to `None` when git cannot answer, and under v1 that dropped the `objects:`
///   entry while the claim landed anyway. Under R4 the hash IS a field of the
///   claim, so the same condition now refuses the pin instead of shipping a row
///   that cannot mean what the schema says it means.
fn pin_row(
    target: &Path,
    fact_anchor: Option<&str>,
    hpath_raw: &[HpathSeg],
    blob: Option<&str>,
) -> Result<PinRow, Box<ErrorBody>> {
    let elements = match fact_anchor {
        // Block grain, sole element, no promotion — R4's anchor form verbatim.
        Some(id) if hpath_raw.is_empty() => vec![format!("^{id}")],
        Some(id) => {
            return Err(bad_request(format!(
                "refused: the anchor ^{id} resolved with a heading chain as well, \
                 and R4 spells an anchor pin as a path array whose SOLE element is \
                 the ^id. A mixed array — headings AND an anchor — has no ruled \
                 meaning, so the engine will not invent one. Nothing was written."
            )));
        }
        None => hpath_raw.iter().map(|s| s.h.clone()).collect(),
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
        // with `n: None`, which DEMANDS uniqueness. So an address that turns
        // ambiguous later starts refusing loudly instead of silently landing on
        // whichever sibling the ordinal now points at — the same law the read
        // face's minimal addresses hold (`wire_map::facts::raw_addresses`).
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

/// The lock `ref`'s fragment — the spelling that must RESOLVE, which is not the
/// same string as the canonical selector the receipt is keyed on.
///
/// `model::selector::Selector::parse` is the normative ref grammar and the verify
/// plane's front door: `#^id` → the block, anything else → a `/`-split chain of
/// **RAW** heading texts matched byte-exactly (`model::resolve`). The host-face
/// selector is SANITIZED (`model::gotext::sanitize_heading` turns every space
/// and `/` into `-`), so writing it into the lock would mint a ref that resolves
/// to nothing for any heading with a space in it — a pin that reads
/// `red(dangling)` the moment it lands.
///
/// # Errors
/// `bad_request` when a heading in the chain carries a `/` or a `#` — the joined
/// grammar cannot round-trip it, and guessing would silently address a different
/// node. The remedy is the node's own `^id`, which has neither problem.
fn lock_ref_fragment(
    doc: &model::Document,
    span: &std::ops::Range<usize>,
    anchor_row: Option<&str>,
    selector: &str,
) -> Result<String, Box<ErrorBody>> {
    if let Some(id) = anchor_row {
        return Ok(format!("^{id}"));
    }
    let Some(chain) = section_hpath_at(&doc.root, span.start) else {
        return Err(bad_request(format!(
            "cannot address \"{selector}\" in the lock ref grammar — no heading chain \
             at that span"
        )));
    };
    if let Some(bad) = chain.iter().find(|h| h.contains('/') || h.contains('#')) {
        return Err(bad_request(format!(
            "the heading \"{bad}\" carries a `/` or `#`, which the lock ref grammar \
             (`page#A/B`, model::selector) cannot round-trip — give that section an \
             explicit ^id and pin that instead"
        )));
    }
    Ok(chain.join("/"))
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
/// An id already in the selector's promotion slot is REUSED verbatim — that is
/// what makes a re-pin idempotent and keeps a benign orphan from accumulating
/// instead of growing one marker per pin. The selector may also BE a block anchor;
/// either way the handle exists and nothing needs writing.
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
/// The promotion is the ONE write in a pin that does not ride `commit_batch`, so
/// both rungs live here rather than at the write site — they must answer while a
/// refusal still costs nothing:
///
/// - **the artifact guard** (fix2a): a marker line must be lock-NEUTRAL, or this
///   door reaches the attestation bytes the batch door refuses to.
/// - **the armed gate** (R25, finding 9): the SAME `gate::gate_write` mount every
///   other target write passes, over this promotion's own before/after states,
///   carrying the armed conventions, the `--force` escape and the
///   never-escapable INDEX-integrity floor. Rev-neutral is not ungated: it is
///   still a write to a page this actor may not own.
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
    // U31: the promotion's bytes and the document both rungs judge are ONE
    // sealed candidate — the same object `fs::replace_file` will demand at the
    // write site far below, so this door can no longer land bytes it never
    // gated.
    let promoted =
        model::candidate_of_body(&target.0, promote_anchor(&target_doc.raw, slot, anchor));
    lock_artifact_guard(target_doc, promoted.document(), None, target)?;
    // THE ARTIFACT GUARD (D9) at the promotion door: an anchor promotion inserts
    // `^slug` and nothing else, so it introduces no address — asserted rather
    // than assumed, because this door lands a SECOND inode and would otherwise
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

/// The RAW heading chain of the section starting at `start` — `model` carries it
/// per node in delimiter-free array form, so nothing here re-derives an address.
fn section_hpath_at(node: &model::Node, start: usize) -> Option<Vec<String>> {
    if matches!(node.kind, model::NodeKind::Section { .. }) && node.span.start == start {
        return node.hpath.clone();
    }
    node.children
        .iter()
        .find_map(|c| section_hpath_at(c, start))
}

/// The read-mint gate (D16 + D6), the WHOLE refusal ladder in one place.
///
/// `actor == None` (or blank) is the bare CLI: local-operator-trusted, the gate
/// is bypassed exactly as `mrd put` bypasses the host's authz. A real session
/// actor must carry a receipt for THIS path and THIS selector — matching is
/// exact, so reading a parent section does not authorize pinning a child
/// (S6 fails closed by design), and only a SECTIONS-mode read mints at all.
/// A held receipt is then re-checked against the live `sec_rev` under the
/// caller's flock: a receipt is not a lease.
///
/// # Errors
/// `read_mint_required` (no covering receipt, or a host with no session layer),
/// `write_conflict` (the receipt covers a rev the target no longer carries).
fn read_mint_gate(
    store: Option<&receipt::read_mint::ReadMintStore>,
    actor: Option<&str>,
    target: &Path,
    selector: &str,
    live_sec_rev: &str,
) -> Result<(), Box<ErrorBody>> {
    let Some(actor) = crate::read::mint_actor(actor) else {
        return Ok(());
    };
    let Some(store) = store else {
        return Err(read_mint_required(
            target,
            format!(
                "pin of {}#{selector} refused: this host holds no read-receipt ledger, so it \
                 cannot know that actor {actor} read the content (the per-request sidecar has \
                 no session — pin through the resident daemon, or from the local CLI)",
                target.0
            ),
        ));
    };
    let Some(receipt) = store.lookup(actor, &target.0, selector) else {
        return Err(read_mint_required(
            target,
            format!(
                "pin of {}#{selector} refused: actor {actor} has not read that selector in this \
                 session — you cannot attest content that was never in your context. Read it \
                 first (mode sections, that exact selector), then pin.",
                target.0
            ),
        ));
    };
    if receipt.sec_rev != live_sec_rev {
        let mut e = ErrorBody::new(ErrorCode::WriteConflict);
        e.path = Some(target.clone());
        e.expected = Some(NodeRev(receipt.sec_rev.clone()));
        e.actual = Some(NodeRev(live_sec_rev.to_owned()));
        e.message = Some(format!(
            "pin of {}#{selector} refused: the receipt covers rev {} but the section now carries \
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

/// Mint the pin's fingerprint over the bytes the promotion will land — the
/// R31 discharge of [`model::fingerprint::fingerprint_span`]'s fallible owner.
///
/// **This refusal is NOT the load-bearing guard, and stage 3 must not read it
/// as one.** Every ref form whose normalized span can be empty is already
/// refused at an EARLIER rung: an own-line anchor projects no read-face fact
/// ("no section addressed"), and a whole-page ref cannot express a selector at
/// all. So this rung is measured-unreachable today — `mrd pin` cannot deliver
/// an empty span to it (`tests/s2fix_empty_span_mint.rs` asserts each rung by
/// name). The guard that BITES is on the verdict side
/// (`model::fingerprint::ContentVerdict::EmptySpan`), because the class arrives
/// through hand- or tool-authored `meridian-lock` blocks, never through here.
///
/// It exists because the owner is fallible and every door discharges it — belt
/// on belt, and a cheap one: if a future read-face change ever projects such a
/// fact, the pin refuses instead of minting a token that matches every
/// document.
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

/// The block id of an anchor whose HOST LINE starts at `line_start`, if the line
/// carries one. This is the idempotence probe against the promotion slot: the
/// slot either already bears a stable id (reuse it, promote nothing) or it does
/// not (mint the slug).
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
/// title (`"Leader's Guideline"` → `leaders-guideline`, the ratified example).
/// Determinism is the whole point — a re-pin recomputes the SAME id, so promotion
/// is idempotent and an orphan never accumulates; a counter or a random id would
/// do neither.
///
/// Apostrophes are dropped rather than separating (`Leader's` is one word); every
/// other run outside the one block-id charset (`[A-Za-z0-9-]`, §2.4) collapses to
/// a single `-`. A slug that collides with an id already on the page is REFUSED
/// (see the caller) rather than uniquified, so the id stays a function of the
/// title alone.
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
/// A promoted marker goes on its OWN LINE there, never at the heading line's
/// tail, and that placement is load-bearing twice over:
///
/// - **Address-neutral.** A heading's text is everything after its `#` run,
///   trimmed (`syntax`'s heading scan), so a tail marker would become PART of
///   the heading text — the section's sanitized hpath would change from
///   `Guide/Leaders-Guideline` to `Guide/Leaders-Guideline-^leaders-guideline`,
///   dangling every existing pin and every reader's address for it. On its own
///   line the heading text is untouched.
/// - **Fingerprint-neutral (D14).** norm-v2's R2 removal takes an own-line
///   anchor's ENTIRE line including its terminator, so the canonical bytes of
///   the section are byte-identical before and after. That exactness is what
///   makes promoting into a target this actor may not own honest: it cannot
///   redden anyone else's pin on the same section.
fn promotion_slot(raw: &str, line_start: usize) -> usize {
    raw.as_bytes()[line_start..]
        .iter()
        .position(|&b| b == b'\n')
        .map_or(raw.len(), |p| line_start + p + 1)
}

/// Write `^id` on its own line at `slot` (see [`promotion_slot`]), matching the
/// file's own line terminator so a CRLF page stays CRLF.
///
/// # The file's EOF terminator state is preserved (finding 7)
/// Rev-neutrality is a claim about EVERY pinned span in the target, not just the
/// one being pinned, so the promotion may not move a single byte outside the
/// marker line. At EOF that is a real constraint: this used to append a
/// terminator after the marker unconditionally, which on a file whose last line
/// carried none added one — and norm-v2 masks the MARKER line, not that byte. The
/// enclosing spans' canonical bytes moved, so another page's green pin over the
/// same target went red on somebody else's pin.
///
/// So a marker landing at an unterminated EOF stays unterminated, and norm-v2's
/// R2b takes it: an own-line anchor with no terminator of its own is removed
/// together with the terminator BEFORE it — which is exactly the terminator this
/// function had to add to give the marker its own line. The canonical bytes come
/// out byte-identical, and `promoting_at_eof_leaves_another_pages_pinned_fingerprint_identical`
/// holds the claim.
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
/// sibling claim), render the canonical bytes, and hand back the span
/// they replace plus THE MINTED BLOCK ITSELF — the one byte form
/// [`lock_artifact_guard`] admits as a lock change (R25).
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
    // NO INGRESS HERE. The R4 row arrived already structural: `mint_pin` built
    // it from the target's read facts ([`pin_row`]), so this door parses
    // nothing, splits nothing, and cannot mint a row that fails to read back.
    // Under v1 this site re-parsed `declared_ref` into an `addr::Addr` and
    // separately keyed a shared `objects:` table by the target's path spelling;
    // R4 has neither — the hash rides the pin row it was minted for, so it can
    // no longer outlive the claim.
    lock.upsert_pin(lock::PinEntry::new(
        &pin.row.object,
        &pin.row.hash,
        pin.row.selector.clone(),
        &pin.fact.fingerprint,
    ));
    let edit = lock_block_splice(doc, found.map(|f| f.span), &lock).0;
    // LOCK-IS-CONTENT (#8 §5): the block sits inside the page's own span, so a
    // page pinning a section of ITSELF that would CONTAIN the block is pinning
    // bytes this very write is about to change — the claim could never be green,
    // on this write or any later one. Refuse rather than mint a permanently-red
    // pin (§5's law: never a silent colour the engine knows is wrong).
    // TOUCHING counts, not just overlap: a fresh block is an EOF INSERT, and a
    // section that runs to EOF absorbs it — `edit.span.start == pin.span.end` is
    // exactly the self-pin-of-the-last-section case.
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
            pin.fact.selector
        )));
    }
    Ok((edit, lock::render(&lock)))
}

/// The `meridian-lock` block's byte form and its placement law, in ONE place —
/// shared by the pin path and [`lock_write`] so the two cannot drift.
///
/// A block that EXISTS is replaced across its exact fence-to-fence span. A fresh
/// block is birthed at EOF (lockfile-at-bottom), separated from existing content
/// by exactly one blank line, and the file ends with one terminator —
/// `lock::render` emits no trailing newline, so terminators are this caller's
/// (the `crates/lock` contract). Returns the edit plus whether it BIRTHED.
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
/// one `crates/lock` read adapter. A present-but-broken lock is an ERROR, never
/// "absent": a sole-writer page reaches that state only by hand editing, and
/// adopting it would launder corruption.
///
/// # Errors
/// `bad_request` naming the `LockError` (malformed, unsupported version, or
/// MULTIPLE blocks).
fn find_lock(doc: &model::Document) -> Result<Option<lock::Found>, Box<ErrorBody>> {
    lock::find(doc).map_err(|e| {
        bad_request(format!(
            "the page's meridian-lock state is corrupt ({e:?}) — the engine is the sole \
             writer (#8 §3); repair the block by hand-removing it, then re-mint"
        ))
    })
}

/// The one `crates/lock` locate adapter: the page's existing block span
/// (fence-to-fence, terminator-exclusive), `None` when the page has no lock,
/// or a teaching `bad_request` when the page's lock state is corrupt (MULTIPLE
/// blocks — sole-writer mints exactly one — or unparseable YAML). Surfacing
/// beats adopting: a hand-edited lock must be repaired deliberately, never
/// silently rewritten over.
fn locate_lock(doc: &model::Document) -> Result<Option<std::ops::Range<usize>>, Box<ErrorBody>> {
    Ok(find_lock(doc)?.map(|found| found.span))
}

/// Acquire the workspace write flock (D9, xproc-race fix) with the typed
/// error split — G2: a held lock (`WouldBlock`, `LOCK_NB`) is the fast
/// `workspace_busy` refusal (retry — transient), and any OTHER lock-file I/O
/// failure maps to the typed `io_error{cause}` frame; nothing here unwraps.
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

/// Workspace-root confinement (d2 §2.5 C3 "+ workspace-root"): the same §1
/// path law the strict decode enforces — no absolute path, no `.`/`..`/empty
/// segment, **and no root separator in the head** — so a write door can never
/// escape the root via `root.join`. A violation is `bad_path`, echoing the
/// offending path.
///
/// **The predicate itself lives in `addr::confined`**, the `std`-only leaf, so
/// the write doors here and the resolver in `model` ask ONE implementation. A
/// second copy of a confinement fact is how one question grows two answers.
///
/// **U11 — why the head-colon arm is part of confinement.** Confinement was an
/// ambient property: every path was joined onto the ONE `fs::WorkspaceRoot`, so
/// the lexical check was the whole story. Multi-root removes that guarantee — a
/// `root:` prefix selects WHICH tree a path is joined onto — so a `root:`-bearing
/// spelling arriving at a write door is an ADDRESS, never a corpus path, and is
/// refused here rather than creating a document no address can ever name (§ 4.2,
/// D11).
fn path_confined(path: &Path) -> Result<(), Box<ErrorBody>> {
    if !addr::confined(&path.0) {
        let mut e = ErrorBody::new(ErrorCode::BadPath);
        e.path = Some(path.clone());
        return Err(Box::new(e));
    }
    Ok(())
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
/// The ONE writer of that field in this crate, because two callers reaching into
/// `NodeKind::Document` by hand is how one of them forgets: the armed gate SCOPES
/// its rules by this value, so an unstamped pre-image is a page no path-scoped
/// convention can see.
fn stamp_path(doc: &mut model::Document, path: &Path) {
    if let model::NodeKind::Document { path: p, .. } = &mut doc.root.kind {
        p.clone_from(&path.0);
    }
}

/// Same FILE, not the same spelling. A pin whose target is the pinning page
/// reached through a DIFFERENT spelling of that one path still writes the page
/// the batch is being composed against, and composing against the pre-promotion
/// bytes would splice the lock block at an offset the file no longer has.
///
/// String equality answers the common case with no I/O; when the spellings differ
/// the filesystem answers, because two address owners is a known open finding and
/// this call adopts neither of them.
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

/// The I4 def-layer discovery anchor and refusal label (S4a): the target file's
/// ABSOLUTE spelling — exactly what a host passes the standalone `check_write`
/// op. `policy::defs` walks upward from it for `defs/` layers, so a
/// workspace-relative path would anchor the ladder at the process cwd instead of
/// the workspace.
fn conformance_target(root: &fs::WorkspaceRoot, path: &Path) -> String {
    root.0.join(&path.0).display().to_string()
}

/// Map an I4 conformance refusal onto its wire envelope: a `bad_request`
/// teaching frame (recovery `fix`) carrying the ladder's `CODE: message —
/// remedy` render verbatim, plus the refused path. Closed-taxonomy discipline —
/// no new §8 reason is minted, so the frozen v2 error surface keeps its shape.
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

/// Assemble the birth/death Delta at the ONE production constructor
/// ([`assemble_delta`]): a `created`/`deleted` file (absent-tense per §7.1 — no
/// `file_rev_before` on birth, no `file_rev_after` on death). `fd` is `None`
/// only if nothing changed, which a real create/remove never is.
fn birth_death_delta(
    seq: u64,
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
    assemble_delta(
        seq,
        root_before.clone(),
        root_after.clone(),
        actor,
        now,
        files,
    )
}

/// The post-commit receipt FACT: resolve the anchor in the just-committed receipt
/// file (host-block-leaf grain — the true after-state, §6.1). `None` when the
/// splice carried no receipt.
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
// The `@fp` strip at DOCUMENT grain + the lock ARTIFACT guard (advisor R25)
// ---------------------------------------------------------------------------
//
// Both rungs read the SAME candidate `after_doc` the ladder, the armed gate and
// the commit read, because both answer a question about BYTES, not about a verb:
//
// - The strip was a walk of named payload fields. A field list is the defect
//   shape: it missed `plan_edits.create.title`, it could not see a token two
//   fields compose between them, and it judged a payload OUT of the document it
//   lands in — stripping a token the document law calls a code sample. One
//   grammar, one grain: identify tokens in the candidate, remove them from the
//   payloads that carry them, and refuse what is left.
// - The read-mint gate guards `splice.pin`. The `meridian-lock` bytes it protects
//   are ordinary page text, so every put shape is a door to them. A guard on the
//   verb is not a guard on the file.

/// One `@fp` token run in the candidate, classified by WHO put it there.
enum FpOrigin {
    /// Bytes THIS batch supplies: request edit `edit`, at `local` inside its
    /// payload. Removable — this is the strip.
    Introduced {
        edit: usize,
        local: std::ops::Range<usize>,
    },
    /// A token already on disk, retained verbatim by this batch. NOT this
    /// write's to remove: deleting bytes the batch never addressed would move
    /// the fingerprint of a node this write does not own, reddening pins that
    /// have nothing to do with it.
    Retained,
}

/// Classify every `@fp` token run in `after` — the ONE identification, shared by
/// the strip and by the assertion that follows it.
///
/// # Errors
/// `bad_request` when a token can be attributed to no single payload: the batch
/// COMPOSED it out of retained bytes plus its own (a claim minted by an edit that
/// never carried it), or two request edits contest the same region.
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

/// Each insertion in AFTER coordinates, with the pre-image REGION that produced
/// it — [`splice_index`]'s first half.
type Inserted = Vec<(std::ops::Range<usize>, std::ops::Range<usize>)>;

/// Each surviving run in AFTER coordinates, with the pre-image OFFSET it came
/// from — [`splice_index`]'s second half.
type Retained = Vec<(std::ops::Range<usize>, usize)>;

/// **The after image, walked ONCE — the ONE attribution index.**
///
/// The sealed spans index the pre-image and are sorted and disjoint, so a single
/// forward scan places every inserted text AND every surviving run in AFTER
/// coordinates, with no shift arithmetic. Returns `(inserted, retained)`:
/// `inserted` carries each insertion with the pre-image REGION that produced it;
/// `retained` carries each surviving run with the pre-image offset it came from.
///
/// It is a shared owner rather than a copied loop because TWO grammars now ask
/// the same question of the same candidate — the `@fp` strip ([`classify_fp`])
/// and U12's stored-form translation ([`classify_cross_root`]) — and two
/// implementations of one attribution law is how one question grows two answers.
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

/// Which request edit produced the sealed region — by the TARGET SPAN the model
/// itself resolved, never by text similarity. `validate_batch` refuses a batch
/// whose target spans are not pairwise disjoint (containment counts), so a
/// non-empty region has at most one container; a region contested past the
/// boundary rule below is `None` (refuse, never guess).
///
/// # The boundary rule, which containment alone cannot decide
/// Sections are contiguous: a section's span ENDS on the byte where its next
/// sibling's span BEGINS. An `md.append_section` plans `put{at:"end"}`, whose
/// replaced region is EMPTY and sits exactly on that shared byte (§4.4), so both
/// siblings contain it. The one the model planned it from is the one that ENDS
/// there — the other merely begins there. Empty regions are the only ones that
/// can land on a shared byte, and `put{at:"end"}` is the only shape that produces
/// one (a `match` needle is non-empty by validation), so this decides every case
/// it applies to and touches no other.
///
/// Ported verbatim from `run::fp::attribute_region` (fix8, `9953cf3b`), where the
/// same false-red was found: without the rule, a decorated `put{at:end}` into any
/// section with a following sibling refuses a legitimate write.
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

/// **The `@fp` strip, at document grain** (advisor R25, structural fix 2): remove
/// every token this batch INTRODUCES from the payload that carries it, re-seal so
/// the commit lands exactly the judged bytes, and assert the candidate introduces
/// none — the loud refusal that catches the next missed door.
///
/// The batch is rewritten rather than the sealed copy because [`commit_batch`]
/// re-validates the REQUEST: a strip applied only to the sealed batch would judge
/// bytes the commit does not write, which is the divergence S4a closed.
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

    // THE ASSERTION (R25): the candidate introduces no token. Live on every
    // write path, dry and real alike — a door that reaches these bytes without
    // passing the strip refuses here instead of landing silently.
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
// U12 — the STORED-FORM translation, guarded at the ARTIFACT (D9)
// ---------------------------------------------------------------------------
//
// `put` translates an agent-plane `root:` address into the `obsidian://` stored
// form; `read` translates back (`crate::read`). The grammar and the positional
// law are `crate::positions`'; what lives here is WHERE it lands:
//
// - the TRANSFORM rewrites the PAYLOAD that introduced an address, never the
//   assembled bytes. `fs::apply_batch` re-splices the sealed spans onto the disk
//   pre-image and REFUSES a candidate that is not that splice's own result
//   (S3-R11(c), `crates/fs/src/lib.rs:566`), so a transform applied to the
//   assembled candidate would land bytes the primitive rejects. The payload is
//   also exactly "what this write introduces", which is the only thing a write
//   may move: rewriting a RETAINED address would change bytes this batch never
//   addressed and redden pins that have nothing to do with it.
// - the GUARD reads the CANDIDATE, on every door, dry and real alike — the
//   artifact, never the verb. A door that reaches these bytes without passing
//   the transform refuses instead of landing an agent-plane spelling on disk.

/// Every AGENT-PLANE cross-root address in the candidate, classified by WHO put
/// it there — the [`classify_fp`] shape over U12's grammar, sharing
/// [`splice_index`]'s one attribution law.
///
/// # Errors
/// `bad_request` when an address can be attributed to no single payload: the
/// batch COMPOSED it out of retained bytes plus its own, or two request edits
/// contest the same region (§ 9.4 P7 — refuse, never transform blind).
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
            // NOT this write's to translate: rewriting bytes the batch never
            // addressed would move the fingerprint of a node this write does not
            // own. The same rule the `@fp` strip follows, for the same reason.
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
/// cross-root address this batch INTRODUCES into its `obsidian://` stored form,
/// re-seal so the commit lands exactly the judged bytes, and leave the artifact
/// guard behind it.
///
/// Mirrors [`strip_fp_candidate`] rung for rung — identify in the candidate,
/// attribute to the payload, rewrite the payload, re-validate, rebuild, assert —
/// because it answers the same kind of question about the same bytes. The batch
/// is rewritten rather than the sealed copy because [`commit_batch`]
/// re-validates the REQUEST.
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
            // Frontmatter is NOT an address position (§ 9.2 A-1): `root:` is a
            // live YAML key in the shipped preset/def grammar, so an address
            // attributed to a composed `{key}: {value}` line means the grammar
            // moved under this code. Refuse rather than translate blind — a
            // blanket rewrite there would corrupt the def AND silently
            // invalidate every pin whose fingerprint covers the line.
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

    // The closing rung goes through the SAME door-facing guard every other door
    // calls — one entry point, so "which doors are guarded" is answerable by
    // counting one name rather than by reading five call sites.
    stored_form_guard_lazy(Some(doc), after_doc, path)
}

/// **The stored-form translation at a WHOLE-BODY door** (D9): the birth door
/// supplies its entire document, so the whole body is this write's payload and
/// "introduced" and "present" are the same set — no attribution walk is needed
/// or possible.
///
/// Ordered after the `@fp` strip for the same reason the splice door orders it
/// there, and lazy for the same reason: a body with no cross-root position never
/// loads a mount table.
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

/// **THE ARTIFACT GUARD** (D9, R20/R30): the candidate introduces NO agent-plane
/// cross-root address in an owned position.
///
/// Live on every write door in this module, dry and real alike. `before` is the
/// pre-image, or `None` for a birth — where "introduced" and "present" are the
/// same set. The comparison is introduce-scoped for the reason
/// [`classify_cross_root`] gives: an address a document already carried is not
/// this write's to move.
///
/// *A guard on a verb is not a guard on a file.* A door that reaches these bytes
/// without passing the translation refuses HERE instead of landing an
/// agent-plane spelling on disk — which is stage 2's criterion-4 machinery,
/// reused at the candidate rather than reinvented at the verb.
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

/// **THE ONE DOOR-FACING ENTRY to the artifact guard.** Every byte-landing door
/// in this module discharges it, and it loads the mount table only if the
/// candidate can carry a cross-root position at all — so an ordinary
/// single-root write never pays for one.
///
/// One entry point rather than five call sites into [`stored_form_guard`],
/// because *"which doors are guarded"* must be answerable by counting a single
/// name; `tests/u12_door_enumeration.rs` counts exactly that.
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

/// **The lock ARTIFACT guard** (advisor R25, structural fix 1): the
/// `meridian-lock` bytes change ONLY as the pin this call minted.
///
/// `minted` is the canonical block this splice's pin composed, or `None` when the
/// call carries no pin. The comparison is over RAW block bytes
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
/// (§4.4: armed edits align 1:1 with request edits) — resolution failures
/// name the failing target exactly (candidates in THE grammar).
///
/// **The ADDRESS half of the `@fp` law is ordered here** (the payload half is
/// [`strip_fp_candidate`]'s, at document grain). `Match{old}` is a NEEDLE matched
/// against stored bytes, which never carry a token, so a needle copied from the
/// decorated render face would otherwise never match its own document. It is
/// stripped for the same reason `read::to_model_ref` strips a `SecRef::Anchor`
/// before the mint-guard sees it: an address is compared, never stored. This is
/// the ONE funnel every native and lowered edit passes through, so no put shape
/// can skip it.
fn model_edits_and_before_facts(
    doc: &model::Document,
    edits: &[Edit],
    path: &Path,
) -> Result<(Vec<model::Edit>, Vec<model::Target>), Box<ErrorBody>> {
    let mut model_edits = Vec::with_capacity(edits.len());
    let mut before_facts = Vec::with_capacity(edits.len());
    for edit in edits {
        let target = to_model_ref(&edit.target)?;
        // `put at:upsert` is the ONE create-or-replace shape: the `fm_key` may
        // not exist yet, so its BEFORE fact is SYNTHESIZED (`fm_upsert_before` —
        // the existing line's rev, or the empty insertion point's rev for a
        // create) rather than resolved; a plain `resolve` would `ref_not_found`
        // on the very key the upsert is about to create. Two guards fence the
        // verb to its domain (design): the target MUST be an `fm_key`, and the
        // value MUST be single-line — the server composes `{key}: {value}`, so a
        // newline in the value would forge extra frontmatter lines.
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
                return Err(bad_request(
                    "put at:upsert value must be single-line (no newline)",
                ));
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
                    text: text.clone(),
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

/// The post-batch document state, built ONCE (the §4.4 one-reparse law's dry
/// twin): apply the sealed span edits in memory → reparse → build, stamping the
/// document path (`model::build` is I/O-free and leaves it empty) so §11.1
/// verdicts carry it. Both the armed AFTER facts and the verdicts read THIS doc,
/// on both the dry and real paths — computed, never arithmetic-shifted.
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
            // the frozen text — refuse loud rather than invent one.
            bad_request("target identity does not survive the edit — armed facts unrepresentable")
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

/// The receipt append for a REAL commit: render the line (facts about what
/// is being ARMED, §6.1), honor the F4 parent-dir obligation (fs does NOT
/// mkdir — the production caller does, real commits only), and fold the
/// append at the receipt file's EOF.
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

/// The §5.2 failure split, mapped: every refusal verdict to its wire frame
/// (code + REQUIRED recovery + the frozen extras). `edits` is the EFFECTIVE
/// batch (post-lowering, U8b) — the request targets the extras echo.
fn verdict_to_wire(
    verdict: &model::SpliceVerdict,
    edits: &[Edit],
    doc: &model::Document,
    before_facts: &[model::Target],
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
        // `model_edits_and_before_facts`, which raises the same teaching. Routed
        // through the shared helper anyway so the two miss sites cannot drift
        // into one sentence and one bare code (issue-08).
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
            // Name each duplicate by node index + ^block via the shared helper
            // (§2.1 / d1 teaching refusal). In the splice flow this arm is
            // normally pre-empted by the per-target resolution in
            // `model_edits_and_before_facts`; routing it through `ambiguous` keeps
            // both refusal sites identical. The offending target is the first
            // edit that resolves ambiguously.
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
        model::SpliceVerdict::CasMismatch { expected, actual } => {
            let mut e = ErrorBody::new(ErrorCode::CasMismatch);
            e.expected = Some(NodeRev(expected.0.clone()));
            e.actual = Some(NodeRev(actual.0.clone()));
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
        model::SpliceVerdict::Overlap { spans } => {
            let mut e = ErrorBody::new(ErrorCode::BadRequest);
            e.message = Some("batch targets must be disjoint (§4.4)".into());
            // Echo the overlapping REQUEST targets (§2.1 grammar): the
            // targets whose resolved pre-batch spans are the overlap pair.
            let overlapping: Vec<SecRef> = edits
                .iter()
                .zip(before_facts)
                .filter(|(_, fact)| spans.contains(&fact.span))
                .map(|(edit, _)| edit.target.clone())
                .collect();
            if !overlapping.is_empty() {
                e.overlap = Some(overlapping);
            }
            e
        }
        model::SpliceVerdict::WouldCorrupt { lost } => {
            let mut e = ErrorBody::new(ErrorCode::WouldCorrupt);
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

/// One commit's inputs: the model-side batch plus the envelope facts the
/// engine records but never invents (§9). `receipt` carries the receipt
/// file's path and the pre-rendered append (rendered by the `receipt` crate,
/// folded in BEFORE validation so it rides the sealed batch and the single
/// root advance — §6.1, D-C3); its presence must pair with the batch's —
/// `fs` enforces the §6.5 seam contract fail-loud before any byte lands.
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
/// root advance). The frame's `seq` is `seq + 1` — the CALLER'S epoch seq
/// advanced by one; the caller advances its own ring with the returned frame
/// (this seam holds no ring, so the resident daemon can commit without one).
///
/// LOCK-FREE primitive (D9): the write flock is the CALLER's — `splice`
/// acquires it around this whole call. A direct caller outside the choke-point
/// (tests) runs unserialized; the D8 pre-image verify still refuses drift.
///
/// # Errors
/// [`CommitError`] — validation refusal, environment failure, or I/O; in
/// every error case nothing was emitted (a Delta exists only for a batch that
/// actually committed).
pub fn commit_batch(
    root: &fs::WorkspaceRoot,
    seq: u64,
    req: &CommitRequest,
) -> Result<DeltaFrame, CommitError> {
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

    // Commit: the two-file atomic write (§6.5). fs enforces the pairing
    // contract fail-loud; a refusal here means no byte landed. The splice
    // SOURCE is read#2's validated bytes (`before_content.raw` — the bytes the
    // sealed spans index), and fs verifies the live file still carries them
    // before any rename (the D8 TOCTOU-gap fix): drift refuses the typed
    // write-conflict instead of blind-splicing stale spans into moved bytes.
    // U31: the commit seam mints the candidate for the bytes it is about to
    // land. Before the seal this door reached disk with no candidate at all —
    // the splice path built one above and `commit_batch` re-validated the
    // REQUEST, so nothing tied the document the gates judged to the bytes this
    // function writes. `fs` now refuses a candidate that is not this batch's
    // splice result, which is what makes that tie a compile-and-run fact.
    let candidate = model::candidate_of_batch(&req.content_path, &before_content.raw, &sealed);
    // THE ARTIFACT GUARD (D9) at the commit seam. The splice door translates and
    // then guards; this is the PUBLIC seam, and a caller reaching it directly
    // has passed no translation at all. Guarding here is what makes the door
    // enumeration a property of the file rather than of today's call sites —
    // a guard on a verb is not a guard on a file.
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

    // Change facts (wire-owned delta.rs) → wire projection, worked-frame file
    // order: content file first, then the receipt file (§7.1 E3/E4 print
    // order).
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
    Ok(assemble_delta(
        seq,
        root_before,
        root_after,
        req.actor.clone(),
        req.now.clone(),
        files,
    ))
}

/// **The ONE production `DeltaFrame` construction site** (§7.3
/// single-constructor law): the commit path and the watcher's external path
/// (F5-WATCH) both assemble here — `seq` is the caller's current epoch seq, so
/// the emitted frame carries `seq + 1` (§7.1 late law, nothing persisted),
/// envelope facts exactly as given (§9: external deltas pass `None`/`None` —
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
            seq: seq + 1,
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

/// The write path's ONE production `policy::evaluate` call site (advisor Ruling 3
/// — the checkable form of the non-divergence claim): run every admitted pack
/// over the touched doc's post-batch state and project the §11.1 findings to
/// `wire::Verdict`. `corpus` is `None` — the caller hands only node/file-class
/// packs (corpus-class is refused at admission). Dry and real share this call
/// over the SAME simulated after-doc, so their verdict sets are byte-identical by
/// construction (advisor Ruling 2). Empty `rulesets` ⇒ `[]` (the BARE commit).
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

/// Project one `policy::Violation` into a `wire::Verdict` (§11.1) — findings in
/// THE grammar: hpath strings become `{h, n:None}` segments (§2.1), byte span →
/// `[u64,u64]`. `wire::Severity` is a distinct enum (no wire→policy edge).
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

    /// D8: the fs write-conflict marker maps to the TYPED `write_conflict`
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

    // The reserved-journal write restriction lived here — two tests proving an
    // ordinary splice targeting `meridian/journal.md` refused `bad_request`, dry
    // or real. The reserved path is retired (ZT 2026-08-02): that page is now
    // ordinary content, so there is no restriction left to prove.
}

/// Guarded `create`/`remove` — file birth and death (d2 §2.5 C3, U2.6). The
/// named gates: create-existing-path refuses (CAS), remove-after-drift refuses
/// citing rev, both emit the `before=absent`/`after=absent` change surface, and
/// both refusals map to their taxonomy rows (`cas_mismatch` + recovery
/// `refresh`, rows 13/14).
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
        let out = create(&root, 0, &create_args("notes/new.md", "# New\n"), &[])
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

    /// GATE — create-existing-path refuses (CAS negative) + taxonomy: a second
    /// `create` at an occupied path refuses `cas_mismatch` with recovery
    /// `refresh` (row 13), and the occupant's bytes are untouched.
    #[test]
    fn create_existing_path_refuses_cas() {
        let (dir, root) = ws();
        create(&root, 0, &create_args("notes/new.md", "# First\n"), &[]).expect("first create");

        let err = create(&root, 0, &create_args("notes/new.md", "# Second\n"), &[])
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
        let born = create(&root, 0, &create_args("notes/new.md", "# New\n"), &[]).unwrap();

        let out = remove(
            &root,
            0,
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

    /// GATE — remove-after-drift refuses citing rev + taxonomy: after the file
    /// drifts from the read rev, `remove` refuses `cas_mismatch` (recovery
    /// `refresh`, row 14) and NAMES the rev read (`expected`) vs found
    /// (`actual`).
    #[test]
    fn remove_after_drift_refuses_citing_rev() {
        let (dir, root) = ws();
        let born = create(&root, 0, &create_args("notes/new.md", "# New\n"), &[]).unwrap();
        let read_rev = born.file_rev_after.clone();

        // The file drifts under the plan (a later edit / foreign write).
        std::fs::write(dir.path().join("notes/new.md"), "# Drifted\n").unwrap();
        let live_rev = super::occupant_rev(&root, &Path("notes/new.md".into()))
            .unwrap()
            .unwrap();
        assert_ne!(read_rev, live_rev, "the fixture actually drifted");

        let err = remove(&root, 0, &remove_args("notes/new.md", &read_rev.0), &[])
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
            0,
            &remove_args("notes/ghost.md", "deadbeefdeadbeef"),
            &[],
        )
        .expect_err("removing an absent file refuses");
        assert_eq!(err.code, ErrorCode::FileNotFound);
    }

    /// Workspace-root confinement (d2 §2.5 C3 "+ workspace-root"): a `..`-escape
    /// or an absolute path refuses `bad_path` for BOTH create and remove — the
    /// op can never reach outside the root.
    #[test]
    fn guarded_ops_confined_to_workspace_root() {
        let (_dir, root) = ws();
        for bad in ["../outside.md", "/etc/passwd", "notes/../../escape.md"] {
            assert_eq!(
                create(&root, 0, &create_args(bad, "x"), &[])
                    .unwrap_err()
                    .code,
                ErrorCode::BadPath,
                "create confined: {bad}"
            );
            assert_eq!(
                remove(&root, 0, &remove_args(bad, "deadbeefdeadbeef"), &[])
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
            0,
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
        let born = create(&root, 0, &create_args("notes/new.md", "# New\n"), &[]).unwrap();
        let dry_dead = remove(
            &root,
            0,
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

    /// **U32 — the write door's own gate, after the journal.** A splice advances
    /// the tree root. The original gate proved that through the journal: each
    /// guarded write appended a row, and the chain recompute proved the run of rows was
    /// continuous, which is what dated the tree for `check`.
    ///
    /// The journal is gone (ZT 2026-08-02, remove-no-replacement), so the CHAIN
    /// half of that gate is gone with it — deliberately, and it is not re-asserted
    /// elsewhere here. What survives is the half that never needed the journal and
    /// that a re-derived `check` baseline still rests on: every splice moves the
    /// ambient root, and no splice leaves it where it found it.
    #[test]
    fn a_run_of_splices_advances_the_root_every_time() {
        let (_dir, root) = ws();
        create(
            &root,
            0,
            &create_args("notes/plan.md", "# Alpha\n\n## Beta\n\nw0\n"),
            &[],
        )
        .expect("birth");

        let mut seen = vec![ambient_root(&root).expect("live root")];
        for step in 1..=5 {
            let out = splice(
                &root,
                0,
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
    // F4 — THE GUARD, driven at its OWN level
    // -----------------------------------------------------------------------
    //
    // These call `stored_form_guard` DIRECTLY with a hand-built `MountSet`: no
    // config on disk, no `machine_mounts()`, and no translation anywhere in the
    // path. That independence is the point. The guard exists for a door that
    // reaches bytes WITHOUT passing the translation, so proving it only through
    // a write whose transform refuses first proves nothing about the case it was
    // built for — the transform would be doing the work and the guard would be
    // decoration.
    //
    // F4 was exactly that failure: the guard's population is supplied by
    // `agent_plane_occupants`, so a root the scanner skipped was invisible to
    // the guard as well. One `continue` disarmed both halves and the guard
    // reported success on an empty set.

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

    /// **THE GATE.** The guard REFUSES an agent-plane address on a
    /// declared-but-unbound root, with no transform anywhere in the call.
    ///
    /// Before the fix this returned `Ok(())` on an empty occupant set — a guard
    /// reporting success because it could not see.
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

    /// **The control that keeps the gate above from being satisfied by a guard
    /// that refuses everything** (S3-R8(c)).
    ///
    /// An external URI parses as a rooted address — `https://example.com` has
    /// root `https` — so this is the exact input a fix keyed on "unbound" rather
    /// than "undeclared" would refuse. Nothing declares `https`, so it is not
    /// this engine's to claim and never reaches the guard's population.
    #[test]
    fn the_guard_leaves_undeclared_schemes_alone() {
        f4_guard(
            "# Page\n\n[ext](https://example.com) and [m](mailto:a@b.example)\n\
             and [rel](./sibling.md) and [[ambient.md]]\n",
        )
        .expect("an ordinary corpus carries no agent-plane occupant and must pass the guard");
    }

    /// A BOUND root's agent-plane spelling still trips the guard — unchanged
    /// behaviour, asserted so the fix cannot be read as narrowing the population
    /// it already covered.
    ///
    /// Reaching the guard at all means the translation was bypassed, which is
    /// precisely what it is here to catch.
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
