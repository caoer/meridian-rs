//! The wire protocol: NDJSON request/response types and the registry entry.
//!
//! One JSON object per line (`\n`-terminated), `serde_json` over the raw
//! socket — no framing crate, no new dependency. [`Request`] is tagged by an
//! `op` field, [`Response`] by a `status` field, so both are self-describing
//! and forward-compatible (an unknown tag is a decode error the peer reports
//! as [`Response::Error`], never a silent misparse).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A registered workspace: the canonical path plus its registry-side
/// timestamps. This is the registry's record, distinct from the `cache`
/// drawer sentinel — see the crate docs' "two lifecycles" note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceEntry {
    /// The canonical workspace path (symlinks and on-disk case resolved by
    /// [`workspace::canonicalize`]). The map key and the identity.
    pub workspace: PathBuf,
    /// Unix seconds at first registration.
    pub registered_at: u64,
    /// Unix seconds of the most recent register-adopt or resolve-adopt. Drives
    /// idle-reap; bumped in memory on every adoption (a `resolve` hit is an
    /// LRU touch).
    pub last_use: u64,
}

/// An RPC request, tagged by its `op` field.
///
/// Verbs (decision 0001 round 5): [`Self::Resolve`] (ancestor lookup — is the
/// cwd inside a registered workspace, adopt if so), [`Self::Register`]
/// (canonicalize → deny-ceiling → insert → drawer sentinel), [`Self::Unregister`],
/// [`Self::List`], [`Self::Ping`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// Liveness probe. The daemon answers [`Response::Pong`].
    Ping,
    /// Ancestor lookup: canonicalize `cwd`, then walk it and its ancestors
    /// against the registry. The nearest registered ancestor is adopted; no
    /// match is a [`Response::Miss`]. Never registers.
    ///
    /// The wire tag is `resolve_ws`, NOT `resolve`: the unified daemon socket
    /// also answers the frozen wire `resolve` op (the §4.5 walk plane), a
    /// different verb entirely. This admin verb (workspace ancestor lookup) is
    /// daemon-internal, so its tag is disambiguated here rather than touching the
    /// frozen `wire` vocabulary (U3 folds this into `hello`).
    #[serde(rename = "resolve_ws")]
    Resolve {
        /// The working directory to resolve.
        cwd: PathBuf,
    },
    /// Explicit registration of `path` as a warm workspace. Canonicalizes,
    /// enforces the deny ceiling, inserts first-writer-wins, and writes the
    /// drawer sentinel via the `cache` crate.
    Register {
        /// The workspace path to register (canonicalized daemon-side).
        path: PathBuf,
    },
    /// Drop `path` from the registry (memory + state file). The drawer is left
    /// for `cache::gc`. Returns [`Response::Unregistered`].
    Unregister {
        /// The workspace path to unregister.
        path: PathBuf,
    },
    /// List all registered workspaces. Returns [`Response::Listed`].
    List,
}

/// An RPC response, tagged by its `status` field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Response {
    /// Answer to [`Request::Ping`].
    Pong,
    /// A [`Request::Resolve`] found and adopted a registered ancestor.
    Resolved {
        /// The adopted entry (its `workspace` is the registered ancestor).
        entry: WorkspaceEntry,
    },
    /// A [`Request::Resolve`] found no registered ancestor — the caller should
    /// degrade to an ephemeral, per-invocation drawer.
    Miss,
    /// A [`Request::Register`] succeeded (or adopted a prior registration of
    /// the same path).
    Registered {
        /// The winning entry — identical for the first writer and any adopter.
        entry: WorkspaceEntry,
        /// `true` when this caller adopted an entry another writer created
        /// first; `false` when this caller created it. Both converge on one
        /// identity (first-writer-wins by serialization).
        adopted: bool,
    },
    /// A [`Request::Unregister`] completed.
    Unregistered {
        /// `true` when an entry was present and removed; `false` when the path
        /// was not registered.
        removed: bool,
    },
    /// Answer to [`Request::List`].
    Listed {
        /// Every registered workspace, unordered.
        entries: Vec<WorkspaceEntry>,
    },
    /// A [`Request::Register`] was refused by the deny ceiling — enforced in
    /// the daemon, not merely client-side (decision 0001 round 5, point 3).
    Denied {
        /// The typed reason, mirroring [`workspace::DenyReason`].
        reason: DenyKind,
        /// A human-readable rendering of `reason` for logs and CLI output.
        detail: String,
    },
    /// The request could not be served (bad input, canonicalization failure,
    /// an I/O error writing the sentinel). Carries a diagnostic message.
    Error {
        /// What went wrong.
        message: String,
    },
}

/// The serde-serializable mirror of [`workspace::DenyReason`]. The `workspace`
/// crate is `std`-only (no serde by Law 1's spirit — a leaf crate carries no
/// wire concern), so the daemon maps its reason into this typed, on-the-wire
/// enum rather than shipping a bare string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DenyKind {
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

impl From<workspace::DenyReason> for DenyKind {
    fn from(reason: workspace::DenyReason) -> Self {
        use workspace::DenyReason as R;
        match reason {
            R::FilesystemRoot => DenyKind::FilesystemRoot,
            R::HomeDir => DenyKind::HomeDir,
            R::TempDir => DenyKind::TempDir,
            R::XdgBaseDir => DenyKind::XdgBaseDir,
            R::CacheRoot => DenyKind::CacheRoot,
            R::MountPoint => DenyKind::MountPoint,
        }
    }
}

impl std::fmt::Display for DenyKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let reason = match self {
            DenyKind::FilesystemRoot => "filesystem root",
            DenyKind::HomeDir => "home directory",
            DenyKind::TempDir => "temporary directory",
            DenyKind::XdgBaseDir => "XDG base directory",
            DenyKind::CacheRoot => "meridian cache root",
            DenyKind::MountPoint => "mount point",
        };
        f.write_str(reason)
    }
}
