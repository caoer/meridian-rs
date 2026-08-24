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

    // The sweep's quiet gate (`Registry::vouched_quiet`) reads the feed's
    // pending state WITHOUT a barrier, so it cannot tell "nothing moved" from
    // "the event has not been delivered yet" — `FsMemo::served_cached` names
    // that skip LATENCY-ONLY by design (merkle-spec §6.7). Kernel delivery is
    // asynchronous and, on a box sharing itself with N CI pipelines, has no
    // bound, so POLLING for it asserted the BOX rather than the sweep: that is
    // the same-sha split at this line (1197 green / 1198 red on 3f4859513;
    // pipeline 1101), where the deadline was 15s and the tree was identical.
    //
    // `currency_refresh` is the daemon's OWN proof of delivery — the call every
    // door takes. It parks on the feed's cookie barrier, which orders this
    // write's event before its own sighting, and when the barrier does not
    // report `Seen` inside the timeout it falls to a full extent walk. On
    // EITHER path the memo has observed this write by the time it returns, so
    // what the sweep does next is a function of the corpus alone and needs no
    // deadline. It refreshes the MEMO, not the engine — the engine is still
    // stamped at the pre-edit fingerprint, so the rebuild below is still the
    // sweep's own work and the assertion still says what it always said.
    registry
        .currency_refresh(&ws, Duration::from_secs(2))
        .expect("the daemon can observe its own workspace");

    assert_eq!(
        registry.prewarm(),
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
