//! G11 design gates that do NOT assert on `fs::fold_count`.
//!
//! Split from `prewarm_quiet.rs`: `fold_count` is process-global, so
//! fold-difference assertions need a dedicated binary.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use registry::{Config, RunningServer, WarmOutcome};

// `Duration::from_days`/`from_mins` are not const-stable at MSRV 1.96, so the
// seconds form is the only option (the crate-wide precedent); silence the lint.
#[allow(clippy::duration_suboptimal_units)]
const NEVER: Duration = Duration::from_secs(365 * 24 * 60 * 60);

/// A server whose background threads never fire, so every sweep in these tests
/// is one the test itself called.
fn quiet_server(tmp: &Path, idle_exit: Option<Duration>, reap_interval: Duration) -> RunningServer {
    let dir = tmp.join("registry");
    fs::create_dir_all(&dir).unwrap();
    RunningServer::start(Config {
        socket_path: dir.join("daemon.sock"),
        state_path: dir.join("state.json"),
        cache_root: tmp.join("cache"),
        idle_threshold: NEVER,
        reap_interval,
        prewarm_interval: NEVER,
        prewarm_quiet_max: NEVER,
        idle_exit,
        push_write_timeout: registry::DEFAULT_PUSH_WRITE_TIMEOUT,
        sub_idle_write_timeout: registry::DEFAULT_SUB_IDLE_WRITE_TIMEOUT,
        // No build identity configured: this fixture is not testing the hello
        // identity field, and an absent sha is the honest state for a server
        // started from a test harness rather than a deployed binary.
        build_sha: None,
    })
    .unwrap()
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

/// The skip is a skip, not a stop: the sweep still catches a real edit, so the
/// latency property pre-warm exists for survives the cheap gate.
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
/// ever ends it, so each isolated run left one behind permanently (measured:
/// exactly 8 per gate run, 281 alive on one host). It must now age out.
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

/// And it must not fire on a daemon that is merely young — an exit horizon that
/// tripped immediately would turn every client call into a cold start.
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
