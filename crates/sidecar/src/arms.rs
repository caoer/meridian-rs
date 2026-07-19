//! The op arms: one function per armed op, `model`/`fs` in, wire body out.
//! Wiring only — parsing is `syntax`'s, tree law is `model`'s, projection is
//! `wire-map`'s, disk is `fs`'s.

use std::io::ErrorKind;

use wire::{ErrorBody, ErrorCode, NodeRev, Op, Path, ResponseBody, Root, SecRef, Span};

/// Route one validated request to its arm. `Splice`/`Root`/`Diff` are known to
/// the wire vocabulary but unarmed at this rung — the strict decode already
/// answers `unknown_op` before construction; the arms here are unreachable
/// mirrors kept for match exhaustiveness.
pub(crate) fn dispatch(root: &fs::WorkspaceRoot, op: Op) -> Result<ResponseBody, Box<ErrorBody>> {
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
        Op::Splice { .. } | Op::Root | Op::Diff { .. } => {
            Err(Box::new(ErrorBody::new(ErrorCode::UnknownOp)))
        }
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
fn ambient_root(root: &fs::WorkspaceRoot) -> Result<Root, Box<ErrorBody>> {
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
    Ok(Root(model::merkle_root(&entries, domain.version()).0))
}

/// `fs::load` with the §8 error split: `file_not_found` (env — the file is
/// gone, path echoed), `invalid_utf8` (refused, never lossy-decoded),
/// `io_error{cause}` otherwise.
fn load_doc(root: &fs::WorkspaceRoot, path: &Path) -> Result<model::Document, Box<ErrorBody>> {
    fs::load(root, std::path::Path::new(&path.0)).map_err(|e| Box::new(match e.kind() {
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
    }))
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
fn cat(root: &fs::WorkspaceRoot, path: &Path, sec: Option<SecRef>) -> Result<ResponseBody, Box<ErrorBody>> {
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
        SecRef::Anchor { anchor } => model::Ref::anchor(anchor.clone())
            .map_err(|bad| crate::bad_request(format!(
                "block id outside the one charset [A-Za-z0-9-] (§2.4): `{id}`",
                id = bad.id
            )))?,
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
