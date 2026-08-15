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
use crate::domain::{self, Domain};

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
    walk_base_dir(&root.0, Path::new(""), domain, &mut rels);
    rels.sort();

    let members: Vec<BaseMember> = rels
        .iter()
        .filter_map(|rel| {
            let path = rel.to_str()?.to_owned();
            let bytes = std::fs::read(root.0.join(rel)).map_err(|e| e.to_string());
            Some(BaseMember { path, bytes })
        })
        .collect();

    let leaves: Vec<model::BaseMemberLeaf<'_>> = members
        .iter()
        .map(|m| model::BaseMemberLeaf {
            path: &m.path,
            leaf: m.bytes.as_ref().ok().map(|b| model::leaf_digest(b)),
        })
        .collect();
    Ok(BaseSnapshot {
        members,
        fold: model::base_fold(&leaves),
    })
}

/// Is `name` — one path segment as READ FROM THE DIRECTORY — a `.base` member
/// name? Case-exact: `abc.BASE` is not a member, the case law the 2026-08-14
/// ruling ratified, applied here for the same reason (a case-folding match
/// would canonize typos on APFS).
fn is_base_name(name: &str) -> bool {
    Path::new(name).extension().is_some_and(|e| e == "base")
}

/// The membership walk. Dot-segments are skipped without descending (the
/// structural floor, above custom rules); custom-ignored directories prune the
/// same way the hash-domain walk prunes them. A directory that will not
/// enumerate contributes nothing — absence, per §3.
fn walk_base_dir(abs_dir: &Path, rel_dir: &Path, domain: &Domain, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(abs_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let name = entry.file_name();
        // A non-UTF-8 name can never match `.base` (§3) — and could not be
        // served as a wire path even if it did.
        let Some(name) = name.to_str() else { continue };
        let rel = rel_dir.join(name);
        if file_type.is_dir() {
            if domain::dot_segment(name) || domain.prunes_dir(&rel) {
                continue;
            }
            walk_base_dir(&entry.path(), &rel, domain, out);
        } else if file_type.is_file() && is_base_name(name) && in_base_domain(domain, &rel) {
            out.push(rel);
        }
    }
}

/// The §3 membership predicate for a FILE whose name already ends `.base`: the
/// hash domain's rules with the md-only floor swapped out.
///
/// [`Domain::exclusion`] answers `NonMarkdown` first for every `.base` path, so
/// the md floor is stepped over here and the other two rules — dot-segment and
/// custom-ignore — are asked exactly as the hash domain asks them. Asking
/// through the domain's own `exclusion` keeps ONE rule surface: a custom
/// ignore that moves md membership moves base membership in the same edit.
fn in_base_domain(domain: &Domain, rel: &Path) -> bool {
    !matches!(
        domain.exclusion(rel),
        Some(domain::ExclusionReason::DotSegment | domain::ExclusionReason::CustomIgnore)
    )
}
