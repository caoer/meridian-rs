//! Succession (`docs/wire-contract.md` §A.3): a successor daemon warms beside
//! the serving one and takes its listening socket, its singleton lock and its
//! idle connections. A caller sees no refusal, no `corpus_warming` and no
//! closed connection across the swap; a successor that never adopts leaves
//! the serving daemon serving.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use registry::{Config, RunningServer};
use serde_json::{Value, json};
use tempfile::TempDir;

/// A daemon config rooted under `tmp`, reap horizons too large to interfere
/// (the `engine_rpc` precedent).
#[allow(clippy::duration_suboptimal_units)]
fn test_config(tmp: &TempDir, build: &str) -> Config {
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
    config.build_sha = Some(build.to_owned());
    config
}

fn write_ws(tmp: &TempDir) -> PathBuf {
    let ws = tmp.path().join("ws");
    fs::create_dir_all(&ws).unwrap();
    fs::write(ws.join("a.md"), "# A\n\nbody\n").unwrap();
    fs::write(ws.join("b.md"), "# B\n").unwrap();
    ws
}

struct Conn {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Conn {
    fn open(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).unwrap();
        // A starved read fails loud instead of hanging the test.
        stream
            .set_read_timeout(Some(Duration::from_secs(60)))
            .unwrap();
        Conn {
            writer: stream.try_clone().unwrap(),
            reader: BufReader::new(stream),
        }
    }

    fn call(&mut self, request: &Value) -> Value {
        let mut line = serde_json::to_string(request).unwrap();
        line.push('\n');
        self.writer.write_all(line.as_bytes()).unwrap();
        let mut response = String::new();
        self.reader.read_line(&mut response).unwrap();
        assert!(
            !response.is_empty(),
            "the daemon closed the connection on {request}"
        );
        serde_json::from_str(&response).unwrap()
    }

    /// `hello` binding `ws`; answers the daemon's build identity.
    fn hello(&mut self, ws: &Path) -> String {
        let greeted = self.call(&json!({
            "op": "hello",
            "proto": 1,
            "contract": "v3",
            "workspace": ws.to_str().unwrap(),
        }));
        assert_eq!(greeted["ok"], json!(true), "hello: {greeted}");
        greeted["body"]["identity"]["build"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn toc(&mut self) -> Value {
        self.call(&json!({"op": "toc", "path": "a.md"}))
    }

    /// Retry `toc` through the cold start's `corpus_warming`.
    fn toc_until_warm(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let response = self.toc();
            if response["ok"] == json!(true) {
                return;
            }
            assert_eq!(response["error"]["code"], json!("corpus_warming"));
            assert!(Instant::now() < deadline, "the drawer did not land");
            thread::sleep(Duration::from_millis(20));
        }
    }
}

/// Callers that keep asking across a swap: one on a connection it holds, one
/// that dials fresh each time. Every answer that is not a served `toc` is
/// kept.
struct Callers {
    stop: Arc<AtomicBool>,
    served: Arc<AtomicUsize>,
    threads: Vec<thread::JoinHandle<Vec<String>>>,
}

impl Callers {
    fn start(socket: &Path, ws: &Path) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let served = Arc::new(AtomicUsize::new(0));
        let mut threads = Vec::new();
        for fresh_dial in [false, true] {
            let (socket, ws) = (socket.to_path_buf(), ws.to_path_buf());
            let (stop, served) = (stop.clone(), served.clone());
            threads.push(thread::spawn(move || {
                let mut failures = Vec::new();
                let mut held = Conn::open(&socket);
                held.hello(&ws);
                while !stop.load(Ordering::SeqCst) {
                    let response = if fresh_dial {
                        let mut conn = Conn::open(&socket);
                        conn.hello(&ws);
                        conn.toc()
                    } else {
                        held.toc()
                    };
                    if response["ok"] == json!(true) {
                        served.fetch_add(1, Ordering::SeqCst);
                    } else {
                        failures.push(response.to_string());
                    }
                }
                failures
            }));
        }
        Callers {
            stop,
            served,
            threads,
        }
    }

    fn served(&self) -> usize {
        self.served.load(Ordering::SeqCst)
    }

    fn finish(self) -> Vec<String> {
        self.stop.store(true, Ordering::SeqCst);
        self.threads
            .into_iter()
            .flat_map(|thread| thread.join().unwrap())
            .collect()
    }
}

fn wait_for(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_successor_takes_the_socket_and_no_caller_sees_it() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp);
    let old = RunningServer::start(test_config(&tmp, "old")).unwrap();
    let socket = old.socket_path().to_path_buf();

    // A connection opened and bound before the swap, idle across it.
    let mut held = Conn::open(&socket);
    assert_eq!(held.hello(&ws), "old");
    held.toc_until_warm();

    let callers = Callers::start(&socket, &ws);
    wait_for("callers served by the old daemon", || callers.served() > 20);

    let new = RunningServer::succeed(test_config(&tmp, "new")).unwrap();
    assert!(old.handed_over(), "the old daemon handed over");
    let before = callers.served();
    wait_for("callers served by the successor", || {
        callers.served() > before + 20
    });
    assert_eq!(
        callers.finish(),
        Vec::<String>::new(),
        "no caller saw a refusal, a warming answer or a closed connection"
    );

    // The idle connection was carried over with its binding: same socket,
    // no new hello, served by the successor's warm engine.
    assert_eq!(held.toc()["ok"], json!(true), "the carried connection");
    assert_eq!(Conn::open(&socket).hello(&ws), "new", "a fresh dial");

    // The old daemon's stop saves nothing and removes nothing.
    old.shutdown();
    assert_eq!(Conn::open(&socket).hello(&ws), "new", "after the old stop");
    assert_eq!(held.toc()["ok"], json!(true));

    // The singleton lock moved with its descriptor: it was never free, and
    // it is the successor's to release.
    let refused = RunningServer::start(test_config(&tmp, "third")).unwrap_err();
    assert_eq!(refused.kind(), std::io::ErrorKind::AlreadyExists);
    new.shutdown();
    RunningServer::start(test_config(&tmp, "third"))
        .expect("the lock is free once the successor stops")
        .shutdown();
}

#[test]
fn a_successor_that_never_adopts_leaves_the_daemon_serving() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp);
    let old = RunningServer::start(test_config(&tmp, "old")).unwrap();
    let socket = old.socket_path().to_path_buf();

    let mut held = Conn::open(&socket);
    held.hello(&ws);
    held.toc_until_warm();

    // A cold successor is told what to warm, and nothing is passed.
    let mut asking = Conn::open(&socket);
    let told = asking.call(&json!({"op": "handover", "warm": []}));
    assert_eq!(told["grant"], json!(false), "{told}");
    assert_eq!(told["reason"], json!("warm"), "{told}");
    let warm = told["warm"].clone();
    assert_eq!(warm.as_array().unwrap().len(), 1, "{told}");

    // A successor that claims the warm set is granted, then dies before it
    // adopts: the daemon resumes with its own copies of everything.
    let mut dying = Conn::open(&socket);
    let granted = dying.call(&json!({"op": "handover", "warm": warm}));
    assert_eq!(granted["grant"], json!(true), "{granted}");
    assert_eq!(
        granted["conns"].as_array().unwrap().len(),
        1,
        "the held connection was parked: {granted}"
    );
    drop(dying);

    assert_eq!(
        held.toc()["ok"],
        json!(true),
        "the parked connection resumed"
    );
    assert_eq!(Conn::open(&socket).hello(&ws), "old", "a fresh dial");
    assert!(!old.handed_over());
    old.shutdown();
    assert!(!socket.exists(), "an ordinary stop removes the socket");
}

#[test]
fn a_request_that_does_not_end_keeps_the_daemon_and_the_answer_names_it() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp);
    let old = RunningServer::start(test_config(&tmp, "old")).unwrap();
    let socket = old.socket_path().to_path_buf();

    let mut held = Conn::open(&socket);
    held.hello(&ws);
    held.toc_until_warm();
    let warm = Conn::open(&socket).call(&json!({"op": "handover", "warm": []}))["warm"].clone();

    // A caller stopped in the middle of a request line: its connection is at
    // no request boundary, so it cannot be parked.
    let mut stalled = Conn::open(&socket);
    stalled.hello(&ws);
    stalled.writer.write_all(br#"{"op": "toc", "#).unwrap();
    // Longer than the tick a connection waits for its next bytes.
    thread::sleep(Duration::from_millis(300));

    // The drain waits its whole bound for that one connection, then the
    // daemon serves on and says what it waited for.
    let asked = Instant::now();
    let told = Conn::open(&socket).call(&json!({"op": "handover", "warm": warm}));
    assert_eq!(told["grant"], json!(false), "{told}");
    assert_eq!(told["reason"], json!("busy"), "{told}");
    // The count is the stalled connection, and for an instant in every accept
    // poll the accept loop's own pass.
    assert!(
        told["busy"]
            .as_str()
            .is_some_and(|what| what.ends_with(" request(s) in flight")),
        "{told}"
    );
    assert!(
        asked.elapsed() >= Duration::from_secs(4),
        "the drain waited its bound: {:?}",
        asked.elapsed()
    );
    assert_eq!(held.toc()["ok"], json!(true), "the daemon serves on");

    // The request ends: both connections are at a boundary, and the next ask
    // is granted at once.
    stalled.writer.write_all(b"\"path\": \"a.md\"}\n").unwrap();
    let mut answer = String::new();
    stalled.reader.read_line(&mut answer).unwrap();
    let answer: Value = serde_json::from_str(&answer).unwrap();
    assert_eq!(answer["ok"], json!(true), "{answer}");
    let asked = Instant::now();
    let mut dying = Conn::open(&socket);
    let granted = dying.call(&json!({"op": "handover", "warm": warm}));
    assert_eq!(granted["grant"], json!(true), "{granted}");
    assert_eq!(granted["conns"].as_array().unwrap().len(), 2, "{granted}");
    assert!(
        asked.elapsed() < Duration::from_secs(4),
        "nothing to wait for: {:?}",
        asked.elapsed()
    );
    drop(dying);

    assert_eq!(held.toc()["ok"], json!(true), "resumed");
    assert!(!old.handed_over());
    old.shutdown();
}
