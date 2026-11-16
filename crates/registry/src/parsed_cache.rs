//! Filesystem ownership of the §6.9 parsed snapshot.
//! The codec knows no paths on disk; registry owns scheduling and save ordering.
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::engine::WorkspaceEngine;
use crate::residency::catalog::{Slot, Slots};

const FILE: &str = "corpus.v1";

pub(crate) fn path(cache_root: &Path, workspace: &Path) -> PathBuf {
    cache::parsed_drawer_dir(cache_root, workspace).join(FILE)
}

/// The workspace's registered snapshot, opened for a restore, with its
/// drawer. `None` when the drawer is not this workspace's or holds no
/// readable snapshot.
fn open(cache_root: &Path, workspace: &Path) -> Option<(PathBuf, File)> {
    let dir = cache::parsed_drawer_dir(cache_root, workspace);
    let cache::Probe::Hit(sentinel) = cache::probe(&dir) else {
        return None;
    };
    if sentinel.workspace != workspace.to_string_lossy() {
        return None;
    }
    match File::open(dir.join(FILE)) {
        Ok(file) => Some((dir, file)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => {
            timing::say!(
                "parse-cache: cannot read {} ({e}); source fallback",
                workspace.display()
            );
            None
        }
    }
}

pub(crate) fn restore(
    cache_root: &Path,
    workspace: &Path,
    fresh: &BTreeMap<PathBuf, [u8; 32]>,
) -> Option<parse_cache::snapshot::Restored> {
    let (dir, file) = open(cache_root, workspace)?;
    match parse_cache::snapshot::read(BufReader::new(file), workspace, fresh) {
        Ok(prior) => {
            let _ = cache::try_stamp_last_use(&dir);
            timing::say!(
                "parse-cache: {} — {} document(s) reusable, complete={}",
                workspace.display(),
                prior.docs.len(),
                prior.complete
            );
            Some(prior)
        }
        Err(e) => {
            timing::say!(
                "parse-cache: {} — {e}; source fallback",
                workspace.display()
            );
            None
        }
    }
}

/// A snapshot adopted into a residency generation's parts (merkle-spec §6.9,
/// restore under a host): the prior of a cold pass over slots.
#[derive(Debug)]
pub(crate) struct Adopted {
    slots: Slots,
    unserved: BTreeMap<String, String>,
    leaves: BTreeMap<PathBuf, [u8; 32]>,
    index: Arc<model::CorpusIndex>,
    /// The saved engine stamp, never a statement about current disk.
    pub(crate) fingerprint: String,
    /// False after any damaged record or missing/invalid footer.
    pub(crate) complete: bool,
}

impl Adopted {
    /// These parts as the prior of the residency pass
    /// ([`crate::residency::catalog::update`]).
    pub(crate) fn as_prior(&self) -> fs::CorpusPrior<'_, Slots> {
        fs::CorpusPrior {
            docs: &self.slots,
            unserved: &self.unserved,
            leaves: &self.leaves,
            index: Some(&self.index),
        }
    }
}

/// [`restore`] under a residency host: each record `fresh` wants is decoded,
/// published as a cold slot at every current path of its digest, entered
/// into the name index, and dropped before the next is read. `None` when
/// there is no usable snapshot or a slot cannot be published.
pub(crate) fn adopt(
    cache_root: &Path,
    workspace: &Path,
    fresh: &BTreeMap<PathBuf, [u8; 32]>,
    host: &crate::residency::Host,
) -> Option<Adopted> {
    use fs::SlotMap as _;
    use parse_cache::snapshot::Object;

    let (dir, file) = open(cache_root, workspace)?;
    // A non-UTF-8 name is integrity-covered but never parsed or served: its
    // leaf alone is the whole member.
    let mut leaves = BTreeMap::new();
    let mut paths: BTreeMap<[u8; 32], Vec<&str>> = BTreeMap::new();
    for (path, digest) in fresh {
        match path.to_str() {
            Some(name) => paths.entry(*digest).or_default().push(name),
            None => {
                leaves.insert(path.clone(), *digest);
            }
        }
    }
    let wanted = paths.keys().copied().collect();
    let mut slots = Slots::default();
    let mut unserved = BTreeMap::new();
    let mut index = model::CorpusIndex::new();
    let scanned = parse_cache::snapshot::scan(
        BufReader::new(file),
        workspace,
        &wanted,
        |digest, object| {
            for name in paths.get(digest).into_iter().flatten() {
                match &object {
                    Object::Doc(doc) => {
                        let slot = Slot::publish(doc, &host.controller, &host.store)?;
                        index.insert_ordered(name, doc);
                        slots.insert((*name).to_owned(), Arc::new(slot));
                    }
                    Object::Unserved(why) => {
                        unserved.insert((*name).to_owned(), why.clone());
                    }
                }
                leaves.insert(PathBuf::from(*name), *digest);
            }
            Ok(())
        },
    );
    match scanned {
        Ok(scanned) => {
            let _ = cache::try_stamp_last_use(&dir);
            timing::say!(
                "parse-cache: {} — {} document(s) reusable, complete={}",
                workspace.display(),
                slots.len(),
                scanned.complete
            );
            Some(Adopted {
                slots,
                unserved,
                leaves,
                index: Arc::new(index),
                fingerprint: scanned.fingerprint,
                complete: scanned.complete,
            })
        }
        Err(e) => {
            timing::say!(
                "parse-cache: {} — {e}; source fallback",
                workspace.display()
            );
            None
        }
    }
}

/// Called under the registry's single persistence gate, outside every request
/// mutex. One active engine pin, one encoded document buffer, one atomic file.
/// A generation that owns its documents encodes each of them; one built under
/// a residency host copies the records of the snapshot it replaces and parses
/// only the members none covers, from their retained backing and outside the
/// allowance (merkle-spec §6.9, saving under a host).
/// Returns the documents the snapshot holds, `None` when the drawer is busy.
pub(crate) fn save(
    cache_root: &Path,
    workspace: &Path,
    engine: &WorkspaceEngine,
) -> io::Result<Option<usize>> {
    let dir = cache::parsed_drawer_dir(cache_root, workspace);
    let Some(_) = cache::try_register(&dir, workspace)? else {
        return Ok(None);
    };
    let cache::Probe::Hit(sentinel) = cache::probe(&dir) else {
        return Err(io::Error::other(
            "parsed drawer has no compatible registration",
        ));
    };
    if sentinel.workspace != workspace.to_string_lossy() {
        return Err(io::Error::other("parsed drawer workspace mismatch"));
    }
    let result = crate::cache_io::write(&dir, FILE, |output| {
        if let Some(docs) = engine.eager_docs() {
            return parse_cache::snapshot::write(
                output,
                workspace,
                &engine.at_fingerprint.0,
                docs,
                &engine.unserved,
                &engine.leaves,
            );
        }
        let prior = File::open(dir.join(FILE)).ok().map(BufReader::new);
        let written = parse_cache::snapshot::rewrite(
            output,
            workspace,
            &engine.at_fingerprint.0,
            &engine.unserved,
            &engine.leaves,
            prior,
            |path, digest| {
                engine
                    .retained_document(path, digest)
                    .as_ref()
                    .and_then(parse_cache::encode)
            },
        )?;
        Ok(written.carried + written.encoded)
    });
    if result.is_ok() {
        cache::try_stamp_last_use(&dir)?;
    }
    result.map(Some)
}

pub(crate) fn discard(cache_root: &Path, workspace: &Path) {
    if let Err(e) = std::fs::remove_file(path(cache_root, workspace))
        && e.kind() != io::ErrorKind::NotFound
    {
        timing::say!("parse-cache: cannot discard {} ({e})", workspace.display());
    }
}
