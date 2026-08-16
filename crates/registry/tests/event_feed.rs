//! The §6.4 event feed's kernel plumbing (quality gate 3): real watcher,
//! real events, across a real engine reap. The deterministic law tests (the
//! O(dirty) counters, the guard-independence gate, the reap split) live in
//! `registry::engine_tests` and drive the feed through its hint door; THIS
//! file proves the kernel half — `FSEvents` on macOS, inotify on Linux —
//! delivers an edit made while the engine is cold into the dirty set, and
//! that the next warm applies it.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use registry::{Config, Registry, RescanCause, WarmOutcome, in_process_registry};

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

    // The next warm applies the dirty set; the served state is the disk
    // truth (compared via the public warm door — the warm's currency pass
    // runs through the same drain).
    reg.warm_or_build(&ws).unwrap();
    let root = reg
        .engine_snapshot(&canonical)
        .expect("engine warm again")
        .at_fingerprint
        .clone();
    let scratch_tmp = tempfile::tempdir().unwrap();
    let scratch = registry_in(scratch_tmp.path());
    scratch.warm_or_build(&ws).unwrap();
    let fresh = scratch
        .engine_snapshot(&canonical)
        .expect("scratch engine warm")
        .at_fingerprint
        .clone();
    assert_eq!(
        root, fresh,
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

/// Chaos fixture (card gate): overflow injection and a labeled instance
/// change each land in the rescan record under their named cause; unnamed
/// rescans are unconstructible. The watcher is NOT restarted across either
/// rescan — a later kernel edit still reaches the pending set.
#[test]
fn chaos_rescans_carry_named_causes_and_the_watcher_stays_up() {
    let tmp = tempfile::tempdir().unwrap();
    let reg = registry_in(tmp.path());
    let ws = write_ws(
        tmp.path(),
        &[("plan.md", "# Plan\n"), ("notes.md", "# N\n")],
    );
    let canonical = workspace::canonicalize(&ws).unwrap();
    reg.warm_or_build(&ws).unwrap();

    // Overflow injection (the kernel's need_rescan / cap-breach class).
    assert!(reg.rescan(&canonical, RescanCause::Overflow));
    // Labeled instance change — the cause the chaos "watcher restart"
    // scenario must carry; the watcher handle itself is not replaced.
    assert!(reg.rescan(&canonical, RescanCause::InstanceChange));
    // The rest of the suspicious-only set, so the record names every cause.
    assert!(reg.rescan(&canonical, RescanCause::MissedEvent));
    assert!(reg.rescan(&canonical, RescanCause::VouchFailure));
    assert!(reg.rescan(&canonical, RescanCause::CookieTimeout));

    let record = reg.rescan_record(&canonical).expect("live feed");
    assert_eq!(
        record,
        [
            RescanCause::Overflow,
            RescanCause::InstanceChange,
            RescanCause::MissedEvent,
            RescanCause::VouchFailure,
            RescanCause::CookieTimeout,
        ],
        "every rescan in the chaos fixture carries its named cause"
    );
    for cause in &record {
        assert!(!cause.name().is_empty(), "unnamed rescans fail the test");
    }
    let stats = reg.feed_stats(&canonical).expect("live feed");
    assert_eq!(
        stats.rescans, stats.overflows,
        "an anonymous collapse would desync the two counters"
    );
    assert_eq!(stats.rescans, 5);

    // Drain the open instance-change (highest rung) so the next kernel
    // event is not swallowed by an all-dirty take.
    let _ = reg.warm_or_build(&ws).unwrap();

    let baseline = reg.feed_stats(&canonical).expect("live feed").events;
    std::fs::write(ws.join("plan.md"), "# Plan\n\nafter rescans\n").unwrap();
    assert!(
        wait_until(Duration::from_secs(10), || {
            let s = reg.feed_stats(&canonical).expect("live feed");
            s.events > baseline || s.all_dirty
        }),
        "the watcher survived both rescans: {:?}",
        reg.feed_stats(&canonical)
    );
}

/// §7(d) / codex gate 10 shape, hermetic: after baseline, a quiet workspace
/// advances neither sweeps nor member_stats. There is no timer on the
/// ladder — a live watcher sitting idle does not schedule work. The live
/// ten-minute / 100k-member bar is the acceptance run of these same
/// counters ([`ten_quiet_minutes_moves_no_sweep_counters`]).
#[test]
fn a_quiet_workspace_does_not_sweep() {
    let tmp = tempfile::tempdir().unwrap();
    let reg = registry_in(tmp.path());
    let ws = write_ws(tmp.path(), &[("a.md", "# A\n"), ("b.md", "# B\n")]);
    let canonical = workspace::canonicalize(&ws).unwrap();
    reg.warm_or_build(&ws).unwrap();
    // Drain any start-up dirt so the quiet window starts from a clean
    // baseline observation.
    let _ = reg.warm_or_build(&ws).unwrap();
    let (sweeps0, stats0) = {
        let cache = reg.domain_cache(&canonical);
        let g = cache.lock().unwrap();
        (g.sweeps(), g.member_stats())
    };
    assert!(sweeps0 >= 1, "baseline observed the corpus");
    assert!(stats0 >= 2, "baseline statted the members");

    std::thread::sleep(Duration::from_millis(1500));

    let (sweeps1, stats1) = {
        let cache = reg.domain_cache(&canonical);
        let g = cache.lock().unwrap();
        (g.sweeps(), g.member_stats())
    };
    assert_eq!(
        (sweeps1, stats1),
        (sweeps0, stats0),
        "a quiet live feed schedules no corpus sweep and no member stat"
    );
    let feed = reg.feed_stats(&canonical).expect("feed stays live");
    assert!(!feed.all_dirty, "quiet did not collapse the set: {feed:?}");
}

/// Live §7(d) bar: ten quiet minutes, zero extra sweeps, zero extra member
/// stats. Ignored in CI — run on the acceptance host against a 100k
/// fixture; the hermetic proof is [`a_quiet_workspace_does_not_sweep`].
#[test]
#[ignore = "live §7(d) 10-minute bar — run on acceptance, not in CI"]
fn ten_quiet_minutes_moves_no_sweep_counters() {
    let tmp = tempfile::tempdir().unwrap();
    let reg = registry_in(tmp.path());
    let ws = write_ws(tmp.path(), &[("a.md", "# A\n")]);
    let canonical = workspace::canonicalize(&ws).unwrap();
    reg.warm_or_build(&ws).unwrap();
    let _ = reg.warm_or_build(&ws).unwrap();
    let (sweeps0, stats0) = {
        let cache = reg.domain_cache(&canonical);
        let g = cache.lock().unwrap();
        (g.sweeps(), g.member_stats())
    };
    std::thread::sleep(Duration::from_secs(10 * 60));
    let (sweeps1, stats1) = {
        let cache = reg.domain_cache(&canonical);
        let g = cache.lock().unwrap();
        (g.sweeps(), g.member_stats())
    };
    assert_eq!(
        (sweeps1, stats1),
        (sweeps0, stats0),
        "ten quiet minutes = 0 corpus sweeps, 0 member stats after baseline"
    );
}
