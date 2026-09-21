//! The `.base` membership walk (`base-projection.md` §3) — the hash domain's
//! rules with the floor swapped from `*.md` to `*.base`.
//!
//! It lives here, beside [`crate::domain_snapshot`], because membership is a
//! DIRECTORY-ENUMERATION question: paths are on-disk spellings by construction,
//! a directory that cannot be enumerated reads as absence, and a member whose
//! bytes cannot be read is NOT absence — the walk saw it, so it comes back with
//! its error attached (§4.4).
//!
//! This crate stays YAML-free (its charter): it hands raw bytes up, and `view`
//! parses them beside its only consumer.

use std::io;
use std::path::{Path, PathBuf};

use crate::WorkspaceRoot;
use crate::domain::Domain;

/// One `.base` member as the walk found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseMember {
    /// Workspace-relative ON-DISK spelling (§3 — never a caller's spelling).
    pub path: String,
    /// The member's raw bytes, or the message of whatever refused to read them
    /// (§4.4: a member the walk saw but could not read is a NAMED row, never
    /// an absence).
    pub bytes: Result<Vec<u8>, String>,
}

/// A `.base` walk's whole finding: the members and the `bf:` witness folded
/// over them (`base-projection.md` §6.2).
#[derive(Debug, Clone)]
pub struct BaseSnapshot {
    /// Members in path byte order — the order the fold used.
    pub members: Vec<BaseMember>,
    /// `bf:` + hex ([`model::base_fold`]).
    pub fold: String,
}

/// Walk `root` for `.base` members and fold the §6.2 witness over them.
///
/// Membership (§3): final extension exactly `.base`, **case-exact** against the
/// name read from the directory, under the SAME ignore rules the hash domain
/// applies — the dot-segment floor and `meridian/domain.md`'s custom list. So
/// membership moves when the domain config moves, and there is no second rule
/// surface to maintain.
///
/// This is a distinct walk from [`crate::domain::LinkTargetProbe`]'s fallback
/// index, which deliberately does NOT prune custom-ignored directories
/// (excluded files are exactly what that index exists to find).
///
/// # Errors
/// I/O failure loading the domain config. A per-entry read failure is NOT an
/// error: it rides its member as [`BaseMember::bytes`]`::Err`. A directory that
/// cannot be enumerated reads as absence (its paths are unknowable — the
/// standing posture).
pub fn base_snapshot(root: &WorkspaceRoot) -> io::Result<BaseSnapshot> {
    let domain = Domain::load(root)?;
    base_snapshot_under(root, &domain)
}

/// [`base_snapshot`] against an already-loaded domain — the shape a caller who
/// holds the domain (every sql lane does) uses to avoid a second config read.
///
/// # Errors
/// As [`base_snapshot`].
pub fn base_snapshot_under(root: &WorkspaceRoot, domain: &Domain) -> io::Result<BaseSnapshot> {
    let mut rels: Vec<PathBuf> = Vec::new();
    crate::walk_extension_dir(&root.0, Path::new(""), domain, "base", &mut rels);
    rels.sort();

    let members: Vec<BaseMember> = rels
        .iter()
        .filter_map(|rel| {
            let path = rel.to_str()?.to_owned();
            let bytes = std::fs::read(root.0.join(rel)).map_err(|e| e.to_string());
            Some(BaseMember { path, bytes })
        })
        .collect();

    // The fold is computed BEFORE the members move into the snapshot: the
    // leaves borrow them, so folding inside the struct literal would move and
    // borrow in one expression.
    let fold = {
        let leaves: Vec<model::BaseMemberLeaf<'_>> = members
            .iter()
            .map(|m| model::BaseMemberLeaf {
                path: &m.path,
                leaf: m.bytes.as_ref().ok().map(|b| model::leaf_digest(b)),
            })
            .collect();
        model::base_fold(&leaves)
    };
    Ok(BaseSnapshot { members, fold })
}

// Membership itself — the case-exact extension match and the two rules that
// survive the swapped floor — is [`crate::walk_extension_dir`], shared with the
// `.canvas` carrier walk so one edit moves both.
