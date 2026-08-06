//! The workspace identity layer: given a working directory, name the workspace it belongs
//! to.
//!
//! # Charter
//! **Owns:** the discovery ladder as pure filesystem functions (env override → git root →
//! cwd default), the canonicalization that defines identity (symlinks and on-disk case
//! resolved to one spelling), and the deny-ceiling predicate that refuses poisonous
//! workspace paths (`$HOME`, `/`, mount points, `/tmp`, XDG base dirs, the meridian cache
//! root).
//!
//! **Never does:** disk writes of any kind. No registration, no sentinel, no cache
//! directory creation — this crate only *names* a situation. Every rung resolves with no
//! daemon and no side effects; the bottom rung returns an [`Answer::CwdDefault`] carrying
//! the canonical cwd, and never auto-registers. Warming an unanchored tree is an explicit
//! `init` or the daemon's job, not this crate's.
//!
//! # The ladder answers one question
//! This ladder answers *"which root does this path belong to"*. Two other planes answer
//! *"which root did someone name"*, and neither is a rung here:
//!
//! - the **mount table** (`config::MountTable`) — name ↔ vault ↔ path for the roots this
//!   machine binds. It cannot be a rung: `config` depends on this crate for
//!   [`deny_reason`], so a declaration rung here would be a dependency cycle. A root's
//!   `MERIDIAN.md` self-declaration (`type: meridian-root`) is therefore read by
//!   `config`, never here.
//! - the **declared root** on the serve path — the `hello` frame's `workspace` field,
//!   pinned by `registry::Registry::pin_declared`. A daemon has no meaningful cwd, so it
//!   never enters this ladder.
//!
//! The three planes meet at exactly one point: [`deny_reason`], reused whole by both,
//! never re-implemented.
//!
//! **Dependencies:** `std` plus a single edge to `cache`, the one owner of cache-root
//! resolution (the deny ceiling must name the same root the drawer addressing uses). No
//! `wire`, `model`, or host edge; `cache` is a leaf utility, not a Go-facing crate.
//!
//! # Identity is [`canonicalize`]
//! Every path comparison runs over the canonical form. On this target (macOS/APFS,
//! case-insensitive default) [`std::fs::canonicalize`] resolves symlinks and returns the
//! on-disk directory-entry casing, so a case-variant spelling (`mixedcase`) resolves to
//! the real casing (`MixedCase`) and two spellings collapse to one identity. On a
//! case-sensitive filesystem a case-variant spelling is a genuinely different path and
//! correctly resolves to a different identity.

use std::env;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// Environment variable that pins the workspace explicitly (tier 1).
pub const ENV_WORKSPACE: &str = "MERIDIAN_WORKSPACE";

/// The git identity anchor. Present as a directory in a normal checkout,
/// or as a file in a linked worktree (`gitdir:` pointer). Either spelling
/// anchors the workspace at the directory that contains it; the pointer is
/// never read or followed (per-worktree identity).
const GIT_ENTRY: &str = ".git";

/// Ceiling on the ancestor walk. Bounds the stat calls per resolution so a
/// pathological deep tree (or a hung network mount) cannot make discovery
/// unbounded. Real workspace paths sit far below 64 path components; a
/// `.git` above this depth is not found, which degrades to the next rung
/// rather than hanging.
const MAX_WALK_DEPTH: usize = 64;

/// Which rung of the discovery ladder answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// The [`ENV_WORKSPACE`] override named the workspace — explicit.
    EnvOverride,
    /// The nearest ancestor holding a `.git` entry — inferred from
    /// filesystem structure.
    GitRoot,
    /// Nothing answered: the canonical cwd, a convenience default that is
    /// named but never registered.
    CwdDefault,
}

impl Tier {
    /// A stable lowercase label for JSON / human output.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::EnvOverride => "env-override",
            Self::GitRoot => "git-root",
            Self::CwdDefault => "cwd-default",
        }
    }
}

/// What the ladder answered, and which rung answered it.
///
/// An enum with no public path field, so a defaulted cwd can never silently
/// become "the workspace": [`Answer::root`] returns `None` for
/// [`Answer::CwdDefault`], and reaching a defaulted path takes
/// [`Answer::root_or_cwd`] — an explicit, greppable acknowledgment at the
/// call site. [`Display`](fmt::Display) writes the provenance sentence — tier
/// and root — so no caller has to assemble it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// [`ENV_WORKSPACE`] named this root.
    EnvOverride {
        /// The canonical workspace path.
        root: PathBuf,
    },
    /// The nearest ancestor `.git` entry anchored this root.
    GitRoot {
        /// The canonical workspace path.
        root: PathBuf,
    },
    /// Nothing answered — the canonical cwd, offered as a default only.
    CwdDefault {
        /// The canonical current directory.
        cwd: PathBuf,
    },
}

impl Answer {
    /// The root when something actually answered, `None` when the ladder
    /// fell through to the cwd default.
    ///
    /// This is the honest accessor: a `None` forces the caller to decide what
    /// an unanchored tree means for it, rather than inheriting a silent cwd.
    #[must_use]
    pub fn root(&self) -> Option<&Path> {
        match self {
            Self::EnvOverride { root } | Self::GitRoot { root } => Some(root),
            Self::CwdDefault { .. } => None,
        }
    }

    /// The path in every case, including the demoted default.
    ///
    /// Calling this IS the acknowledgment that a defaulted answer is
    /// acceptable here — it is deliberately more verbose than [`root`](Self::root)
    /// and greppable in review.
    #[must_use]
    pub fn root_or_cwd(&self) -> &Path {
        match self {
            Self::EnvOverride { root } | Self::GitRoot { root } => root,
            Self::CwdDefault { cwd } => cwd,
        }
    }

    /// Which rung answered.
    #[must_use]
    pub fn tier(&self) -> Tier {
        match self {
            Self::EnvOverride { .. } => Tier::EnvOverride,
            Self::GitRoot { .. } => Tier::GitRoot,
            Self::CwdDefault { .. } => Tier::CwdDefault,
        }
    }
}

impl fmt::Display for Answer {
    /// The provenance sentence: the tier word, then the path it named.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.tier().word(), self.root_or_cwd().display())
    }
}

/// A failure to resolve a workspace. Discovery itself never fails below the
/// override rung (a missing `.git` simply falls through to the cwd default);
/// only canonicalization can fail.
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
pub fn resolve(cwd: &Path) -> Result<Answer, ResolveError> {
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
) -> Result<Answer, ResolveError> {
    // Rung 1 — the explicit override wins. A relative override resolves
    // against the given cwd, so the function stays independent of the
    // process cwd.
    if let Some(raw) = workspace_override.filter(|value| !value.is_empty()) {
        let candidate = Path::new(raw);
        let joined = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            cwd.join(candidate)
        };
        let root =
            fs::canonicalize(&joined).map_err(|source| ResolveError::EnvWorkspaceNotFound {
                path: joined,
                source,
            })?;
        return Ok(Answer::EnvOverride { root });
    }

    // Canonicalize the cwd once: ancestors are then derived by
    // path-component trimming — no per-ancestor canonicalize syscall.
    let canonical = canonicalize(cwd)?;

    // Rung 2 — the nearest `.git`: the first hit wins outright and the walk
    // stops.
    for dir in canonical.ancestors().take(MAX_WALK_DEPTH) {
        if has_git(dir) {
            return Ok(Answer::GitRoot {
                root: dir.to_path_buf(),
            });
        }
    }

    // Rung 3 — nothing answered. Named, never registered, and reachable only
    // through `root_or_cwd`.
    Ok(Answer::CwdDefault { cwd: canonical })
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
/// Used by the daemon and by `init`; a pure, read-only predicate (it stats
/// paths, never writes). It refuses `$HOME`, the filesystem root, mount
/// points, `/tmp`, the XDG base directories, and any path descending from
/// the meridian cache root. Both `path` and the reference directories are
/// canonicalized before comparison, so an alternate spelling cannot smuggle
/// past the ceiling.
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
/// only names the root (for the deny ceiling); it creates nothing.
///
/// The resolution logic has one owner — [`cache::cache_root`] — so the deny
/// ceiling refuses exactly the directory the drawer addressing uses.
#[must_use]
pub fn cache_root() -> Option<PathBuf> {
    cache::cache_root().ok()
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
/// parent's (`st_dev` change).
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
