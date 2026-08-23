//! End-to-end RPC gates for the workspace registry daemon (decision 0001
//! round 5). Each test starts a real daemon on a temp socket and drives it
//! through the client library.

use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use registry::{Client, Config, Response, RunningServer};
use tempfile::TempDir;

/// A daemon config rooted entirely under `tmp`, with reap horizons large enough
/// that the background reaper never fires mid-test (reap is exercised directly).
// `Duration::from_hours` is not const-stable at MSRV 1.96; the seconds form is
// the workspace precedent (cache::DEFAULT_GC_THRESHOLD).
#[allow(clippy::duration_suboptimal_units)]
fn test_config(tmp: &TempDir) -> Config {
    let forever = Duration::from_secs(365 * 24 * 60 * 60);
    let dir = tmp.path().join("registry");
    let mut config = Config::for_cache_root(tmp.path().join("cache"));
    config.socket_path = dir.join("daemon.sock");
    config.state_path = dir.join("state.json");
    config.idle_threshold = forever;
    config.reap_interval = forever;
    config.prewarm_interval = forever;
    config.prewarm_quiet_max = forever;
    // No idle exit: a daemon that reaped itself mid-assertion would flake.
    config.idle_exit = None;
    config.drain_cold_builds = Duration::from_secs(30);
    config
}

/// Canonical form (the daemon keys and stores canonical paths).
fn canonical(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn mkdirs(path: &Path) {
    fs::create_dir_all(path).unwrap();
}

#[test]
fn ping_answers() {
    let tmp = TempDir::new().unwrap();
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let client = Client::new(server.socket_path().to_path_buf());
    assert!(client.ping().unwrap(), "a running daemon answers ping");
    server.shutdown();
}

/// Two concurrent register+resolve flows for one bare tree, from different
/// cwds inside it, converge on one identity: one entry, one drawer sentinel.
#[test]
fn concurrent_register_converges_on_one_identity() {
    let tmp = TempDir::new().unwrap();
    let tree = tmp.path().join("bare");
    let sub_a = tree.join("a/aa");
    let sub_b = tree.join("b/bb");
    mkdirs(&sub_a);
    mkdirs(&sub_b);
    let want = canonical(&tree);

    let config = test_config(&tmp);
    let cache_root = config.cache_root.clone();
    let server = RunningServer::start(config).unwrap();
    let sock = server.socket_path().to_path_buf();

    let handles: Vec<_> = [(tree.clone(), sub_a), (tree.clone(), sub_b)]
        .into_iter()
        .map(|(root, sub)| {
            let sock = sock.clone();
            thread::spawn(move || {
                let client = Client::new(sock);
                let registered = match client.register(&root).unwrap() {
                    Response::Registered { entry, .. } => entry.workspace,
                    other => panic!("register did not succeed: {other:?}"),
                };
                let adopted = client
                    .resolve(&sub)
                    .unwrap()
                    .expect("resolve from a subdir must adopt the registered tree");
                (registered, adopted.workspace)
            })
        })
        .collect();

    for handle in handles {
        let (registered, adopted) = handle.join().unwrap();
        assert_eq!(registered, want, "register converged on the canonical tree");
        assert_eq!(adopted, want, "resolve adopted the canonical tree");
    }

    let client = Client::new(sock);
    let entries = client.list().unwrap();
    assert_eq!(entries.len(), 1, "exactly one registry entry");
    assert_eq!(entries[0].workspace, want);

    let drawer = cache::drawer_dir(&cache_root, &want);
    assert!(
        matches!(cache::probe(&drawer), cache::Probe::Hit(_)),
        "the daemon wrote one drawer sentinel for the converged identity"
    );

    server.shutdown();
}

/// The deny ceiling is enforced in the daemon: `$HOME` and `/tmp` are refused
/// with a typed reason, and neither is registered.
#[test]
fn daemon_denies_home_and_tmp() {
    let tmp = TempDir::new().unwrap();
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let client = Client::new(server.socket_path().to_path_buf());

    let home = PathBuf::from(std::env::var("HOME").expect("HOME set"));
    match client.register(&home).unwrap() {
        Response::Denied { reason, .. } => {
            assert_eq!(reason, registry::DenyKind::HomeDir, "$HOME → HomeDir");
        }
        other => panic!("register($HOME) was not denied: {other:?}"),
    }

    match client.register(Path::new("/tmp")).unwrap() {
        Response::Denied { reason, .. } => {
            assert_eq!(reason, registry::DenyKind::TempDir, "/tmp → TempDir");
        }
        other => panic!("register(/tmp) was not denied: {other:?}"),
    }

    assert!(
        client.list().unwrap().is_empty(),
        "a denied register creates no entry"
    );
    server.shutdown();
}

/// A corrupt state file on startup: the daemon starts empty and serves
/// normally (cold is never wrong).
#[test]
fn corrupt_state_starts_empty_and_serves() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    fs::create_dir_all(config.state_path.parent().unwrap()).unwrap();
    fs::write(&config.state_path, b"{ this is not: valid json ]]").unwrap();

    let server = RunningServer::start(config).unwrap();
    let client = Client::new(server.socket_path().to_path_buf());
    assert!(
        client.list().unwrap().is_empty(),
        "corrupt state loads as the empty set"
    );

    let ws = tmp.path().join("live");
    mkdirs(&ws);
    assert!(matches!(
        client.register(&ws).unwrap(),
        Response::Registered { .. }
    ));
    assert_eq!(client.list().unwrap().len(), 1, "serves after a cold start");
    server.shutdown();
}

/// Restart: a warm registration survives daemon stop and restart via the state
/// file.
#[test]
fn entry_survives_restart() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("workspace");
    mkdirs(&ws);
    let want = canonical(&ws);
    let config = test_config(&tmp);

    {
        let server = RunningServer::start(config.clone()).unwrap();
        let client = Client::new(server.socket_path().to_path_buf());
        assert!(matches!(
            client.register(&ws).unwrap(),
            Response::Registered { .. }
        ));
        assert_eq!(client.list().unwrap().len(), 1);
        server.shutdown();
    }

    {
        let server = RunningServer::start(config).unwrap();
        let client = Client::new(server.socket_path().to_path_buf());
        let entries = client.list().unwrap();
        assert_eq!(entries.len(), 1, "the warm entry survived restart");
        assert_eq!(entries[0].workspace, want);
        server.shutdown();
    }
}

/// Resolve adopts from a subdirectory of a registered workspace, and misses on
/// an unregistered tree.
#[test]
fn resolve_adopts_subdir_and_misses_unregistered() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("registered");
    let sub = ws.join("nested/deep");
    mkdirs(&sub);
    let unrelated = tmp.path().join("other");
    mkdirs(&unrelated);
    let want = canonical(&ws);

    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let client = Client::new(server.socket_path().to_path_buf());

    client.register(&ws).unwrap();

    let adopted = client
        .resolve(&sub)
        .unwrap()
        .expect("a subdir of a registered workspace adopts it");
    assert_eq!(adopted.workspace, want);

    assert!(
        client.resolve(&unrelated).unwrap().is_none(),
        "an unregistered tree resolves to a miss"
    );
    server.shutdown();
}

/// Idle-reap with an injected clock, under the §6.4 registration-lifetime
/// law: fresh entries survive their horizon; an aged WARM workspace is
/// demoted (engine dropped) while its registration stays listed; a
/// never-warmed registration has nothing to shed and is not reported. Only
/// `unregister` ends a registration.
#[test]
fn idle_reap_demotes_stale_entries_and_keeps_registrations() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("workspace");
    mkdirs(&ws);
    fs::write(ws.join("a.md"), "# A\n").unwrap();
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let client = Client::new(server.socket_path().to_path_buf());
    let five_days = 5 * 24 * 60 * 60;

    client.register(&ws).unwrap();
    assert_eq!(client.list().unwrap().len(), 1);

    // Fresh entry, real clock, 5-day horizon → untouched.
    let survivors = server.registry().reap(now_secs(), five_days);
    assert!(survivors.is_empty(), "a fresh entry is within its horizon");
    assert_eq!(client.list().unwrap().len(), 1);

    // A never-warmed registration holds nothing reapable: an aged clock
    // demotes nothing and the entry stays listed.
    let future = now_secs() + 10 * 365 * 24 * 60 * 60;
    let reaped = server.registry().reap(future, five_days);
    assert!(
        reaped.is_empty(),
        "a cold registration has nothing to shed: {reaped:?}"
    );
    assert_eq!(client.list().unwrap().len(), 1);

    // Warm it, then age it: the engine is demoted, the registration is not.
    server.registry().warm_or_build(&ws).unwrap();
    let reaped = server.registry().reap(future, five_days);
    assert_eq!(reaped.len(), 1, "the aged warm workspace is demoted");
    assert_eq!(
        client.list().unwrap().len(),
        1,
        "an idle-reaped engine keeps its registration (merkle-spec §6.4)"
    );

    // Only unregister ends the registration.
    assert!(client.unregister(&ws).unwrap());
    assert!(client.list().unwrap().is_empty());

    server.shutdown();
}

/// A second daemon on the same registry directory refuses to start (singleton).
#[test]
fn second_daemon_refuses_to_start() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let server = RunningServer::start(config.clone()).unwrap();

    let err = RunningServer::start(config).expect_err("a second daemon must refuse");
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);

    server.shutdown();
}

/// Unregister drops an entry and reports whether one was present.
#[test]
fn unregister_removes_and_reports() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("workspace");
    mkdirs(&ws);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let client = Client::new(server.socket_path().to_path_buf());

    client.register(&ws).unwrap();
    assert!(client.unregister(&ws).unwrap(), "removes a present entry");
    assert!(client.list().unwrap().is_empty());
    assert!(
        !client.unregister(&ws).unwrap(),
        "unregister of an absent path reports false"
    );
    server.shutdown();
}
