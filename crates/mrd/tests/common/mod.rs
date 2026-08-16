//! Shared test helpers for CLI-IPC write tests.

use std::path::Path;

/// SIGTERM the resident daemon whose pidfile lives under this cache home.
pub(crate) fn reap_daemon(cache_home: &Path) {
    let pidfile = cache_home.join("registry").join("daemon.pid");
    let Ok(text) = std::fs::read_to_string(pidfile) else {
        return;
    };
    let Ok(pid) = text.trim().parse::<i32>() else {
        return;
    };
    // SAFETY: pid came from this sandbox's own pidfile.
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }
}
