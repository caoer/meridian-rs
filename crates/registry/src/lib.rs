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
//! Registry entry ([`WorkspaceEntry`]) drives idle-reap ([`DEFAULT_IDLE_REAP`]).
//! Drawer sentinel (`cache` `registered.json`) drives 30-day last-use GC.
//! Register writes both; idle-reap drops only the entry — never conflate.

mod client;
mod engine;
mod protocol;
mod refresh;
mod registry;
pub mod ring;
mod server;
mod state;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub use client::Client;
pub use engine::{WarmOutcome, WorkspaceEngine};
pub use protocol::{DenyKind, Request, Response, WorkspaceEntry};
pub use registry::{RegisterOutcome, Registry, ResolveOutcome};
pub use server::{Config, RunningServer, default_socket_path};

/// Idle-reap horizon: unused entry (and its warm engine) dropped from memory
/// and state. Reaper never touches the drawer (`cache::gc` 30-day horizon).
/// One hour (G11) — entry cost is a warm parsed corpus, not a cheap
/// registration; re-register is one rebuild on next query.
// `Duration::from_hours`/`from_days` not const-stable at MSRV 1.96.
#[allow(clippy::duration_suboptimal_units)]
pub const DEFAULT_IDLE_REAP: Duration = Duration::from_secs(60 * 60);

/// Reaper scan cadence. Must be well under [`DEFAULT_IDLE_EXIT`].
#[allow(clippy::duration_suboptimal_units)]
pub const DEFAULT_REAP_INTERVAL: Duration = Duration::from_secs(60);

/// Idle-exit horizon (G11): no client request for this long ⇒ shut down.
/// Detached daemons are reparented to init; without this they are immortal.
/// Safe: daemon is an optimization — client auto-spawns on missing socket.
#[allow(clippy::duration_suboptimal_units)]
pub const DEFAULT_IDLE_EXIT: Duration = Duration::from_secs(15 * 60);

/// Push-plane write deadline (R2/S1). The daemon is thread-per-connection, so a
/// subscriber that stops draining parks an OS thread and its `SubGuard` for as
/// long as it stays wedged. Past this deadline the connection is dropped and the
/// subscription freed; the client redials and resyncs (§7.1). Matches the
/// client-side 10 s op deadline (D4).
#[allow(clippy::duration_suboptimal_units)]
pub const DEFAULT_PUSH_WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// Pre-warm sweep interval while busy (P2 — latency only; correctness is
/// fingerprint). Poll interim; OS notifier is the upgrade path.
pub const DEFAULT_PREWARM_INTERVAL: Duration = Duration::from_secs(1);

/// Pre-warm quiet backoff ceiling (G11). Interval doubles from
/// [`DEFAULT_PREWARM_INTERVAL`] toward this on quiet sweeps; rebuild or client
/// traffic restores base. Quiet path: stat signature first, then metadata walk
/// at this cadence — not a full corpus fold every second.
#[allow(clippy::duration_suboptimal_units)]
pub const DEFAULT_PREWARM_QUIET_MAX: Duration = Duration::from_secs(60);

/// Current unix time in whole seconds. Returns `0` if the clock predates the
/// epoch — a registration or state write must never be a failure mode, so this
/// never panics (mirrors `cache::now_secs`).
#[must_use]
pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
