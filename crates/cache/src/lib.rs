//! The central hashed cache drawer for meridian.
//!
//! A *drawer* is the per-workspace directory that holds a cache payload. This crate owns
//! four concerns and nothing else:
//!
//! 1. **Addressing** — [`cache_root`], [`drawer_key`], [`drawer_dir`] map a canonical
//!    workspace path to `<cache-root>/<sha256(path)[:16]>/<ver>-<salt>/`.
//! 2. **Registration** — [`register`] writes the [`Sentinel`] (`registered.json`) with
//!    crash-safe atomic + create-exclusive semantics; `mkdir` is not the registration
//!    signal, the sentinel is (amendment C1).
//! 3. **Probing** — [`probe`] stats the *sentinel*, never the directory. A half-created
//!    drawer, a corrupt sentinel, or a future-schema sentinel is a plain [`Probe::Miss`] —
//!    never an error, never a panic (downgrade gate).
//! 4. **Housekeeping** — [`stamp_last_use`], [`list_drawers`], [`gc`], [`remove_drawer`],
//!    each serialized by a per-drawer [`DrawerLock`] (m3) so the reaper never races a live
//!    workspace.
//!
//! # Precondition: the input path is already canonical
//!
//! Every function that takes a workspace path assumes it is **already canonical**
//! (absolute, symlinks resolved, on-disk case resolved). Canonicalization is
//! `crates/workspace`'s job; this crate never performs it, and never compiles against
//! that crate. Feed a non-canonical path and two spellings of one workspace hash to two
//! drawers — a caller bug, not ours.
//!
//! # Caching is an optimization, never a failure mode
//!
//! A caller with no drawer (tier-4 bare tree, no daemon) opens a
//! [`CacheDrawer::Ephemeral`] via [`CacheDrawer::open`]: an in-memory, per-invocation
//! handle that satisfies the same API as the disk-backed one and can never fail a run
//! (shipped Go `NewStore("")` precedent).

mod layout;
mod locking;
mod sentinel;
mod sweep;

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub use layout::{cache_root, drawer_dir, drawer_key, sock_key};
pub use locking::DrawerLock;
pub use sentinel::{Probe, Sentinel, probe, register, stamp_last_use, supersede};
pub use sweep::{DrawerInfo, GcReport, gc, list_drawers, remove_drawer};

/// Schema salt — the discriminator bumped whenever the on-disk *payload*
/// schema changes (amendment M4). It rides the drawer path segment
/// (`<ver>-<salt>`), so a bump lands payloads in a fresh directory for
/// `gc`/`clean` to reap — never a decode-into-the-wrong-struct. A bare version
/// segment corrupts on downgrade and dev builds; the salt is the real
/// discriminator.
///
/// `s1`: the drawer gained its first payload — `view::store`'s `sql.duckdb`
/// cache. Bump this together with `view::SCHEMA_VERSION` /
/// `view::store::CACHE_SCHEMA_VERSION` on any projection or hist DDL change:
/// the old drawer path simply stops resolving and `gc` reaps it.
///
/// `s2`: the dangling-base-exclusion ruling (2026-08-14) — `dangling` view
/// narrowed and the exclusion mint's fallback restamped `link.exclusion`.
///
/// `s3`: `frontmatter.prop_rev` — the per-key CAS token
/// (`node-rev-merkle-spec.md` §2.1) landed as a column on both the ephemeral
/// projection and `hist.frontmatter`.
///
/// `s4`: the `card` view became `record` (cards sql-record-rename +
/// sql-task-text-marker, one bump), and `task.text` dropped its list-marker +
/// checkbox prefix — every task row's payload bytes moved.
///
/// `s5`: `section.n` — the occurrence index served as its own column on both
/// the ephemeral projection and `hist.section` (card editset-n-column,
/// `wire-contract.md` § A.11).
///
/// `s6`: the `.base` projection (`docs/base-projection.md`) — three `hist.base*`
/// tables, `hist.link.exclusion_path`, and `hist.pin.base_fold`. The appender
/// loads POSITIONALLY, so an s5 drawer would land every column after
/// `exclusion` one slot left; and the §5.1 mint rule restamped `link.exclusion`
/// content for identical corpora, exactly as `s2`'s ruling did.
///
/// `s7`: the body projection — `body` relation on the ephemeral schema,
/// `hist.body` + `hist.body_text` on the cache (`docs/body-projection.md`).
///
/// `s8`: block-sequence `tags:` project their items (`model::fm_tags`). No
/// column moved, but an s7 drawer's rows are WRONG at a fingerprint that will
/// never advance — an unchanged file is never re-staged, so the append delta
/// cannot repair it and a fresh drawer is the only route (card
/// `tag-all-block-form-blindness`).
pub const SCHEMA_SALT: &str = "s8";

/// Default GC threshold: a drawer whose last-use is older than this is reapable.
/// 30 days, mirroring Cargo's registry auto-GC horizon. A path-keyed drawer store
/// without a reaper is the Bazel/VSCode disk-leak class (decision 0001 round 4).
// `Duration::from_days` is not const-stable at MSRV 1.96.
#[allow(clippy::duration_suboptimal_units)]
pub const DEFAULT_GC_THRESHOLD: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// Current unix time in whole seconds. Returns `0` if the clock predates the
/// epoch (never panics — sentinel writes must not be a failure mode).
#[must_use]
pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// A cache drawer handle: disk-backed when a cache root resolved, ephemeral when
/// it could not or the caller has no drawer (tier-4, no daemon).
///
/// Both variants expose the same API. The ephemeral variant is a cold, no-op
/// cache: every [`CacheDrawer::probe`] is a [`Probe::Miss`], registration and
/// stamping succeed without touching disk. Opening never fails.
#[derive(Debug, Clone)]
pub enum CacheDrawer {
    /// Persistent, addressed at `dir`. `workspace` is the canonical path written
    /// into the sentinel on registration.
    Disk {
        /// The drawer directory, `<cache-root>/<hash>/<ver>-<salt>/`.
        dir: PathBuf,
        /// The canonical workspace path this drawer belongs to.
        workspace: PathBuf,
    },
    /// In-memory, per-invocation. Carries the workspace path for the identity a
    /// no-op registration reports.
    Ephemeral {
        /// The canonical workspace path this ephemeral run belongs to.
        workspace: PathBuf,
    },
}

impl CacheDrawer {
    /// Open the disk drawer for a canonical `workspace`, degrading to
    /// [`CacheDrawer::Ephemeral`] when the cache root cannot be resolved (no
    /// `HOME`/`XDG_CACHE_HOME`). Never fails — caching is an optimization.
    #[must_use]
    pub fn open(workspace: &Path) -> Self {
        match cache_root() {
            Ok(root) => CacheDrawer::Disk {
                dir: drawer_dir(&root, workspace),
                workspace: workspace.to_path_buf(),
            },
            Err(_) => CacheDrawer::Ephemeral {
                workspace: workspace.to_path_buf(),
            },
        }
    }

    /// Force an ephemeral drawer for `workspace` regardless of cache-root
    /// availability (tier-4 bare tree with no `init` and no daemon).
    #[must_use]
    pub fn ephemeral(workspace: &Path) -> Self {
        CacheDrawer::Ephemeral {
            workspace: workspace.to_path_buf(),
        }
    }

    /// The on-disk drawer directory, or `None` for an ephemeral drawer.
    #[must_use]
    pub fn dir(&self) -> Option<&Path> {
        match self {
            CacheDrawer::Disk { dir, .. } => Some(dir),
            CacheDrawer::Ephemeral { .. } => None,
        }
    }

    /// Whether this drawer is in-memory only.
    #[must_use]
    pub fn is_ephemeral(&self) -> bool {
        matches!(self, CacheDrawer::Ephemeral { .. })
    }

    /// Probe the drawer. Disk drawers stat the sentinel; an ephemeral drawer is
    /// always a cold [`Probe::Miss`].
    #[must_use]
    pub fn probe(&self) -> Probe {
        match self {
            CacheDrawer::Disk { dir, .. } => probe(dir),
            CacheDrawer::Ephemeral { .. } => Probe::Miss,
        }
    }

    /// Register the drawer, returning the winning sentinel (adopted if another
    /// registrar won the race). An ephemeral drawer returns a fresh in-memory
    /// sentinel without touching disk and never fails.
    ///
    /// # Errors
    ///
    /// Propagates I/O failures from disk registration (directory creation,
    /// sentinel write). The ephemeral path is infallible.
    pub fn register(&self) -> std::io::Result<Sentinel> {
        match self {
            CacheDrawer::Disk { dir, workspace } => register(dir, workspace),
            CacheDrawer::Ephemeral { workspace } => Ok(Sentinel::fresh(workspace)),
        }
    }

    /// Stamp the drawer's last-use to now. No-op for an ephemeral drawer.
    ///
    /// # Errors
    ///
    /// Propagates I/O failures from the disk sentinel rewrite.
    pub fn stamp_last_use(&self) -> std::io::Result<()> {
        match self {
            CacheDrawer::Disk { dir, .. } => stamp_last_use(dir),
            CacheDrawer::Ephemeral { .. } => Ok(()),
        }
    }
}
