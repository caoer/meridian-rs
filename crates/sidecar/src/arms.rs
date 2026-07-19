//! The op arms: one function per armed op, `model`/`fs` in, wire body out.
//! Wiring only — parsing is `syntax`'s, tree law is `model`'s, projection is
//! `wire-map`'s, disk is `fs`'s.

use std::io::ErrorKind;

use wire::{ErrorBody, ErrorCode, NodeRev, Op, Path, ResponseBody, Root, SecRef, Span};

/// Route one validated request to its arm. `id` is the frame's correlation
/// token — the splice arm records it into the receipt line (§6.1 fact list);
/// no other arm reads it.
pub(crate) fn dispatch(
    root: &fs::WorkspaceRoot,
    epoch: &mut crate::ring::RootRing,
    id: Option<u64>,
    op: Op,
) -> Result<ResponseBody, Box<ErrorBody>> {
    match op {
        Op::Hello { .. } => Ok(hello(root)),
        Op::Toc { path } => toc(root, &path),
        Op::Cat { path, sec } => cat(root, &path, sec),
        Op::Extract { path, kinds } => extract(root, &path, kinds),
        Op::Resolve {
            from,
            r#ref,
            content,
        } => resolve(root, &from, &r#ref, content.unwrap_or(false)),
        Op::Root => root_op(root, epoch),
        Op::Diff { from_root, to_root } => diff_op(root, epoch, &from_root, &to_root),
        Op::Links { path, require_root } => links_op(root, epoch, path.as_ref(), require_root),
        // Armed, but registered at the serve layer (the loop owns the
        // subscription list) — unreachable through serve; answering internal
        // (never a panic) keeps a future misroute non-fatal.
        Op::Sub { .. } => Err(Box::new(ErrorBody::new(ErrorCode::Internal))),
        Op::Splice {
            path,
            actor,
            now,
            receipt,
            if_root,
            dry,
            edits,
        } => splice_op(
            root,
            epoch,
            &SpliceArgs {
                id,
                path,
                actor,
                now,
                receipt,
                if_root,
                dry: dry.unwrap_or(false),
                edits,
            },
        ),
    }
}

/// One splice request's decoded fields, bundled (the arm reads them as a
/// unit; `id` rides only into the receipt line).
struct SpliceArgs {
    id: Option<u64>,
    path: Path,
    actor: Option<String>,
    now: Option<String>,
    receipt: Option<wire::ReceiptAddr>,
    if_root: Option<Root>,
    dry: bool,
    edits: Vec<wire::Edit>,
}

/// v2 §4.4 the only write op, armed end-to-end (D4-SPLICE): strict-decoded
/// edits → §5.1-ordered validation → the D4 commit seam (`commit_batch`:
/// validate → `fs::apply_batch` → Delta emission) — one exchange, one
/// reparse, one root advance, one Delta. `dry:true` runs everything except
/// disk: same response shape, `root_after:null`, no receipt written, no ring
/// advance, no Delta, no mkdir (zero disk effects means zero).
///
/// The production `apply_batch` caller obligations (F4 seam memo) live HERE:
/// receipt pairing rides `CommitRequest` (fs re-checks fail-loud), the
/// receipt line renders via `crates/receipt` and folds in pre-validation
/// (§6.1 — same sealed batch, ONE root advance), and the receipt parent dir
/// is created on REAL commits only (fs does not mkdir).
fn splice_op(
    root: &fs::WorkspaceRoot,
    epoch: &mut crate::ring::RootRing,
    args: &SpliceArgs,
) -> Result<ResponseBody, Box<ErrorBody>> {
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
    let armed_edits = simulate_armed_edits(&doc, &sealed, &args.edits, &before_facts)?;

    // Dry short-circuit (§4.4 batch law): everything except disk — and
    // therefore no receipt, no root advance, no Delta, no mkdir.
    if args.dry {
        return Ok(ResponseBody::Splice {
            armed: wire::Armed {
                path: args.path.clone(),
                edits: armed_edits,
            },
            receipt: None,
            root_before,
            root_after: None,
            seq: None,
            dry: Some(true),
            verdicts: vec![],
        });
    }

    // REAL commit: render the receipt line (facts about what is being
    // ARMED — §6.1), fold the append, honor the parent-dir obligation,
    // then drive the D4 commit seam (validate → apply → emit).
    let receipt_input = match &args.receipt {
        Some(addr) => Some(receipt_input(root, args, &root_before, &armed_edits, addr)?),
        None => None,
    };
    let frame = crate::commit::commit_batch(
        root,
        epoch,
        &crate::commit::CommitRequest {
            content_path: args.path.0.clone(),
            batch,
            receipt: receipt_input,
            actor: args.actor.clone(),
            now: args.now.clone(),
        },
    )
    .map_err(|e| match e {
        crate::commit::CommitError::Refused(v) => verdict_to_wire(&v, args, &before_facts),
        crate::commit::CommitError::Env(err) => err,
        crate::commit::CommitError::Io(err) => {
            let mut w = ErrorBody::new(ErrorCode::IoError);
            w.cause = Some(err.to_string());
            Box::new(w)
        }
    })?;

    // The receipt FACT from the true post-state: the anchor resolved in the
    // just-committed receipt file (host-block-leaf grain).
    let receipt_fact = match &args.receipt {
        Some(addr) => {
            let receipt_doc = load_doc(root, &addr.path)?;
            let target = model::Ref::anchor(addr.anchor.clone())
                .map_err(|_| crate::bad_request("receipt anchor failed the mint-guard"))?;
            let resolved = model::resolve(&receipt_doc, &target).map_err(|_| {
                crate::bad_request("committed receipt anchor did not resolve — receipt corrupt")
            })?;
            Some(wire::ReceiptFact {
                path: addr.path.clone(),
                anchor: addr.anchor.clone(),
                node_rev: NodeRev(resolved.node_rev.0),
                span_after: Span(resolved.span.start as u64, resolved.span.end as u64),
            })
        }
        None => None,
    };

    Ok(ResponseBody::Splice {
        armed: wire::Armed {
            path: args.path.clone(),
            edits: armed_edits,
        },
        receipt: receipt_fact,
        root_before: frame.delta.root_before.clone(),
        root_after: Some(frame.delta.root_after.clone()),
        seq: Some(frame.delta.seq),
        dry: None,
        verdicts: vec![],
    })
}

/// Per-target BEFORE facts + the wire→model edit conversion, request order
/// (§4.4: armed edits align 1:1 with request edits) — resolution failures
/// name the failing target exactly (candidates in THE grammar).
fn model_edits_and_before_facts(
    doc: &model::Document,
    edits: &[wire::Edit],
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
                wire::EditShape::Match { old, new } => model::EditKind::Match {
                    old: old.clone(),
                    new: new.clone(),
                },
                wire::EditShape::Put { at, text } => model::EditKind::Put {
                    at: match at {
                        wire::PutAt::All => model::PutAt::All,
                        wire::PutAt::Content => model::PutAt::Content,
                        wire::PutAt::End => model::PutAt::End,
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

/// The armed AFTER facts from a real parse of the simulated post-batch bytes
/// (the §4.4 one-reparse law's dry twin) — computed, never
/// arithmetic-shifted.
fn simulate_armed_edits(
    doc: &model::Document,
    sealed: &model::ValidatedBatch,
    edits: &[wire::Edit],
    before_facts: &[model::Target],
) -> Result<Vec<wire::ArmedEdit>, Box<ErrorBody>> {
    let after_raw = apply_validated(&doc.raw, sealed);
    let after_tree = syntax::parse(&after_raw);
    let after_doc = model::build(after_raw, after_tree);
    let mut armed_edits = Vec::with_capacity(edits.len());
    for (edit, before) in edits.iter().zip(before_facts) {
        let target = to_model_ref(&edit.target)?;
        let after = model::resolve(&after_doc, &target).map_err(|_| {
            // A target whose identity does not survive its own edit (e.g. a
            // heading rewritten by put at:all) has no worked armed shape in
            // the frozen text — refuse loud rather than invent one.
            crate::bad_request(
                "target identity does not survive the edit — armed facts unrepresentable",
            )
        })?;
        armed_edits.push(wire::ArmedEdit {
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
    armed_edits: &[wire::ArmedEdit],
    addr: &wire::ReceiptAddr,
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
                            .map(|h| wire::HpathSeg {
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

/// v2 §4.6 the edge map under the §10.1 triple, served from `query` over the
/// domain snapshot (§10.3: facts come from the world model — the §12 hash
/// domain, not the walk plane's addressable superset). `as_of_root` folds the
/// EXACT bytes the answer parses; `live_root` is a fresh fold after the
/// computation — under a concurrent splice the two may differ, which is a
/// legal frame, never an error (§10.1: no lag bounds are promised, ever).
fn links_op(
    root: &fs::WorkspaceRoot,
    epoch: &crate::ring::RootRing,
    path: Option<&Path>,
    require_root: Option<Root>,
) -> Result<ResponseBody, Box<ErrorBody>> {
    let (files, as_of_root) = domain_snapshot(root)?;
    // The Delta counter at as_of_root (§10.1) — sampled with the snapshot;
    // §7.1 per-daemon-epoch semantics ride along unchanged.
    let changes_seq = epoch.seq();
    if let Some(required) = require_root
        && required != as_of_root
    {
        // §10.2: the opt-in strictness refusal — retryable, never silent.
        let mut e = ErrorBody::new(ErrorCode::StaleView);
        e.required = Some(required);
        e.as_of_root = Some(as_of_root.clone());
        e.live_root = Some(as_of_root);
        return Err(Box::new(e));
    }
    let mut index = model::CorpusIndex::new();
    let mut docs = std::collections::BTreeMap::new();
    for (rel, bytes) in files {
        // The fact plane refuses what it cannot parse — loud, never skipped
        // (the walk plane's skip-broken-files posture is resolve's, §4.5).
        let text = String::from_utf8(bytes)
            .map_err(|_| Box::new(ErrorBody::new(ErrorCode::InvalidUtf8)))?;
        let doc = model::build(text.clone(), syntax::parse(&text));
        index.insert(&rel, &doc);
        docs.insert(rel, doc);
    }
    if let Some(p) = path
        && !docs.contains_key(&p.0)
    {
        let mut e = ErrorBody::new(ErrorCode::FileNotFound);
        e.path = Some(p.clone());
        return Err(Box::new(e));
    }
    let map = query::links(&index, &docs, path.map(|p| p.0.as_str()));
    let live_root = domain_snapshot(root)?.1;
    Ok(ResponseBody::Links {
        as_of_root,
        live_root,
        changes_seq,
        files: map
            .into_iter()
            .map(|(p, e)| {
                (
                    p,
                    wire::FileLinks {
                        resolved: e.resolved,
                        unresolved: e.unresolved,
                    },
                )
            })
            .collect(),
    })
}

/// v2 §4.7: the current workspace root cursor (computed fresh from disk —
/// truth over cache while no watcher runs) + this epoch's `seq`.
fn root_op(
    root: &fs::WorkspaceRoot,
    epoch: &crate::ring::RootRing,
) -> Result<ResponseBody, Box<ErrorBody>> {
    Ok(ResponseBody::Root {
        root: ambient_root(root)?,
        seq: epoch.seq(),
    })
}

/// v2 §4.7/§7.3 replay over this epoch's retained history. Until rung 4
/// emits batches the history is exactly the current root: same-root diff is
/// truthfully empty, anything else — stale, evicted, previous-epoch,
/// backwards — is `root_unknown` → full resync (degrade to re-derive, never
/// to wrong data).
fn diff_op(
    root: &fs::WorkspaceRoot,
    epoch: &crate::ring::RootRing,
    from: &Root,
    to: &Root,
) -> Result<ResponseBody, Box<ErrorBody>> {
    let current = ambient_root(root)?;
    match epoch.batches_between(from, to, &current) {
        Some(batches) => Ok(ResponseBody::Diff { batches }),
        None => Err(Box::new(ErrorBody::new(ErrorCode::RootUnknown))),
    }
}

/// v2 §3.2: proto in effect, server name, the complete armed cap set. `root`
/// is optional in the hello body — present when the walk computes (the first
/// ambient root), honestly absent on any I/O failure.
fn hello(root: &fs::WorkspaceRoot) -> ResponseBody {
    ResponseBody::Hello {
        proto: crate::PROTO,
        server: crate::SERVER_NAME.to_string(),
        caps: crate::CAPS.iter().map(ToString::to_string).collect(),
        root: ambient_root(root).ok(),
    }
}

/// The ambient workspace root (v2 §4.1/§12): the §12 hash domain's file
/// bytes folded through `model::merkle_root` — the one blake3 home — with the
/// domain config's prefix version.
pub(crate) fn ambient_root(root: &fs::WorkspaceRoot) -> Result<Root, Box<ErrorBody>> {
    Ok(domain_snapshot(root)?.1)
}

/// The domain files as `(workspace-relative path, raw bytes)` pairs.
type DomainFiles = Vec<(String, Vec<u8>)>;

/// The §12 hash-domain snapshot: every domain file's bytes + the root folded
/// over exactly those bytes — one read, one fold, so a consumer (the `links`
/// fact plane) parses the same bytes its `as_of_root` describes and the
/// answer cannot drift from its stamp.
fn domain_snapshot(root: &fs::WorkspaceRoot) -> Result<(DomainFiles, Root), Box<ErrorBody>> {
    let io_err = |e: std::io::Error| {
        let mut err = ErrorBody::new(ErrorCode::IoError);
        err.cause = Some(e.to_string());
        Box::new(err)
    };
    let domain = fs::domain::Domain::load(root).map_err(io_err)?;
    let rels = fs::hash_domain(root, &domain).map_err(io_err)?;
    let mut files = Vec::with_capacity(rels.len());
    for rel in rels {
        let bytes = std::fs::read(root.0.join(&rel)).map_err(io_err)?;
        files.push((rel.to_string_lossy().replace('\\', "/"), bytes));
    }
    let entries: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(p, b)| (p.as_str(), b.as_slice()))
        .collect();
    let folded = Root(model::merkle_root(&entries, domain.version()).0);
    Ok((files, folded))
}

/// `fs::load` with the §8 error split: `file_not_found` (env — the file is
/// gone, path echoed), `invalid_utf8` (refused, never lossy-decoded),
/// `io_error{cause}` otherwise.
fn load_doc(root: &fs::WorkspaceRoot, path: &Path) -> Result<model::Document, Box<ErrorBody>> {
    fs::load(root, std::path::Path::new(&path.0)).map_err(|e| {
        Box::new(match e.kind() {
            ErrorKind::NotFound => {
                let mut err = ErrorBody::new(ErrorCode::FileNotFound);
                err.path = Some(path.clone());
                err
            }
            ErrorKind::InvalidData => ErrorBody::new(ErrorCode::InvalidUtf8),
            _ => {
                let mut err = ErrorBody::new(ErrorCode::IoError);
                err.cause = Some(e.to_string());
                err
            }
        })
    })
}

/// v2 §4.1: the map — header `file_rev` (the document root's rev, same family,
/// whole-file bytes) + ambient `root`, rows from the `wire-map` projection.
fn toc(root: &fs::WorkspaceRoot, path: &Path) -> Result<ResponseBody, Box<ErrorBody>> {
    let doc = load_doc(root, path)?;
    Ok(ResponseBody::Toc {
        path: path.clone(),
        file_rev: NodeRev(doc.root.node_rev.0.clone()),
        root: ambient_root(root)?,
        nodes: wire_map::project_toc(&doc),
    })
}

/// v2 §4.2: full span bytes (heading-inclusive), rev over precisely those
/// bytes. `sec` absent → whole file + `file_rev` riding the `node_rev` slot.
fn cat(
    root: &fs::WorkspaceRoot,
    path: &Path,
    sec: Option<SecRef>,
) -> Result<ResponseBody, Box<ErrorBody>> {
    let doc = load_doc(root, path)?;
    let Some(sec) = sec else {
        return Ok(ResponseBody::Cat {
            span: Span(0, doc.raw.len() as u64),
            node_rev: NodeRev(doc.root.node_rev.0.clone()),
            content: doc.raw.clone(),
        });
    };
    let target = model::resolve(&doc, &to_model_ref(&sec)?).map_err(|e| {
        Box::new(match e {
            model::ResolveError::NotFound => ErrorBody::new(ErrorCode::RefNotFound),
            model::ResolveError::Ambiguous(candidates) => ambiguous(&sec, candidates.len()),
        })
    })?;
    Ok(ResponseBody::Cat {
        span: Span(target.span.start as u64, target.span.end as u64),
        node_rev: NodeRev(target.node_rev.0),
        content: doc.raw[target.span].to_string(),
    })
}

/// The wire→model ref bridge (the crates never share a type — no-serde law).
/// The anchor form re-passes the mint-guard; the strict decode already
/// refused out-of-charset ids, so this is the belt to that suspender.
fn to_model_ref(sec: &SecRef) -> Result<model::Ref, Box<ErrorBody>> {
    Ok(match sec {
        SecRef::Hpath { hpath } => model::Ref::Hpath(
            hpath
                .iter()
                .map(|s| model::HpathSeg {
                    h: s.h.clone(),
                    n: s.n,
                })
                .collect(),
        ),
        SecRef::Anchor { anchor } => model::Ref::anchor(anchor.clone()).map_err(|bad| {
            crate::bad_request(format!(
                "block id outside the one charset [A-Za-z0-9-] (§2.4): `{id}`",
                id = bad.id
            ))
        })?,
        SecRef::FmKey { fm_key } => model::Ref::FmKey(fm_key.clone()),
    })
}

/// `ambiguous_ref` (§2.1: the strict plane never silently picks) with
/// `candidates` in THE grammar: hpath duplicates are nameable exactly by
/// occurrence index on the final segment; duplicate block ids have no exact
/// §2.1 spelling per target (the occurrence index is hpath-segment syntax
/// only), so `candidates` stays type-level EMPTY — `[]`, never prose inside
/// the grammar field — with the human message carrying the count.
fn ambiguous(sec: &SecRef, count: usize) -> ErrorBody {
    let mut e = ErrorBody::new(ErrorCode::AmbiguousRef);
    match sec {
        SecRef::Hpath { hpath } => {
            e.candidates = Some(
                (1..=count)
                    .map(|n| {
                        let mut segs = hpath.clone();
                        if let Some(last) = segs.last_mut() {
                            last.n = Some(u32::try_from(n).unwrap_or(u32::MAX));
                        }
                        SecRef::Hpath { hpath: segs }
                    })
                    .collect(),
            );
        }
        SecRef::Anchor { .. } | SecRef::FmKey { .. } => {
            e.candidates = Some(Vec::new());
            e.message = Some(format!("{count} duplicate targets in one file"));
        }
    }
    e
}

/// v2 §4.3: the full node inventory via the `wire-map` projection, `kinds`
/// filtered (values already validated against the closed enum at decode).
fn extract(
    root: &fs::WorkspaceRoot,
    path: &Path,
    kinds: Option<Vec<String>>,
) -> Result<ResponseBody, Box<ErrorBody>> {
    let doc = load_doc(root, path)?;
    let mut nodes = wire_map::project(&doc);
    if let Some(kinds) = kinds {
        let keep: Vec<wire::NodeKind> = kinds
            .iter()
            .filter_map(|s| serde_json::from_value(serde_json::Value::String(s.clone())).ok())
            .collect();
        nodes.retain(|n| keep.contains(&n.kind));
    }
    Ok(ResponseBody::Nodes {
        path: path.clone(),
        nodes,
    })
}

/// v2 §4.5: the walk plane — best-effort app-compatible two-stage walk over
/// the whole corpus (stage 1 is a vault-namespace question). Location facts
/// only; `content:true` additionally returns the fragment bytes, still no
/// rev. Files that fail to load (unreadable, non-UTF-8) are skipped from the
/// walk corpus — the app indexes nothing it cannot read, and one broken file
/// must not kill resolution for the rest.
fn resolve(
    root: &fs::WorkspaceRoot,
    from: &Path,
    link: &str,
    want_content: bool,
) -> Result<ResponseBody, Box<ErrorBody>> {
    let rels = fs::walk(root).map_err(|e| {
        let mut err = ErrorBody::new(ErrorCode::IoError);
        err.cause = Some(e.to_string());
        err
    })?;
    let mut index = model::CorpusIndex::new();
    let mut docs = std::collections::BTreeMap::new();
    for rel in rels {
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if let Ok(doc) = fs::load(root, &rel) {
            index.insert(&rel_str, &doc);
            docs.insert(rel_str, doc);
        }
    }
    match model::walk::walk(&index, &docs, &from.0, link) {
        Ok(loc) => {
            let content = want_content.then(|| docs[&loc.dest].raw[loc.span.clone()].to_string());
            Ok(ResponseBody::Resolve {
                dest: Path(loc.dest),
                span: Span(loc.span.start as u64, loc.span.end as u64),
                content,
            })
        }
        Err(miss) => {
            let mut e = ErrorBody::new(ErrorCode::RefNotFound);
            e.stage = Some(u32::from(miss.stage.number()));
            e.dest = miss.dest.map(Path);
            Err(Box::new(e))
        }
    }
}
