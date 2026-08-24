//! TEMPORARY measurement harness for card `register-disk-io-under-global-write-lock`.
//!
//! Not a gate — it asserts nothing about time. It prints greet-latency
//! percentiles while N threads register FRESH workspaces (the first-writer path:
//! drawer sentinel + state-file rewrite). Run it on both sides of the fix:
//!
//!   cargo test -p registry --test greet_latency_harness -- --ignored --nocapture
//!
//! Deleted once the numbers are on the card; recoverable from this branch's
//! history.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use registry::{Client, Config, RunningServer};
use serde_json::{Value, json};
use tempfile::TempDir;

/// Loader threads hammering fresh registrations.
const LOADERS: usize = 8;
/// Fresh workspaces each loader registers.
const PER_LOADER: usize = 150;
/// Greets timed against the one warm workspace.
const GREETS: usize = 400;

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
    config.idle_exit = None;
    config.drain_cold_builds = Duration::from_secs(30);
    config
}

/// One hello frame on a fresh connection; returns the frame round-trip.
fn timed_greet(socket: &Path, ws: &Path) -> Duration {
    let stream = UnixStream::connect(socket).unwrap();
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);
    let request: Value = json!({
        "op": "hello",
        "proto": 1,
        "contract": "v3",
        "workspace": ws.to_str().unwrap(),
    });
    let mut line = serde_json::to_string(&request).unwrap();
    line.push('\n');

    let started = Instant::now();
    writer.write_all(line.as_bytes()).unwrap();
    writer.flush().unwrap();
    let mut response = String::new();
    reader.read_line(&mut response).unwrap();
    let elapsed = started.elapsed();

    let body: Value = serde_json::from_str(&response).unwrap();
    assert_eq!(body["ok"], json!(true), "greet failed: {body}");
    elapsed
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    let idx = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[idx]
}

#[test]
#[ignore = "measurement harness: run with --ignored --nocapture and read the percentiles"]
fn greet_latency_under_fresh_register_load() {
    let tmp = TempDir::new().unwrap();

    // The warm workspace every greet binds — registered once up front, so each
    // greet is the ADOPT path (the map take under measurement).
    let warm = tmp.path().join("warm");
    fs::create_dir_all(&warm).unwrap();
    fs::write(warm.join("a.md"), "# A\n").unwrap();

    // Fresh workspaces for the loaders, all created before the clock starts so
    // mkdir cost is not attributed to the daemon.
    let mut lanes: Vec<Vec<PathBuf>> = Vec::with_capacity(LOADERS);
    for lane in 0..LOADERS {
        let mut paths = Vec::with_capacity(PER_LOADER);
        for i in 0..PER_LOADER {
            let ws = tmp.path().join(format!("load-{lane}-{i}"));
            fs::create_dir_all(&ws).unwrap();
            fs::write(ws.join("a.md"), "# A\n").unwrap();
            paths.push(ws);
        }
        lanes.push(paths);
    }

    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let sock = server.socket_path().to_path_buf();
    timed_greet(&sock, &warm);

    // Baseline: greets with no register load at all.
    let mut quiet: Vec<Duration> = (0..GREETS).map(|_| timed_greet(&sock, &warm)).collect();
    quiet.sort_unstable();

    let stop = Arc::new(AtomicBool::new(false));
    let loaders: Vec<_> = lanes
        .into_iter()
        .map(|paths| {
            let (sock, stop) = (sock.clone(), Arc::clone(&stop));
            thread::spawn(move || {
                let client = Client::new(sock);
                let mut done = 0usize;
                for ws in paths {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    client.register(&ws).unwrap();
                    done += 1;
                }
                done
            })
        })
        .collect();

    let mut loaded: Vec<Duration> = (0..GREETS).map(|_| timed_greet(&sock, &warm)).collect();
    stop.store(true, Ordering::Relaxed);
    let registered: usize = loaders.into_iter().map(|h| h.join().unwrap()).sum();
    loaded.sort_unstable();

    for (label, samples) in [("quiet", &quiet), ("under-load", &loaded)] {
        println!(
            "greet {label:<10} n={} min={:?} p50={:?} p90={:?} p99={:?} max={:?}",
            samples.len(),
            samples[0],
            percentile(samples, 0.50),
            percentile(samples, 0.90),
            percentile(samples, 0.99),
            samples[samples.len() - 1],
        );
    }
    println!("fresh registrations completed during the loaded window: {registered}");

    server.shutdown();
}
