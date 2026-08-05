//! The daemon-held workspace registry — the watchman model (decision 0001,
//! round 5).
//!
//! # Charter
//! **Owns:** the long-lived daemon that holds a mutex-guarded map keyed by
//! canonical workspace path, an NDJSON RPC surface over a per-user unix
//! socket ([`server`]), the matching [`Client`] library, the single-writer
//! atomic state file that lets warm registrations survive a restart, and the
//! idle-reap that drops long-unused entries.
//!
//! **The daemon is the ONLY writer of warm tier-4 registrations.** A bare
//! tree with no daemon stays ephemeral (cold, per-invocation memory); it is
//! never silently promoted. Only an explicit [`Request::Register`] — driven
//! by `mrd init` or a future daemon-mediated flow — creates an entry.
//! [`Request::Resolve`] on an unregistered tree returns [`Response::Miss`],
//! and the client degrades to ephemeral (decision 0001, round 5, point 6).
//!
//! **Never does:** auto-register on resolve, name a workspace (that is the
//! `workspace` crate's job — this crate calls [`workspace::canonicalize`] and
//! [`workspace::deny_reason`]), or own the drawer payload lifecycle (the
//! drawer sentinel it writes on register, and the drawer's own last-use GC,
//! belong to the `cache` crate; the registry entry and the drawer sentinel
//! are two separate lifecycles addressing one workspace).
//!
//! # Boundary with the `mrd` CLI
//! This crate is a library: the server engine ([`server::RunningServer`]) and
//! the [`Client`]. Wiring a daemon binary (fork/detach, signal handling, the
//! `mrd init` → register call) belongs to the `mrd` CLI task; this crate only
//! provides [`RunningServer::start`] and [`Client`].
//!
//! # Two lifecycles, one workspace
//! A *registry entry* ([`WorkspaceEntry`]) records that a workspace is warm
//! and drives idle-reap (one-hour default — see [`DEFAULT_IDLE_REAP`], and
//! why it is no longer watchman's five days). A *drawer
//! sentinel* (the `cache` crate's `registered.json`) records that the drawer
//! exists and drives the drawer's own 30-day last-use GC. Register writes
//! both; idle-reap drops only the registry entry — the drawer and its
//! sentinel outlive the registration and are reaped by `cache::gc` on their
//! own horizon. Never conflate the two.

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

/// Idle-reap horizon: a registry entry unused for longer than this is dropped
/// from memory and the state file, and its warm engine with it. The reaper only
/// deregisters; it never touches the drawer (that is `cache::gc`'s 30-day
/// horizon).
///
/// **One hour, not watchman's five days (G11).** The five-day figure copied
/// watchman's `idle_reap_age_seconds` without copying what an entry COSTS here:
/// watchman's is a cheap registration, ours pins a warm engine — a parsed
/// corpus resident in memory that a background thread keeps sweeping. Measured
/// on one host, that horizon held 281 daemons and 9.6 GB resident, from
/// workspaces nobody had touched for days. An hour keeps a workspace warm
/// across a working session, and re-registration after it is a single rebuild
/// on the next query.
// `Duration::from_hours`/`from_days` are not const-stable at MSRV 1.96, so the
// seconds form is the only option here (cache::DEFAULT_GC_THRESHOLD precedent);
// silence the "use a larger unit" pedantic lint.
#[allow(clippy::duration_suboptimal_units)]
pub const DEFAULT_IDLE_REAP: Duration = Duration::from_secs(60 * 60);

/// How often the daemon's reaper thread scans for idle entries and tests the
/// idle-exit horizon. Reaping is cheap; the scan cadence must be well under
/// [`DEFAULT_IDLE_EXIT`], or a daemon would outlive its horizon by up to a
/// whole interval.
#[allow(clippy::duration_suboptimal_units)]
pub const DEFAULT_REAP_INTERVAL: Duration = Duration::from_secs(60);

/// How long a daemon with NO client request stays resident before shutting
/// itself down (G11). Every daemon detaches with `setsid` and is reparented to
/// init, so nothing else will ever end it: without this horizon a daemon is
/// immortal, and each isolated test run — a fresh `XDG_CACHE_HOME` is a fresh
/// daemon — added one permanently (measured: exactly 8 leaked per gate run).
///
/// Exiting is safe because the daemon is an optimization, never a dependency:
/// a client that finds no socket auto-spawns one (decision 0002 §3), so the
/// only cost of exiting early is one cold start.
#[allow(clippy::duration_suboptimal_units)]
pub const DEFAULT_IDLE_EXIT: Duration = Duration::from_secs(15 * 60);

/// How often the daemon's pre-warm thread sweeps the warm workspaces while the
/// daemon is BUSY, so a file change pays its parse on the watch event, not on
/// the next query (decision 0002, P2 — latency only, correctness stays
/// fingerprint). This is the poll-based interim; an OS notifier
/// (FSEvents/inotify) is the future upgrade (decision 0001).
pub const DEFAULT_PREWARM_INTERVAL: Duration = Duration::from_secs(1);

/// The ceiling the pre-warm cadence backs off to while a workspace is quiet
/// (G11). The sweep interval doubles from [`DEFAULT_PREWARM_INTERVAL`] up to
/// this whenever a sweep finds nothing changed, and drops back to the base the
/// moment a rebuild happens or a client request arrives.
///
/// # Why a per-second sweep was never as cheap as its comment claimed
/// The old comment reasoned that "a quiet sweep only re-folds the content hash
/// (the cheap half — no parse), so a one-second cadence is inexpensive." Cheap
/// relative to parsing, yes; inexpensive, no — that fold READS EVERY BYTE of
/// the corpus. On a 20 GB vault it measured **28.6% of a core, continuously,
/// with zero client traffic**, against 0.07% for a one-file workspace: the
/// corpus size was the whole variable.
///
/// Two changes remove it. The sweep now asks a stat-only signature first
/// (`fs::domain_stat_signature`) and skips the content fold when nothing
/// moved, and that stat walk itself backs off to this ceiling while the
/// workspace stays quiet. A quiet corpus therefore costs one metadata walk a
/// minute instead of a full corpus read a second.
///
/// The latency cost is bounded and paid only by an idle daemon: an edit landing
/// during a backed-off stretch is picked up within this window, and a query
/// arriving before that rebuilds on the query path exactly as it did before the
/// pre-warm thread existed. Correctness never depended on the sweep.
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
