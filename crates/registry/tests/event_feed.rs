//! The §6.4 event feed's kernel plumbing (quality gate 3): real watcher,
//! real events, across a real engine reap. The deterministic law tests (the
//! O(dirty) counters, the guard-independence gate, the reap split) live in
//! `registry::engine_tests` and drive the feed through its hint door; THIS
//! file proves the kernel half — `FSEvents` on macOS, inotify on Linux —
//! delivers an edit made while the engine is cold into the dirty set, and
//! that the next warm applies it.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use registry::{Config, Registry, WarmOutcome, in_process_registry};

/// An in-process registry rooted under `tmp` (no socket, no reaper thread).
fn registry_in(tmp: &Path) -> Registry {
    let dir = tmp.join("registry");
    let mut config = Config::for_cache_root(tmp.join("cache"));
    config.socket_path = dir.join("daemon.sock");
    config.state_path = dir.join("state.json");
    in_process_registry(&config).expect("in-process registry")
}

/// A workspace seeded with `files`, sibling of the registry dirs.
fn write_ws(tmp: &Path, files: &[(&str, &str)]) -> PathBuf {
    let ws = tmp.join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    for (rel, content) in files {
        let path = ws.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }
    ws
}

/// Poll `probe` until it answers true, up to `cap`. Kernel event delivery is
/// asynchronous by nature; polling a counter is the deterministic form of
/// waiting for it (no fixed sleeps, generous ceiling for a loaded runner).
fn wait_until(cap: Duration, probe: impl Fn() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < cap {
        if probe() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    probe()
}

/// Gate 3: events across a reap cycle are not lost. The watcher outlives the
/// engine (registration-lifetime law): an edit landing while the engine is
/// cold reaches the dirty set through the KERNEL, and the next warm applies
/// it — the re-derived root equals a from-scratch derivation.
#[test]
fn an_edit_while_the_engine_is_cold_survives_into_the_next_warm() {
    let tmp = tempfile::tempdir().unwrap();
    let reg = registry_in(tmp.path());
    let ws = write_ws(
        tmp.path(),
        &[("plan.md", "# Plan\n"), ("notes.md", "# N\n")],
    );
    let canonical = workspace::canonicalize(&ws).unwrap();
    reg.register(&canonical);

    assert_eq!(
        reg.warm_or_build(&ws).unwrap(),
        WarmOutcome::Built { docs: 2 },
        "first warm builds and starts the feed"
    );
    let stats = reg.feed_stats(&canonical).expect("feed is live after warm");
    let events_before = stats.events;

    // The engine goes cold; the watcher stays (kimi D1).
    assert!(reg.reap(u64::MAX, 0).contains(&canonical));
    assert!(
        reg.feed_stats(&canonical).is_some(),
        "an idle-reaped engine keeps its watcher"
    );

    // An external edit lands while nothing is warm.
    std::fs::write(ws.join("plan.md"), "# Plan\n\nedited while cold\n").unwrap();
    assert!(
        wait_until(Duration::from_secs(10), || {
            let s = reg.feed_stats(&canonical).expect("feed stays live");
            s.events > events_before || s.all_dirty
        }),
        "the kernel delivered the cold-gap edit into the dirty set: {:?}",
        reg.feed_stats(&canonical)
    );

    // The next warm applies the dirty set; the served state is the disk truth.
    let root = reg.currency_root(&canonical).unwrap();
    let scratch_tmp = tempfile::tempdir().unwrap();
    let scratch = registry_in(scratch_tmp.path());
    assert_eq!(
        root,
        scratch.currency_root(&canonical).unwrap(),
        "the post-gap root equals a from-scratch derivation of the same disk"
    );
}

/// The kernel plumbing at its smallest: a warm workspace's edit reaches the
/// pending set without any hint-door help.
#[test]
fn a_kernel_event_reaches_the_pending_set() {
    let tmp = tempfile::tempdir().unwrap();
    let reg = registry_in(tmp.path());
    let ws = write_ws(tmp.path(), &[("a.md", "# A\n")]);
    let canonical = workspace::canonicalize(&ws).unwrap();
    reg.warm_or_build(&ws).unwrap();
    let baseline = reg.feed_stats(&canonical).expect("live feed").events;

    std::fs::write(ws.join("a.md"), "# A\n\nmoved\n").unwrap();
    assert!(
        wait_until(Duration::from_secs(10), || {
            let s = reg.feed_stats(&canonical).expect("live feed");
            s.events > baseline || s.all_dirty
        }),
        "the edit reached the feed: {:?}",
        reg.feed_stats(&canonical)
    );
}
