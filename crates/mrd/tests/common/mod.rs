//! Shared test helpers for CLI-IPC write tests.
//!
//! Each integration-test binary compiles this module separately and uses a
//! different subset of it, so an unused helper here is the normal case, not
//! debt — hence the module-wide allow. (Without it, adding a helper reddens
//! `-D warnings` in every test file that does not happen to call it.)
#![allow(dead_code)]

use std::io::{ErrorKind, Write as _};
use std::path::Path;
use std::process::Child;

/// Feed a spawned child's stdin, tolerating a child that has already stopped
/// reading, then close the pipe so the child sees EOF.
///
/// **`BrokenPipe` here is DATA, not failure.** A CLI that refuses FAST exits
/// before the parent finishes writing; the parent's `write_all` then takes
/// `EPIPE`, and a bare `.expect("write stdin")` panics in the TEST instead of
/// surfacing the child's refusal. The failure is load-dependent — which test
/// dies varies run to run — so it reads as a flaky suite rather than as the
/// harness bug it is. Measured twice: 2026-08-19 on `scoped_guards_client`
/// (one sha, three full-suite runs, three different outcomes; 5/5 green in
/// isolation), and 2026-08-20 as PR #158 pipeline 985 red on
/// `put_dry_and_validate_together_refuse_loudly` while pipeline 984 was green
/// on that same sha.
///
/// This weakens no assertion. A fast refusal is asserted on the child's own
/// output through `wait_with_output` — which is where the refusal always
/// lived; this only stops the parent from panicking first and hiding it.
/// Every io error that is NOT `BrokenPipe` still panics loud, and a child
/// that never refuses still has to produce whatever its test asserts.
pub(crate) fn feed_stdin(child: &mut Child, bytes: &[u8]) {
    let mut pipe = child
        .stdin
        .take()
        .expect("child was spawned with piped stdin");
    if let Err(e) = pipe.write_all(bytes).and_then(|()| pipe.flush())
        && e.kind() != ErrorKind::BrokenPipe
    {
        panic!("feeding child stdin failed with a non-EPIPE error: {e}");
    }
    // Dropping `pipe` closes the child's stdin: a child still reading sees EOF.
}

/// SIGTERM the resident daemon whose pidfile lives under this cache home.
pub(crate) fn reap_daemon(cache_home: &Path) {
    let pidfile = cache_home
        .join("meridian")
        .join("registry")
        .join("daemon.pid");
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
