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

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
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

// ── notify intent continuity across warm succession ──────────────────────────

const HOOK_ID: &str = "task.review-notify";

const HOOK_PAGE: &str = r#"---
tags: [type/rule, rules/hook]
id: task.review-notify
kind: hook
severity: info
paths: ["tasks/*.md"]
caps:  [proto.send]
budget: { steps: 10000, mem: 4194304 }
how:
  route: { info: channel-review }
---

```starlark
def on_change(event):
    for delta in event.changes:
        if delta.kind != "frontmatter" or delta.key != "status":
            continue
        if delta.new != "review":
            continue
        return intent(
            action = "notify",
            target = event.facts.fm.get("reviewer"),
            severity = "info",
            payload = "task moved to review",
            receipt = receipt_addr(event.file, event.fingerprint_after),
        )
```
"#;

fn task_card(status: &str) -> String {
    format!("---\ntype: task\nstatus: {status}\nreviewer: e4201e72\n---\n\n# Task\n")
}

/// Workspace with an armed `rules/hook` page over `tasks/*.md` and one initial
/// task card in `todo`.
fn write_armed_hook_ws(tmp: &TempDir) -> PathBuf {
    use policy::armed::{ArmRequest, ArmRoot, Mode};
    use policy::{PageRef, RuleId, RuleIndex, ScopeLayer, page_rev};

    let ws = tmp.path().join("ws");
    fs::create_dir_all(ws.join("tasks")).unwrap();
    fs::write(ws.join("task-review-notify.md"), HOOK_PAGE).unwrap();
    fs::write(ws.join("tasks/card.md"), task_card("todo")).unwrap();

    let pages = [("task-review-notify.md", HOOK_PAGE)];
    let index = RuleIndex::discover(pages.iter().map(|(path, body)| PageRef {
        layer: ScopeLayer::Workspace,
        page: path,
        bytes: body,
    }));
    let source: std::collections::BTreeMap<String, String> = pages
        .iter()
        .map(|(path, body)| ((*path).to_string(), (*body).to_string()))
        .collect();
    let artifact = policy::armed::arm(
        &index,
        &ArmRoot::parse(".").expect("workspace arm root"),
        [ArmRequest {
            id: RuleId::parse(HOOK_ID).expect("rule id"),
            mode: Mode::Armed,
            attested_rev: page_rev(HOOK_PAGE),
        }],
        &source,
        policy::CheckLimits::default(),
    )
    .expect("arm hook rule");

    let armed_path = ws.join(policy::armed::ARMED_RULES_PATH);
    fs::create_dir_all(armed_path.parent().unwrap()).unwrap();
    fs::write(&armed_path, artifact.render()).unwrap();
    let marker_path = ws.join(::fs::domain::ATTESTED_MARKER_PATH);
    fs::create_dir_all(marker_path.parent().unwrap()).unwrap();
    fs::write(&marker_path, "").unwrap();
    ws
}

impl Conn {
    fn sub(&mut self) -> Value {
        self.call(&json!({"op": "sub"}))
    }

    fn sub_at(&mut self, tree_instance: &str, from_seq: u64) -> Value {
        self.call(&json!({
            "op": "sub",
            "tree_instance": tree_instance,
            "from_seq": from_seq,
        }))
    }

    fn drain_until_eof(&mut self) -> Vec<Value> {
        self.reader
            .get_ref()
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut out = Vec::new();
        loop {
            let mut line = String::new();
            match self.reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if !line.trim().is_empty() {
                        out.push(serde_json::from_str(&line).unwrap());
                    }
                }
                Err(e) => panic!("expected EOF on handed-over sub connection, got {e}"),
            }
        }
        out
    }

    fn read_n_frames(&mut self, count: usize) -> Vec<Value> {
        self.reader
            .get_ref()
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut out = Vec::with_capacity(count);
        while out.len() < count {
            let mut line = String::new();
            let n = self.reader.read_line(&mut line).unwrap();
            assert!(n > 0, "unexpected EOF waiting for frame {}", out.len() + 1);
            if !line.trim().is_empty() {
                out.push(serde_json::from_str(&line).unwrap());
            }
        }
        out
    }
}

fn notify_intents_in(frames: &[Value]) -> Vec<(String, String)> {
    let mut intents = Vec::new();
    for frame in frames {
        if let Some(effects) = frame.get("effects").and_then(Value::as_array) {
            for env in effects {
                if let Some(list) = env.get("intents").and_then(Value::as_array) {
                    for intent in list {
                        intents.push((
                            intent["rule_id"].as_str().unwrap_or("").to_owned(),
                            intent["action"].as_str().unwrap_or("").to_owned(),
                        ));
                    }
                }
            }
        }
    }
    intents
}

/// Reconnect to the successor using the `ccc-statusd` `notifysub.go` protocol:
/// old cursor → `DeadInstance` → live `sub` → if `ack.seq > 0`, re-anchor at
/// `{new_instance, 0}` and replay `1..=ack.seq`. Returns `(ack_fp, replayed_frames)`.
fn reconnect_like_notifysub_build(
    socket: &Path,
    ws: &Path,
    expected_build: &str,
    old_instance: &str,
    old_seq: u64,
) -> (String, Vec<Value>) {
    let mut c = Conn::open(socket);
    assert_eq!(c.hello(ws), expected_build);
    let refused = c.sub_at(old_instance, old_seq);
    assert_eq!(
        refused["ok"],
        json!(false),
        "old instance must refuse: {refused}"
    );
    assert_eq!(
        refused["error"]["code"],
        json!("fingerprint_unknown"),
        "dead instance refusal: {refused}"
    );
    let ack = c.sub();
    assert_eq!(ack["ok"], json!(true), "live sub on successor: {ack}");
    let new_inst = ack["body"]["tree_instance"].as_str().unwrap().to_owned();
    let tip_seq = ack["body"]["seq"].as_u64().unwrap();
    let ack_fp = ack["body"]["fingerprint"].as_str().unwrap().to_owned();
    if tip_seq == 0 {
        return (ack_fp, Vec::new());
    }
    drop(c);
    let mut replay = Conn::open(socket);
    assert_eq!(replay.hello(ws), expected_build);
    let reack = replay.sub_at(&new_inst, 0);
    assert_eq!(reack["ok"], json!(true), "re-anchor at seq 0: {reack}");
    let frames = replay.read_n_frames(usize::try_from(tip_seq).unwrap());
    let final_fp = frames.last().unwrap()["delta"]["fingerprint_after"]
        .as_str()
        .unwrap()
        .to_owned();
    (final_fp, frames)
}

fn reconnect_like_notifysub(
    socket: &Path,
    ws: &Path,
    old_instance: &str,
    old_seq: u64,
) -> (String, Vec<Value>) {
    reconnect_like_notifysub_build(socket, ws, "new", old_instance, old_seq)
}

/// Window A: records dirtied externally right before a warm succession must
/// still produce their delta frame and `rules/hook` notify intent with an
/// unbroken fingerprint chain (no silent fold into the successor's baseline).
#[test]
fn dirty_records_before_warm_succession_deliver_notify_intents() {
    let tmp = TempDir::new().unwrap();
    let ws = write_armed_hook_ws(&tmp);
    let old = RunningServer::start(test_config(&tmp, "old")).unwrap();
    let socket = old.socket_path().to_path_buf();

    // Warm the engine and arm a v3 subscriber.
    let mut reader = Conn::open(&socket);
    assert_eq!(reader.hello(&ws), "old");
    wait_for("initial workspace warm", || {
        reader.call(&json!({"op": "toc", "path": "tasks/card.md"}))["ok"] == json!(true)
    });

    let mut sub = Conn::open(&socket);
    assert_eq!(sub.hello(&ws), "old");
    let ack1 = sub.sub();
    assert_eq!(ack1["ok"], json!(true), "initial sub: {ack1}");
    let old_inst = ack1["body"]["tree_instance"].as_str().unwrap().to_owned();
    let start_fp = ack1["body"]["fingerprint"].as_str().unwrap().to_owned();

    // Dirty the card externally immediately before forcing warm succession
    // (inside DETECT_CADENCE so the resident has not yet polled it).
    fs::write(ws.join("tasks/card.md"), task_card("review")).unwrap();
    let new = RunningServer::succeed(test_config(&tmp, "new")).unwrap();
    assert!(old.handed_over(), "old daemon handed over");

    // Drain any pre-handover frames until EOF, then reconnect to the successor.
    let old_frames = sub.drain_until_eof();
    let (mut prev_fp, old_seq) = assert_chain(&start_fp, &old_frames);

    let (succ_fp, new_frames) = reconnect_like_notifysub(&socket, &ws, &old_inst, old_seq);
    if new_frames.is_empty() {
        assert_eq!(
            succ_fp, prev_fp,
            "notify LOSS: successor baseline {succ_fp} jumped past last delivered {prev_fp}"
        );
    } else {
        (prev_fp, _) = assert_chain(&prev_fp, &new_frames);
    }

    let mut all_frames = old_frames;
    all_frames.extend(new_frames);
    let intents = notify_intents_in(&all_frames);
    assert_eq!(
        intents,
        vec![(HOOK_ID.to_owned(), "notify".to_owned())],
        "expected exactly one notify intent across succession; got frames: {all_frames:?}"
    );
    assert_ne!(prev_fp, start_fp, "corpus moved to the review state");

    old.shutdown();
    new.shutdown();
}

/// Window B: records dirtied externally AFTER the warm handover commits and
/// BEFORE the subscriber finishes redialing the successor (e.g. during
/// ccc-statusd's 1 s redial floor) must still produce their delta frame and
/// `rules/hook` notify intent, chaining from the pre-handover fingerprint.
#[test]
fn dirty_records_after_handover_before_redial_deliver_notify_intents() {
    let tmp = TempDir::new().unwrap();
    let ws = write_armed_hook_ws(&tmp);
    let old = RunningServer::start(test_config(&tmp, "old")).unwrap();
    let socket = old.socket_path().to_path_buf();

    let mut sub = Conn::open(&socket);
    assert_eq!(sub.hello(&ws), "old");
    let ack1 = sub.sub();
    assert_eq!(ack1["ok"], json!(true), "initial sub: {ack1}");
    let old_inst = ack1["body"]["tree_instance"].as_str().unwrap().to_owned();
    let start_fp = ack1["body"]["fingerprint"].as_str().unwrap().to_owned();

    // Hand over while quiet, then drain old sub to EOF.
    let new = RunningServer::succeed(test_config(&tmp, "new")).unwrap();
    assert!(old.handed_over(), "old daemon handed over");
    let old_frames = sub.drain_until_eof();
    assert!(
        old_frames.is_empty(),
        "quiet handover emits no frames: {old_frames:?}"
    );

    // Dirty the card AFTER handover commit, BEFORE the subscriber redials `new`.
    fs::write(ws.join("tasks/card.md"), task_card("review")).unwrap();

    let (succ_fp, new_frames) = reconnect_like_notifysub(&socket, &ws, &old_inst, 0);
    assert!(
        !new_frames.is_empty(),
        "post-handover edit must not fold silently into successor prime() (ack fp={succ_fp}, start={start_fp})"
    );
    assert_eq!(
        new_frames[0]["delta"]["fingerprint_before"]
            .as_str()
            .unwrap(),
        start_fp,
        "first successor frame must chain from pre-handover fingerprint: {new_frames:?}"
    );
    assert_eq!(
        notify_intents_in(&new_frames),
        vec![(HOOK_ID.to_owned(), "notify".to_owned())],
        "expected notify intent for post-handover edit: {new_frames:?}"
    );

    old.shutdown();
    new.shutdown();
}

/// Measure handover stay time (wall time of `RunningServer::succeed` and max
/// held caller latency on a quiet subscribed workspace of 200 files, 5 rounds).
#[test]
fn handover_stay_time_baseline_measurement() {
    let tmp = TempDir::new().unwrap();
    let ws = write_armed_hook_ws(&tmp);
    for i in 0..200 {
        fs::write(ws.join(format!("tasks/doc-{i:03}.md")), task_card("todo")).unwrap();
    }
    let mut server = RunningServer::start(test_config(&tmp, "gen0")).unwrap();
    let socket = server.socket_path().to_path_buf();

    let mut warm_conn = Conn::open(&socket);
    warm_conn.hello(&ws);
    wait_for("initial workspace warm", || {
        warm_conn.call(&json!({"op": "toc", "path": "tasks/card.md"}))["ok"] == json!(true)
    });

    let mut stay_us = Vec::new();
    let mut max_call_us = Vec::new();
    for round in 1..=5 {
        let mut sub = Conn::open(&socket);
        sub.hello(&ws);
        assert_eq!(sub.sub()["ok"], json!(true));

        let stop = Arc::new(AtomicBool::new(false));
        let worst = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let s_clone = socket.clone();
        let w_clone = ws.clone();
        let stop_c = stop.clone();
        let worst_c = worst.clone();
        let caller = thread::spawn(move || {
            let mut c = Conn::open(&s_clone);
            c.hello(&w_clone);
            while !stop_c.load(Ordering::SeqCst) {
                let t0 = Instant::now();
                let resp = c.call(&json!({"op": "toc", "path": "tasks/card.md"}));
                assert_eq!(resp["ok"], json!(true), "{resp}");
                let us = u64::try_from(t0.elapsed().as_micros()).unwrap_or(u64::MAX);
                worst_c.fetch_max(us, Ordering::SeqCst);
                thread::sleep(Duration::from_millis(5));
            }
        });
        thread::sleep(Duration::from_millis(50));
        let t0 = Instant::now();
        let next = RunningServer::succeed(test_config(&tmp, &format!("gen{round}"))).unwrap();
        let elapsed_us = u64::try_from(t0.elapsed().as_micros()).unwrap_or(u64::MAX);
        assert!(server.handed_over());
        let _ = sub.drain_until_eof();
        stop.store(true, Ordering::SeqCst);
        caller.join().unwrap();
        stay_us.push(elapsed_us);
        max_call_us.push(worst.load(Ordering::SeqCst));
        server.shutdown();
        server = next;
    }
    eprintln!("HANDOVER_STAY_US={stay_us:?} MAX_CALL_US={max_call_us:?}");
    server.shutdown();
}

fn assert_chain(start_fp: &str, frames: &[Value]) -> (String, u64) {
    let mut prev_fp = start_fp.to_owned();
    let mut last_seq = 0u64;
    for frame in frames {
        assert_eq!(
            frame["delta"]["fingerprint_before"].as_str().unwrap(),
            prev_fp,
            "fingerprint chain broken (notify LOSS): {frame}"
        );
        frame["delta"]["fingerprint_after"]
            .as_str()
            .unwrap()
            .clone_into(&mut prev_fp);
        last_seq = frame["delta"]["seq"].as_u64().unwrap();
    }
    (prev_fp, last_seq)
}

/// Concurrent wire `splice` writers + keepalive readers across warm succession
/// with external edits both before and after handover: zero caller refusals
/// (including zero `workspace_busy` from successor pre-priming), unbroken
/// fingerprint chain, and both notify intents delivered exactly once.
#[test]
fn concurrent_puts_and_dirty_edits_across_warm_succession_lose_nothing() {
    let tmp = TempDir::new().unwrap();
    let ws = write_armed_hook_ws(&tmp);
    fs::write(ws.join("notes.md"), "# Notes\n\nseed\n").unwrap();
    fs::write(ws.join("tasks/card2.md"), task_card("todo")).unwrap();

    let old = RunningServer::start(test_config(&tmp, "old")).unwrap();
    let socket = old.socket_path().to_path_buf();
    let mut warm_conn = Conn::open(&socket);
    assert_eq!(warm_conn.hello(&ws), "old");
    wait_for("initial workspace warm", || {
        warm_conn.call(&json!({"op": "toc", "path": "notes.md"}))["ok"] == json!(true)
    });

    let mut sub = Conn::open(&socket);
    assert_eq!(sub.hello(&ws), "old");
    let ack1 = sub.sub();
    assert_eq!(ack1["ok"], json!(true), "{ack1}");
    let old_inst = ack1["body"]["tree_instance"].as_str().unwrap().to_owned();
    let start_fp = ack1["body"]["fingerprint"].as_str().unwrap().to_owned();

    let stop = Arc::new(AtomicBool::new(false));
    let puts_ok = Arc::new(AtomicUsize::new(0));
    let (s_clone, w_clone, stop_c, puts_c) =
        (socket.clone(), ws.clone(), stop.clone(), puts_ok.clone());
    let writer = thread::spawn(move || {
        let mut c = Conn::open(&s_clone);
        c.hello(&w_clone);
        let mut failures = Vec::new();
        let mut n = 0usize;
        while !stop_c.load(Ordering::SeqCst) {
            n += 1;
            let resp = c.call(&json!({
                "id": n, "op": "splice", "path": "notes.md", "force": true,
                "actor": "agent:load", "now": "2026-11-28T01:00:00Z",
                "edits": [{"target": {"hpath": [{"h": "Notes"}]}, "edit": {"put": {"at": "end", "text": format!("line {n}\n")}}}],
            }));
            if resp["ok"] == json!(true) {
                puts_c.fetch_add(1, Ordering::SeqCst);
            } else {
                failures.push(resp.to_string());
            }
            thread::sleep(Duration::from_millis(15));
        }
        failures
    });

    wait_for("puts landed before swap", || {
        puts_ok.load(Ordering::SeqCst) >= 3
    });
    fs::write(ws.join("tasks/card.md"), task_card("review")).unwrap();
    let new = RunningServer::succeed(test_config(&tmp, "new")).unwrap();
    assert!(old.handed_over(), "old daemon handed over");
    let old_frames = sub.drain_until_eof();

    fs::write(ws.join("tasks/card2.md"), task_card("review")).unwrap();
    let puts_after_swap = puts_ok.load(Ordering::SeqCst);
    wait_for("puts landed after swap", || {
        puts_ok.load(Ordering::SeqCst) >= puts_after_swap + 3
    });
    stop.store(true, Ordering::SeqCst);
    assert_eq!(
        writer.join().unwrap(),
        Vec::<String>::new(),
        "zero writer refusals across swap"
    );

    let (mid_fp, old_seq) = assert_chain(&start_fp, &old_frames);
    let (succ_fp, new_frames) = reconnect_like_notifysub(&socket, &ws, &old_inst, old_seq);
    assert!(
        !new_frames.is_empty(),
        "post-swap edits must emit frames (ack={succ_fp})"
    );
    assert_chain(&mid_fp, &new_frames);

    let mut all_frames = old_frames;
    all_frames.extend(new_frames);
    assert_eq!(
        notify_intents_in(&all_frames),
        vec![
            (HOOK_ID.to_owned(), "notify".to_owned()),
            (HOOK_ID.to_owned(), "notify".to_owned()),
        ],
        "both Window A and Window B notify intents must arrive exactly once: {all_frames:?}"
    );
    old.shutdown();
    new.shutdown();
}

/// Unvouched feed (blocked `.meridian/cookies`): `flush_detect` in `drain()`
/// still floors cleanly before handover and delivers the notify intent.
#[test]
fn dirty_records_before_succession_deliver_with_unvouched_feed() {
    let tmp = TempDir::new().unwrap();
    let ws = write_armed_hook_ws(&tmp);
    // Block cookie sentinels so the feed can never vouch.
    fs::create_dir_all(ws.join(".meridian")).unwrap();
    fs::write(ws.join(".meridian/cookies"), "").unwrap();

    let old = RunningServer::start(test_config(&tmp, "old")).unwrap();
    let socket = old.socket_path().to_path_buf();

    let mut sub = Conn::open(&socket);
    assert_eq!(sub.hello(&ws), "old");
    let ack1 = sub.sub();
    assert_eq!(ack1["ok"], json!(true), "{ack1}");
    let old_inst = ack1["body"]["tree_instance"].as_str().unwrap().to_owned();
    let start_fp = ack1["body"]["fingerprint"].as_str().unwrap().to_owned();

    fs::write(ws.join("tasks/card.md"), task_card("review")).unwrap();
    let new = RunningServer::succeed(test_config(&tmp, "new")).unwrap();
    assert!(old.handed_over());

    let old_frames = sub.drain_until_eof();
    let (prev_fp, old_seq) = assert_chain(&start_fp, &old_frames);
    let (succ_fp, new_frames) = reconnect_like_notifysub(&socket, &ws, &old_inst, old_seq);
    if new_frames.is_empty() {
        assert_eq!(succ_fp, prev_fp, "unbroken chain across unvouched swap");
    } else {
        assert_chain(&prev_fp, &new_frames);
    }
    let mut all_frames = old_frames;
    all_frames.extend(new_frames);
    assert_eq!(
        notify_intents_in(&all_frames),
        vec![(HOOK_ID.to_owned(), "notify".to_owned())],
        "notify intent delivered even when feed cannot vouch: {all_frames:?}"
    );

    old.shutdown();
    new.shutdown();
}

/// P1-2: an external burst (the `.302` git-merge shape) running continuously
/// across the handover adopt window must NEVER fold raced edits silently into
/// the successor's baseline after `MAX_ADOPT_RESYNCS`. It backs off, retries,
/// and delivers every notify intent with an unbroken fingerprint chain (0 LOSS).
#[test]
fn continuous_external_writer_across_handover_never_silently_folds() {
    const CARDS: usize = 20;
    let tmp = TempDir::new().unwrap();
    let ws = write_armed_hook_ws(&tmp);
    for i in 0..CARDS {
        fs::write(ws.join(format!("tasks/burst-{i:02}.md")), task_card("todo")).unwrap();
    }
    fs::write(ws.join("scratch.md"), "# Scratch\n\nseed\n").unwrap();

    let old = RunningServer::start(test_config(&tmp, "old")).unwrap();
    let socket = old.socket_path().to_path_buf();

    let mut warm_conn = Conn::open(&socket);
    assert_eq!(warm_conn.hello(&ws), "old");
    wait_for("initial workspace warm", || {
        warm_conn.call(&json!({"op": "toc", "path": "scratch.md"}))["ok"] == json!(true)
    });

    let mut sub = Conn::open(&socket);
    assert_eq!(sub.hello(&ws), "old");
    let ack1 = sub.sub();
    assert_eq!(ack1["ok"], json!(true), "{ack1}");
    let old_inst = ack1["body"]["tree_instance"].as_str().unwrap().to_owned();
    let start_fp = ack1["body"]["fingerprint"].as_str().unwrap().to_owned();

    // External writer runs across the entire handover window (~900 ms, > 3
    // unbacked resync attempts on the buggy head, yet finishes inside the
    // WARM_RETRY + BUSY_RETRY backoff budget).
    let ws_writer = ws.clone();
    let writer_started = Arc::new(AtomicBool::new(false));
    let started_c = writer_started.clone();
    let writer = thread::spawn(move || {
        started_c.store(true, Ordering::SeqCst);
        for i in 0..CARDS {
            fs::write(
                ws_writer.join(format!("tasks/burst-{i:02}.md")),
                task_card("review"),
            )
            .unwrap();
            thread::sleep(Duration::from_millis(20));
            fs::write(
                ws_writer.join("scratch.md"),
                format!("# Scratch\n\ntick {i}\n"),
            )
            .unwrap();
            thread::sleep(Duration::from_millis(20));
        }
    });
    wait_for("external burst started", || {
        writer_started.load(Ordering::SeqCst)
    });
    thread::sleep(Duration::from_millis(40));

    let new = RunningServer::succeed(test_config(&tmp, "new")).unwrap();
    writer.join().unwrap();
    assert!(old.handed_over(), "old daemon handed over");

    let old_frames = sub.drain_until_eof();
    let (mut prev_fp, old_seq) = assert_chain(&start_fp, &old_frames);

    // Wait briefly for the successor to observe any trailing burst edits, then
    // reconnect via the notifysub protocol.
    let (succ_fp, mut new_frames) = reconnect_like_notifysub(&socket, &ws, &old_inst, old_seq);
    if new_frames.is_empty() {
        assert_eq!(
            succ_fp, prev_fp,
            "notify LOSS: successor baseline {succ_fp} jumped past last delivered {prev_fp}"
        );
    } else {
        (prev_fp, _) = assert_chain(&prev_fp, &new_frames);
    }

    // Drain any post-reconnect frames if the trailing write had not landed before
    // reconnect (all 20 cards are already on disk because `writer.join()` finished).
    let mut all_frames = old_frames;
    all_frames.append(&mut new_frames);
    let intents = notify_intents_in(&all_frames);
    assert_eq!(
        intents.len(),
        CARDS,
        "expected all {CARDS} notify intents with 0 silent fold; got {} (prev_fp={prev_fp})",
        intents.len()
    );
    old.shutdown();
    new.shutdown();
}

/// P2-1: two back-to-back successions (`gen0 -> gen1 -> gen2`) while the
/// subscriber is still inside its redial floor (0 subscribers attached to
/// `gen1`) must preserve both external edits and wire `put`s from Window B
/// without silent folding.
#[test]
fn back_to_back_successions_during_redial_floor_preserve_window_b_edits() {
    let tmp = TempDir::new().unwrap();
    let ws = write_armed_hook_ws(&tmp);
    fs::write(ws.join("tasks/card2.md"), task_card("todo")).unwrap();
    fs::write(ws.join("tasks/card3.md"), task_card("todo")).unwrap();

    let r = RunningServer::start(test_config(&tmp, "gen0")).unwrap();
    let socket = r.socket_path().to_path_buf();

    let mut sub = Conn::open(&socket);
    assert_eq!(sub.hello(&ws), "gen0");
    let ack1 = sub.sub();
    assert_eq!(ack1["ok"], json!(true), "{ack1}");
    let old_inst = ack1["body"]["tree_instance"].as_str().unwrap().to_owned();
    let start_fp = ack1["body"]["fingerprint"].as_str().unwrap().to_owned();

    // First succession: gen0 -> gen1. Drain old sub to EOF and DO NOT redial
    // yet (simulates ccc-statusd sleeping its 1 s notifyRedialFloor).
    let s1 = RunningServer::succeed(test_config(&tmp, "gen1")).unwrap();
    assert!(r.handed_over(), "gen0 handed over to gen1");
    let old_frames = sub.drain_until_eof();
    assert!(old_frames.is_empty(), "quiet gen0->gen1: {old_frames:?}");
    r.shutdown();

    // Window B on gen1 with ZERO subscribers attached:
    // 1) an external edit (tasks/card.md -> review)
    // 2) a wire splice on gen1 (tasks/card2.md -> review)
    fs::write(ws.join("tasks/card.md"), task_card("review")).unwrap();
    let mut writer = Conn::open(&socket);
    assert_eq!(writer.hello(&ws), "gen1");
    let spliced = writer.call(&json!({
        "op": "splice",
        "path": "tasks/card2.md",
        "force": true,
        "actor": "agent:window-b",
        "now": "2026-11-28T02:00:00Z",
        "edits": [{"target": {"fm_key": "status"}, "edit": {"put": {"at": "upsert", "text": "review"}}}],
    }));
    assert_eq!(spliced["ok"], json!(true), "splice on gen1: {spliced}");

    // Second succession immediately: gen1 -> gen2, STILL before subscriber redials.
    let s2 = RunningServer::succeed(test_config(&tmp, "gen2")).unwrap();
    assert!(s1.handed_over(), "gen1 handed over to gen2");
    s1.shutdown();

    // One more external edit after gen2 commits, still before subscriber redials.
    fs::write(ws.join("tasks/card3.md"), task_card("review")).unwrap();

    // Now the subscriber wakes from its redial floor and reconnects to gen2.
    let (succ_fp, new_frames) = reconnect_like_notifysub_build(&socket, &ws, "gen2", &old_inst, 0);
    assert!(
        !new_frames.is_empty(),
        "back-to-back successions lost Window B edits (ack_fp={succ_fp}, start_fp={start_fp})"
    );
    assert_chain(&start_fp, &new_frames);
    let paths: Vec<&str> = new_frames
        .iter()
        .flat_map(|f| f["delta"]["files"].as_array().into_iter().flatten())
        .filter_map(|file| file["path"].as_str())
        .collect();
    assert_eq!(
        paths,
        vec!["tasks/card.md", "tasks/card2.md", "tasks/card3.md"],
        "all 3 Window B edits (external + wire splice on gen1, external on gen2) must survive gen0->gen1->gen2: {new_frames:?}"
    );
    assert_eq!(
        new_frames[1]["delta"]["actor"].as_str(),
        Some("agent:window-b"),
        "wire splice frame from gen1 must survive intact into gen2: {new_frames:?}"
    );
    assert_eq!(
        notify_intents_in(&new_frames),
        vec![
            (HOOK_ID.to_owned(), "notify".to_owned()),
            (HOOK_ID.to_owned(), "notify".to_owned()),
        ],
        "external Window B notify intents on gen1 and gen2 must both arrive: {new_frames:?}"
    );
    s2.shutdown();
}

/// P2-2: a persistent `prime_beside` failure on one subscribed workspace must
/// NOT abort the entire succession; the handover still succeeds and every
/// healthy subscribed workspace preserves its notify intents with 0 LOSS.
#[test]
fn unprimable_subscribed_workspace_does_not_block_succession() {
    use std::os::unix::fs::PermissionsExt;

    // Guard cleanup so TempDir always succeeds even if an assertion fails.
    struct RestorePerms(PathBuf);
    impl Drop for RestorePerms {
        fn drop(&mut self) {
            let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o644));
        }
    }

    let tmp = TempDir::new().unwrap();
    let ws_ok = write_armed_hook_ws(&tmp);
    fs::write(ws_ok.join("tasks/card2.md"), task_card("todo")).unwrap();

    let ws_bad = tmp.path().join("ws_bad");
    fs::create_dir_all(&ws_bad).unwrap();
    let bad_file = ws_bad.join("locked.md");
    fs::write(&bad_file, "# Locked\n\ninitial\n").unwrap();

    let old = RunningServer::start(test_config(&tmp, "old")).unwrap();
    let socket = old.socket_path().to_path_buf();

    // Warm and subscribe ws_ok.
    let mut warm_ok = Conn::open(&socket);
    assert_eq!(warm_ok.hello(&ws_ok), "old");
    wait_for("ws_ok warm", || {
        warm_ok.call(&json!({"op": "toc", "path": "tasks/card.md"}))["ok"] == json!(true)
    });
    let mut sub_ok = Conn::open(&socket);
    assert_eq!(sub_ok.hello(&ws_ok), "old");
    let ack_ok = sub_ok.sub();
    assert_eq!(ack_ok["ok"], json!(true), "{ack_ok}");
    let ok_inst = ack_ok["body"]["tree_instance"].as_str().unwrap().to_owned();
    let ok_start_fp = ack_ok["body"]["fingerprint"].as_str().unwrap().to_owned();

    // Subscribe ws_bad WITHOUT warming it so only `prime_beside` touches it.
    let mut sub_bad = Conn::open(&socket);
    assert_eq!(sub_bad.hello(&ws_bad), "old");
    let ack_bad = sub_bad.sub();
    assert_eq!(ack_bad["ok"], json!(true), "{ack_bad}");

    // Make ws_bad/locked.md unreadable so the successor's `prime_beside` fails
    // on ws_bad while ws_bad still exists as a directory.
    fs::set_permissions(&bad_file, fs::Permissions::from_mode(0o000)).unwrap();
    let _restore = RestorePerms(bad_file.clone());

    // Dirty ws_ok (Window A) and trigger succession.
    fs::write(ws_ok.join("tasks/card.md"), task_card("review")).unwrap();
    let new = RunningServer::succeed(test_config(&tmp, "new"))
        .expect("a single unprimable subscribed workspace must not fail succession");
    assert!(old.handed_over(), "old daemon handed over");

    let old_frames = sub_ok.drain_until_eof();
    let _ = sub_bad.drain_until_eof();
    let (mid_fp, old_seq) = assert_chain(&ok_start_fp, &old_frames);

    // Dirty ws_ok again (Window B) before reconnecting.
    fs::write(ws_ok.join("tasks/card2.md"), task_card("review")).unwrap();
    let (succ_fp, new_frames) = reconnect_like_notifysub(&socket, &ws_ok, &ok_inst, old_seq);
    assert!(
        !new_frames.is_empty(),
        "healthy workspace Window B edit preserved (ack={succ_fp})"
    );
    assert_chain(&mid_fp, &new_frames);

    let mut all_frames = old_frames;
    all_frames.extend(new_frames);
    assert_eq!(
        notify_intents_in(&all_frames),
        vec![
            (HOOK_ID.to_owned(), "notify".to_owned()),
            (HOOK_ID.to_owned(), "notify".to_owned()),
        ],
        "healthy subscribed workspace loses zero intents: {all_frames:?}"
    );
    old.shutdown();
    new.shutdown();
}

/// P3-1: frames recorded by `drain()`'s final `flush_detect` must already be
/// written onto subscriber sockets BEFORE `grant:true` / commit, so even a
/// SIGKILL of the predecessor immediately after commit loses nothing.
#[test]
fn drain_flushes_push_frames_onto_wire_before_commit() {
    let tmp = TempDir::new().unwrap();
    let ws = write_armed_hook_ws(&tmp);
    let old = RunningServer::start(test_config(&tmp, "old")).unwrap();
    let socket = old.socket_path().to_path_buf();

    let mut sub = Conn::open(&socket);
    assert_eq!(sub.hello(&ws), "old");
    let ack1 = sub.sub();
    assert_eq!(ack1["ok"], json!(true), "{ack1}");
    let start_fp = ack1["body"]["fingerprint"].as_str().unwrap().to_owned();

    // Probe warm/subscribed sets from the resident.
    let told = Conn::open(&socket).call(&json!({"op": "handover", "warm": [], "primed": []}));
    assert_eq!(told["grant"], json!(false), "{told}");
    let warm = told["warm"].clone();
    let primed: Vec<Value> = told["subscribed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["workspace"].clone())
        .collect();
    assert_eq!(primed.len(), 1, "expected 1 subscribed workspace: {told}");

    // Dirty the card externally and immediately ask for handover.
    fs::write(ws.join("tasks/card.md"), task_card("review")).unwrap();
    let mut asking = Conn::open(&socket);
    let granted = asking.call(&json!({
        "op": "handover",
        "warm": warm,
        "primed": primed,
    }));
    assert_eq!(granted["grant"], json!(true), "{granted}");

    // BEFORE sending `adopted` (and before `old` commits or closes `sub`),
    // the frame flushed in `drain()` must ALREADY sit on `sub`'s wire buffer!
    sub.reader
        .get_ref()
        .set_read_timeout(Some(Duration::from_millis(10)))
        .unwrap();
    let mut line = String::new();
    let n = sub
        .reader
        .read_line(&mut line)
        .expect("drain() must flush push_loop frames onto the wire BEFORE grant:true / commit");
    assert!(n > 0, "expected flushed frame before commit");
    let frame: Value = serde_json::from_str(&line).unwrap();
    assert_chain(&start_fp, std::slice::from_ref(&frame));
    assert_eq!(
        notify_intents_in(std::slice::from_ref(&frame)),
        vec![(HOOK_ID.to_owned(), "notify".to_owned())],
        "flushed frame carries notify intent: {frame}"
    );

    // Abort cleanly so `old` resumes and shuts down.
    drop(asking);
    assert!(!old.handed_over());
    old.shutdown();
}

/// P3-2 manual benchmark: measure handover stay and parked-caller latency on a
/// real-size workspace (`MRD_REAL_WS`, e.g. a scratch copy of `home-wiki-sessions`)
/// alongside 2 extra subscribed workspaces (3 total), for both quiet and
/// dirty-flush handovers. Run with `--ignored --nocapture`.
#[test]
#[ignore = "manual real-workspace handover stay measurement (set MRD_REAL_WS)"]
fn handover_stay_time_real_workspace() {
    let real_ws = PathBuf::from(
        std::env::var("MRD_REAL_WS").expect("set MRD_REAL_WS to a scratch real-size workspace"),
    );
    assert!(real_ws.is_dir(), "MRD_REAL_WS must exist");
    let probe_rel = std::env::var("MRD_REAL_PROBE").unwrap_or_else(|_| "README.md".to_owned());

    let tmp = TempDir::new().unwrap();
    let ws2 = tmp.path().join("sub2");
    let ws3 = tmp.path().join("sub3");
    fs::create_dir_all(&ws2).unwrap();
    fs::create_dir_all(&ws3).unwrap();
    fs::write(ws2.join("a.md"), "# S2\n\nseed\n").unwrap();
    fs::write(ws3.join("a.md"), "# S3\n\nseed\n").unwrap();

    let mut server = RunningServer::start(test_config(&tmp, "gen0")).unwrap();
    let socket = server.socket_path().to_path_buf();

    // Warm all 3 workspaces first.
    for (ws, probe) in [
        (&real_ws, probe_rel.as_str()),
        (&ws2, "a.md"),
        (&ws3, "a.md"),
    ] {
        let mut c = Conn::open(&socket);
        c.hello(ws);
        wait_for("workspace warm", || {
            c.call(&json!({"op": "toc", "path": probe}))["ok"] == json!(true)
        });
    }

    let mut quiet_stay_ms = Vec::new();
    let mut quiet_park_ms = Vec::new();
    let mut dirty_stay_ms = Vec::new();
    let mut dirty_park_ms = Vec::new();

    for round in 1..=8 {
        let dirty = round > 5;
        let mut subs = Vec::new();
        for ws in [&real_ws, &ws2, &ws3] {
            let mut sub = Conn::open(&socket);
            sub.hello(ws);
            assert_eq!(sub.sub()["ok"], json!(true));
            subs.push(sub);
        }

        let stop = Arc::new(AtomicBool::new(false));
        let worst_us = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let (s_c, w_c, p_c, stop_c, worst_c) = (
            socket.clone(),
            real_ws.clone(),
            probe_rel.clone(),
            stop.clone(),
            worst_us.clone(),
        );
        let caller = thread::spawn(move || {
            let mut c = Conn::open(&s_c);
            c.hello(&w_c);
            while !stop_c.load(Ordering::SeqCst) {
                let t0 = Instant::now();
                let resp = c.call(&json!({"op": "toc", "path": p_c}));
                assert_eq!(resp["ok"], json!(true), "{resp}");
                let us = u64::try_from(t0.elapsed().as_micros()).unwrap_or(u64::MAX);
                worst_c.fetch_max(us, Ordering::SeqCst);
                thread::sleep(Duration::from_millis(5));
            }
        });
        thread::sleep(Duration::from_millis(50));
        let mut created_probe = None;
        if dirty {
            // Dirty one domain-member file in the real workspace right before succession.
            let target = real_ws.join(format!("stay-probe-{round}.md"));
            fs::write(&target, format!("# Probe {round}\n\nbody {round}\n")).unwrap();
            created_probe = Some(target);
        }
        let t0 = Instant::now();
        let next = RunningServer::succeed(test_config(&tmp, &format!("gen{round}"))).unwrap();
        let stay_ms = t0.elapsed().as_secs_f64() * 1000.0;
        assert!(server.handed_over());
        for mut sub in subs {
            let _ = sub.drain_until_eof();
        }
        stop.store(true, Ordering::SeqCst);
        caller.join().unwrap();
        if let Some(target) = created_probe {
            let _ = fs::remove_file(target);
        }
        let park_ms = Duration::from_micros(worst_us.load(Ordering::SeqCst)).as_secs_f64() * 1000.0;
        if dirty {
            dirty_stay_ms.push(stay_ms);
            dirty_park_ms.push(park_ms);
        } else {
            quiet_stay_ms.push(stay_ms);
            quiet_park_ms.push(park_ms);
        }
        server.shutdown();
        server = next;
    }
    eprintln!(
        "REAL_WS_STAY quiet_stay_ms={quiet_stay_ms:.1?} quiet_park_ms={quiet_park_ms:.1?} dirty_stay_ms={dirty_stay_ms:.1?} dirty_park_ms={dirty_park_ms:.1?}"
    );
    server.shutdown();
}
