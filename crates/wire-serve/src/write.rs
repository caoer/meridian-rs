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
        refused => return Err(verdict_to_wire(&refused, args, &before_facts)),
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
        CommitError::Refused(v) => verdict_to_wire(&v, args, &before_facts),
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
        let resolved = model::resolve(doc, &target).map_err(|e| {
            Box::new(match e {
                model::ResolveError::NotFound => ErrorBody::new(ErrorCode::RefNotFound),
                model::ResolveError::Ambiguous(c) => ambiguous(&edit.target, c.len()),
            })
        })?;
        before_facts.push(resolved);
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
            let mut e = ErrorBody::new(ErrorCode::AmbiguousRef);
            e.message = Some(format!(
                "{} duplicate targets in one file",
                candidates.len()
            ));
            e.candidates = Some(Vec::new());
            e
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
