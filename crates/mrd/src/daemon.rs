//! `mrd daemon` — run the registry server in the foreground until a signal.
//!
//! No auto-spawn or detach in this iteration: warming a tier-4 tree via the
//! daemon requires the user to have started it (or to run `mrd init`, which is
//! marker-based and daemon-independent). SIGINT/SIGTERM shut it down cleanly so
//! the final state file is flushed and the socket removed.

use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use registry::{Config, RunningServer};

use crate::Fail;

/// Set by the signal handler; polled by the foreground loop.
static SIGNALLED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_sig: libc::c_int) {
    SIGNALLED.store(true, Ordering::SeqCst);
}

/// Run `mrd daemon`.
pub(crate) fn run() -> Result<(), Fail> {
    let config = Config::resolve()
        .map_err(|e| Fail::tool(format!("cannot resolve the daemon layout: {e}")))?;
    install_signal_handlers();
    let server = RunningServer::start(config)
        .map_err(|e| Fail::tool(format!("cannot start the registry daemon: {e}")))?;
    eprintln!(
        "meridian registry daemon listening on {}",
        server.socket_path().display()
    );
    eprintln!("press Ctrl-C to stop");

    while !SIGNALLED.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(200));
    }
    eprintln!("shutting down");
    server.shutdown();
    Ok(())
}

fn install_signal_handlers() {
    // SAFETY: `on_signal` only stores to a static AtomicBool — an
    // async-signal-safe operation (no allocation, no lock, no reentrancy).
    let handler = on_signal as *const () as libc::sighandler_t;
    unsafe {
        let _ = libc::signal(libc::SIGINT, handler);
        let _ = libc::signal(libc::SIGTERM, handler);
    }
}
