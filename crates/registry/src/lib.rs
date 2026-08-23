//! Daemon-held workspace registry (watchman model, decision 0001 round 5).
//!
//! **Owns:** long-lived daemon with a map keyed by canonical workspace path,
//! NDJSON RPC over a per-user unix socket ([`server`]), [`Client`], single-
//! writer atomic state file, and idle-reap.
//!
//! **Only writer of warm tier-4 registrations.** Bare trees stay ephemeral;
//! only explicit [`Request::Register`] creates an entry. Unregistered resolve
//! ⇒ [`Response::Miss`] (client degrades to ephemeral).
//!
//! **Never:** auto-register on resolve, name a workspace (`workspace` crate),
//! or own drawer payload lifecycle (`cache` crate).
//!
//! Library only ([`RunningServer::start`], [`Client`]); daemon binary wiring
//! is the `mrd` CLI.
//!
//! # Two lifecycles, one workspace
//! Registry entry ([`WorkspaceEntry`]) drives idle-reap ([`DEFAULT_IDLE_REAP`]):
//! the reap DEMOTES — warm engine, ring, read-mint ledger, and sql handle drop;
//! the entry, the §6.4 event feed, and the resident memo survive (merkle-spec
//! §6.4 registration-lifetime law — the feed's dirty set is what makes the next
//! warm O(dirty)). Drawer sentinel (`cache` `registered.json`) drives 30-day
//! last-use GC. Only `unregister` ends a registration — and the feed with it.

mod checkpoint;
mod client;
mod delta_sink;
mod engine;
mod feed;
mod mounts;
pub mod mw_sql;
mod protocol;
mod registry;
pub mod ring;
mod run_modules;
mod run_op;
mod script_op;
mod server;
mod sql_op;
mod state;
mod walk_op;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub use checkpoint::CheckpointReceipt;
pub use client::Client;
pub use engine::{WarmOutcome, WorkspaceEngine};
pub use feed::{FeedStats, RescanCause};
pub use protocol::{DenyKind, Request, Response, WorkspaceEntry};
pub use registry::{RegisterOutcome, Registry, ResolveOutcome};
pub use server::{
    Config, RunningServer, ServeOutcome, default_socket_path, in_process_registry, serve_lines,
    socket_path_for_cache_root, socket_path_under_home,
};

/// Idle-reap horizon: an unused workspace's warm serving state (engine, ring,
/// read-mint ledger, sql handle) drops from memory. The registration, its
/// §6.4 event feed, and the resident memo survive the horizon (merkle-spec
/// §6.4). Reaper never touches the drawer (`cache::gc` 30-day horizon).
// `Duration::from_hours`/`from_days` not const-stable at MSRV 1.96.
#[allow(clippy::duration_suboptimal_units)]
pub const DEFAULT_IDLE_REAP: Duration = Duration::from_secs(60 * 60);

/// Reaper scan cadence. Must be well under [`DEFAULT_IDLE_EXIT`].
#[allow(clippy::duration_suboptimal_units)]
pub const DEFAULT_REAP_INTERVAL: Duration = Duration::from_secs(60);

/// Idle-exit horizon (G11): no client request for this long ⇒ shut down.
/// Detached daemons are reparented to init; without this they are immortal.
#[allow(clippy::duration_suboptimal_units)]
pub const DEFAULT_IDLE_EXIT: Duration = Duration::from_secs(15 * 60);

/// Push-plane write deadline (R2/S1). The daemon is thread-per-connection, so a
/// subscriber that stops draining parks an OS thread and its `SubGuard`. Past
/// this deadline the connection is dropped and the subscription freed; the
/// client redials and resyncs (§7.1). Matches the client-side op deadline (D4).
#[allow(clippy::duration_suboptimal_units)]
pub const DEFAULT_PUSH_WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// Push-plane idle-write horizon (R2b): an armed sub with zero frames written
/// for this long is dropped; any frame written resets it. Covers the residency
/// mode nothing else bounds — owner wedged and workspace permanently quiet,
/// where neither [`DEFAULT_PUSH_WRITE_TIMEOUT`] nor the `peer_closed` probe can
/// fire.
///
/// Coupled: must stay above D3's 30-minute client-side drain TTL, else the
/// server pre-empts a healthy client's own TTL and churns idle feeds. Neither
/// number may be re-tuned without the other.
#[allow(clippy::duration_suboptimal_units)]
pub const DEFAULT_SUB_IDLE_WRITE_TIMEOUT: Duration = Duration::from_secs(45 * 60);

/// Pre-warm sweep interval while busy (P2 — latency only; correctness is
/// fingerprint).
pub const DEFAULT_PREWARM_INTERVAL: Duration = Duration::from_secs(1);

/// Pre-warm quiet backoff ceiling (G11). Interval doubles from
/// [`DEFAULT_PREWARM_INTERVAL`] toward this on quiet sweeps; rebuild or client
/// traffic restores base.
#[allow(clippy::duration_suboptimal_units)]
pub const DEFAULT_PREWARM_QUIET_MAX: Duration = Duration::from_secs(60);

/// Current unix time in whole seconds. Returns `0` if the clock predates the
/// epoch; never panics (mirrors `cache::now_secs`).
#[must_use]
pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
