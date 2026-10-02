//! Opportunistic auto-GC (Cargo model, spec §5), run from `init`, `unregister` and the daemon's
//! sweeper. A path-keyed drawer store without a reaper leaks disk without bound.

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// The stamp file recording the last auto-GC time (unix seconds).
const GC_STAMP: &str = ".last-gc";

/// Minimum interval between opportunistic sweeps (24h, Cargo's horizon).
const AUTO_GC_INTERVAL_SECS: u64 = 24 * 60 * 60;

/// Run the automatic sweep ([`cache::gc`]: past last-use, or workspace observed
/// gone) if at least a day has passed since the last one. The stamp is taken
/// first, so the daemon's sweeper and an `init` arriving during a long sweep do
/// not both start one. Best effort: any error is swallowed so a normal
/// invocation never fails on GC.
pub(crate) fn maybe_auto_gc(cache_root: &Path) {
    if !due(cache_root) {
        return;
    }
    stamp(cache_root);
    let _ = cache::gc(cache_root, cache::DEFAULT_GC_THRESHOLD);
}

/// Whether a sweep is due: no readable stamp, an unparseable one, or a stamp at
/// least [`AUTO_GC_INTERVAL_SECS`] old all mean "due".
fn due(cache_root: &Path) -> bool {
    let Ok(contents) = fs::read_to_string(cache_root.join(GC_STAMP)) else {
        return true;
    };
    let Ok(last) = contents.trim().parse::<u64>() else {
        return true;
    };
    now_secs().saturating_sub(last) >= AUTO_GC_INTERVAL_SECS
}

/// Record the current time as the last-GC stamp. Best-effort.
fn stamp(cache_root: &Path) {
    if fs::create_dir_all(cache_root).is_ok() {
        let _ = fs::write(cache_root.join(GC_STAMP), now_secs().to_string());
    }
}

/// Current unix time in whole seconds; `0` if the clock predates the epoch.
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_gc_removes_drawer_of_gone_workspace_and_stamps() {
        let cache_root = tempfile::tempdir().unwrap();
        let wss = tempfile::tempdir().unwrap();
        let live_ws = wss.path().join("live");
        let gone_ws = wss.path().join("gone");
        fs::create_dir_all(&live_ws).unwrap();
        fs::create_dir_all(&gone_ws).unwrap();
        let live = cache::drawer_dir(cache_root.path(), &live_ws);
        let gone = cache::drawer_dir(cache_root.path(), &gone_ws);
        cache::register(&live, &live_ws).unwrap();
        cache::register(&gone, &gone_ws).unwrap();
        fs::remove_dir(&gone_ws).unwrap();

        maybe_auto_gc(cache_root.path());

        assert!(!gone.exists(), "gone workspace's drawer swept");
        assert!(live.exists(), "live workspace's drawer kept");
        assert!(!due(cache_root.path()), "the sweep stamped its day");
    }
}
