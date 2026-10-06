//! The `walk` op (`docs/wire-contract.md` § A.10): pin-graph context assembly
//! over the wire — up = what the page draws from, down = who pins the page
//! (blast radius), every answer citing the doc revs it read.
//!
//! The walk COMPUTER is the one the CLI verbs color through
//! ([`view::walk::walk_leased`], the leased arm of the one walk), and the
//! mount-table corpus assembly is the shared owner
//! ([`wire_serve::mount_corpus`]) — this module only admits the named page,
//! hands the computer the warm engine's generation as a catalog with the
//! mounted roots' resident engines beside it
//! ([`Registry::mounted_corpus`](crate::Registry::mounted_corpus)), and
//! projects the report onto the wire shape. The generation is read through
//! its descriptors and one leased body at a time (merkle-spec §6.8).
//! Read-only: writes nothing, mints no receipt; the walk is computed per
//! query, never stored.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::Path;

use model::catalog::{DocumentCatalog, DocumentLease};
use view::walk::{self, Direction, LockFacts, WalkError, WalkStop};
use wire::{ErrorBody, ErrorCode, ResponseBody, WalkCite, WalkRow};

use crate::server::file_not_found;
use crate::{Registry, WorkspaceEngine};

/// Serve one `walk` call from the pinned warm engine `engine` — an
/// [`engine_snapshot`](Registry::engine_snapshot) clone, no registry lock
/// held, so the mounted roots can be warmed through `registry` here.
///
/// # Errors
/// `file_not_found` when the named root has no file; `invalid_utf8` when it
/// is an unserved domain member; `walk_cycle` (env) on an in-snapshot cycle;
/// `io_error` when the workspace's hash domain cannot be read — degrading to
/// the default domain would claim every path is hashed, the false-red
/// fail-open decision 0034 ruled out — or when a member the generation holds
/// cannot be served, naming that member.
pub(crate) fn serve(
    registry: &Registry,
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
    let admitted;
    let catalog: &dyn DocumentCatalog;
    let locks: &dyn LockFacts;
    if engine.revision(page).is_some() {
        catalog = engine;
        locks = engine;
    } else {
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
        let mut index = (*engine.index).clone();
        index.insert_ordered(page, &doc);
        admitted = Admitted {
            engine,
            path: page,
            doc,
            index,
        };
        catalog = &admitted;
        locks = &admitted;
    }

    // An unreadable domain config FAILS the door (decision 0034; see Errors).
    let domain = fs::domain::Domain::load(&root).map_err(|e| {
        let mut err = ErrorBody::new(ErrorCode::IoError);
        err.cause = Some(format!("cannot read the hash domain: {e}"));
        Box::new(err)
    })?;

    // The mount table, with a corpus for exactly the roots this corpus's own
    // lock addresses name — the shared assembly, so this door and the CLI pin
    // planes hand the colour computer the same corpus. Each corpus is this
    // daemon's resident engine for that root: warmed once, refreshed O(dirty),
    // never rebuilt from disk per call.
    let mounts = wire_serve::mount_corpus::load_mounts_for_from(&locks.lock_roots(), &|root| {
        registry.mounted_corpus(root)
    });
    let corpus = mounts
        .rooted_catalog(catalog)
        .with_hash_domain(&domain)
        .with_ambient_disk(&root);

    let direction = if down { Direction::Down } else { Direction::Up };
    let report = walk::walk_leased(&corpus, locks, mounts.set(), page, direction, depth).map_err(
        |stop| match stop {
            WalkStop::Walk(WalkError::RootNotFound(_)) => file_not_found(path),
            WalkStop::Walk(WalkError::Cycle(loop_pages)) => {
                let mut e = ErrorBody::new(ErrorCode::WalkCycle);
                e.message = Some(format!("in-snapshot cycle: {}", loop_pages.join(" -> ")));
                Box::new(e)
            }
            WalkStop::Fault(fault) => wire_serve::read::member_fault(fault),
        },
    )?;

    // §12.1 enumerator clause, down only: the blast-radius census names the
    // markdown the hash domain left out. Up drops nothing — it names an
    // excluded ancestor by its correct path at a red edge — so it owes none.
    let excluded = if down {
        excluded_population(&root, catalog, &engine.unserved)
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
/// is a member of `catalog`, so a door's own subject never reads as an
/// exclusion.
fn excluded_population(
    root: &fs::WorkspaceRoot,
    catalog: &dyn DocumentCatalog,
    unserved: &BTreeMap<String, String>,
) -> Vec<String> {
    let Ok(all) = fs::walk(root) else {
        return Vec::new();
    };
    let mut excluded: Vec<String> = all
        .iter()
        .filter_map(|rel| rel.to_str().map(str::to_owned))
        .filter(|rel| catalog.revision(rel).is_none() && !unserved.contains_key(rel))
        .collect();
    excluded.sort();
    excluded
}

/// The engine's generation with the one named page the hash domain left out
/// admitted beside it: a member like any other to the walk, its document held
/// here for the call. The name index is the generation's with that page
/// entered, so a pin resolves in the world the walk was asked about.
#[derive(Debug)]
struct Admitted<'a> {
    engine: &'a WorkspaceEngine,
    path: &'a str,
    doc: model::Document,
    index: model::CorpusIndex,
}

impl DocumentCatalog for Admitted<'_> {
    fn revisions(&self) -> Box<dyn Iterator<Item = (&str, &model::NodeRev)> + '_> {
        Box::new(
            self.engine
                .revisions()
                .chain(std::iter::once((self.path, &self.doc.root.node_rev))),
        )
    }
    fn revision(&self, path: &str) -> Option<&model::NodeRev> {
        if path == self.path {
            Some(&self.doc.root.node_rev)
        } else {
            self.engine.revision(path)
        }
    }
    fn index(&self) -> &model::CorpusIndex {
        &self.index
    }
    fn link_roots(&self) -> BTreeSet<addr::MountName> {
        let mut roots = self.engine.link_roots();
        roots.extend(model::catalog::link_roots(&self.doc));
        roots
    }
    fn pin(&self, path: &str) -> io::Result<Option<Box<dyn DocumentLease + '_>>> {
        if path == self.path {
            Ok(Some(model::catalog::resident_lease(&self.doc)))
        } else {
            self.engine.pin(path)
        }
    }
}

impl LockFacts for Admitted<'_> {
    fn lock_roots(&self) -> BTreeSet<addr::MountName> {
        let mut roots = self.engine.lock_roots();
        roots.extend(walk::lock_fact(&self.doc).into_iter().flatten());
        roots
    }
    fn lock_holders(&self) -> Box<dyn Iterator<Item = &str> + '_> {
        Box::new(
            self.engine
                .lock_holders()
                .chain(walk::lock_fact(&self.doc).map(|_| self.path)),
        )
    }
}
