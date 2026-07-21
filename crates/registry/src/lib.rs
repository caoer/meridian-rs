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
//! and drives idle-reap (5-day default, watchman's horizon). A *drawer
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
mod server;
mod state;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub use client::Client;
pub use engine::{WarmOutcome, WorkspaceEngine};
pub use protocol::{DenyKind, Request, Response, WorkspaceEntry};
pub use registry::{RegisterOutcome, Registry, ResolveOutcome};
pub use server::{Config, RunningServer, default_socket_path};

/// Idle-reap horizon: a registry entry unused for longer than this is dropped
/// from memory and the state file. 5 days mirrors watchman's
/// `idle_reap_age_seconds` default (432000s) — long enough that a workspace
/// touched across a normal work week survives, short enough that abandoned
/// trees do not accumulate. The reaper only deregisters; it never touches the
/// drawer (that is `cache::gc`'s 30-day horizon).
// `Duration::from_hours`/`from_days` are not const-stable at MSRV 1.96, so the
// seconds form is the only option here (cache::DEFAULT_GC_THRESHOLD precedent);
// silence the "use a larger unit" pedantic lint.
#[allow(clippy::duration_suboptimal_units)]
pub const DEFAULT_IDLE_REAP: Duration = Duration::from_secs(5 * 24 * 60 * 60);

/// How often the daemon's reaper thread scans for idle entries. Reaping is
/// cheap and non-urgent, so once an hour is ample; the reap horizon is days.
#[allow(clippy::duration_suboptimal_units)]
pub const DEFAULT_REAP_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// How often the daemon's pre-warm thread sweeps the warm workspaces so a file
/// change pays its parse on the watch event, not on the next query (decision
/// 0002, P2 — latency only, correctness stays fingerprint). Much shorter than
/// the reap horizon: pre-warm is latency-sensitive, reaping is a days-scale
/// cleanup. A quiet sweep only re-folds each warm corpus's content hash (the
/// cheap half — no parse), so a one-second cadence is inexpensive. This is the
/// poll-based interim; an OS notifier (FSEvents/inotify) is the future upgrade
/// (decision 0001, "Daemon optional: FSEvents/macOS, inotify/Linux").
pub const DEFAULT_PREWARM_INTERVAL: Duration = Duration::from_secs(1);

/// Current unix time in whole seconds. Returns `0` if the clock predates the
/// epoch — a registration or state write must never be a failure mode, so this
/// never panics (mirrors `cache::now_secs`).
#[must_use]
pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
