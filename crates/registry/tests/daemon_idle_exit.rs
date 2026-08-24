//! G11 design gates that do not assert on `fs::fold_count`.
//!
//! Split from `prewarm_quiet.rs`: `fold_count` is process-global, so
//! fold-difference assertions need a dedicated binary.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use registry::{Config, RunningServer, WarmOutcome};

// `Duration::from_days`/`from_mins` are not const-stable at MSRV 1.96, so the
// seconds form is the only option.
#[allow(clippy::duration_suboptimal_units)]
const NEVER: Duration = Duration::from_secs(365 * 24 * 60 * 60);

/// A server whose background threads never fire, so every sweep in these tests
/// is one the test itself called.
fn quiet_server(tmp: &Path, idle_exit: Option<Duration>, reap_interval: Duration) -> RunningServer {
    quiet_server_parked(tmp, idle_exit, reap_interval, None)
}

/// [`quiet_server`] plus the born-parked activity floor
/// ([`Config::activity_park`]).
fn quiet_server_parked(
    tmp: &Path,
    idle_exit: Option<Duration>,
    reap_interval: Duration,
    activity_park: Option<Duration>,
) -> RunningServer {
    let dir = tmp.join("registry");
    fs::create_dir_all(&dir).unwrap();
    let mut config = Config::for_cache_root(tmp.join("cache"));
    config.socket_path = dir.join("daemon.sock");
    config.state_path = dir.join("state.json");
    config.idle_threshold = NEVER;
    config.reap_interval = reap_interval;
    config.prewarm_interval = NEVER;
    config.prewarm_quiet_max = NEVER;
    config.idle_exit = idle_exit;
    config.activity_park = activity_park;
    config.drain_cold_builds = Duration::from_secs(30);
    RunningServer::start(config).unwrap()
}

fn workspace(tmp: &Path, files: &[(&str, &str)]) -> PathBuf {
    let ws = tmp.join("ws");
    for (rel, body) in files {
        let path = ws.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }
    fs::canonicalize(&ws).unwrap()
}

/// A prewarm skip is a skip, not a stop: a real edit still rebuilds on the sweep.
#[test]
fn a_changed_corpus_is_still_rebuilt_on_the_sweep() {
    let tmp = tempfile::tempdir().unwrap();
    let server = quiet_server(tmp.path(), None, NEVER);
    let ws = workspace(tmp.path(), &[("a.md", "# A\n")]);
    let registry = server.registry();

    registry.warm_or_build(&ws).unwrap();
    let _ = registry.prewarm();
    assert!(registry.prewarm().is_empty(), "quiet before the edit");

    fs::write(ws.join("a.md"), "# A changed\n\nnew body\n").unwrap();

    // Kernel delivery is asynchronous: a sweep landing before the event is a
    // lawful skip (latency-only). Poll — the same posture as
    // `prewarm_absorbs_the_change_so_the_next_query_parses_nothing`. Under
    // load this is the class-1 flake at daemon_idle_exit.rs:57 (pipeline 1101).
    let deadline = Instant::now() + Duration::from_secs(15);
    let rebuilt = loop {
        let got = registry.prewarm();
        if !got.is_empty() || Instant::now() > deadline {
            break got;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(
        rebuilt,
        vec![ws.clone()],
        "the edit must rebuild on the sweep, not lazily on the next query"
    );
    assert_eq!(
        registry.warm_or_build(&ws).unwrap(),
        WarmOutcome::Reused,
        "and the next query must then find the engine already warm — zero parse"
    );
}

/// The leak half of G11: a detached daemon is reparented to init and nothing
/// else ever ends it, so it must age out.
#[test]
fn an_idle_daemon_asks_to_exit() {
    let tmp = tempfile::tempdir().unwrap();
    // Horizon zero: every reaper pass is past it.
    let server = quiet_server(tmp.path(), Some(Duration::ZERO), Duration::from_millis(100));

    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && !server.idle_exit_requested() {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        server.idle_exit_requested(),
        "a daemon past its idle horizon must ask its host process to exit"
    );
}

/// The observable half of the born-parked floor, and the exact counterweight to
/// `an_idle_daemon_asks_to_exit` above: **same horizon, same reap interval, one
/// config field different.**
///
/// Horizon ZERO means every reaper pass is past it, so the unparked twin latches
/// on the first pass — that is what the test above measures. Under
/// `activity_park` the flag must still be down after many passes, and the only
/// way that can be true is that the floor was up *before the first pass*, which
/// is before `start()` returned.
///
/// That window is why the field exists. The activity clock starts inside
/// `Registry::new` and the reaper is spawned before `start()` returns, so a
/// fixture parking the handle `start()` hands back has already been reapable for
/// the length of `start()`'s body — with a whole-second clock, ~1.0 s of wall
/// time is enough to latch a 2 s horizon (review
/// `results/review-193-claude-e540dc0b.md` § F1). Nothing outside can sample the
/// clock mid-construction, so this is the strongest outside instrument there is;
/// the clock itself is asserted in
/// `registry::engine_tests::a_config_borne_park_is_up_before_the_constructor_returns`.
///
/// *Mutation:* stop threading `Config::activity_park` into
/// `Registry::new_shared` — the floor is then raised (if at all) after the
/// reaper is already reading the clock, and this goes red on the first pass.
///
/// Card `registry-sweep-poll-flake-instance-1` § F1 full-close.
#[test]
fn a_born_parked_daemon_never_latches_even_at_horizon_zero() {
    let tmp = tempfile::tempdir().unwrap();
    let server = quiet_server_parked(
        tmp.path(),
        Some(Duration::ZERO),
        Duration::from_millis(100),
        Some(NEVER),
    );

    // ~15 reaper passes at REAP_TICK = 200ms. The unparked twin latches on the
    // first one.
    std::thread::sleep(Duration::from_secs(3));
    assert!(
        !server.idle_exit_requested(),
        "a born-parked daemon cannot latch idle-exit, even at horizon zero — if \
         it did, the floor went up after the reaper was already reading the \
         clock, which is the window the config field exists to close"
    );

    // And the born park is still releasable from outside: a floor is a floor
    // whoever raised it, so a fixture can still say "the horizon starts here".
    server.registry().release_activity_park();
    server.registry().note_liveness();
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && !server.idle_exit_requested() {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        server.idle_exit_requested(),
        "releasing a born park restores mortality — a park that could not be \
         released would disable idle-exit rather than delay it"
    );
}

/// The horizon must not fire on a young daemon — one that tripped immediately
/// would turn every client call into a cold start.
#[test]
fn a_daemon_inside_its_horizon_stays() {
    let tmp = tempfile::tempdir().unwrap();
    let server = quiet_server(tmp.path(), Some(NEVER), Duration::from_millis(100));

    std::thread::sleep(Duration::from_millis(600));
    assert!(
        !server.idle_exit_requested(),
        "a daemon inside its idle horizon must stay resident"
    );
}

/// Idle exit is opt-out: `None` means "never", which is what an in-process
/// server embedded in a longer-lived host needs.
#[test]
fn idle_exit_none_never_fires() {
    let tmp = tempfile::tempdir().unwrap();
    let server = quiet_server(tmp.path(), None, Duration::from_millis(100));

    std::thread::sleep(Duration::from_millis(600));
    assert!(
        !server.idle_exit_requested(),
        "idle_exit: None must disable the horizon entirely"
    );
}
