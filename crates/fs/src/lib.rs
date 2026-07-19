//! Disk truth in, atomic splices out: read/walk/watch feeding the model;
//! tmp+fsync+rename splice execution.
//!
//! # Charter
//! **Owns:** the disk boundary. Reading workspace files (refusing non-UTF-8 —
//! spans must denote exact disk bytes or splice corrupts files), walking the
//! corpus, watching for changes (rung 4), and *executing* validated splices
//! atomically (tmp + fsync + rename — meridian's write discipline relocates
//! verbatim). Feeds `model` and nothing else.
//!
//! **Never does:** writing bytes it didn't splice, caching anything to disk
//! (law 2: no snapshot files, no second database — the moment memory can't be
//! thrown away, the architecture has been violated), interpreting content.
//!
//! # Law enforcement (candidate thesis, this crate's part)
//! Write execution demands `model::ValidatedSplice` — a token only `model`'s CAS
//! validation can mint. An unvalidated write cannot reach disk by construction;
//! the splice pipeline (validate in `model`, execute here) is enforced by types,
//! not review.
//!
//! # Rungs
//! Rung 1: read + walk. Rung 2: atomic splice execution. Rung 4: watch feeds
//! the daemon's change feed.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub mod domain;

/// The workspace root every wire `path` resolves strictly inside. Constructed
/// once at process start; path-escape rejection (`bad_path`) anchors here.
#[derive(Debug, Clone)]
pub struct WorkspaceRoot(pub PathBuf);

/// Read one workspace file into a parsed document (via `syntax` + `model`).
/// Non-UTF-8 files are refused, never lossy-decoded (wire-contract §8 row 1).
///
/// # Errors
/// I/O failure reading the file, or non-UTF-8 content (refused).
pub fn load(root: &WorkspaceRoot, rel_path: &Path) -> io::Result<model::Document> {
    let _ = (root, rel_path);
    todo!("rung 1: read → syntax::parse → model::build")
}

/// Walk the corpus: every markdown file under the root, as root-relative paths,
/// sorted. This is the ADDRESSABLE set — dot-dir md files (`.github/README.md`)
/// are included, since they stay `load`-able even when ignored for hashing
/// (§12.1). Cold rebuild of the whole world model from this walk is the
/// recovery path — measured 2.16 s. Symlinks are not followed.
///
/// # Errors
/// I/O failure traversing the root.
pub fn walk(root: &WorkspaceRoot) -> io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    walk_dir(&root.0, &PathBuf::new(), &mut out)?;
    out.sort();
    Ok(out)
}

fn walk_dir(abs_dir: &Path, rel_dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(abs_dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let name = entry.file_name();
        let rel = rel_dir.join(&name);
        if file_type.is_dir() {
            walk_dir(&entry.path(), &rel, out)?;
        } else if file_type.is_file()
            && Path::new(&name)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("md"))
        {
            out.push(rel);
        }
    }
    Ok(())
}

/// The §12 hash domain: [`walk`] narrowed to the files whose bytes enter the
/// merkle root — md-only, dot-segment-ignored, and custom-ignored removed. This
/// is where `walk` gains the [`domain`] filter; the filter gates HASHING, never
/// `load` — an ignored md file is absent here yet still addressable by path
/// (`hash ⊂ addressable`, §12.1).
///
/// # Errors
/// I/O failure traversing the root.
pub fn hash_domain(root: &WorkspaceRoot, domain: &domain::Domain) -> io::Result<Vec<PathBuf>> {
    Ok(walk(root)?
        .into_iter()
        .filter(|rel| domain.contains(rel))
        .collect())
}

/// Execute a validated splice atomically: write the spliced content to a temp
/// file, fsync, rename over the original. Accepts only `model`'s capability
/// token — see crate doc.
///
/// # Errors
/// I/O failure at any step of tmp-write, fsync, or rename.
pub fn apply_splice(
    root: &WorkspaceRoot,
    rel_path: &Path,
    splice: &model::ValidatedSplice,
) -> io::Result<()> {
    let _ = (root, rel_path, splice);
    todo!("rung 2: tmp + fsync + rename")
}

/// Filesystem watcher (rung 4): feeds model diffs to the daemon's change feed.
/// Hook *dispatch* (running agent work on change) stays Go — Rust never
/// executes agent work.
#[derive(Debug)]
pub struct Watcher {
    _root: WorkspaceRoot,
}

impl Watcher {
    #[must_use]
    pub fn new(root: WorkspaceRoot) -> Self {
        Watcher { _root: root }
    }
}
