//! G11: what a quiet daemon may cost — assert on corpus folds, not CPU %.
//!
//! Quiet pre-warm must run ZERO full-corpus folds (`fs::fold_count`). This
//! binary must be the only fold-asserting work in its process (`fold_count` is
//! process-global; assert as a difference).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

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

/// **The G11 gate.** A sweep over an unchanged corpus must fold NOTHING.
///
/// Before the fix this test fails by exactly one fold per sweep per warm
/// workspace — which, once a second against 20 GB, is the whole gap.
#[test]
fn a_quiet_sweep_does_not_fold_the_corpus_at_all() {
    let tmp = tempfile::tempdir().unwrap();
    let server = quiet_server(tmp.path(), None, NEVER);
    let ws = workspace(tmp.path(), &[("a.md", "# A\n"), ("sub/b.md", "# B\n")]);
    let registry = server.registry();

    // Warm it once — this fold is the legitimate one, paid on first use.
    assert!(matches!(
        registry.warm_or_build(&ws).unwrap(),
        WarmOutcome::Built { .. }
    ));

    // The first sweep after a warm still folds once: the sweep has no recorded
    // signature for this workspace yet, so it cannot know the corpus is quiet.
    let _ = registry.prewarm();

    // From here the corpus does not move, and neither may the fold counter —
    // for any number of sweeps.
    let before = fs::metadata(ws.join("a.md")).unwrap().len();
    let folds_before = ::fs::fold_count();
    for _ in 0..20 {
        assert!(
            registry.prewarm().is_empty(),
            "a quiet sweep rebuilds nothing"
        );
    }
    assert_eq!(
        ::fs::fold_count(),
        folds_before,
        "20 quiet sweeps must read the corpus ZERO times — this is G11: the old \
         sweep folded every byte on every tick, and did it once a second forever"
    );
    assert_eq!(
        fs::metadata(ws.join("a.md")).unwrap().len(),
        before,
        "and a sweep must not have touched the corpus"
    );
}
