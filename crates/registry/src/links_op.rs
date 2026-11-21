//! The `links` op (`docs/wire-contract.md` §4.6, `docs/laws.md` § C-1):
//! outgoing wikilink/embed edges over the wire, resolving declared cross-vault
//! roots through the daemon's resident engines (`Registry::mounted_corpus`)
//! exactly as `walk` (`walk_op`) and `sql` (`sql_op`) do.
//!
//! Table-external heads (`https://…`, undeclared roots, non-canonical mount
//! names) stay ambient `unresolved` verbatim at exit 0. When the queried page
//! (or corpus) names no canonical mount root at all, the mount table is not
//! consulted and zero config I/O is paid.

use std::collections::BTreeSet;
use std::path::Path;

use model::catalog::DocumentCatalog;
use wire::{ErrorBody, ResponseBody, Root};

use crate::{Registry, WorkspaceEngine};

/// Serve one `links` call from the pinned warm engine `engine` — no registry
/// lock held, so mounted roots can be warmed through `registry` here.
///
/// # Errors
/// `root_mismatch` when `require_root` does not match `engine`'s root;
/// `file_not_found` / `invalid_utf8` / `io_error` for a missing, unserved, or
/// unreadable named path or retained member.
pub(crate) fn serve(
    registry: &Registry,
    engine: &WorkspaceEngine,
    ws: &Path,
    path: Option<&wire::Path>,
    require_root: Option<&Root>,
) -> Result<ResponseBody, Box<ErrorBody>> {
    let as_of = Root(engine.at_fingerprint.0.clone());
    wire_serve::read::require_root_check(require_root, &as_of)?;
    let root = fs::WorkspaceRoot(ws.to_path_buf());

    let needed = needed_roots(&root, engine, path)?;
    let live = as_of.clone();
    if needed.is_empty() {
        return wire_serve::read::links(
            &root,
            &engine.index,
            engine,
            &engine.unserved,
            path,
            as_of,
            0,
            || Ok(live),
        );
    }

    let mounts = wire_serve::mount_corpus::load_mounts_for_from(&needed, &|mount_root| {
        registry.mounted_corpus(mount_root)
    });
    let domain = fs::domain::Domain::load(&root).unwrap_or_default();
    let corpus = mounts
        .rooted_catalog(engine)
        .with_hash_domain(&domain)
        .with_ambient_disk(&root);
    wire_serve::read::links_in_rooted(
        &root,
        &engine.index,
        engine,
        &engine.unserved,
        &corpus,
        mounts.set(),
        path,
        as_of,
        0,
        || Ok(live),
    )
}

/// Mount names referenced by the queried document(s). A named member pins only
/// itself; a named nonmember loads once from disk; a whole-corpus query reads
/// descriptor metadata (`DocumentCatalog::link_roots`) without hydrating any
/// body.
fn needed_roots(
    root: &fs::WorkspaceRoot,
    engine: &WorkspaceEngine,
    path: Option<&wire::Path>,
) -> Result<BTreeSet<addr::MountName>, Box<ErrorBody>> {
    let Some(p) = path else {
        return Ok(engine.link_roots());
    };
    if engine.revision(&p.0).is_some() {
        return model::catalog::scan(engine, &p.0, model::catalog::link_roots)
            .map(Option::unwrap_or_default)
            .map_err(wire_serve::read::member_fault);
    }
    if engine.unserved.contains_key(&p.0) {
        return Ok(BTreeSet::new());
    }
    // Named nonmember: if readable, inspect its link roots; otherwise let
    // `links_nonmember` mint the authoritative refusal.
    match fs::load(root, Path::new(&p.0)) {
        Ok(doc) => Ok(model::catalog::link_roots(&doc)),
        Err(_) => Ok(BTreeSet::new()),
    }
}
