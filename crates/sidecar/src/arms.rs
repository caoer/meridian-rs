//! Op arms: one fn per armed op, `model`/`fs` in, wire body out. Wiring only —
//! parse/tree/project/disk live elsewhere. Shared strict-decode, read arms,
//! and `splice → commit` live in `wire-serve`; this is the sidecar's
//! per-request driver.

use std::io::ErrorKind;

/// Sidecar `seq` allocator: live ring at the moment the write path asks.
/// Single producer (no race); sink is the shared host seam. Immutable borrow
/// ends before the caller advances the ring.
struct EpochSink<'a>(&'a wire_serve::ring::RootRing);

impl wire_serve::seq::SeqSink for EpochSink<'_> {
    fn allocate(&self, _before: &Root, _after: &Root, _files: &[wire::DeltaFile]) -> u64 {
        self.0.allocate_seq()
    }
}

use wire::{ErrorBody, ErrorCode, Op, Path, ResponseBody, Root, SecRef};
use wire_serve::{ambient_root, domain_snapshot, load_doc};

/// Route one validated request. `id` is the frame correlation token (splice
/// records it in the receipt, §6.1; no other arm reads it). `v3` is the
/// session's negotiated rev: extract enriches headings with U2 facts under
/// v3 only (frozen v2 bytes never change).
#[expect(
    clippy::too_many_lines,
    reason = "exhaustive op router — one arm per wire op; splitting arms adds indirection, not insight"
)]
pub(crate) fn dispatch(
    root: &fs::WorkspaceRoot,
    epoch: &mut crate::ring::RootRing,
    id: Option<u64>,
    op: Op,
    rulesets: &[policy::CompiledRuleset],
    v3: bool,
) -> Result<ResponseBody, Box<ErrorBody>> {
    match op {
        Op::Hello { .. } => Ok(hello(root, v3)),
        Op::Toc { path } => toc(root, &path),
        Op::Cat { path, sec } => cat(root, &path, sec),
        Op::Extract { path, kinds } => extract(root, &path, kinds, v3),
        // Composed read — v3-only (absent from frozen v2 caps; §3.2).
        Op::Read {
            path,
            frag,
            sections,
            display_path,
            actor,
        } => {
            if !v3 {
                return Err(Box::new(ErrorBody::new(ErrorCode::UnknownOp)));
            }
            let doc = load_doc(root, &path)?;
            let ambient = ambient_root(root)?;
            wire_serve::read::composed_read(
                &doc,
                &path,
                &ambient,
                &wire_serve::read::ReadParams {
                    frag,
                    sections,
                    display_path,
                    actor,
                },
                // S6: per-request, no session — mints no read receipt.
                None,
                // S10: no corpus — no pin decoration (guess would lie).
                &wire_serve::read::NO_DECORATIONS,
            )
        }
        // I4 def-conformance — v3-only (§3.2). Read-only.
        Op::CheckWrite {
            path,
            target,
            actor,
            now,
            edits,
        } => {
            if !v3 {
                return Err(Box::new(ErrorBody::new(ErrorCode::UnknownOp)));
            }
            let doc = load_doc(root, &path)?;
            Ok(wire_serve::check_write::check_write(
                &doc, &target, &actor, &now, &edits,
            ))
        }
        Op::Resolve {
            from,
            r#ref,
            content,
        } => resolve(root, &from, &r#ref, content.unwrap_or(false)),
        Op::Root => root_op(root, epoch),
        Op::Diff { from_root, to_root } => diff_op(root, epoch, &from_root, &to_root),
        Op::Links { path, require_root } => {
            links_op(root, epoch, path.as_ref(), require_root.as_ref())
        }
        // Registered at serve layer — unreachable here; `internal` not panic.
        Op::Sub { .. } => Err(Box::new(ErrorBody::new(ErrorCode::Internal))),
        // §Q2 view_path is daemon-only (OD6 persistent builder; C2 forbids
        // sidecar→view). Refuse `daemon_only`; never advertise in caps.
        Op::ViewPath { .. } => Err(Box::new(ErrorBody::new(ErrorCode::DaemonOnly))),
        // v2 §4.4 sole write op (v3 adds `create`): shared `splice → commit`
        // (`wire_serve::write::splice`). Advances per-epoch ring; body is the
        // choke-point's (byte-identical across hosts). Sidecar admits packs;
        // daemon hands `&[]`.
        Op::Splice {
            path,
            actor,
            now,
            receipt,
            if_root,
            dry,
            force,
            edits,
            plan_edits,
            pin,
        } => {
            let out = wire_serve::write::splice(
                root,
                Some(&EpochSink(epoch)),
                &wire_serve::write::SpliceArgs {
                    id,
                    // U10: decoded wire frames → wire door; enforces like the
                    // daemon. Law binds the door, not the client.
                    origin: wire_serve::guard::Origin::Wire,
                    path,
                    actor,
                    now,
                    receipt,
                    if_root,
                    dry: dry.unwrap_or(false),
                    force: force.unwrap_or(false),
                    edits,
                    plan_edits,
                    pin,
                },
                rulesets,
                // S7: no session → no read-receipt ledger; session-actor pin
                // refuses `read_mint_required`.
                None,
            )?;
            if let Some(frame) = out.committed {
                epoch.advance(frame);
            }
            Ok(out.body)
        }
        // Birth — v3-only (§3.2). Forwards to `wire_serve::write::create`
        // (same guarded door); opens no file itself. Ring advances on birth
        // Delta like `splice`.
        Op::Create {
            path,
            body,
            actor,
            now,
            if_root,
            dry,
        } => {
            if !v3 {
                return Err(Box::new(ErrorBody::new(ErrorCode::UnknownOp)));
            }
            let sink = EpochSink(epoch);
            let out = wire_serve::write::create(
                root,
                Some(&sink),
                &wire_serve::write::CreateArgs {
                    id,
                    path: path.clone(),
                    body,
                    actor,
                    now,
                    if_root,
                    dry: dry.unwrap_or(false),
                },
                rulesets,
            )?;
            let response = wire_serve::write::create_response(path, &out);
            if let Some(frame) = out.committed {
                epoch.advance(frame);
            }
            Ok(response)
        }
    }
}

/// v2 §4.6 edge map under the §10.1 triple, from `query` over the domain
/// snapshot (§10.3 / §12 hash domain). `as_of_root` folds the exact bytes
/// parsed; `live_root` is a fresh post-compute fold — concurrent splice may
/// diverge (legal; §10.1 promises no lag bounds).
fn links_op(
    root: &fs::WorkspaceRoot,
    epoch: &crate::ring::RootRing,
    path: Option<&Path>,
    require_root: Option<&Root>,
) -> Result<ResponseBody, Box<ErrorBody>> {
    let (files, as_of_root) = domain_snapshot(root)?;
    // Delta counter at as_of_root (§10.1), sampled with the snapshot.
    let changes_seq = epoch.seq();
    // §10.2 opt-in strictness — before parse, avoid stale `build_corpus`.
    wire_serve::read::require_root_check(require_root, &as_of_root)?;
    // Fact plane refuses unparseable files loud (unlike resolve's skip,
    // §4.5). Shared `fs::build_corpus`; invalid_data → wire `invalid_utf8`.
    let (index, docs) = fs::build_corpus(files).map_err(|e| {
        Box::new(if e.kind() == ErrorKind::InvalidData {
            ErrorBody::new(ErrorCode::InvalidUtf8)
        } else {
            let mut err = ErrorBody::new(ErrorCode::IoError);
            err.cause = Some(e.to_string());
            err
        })
    })?;
    // Shared read arm; `live_root` sampled only on success.
    wire_serve::read::links(&index, &docs, path, as_of_root, changes_seq, || {
        Ok(domain_snapshot(root)?.1)
    })
}

/// v2 §4.7: workspace root cursor (fresh from disk) + epoch `seq`.
fn root_op(
    root: &fs::WorkspaceRoot,
    epoch: &crate::ring::RootRing,
) -> Result<ResponseBody, Box<ErrorBody>> {
    Ok(ResponseBody::Root {
        root: ambient_root(root)?,
        seq: epoch.seq(),
    })
}

/// v2 §4.7/§7.3 replay over retained history. Same-root → empty; else
/// `root_unknown` → resync (re-derive, never wrong data).
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

/// v2 §3.2: proto, server name, armed caps. `root` optional — present when
/// the walk computes, absent on I/O failure.
///
/// R1 identity: under v3 the sidecar publishes the field with the honest value
/// `unknown`. It bakes no build sha — it has no build script, and it is on
/// death row — so `unknown` is the true answer for it rather than a
/// placeholder. It publishes the field anyway so one v3 client speaks to both
/// hosts through one code path. Under v2 the field is `None` and the frozen
/// hello body is byte-identical.
fn hello(root: &fs::WorkspaceRoot, v3: bool) -> ResponseBody {
    ResponseBody::Hello {
        proto: crate::PROTO,
        server: crate::SERVER_NAME.to_string(),
        caps: crate::CAPS.iter().map(ToString::to_string).collect(),
        root: ambient_root(root).ok(),
        // One workspace per process; drawer is client-side (daemon pins §4).
        storage: None,
        // Root is the launch arg, not a hello field — no bound root to echo.
        workspace: None,
        identity: v3.then(|| wire::Identity {
            build: wire_serve::UNKNOWN_BUILD.to_string(),
        }),
    }
}

/// v2 §4.1: map — `file_rev` + ambient `root` + `wire-map` rows.
fn toc(root: &fs::WorkspaceRoot, path: &Path) -> Result<ResponseBody, Box<ErrorBody>> {
    let doc = load_doc(root, path)?;
    Ok(wire_serve::read::toc(&doc, path, &ambient_root(root)?))
}

/// v2 §4.2: full span bytes (heading-inclusive), rev over those bytes.
/// `sec` absent → whole file; `file_rev` in the `node_rev` slot.
fn cat(
    root: &fs::WorkspaceRoot,
    path: &Path,
    sec: Option<SecRef>,
) -> Result<ResponseBody, Box<ErrorBody>> {
    let doc = load_doc(root, path)?;
    wire_serve::read::cat(&doc, sec)
}

/// v2 §4.3: node inventory via `wire-map`; `kinds` filtered at decode.
/// v3 headings carry U2 addressing facts.
fn extract(
    root: &fs::WorkspaceRoot,
    path: &Path,
    kinds: Option<Vec<String>>,
    enrich: bool,
) -> Result<ResponseBody, Box<ErrorBody>> {
    let doc = load_doc(root, path)?;
    Ok(wire_serve::read::extract(&doc, path, kinds, enrich))
}

/// v2 §4.5 walk plane: best-effort two-stage walk. Location facts only;
/// `content:true` adds fragment bytes, still no rev. Unreadable/non-UTF-8
/// files skipped (one broken file must not kill the rest).
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
    wire_serve::read::resolve(&index, &docs, from, link, want_content)
}
