//! Recovery and lifecycle gates for the disposable document cache.
use super::*;
use std::fs as disk;

#[test]
fn server_shutdown_flushes_the_latest_world_and_restart_parses_zero() {
    fn config(base: &Path) -> crate::Config {
        let mut config = crate::Config::for_cache_root(base.join("cache"));
        config.socket_path = base.join("server.sock");
        config.drain_cold_builds = Duration::from_secs(30);
        config
    }
    let fixture = crate::test_support::TestServer::start(config).unwrap();
    let ws = fixture.path().join("ws");
    disk::create_dir_all(&ws).unwrap();
    disk::write(ws.join("a.md"), "# First\n").unwrap();
    let ws = workspace::canonicalize(&ws).unwrap();
    fixture.with(|server| {
        server.registry().register(&ws);
        server.registry().warm_or_build(&ws).unwrap();
        server.registry().save_checkpoints();
    });
    let cache = fixture.path().join("cache");
    let file = crate::parsed_cache::path(&cache, &ws);
    let started = Instant::now();
    while !file.is_file() && started.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(file.is_file(), "first baseline saved");
    disk::write(ws.join("a.md"), "# Latest\n").unwrap();
    fixture.with(|server| {
        server.registry().warm_or_build(&ws).unwrap();
    });
    fixture.stop(); // retain the fixture tree across a real server stop
    fixture.ensure_live(config).unwrap();
    fixture.with(|server| {
        assert_eq!(
            server.registry().warm_or_build(&ws).unwrap(),
            WarmOutcome::Built { docs: 0 }
        );
        assert_eq!(
            server.registry().engine_snapshot(&ws).unwrap().docs["a.md"].raw,
            "# Latest\n"
        );
    });
}

#[test]
fn corrupt_snapshot_is_healed_even_after_an_earlier_save_in_this_process() {
    let home = tempfile::tempdir().unwrap();
    let (reg, ws) = fixture(home.path());
    reg.save_checkpoints();
    let file = crate::parsed_cache::path(&reg.cache_root, &ws);
    disk::write(&file, b"corrupt header").unwrap();
    reg.reap_to_budget(1);
    assert_eq!(
        reg.warm_or_build(&ws).unwrap(),
        WarmOutcome::Built { docs: 2 }
    );
    reg.save_pending_parsed();
    assert_ne!(disk::read(&file).unwrap(), b"corrupt header");
    reg.reap_to_budget(1);
    assert_eq!(
        reg.warm_or_build(&ws).unwrap(),
        WarmOutcome::Built { docs: 0 }
    );
}

#[test]
fn restored_builder_cannot_regress_a_newer_concurrent_publication() {
    let home = tempfile::tempdir().unwrap();
    let (reg, ws) = fixture(home.path());
    reg.save_checkpoints();
    drop(reg);
    let restarted = Arc::new(registry(home.path()));
    restarted.register(&ws);
    let (arrived_tx, arrived_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    *restarted.pause_before_insert.lock().unwrap() = Some((arrived_tx, release_rx));
    let a = Arc::clone(&restarted);
    let a_ws = ws.clone();
    let first = std::thread::spawn(move || a.warm_or_build(&a_ws).unwrap());
    arrived_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("restored builder reached publication");
    disk::write(ws.join("a.md"), "# Newer publication\n").unwrap();
    assert_eq!(
        restarted.warm_or_build(&ws).unwrap(),
        WarmOutcome::Built { docs: 1 }
    );
    let newer = restarted
        .engine_snapshot(&ws)
        .unwrap()
        .at_fingerprint
        .clone();
    release_tx.send(()).unwrap();
    first.join().unwrap();
    let final_engine = restarted.engine_snapshot(&ws).unwrap();
    assert_eq!(final_engine.at_fingerprint, newer);
    assert_eq!(final_engine.docs["a.md"].raw, "# Newer publication\n");
}

fn fixture(home: &Path) -> (Registry, PathBuf) {
    let ws = home.join("ws");
    disk::create_dir_all(&ws).unwrap();
    disk::write(ws.join("a.md"), "# A\n").unwrap();
    disk::write(ws.join("b.md"), "# B\n").unwrap();
    disk::write(ws.join("bad.md"), [0xff]).unwrap();
    let ws = workspace::canonicalize(&ws).unwrap();
    let reg = registry(home);
    reg.register(&ws);
    assert_eq!(
        reg.warm_or_build(&ws).unwrap(),
        WarmOutcome::Built { docs: 2 }
    );
    (reg, ws)
}

fn registry(home: &Path) -> Registry {
    let cache = home.join("cache");
    disk::create_dir_all(&cache).unwrap();
    Registry::new(StateStore::new(home.join("state.json")), cache, Vec::new())
}

#[test]
fn unchanged_restart_parses_zero_and_retains_invalid_utf8() {
    let home = tempfile::tempdir().unwrap();
    let (reg, ws) = fixture(home.path());
    reg.save_pending_parsed();
    let original = reg.engine_snapshot(&ws).unwrap();
    drop(reg);
    let restarted = registry(home.path());
    restarted.register(&ws);
    assert_eq!(
        restarted.warm_or_build(&ws).unwrap(),
        WarmOutcome::Built { docs: 0 }
    );
    let restored = restarted.engine_snapshot(&ws).unwrap();
    assert_eq!(restored.at_fingerprint, original.at_fingerprint);
    assert_eq!(restored.unserved, original.unserved);
    assert_eq!(restored.leaves, original.leaves);
    for (path, doc) in &original.docs {
        assert_eq!(restored.docs[path].root, doc.root);
    }
}

#[test]
fn older_snapshot_reconciles_one_mover_and_a_rename() {
    let home = tempfile::tempdir().unwrap();
    let (reg, ws) = fixture(home.path());
    reg.save_pending_parsed();
    drop(reg);
    disk::write(ws.join("a.md"), "# Changed\n").unwrap();
    disk::rename(ws.join("b.md"), ws.join("renamed.md")).unwrap();
    let restarted = registry(home.path());
    restarted.register(&ws);
    assert_eq!(
        restarted.warm_or_build(&ws).unwrap(),
        WarmOutcome::Built { docs: 1 }
    );
    let engine = restarted.engine_snapshot(&ws).unwrap();
    assert!(!engine.docs.contains_key("b.md"));
    assert_eq!(engine.docs["renamed.md"].raw, "# B\n");
    assert_eq!(
        engine.at_fingerprint,
        fs::domain_snapshot(&fs::WorkspaceRoot(ws)).unwrap().1
    );
}

#[test]
fn deletion_addition_and_invalid_utf8_repair_match_fresh_truth() {
    let home = tempfile::tempdir().unwrap();
    let (reg, ws) = fixture(home.path());
    reg.save_checkpoints();
    drop(reg);
    disk::remove_file(ws.join("a.md")).unwrap();
    disk::write(ws.join("new.md"), "# New\n").unwrap();
    disk::write(ws.join("bad.md"), "# Repaired\n").unwrap();
    let restarted = registry(home.path());
    restarted.register(&ws);
    assert_eq!(
        restarted.warm_or_build(&ws).unwrap(),
        WarmOutcome::Built { docs: 2 }
    );
    let engine = restarted.engine_snapshot(&ws).unwrap();
    assert!(!engine.docs.contains_key("a.md"));
    assert!(engine.unserved.is_empty());
    assert_eq!(
        engine.at_fingerprint,
        fs::domain_snapshot(&fs::WorkspaceRoot(ws)).unwrap().1
    );
}

#[test]
fn budget_eviction_preserves_a_zero_parse_rewarm() {
    let home = tempfile::tempdir().unwrap();
    let (reg, ws) = fixture(home.path());
    assert_eq!(reg.reap_to_budget(1), vec![ws.clone()]);
    assert!(reg.engine_snapshot(&ws).is_none());
    assert!(crate::parsed_cache::path(&reg.cache_root, &ws).is_file());
    assert_eq!(
        reg.warm_or_build(&ws).unwrap(),
        WarmOutcome::Built { docs: 0 }
    );
}

#[test]
fn a_busy_saver_neither_prevents_eviction_nor_pins_queued_engines() {
    let home = tempfile::tempdir().unwrap();
    let (reg, ws) = fixture(home.path());
    let engine = reg.engine_snapshot(&ws).unwrap();
    let weak = Arc::downgrade(&engine);
    drop(engine);
    let gate = reg.persistence_gate.lock().unwrap();
    assert_eq!(reg.reap_to_budget(1), vec![ws.clone()]);
    assert!(
        weak.upgrade().is_none(),
        "queued path must not retain the engine"
    );
    drop(gate);
    reg.save_pending_parsed();
    assert!(
        reg.engine_snapshot(&ws).is_none(),
        "save must never warm a victim"
    );
}

#[test]
fn a_held_memo_does_not_park_the_saver_or_corpus_lookup() {
    let home = tempfile::tempdir().unwrap();
    let (reg, ws) = fixture(home.path());
    let memo = reg.domain_cache(&ws);
    let _held = memo.lock().unwrap();
    assert!(!reg.persist_workspace(&ws, true), "memo was skipped");
    assert!(reg.engine_snapshot(&ws).is_some());
    assert!(crate::parsed_cache::path(&reg.cache_root, &ws).is_file());
}

#[test]
fn failed_cache_storage_preserves_service_and_eviction() {
    let home = tempfile::tempdir().unwrap();
    let (reg, ws) = fixture(home.path());
    let dir = cache::parsed_drawer_dir(&reg.cache_root, &ws);
    disk::create_dir_all(dir.parent().unwrap()).unwrap();
    disk::write(&dir, "a file blocks the cache directory").unwrap();
    assert!(!reg.persist_workspace(&ws, true));
    assert_eq!(reg.warm_or_build(&ws).unwrap(), WarmOutcome::Reused);
    assert_eq!(reg.reap_to_budget(1), vec![ws.clone()]);
    assert_eq!(
        reg.warm_or_build(&ws).unwrap(),
        WarmOutcome::Built { docs: 2 }
    );
}

#[test]
fn unrelated_sql_drawer_loss_does_not_discard_parsed_documents() {
    let home = tempfile::tempdir().unwrap();
    let (reg, ws) = fixture(home.path());
    reg.save_checkpoints();
    cache::remove_drawer(&cache::drawer_dir(&reg.cache_root, &ws)).unwrap();
    drop(reg);
    let restarted = registry(home.path());
    restarted.register(&ws);
    assert_eq!(
        restarted.warm_or_build(&ws).unwrap(),
        WarmOutcome::Built { docs: 0 }
    );
}

#[test]
fn unregister_discards_the_snapshot_and_pending_save() {
    let home = tempfile::tempdir().unwrap();
    let (reg, ws) = fixture(home.path());
    reg.save_checkpoints();
    assert!(reg.unregister(&ws));
    reg.save_pending_parsed();
    assert!(!crate::parsed_cache::path(&reg.cache_root, &ws).exists());
    assert!(!crate::checkpoint::path(&reg.cache_root, &ws).exists());
}

#[test]
fn periodic_observation_saves_do_not_rewrite_the_parsed_snapshot() {
    let home = tempfile::tempdir().unwrap();
    let (reg, ws) = fixture(home.path());
    reg.save_checkpoints();
    let file = crate::parsed_cache::path(&reg.cache_root, &ws);
    let before = disk::read(&file).unwrap();
    disk::write(ws.join("a.md"), "# Changed\n").unwrap();
    reg.warm_or_build(&ws).unwrap();
    reg.save_observation_checkpoints();
    assert_eq!(disk::read(&file).unwrap(), before);
    reg.save_checkpoints();
    assert_ne!(disk::read(&file).unwrap(), before);
}

#[test]
fn restore_skips_last_use_stamp_while_the_drawer_is_busy() {
    let home = tempfile::tempdir().unwrap();
    let (reg, ws) = fixture(home.path());
    reg.save_pending_parsed();
    drop(reg);

    let restarted = Arc::new(registry(home.path()));
    restarted.register(&ws);
    let dir = cache::parsed_drawer_dir(&restarted.cache_root, &ws);
    let _held = cache::DrawerLock::acquire(&dir).unwrap();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let worker = Arc::clone(&restarted);
    let worker_ws = ws.clone();
    std::thread::spawn(move || {
        let result = worker.warm_or_build(&worker_ws);
        done_tx.send(result).unwrap();
    });
    let outcome = done_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("restore must not wait for the optional last-use stamp")
        .unwrap();
    assert_eq!(outcome, WarmOutcome::Built { docs: 0 });
}

#[test]
fn running_server_shutdown_completes_with_a_busy_parsed_drawer() {
    let config = |base: &Path| {
        let mut config = crate::Config::for_cache_root(base.join("cache"));
        config.socket_path = base.join("server.sock");
        config.drain_cold_builds = Duration::from_secs(30);
        config
    };
    let fixture = crate::test_support::TestServer::start(config).unwrap();
    let ws = fixture.path().join("ws");
    disk::create_dir_all(&ws).unwrap();
    disk::write(ws.join("a.md"), "# A\n").unwrap();
    let ws = workspace::canonicalize(&ws).unwrap();
    fixture.with(|server| {
        server.registry().register(&ws);
        server.registry().warm_or_build(&ws).unwrap();
        server.registry().save_pending_parsed();
        let old_fingerprint = server
            .registry()
            .engine_snapshot(&ws)
            .unwrap()
            .at_fingerprint
            .clone();
        disk::write(ws.join("a.md"), "# Changed before shutdown\n").unwrap();
        assert_eq!(
            server.registry().warm_or_build(&ws).unwrap(),
            WarmOutcome::Built { docs: 1 }
        );
        assert_ne!(
            server
                .registry()
                .engine_snapshot(&ws)
                .unwrap()
                .at_fingerprint,
            old_fingerprint,
            "shutdown must have a dirty parsed snapshot to save"
        );
    });
    let dir = cache::parsed_drawer_dir(&fixture.path().join("cache"), &ws);
    let held = cache::DrawerLock::acquire(&dir).unwrap();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let shutdown = std::thread::spawn(move || {
        fixture.shutdown();
        done_tx.send(()).unwrap();
    });
    let completion = done_rx.recv_timeout(Duration::from_secs(5));
    drop(held);
    shutdown.join().unwrap();
    completion.expect("server shutdown must not wait for the optional parsed-cache save");
}

#[test]
fn observation_checkpoint_stamping_skips_a_busy_parsed_drawer() {
    let home = tempfile::tempdir().unwrap();
    let (reg, ws) = fixture(home.path());
    reg.save_pending_parsed();
    let reg = Arc::new(reg);
    let dir = cache::parsed_drawer_dir(&reg.cache_root, &ws);
    let held = cache::DrawerLock::acquire(&dir).unwrap();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let worker = Arc::clone(&reg);
    let observation = std::thread::spawn(move || {
        worker.save_observation_checkpoints();
        done_tx.send(()).unwrap();
    });
    let completion = done_rx.recv_timeout(Duration::from_secs(5));
    drop(held);
    observation.join().unwrap();
    completion.expect("observation stamping must not wait for the optional cache lock");
}

/// Run only on an explicitly copied corpus; the marker is a custody receipt.
/// The normal test suite stays small. The caller controls OS cache conditions.
#[test]
#[ignore = "set MRD_BENCH_CORPUS to an owned corpus copy with .durable-cache-benchmark marker"]
fn durable_corpus_restore_measurement() {
    let ws = PathBuf::from(std::env::var_os("MRD_BENCH_CORPUS").expect("owned corpus copy"));
    assert!(ws.join(".durable-cache-benchmark").is_file());
    let home = tempfile::tempdir().unwrap();
    let reg = registry(home.path());
    let ws = workspace::canonicalize(&ws).unwrap();
    reg.register(&ws);
    let began = Instant::now();
    let first = reg.warm_or_build(&ws).unwrap();
    let cold = began.elapsed();
    let began = Instant::now();
    reg.save_checkpoints();
    let save = began.elapsed();
    let (fingerprint, raw_bytes) = {
        let engine = reg.engine_snapshot(&ws).unwrap();
        (
            engine.at_fingerprint.clone(),
            engine.docs.values().map(|d| d.raw.len()).sum::<usize>(),
        )
    };
    let snapshot_bytes = disk::metadata(crate::parsed_cache::path(&reg.cache_root, &ws))
        .unwrap()
        .len();
    drop(reg);
    let restarted = registry(home.path());
    restarted.register(&ws);
    let began = Instant::now();
    let second = restarted.warm_or_build(&ws).unwrap();
    let restore = began.elapsed();
    assert_eq!(second, WarmOutcome::Built { docs: 0 });
    assert_eq!(
        restarted.engine_snapshot(&ws).unwrap().at_fingerprint,
        fingerprint
    );
    eprintln!(
        "durable-cache-bench first={first:?} restored={second:?} cold_s={:.3} save_s={:.3} restore_s={:.3}",
        cold.as_secs_f64(),
        save.as_secs_f64(),
        restore.as_secs_f64()
    );
    eprintln!(
        "durable-cache-bench raw_bytes={raw_bytes} snapshot_bytes={snapshot_bytes} fingerprint={} semantic_generation={}",
        fingerprint.0,
        parse_cache::GENERATION
    );
}
