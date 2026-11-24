//! The caller-leave gate (`docs/wire-contract.md` § A.11), over a real
//! daemon socket: a `sql` whose caller hangs up mid-statement stops within a
//! bounded time and lets go of the workspace's projection cache, and a caller
//! that only half-closes still gets its answer.

use std::io::{BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tempfile::TempDir;

use super::{Config, RunningServer};

/// A statement with no I/O whose runtime is hours on any box this runs on:
/// only an interrupt ends it inside the gate's bound.
const ENDLESS_SQL: &str = "SELECT sum(hash(i)) FROM range(1000000000000) t(i)";

/// How long a statement may outlive its caller's hangup. The watcher's tick
/// is 50 ms; the rest is `DuckDB` reaching its next interrupt check and the
/// rollback, with room for a loaded CI box.
const STOP_BOUND: Duration = Duration::from_secs(10);

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

fn send(stream: &mut UnixStream, request: &Value) {
    let mut line = serde_json::to_string(request).unwrap();
    line.push('\n');
    stream.write_all(line.as_bytes()).unwrap();
}

fn receive(reader: &mut BufReader<UnixStream>) -> Value {
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

/// A connection bound to `ws` whose projection cache is warm, so the next
/// `sql` on it opens its read at once.
fn bound(socket: &Path, ws: &Path) -> (UnixStream, BufReader<UnixStream>) {
    let mut stream = UnixStream::connect(socket).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    send(
        &mut stream,
        &json!({"op": "hello", "proto": 1, "contract": "v3", "workspace": ws.to_str().unwrap()}),
    );
    assert_eq!(receive(&mut reader)["ok"], true, "hello");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        send(&mut stream, &json!({"id": 1, "op": "sql", "query": "SELECT 1"}));
        let answer = receive(&mut reader);
        if answer["ok"] == true {
            return (stream, reader);
        }
        assert_eq!(answer["error"]["code"], "corpus_warming", "{answer}");
        assert!(Instant::now() < deadline, "never warmed: {answer}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn until(bound: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + bound;
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    done()
}

fn workspace(tmp: &TempDir) -> std::path::PathBuf {
    let ws = tmp.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("a.md"), "# A\n").unwrap();
    ws
}

#[test]
fn a_sql_whose_caller_hangs_up_stops_and_releases_the_cache() {
    let tmp = TempDir::new().unwrap();
    let ws = workspace(&tmp);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let socket = server.socket_path().to_path_buf();
    let (mut other, mut other_reader) = bound(&socket, &ws);

    let store = server
        .registry()
        .sql_store(&std::fs::canonicalize(&ws).unwrap())
        .unwrap();
    let reads_open = || {
        !store
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .wait_for_reads(Duration::ZERO)
    };
    assert!(!reads_open(), "no read is open before the call");

    let (mut leaver, leaver_reader) = bound(&socket, &ws);
    send(&mut leaver, &json!({"id": 2, "op": "sql", "query": ENDLESS_SQL}));
    // The instrument first: the statement really is running on a read of
    // THIS store. A store looked up under the wrong key would have no reads,
    // and the release below would pass for the wrong reason.
    assert!(
        until(Duration::from_secs(30), reads_open),
        "the statement never opened a read on the workspace's cache"
    );
    std::thread::sleep(Duration::from_millis(300));
    assert!(reads_open(), "the statement ended on its own");

    drop(leaver_reader);
    drop(leaver);
    let left = Instant::now();
    assert!(
        until(STOP_BOUND, || !reads_open()),
        "the statement still holds the cache {:?} after its caller hung up",
        left.elapsed()
    );
    eprintln!("caller-leave gate: read released {:?} after the hangup", left.elapsed());

    // The workspace serves the next caller.
    send(&mut other, &json!({"id": 3, "op": "sql", "query": "SELECT 7"}));
    let answer = receive(&mut other_reader);
    assert_eq!(answer["body"]["rows"], json!([[7]]), "{answer}");
}

/// The control: bytes alone are no hangup. A caller that sends its request
/// and shuts its write side still reads its answer — the read loop sees EOF
/// only after the answer is written.
#[test]
fn a_caller_that_half_closes_still_gets_its_answer() {
    let tmp = TempDir::new().unwrap();
    let ws = workspace(&tmp);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let (mut stream, mut reader) = bound(server.socket_path(), &ws);
    send(
        &mut stream,
        &json!({"id": 2, "op": "sql", "query": "SELECT count(*)::BIGINT FROM range(20000000)"}),
    );
    stream.shutdown(Shutdown::Write).unwrap();
    let answer = receive(&mut reader);
    assert_eq!(answer["body"]["rows"], json!([[20_000_000]]), "{answer}");
}
