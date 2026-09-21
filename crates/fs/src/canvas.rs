//! The `.canvas` carrier walk (`move.md` §4 class 5) — the hash domain's rules
//! with the floor swapped from `*.md` to `*.canvas`.
//!
//! A JSON Canvas is never a corpus member: its bytes do not hash, it holds no
//! headings and answers no read. It is a *carrier* — it names pages by path —
//! and the move door rewrites those names, so the door must find canvases
//! exactly where a hashed page would be found and nowhere else.
//!
//! This crate stays JSON-free (its charter): it hands raw bytes up, and `query`
//! parses them beside its only consumer.

use std::io;
use std::path::{Path, PathBuf};

use crate::domain::Domain;
use crate::{DomainFiles, WorkspaceRoot};

/// Every `.canvas` file of the workspace as `(workspace-relative path, raw
/// bytes)`, path byte order — the shape the move door's planner consumes.
///
/// A canvas the walk found but cannot read is an ERROR, not an absence: the
/// door is about to rewrite references, and a carrier it could not read is one
/// it cannot promise anything about. This is the same posture
/// [`crate::hash_domain`]'s reader takes for a domain file.
///
/// # Errors
/// I/O failure loading the domain config, or reading a canvas the walk found.
pub fn canvas_files(root: &WorkspaceRoot) -> io::Result<DomainFiles> {
    let domain = Domain::load(root)?;
    canvas_files_under(root, &domain)
}

/// [`canvas_files`] against an already-loaded domain.
///
/// # Errors
/// As [`canvas_files`].
pub fn canvas_files_under(root: &WorkspaceRoot, domain: &Domain) -> io::Result<DomainFiles> {
    let mut rels: Vec<PathBuf> = Vec::new();
    crate::walk_extension_dir(&root.0, Path::new(""), domain, "canvas", &mut rels);
    rels.sort();

    let mut out = DomainFiles::with_capacity(rels.len());
    for rel in rels {
        let Some(path) = rel.to_str() else { continue };
        let bytes = std::fs::read(root.0.join(&rel))?;
        out.push((path.to_owned(), bytes));
    }
    Ok(out)
}
