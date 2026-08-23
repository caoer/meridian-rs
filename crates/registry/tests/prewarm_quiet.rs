//! G11: quiet pre-warm must run zero full-corpus folds (`fs::fold_count`).
//!
//! This binary must be the only fold-asserting work in its process
//! (`fold_count` is process-global; assert as a difference).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

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

/// A sweep over an unchanged corpus must fold nothing (G11).
#[test]
fn a_quiet_sweep_does_not_fold_the_corpus_at_all() {
    let tmp = tempfile::tempdir().unwrap();
    let server = quiet_server(tmp.path(), None, NEVER);
    let ws = workspace(tmp.path(), &[("a.md", "# A\n"), ("sub/b.md", "# B\n")]);
    let registry = server.registry();

    // The first-use fold is the legitimate one.
    assert!(matches!(
        registry.warm_or_build(&ws).unwrap(),
        WarmOutcome::Built { .. }
    ));

    // §6.7: with a live feed the quiet gate is the feed's own state, so even
    // the first sweep is O(1); on the no-feed fallback it would pay one
    // signature walk here. Either way it folds nothing.
    let _ = registry.prewarm();

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
