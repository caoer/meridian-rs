//! The op arms: one function per armed op, `model`/`fs` in, wire body out.
//! Wiring only — parsing is `syntax`'s, tree law is `model`'s, projection is
//! `wire-map`'s, disk is `fs`'s. The shared strict-decode, the read arms, and
//! the `splice → commit` write choke-point live in `wire-serve` (ONE
//! implementation with the resident daemon); this module is the sidecar's
//! per-request driver over them.

use std::io::ErrorKind;

use wire::{ErrorBody, ErrorCode, Op, Path, ResponseBody, Root, SecRef};
use wire_serve::{ambient_root, domain_snapshot, load_doc};

/// Route one validated request to its arm. `id` is the frame's correlation
/// token — the splice choke-point records it into the receipt line (§6.1 fact
/// list); no other arm reads it. `v3` is the session's negotiated rev: the
/// extract arm enriches heading nodes with the U2 addressing facts under v3
/// only (v3-additive keys; the frozen v2 bytes never change).
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
        Op::Hello { .. } => Ok(hello(root)),
        Op::Toc { path } => toc(root, &path),
        Op::Cat { path, sec } => cat(root, &path, sec),
        Op::Extract { path, kinds } => extract(root, &path, kinds, v3),
        // M1 U4a2 the composed read op — v3-ONLY (absent from the frozen v2
        // caps; §3.2: an op is in `caps` or answers `unknown_op`).
        Op::Read {
            path,
            mode,
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
                    mode,
                    frag,
                    sections,
                    display_path,
                    actor,
                },
                // S6: the sidecar builds its state per REQUEST and holds no
                // session, so there is no daemon-session layer for a read
                // receipt to live in — it mints none (the registry daemon is
                // the one host that does).
                None,
                // S10: and it holds no corpus either — one document per
                // request cannot color a pin whose target is another page, so
                // it decorates nothing rather than guessing (an undecorated
                // link claims nothing; a wrongly-colored one lies).
                &wire_serve::read::NO_DECORATIONS,
            )
        }
        // M1 U8c the I4 def-conformance verdict — v3-ONLY (absent from the
        // frozen v2 caps; §3.2 discovery honesty). Read-only: loads the prev
        // bytes, never writes.
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
        // Armed, but registered at the serve layer (the loop owns the
        // subscription list) — unreachable through serve; answering internal
        // (never a panic) keeps a future misroute non-fatal.
        Op::Sub { .. } => Err(Box::new(ErrorBody::new(ErrorCode::Internal))),
        // V2 §Q2 the view-organ path forwarder is DAEMON-ONLY: the daemon is
        // the sole persistent builder (OD6), and this per-process sidecar
        // cannot publish — C2 forbids `sidecar`→`view`, and an in-memory
        // `:memory:` build has no filesystem path to forward. So `view_path`
        // is refused LOUD with `daemon_only` (the wire code's own sidecar-mode
        // carve-out), never advertised in the sidecar's caps.
        Op::ViewPath { .. } => Err(Box::new(ErrorBody::new(ErrorCode::DaemonOnly))),
        // v2 §4.4 the only write op under v2 (D4-SPLICE; v3 adds the `create`
        // birth arm below), driven through the ONE shared
        // `splice → commit` choke-point (`wire_serve::write::splice`, W1): load
        // → §5.1 guard → validate → verdicts → commit, one exchange, one Delta.
        // The sidecar advances its per-epoch ring with the emitted frame; the
        // response body is the choke-point's, byte-identical across hosts. The
        // sidecar admits packs (`rulesets`); the resident daemon hands `&[]`.
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
                epoch.seq(),
                &wire_serve::write::SpliceArgs {
                    id,
                    // U10: the per-process sidecar serves DECODED WIRE FRAMES,
                    // so it is a wire door exactly like the resident daemon and
                    // enforces identically. This door is NOT MCP — which is the
                    // point: the ruling covers every wire door, not a client.
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
                // S7: the per-request sidecar HOLDS NO SESSION, so it holds no
                // read-receipt ledger. A pin from a session actor therefore
                // refuses `read_mint_required` here, naming that reason — the
                // honest answer, since this host cannot know what was read.
                None,
            )?;
            if let Some(frame) = out.committed {
                epoch.advance(frame);
            }
            Ok(out.body)
        }
        // The BIRTH op — v3-ONLY (absent from the frozen v2 caps; §3.2
        // discovery honesty), driven through the SAME guarded door every
        // in-process caller uses (`wire_serve::write::create`). This arm
        // FORWARDS: it opens no file, computes no rev, writes no journal row.
        // That is the whole point — a daemon-side birth would bypass the four
        // birth guards and leave the newborn with no journal row to date it.
        // The ring advances on the emitted birth Delta exactly like `splice`.
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
            let out = wire_serve::write::create(
                root,
                epoch.seq(),
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
    require_root: Option<&Root>,
) -> Result<ResponseBody, Box<ErrorBody>> {
    let (files, as_of_root) = domain_snapshot(root)?;
    // The Delta counter at as_of_root (§10.1) — sampled with the snapshot;
    // §7.1 per-daemon-epoch semantics ride along unchanged.
    let changes_seq = epoch.seq();
    // §10.2: the opt-in strictness refusal — checked BEFORE the parse, so a
    // stale view refuses without paying `build_corpus`.
    wire_serve::read::require_root_check(require_root, &as_of_root)?;
    // The fact plane refuses what it cannot parse — loud, never skipped (the
    // walk plane's skip-broken-files posture is resolve's, §4.5). Parse + build
    // + index is the shared [`fs::build_corpus`] primitive (the daemon's
    // resident builder parses the identical bytes — one implementation, no
    // drift); `invalid_data` is the shared UTF-8 refusal, re-homed to the wire
    // `invalid_utf8` frame.
    let (index, docs) = fs::build_corpus(files).map_err(|e| {
        Box::new(if e.kind() == ErrorKind::InvalidData {
            ErrorBody::new(ErrorCode::InvalidUtf8)
        } else {
            let mut err = ErrorBody::new(ErrorCode::IoError);
            err.cause = Some(e.to_string());
            err
        })
    })?;
    // The edge map + the §10.1 `live_root` (a fresh fold AFTER the computation)
    // is the shared read arm; `live_root` is sampled only on the success path.
    wire_serve::read::links(&index, &docs, path, as_of_root, changes_seq, || {
        Ok(domain_snapshot(root)?.1)
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
        // The sidecar runs one workspace per process and opens its drawer
        // client-side; the resident daemon is the one that pins storage (§4).
        storage: None,
        // Nothing was declared to this process over the wire — its root is its
        // launch argument, not a `hello` field — so there is no bound root to
        // report back. The daemon is the arm that answers this.
        workspace: None,
    }
}

/// v2 §4.1: the map — header `file_rev` (the document root's rev, same family,
/// whole-file bytes) + ambient `root`, rows from the `wire-map` projection.
fn toc(root: &fs::WorkspaceRoot, path: &Path) -> Result<ResponseBody, Box<ErrorBody>> {
    let doc = load_doc(root, path)?;
    Ok(wire_serve::read::toc(&doc, path, &ambient_root(root)?))
}

/// v2 §4.2: full span bytes (heading-inclusive), rev over precisely those
/// bytes. `sec` absent → whole file + `file_rev` riding the `node_rev` slot.
fn cat(
    root: &fs::WorkspaceRoot,
    path: &Path,
    sec: Option<SecRef>,
) -> Result<ResponseBody, Box<ErrorBody>> {
    let doc = load_doc(root, path)?;
    wire_serve::read::cat(&doc, sec)
}

/// v2 §4.3: the full node inventory via the `wire-map` projection, `kinds`
/// filtered (values already validated against the closed enum at decode).
/// Under v3 the heading nodes carry the U2 addressing facts.
fn extract(
    root: &fs::WorkspaceRoot,
    path: &Path,
    kinds: Option<Vec<String>>,
    enrich: bool,
) -> Result<ResponseBody, Box<ErrorBody>> {
    let doc = load_doc(root, path)?;
    Ok(wire_serve::read::extract(&doc, path, kinds, enrich))
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
    wire_serve::read::resolve(&index, &docs, from, link, want_content)
}
