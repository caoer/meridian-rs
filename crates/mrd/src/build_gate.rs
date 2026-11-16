//! The in-process build gate: one whole-corpus build at a time per cache root
//! (`docs/status.md` § The in-process build gate).
//!
//! A verb that builds a whole corpus in its own process holds the root's
//! bytes and every parsed document until it exits, and nothing about the
//! caller bounds how many such verbs run together: a fleet of agents on one
//! host issues them in the same second. Each caller alone is correct; together
//! they hold one corpus apiece and push the host into swap, which slows every
//! one of them and the daemon beside them.
//!
//! The gate is an exclusive `flock` on a directory beside the daemon's socket,
//! keyed on the cache root exactly as the socket is, taken before the first
//! whole-corpus read and held until the process exits — the memory it guards
//! is held that long. The kernel releases it when the process ends however it
//! ends, so a killed holder strands no waiter.

use std::path::PathBuf;
use std::sync::OnceLock;

/// The hold, taken once per process. `None` inside: no gate applies to this
/// process (see [`enter`]).
static HELD: OnceLock<Option<cache::DrawerLock>> = OnceLock::new();

/// Take the gate before a whole-corpus build in this process, waiting for
/// another process's build to finish first. Idempotent: the first call takes
/// the hold and every later call returns at once, so a verb that builds its
/// workspace and then its mounted roots queues once.
///
/// Never an error. A process whose cache root does not resolve, or whose gate
/// directory cannot be made or locked, builds ungated: the gate bounds memory
/// across callers, and a caller that cannot reach it is still owed its answer.
pub(crate) fn enter() {
    HELD.get_or_init(acquire);
}

/// The gate directory: the daemon socket's own stem with a `build` extension,
/// so one cache root has one socket, one daemon and one in-process build.
fn gate_dir() -> Option<PathBuf> {
    let client = registry::Client::from_default().ok()?;
    Some(client.socket_path().with_extension("build"))
}

fn acquire() -> Option<cache::DrawerLock> {
    let dir = gate_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    if let Some(lock) = cache::DrawerLock::try_acquire(&dir).ok()? {
        return Some(lock);
    }
    eprintln!(
        "mrd: another mrd is building a corpus in its own process; waiting for it — one \
         in-process build runs at a time per cache root ({}).",
        dir.display()
    );
    let waiting = timing::phase("build.gate");
    let lock = cache::DrawerLock::acquire(&dir).ok();
    waiting.stop();
    lock
}
