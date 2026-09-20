//! Filesystem ownership of the §6.9 parsed snapshot.
//! The codec knows no paths on disk; registry owns scheduling and save ordering.
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};

use crate::engine::WorkspaceEngine;

const FILE: &str = "corpus.v1";

pub(crate) fn path(cache_root: &Path, workspace: &Path) -> PathBuf {
    cache::parsed_drawer_dir(cache_root, workspace).join(FILE)
}

pub(crate) fn restore(
    cache_root: &Path,
    workspace: &Path,
    fresh: &BTreeMap<PathBuf, [u8; 32]>,
) -> Option<parse_cache::snapshot::Restored> {
    let dir = cache::parsed_drawer_dir(cache_root, workspace);
    let cache::Probe::Hit(sentinel) = cache::probe(&dir) else {
        return None;
    };
    if sentinel.workspace != workspace.to_string_lossy() {
        return None;
    }
    let file = match File::open(dir.join(FILE)) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return None,
        Err(e) => {
            eprintln!(
                "parse-cache: cannot read {} ({e}); source fallback",
                workspace.display()
            );
            return None;
        }
    };
    match parse_cache::snapshot::read(BufReader::new(file), workspace, fresh) {
        Ok(prior) => {
            let _ = cache::stamp_last_use(&dir);
            eprintln!(
                "parse-cache: {} — {} document(s) reusable, complete={}",
                workspace.display(),
                prior.docs.len(),
                prior.complete
            );
            Some(prior)
        }
        Err(e) => {
            eprintln!(
                "parse-cache: {} — {e}; source fallback",
                workspace.display()
            );
            None
        }
    }
}

/// Called under the registry's single persistence gate, outside every request
/// mutex. One active engine pin, one encoded document buffer, one atomic file.
pub(crate) fn save(
    cache_root: &Path,
    workspace: &Path,
    engine: &WorkspaceEngine,
) -> io::Result<usize> {
    let dir = cache::parsed_drawer_dir(cache_root, workspace);
    cache::register(&dir, workspace)?;
    let cache::Probe::Hit(sentinel) = cache::probe(&dir) else {
        return Err(io::Error::other(
            "parsed drawer has no compatible registration",
        ));
    };
    if sentinel.workspace != workspace.to_string_lossy() {
        return Err(io::Error::other("parsed drawer workspace mismatch"));
    }
    let result = crate::cache_io::write(&dir, FILE, |output| {
        parse_cache::snapshot::write(
            output,
            workspace,
            &engine.at_fingerprint.0,
            &engine.docs,
            &engine.unserved,
            &engine.leaves,
        )
    });
    if result.is_ok() {
        cache::stamp_last_use(&dir)?;
    }
    result
}

pub(crate) fn discard(cache_root: &Path, workspace: &Path) {
    if let Err(e) = std::fs::remove_file(path(cache_root, workspace))
        && e.kind() != io::ErrorKind::NotFound
    {
        eprintln!("parse-cache: cannot discard {} ({e})", workspace.display());
    }
}
