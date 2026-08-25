//! **A write door may not park unboundedly while it holds the D9 flock.**
//!
//! The write door's stated lock discipline (`crates/wire-serve/src/write.rs`,
//! "Lock discipline") rests on one premise: *"every door touches the cache
//! only INSIDE the D9 write flock — cooperating writers serialize before
//! their first cache touch … nothing outside this seam locks it."* If that
//! held, the shared domain memo could never be contended at a door, and an
//! unbounded `lock()` there would cost nothing.
//!
//! **The premise is false.** The READ path locks the same memo outside the
//! flock: `Registry::warm_or_build` → `Registry::currency_refresh` holds
//! `domain_cache(ws)` across `DomainCache::root()` — the §6.2 extent-refresh
//! floor, a full stat sweep — on every vouch miss. So an ordinary read can
//! hold the memo while a write door parks on it.
//!
//! A write op waits on that memo TWICE, and both waits were unbounded:
//!
//! 1. At the arm's entry, `registry.domain_cache(ws)` → `patched_cache`, which
//!    locks the memo to apply the feed's pending set. This is BEFORE
//!    `acquire_write_lock`, so no flock is held across it — it is the wait a
//!    splice reaches first, and the one this file measures.
//! 2. Inside the door, `observed_root` → `Registry::door_observation`. That
//!    one runs *within* the D9 flock, which the door takes `LOCK_NB`
//!    precisely "so a hung holder can never make callers hang"
//!    (`crates/fs/src/lib.rs`, `WriteLock`) — an unbounded wait there hands
//!    back the property the flock was written to guarantee, and holds the
//!    workspace's one write token while it does. Bounded here too; not
//!    separately reproduced.
//!
//! The consequence that makes this the CONTINUITY-RISK class: the engine
//! returns no verdict, so the only thing that ends the call is the CLIENT's
//! per-op deadline ([`CLIENT_D4_BOUND`]) — and a caller whose own deadline
//! fired cannot say whether the write landed. A rotation SEAL put is how a
//! seat hands its state to a successor; an ambiguous timeout there lands at
//! exactly the moment nothing else can recover it.
//!
//! The contract asserted here: **the door owes its own verdict before the
//! client's deadline, and that verdict must state what happened to the bytes.**
//! The write half knows the answer — the observation runs before any byte
//! moves — so "nothing was committed" is provable, not hedged (the same law
//! `wedge_write_half.rs` puts on the transport half). The second test is the
//! zero control: the bound must cost an uncontended write nothing, which is
//! what makes it safe to put on every door.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use registry::{Config, RunningServer};
use serde_json::{Value, json};
use tempfile::TempDir;

mod common;

/// The per-op deadline every MCP caller rides into this engine: ccc-statusd
/// `internal/registryclient/client.go:25`,
/// `DefaultRequestTimeout = 10 * time.Second` — *"the D4 per-op bound: 10s on
/// both the read and the write side of every request op."*
///
/// It is a CLIENT constant, and that is the whole point: when it is the thing
/// that ends the call, the engine contributed no verdict and the caller cannot
/// distinguish "nothing landed" from "landed, answer lost".
const CLIENT_D4_BOUND: Duration = Duration::from_secs(10);

/// The engine's own per-wait budget at a write door —
/// `registry::DOOR_COOKIE_TIMEOUT`, the constant the arms already pass to
/// `door_observation`. A door makes at most three such waits (arm entry,
/// cookie barrier, door observation), so its worst case is 3×this, and the
/// property that matters is that the worst case stays under
/// [`CLIENT_D4_BOUND`]: the ENGINE is always what ends the call.
const DOOR_BUDGET: Duration = Duration::from_secs(2);

/// How long the competing reader holds the shared memo. Comfortably past
/// [`CLIENT_D4_BOUND`] so a door that answers in time cannot be an artifact of
/// the hold ending on its own — the holder is still holding when we assert.
const HOLD: Duration = Duration::from_secs(30);

/// Backstop before a test calls a bounded door unbounded. A BACKSTOP, never a
/// budget: it only decides whether a regression FAILS or hangs the suite.
#[allow(clippy::duration_suboptimal_units)]
const NEVER: Duration = Duration::from_secs(180);

/// A daemon config rooted under `tmp`, with reap horizons large enough that the
/// background reaper never evicts a warm engine mid-test.
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

/// A workspace `tmp/ws` seeded with `files` — a sibling of the cache root, so
/// the corpus walk never sees the drawer.
fn write_ws(tmp: &TempDir, files: &[(&str, &str)]) -> PathBuf {
    let ws = tmp.path().join("ws");
    for (rel, content) in files {
        let path = ws.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }
    ws
}

/// A persistent NDJSON connection: `hello` binds the workspace, then every op
/// rides the same connection.
struct Conn {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Conn {
    fn open(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).unwrap();
        stream
            .set_read_timeout(Some(NEVER))
            .expect("a bounded read, so a wedged door fails the test instead of hanging it");
        Conn {
            writer: stream.try_clone().unwrap(),
            reader: BufReader::new(stream),
        }
    }

    /// One op, no retry loop: this file MEASURES the door, so it must never
    /// re-send underneath its own stopwatch.
    fn call_once(&mut self, request: &Value) -> Value {
        let mut line = serde_json::to_string(request).unwrap();
        line.push('\n');
        self.writer.write_all(line.as_bytes()).unwrap();
        self.writer.flush().unwrap();
        let mut response = String::new();
        self.reader
            .read_line(&mut response)
            .expect("the door answered at all");
        serde_json::from_str(&response).unwrap()
    }

    /// For setup ops only, where `corpus_warming` is the contract.
    fn call(&mut self, request: &Value) -> Value {
        common::honour_retry(|| self.call_once(request))
    }

    fn hello(&mut self, ws: &Path) -> Value {
        self.call(&json!({
            "op": "hello",
            "proto": 1,
            "contract": "v3",
            "workspace": ws.to_str().unwrap(),
        }))
    }
}

const PLAN: &str = "# Goals\n\nship by August\n";
const PLAN_AFTER: &str = "# Goals\n\nship by September\n";

/// A guarded `match` edit inside `Goals`. The `toc` it needs is read BEFORE the
/// memo is held — a `toc` is a read, and a read is exactly what parks on the
/// held memo, so reading it later would measure the wrong door.
fn splice_frame(conn: &mut Conn, path: &str, heading: &str) -> Value {
    let toc = conn.call(&json!({"op": "toc", "path": path}));
    let rev = toc["body"]["nodes"]
        .as_array()
        .expect("toc nodes")
        .iter()
        .find(|n| n["hpath"][0]["h"] == json!(heading))
        .unwrap_or_else(|| panic!("{heading} in toc: {toc}"))["node_rev"]
        .as_str()
        .expect("node_rev")
        .to_string();
    json!({
        "id": 7,
        "op": "splice",
        "path": path,
        "edits": [{
            "target": {"hpath": [{"h": heading}]},
            "edit": {"match": {"old": "ship by August", "new": "ship by September"}},
            "if_node_rev": rev,
        }],
    })
}

/// A reader that holds the workspace's shared domain memo — the same
/// `Arc<Mutex<DomainCache>>` a write door observes through. It stands in for
/// what `currency_refresh` does for real on a vouch miss: hold this memo across
/// `DomainCache::root()`, the full stat sweep, from OUTSIDE the write flock.
///
/// A holder, not a sleep: the test releases it explicitly, so the hold provably
/// outlives every assertion instead of racing them.
struct MemoHolder {
    release: Option<mpsc::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl MemoHolder {
    fn take(server: &RunningServer, ws: &Path) -> Self {
        let cache = server.registry().domain_cache(ws);
        let (held_tx, held_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let thread = thread::spawn(move || {
            let _guard = cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            held_tx.send(()).expect("the test is still waiting");
            // Whichever comes first: the test releasing us, or the hold's own
            // horizon. The horizon only bounds a failed run.
            let _ = release_rx.recv_timeout(HOLD);
        });
        held_rx
            .recv_timeout(NEVER)
            .expect("the holder took the shared memo");
        MemoHolder {
            release: Some(release_tx),
            thread: Some(thread),
        }
    }
}

impl Drop for MemoHolder {
    fn drop(&mut self) {
        drop(self.release.take());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Canonical form — `domain_cache` keys by the path `hello` bound, and on macOS
/// a tempdir reaches the daemon as `/private/var/...`. A holder on the
/// uncanonicalised key would lock a DIFFERENT memo and the test would pass by
/// measuring nothing.
fn canonical(ws: &Path) -> PathBuf {
    std::fs::canonicalize(ws).expect("the workspace exists")
}

/// **THE REGRESSION.** A read holds the shared memo; a splice arrives. The door
/// must answer with its own typed verdict before the client's deadline — and
/// that verdict must say the bytes never moved.
///
/// Pre-fix this call does not return at all until the holder lets go: the door
/// is parked in `observed_root`'s unbounded `lock()`, inside the flock it
/// already took. What ends it in production is [`CLIENT_D4_BOUND`], which is
/// the caller giving up, not the engine answering — and a caller that gave up
/// cannot tell a lost seal from a landed one.
#[test]
fn a_held_shared_memo_makes_the_write_door_refuse_in_time_instead_of_parking() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let ws = write_ws(&tmp, &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).expect("daemon");

    let mut conn = Conn::open(&server.socket_path());
    conn.hello(&ws);
    // Warm first, and mint the frame first: both are reads, and a read is what
    // parks on a held memo.
    let frame = splice_frame(&mut conn, "plan.md", "Goals");

    let holder = MemoHolder::take(&server, &canonical(&ws));

    let started = Instant::now();
    let resp = conn.call_once(&frame);
    let elapsed = started.elapsed();

    assert!(
        elapsed < CLIENT_D4_BOUND,
        "the door parked {elapsed:?} on a memo it does not own, past the client's own \
         {CLIENT_D4_BOUND:?} deadline — so the only thing that ends this call in production is \
         the caller giving up. Read the response: the write LANDS, later. A caller that gave up \
         at the deadline is therefore asking an unanswerable question about its own seal, and \
         the answer changes after it stops listening. Response was: {resp}"
    );
    assert_eq!(
        resp["ok"],
        json!(false),
        "a door that could not observe its own entry state must refuse, never guess: {resp}"
    );
    assert_eq!(
        resp["error"]["code"],
        json!("workspace_busy"),
        "the memo is contended exactly like the flock is contended, and the flock's answer is \
         already `workspace_busy` (transient — retry). Two contended door resources, one \
         vocabulary: {resp}"
    );
    let message = resp["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("nothing was committed"),
        "`observed_root` runs before any byte moves, so the write half KNOWS the bytes never \
         moved and owes the caller that fact — 'unknown' is what the client's timeout says, and \
         replacing it is the point: {resp}"
    );

    drop(holder);
}

/// **The zero control.** With nobody holding the memo, the bound must be
/// invisible: the write lands, and it lands fast. A bound that made the healthy
/// path pay — or worse, refuse — would be a worse defect than the one it fixes,
/// and this is the row that says it does not.
///
/// The same shape as `wedge_write_half.rs`'s draining-peer arm: the discipline
/// is only safe to put on every door because the uncontended door never
/// notices it.
#[test]
fn an_uncontended_write_door_is_untouched_by_the_bound() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let ws = write_ws(&tmp, &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).expect("daemon");

    let mut conn = Conn::open(&server.socket_path());
    conn.hello(&ws);
    let frame = splice_frame(&mut conn, "plan.md", "Goals");

    let started = Instant::now();
    let resp = conn.call_once(&frame);
    let elapsed = started.elapsed();

    assert_eq!(
        resp["ok"],
        json!(true),
        "an uncontended door commits — the bound refuses only a memo that is actually held: {resp}"
    );
    assert!(
        elapsed < DOOR_BUDGET,
        "and it never reaches the budget, so no uncontended write pays for the bound: {elapsed:?}"
    );
    assert_eq!(
        std::fs::read_to_string(ws.join("plan.md")).expect("plan.md"),
        PLAN_AFTER,
        "and the bytes are on disk"
    );
}
