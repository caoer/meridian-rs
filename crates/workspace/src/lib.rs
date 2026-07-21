//! The workspace identity layer: given a working directory, name the
//! workspace it belongs to.
//!
//! # Charter
//! **Owns:** the discovery ladder as pure filesystem functions
//! (env override → `.meridian.toml` marker walk-up → git root → bare),
//! the canonicalization that defines identity (symlinks and on-disk case
//! resolved to one spelling), and the deny-ceiling predicate that refuses
//! poisonous workspace paths (`$HOME`, `/`, mount points, `/tmp`, XDG base
//! dirs, the meridian cache root).
//!
//! **Never does:** disk writes of any kind. No registration, no sentinel,
//! no cache directory creation — this crate only *names* a situation.
//! Tiers 1-3 resolve with no daemon and no side effects; tier 4 returns a
//! [`Tier::Bare`] outcome carrying the canonical cwd, and NEVER
//! auto-registers. Warming a bare tree is an explicit `init` or the
//! daemon's job (decision 0001, round 5), not this crate's.
//!
//! **Dependencies:** `std` only — no `wire`, `model`, or `sidecar` edge
//! (a leaf crate, Law 3 corollary).
//!
//! # Identity is [`canonicalize`]
//! Every path comparison and (later) fingerprint hash runs over the
//! canonical form. On this target (macOS/APFS, case-insensitive default)
//! [`std::fs::canonicalize`] resolves symlinks AND returns the on-disk
//! directory-entry casing — a case-variant spelling (`mixedcase`) resolves
//! to the real casing (`MixedCase`), so two spellings collapse to one
//! identity for free. This is proven, not assumed: see the case-variant
//! and symlink tests in `tests/discovery.rs`. Because `canonicalize`
//! already yields on-disk case here, no per-component directory-entry
//! resolver is needed. On a case-sensitive filesystem a case-variant
//! spelling is a genuinely different path and correctly resolves to a
//! different identity.

use std::env;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// Environment variable that pins the workspace explicitly (tier 1).
pub const ENV_WORKSPACE: &str = "MERIDIAN_WORKSPACE";

/// The canonical marker filename. `meridian-rs` is its only writer;
/// existence alone defines identity — the contents are never parsed here.
const MARKER_CANONICAL: &str = ".meridian.toml";

/// The legacy Go marker, recognized by EXISTENCE ONLY during transition.
/// Its contents (YAML) are never parsed; it is an identity anchor only.
const MARKER_LEGACY: &str = ".meridian.yaml";

/// The git identity anchor. Present as a directory in a normal checkout,
/// or as a FILE in a linked worktree (`gitdir:` pointer). Either spelling
/// anchors the workspace at the directory that contains it; the pointer is
/// never read or followed (per-worktree identity, decision 0001 round 4).
const GIT_ENTRY: &str = ".git";

/// Ceiling on the ancestor walk. Bounds the stat calls per resolution so a
/// pathological deep tree (or a hung network mount) cannot make discovery
/// unbounded. Real workspace paths sit far below 64 path components; a
/// marker or `.git` above this depth is not found, which degrades to the
/// next tier rather than hanging (M3, adversarial review).
const MAX_WALK_DEPTH: usize = 64;

/// Which tier of the discovery ladder resolved the workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// Tier 1: the [`ENV_WORKSPACE`] override named the workspace.
    EnvOverride,
    /// Tier 2: the nearest ancestor holding a marker file.
    Marker,
    /// Tier 3: the nearest ancestor holding a `.git` entry.
    GitRoot,
    /// Tier 4: no override, marker, or git — the canonical cwd itself,
    /// named but never registered.
    Bare,
}

/// The resolved workspace identity: the canonical workspace path plus the
/// tier that resolved it. When `tier` is [`Tier::Bare`], `workspace` is the
/// canonical cwd.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    /// The canonical workspace path (symlinks and on-disk case resolved).
    pub workspace: PathBuf,
    /// The tier that produced `workspace`.
    pub tier: Tier,
}

/// A failure to resolve a workspace. Discovery itself never fails for
/// tiers 2-4 (a missing marker/git simply falls through to bare); only
/// canonicalization can fail.
#[derive(Debug)]
pub enum ResolveError {
    /// [`ENV_WORKSPACE`] named a path that could not be canonicalized —
    /// typically because it does not exist. A loud, explicit-input error.
    EnvWorkspaceNotFound {
        /// The path (after joining a relative override onto the cwd).
        path: PathBuf,
        /// The underlying filesystem error.
        source: io::Error,
    },
    /// The working directory itself could not be canonicalized.
    Canonicalize {
        /// The path that failed to resolve.
        path: PathBuf,
        /// The underlying filesystem error.
        source: io::Error,
    },
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EnvWorkspaceNotFound { path, source } => write!(
                f,
                "{ENV_WORKSPACE} names a workspace path that cannot be resolved: {} ({source})",
                path.display()
            ),
            Self::Canonicalize { path, source } => write!(
                f,
                "cannot canonicalize workspace path {} ({source})",
                path.display()
            ),
        }
    }
}

impl std::error::Error for ResolveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::EnvWorkspaceNotFound { source, .. } | Self::Canonicalize { source, .. } => {
                Some(source)
            }
        }
    }
}

/// Resolve `path` to its canonical form — the identity function.
///
/// Resolves `.`/`..`/`//`, symlinks, and (on a case-insensitive
/// filesystem) on-disk case, so every alternate spelling of one directory
/// collapses to a single identity.
///
/// # Errors
/// Returns [`ResolveError::Canonicalize`] when `path` does not exist or
/// cannot be resolved.
pub fn canonicalize(path: &Path) -> Result<PathBuf, ResolveError> {
    fs::canonicalize(path).map_err(|source| ResolveError::Canonicalize {
        path: path.to_path_buf(),
        source,
    })
}

/// Resolve the workspace for `cwd`, reading the tier-1 override from the
/// process environment ([`ENV_WORKSPACE`]).
///
/// # Errors
/// Returns [`ResolveError::EnvWorkspaceNotFound`] when the override names an
/// unresolvable path, or [`ResolveError::Canonicalize`] when `cwd` itself
/// cannot be canonicalized.
pub fn resolve(cwd: &Path) -> Result<Resolution, ResolveError> {
    resolve_with_override(cwd, env::var_os(ENV_WORKSPACE).as_deref())
}

/// Resolve the workspace for `cwd` with the tier-1 override supplied
/// explicitly (an empty override is treated as unset). This is the pure
/// core [`resolve`] wraps; taking the override as a parameter keeps the
/// ladder testable without mutating the process environment.
///
/// # Errors
/// Returns [`ResolveError::EnvWorkspaceNotFound`] when `workspace_override`
/// names an unresolvable path, or [`ResolveError::Canonicalize`] when `cwd`
/// itself cannot be canonicalized.
pub fn resolve_with_override(
    cwd: &Path,
    workspace_override: Option<&OsStr>,
) -> Result<Resolution, ResolveError> {
    // Tier 1 — explicit override wins. A relative override resolves against
    // the given cwd, so the function stays independent of the process cwd.
    if let Some(raw) = workspace_override.filter(|value| !value.is_empty()) {
        let candidate = Path::new(raw);
        let joined = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            cwd.join(candidate)
        };
        let workspace =
            fs::canonicalize(&joined).map_err(|source| ResolveError::EnvWorkspaceNotFound {
                path: joined,
                source,
            })?;
        return Ok(Resolution {
            workspace,
            tier: Tier::EnvOverride,
        });
    }

    // Canonicalize the cwd ONCE (M3): ancestors are then derived by
    // path-component trimming — no per-ancestor canonicalize syscall.
    let canonical = canonicalize(cwd)?;

    // Tiers 2 and 3 share one upward walk. A marker beats git GLOBALLY, so
    // the first marker found (nearest) wins outright; git is only a
    // fallback, so we remember the nearest `.git` but keep climbing for a
    // higher marker before committing to it.
    let mut nearest_git: Option<PathBuf> = None;
    for dir in canonical.ancestors().take(MAX_WALK_DEPTH) {
        if has_marker(dir) {
            return Ok(Resolution {
                workspace: dir.to_path_buf(),
                tier: Tier::Marker,
            });
        }
        if nearest_git.is_none() && has_git(dir) {
            nearest_git = Some(dir.to_path_buf());
        }
    }
    if let Some(git_root) = nearest_git {
        return Ok(Resolution {
            workspace: git_root,
            tier: Tier::GitRoot,
        });
    }

    // Tier 4 — bare. Named, never registered.
    Ok(Resolution {
        workspace: canonical,
        tier: Tier::Bare,
    })
}

/// True when `dir` holds a marker file — the canonical `.meridian.toml` or
/// the legacy `.meridian.yaml`. Existence defines identity; contents are
/// never read.
fn has_marker(dir: &Path) -> bool {
    dir.join(MARKER_CANONICAL).exists() || dir.join(MARKER_LEGACY).exists()
}

/// True when `dir` holds a `.git` entry — directory or file. A file is a
/// linked worktree; either way this directory is the workspace and the
/// `gitdir:` pointer is never read.
fn has_git(dir: &Path) -> bool {
    dir.join(GIT_ENTRY).exists()
}

/// Why a path is refused as a workspace ceiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenyReason {
    /// The filesystem root `/`.
    FilesystemRoot,
    /// The user's `$HOME` directory.
    HomeDir,
    /// The system temporary directory (`/tmp`).
    TempDir,
    /// An XDG base directory (cache/config/data/state).
    XdgBaseDir,
    /// The meridian cache root, or any path descending from it.
    CacheRoot,
    /// A mount point (its device differs from its parent's).
    MountPoint,
}

impl fmt::Display for DenyReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self {
            Self::FilesystemRoot => "filesystem root",
            Self::HomeDir => "home directory",
            Self::TempDir => "temporary directory",
            Self::XdgBaseDir => "XDG base directory",
            Self::CacheRoot => "meridian cache root",
            Self::MountPoint => "mount point",
        };
        f.write_str(reason)
    }
}

/// The deny-ceiling predicate: return the reason `path` must not become a
/// workspace, or `None` when it is acceptable.
///
/// Used later by the daemon and by `init`; here it is a pure, read-only
/// predicate (it stats paths, never writes). It refuses `$HOME`, the
/// filesystem root, mount points, `/tmp`, the XDG base directories, and any
/// path descending from the meridian cache root (M1/m1, adversarial
/// review). Both `path` and the reference directories are canonicalized
/// before comparison, so an alternate spelling cannot smuggle past the
/// ceiling.
#[must_use]
pub fn deny_reason(path: &Path) -> Option<DenyReason> {
    let target = resolve_ref(path);

    if target == Path::new("/") {
        return Some(DenyReason::FilesystemRoot);
    }
    if let Some(home) = env_dir("HOME")
        && target == resolve_ref(&home)
    {
        return Some(DenyReason::HomeDir);
    }
    if target == resolve_ref(Path::new("/tmp")) {
        return Some(DenyReason::TempDir);
    }
    for base in xdg_base_dirs() {
        if target == resolve_ref(&base) {
            return Some(DenyReason::XdgBaseDir);
        }
    }
    if let Some(root) = cache_root() {
        let root = resolve_ref(&root);
        if target == root || target.starts_with(&root) {
            return Some(DenyReason::CacheRoot);
        }
    }
    if is_mount_point(&target) {
        return Some(DenyReason::MountPoint);
    }
    None
}

/// The meridian cache root: `${XDG_CACHE_HOME:-$HOME/.cache}/meridian`.
/// Returns `None` when neither `XDG_CACHE_HOME` nor `HOME` is set. This
/// only NAMES the root (for the deny ceiling); it creates nothing.
#[must_use]
pub fn cache_root() -> Option<PathBuf> {
    let base =
        env_dir("XDG_CACHE_HOME").or_else(|| env_dir("HOME").map(|home| home.join(".cache")))?;
    Some(base.join("meridian"))
}

/// Canonicalize a reference path, falling back to the path as-given when it
/// cannot be resolved (a not-yet-created cache root, for instance).
fn resolve_ref(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Read an environment variable as a non-empty directory path.
fn env_dir(key: &str) -> Option<PathBuf> {
    env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// The XDG base directories, honoring the `XDG_*_HOME` overrides and
/// falling back to the spec defaults under `$HOME`.
fn xdg_base_dirs() -> Vec<PathBuf> {
    let home = env_dir("HOME");
    let defaults = [
        ("XDG_CACHE_HOME", ".cache"),
        ("XDG_CONFIG_HOME", ".config"),
        ("XDG_DATA_HOME", ".local/share"),
        ("XDG_STATE_HOME", ".local/state"),
    ];
    defaults
        .iter()
        .filter_map(|&(key, default)| {
            env_dir(key).or_else(|| home.as_ref().map(|home| home.join(default)))
        })
        .collect()
}

/// True when `path` is a mount point: its device id differs from its
/// parent's (`st_dev` change, per the adversarial review's ceiling rule).
fn is_mount_point(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    let Some(parent) = path.parent() else {
        return false;
    };
    let Ok(parent_meta) = fs::metadata(parent) else {
        return false;
    };
    meta.dev() != parent_meta.dev()
}
