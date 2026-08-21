//! The `walk` op (`docs/wire-contract.md` § A.10): pin-graph context assembly
//! over the wire — up = what the page draws from, down = who pins the page
//! (blast radius), every answer citing the doc revs it read.
//!
//! The walk COMPUTER is the one the CLI verbs color through
//! ([`view::walk::walk_rooted`]), and the mount-table corpus assembly is the
//! shared owner ([`wire_serve::mount_corpus`]) — this module only admits the
//! named page, hands the computer the warm engine's served projection, and
//! projects the report onto the wire shape. Read-only: writes nothing, mints
//! no receipt; the walk is computed per query, never stored.

use std::collections::BTreeMap;
use std::path::Path;

use view::walk::{self, Direction, WalkError};
use wire::{ErrorBody, ErrorCode, ResponseBody, WalkCite, WalkRow};

use crate::WorkspaceEngine;
use crate::server::file_not_found;

/// Serve one `walk` call from the warm engine's borrowed state.
///
/// # Errors
/// `file_not_found` when the named root has no file; `invalid_utf8` when it
/// is an unserved domain member; `walk_cycle` (env) on an in-snapshot cycle;
/// `io_error` when the workspace's hash domain cannot be read — degrading to
/// the default domain would claim every path is hashed, the false-red
/// fail-open decision 0034 ruled out.
pub(crate) fn serve(
    engine: &WorkspaceEngine,
    ws: &Path,
    path: &wire::Path,
    down: bool,
    depth: Option<u32>,
) -> Result<ResponseBody, Box<ErrorBody>> {
    let root = fs::WorkspaceRoot(ws.to_path_buf());
    let page = path.0.as_str();

    // §12.1 door-family clause: a door the caller names ONE path at serves it
    // — the hash domain gates enumerations, never what a named path is
    // entitled to. The warm engine holds the served projection; a named page
    // outside it is admitted by the same single-file load the read doors run.
    let mut owned: Option<model::Docs> = None;
    if !engine.docs.contains_key(page) {
        if let Some(condition) = engine.unserved.get(page) {
            let mut e = ErrorBody::new(ErrorCode::InvalidUtf8);
            e.path = Some(path.clone());
            e.message = Some(format!(
                "invalid_utf8: {page} is in the hash domain but serves no spans ({condition})"
            ));
            return Err(Box::new(e));
        }
        let Ok(doc) = fs::load(&root, Path::new(page)) else {
            return Err(file_not_found(path));
        };
        let mut admitted = engine.docs.clone();
        admitted.insert(page.to_owned(), std::sync::Arc::new(doc));
        owned = Some(admitted);
    }
    let docs = owned.as_ref().unwrap_or(&engine.docs);

    // An unreadable domain config FAILS the door (decision 0034; see Errors).
    let domain = fs::domain::Domain::load(&root).map_err(|e| {
        let mut err = ErrorBody::new(ErrorCode::IoError);
        err.cause = Some(format!("cannot read the hash domain: {e}"));
        Box::new(err)
    })?;

    // The mount table, with a corpus for exactly the roots this corpus's own
    // lock addresses name — the shared assembly, so this door and the CLI pin
    // planes hand the colour computer the same corpus.
    let mounts = wire_serve::mount_corpus::load_mounts_for(&walk::lock_addressed_roots(docs));
    let corpus = mounts.rooted(docs, &domain, &root);

    let direction = if down { Direction::Down } else { Direction::Up };
    let report =
        walk::walk_rooted(&corpus, mounts.set(), page, direction, depth).map_err(|error| {
            match error {
                WalkError::RootNotFound(_) => file_not_found(path),
                WalkError::Cycle(loop_pages) => {
                    let mut e = ErrorBody::new(ErrorCode::WalkCycle);
                    e.message = Some(format!("in-snapshot cycle: {}", loop_pages.join(" -> ")));
                    Box::new(e)
                }
            }
        })?;

    // §12.1 enumerator clause, down only: the blast-radius census names the
    // markdown the hash domain left out. Up drops nothing — it names an
    // excluded ancestor by its correct path at a red edge — so it owes none.
    let excluded = if down {
        excluded_population(&root, docs, &engine.unserved)
    } else {
        Vec::new()
    };

    let entries = report
        .entries
        .iter()
        .map(|entry| WalkRow {
            depth: entry.depth,
            selector: entry.selector.clone(),
            rev: entry.rev.clone(),
            color: walk::color_tone(&entry.color).to_string(),
            reason: walk::color_reason(&entry.color).map(str::to_owned),
            detail: walk::color_detail(&entry.color),
            teaching: walk::color_teaching(&entry.color, &entry.selector),
        })
        .collect();
    let revs_read = report
        .revs_read
        .iter()
        .map(|cite| WalkCite {
            path: cite.path.clone(),
            doc_rev: cite.doc_rev.clone(),
        })
        .collect();
    Ok(ResponseBody::Walk {
        direction: report.direction.label().to_string(),
        page: report.root,
        depth_bound: report.depth_bound,
        entries,
        revs_read,
        excluded,
    })
}

/// The markdown under the root that the served projection does not hold —
/// on disk, outside the hash domain, in path order. The admitted named page
/// rides `docs`, so a door's own subject never reads as an exclusion.
fn excluded_population(
    root: &fs::WorkspaceRoot,
    docs: &model::Docs,
    unserved: &BTreeMap<String, String>,
) -> Vec<String> {
    let Ok(all) = fs::walk(root) else {
        return Vec::new();
    };
    let mut excluded: Vec<String> = all
        .iter()
        .filter_map(|rel| rel.to_str().map(str::to_owned))
        .filter(|rel| !docs.contains_key(rel) && !unserved.contains_key(rel))
        .collect();
    excluded.sort();
    excluded
}
