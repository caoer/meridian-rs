//! Opportunistic last-use auto-GC (Cargo model, spec §5). A path-keyed drawer store without a
//! reaper is the Bazel/VSCode disk-leak class (decision 0001 round 4).
//!
//!
//!
//!
//!
//!
//!
//!

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// The stamp file recording the last auto-GC time (unix seconds).
const GC_STAMP: &str = ".last-gc";

/// Minimum interval between opportunistic sweeps (24h, Cargo's horizon).
const AUTO_GC_INTERVAL_SECS: u64 = 24 * 60 * 60;

/// Run a last-use sweep if at least a day has passed since the last one. Best
/// effort: any error is swallowed so a normal invocation never fails on GC.
pub(crate) fn maybe_auto_gc(cache_root: &Path) {
    if !due(cache_root) {
        return;
    }
    let _ = cache::gc(cache_root, cache::DEFAULT_GC_THRESHOLD);
    stamp(cache_root);
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
