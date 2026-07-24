//! The read-op arms over BORROWED parsed state — one implementation for both
//! hosts. Each arm takes already-built `model` state (a `&Document`, or the
//! `&CorpusIndex` + document map) plus the ambient root as data; the caller
//! obtains that state (the sidecar builds it per request from `fs`; the daemon
//! reuses its warm engine). Parsing is the caller's, projection is `wire-map`'s,
//! edges are `query`'s — these arms only wire model facts into wire bodies.

use std::collections::BTreeMap;

use wire::{ErrorBody, ErrorCode, NodeRev, Path, ResponseBody, Root, SecRef, Span};

use crate::bad_request;

/// wire §4.1 the map: header `file_rev` (the document root's rev over whole-file
/// bytes) + the ambient `root`, rows from the `wire-map` projection. `ambient`
/// is the corpus content-hash cursor the caller already holds (the sidecar folds
/// it from disk; the daemon reads its warm engine's fingerprint).
#[must_use]
pub fn toc(doc: &model::Document, path: &Path, ambient: &Root) -> ResponseBody {
    ResponseBody::Toc {
        path: path.clone(),
        file_rev: NodeRev(doc.root.node_rev.0.clone()),
        root: ambient.clone(),
        nodes: wire_map::project_toc(doc),
    }
}

/// wire §4.2: full span bytes (heading-inclusive), rev over precisely those
/// bytes. `sec` absent → whole file + `file_rev` riding the `node_rev` slot.
///
/// # Errors
/// The `sec` names no node (`ref_not_found`) or names more than one
/// (`ambiguous_ref`), or a malformed anchor id (`bad_request`).
pub fn cat(doc: &model::Document, sec: Option<SecRef>) -> Result<ResponseBody, Box<ErrorBody>> {
    let Some(sec) = sec else {
        return Ok(ResponseBody::Cat {
            span: Span(0, doc.raw.len() as u64),
            node_rev: NodeRev(doc.root.node_rev.0.clone()),
            content: doc.raw.clone(),
        });
    };
    let target = model::resolve(doc, &to_model_ref(&sec)?).map_err(|e| {
        Box::new(match e {
            model::ResolveError::NotFound => ErrorBody::new(ErrorCode::RefNotFound),
            model::ResolveError::Ambiguous(candidates) => ambiguous(&sec, doc, &candidates),
        })
    })?;
    Ok(ResponseBody::Cat {
        span: Span(target.span.start as u64, target.span.end as u64),
        node_rev: NodeRev(target.node_rev.0),
        content: doc.raw[target.span].to_string(),
    })
}

/// wire §4.3: the full node inventory via the `wire-map` projection, `kinds`
/// filtered (values already validated against the closed enum at decode).
///
/// `enrich` (M1 U2, v3 sessions ONLY): attach the host-face addressing facts
/// — dewey `n`, sanitized `hpath_text`, subtree `words` — to heading nodes,
/// so `extract` serves every addressing fact ccc-statusd re-derived
/// host-side. A v2 session never enriches: the new keys are v3-additive and
/// the frozen v2 bytes stay byte-identical (`contract_v2.rs`).
#[must_use]
pub fn extract(
    doc: &model::Document,
    path: &Path,
    kinds: Option<Vec<String>>,
    enrich: bool,
) -> ResponseBody {
    let mut nodes = wire_map::project(doc);
    if let Some(kinds) = kinds {
        let keep: Vec<wire::NodeKind> = kinds
            .iter()
            .filter_map(|s| serde_json::from_value(serde_json::Value::String(s.clone())).ok())
            .collect();
        nodes.retain(|n| keep.contains(&n.kind));
    }
    if enrich {
        let facts = wire_map::facts::read_facts(&wire_map::project_toc(doc), doc.raw.as_bytes());
        let by_span: BTreeMap<(u64, u64), &wire_map::facts::ReadFact> = facts
            .iter()
            .filter(|f| f.depth > 0)
            .map(|f| ((f.span.0, f.span.1), f))
            .collect();
        for node in &mut nodes {
            if node.kind == wire::NodeKind::Heading
                && let Some(fact) = by_span.get(&(node.span.0, node.span.1))
            {
                node.n = Some(fact.n.clone());
                node.hpath_text = Some(fact.hpath.clone());
                node.words = Some(fact.words);
            }
        }
    }
    ResponseBody::Nodes {
        path: path.clone(),
        nodes,
    }
}

/// wire §10.2 the opt-in strictness guard for `links`: refuse when the caller
/// pinned a `require_root` the world no longer meets — retryable, never silent.
/// Checked BEFORE the corpus is built (the sidecar's timing: refuse a stale view
/// without paying the parse), so both hosts call this against the `as_of_root`
/// they already hold, before handing the built corpus to [`links`].
///
/// # Errors
/// `stale_view` when `require_root` is present and differs from `as_of_root`.
pub fn require_root_check(
    require_root: Option<&Root>,
    as_of_root: &Root,
) -> Result<(), Box<ErrorBody>> {
    if let Some(required) = require_root
        && required != as_of_root
    {
        let mut e = ErrorBody::new(ErrorCode::StaleView);
        e.required = Some(required.clone());
        e.as_of_root = Some(as_of_root.clone());
        // At the refusal point no live root was sampled; echo `as_of` for both
        // (wire §10.2 worked frame).
        e.live_root = Some(as_of_root.clone());
        return Err(Box::new(e));
    }
    Ok(())
}

/// wire §4.6 the corpus edge map under the §10.1 staleness triple, served from
/// `query` over the borrowed corpus. `as_of_root` folds the EXACT bytes the
/// answer parses; `live_root` is sampled AFTER the computation — under a
/// concurrent write the two may differ, which is a legal frame, never an error
/// (§10.1: no lag bounds are promised). `live_root` is a closure so it is
/// sampled only on the success path (a `file_not_found` never pays a second
/// fold). Call [`require_root_check`] before this — the §10.2 refusal is
/// checked before the corpus is built, so it is not this arm's.
///
/// # Errors
/// `path` names a file the corpus does not carry (`file_not_found`), or the
/// caller's `live_root` sample fails.
pub fn links(
    index: &model::CorpusIndex,
    docs: &BTreeMap<String, model::Document>,
    path: Option<&Path>,
    as_of_root: Root,
    changes_seq: u64,
    live_root: impl FnOnce() -> Result<Root, Box<ErrorBody>>,
) -> Result<ResponseBody, Box<ErrorBody>> {
    if let Some(p) = path
        && !docs.contains_key(&p.0)
    {
        let mut e = ErrorBody::new(ErrorCode::FileNotFound);
        e.path = Some(p.clone());
        return Err(Box::new(e));
    }
    let map = query::links(index, docs, path.map(|p| p.0.as_str()));
    let live = live_root()?;
    Ok(ResponseBody::Links {
        as_of_root,
        live_root: live,
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

/// wire §4.5 the walk plane: best-effort app-compatible two-stage walk over the
/// corpus. Location facts only; `want_content` additionally returns the fragment
/// bytes, still no rev. The corpus here is the walk-plane SUPERSET (the caller
/// builds it skip-broken — the app indexes nothing it cannot read), which is a
/// different corpus from the §12 hash-domain fact plane [`links`] reads.
///
/// # Errors
/// The linkpath resolves to no file (`ref_not_found` stage 1) or the file is
/// found but the subpath is not (`ref_not_found` stage 2, with `dest`).
pub fn resolve(
    index: &model::CorpusIndex,
    docs: &BTreeMap<String, model::Document>,
    from: &Path,
    link: &str,
    want_content: bool,
) -> Result<ResponseBody, Box<ErrorBody>> {
    match model::walk::walk(index, docs, &from.0, link) {
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

/// The wire→model ref bridge (the crates never share a type — no-serde law).
/// The anchor form re-passes the mint-guard; the strict decode already refused
/// out-of-charset ids, so this is the belt to that suspender. `pub` because the
/// sidecar's write path (splice) resolves the same §2.1 targets through it — one
/// bridge, not two.
///
/// # Errors
/// An anchor id outside the block-id charset `[A-Za-z0-9-]` (`bad_request`).
pub fn to_model_ref(sec: &SecRef) -> Result<model::Ref, Box<ErrorBody>> {
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
            bad_request(format!(
                "block id outside the one charset [A-Za-z0-9-] (§2.4): `{id}`",
                id = bad.id
            ))
        })?,
        SecRef::FmKey { fm_key } => model::Ref::FmKey(fm_key.clone()),
    })
}

/// `ambiguous_ref` (§2.1: the strict plane never silently picks) — the U2.2
/// refuse-ambiguous-only refusal, naming EACH duplicate by both addressable
/// disambiguators the teaching refusal offers: its node index (`n=`, the §2.1
/// occurrence index) and its block id (`^block`) when it carries one. `candidates`
/// holds the machine-addressable `n=` forms (hpath duplicates are nameable
/// exactly by occurrence index on the final segment; duplicate block ids share
/// one id that cannot disambiguate them, so their `candidates` stays type-level
/// EMPTY — `[]`, never prose inside the grammar field). `message` carries the d1
/// teaching refusal verbatim (both spellings interpolated, "Unambiguous writes to
/// this file remain served"). `pub` because the sidecar's write path raises the
/// same refusal against a splice target — one spelling of it, shared.
#[must_use]
pub fn ambiguous(sec: &SecRef, doc: &model::Document, candidates: &[model::Target]) -> ErrorBody {
    let mut e = ErrorBody::new(ErrorCode::AmbiguousRef);
    // One naming per duplicate: node index (1-based occurrence) + its ^block.
    let named: Vec<model::selector::AmbiguityCandidate> = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| model::selector::AmbiguityCandidate {
            node_index: u32::try_from(i + 1).unwrap_or(u32::MAX),
            block: model::selector::first_anchor_in_span(doc, &c.span),
        })
        .collect();
    let display = match sec {
        SecRef::Hpath { hpath } => {
            // Machine-addressable `n=` forms, one per duplicate: the occurrence
            // index on the final segment resolves each uniquely (§2.1).
            e.candidates = Some(
                named
                    .iter()
                    .map(|c| {
                        let mut segs = hpath.clone();
                        if let Some(last) = segs.last_mut() {
                            last.n = Some(c.node_index);
                        }
                        SecRef::Hpath { hpath: segs }
                    })
                    .collect(),
            );
            hpath
                .iter()
                .map(|s| s.h.as_str())
                .collect::<Vec<_>>()
                .join("/")
        }
        SecRef::Anchor { anchor } => {
            // Duplicate block ids share one id — it cannot disambiguate them, so
            // `candidates` stays EMPTY (no exact §2.1 spelling per target); the
            // message names them by node index (d1: "by ... node index").
            e.candidates = Some(Vec::new());
            format!("^{anchor}")
        }
        SecRef::FmKey { fm_key } => {
            e.candidates = Some(Vec::new());
            fm_key.clone()
        }
    };
    e.message = Some(model::selector::render_ambiguity(&display, &named));
    e
}
