//! Pidfile ordering gates: the pidfile's validity interval must contain the
//! socket's serving interval.
//!
//! Three consumers treat `daemon.pid` as the kill handle for the SERVING
//! daemon and read it on the strength of a pong: ccc-statusd's
//! `attestKillTarget` (its remediation ladder), `just install`'s 0025 restart
//! duty (`pkill -TERM -F …/daemon.pid mrd`), and the skew-refusal remedy text
//! a human follows. A daemon that answers its first ping before the pidfile
//! names it hands all three an empty — or worse, a crashed predecessor's
//! stale — pid at exactly the moment they are licensed to act on it.
//!
//! So the write is ordered inside [`RunningServer::start`]: after the
//! singleton flock (only the winner may claim the file), before the accept
//! loop spawns (no pong can precede it). Removal mirrors it in shutdown,
//! after the accept thread joins. The write stays advisory — a daemon that
//! cannot write its pidfile still serves — which these gates respect by
//! asserting order, never write-failure behavior.

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use registry::{Client, Config, RunningServer};
use tempfile::TempDir;

/// A daemon config rooted entirely under `tmp`, background horizons parked so
/// nothing fires mid-test (the `rpc.rs` precedent).
// `Duration::from_hours` is not const-stable at MSRV 1.96; the seconds form is
// the workspace precedent (cache::DEFAULT_GC_THRESHOLD).
#[allow(clippy::duration_suboptimal_units)]
fn test_config(tmp: &TempDir) -> Config {
    let forever = Duration::from_secs(365 * 24 * 60 * 60);
    let dir = tmp.path().join("registry");
    let mut config = Config::for_cache_root(tmp.path().join("cache"));
    config.socket_path = dir.join("daemon.sock");
    config.state_path = dir.join("state.json");
    config.idle_threshold = forever;
    config.reap_interval = forever;
    config.prewarm_interval = forever;
    config.prewarm_quiet_max = forever;
    config.idle_exit = None;
    config
}

/// The pidfile lives beside the socket, by the fixed `daemon.pid` name every
/// consumer derives.
fn pid_path(server: &RunningServer) -> PathBuf {
    server.socket_path().with_file_name("daemon.pid")
}

fn read_pid(path: &PathBuf) -> u32 {
    let raw = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("the pidfile {} must be readable: {e}", path.display()));
    raw.trim()
        .parse()
        .unwrap_or_else(|e| panic!("the pidfile must name a pid, got {raw:?}: {e}"))
}

/// The ordering law at its root: when `start` returns, the accept loop is
/// live, so the pidfile must already be written — a pidfile the host process
/// writes afterwards leaves a window in which the daemon serves anonymously.
#[test]
fn start_returns_with_the_pidfile_already_written() {
    let tmp = TempDir::new().unwrap();
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let pidfile = pid_path(&server);
    assert!(
        pidfile.exists(),
        "the pidfile must exist the moment start returns — the accept loop is already serving"
    );
    assert_eq!(
        read_pid(&pidfile),
        std::process::id(),
        "the pidfile must name the process that owns the accept loop"
    );
    server.shutdown();
}

/// The consumer contract verbatim: a client that has its first pong in hand
/// reads a pidfile naming the daemon that served it. This is the exact
/// attest-at-first-pong sequence ccc-statusd's remediation ladder runs.
#[test]
fn a_pong_proves_the_pidfile() {
    let tmp = TempDir::new().unwrap();
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let client = Client::new(server.socket_path().to_path_buf());
    assert!(client.ping().unwrap(), "a running daemon answers ping");
    assert_eq!(
        read_pid(&pid_path(&server)),
        std::process::id(),
        "a served pong proves the pidfile: it must already name the serving process"
    );
    server.shutdown();
}

/// A `SIGKILL`ed predecessor removes nothing, so its pidfile survives it. The
/// successor must claim the file before its first pong — a late write leaves
/// the window in which the socket answers while the file names a dead
/// (recyclable) pid, the one shape no reader can detect: the file parses, the
/// pid is simply wrong.
#[test]
fn a_stale_predecessor_pidfile_is_claimed_before_serving() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let dir = config.socket_path.parent().unwrap().to_path_buf();
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("daemon.pid"), "99999\n").unwrap();

    let server = RunningServer::start(config).unwrap();
    assert_eq!(
        read_pid(&pid_path(&server)),
        std::process::id(),
        "start must overwrite a crashed predecessor's pidfile before the accept loop serves"
    );
    server.shutdown();
}

/// The mirror edge: a graceful shutdown removes the pidfile, and only after
/// the accept thread has joined — the kill handle outlives the last pong, so
/// no reader holding a fresh pong finds the file already gone.
#[test]
fn shutdown_removes_the_pidfile() {
    let tmp = TempDir::new().unwrap();
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let pidfile = pid_path(&server);
    assert!(pidfile.exists(), "written at start");
    server.shutdown();
    assert!(
        !pidfile.exists(),
        "a graceful shutdown must remove the pidfile it wrote"
    );
}
