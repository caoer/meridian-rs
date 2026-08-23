//! `mrd daemon` — run the registry+engine server, and the client-side auto-spawn that starts it
//! detached on first use (decision 0002 §3, the watchman model). Two entry points share this
//! module: - [`run`] is the daemon body: start the server (which binds the socket and writes
//! the pidfile, in that contract's order — `registry::RunningServer`), and block on a signal
//! loop until SIGINT/SIGTERM.

use std::fs::OpenOptions;
use std::io;
use std::os::unix::fs::PermissionsExt;
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

/// Environment override for the binary [`spawn_detached`] launches as the daemon (default: the
/// running executable). A packager points it at a dedicated daemon binary; a test points it at
/// a nonexistent path to force the spawn-impossible degrade.
const DAEMON_BIN_ENV: &str = "MERIDIAN_DAEMON_BIN";

/// The extension of the detached daemon's voice file, beside the socket and the
/// pidfile that already key off the same stem ([`voice_path`]).
const VOICE_EXTENSION: &str = "log";

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

/// Run `mrd daemon`: start the server (socket bound, pidfile written before
/// the first request is served — the server owns that order and its mirror on
/// shutdown), and block until a signal.
pub(crate) fn run() -> Result<(), Fail> {
    let mut config = Config::resolve()
        .map_err(|e| Fail::tool(format!("cannot resolve the daemon layout: {e}")))?;
    // Only this crate can read the baked sha (`build.rs` bakes it into this crate's
    // compilation environment), so the host hands it to the registry. The `unknown`
    // fallback rides through verbatim: publishing an identity, even an unnamed one,
    // is distinguishable from publishing none (`docs/wire-contract.md`).
    config.build_sha = Some(env!("MRD_BUILD_SHA").to_owned());
    install_signal_handlers();
    let server = RunningServer::start(config)
        .map_err(|e| Fail::tool(format!("cannot start the registry daemon: {e}")))?;
    eprintln!(
        "meridian registry daemon listening on {}",
        server.socket_path().display()
    );
    eprintln!("press Ctrl-C to stop");

    // Two ways out — a signal, or the idle-exit horizon the reaper watches. A detached daemon is
    // reparented to init, so without the second condition every isolated run leaks one forever.
    while !SIGNALLED.load(Ordering::SeqCst) && !server.idle_exit_requested() {
        thread::sleep(Duration::from_millis(200));
    }
    eprintln!("shutting down");
    server.shutdown();
    Ok(())
}

/// Auto-spawn the resident daemon DETACHED (decision 0002 §3): launch `mrd daemon` in a new
/// session with null stdin/stdout, so it survives this clients exit. Its stderr is null too
/// unless the timing mode is on, in which case it gets a file to speak into ([`voice`]).
/// Returns as soon as the child is launched — the caller polls the socket for readiness (a
/// launched daemon that never binds is a caller-side timeout, then a degrade).
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
        .stderr(voice());
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

/// Where a detached daemon speaks: beside the socket, `<socket-stem>.log` — the
/// same per-cache-root keying the pidfile already uses (`<socket-stem>.pid`), so
/// client and daemon derive it from one function and always agree.
///
/// # Errors
///
/// Returns [`io::ErrorKind::NotFound`] when no cache root resolves — the same
/// condition that leaves the client with no socket to dial.
fn voice_path() -> io::Result<PathBuf> {
    Ok(registry::default_socket_path()?.with_extension(VOICE_EXTENSION))
}

/// The socket directory, created if this client got there before any daemon.
///
/// It is the daemon's own directory and the daemon wants it 0700
/// (`registry::server` § `prepare_dir`), so a client that creates it first must
/// not leave it looser than the daemon would. An existing directory is left
/// exactly as its owner set it.
fn prepare_voice_dir(dir: &Path) -> io::Result<()> {
    if dir.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    Ok(())
}

/// The child's stderr.
///
/// A detached daemon has no terminal, so this was `Stdio::null()` unconditionally
/// — and that null was the whole reason the timing mode is deaf on the daemon
/// lane. Both file-sink refusals degrade to stderr, and the `MRD_TIMING=1` form
/// IS stderr (`crates/timing`), so an operator who pointed a daemon at a corpus
/// extension or an unopenable path got no complaint AND no measurements: exactly
/// the "the code never ran there" answer the instrument must never fake.
///
/// So while the mode is ON the daemon gets a real voice — [`voice_path`], opened
/// append — and every `mrd-timing:` diagnostic, every degraded measurement, and
/// the registry's own startup and error lines land in it. **Off, nothing changes
/// and no file is created**: the gate is the operator's own switch, so this
/// costs a run that did not ask for the mode nothing at all.
///
/// The file grows for as long as the mode is on and is nobody's to rotate — it
/// is the operator's to read and remove.
///
/// A voice that cannot be opened is said out loud HERE, on the spawning client's
/// stderr, which is a lane somebody hears: the daemon then starts mute, and the
/// diagnostic says so rather than letting the operator read silence as "the
/// daemon measured nothing".
fn voice() -> Stdio {
    if !timing::on() {
        return Stdio::null();
    }
    let path = match voice_path() {
        Ok(path) => path,
        Err(error) => {
            timing::diagnostic(&format!(
                "no cache root resolves ({error}), so the daemon has nowhere to speak — it \
                 starts with no stderr and its measurements are LOST."
            ));
            return Stdio::null();
        }
    };
    let opened = match path.parent() {
        Some(dir) => prepare_voice_dir(dir),
        None => Ok(()),
    }
    .and_then(|()| OpenOptions::new().create(true).append(true).open(&path));
    match opened {
        Ok(file) => Stdio::from(file),
        Err(error) => {
            timing::diagnostic(&format!(
                "cannot open the daemon's lane `{}` ({error}) — it starts with no stderr, so \
                 its measurements and any refusal of the sink you named are LOST.",
                path.display()
            ));
            Stdio::null()
        }
    }
}
