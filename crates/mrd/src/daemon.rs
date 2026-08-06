//! `mrd daemon` — run the registry+engine server, and the client-side auto-spawn that starts it
//! detached on first use (decision 0002 §3, the watchman model). Two entry points share this
//! module: - [`run`] is the daemon body: bind the socket, write a pidfile, and block on a
//! signal loop until SIGINT/SIGTERM.
//!
//!
//!
//!
//!
//!
//!
//!
//!

use std::io;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use registry::{Config, RunningServer};

use crate::Fail;

/// Set by the signal handler; polled by the foreground loop.
static SIGNALLED: AtomicBool = AtomicBool::new(false);

/// The pidfile name, beside the socket in the registry directory. Written by the
/// singleton-winning daemon so an operator (or a test) can signal the resident daemon without
/// hunting the process table; removed on graceful shutdown.
const PID_NAME: &str = "daemon.pid";

/// Environment override for the binary [`spawn_detached`] launches as the daemon (default: the
/// running executable). A packager points it at a dedicated daemon binary; a test points it at
/// a nonexistent path to force the spawn-impossible degrade.
///
const DAEMON_BIN_ENV: &str = "MERIDIAN_DAEMON_BIN";

extern "C" fn on_signal(_sig: libc::c_int) {
    SIGNALLED.store(true, Ordering::SeqCst);
}

/// Install the SIGINT/SIGTERM handler that flips [`SIGNALLED`] so the foreground
/// loop tears the daemon down cleanly.
fn install_signal_handlers() {
    // SAFETY: `on_signal` only stores to a static AtomicBool — an
    // async-signal-safe operation (no allocation, no lock, no reentrancy).
    let handler = on_signal as *const () as libc::sighandler_t;
    unsafe {
        let _ = libc::signal(libc::SIGINT, handler);
        let _ = libc::signal(libc::SIGTERM, handler);
    }
}

/// Run `mrd daemon`: bind the socket, write the pidfile, and block until a
/// signal, then shut down cleanly (flush state, remove the socket + pidfile).
pub(crate) fn run() -> Result<(), Fail> {
    let mut config = Config::resolve()
        .map_err(|e| Fail::tool(format!("cannot resolve the daemon layout: {e}")))?;
    // R1: this crate is the only one that can read the baked sha
    // (`build.rs` bakes it into THIS crate's compilation environment), so the
    // daemon host hands it to the registry rather than the registry reading it.
    // The `unknown` fallback the build script bakes rides through verbatim — a
    // build that cannot name a commit still publishes an identity, which is
    // what lets a client tell it apart from a host that publishes none
    // (`docs/wire-contract.md`).
    config.build_sha = Some(env!("MRD_BUILD_SHA").to_owned());
    install_signal_handlers();
    let server = RunningServer::start(config)
        .map_err(|e| Fail::tool(format!("cannot start the registry daemon: {e}")))?;
    let pid_path = pid_path(server.socket_path());
    // Advisory only — a failed pidfile write must not fail the daemon (the
    // socket is the real liveness handle); log and carry on.
    if let Err(e) = std::fs::write(&pid_path, format!("{}\n", std::process::id())) {
        eprintln!(
            "registry: cannot write pidfile {} ({e})",
            pid_path.display()
        );
    }
    eprintln!(
        "meridian registry daemon listening on {}",
        server.socket_path().display()
    );
    eprintln!("press Ctrl-C to stop");

    // G11: two ways out — a signal, or the idle-exit horizon the reaper watches. A detached daemon
    // is reparented to init and nothing else will ever end it, so without the second condition
    // every isolated run leaves one behind forever. Teardown is identical either way; only the
    // reason differs.
    while !SIGNALLED.load(Ordering::SeqCst) && !server.idle_exit_requested() {
        thread::sleep(Duration::from_millis(200));
    }
    eprintln!("shutting down");
    let _ = std::fs::remove_file(&pid_path);
    server.shutdown();
    Ok(())
}

/// The pidfile path for a daemon whose socket is `socket_path` (its sibling in
/// the registry directory).
fn pid_path(socket_path: &Path) -> PathBuf {
    socket_path.with_file_name(PID_NAME)
}

/// Auto-spawn the resident daemon DETACHED (decision 0002 §3): launch `mrd daemon` in a new
/// session with null stdio, so it survives this clients exit. Returns as soon as the child is
/// launched — the caller polls the socket for readiness (a launched daemon that never binds is
/// a caller-side timeout, then a degrade).
///
///
///
///
///
///
///
///
///
///
pub(crate) fn spawn_detached() -> io::Result<()> {
    let bin = match std::env::var_os(DAEMON_BIN_ENV) {
        Some(path) => PathBuf::from(path),
        None => std::env::current_exe()?,
    };
    let mut command = Command::new(bin);
    command
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: `setsid(2)` is async-signal-safe (a single syscall, no allocation,
    // no lock) — the only work done in the forked child before exec.
    unsafe {
        command.pre_exec(|| {
            // A new session detaches the daemon from the client's controlling
            // terminal and process group; the leader has no controlling tty.
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    // Launch and detach: the child is reparented to init once this client exits,
    // so dropping the handle leaks no zombie in practice.
    command.spawn().map(drop)
}
