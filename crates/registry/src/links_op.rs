//! The `links` op (`docs/wire-contract.md` §4.6): the outgoing wikilink/embed
//! edge map over the wire.
//!
//! The serve is the shared one ([`wire_serve::read::links`], which the
//! in-process lane of `mrd links` also calls); this module hands it the mount
//! table with this daemon's resident engine for each mounted root the
//! answer's links name
//! ([`Registry::mounted_corpus`](crate::Registry::mounted_corpus)), as `walk`
//! (`walk_op`) and `sql` (`sql_op`) take theirs. The table is read from this
//! daemon's environment on every call; the needed roots narrow the corpora,
//! never the table.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::Path;

use wire::{ErrorBody, ErrorCode, ResponseBody, Root};
use wire_serve::mount_corpus::MountCorpora;

use crate::registry::ColdGate;
use crate::{Registry, WorkspaceEngine};

/// Serve one `links` call from the pinned warm engine `engine` — an
/// [`engine_snapshot`](Registry::engine_snapshot) clone, no registry lock
/// held, so mounted roots can be warmed through `registry` here.
///
/// # Errors
/// `stale_view` when `require_root` does not match `engine`'s root;
/// `file_not_found` / `invalid_utf8` / `io_error` for a missing, unserved, or
/// unreadable named path or retained member; `corpus_warming` (retry) while a
/// mounted root the answer names has no resident engine ([`mounted_roots`]).
pub(crate) fn serve(
    registry: &Registry,
    engine: &WorkspaceEngine,
    ws: &Path,
    path: Option<&wire::Path>,
    require_root: Option<&Root>,
) -> Result<ResponseBody, Box<ErrorBody>> {
    let as_of = Root(engine.at_fingerprint.0.clone());
    wire_serve::read::require_root_check(require_root, &as_of)?;
    let live = as_of.clone();
    wire_serve::read::links(
        &fs::WorkspaceRoot(ws.to_path_buf()),
        &engine.index,
        engine,
        &engine.unserved,
        path,
        |needed| mounted_roots(registry, needed),
        as_of,
        0,
        || Ok(live),
    )
}

/// The mount table with this daemon's resident engine for each root in
/// `needed`. A root with no resident engine goes through the §3.2 cold gate
/// ([`Registry::cold_gate`]) exactly as a served workspace does: its rebuild
/// runs on a background thread, a small one lands inside the gate's bounded
/// wait and serves, and a longer one refuses `corpus_warming` so the caller
/// re-asks instead of holding a request open for the length of a corpus build.
/// A rebuild that failed carries the root as unreachable with its cause, as a
/// failed inline warm does.
fn mounted_roots(
    registry: &Registry,
    needed: &BTreeSet<addr::MountName>,
) -> Result<MountCorpora, Box<ErrorBody>> {
    let warming = RefCell::new(Vec::new());
    let resident = |root: &Path| match registry.cold_gate(root) {
        Ok(ColdGate::Serve) => registry.mounted_corpus(root),
        Ok(ColdGate::Warming) => {
            warming.borrow_mut().push(root.display().to_string());
            Err("the mounted corpus is still warming".to_owned())
        }
        Ok(ColdGate::Failed(cause)) => Err(format!("cannot read the mounted corpus: {cause}")),
        Err(e) => Err(format!("cannot read the mounted corpus: {e}")),
    };
    let mounts = wire_serve::mount_corpus::load_mounts_for_from(needed, &resident);
    let warming = warming.into_inner();
    if warming.is_empty() {
        return Ok(mounts);
    }
    let mut e = ErrorBody::new(ErrorCode::CorpusWarming);
    e.message = Some(format!(
        "a mounted root this page's links name is still warming ({}): it has no resident \
         corpus engine in this daemon yet, and its whole-corpus rebuild is running in the \
         background — transient; the same request serves once the rebuild lands.",
        warming.join(", ")
    ));
    Err(Box::new(e))
}
