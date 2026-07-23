//! The shared WRITE choke-point — `splice → commit` — lifted out of the sidecar
//! so the resident registry daemon and the per-workspace sidecar commit through
//! ONE implementation (arch map A6/W1: "lift, don't duplicate").
//!
//! # The single choke-point (decision 0002 W1)
//! [`splice`] is THE one function the write path flows through: load → §5.1 world
//! guard → validate → build the post-batch doc ONCE → evaluate verdicts → dry
//! short-circuit → (real) render the receipt + [`commit_batch`] (the D4 seam:
//! validate → `fs::apply_batch` → one Delta). Both hosts call `splice`; a later
//! per-session rule-evaluation hook carves in at the ONE marked verdict site,
//! never a rewrite. This unit builds NO hook, NO rule types — a BARE meridian-fs
//! commit (Advisor R3 ruling); the resident rule-engine placement is reserved.
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
    /// The requested edits, 1:1 with the armed edits in the response.
    pub edits: Vec<Edit>,
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
/// ambient-root/domain failure, or an I/O error — in every error case nothing was
/// committed and no Delta exists.
pub fn splice(
    root: &fs::WorkspaceRoot,
    seq: u64,
    args: &SpliceArgs,
    rulesets: &[policy::CompiledRuleset],
) -> Result<SpliceOutcome, Box<ErrorBody>> {
    // Journal write restriction (d2 §2.1 A3/A9; F4): the reserved receipt
    // journal is writable ONLY by the receipt engine (a receipt append rides
    // `args.receipt`, engine-rendered from armed facts). An ORDINARY splice
    // whose content target IS the reserved path is a forge attempt — refuse it,
    // dry or real. This restriction, plus the git witness, is what detects a
    // root-preserving forged row that chain continuity cannot (named residual,
    // `receipt::journal`). A `bad_request` teaching refusal (no new taxonomy
    // reason minted — U2.1 carries no U4.1 dependency).
    if fs::domain::is_reserved_journal(FsPath::new(&args.path.0)) {
        return Err(bad_request(format!(
            "refused: {} is the reserved receipt journal — writable only by the \
             receipt engine (d2 §2.1); an ordinary splice targeting it is a forged-row attempt",
            args.path.0
        )));
    }

    let doc = load_doc(root, &args.path)?;
    let root_before = ambient_root(root)?;

    // §5.1 order: the world guard FIRST — checked here so a stale plan
    // refuses before any per-target resolution can answer for it.
    if let Some(expected) = &args.if_root
        && *expected != root_before
    {
        let mut e = ErrorBody::new(ErrorCode::RootMismatch);
        e.expected = Some(NodeRev(expected.0.clone()));
        e.actual = Some(NodeRev(root_before.0.clone()));
        return Err(Box::new(e));
    }

    let (model_edits, before_facts) = model_edits_and_before_facts(&doc, &args.edits)?;
    let batch = model::SpliceRequest {
        if_root: args
            .if_root
            .as_ref()
            .map(|r| model::MerkleRoot(r.0.clone())),
        edits: model_edits,
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
        refused => return Err(verdict_to_wire(&refused, args, &doc, &before_facts)),
    };

    // Build the post-batch document state ONCE, shared by BOTH the armed AFTER
    // facts and the verdicts, for BOTH the dry and real paths — the single point
    // that makes the dry twin incapable of diverging from the real one (§4.4
    // one-reparse law; advisor Ruling 2). The real commit writes exactly these
    // bytes, so evaluating this simulated doc is evaluating the committed doc.
    let after_doc = build_after_doc(&doc, &sealed, &args.path);
    let armed_edits = simulate_armed_edits(&after_doc, &args.edits, &before_facts)?;
    // THE carve-in point (W1): the ONE production `policy::evaluate` call site
    // (advisor Ruling 3) — the touched doc's post-batch state → §11.1 verdicts,
    // filled identically on dry and real. Empty when the caller loads no pack
    // (the resident daemon's BARE commit). A later per-session rule-evaluation
    // hook attaches HERE, not by rewriting the path.
    let verdicts = evaluate_verdicts(rulesets, &after_doc);

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
                },
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

    // REAL commit: render the receipt line (facts about what is being
    // ARMED — §6.1), fold the append, honor the parent-dir obligation,
    // then drive the D4 commit seam (validate → apply → emit).
    let receipt_input = match &args.receipt {
        Some(addr) => Some(receipt_input(root, args, &root_before, &armed_edits, addr)?),
        None => None,
    };
    let frame = commit_batch(
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
        CommitError::Refused(v) => verdict_to_wire(&v, args, &doc, &before_facts),
        CommitError::Env(err) => err,
        CommitError::Io(err) => {
            let mut w = ErrorBody::new(ErrorCode::IoError);
            w.cause = Some(err.to_string());
            Box::new(w)
        }
    })?;

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
                file_rev_after: Some(NodeRev(after_doc.root.node_rev.0.clone())),
                edits: armed_edits,
            },
            receipt: receipt_fact,
            root_before: frame.delta.root_before.clone(),
            root_after: Some(frame.delta.root_after.clone()),
            seq: Some(frame.delta.seq),
            dry: None,
            verdicts,
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
// workspace-root. Both are journaled (write-mechanics only, A5 bound: op, path,
// actor, now, BOTH roots, the whole-file rev transition) and both expose the
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
    /// Dry run — everything except disk (no file, no journal row, no root advance).
    pub dry: bool,
}

/// The outcome of a guarded `create` (birth). `committed`/`root_after`/
/// `journal_anchor` are absent on a dry run — nothing landed. `verdicts` is the
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
    /// The appended journal row's anchor (`r-NNNNNN`); `None` on a dry run.
    pub journal_anchor: Option<String>,
    /// The birth Delta (`created`, `file_rev_before` absent); `None` on dry.
    pub committed: Option<DeltaFrame>,
    pub verdicts: Vec<Verdict>,
    pub dry: bool,
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
    pub journal_anchor: Option<String>,
    /// The death Delta (`deleted`, `file_rev_after` absent); `None` on dry.
    pub committed: Option<DeltaFrame>,
    pub verdicts: Vec<Verdict>,
    pub dry: bool,
}

/// **Guarded `create`** (d2 §2.5 C3): birth one file under CAS `if_absent` +
/// workspace-root, journal the birth, and emit the `created` change surface.
///
/// Order: path confinement → reserved-journal guard → world guard (§5.1) → the
/// gate seam over the birth's after-state → the `if_absent` CAS at the disk edge
/// ([`fs::create_file`], the single source of the guard) → root advance → birth
/// Delta → journal row (`before=absent`). `dry: true` runs everything except
/// disk and still refuses a would-be clobber.
///
/// # Errors
/// `bad_path` (escapes the workspace), `bad_request` (targets the reserved
/// journal), `root_mismatch` (stale world guard), `cas_mismatch` (the path is
/// occupied — taxonomy row 13, recovery `refresh`), or an I/O failure. In every
/// error case nothing was created and no journal row was written.
pub fn create(
    root: &fs::WorkspaceRoot,
    seq: u64,
    args: &CreateArgs,
    rulesets: &[policy::CompiledRuleset],
) -> Result<CreateOutcome, Box<ErrorBody>> {
    let fs_path = FsPath::new(&args.path.0);
    path_confined(&args.path)?;
    reserved_journal_guard(fs_path)?;

    let root_before = ambient_root(root)?;
    world_guard(args.if_root.as_ref(), &root_before)?;

    // The birth's after-state, built once from the body (path-stamped so the
    // gate sees it). Its whole-file rev is the born file's rev.
    let after_doc = build_doc(&args.path, &args.body);
    let file_rev_after = NodeRev(after_doc.root.node_rev.0.clone());

    // THE gate seam (advisor Ruling 3, mirrored from splice): the ONE
    // production verdict-evaluation site for a birth — U4.2 mounts `gate()`
    // here. Empty rulesets ⇒ `[]` (the BARE commit).
    let verdicts = evaluate_verdicts(rulesets, &after_doc);

    if args.dry {
        // A dry birth honors if_absent too — a rehearsal of a clobber refuses.
        if let Some(actual) = occupant_rev(root, &args.path)? {
            return Err(cas_mismatch(&absent_rev(), &actual));
        }
        return Ok(CreateOutcome {
            root_before,
            root_after: None,
            file_rev_after,
            journal_anchor: None,
            committed: None,
            verdicts,
            dry: true,
        });
    }

    // The if_absent CAS lives at the disk edge (`fs::create_file`): an occupied
    // path is `AlreadyExists`, mapped to `cas_mismatch{expected:absent,
    // actual:occupant-rev}` (row 13, recovery refresh — "re-read, it exists").
    if let Err(e) = fs::create_file(root, fs_path, &args.body) {
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
        model::delta::file_delta(None, Some(&after_doc)).as_ref(),
    );
    let journal_anchor = journal_write(
        root,
        "create",
        &args.path,
        args.actor.as_deref(),
        args.now.as_deref(),
        &root_before,
        &root_after,
        receipt::journal::FileTransition {
            before: None,
            after: Some(&file_rev_after.0),
        },
    )?;

    Ok(CreateOutcome {
        root_before,
        root_after: Some(root_after),
        file_rev_after,
        journal_anchor: Some(journal_anchor),
        committed: Some(committed),
        verdicts,
        dry: false,
    })
}

/// **Guarded `remove`** (d2 §2.5 C3): death of one file under CAS
/// remove-what-you-read + workspace-root, journal the death, and emit the
/// `deleted` change surface.
///
/// Order: path confinement → reserved-journal guard → world guard (§5.1) → load
/// the live file (absent ⇒ `file_not_found`) → the remove-what-you-read CAS
/// (the live rev must equal `if_file_rev`, else refuse citing rev read vs found)
/// → the gate seam over the death's before-state → unlink → root advance →
/// death Delta → journal row (`after=absent`).
///
/// # Errors
/// `bad_path`, `bad_request` (reserved journal), `root_mismatch`,
/// `file_not_found` (nothing to remove), `cas_mismatch` (the file drifted from
/// the read rev — taxonomy row 14, recovery `refresh`), or an I/O failure. In
/// every error case nothing was removed and no journal row was written.
pub fn remove(
    root: &fs::WorkspaceRoot,
    seq: u64,
    args: &RemoveArgs,
    rulesets: &[policy::CompiledRuleset],
) -> Result<RemoveOutcome, Box<ErrorBody>> {
    let fs_path = FsPath::new(&args.path.0);
    path_confined(&args.path)?;
    reserved_journal_guard(fs_path)?;

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

    // The gate seam over the death's before-state (what is being removed).
    let verdicts = evaluate_verdicts(rulesets, &before_doc);

    if args.dry {
        return Ok(RemoveOutcome {
            root_before,
            root_after: None,
            file_rev_before: current,
            journal_anchor: None,
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
    let journal_anchor = journal_write(
        root,
        "remove",
        &args.path,
        args.actor.as_deref(),
        args.now.as_deref(),
        &root_before,
        &root_after,
        receipt::journal::FileTransition {
            before: Some(&current.0),
            after: None,
        },
    )?;

    Ok(RemoveOutcome {
        root_before,
        root_after: Some(root_after),
        file_rev_before: current,
        journal_anchor: Some(journal_anchor),
        committed: Some(committed),
        verdicts,
        dry: false,
    })
}

// ---------------------------------------------------------------------------
// Guarded pin lock-write — the `^inputs` splice (d2 §2.5, U2.4)
// ---------------------------------------------------------------------------
//
// `pin` reads a page's `inputs:` manifest, resolves + composes every ref, and
// splices the `^inputs` lock in ONE CAS-guarded write minting one receipt (d2
// §2.5). The manifest read, resolve, compose, `check:` evaluation, merge law,
// and the lock's byte rendering all live in `crates/pin` (the orchestration).
// This is the WRITE choke-point half: a core span splice that lands the
// engine-COMPUTED block bytes and journals the write — mirroring the
// create/remove shape, so the one-write-shape law holds (no second write path).

/// One `pin` lock-write request (d2 §2.5): a COMPUTED span edit that splices the
/// `^inputs` lock into `path`. `span` is the pre-batch byte range the new block
/// replaces — an empty range (`start == end`) at the append point BIRTHS the
/// block. The span is engine-computed from parse (source 1), never
/// client-supplied, so the D-C1 "no request-side span" law holds: `pin` is a
/// core op, not a wire `splice`. `if_file_rev` is the pinning page's whole-file
/// rev the pin read (write-what-you-read CAS); `if_root` the §5.1 world guard;
/// `before_rev`/`after_rev` the lock block's node rev transition for the journal.
#[derive(Debug, Clone)]
pub struct PinLockArgs {
    /// Frame correlation token — recorded only.
    pub id: Option<u64>,
    /// The pinning page the `^inputs` lock lives in (workspace-confined).
    pub path: Path,
    pub actor: Option<String>,
    pub now: Option<String>,
    /// The optional §5.1 world guard: refuse if the ambient root differs.
    pub if_root: Option<Root>,
    /// The pinning page's whole-file rev the pin computed against — the
    /// write-what-you-read CAS; a drift refuses `cas_mismatch`.
    pub if_file_rev: NodeRev,
    /// The pre-batch byte range the new block replaces (`start == end` = birth).
    pub span: Span,
    /// The new `^inputs` block bytes (fence to fence), engine-rendered by `pin`.
    pub new_text: String,
    /// The lock block's node rev BEFORE the write (empty-span rev on a birth).
    pub before_rev: NodeRev,
    /// The lock block's node rev AFTER the write.
    pub after_rev: NodeRev,
    /// Dry run — everything except disk (no bytes, no journal row, no advance).
    pub dry: bool,
}

/// The outcome of a guarded `pin` lock-write. Absences mirror [`CreateOutcome`]:
/// `root_after`/`journal_anchor`/`committed` are `None` on a dry run.
#[derive(Debug)]
pub struct PinLockOutcome {
    pub root_before: Root,
    pub root_after: Option<Root>,
    /// The pinning page's whole-file rev before the write (the CAS-confirmed rev).
    pub file_rev_before: NodeRev,
    /// The pinning page's whole-file rev after the lock landed.
    pub file_rev_after: NodeRev,
    pub journal_anchor: Option<String>,
    pub committed: Option<DeltaFrame>,
    pub dry: bool,
}

/// **Guarded `pin` lock-write** (d2 §2.5, U2.4): splice the engine-computed
/// `^inputs` lock into `path` under CAS write-what-you-read + workspace-root,
/// journal ONE `splice` row, and emit the `modified` change surface.
///
/// Order: path confinement → reserved-journal guard → world guard (§5.1) → load
/// the page → the write-what-you-read CAS (the live rev must equal `if_file_rev`)
/// → span-bound + char-boundary guard → apply the span replacement in memory →
/// [`fs::replace_file`] (the atomic overwrite) → root advance → splice Delta →
/// journal row (`op=splice`, `edits=1` naming the `^inputs` lock). `dry: true`
/// runs everything except disk.
///
/// # Errors
/// `bad_path`, `bad_request` (reserved journal, or a bad computed span),
/// `root_mismatch` (stale world guard), `cas_mismatch` (the page drifted from
/// the read rev), or an I/O failure. In every error case nothing was written and
/// no journal row was appended.
pub fn pin_lock(
    root: &fs::WorkspaceRoot,
    seq: u64,
    args: &PinLockArgs,
) -> Result<PinLockOutcome, Box<ErrorBody>> {
    let fs_path = FsPath::new(&args.path.0);
    path_confined(&args.path)?;
    reserved_journal_guard(fs_path)?;

    let before_doc = load_doc(root, &args.path)?;
    let file_rev_before = NodeRev(before_doc.root.node_rev.0.clone());

    let root_before = ambient_root(root)?;
    world_guard(args.if_root.as_ref(), &root_before)?;

    // write-what-you-read CAS: the page must still carry the rev the pin read
    // (a drift means the compose the lock records was against stale bytes).
    if args.if_file_rev != file_rev_before {
        return Err(cas_mismatch(&args.if_file_rev, &file_rev_before));
    }

    // The span is engine-computed, but guard it defensively: in bounds and on
    // char boundaries so the splice can never corrupt bytes.
    let start = usize::try_from(args.span.0).unwrap_or(usize::MAX);
    let end = usize::try_from(args.span.1).unwrap_or(usize::MAX);
    let raw = &before_doc.raw;
    if start > end || end > raw.len() || !raw.is_char_boundary(start) || !raw.is_char_boundary(end)
    {
        return Err(bad_request(
            "pin lock span is out of bounds or splits a character",
        ));
    }
    let mut new_raw = String::with_capacity(raw.len() + args.new_text.len());
    new_raw.push_str(&raw[..start]);
    new_raw.push_str(&args.new_text);
    new_raw.push_str(&raw[end..]);
    let after_doc = build_doc(&args.path, &new_raw);
    let file_rev_after = NodeRev(after_doc.root.node_rev.0.clone());

    if args.dry {
        return Ok(PinLockOutcome {
            root_before,
            root_after: None,
            file_rev_before,
            file_rev_after,
            journal_anchor: None,
            committed: None,
            dry: true,
        });
    }

    fs::replace_file(root, fs_path, &new_raw).map_err(|e| io_to_wire(&e))?;
    let root_after = ambient_root(root)?;

    let files = model::delta::file_delta(Some(&before_doc), Some(&after_doc))
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
    let journal_anchor = pin_journal_write(root, args, &root_before, &root_after)?;

    Ok(PinLockOutcome {
        root_before,
        root_after: Some(root_after),
        file_rev_before,
        file_rev_after,
        journal_anchor: Some(journal_anchor),
        committed: Some(committed),
        dry: false,
    })
}

/// Journal one guarded `pin`: an `op=splice` row with a single edit naming the
/// `^inputs` lock (target `^inputs`, the block's before→after rev). The row
/// carries BOTH roots (chain continuity) and appends to the reserved
/// root-EXCLUDED journal via the receipt-engine append. Returns the row anchor.
fn pin_journal_write(
    root: &fs::WorkspaceRoot,
    args: &PinLockArgs,
    root_before: &Root,
    root_after: &Root,
) -> Result<String, Box<ErrorBody>> {
    let seq = next_journal_seq(root)?;
    // The lock block has no request-side ref grammar; name it by its `^inputs`
    // anchor for the journal display (a whole-slot write of the lock).
    let target = SecRef::Anchor {
        anchor: "inputs".to_string(),
    };
    let shape = EditShape::Put {
        at: PutAt::All,
        text: String::new(),
    };
    let line = receipt::journal::render_row(&receipt::journal::JournalRow {
        seq,
        op: "splice",
        path: &args.path.0,
        actor: args.actor.as_deref(),
        now: args.now.as_deref(),
        root_before: &root_before.0,
        root_after: &root_after.0,
        file: None,
        edits: vec![receipt::EditFact {
            target: &target,
            shape: &shape,
            before: &args.before_rev,
            after: &args.after_rev,
        }],
    });
    fs::append_line(root, FsPath::new(fs::domain::RESERVED_JOURNAL_PATH), &line)
        .map_err(|e| io_to_wire(&e))?;
    Ok(receipt::anchor(seq))
}

/// Workspace-root confinement (d2 §2.5 C3 "+ workspace-root"): the same §1
/// path law the strict decode enforces — no absolute path, no `.`/`..`/empty
/// segment — so a `create`/`remove` can never escape the root via `root.join`.
/// A violation is `bad_path`, echoing the offending path.
fn path_confined(path: &Path) -> Result<(), Box<ErrorBody>> {
    let s = &path.0;
    let violates = s.is_empty()
        || s.starts_with('/')
        || s.split('/')
            .any(|seg| seg.is_empty() || seg == "." || seg == "..");
    if violates {
        let mut e = ErrorBody::new(ErrorCode::BadPath);
        e.path = Some(path.clone());
        return Err(Box::new(e));
    }
    Ok(())
}

/// The reserved receipt journal is writable ONLY by the receipt engine (d2
/// §2.1) — a guarded `create`/`remove` targeting it is a tamper attempt,
/// refused with a teaching `bad_request` (the same restriction `splice` makes,
/// shared via `fs::domain::is_reserved_journal` so the two cannot drift).
fn reserved_journal_guard(path: &FsPath) -> Result<(), Box<ErrorBody>> {
    if fs::domain::is_reserved_journal(path) {
        return Err(bad_request(format!(
            "refused: {} is the reserved receipt journal — writable only by the \
             receipt engine (d2 §2.1); a guarded create/remove targeting it is a tamper attempt",
            fs::domain::RESERVED_JOURNAL_PATH
        )));
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
    if let model::NodeKind::Document { path: p, .. } = &mut doc.root.kind {
        p.clone_from(&path.0);
    }
    doc
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

/// Journal one guarded birth/death: render the row through `receipt::journal`
/// (BOTH roots + the whole-file transition, `edits=0`) and append it to the
/// reserved root-EXCLUDED journal page via the receipt-engine append
/// (`fs::append_line`). The next `seq` is derived from the page itself (the
/// journal is the only durable home of its own counter). Returns the row anchor.
#[allow(clippy::too_many_arguments)]
fn journal_write(
    root: &fs::WorkspaceRoot,
    op: &str,
    path: &Path,
    actor: Option<&str>,
    now: Option<&str>,
    root_before: &Root,
    root_after: &Root,
    file: receipt::journal::FileTransition<'_>,
) -> Result<String, Box<ErrorBody>> {
    let seq = next_journal_seq(root)?;
    let line = receipt::journal::render_row(&receipt::journal::JournalRow {
        seq,
        op,
        path: &path.0,
        actor,
        now,
        root_before: &root_before.0,
        root_after: &root_after.0,
        file: Some(file),
        edits: Vec::new(),
    });
    fs::append_line(root, FsPath::new(fs::domain::RESERVED_JOURNAL_PATH), &line)
        .map_err(|e| io_to_wire(&e))?;
    Ok(receipt::anchor(seq))
}

/// The next journal row counter: one past the highest `r-NNNNNN` the reserved
/// page already carries (an absent page starts at 1). The journal is the sole
/// durable home of this counter (no separate on-disk sequence, §14).
fn next_journal_seq(root: &fs::WorkspaceRoot) -> Result<u64, Box<ErrorBody>> {
    let page = root.0.join(fs::domain::RESERVED_JOURNAL_PATH);
    let text = match std::fs::read_to_string(&page) {
        Ok(t) => t,
        Err(e) if e.kind() == ErrorKind::NotFound => String::new(),
        Err(e) => return Err(io_to_wire(&e)),
    };
    let max = receipt::journal::parse_rows(&text)
        .iter()
        .filter_map(|r| {
            r.anchor
                .strip_prefix("r-")
                .and_then(|n| n.parse::<u64>().ok())
        })
        .max()
        .unwrap_or(0);
    Ok(max + 1)
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

/// Per-target BEFORE facts + the wire→model edit conversion, request order
/// (§4.4: armed edits align 1:1 with request edits) — resolution failures
/// name the failing target exactly (candidates in THE grammar).
fn model_edits_and_before_facts(
    doc: &model::Document,
    edits: &[Edit],
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
                    model::ResolveError::NotFound => ErrorBody::new(ErrorCode::RefNotFound),
                    model::ResolveError::Ambiguous(c) => ambiguous(&edit.target, doc, &c),
                })
            })?
        };
        before_facts.push(before);
        model_edits.push(model::Edit {
            target,
            edit: match &edit.edit {
                EditShape::Match { old, new } => model::EditKind::Match {
                    old: old.clone(),
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
) -> model::Document {
    let after_raw = apply_validated(&doc.raw, sealed);
    let after_tree = syntax::parse(&after_raw);
    let mut after_doc = model::build(after_raw, after_tree);
    if let model::NodeKind::Document { path: p, .. } = &mut after_doc.root.kind {
        p.clone_from(&path.0);
    }
    after_doc
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
        edits: args
            .edits
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

/// Apply a sealed batch's span edits in memory (disjoint, sorted — applied
/// back-to-front so earlier spans stay valid). The dry/armed-fact twin of
/// fs's staged apply; the real bytes land through fs alone.
fn apply_validated(raw: &str, sealed: &model::ValidatedBatch) -> String {
    let mut out = raw.to_string();
    for edit in sealed.edits.iter().rev() {
        out.replace_range(edit.span.clone(), &edit.text);
    }
    out
}

/// The §5.2 failure split, mapped: every refusal verdict to its wire frame
/// (code + REQUIRED recovery + the frozen extras).
fn verdict_to_wire(
    verdict: &model::SpliceVerdict,
    args: &SpliceArgs,
    doc: &model::Document,
    before_facts: &[model::Target],
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
        model::SpliceVerdict::RefNotFound => ErrorBody::new(ErrorCode::RefNotFound),
        model::SpliceVerdict::Ambiguous(candidates) => {
            // Name each duplicate by node index + ^block via the shared helper
            // (§2.1 / d1 teaching refusal). In the splice flow this arm is
            // normally pre-empted by the per-target resolution in
            // `model_edits_and_before_facts`; routing it through `ambiguous` keeps
            // both refusal sites identical. The offending target is the first
            // edit that resolves ambiguously.
            let offending = args.edits.iter().map(|e| &e.target).find(|t| {
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
            let overlapping: Vec<SecRef> = args
                .edits
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
    // contract fail-loud; a refusal here means no byte landed.
    fs::apply_batch(
        root,
        FsPath::new(&req.content_path),
        req.receipt.as_ref().map(|(rp, _)| FsPath::new(rp.as_str())),
        &sealed,
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
    use std::path::PathBuf;

    use wire::{Edit, EditShape, ErrorCode, Path, SecRef};

    use super::{SpliceArgs, splice};

    fn journal_splice(dry: bool) -> SpliceArgs {
        SpliceArgs {
            id: None,
            path: Path(fs::domain::RESERVED_JOURNAL_PATH.to_string()),
            actor: Some("mallory".into()),
            now: None,
            receipt: None,
            if_root: None,
            dry,
            // An ordinary content edit aimed at the journal — a forged-row attempt.
            edits: vec![Edit {
                target: SecRef::Hpath { hpath: Vec::new() },
                edit: EditShape::Put {
                    at: wire::PutAt::End,
                    text: "- op=splice root_before=b3:x root_after=b3:y edits=0 ^r-000999".into(),
                },
                if_node_rev: None,
            }],
        }
    }

    /// F4 / d2 §2.1: an ordinary `^put`/splice whose target is the reserved
    /// journal refuses with a teaching `bad_request` — BEFORE any disk touch,
    /// so the fake root need not resolve. The receipt engine's own append rides
    /// `args.receipt` (engine-rendered) and is unaffected by this restriction.
    #[test]
    fn ordinary_splice_at_journal_path_refuses() {
        let root = fs::WorkspaceRoot(PathBuf::from("/nonexistent-workspace-u2-1"));
        let err = splice(&root, 0, &journal_splice(false), &[])
            .expect_err("a splice targeting the reserved journal must refuse");
        assert_eq!(err.code, ErrorCode::BadRequest);
        assert!(
            err.message
                .as_deref()
                .is_some_and(|m| m.contains(fs::domain::RESERVED_JOURNAL_PATH)
                    && m.contains("receipt engine")),
            "the refusal teaches: names the reserved path + receipt-engine-only rule: {:?}",
            err.message
        );
    }

    /// The restriction holds on a DRY run too — a rehearsal of a forbidden
    /// write is still forbidden (never a silent "would-succeed").
    #[test]
    fn dry_splice_at_journal_path_also_refuses() {
        let root = fs::WorkspaceRoot(PathBuf::from("/nonexistent-workspace-u2-1"));
        let err = splice(&root, 0, &journal_splice(true), &[])
            .expect_err("dry splice at the journal must also refuse");
        assert_eq!(err.code, ErrorCode::BadRequest);
    }
}

/// Guarded `create`/`remove` — file birth and death (d2 §2.5 C3, U2.6). The
/// named gates: create-existing-path refuses (CAS), remove-after-drift refuses
/// citing rev, both journal rows carry the `before=absent`/`after=absent`
/// shape, and both refusals map to their taxonomy rows (`cas_mismatch` +
/// recovery `refresh`, rows 13/14).
#[cfg(test)]
mod guarded_create_remove {
    use wire::{ErrorCode, FileChange, NodeRev, Path, Recovery};

    use super::{CreateArgs, RemoveArgs, create, remove};

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

    fn journal_text(root: &fs::WorkspaceRoot) -> String {
        std::fs::read_to_string(root.0.join(fs::domain::RESERVED_JOURNAL_PATH)).unwrap_or_default()
    }

    /// Birth: `create` lands the file, advances the root, emits a `created`
    /// Delta (`file_rev_before` absent — the change surface's before=absent),
    /// and journals a row whose whole-file token reads `before=absent`, carries
    /// BOTH roots, and counts `edits=0`.
    #[test]
    fn create_births_file_and_journals_before_absent() {
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

        // (d) the journal row: op=create, before=absent, both roots, edits=0.
        let row = journal_text(&root);
        assert!(row.contains("op=create path=notes/new.md"), "{row}");
        assert!(
            row.contains(" before=absent "),
            "create row before=absent: {row}"
        );
        assert!(
            row.contains(&format!(
                "root_before={} root_after={}",
                out.root_before.0, root_after.0
            )),
            "row carries BOTH roots: {row}"
        );
        assert!(
            row.contains("edits=0"),
            "whole-file create has no node edits: {row}"
        );
        assert_eq!(out.journal_anchor.as_deref(), Some("r-000001"));
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
    /// emits a `deleted` Delta (`file_rev_after` absent — after=absent), and
    /// journals a row whose whole-file token reads `after=absent`.
    #[test]
    fn remove_death_and_journals_after_absent() {
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

        // journal: the create row then the remove row (after=absent).
        let text = journal_text(&root);
        assert!(text.contains("op=remove path=notes/new.md"), "{text}");
        let remove_row = text.lines().find(|l| l.contains("op=remove")).unwrap();
        assert!(
            remove_row.contains(" after=absent"),
            "remove row after=absent: {remove_row}"
        );
        assert_eq!(out.journal_anchor.as_deref(), Some("r-000002"));
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

        // The drift refusal wrote nothing: only the create row is journalled.
        assert_eq!(
            journal_text(&root)
                .lines()
                .filter(|l| l.contains("^r-"))
                .count(),
            1,
            "a refused remove appends no journal row"
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

    /// The reserved receipt journal is receipt-engine-only (d2 §2.1): a guarded
    /// create/remove targeting it refuses with a teaching `bad_request` — the
    /// same restriction `splice` makes, so the journal cannot be tampered.
    #[test]
    fn guarded_ops_refuse_reserved_journal() {
        let (_dir, root) = ws();
        let jp = fs::domain::RESERVED_JOURNAL_PATH;
        let ce = create(&root, 0, &create_args(jp, "- forged ^r-000999"), &[]).unwrap_err();
        assert_eq!(ce.code, ErrorCode::BadRequest);
        assert!(
            ce.message
                .as_deref()
                .is_some_and(|m| m.contains("receipt engine")),
            "teaching refusal names the engine-only rule: {:?}",
            ce.message
        );
        let re = remove(&root, 0, &remove_args(jp, "deadbeefdeadbeef"), &[]).unwrap_err();
        assert_eq!(re.code, ErrorCode::BadRequest);
    }

    /// Dry runs touch no disk (§4.4 batch law, applied to birth/death): a dry
    /// create writes no file and no journal row; a dry remove leaves the file
    /// and journals nothing. Both still run the gate seam (empty ⇒ `[]`).
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
        assert!(dry_born.journal_anchor.is_none() && dry_born.verdicts.is_empty());
        assert!(
            journal_text(&root).is_empty(),
            "dry create writes no journal row"
        );

        // A real file to dry-remove.
        let born = create(&root, 0, &create_args("notes/new.md", "# New\n"), &[]).unwrap();
        let before = journal_text(&root);
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
        assert!(dry_dead.committed.is_none() && dry_dead.journal_anchor.is_none());
        assert_eq!(
            journal_text(&root),
            before,
            "dry remove writes no journal row"
        );
    }

    /// The journal integration composes with U2.1's chain detector: a `create`
    /// then a `remove` leave a CONTINUOUS chain (`root_after(1)` ==
    /// `root_before(2)`), because the root-EXCLUDED journal append never moves
    /// the root it just recorded.
    #[test]
    fn create_then_remove_leaves_a_continuous_chain() {
        let (_dir, root) = ws();
        let born = create(&root, 0, &create_args("notes/new.md", "# New\n"), &[]).unwrap();
        remove(
            &root,
            0,
            &remove_args("notes/new.md", &born.file_rev_after.0),
            &[],
        )
        .unwrap();

        let rows = receipt::journal::parse_rows(&journal_text(&root));
        assert_eq!(rows.len(), 2, "one row per guarded write");
        let report = receipt::journal::check_chain(&rows);
        assert!(
            report.is_green(),
            "birth→death chain is continuous: {report:?}"
        );
    }
}
