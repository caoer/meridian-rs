//! The published socket path: the lock holder is always reachable
//! (`server` § The published socket path; `docs/wire-contract.md` § The
//! published socket path).
//!
//! The singleton flock is keyed on the cache root alone; the socket's base is
//! not — `XDG_RUNTIME_DIR` set or unset derives a different path over the
//! same root. Measured 2026-09-02: a client without it spawned the daemon
//! under `$HOME/.cache/mrd-run/`, and every client WITH it dialled an absent
//! `$XDG_RUNTIME_DIR/mrd/<12hex>.sock`, spawned a successor, and was refused
//! "another meridian registry daemon is already running" by a daemon it could
//! not reach. So the daemon publishes the socket it bound in the lock's own
//! directory, and a client whose derived path is absent dials that one.
//!
//! These gates are the pidfile's (`daemon_pidfile.rs`), one file over: written
//! before the first pong, claimed over a predecessor's, removed on a clean
//! shutdown — plus the client half, against a real daemon.

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use registry::{Client, Config, RunningServer, published_socket_path, socket_path_for_cache_root};
use tempfile::TempDir;

/// The cache root — where the lock, the state file, and the publication live.
fn cache_root(tmp: &TempDir) -> PathBuf {
    tmp.path().join("cache")
}

/// A daemon rooted under `tmp` whose socket is bound where NO environment
/// derives it — the lock holder's shape in the incident — with the registry
/// directory at its production place under the cache root, background
/// horizons parked so nothing fires mid-test (the `rpc.rs` precedent).
// `Duration::from_hours` is not const-stable at MSRV 1.96; the seconds form is
// the workspace precedent (cache::DEFAULT_GC_THRESHOLD).
#[allow(clippy::duration_suboptimal_units)]
fn test_config(tmp: &TempDir) -> Config {
    let forever = Duration::from_secs(365 * 24 * 60 * 60);
    let mut config = Config::for_cache_root(cache_root(tmp));
    config.socket_path = tmp.path().join("elsewhere").join("daemon.sock");
    config.idle_threshold = forever;
    config.reap_interval = forever;
    config.prewarm_interval = forever;
    config.prewarm_quiet_max = forever;
    config.idle_exit = None;
    config.drain_cold_builds = Duration::from_secs(30);
    config
}

/// Where the daemon publishes: beside the state file, by the fixed
/// `daemon.sock-path` name every consumer derives.
fn publication(tmp: &TempDir) -> PathBuf {
    cache_root(tmp).join("registry").join("daemon.sock-path")
}

/// The ordering law at its root: when `start` returns the accept loop is live,
/// so the publication must already be there — and it must live in the LOCK's
/// directory, not beside the socket, because the reader is a client whose
/// environment derives a different socket base and can only find the cache
/// root.
#[test]
fn start_returns_with_the_socket_path_published_in_the_lock_directory() {
    let tmp = TempDir::new().unwrap();
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let file = publication(&tmp);
    assert!(
        file.exists(),
        "the publication must exist the moment start returns — the accept loop is already serving"
    );
    assert_eq!(
        fs::read_to_string(&file).unwrap().trim(),
        server.socket_path().to_str().unwrap(),
        "the publication names the bound socket, absolute, one line"
    );
    assert_eq!(
        published_socket_path(&cache_root(&tmp)).as_deref(),
        Some(server.socket_path()),
        "and the reader every client uses decodes it to the same path"
    );
    server.shutdown();
}

/// A `SIGKILL`ed predecessor removes nothing, so its publication survives it
/// — and it may name a socket under the OTHER base. The successor claims the
/// file before its first pong; a client reading it on the strength of a pong
/// must never be sent to the dead daemon's socket.
#[test]
fn a_stale_predecessor_publication_is_claimed_before_serving() {
    let tmp = TempDir::new().unwrap();
    let dir = cache_root(&tmp).join("registry");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("daemon.sock-path"),
        "/run/user/1000/mrd/deadbeefcafe.sock\n",
    )
    .unwrap();

    let server = RunningServer::start(test_config(&tmp)).unwrap();
    assert_eq!(
        published_socket_path(&cache_root(&tmp)).as_deref(),
        Some(server.socket_path()),
        "start must overwrite a crashed predecessor's publication before the accept loop serves"
    );
    server.shutdown();
}

/// The mirror edge: a graceful shutdown removes the publication with the
/// socket it names, so a client that finds none keeps its derived path — the
/// spawn target — rather than dialling a name with nothing behind it.
#[test]
fn shutdown_removes_the_publication() {
    let tmp = TempDir::new().unwrap();
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let file = publication(&tmp);
    assert!(file.exists(), "written at start");
    server.shutdown();
    assert!(
        !file.exists(),
        "a graceful shutdown must remove the publication it wrote"
    );
    assert_eq!(
        published_socket_path(&cache_root(&tmp)),
        None,
        "and the reader answers None, not a stale path"
    );
}

/// The incident, closed at the library door: this process's environment
/// derives a socket nobody bound, the lock holder is bound elsewhere, and the
/// client reaches it through the publication — no spawn, no "already
/// running", a pong.
#[test]
fn a_client_on_a_foreign_base_reaches_the_lock_holder_through_the_publication() {
    let tmp = TempDir::new().unwrap();
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let derived = socket_path_for_cache_root(&cache_root(&tmp));
    assert!(
        !derived.exists(),
        "the fixture's premise: this environment derives a socket nobody bound ({})",
        derived.display()
    );
    assert_ne!(
        derived,
        server.socket_path(),
        "and the daemon is bound somewhere else — the two-bases shape"
    );

    let client = Client::for_cache_root(&cache_root(&tmp));
    assert_eq!(
        client.socket_path(),
        server.socket_path(),
        "an absent derived path yields to the published socket"
    );
    assert!(client.ping().unwrap(), "and the lock holder answers on it");
    server.shutdown();
}

/// A publication is a claim the reader still has to be able to act on: a
/// relative or empty path is no path to dial, and reads as none.
#[test]
fn a_publication_that_is_not_an_absolute_path_reads_as_none() {
    let tmp = TempDir::new().unwrap();
    let dir = cache_root(&tmp).join("registry");
    fs::create_dir_all(&dir).unwrap();
    for bad in ["", "\n", "relative/daemon.sock\n"] {
        fs::write(dir.join("daemon.sock-path"), bad).unwrap();
        assert_eq!(
            published_socket_path(&cache_root(&tmp)),
            None,
            "{bad:?} is not a socket path a client can dial"
        );
    }
}
