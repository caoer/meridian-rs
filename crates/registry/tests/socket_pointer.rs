//! The published socket pointer (`server.rs` § Socket placement): the daemon
//! writes the path it bound at `<cache-root>/registry/socket`, keyed by the
//! cache root alone, so a client whose OWN environment derives a different
//! socket for that cache root still reaches the flock holder.
//!
//! Why a derivation alone was not enough. Client and daemon derive through
//! one function, but from their own environments — and on workstation-nyc-2
//! a systemd-started process carried `XDG_RUNTIME_DIR=/run/user/1000` while
//! a non-interactive ssh shell had it unset. Two sockets for one cache root;
//! the shell's spawn lost the flock and exited; the holder answered nobody
//! from that shell.
//!
//! These gates hold the pointer to the pidfile's order (written before the
//! accept loop serves, removed at shutdown) and hold the reader to the one
//! rule that makes a stale pointer harmless: presence is not liveness, so
//! the published path is dialed before it is trusted.

use std::fs;
use std::time::Duration;

use registry::{
    Client, Config, RunningServer, published_socket_path, reachable_socket_path,
    socket_path_for_cache_root, socket_pointer_path,
};
use tempfile::TempDir;

/// A daemon config in the production LAYOUT under `tmp` (`for_cache_root`:
/// state and flock at `<cache-root>/registry/`), background horizons parked,
/// and the socket placed BY HAND where the law would never derive it — the
/// disagreement the pointer exists to bridge, produced without touching this
/// process's environment.
// `Duration::from_hours` is not const-stable at MSRV 1.96; the seconds form is
// the workspace precedent (cache::DEFAULT_GC_THRESHOLD).
#[allow(clippy::duration_suboptimal_units)]
fn test_config(tmp: &TempDir) -> Config {
    let forever = Duration::from_secs(365 * 24 * 60 * 60);
    let mut config = Config::for_cache_root(tmp.path().join("cache"));
    config.socket_path = tmp.path().join("elsewhere").join("daemon.sock");
    config.idle_threshold = forever;
    config.reap_interval = forever;
    config.prewarm_interval = forever;
    config.prewarm_quiet_max = forever;
    config.idle_exit = None;
    config.drain_cold_builds = Duration::from_secs(30);
    config
}

/// The pidfile's order, for the pointer: when `start` returns the accept
/// loop is live, so the pointer already names the bound socket — and it
/// sits in the registry directory, beside the state file and the flock.
#[test]
fn start_publishes_the_bound_socket_in_the_registry_directory() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let cache_root = config.cache_root.clone();
    let registry_dir = config.state_path.parent().unwrap().to_path_buf();
    let server = RunningServer::start(config).unwrap();

    let pointer = socket_pointer_path(&cache_root);
    assert_eq!(
        pointer.parent(),
        Some(registry_dir.as_path()),
        "the pointer lives in the registry directory, keyed by the cache root"
    );
    let raw = fs::read_to_string(&pointer)
        .unwrap_or_else(|e| panic!("the pointer {} must be readable: {e}", pointer.display()));
    assert_eq!(
        raw.trim_end(),
        server.socket_path().to_string_lossy(),
        "the pointer names the socket the accept loop is serving on"
    );
    assert_eq!(
        published_socket_path(&cache_root).as_deref(),
        Some(server.socket_path()),
        "the reader decodes the same path"
    );
    server.shutdown();
}

/// The mirror edge: a graceful shutdown removes the pointer it wrote, so a
/// successor's reader finds nothing stale to dial.
#[test]
fn shutdown_removes_the_pointer() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let cache_root = config.cache_root.clone();
    let server = RunningServer::start(config).unwrap();
    assert!(
        socket_pointer_path(&cache_root).exists(),
        "written at start"
    );
    server.shutdown();
    assert!(
        !socket_pointer_path(&cache_root).exists(),
        "a graceful shutdown must remove the pointer it wrote"
    );
    assert_eq!(published_socket_path(&cache_root), None);
}

/// The reader's whole contract. With nothing published the law answers;
/// with a LIVE daemon published somewhere the law would not derive, the
/// published path answers — and a client built on it is served; after the
/// daemon's clean shutdown the law answers again.
#[test]
fn a_reader_whose_own_derivation_differs_reaches_the_holder() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let cache_root = config.cache_root.clone();
    let derived = socket_path_for_cache_root(&cache_root);
    assert_ne!(
        derived, config.socket_path,
        "the fixture must disagree with the law, or the gate measures nothing"
    );
    assert_eq!(
        reachable_socket_path(&cache_root),
        derived,
        "nothing published: the law answers"
    );

    let server = RunningServer::start(config).unwrap();
    let reachable = reachable_socket_path(&cache_root);
    assert_eq!(
        reachable,
        server.socket_path(),
        "a live daemon published elsewhere: the published path answers"
    );
    let client = Client::new(reachable);
    assert!(
        client.ping().unwrap(),
        "and a client built on that answer is served by the holder"
    );
    server.shutdown();

    assert_eq!(
        reachable_socket_path(&cache_root),
        derived,
        "after a clean shutdown the law answers again"
    );
}

/// A `SIGKILL`ed predecessor removes nothing, so its pointer survives it and
/// names a socket nobody listens on. Presence is not liveness: the reader
/// dials, finds no listener, and answers the law — so the client's own spawn
/// takes the flock and republishes, instead of dialing a corpse forever.
#[test]
fn a_stale_pointer_names_no_listener_so_the_law_answers() {
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join("cache");
    let dead = tmp.path().join("dead.sock");
    let pointer = socket_pointer_path(&cache_root);
    fs::create_dir_all(pointer.parent().unwrap()).unwrap();
    fs::write(&pointer, format!("{}\n", dead.display())).unwrap();

    assert_eq!(
        published_socket_path(&cache_root).as_deref(),
        Some(dead.as_path()),
        "the pointer is read as written"
    );
    assert_eq!(
        reachable_socket_path(&cache_root),
        socket_path_for_cache_root(&cache_root),
        "but no listener answers there, so the law answers"
    );
}
