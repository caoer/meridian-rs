//! V2 §Q2/§Q5 gates for the resident daemon's `view_path` op — the view-organ
//! **path forwarder**. A real client sends `view_path` over the socket; the
//! daemon (the sole persistent builder, OD6) publishes `view.duckdb` under its
//! per-workspace publish mutex and returns a stamped PATH plus a pre-open
//! freshness hint. **Never rows.**
//!
//! The §Q5 gate, demonstrated end to end:
//! - the first `view_path` builds → `FRESH_AT_SAMPLE` (`as_of == live`);
//! - after a corpus mutation, a default `view_path` serves the last-good file
//!   with `as_of != live` — a **visible, legal STALE frame**, rows never
//!   involved, never an error;
//! - `fresh:true` makes `as_of == live` (`FRESH_AT_SAMPLE`) in quiescence, or
//!   reports `RACED` under churn — never a loop, never a fresh lie.
//!
//! The reply's `stale` is asserted **always null** (a pre-open hint is never a
//! verdict, B5+C3), and no tabular/row field ever appears.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use registry::{Config, RunningServer};
use serde_json::{Value, json};
use tempfile::TempDir;

/// A daemon config rooted under `tmp`, with reap + pre-warm horizons large
/// enough that no background thread rebuilds or evicts mid-test — every
/// fingerprint move in these gates is the test's own corpus edit.
#[allow(clippy::duration_suboptimal_units)]
fn test_config(tmp: &TempDir) -> Config {
    let dir = tmp.path().join("registry");
    Config {
        socket_path: dir.join("daemon.sock"),
        state_path: dir.join("state.json"),
        cache_root: tmp.path().join("cache"),
        idle_threshold: Duration::from_secs(365 * 24 * 60 * 60),
        reap_interval: Duration::from_secs(365 * 24 * 60 * 60),
        prewarm_interval: Duration::from_secs(365 * 24 * 60 * 60),
        prewarm_quiet_max: Duration::from_secs(365 * 24 * 60 * 60),
        // No idle exit: this server's lifetime is the test's, and a daemon that
        // reaped itself mid-assertion would fail as a flake, not a finding.
        idle_exit: None,
        push_write_timeout: registry::DEFAULT_PUSH_WRITE_TIMEOUT,
        // No build identity configured: this fixture is not testing the hello
        // identity field, and an absent sha is the honest state for a server
        // started from a test harness rather than a deployed binary.
        build_sha: None,
    }
}

/// A workspace `tmp/ws` seeded with `files` — a sibling of the cache root, so the
/// corpus walk never sees the drawer.
fn write_ws(tmp: &TempDir, files: &[(&str, &str)]) -> PathBuf {
    let ws = tmp.path().join("ws");
    for (rel, content) in files {
        let path = ws.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }
    ws
}

/// A persistent connection speaking raw NDJSON. A v2 session (no `contract`), so
/// the reply carries the frozen `root` vocabulary (`as_of_root`/`live_root`).
struct Conn {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Conn {
    fn open(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).unwrap();
        Conn {
            writer: stream.try_clone().unwrap(),
            reader: BufReader::new(stream),
        }
    }

    fn call(&mut self, request: &Value) -> Value {
        let mut line = serde_json::to_string(request).unwrap();
        line.push('\n');
        self.writer.write_all(line.as_bytes()).unwrap();
        self.writer.flush().unwrap();
        let mut response = String::new();
        self.reader.read_line(&mut response).unwrap();
        serde_json::from_str(&response).unwrap()
    }

    /// `view_path` with an explicit `cwd`; `fresh` rides only when `Some`.
    fn view_path(&mut self, cwd: &Path, fresh: Option<bool>) -> Value {
        let mut req = json!({"id": 1, "op": "view_path", "cwd": cwd.to_str().unwrap()});
        if let Some(f) = fresh {
            req["fresh"] = json!(f);
        }
        self.call(&req)
    }
}

/// Assert the invariants EVERY `view_path` reply must hold (path forwarder, not
/// a SQL proxy): `ok`, a stamped `view.duckdb` PATH, both `root`-vocabulary
/// fingerprints present, `stale` ALWAYS null, `live_source` a pre-open hint
/// (`watch`/`none`, never `fold`), and NO tabular/row field anywhere.
fn assert_reply_invariants(reply: &Value) {
    assert_eq!(reply["ok"], json!(true), "view_path ok: {reply}");
    let body = &reply["body"];
    let path = body["path"].as_str().expect("path is a string");
    assert!(
        path.ends_with("view.duckdb"),
        "path forwards the stamped view.duckdb: {reply}"
    );
    assert!(
        Path::new(path).is_file(),
        "the forwarded path exists on disk: {reply}"
    );
    assert!(
        body["as_of_root"]
            .as_str()
            .is_some_and(|s| s.starts_with("b3:")),
        "as_of_root is a b3 fingerprint hint: {reply}"
    );
    assert!(
        body["live_root"]
            .as_str()
            .is_some_and(|s| s.starts_with("b3:")),
        "live_root is a b3 fingerprint hint: {reply}"
    );
    // The one field the design nails down hardest: a PRE-OPEN hint is never a
    // verdict, so `stale` is present AND null on every reply.
    assert!(
        body.as_object().unwrap().contains_key("stale"),
        "stale is always present: {reply}"
    );
    assert_eq!(body["stale"], Value::Null, "stale is ALWAYS null: {reply}");
    assert_eq!(
        body["live_source"],
        json!("watch"),
        "the daemon's live sample is a warm hint, never a post-result fold: {reply}"
    );
    assert_eq!(
        body["refresh_in_progress"],
        json!(false),
        "round-1 rebuilds are synchronous — no refresh in flight at reply time: {reply}"
    );
    // Path forwarder, NOT a SQL proxy: no row/column/result surface ever rides.
    for tabular in ["rows", "columns", "row_count", "result"] {
        assert!(
            body.get(tabular).is_none(),
            "view_path marshals no rows (`{tabular}` absent): {reply}"
        );
    }
}

/// The §Q5 gate, end to end: build → mutation-is-visible-STALE → `--fresh` →
/// `FRESH_AT_SAMPLE`, all over the socket, rows never involved.
#[test]
fn q5_gate_build_then_mutation_is_a_visible_stale_frame_then_fresh_reconverges() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("a.md", "# A\n\nsee [[b]]\n"), ("b.md", "# B\n")]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    // (1) First call: absent view → the daemon builds + publishes. Quiescent, so
    // the post-build sample matches: FRESH_AT_SAMPLE, as_of == live.
    let first = conn.view_path(&ws, None);
    assert_reply_invariants(&first);
    assert_eq!(
        first["body"]["state"],
        json!("FRESH_AT_SAMPLE"),
        "the first build is fresh at its sample: {first}"
    );
    assert_eq!(
        first["body"]["as_of_root"], first["body"]["live_root"],
        "FRESH_AT_SAMPLE ⇒ as_of == live: {first}"
    );
    let as_of_built = first["body"]["as_of_root"].as_str().unwrap().to_string();

    // (2) Mutate the corpus — disk moves off the built fingerprint.
    fs::write(ws.join("a.md"), "# A changed\n\nsee [[b]] and more\n").unwrap();

    // A DEFAULT (non-fresh) view_path serves the last-good file WITHOUT a
    // rebuild — as_of (the published stamp) != live (the fresh disk fold): a
    // visible, legal STALE frame. Rows never involved.
    let stale = conn.view_path(&ws, None);
    assert_reply_invariants(&stale);
    assert_eq!(
        stale["body"]["state"],
        json!("STALE"),
        "a mutated corpus is a visible STALE frame: {stale}"
    );
    assert_eq!(
        stale["body"]["as_of_root"],
        json!(as_of_built),
        "STALE still serves the last-good published as_of (no rebuild): {stale}"
    );
    assert_ne!(
        stale["body"]["as_of_root"], stale["body"]["live_root"],
        "STALE ⇒ as_of != live is VISIBLE, never an error: {stale}"
    );

    // (3) fresh:true asks for a bounded rebuild. Quiescent now → the rebuild
    // reaches as_of == live: FRESH_AT_SAMPLE, at the NEW fingerprint.
    let fresh = conn.view_path(&ws, Some(true));
    assert_reply_invariants(&fresh);
    assert_eq!(
        fresh["body"]["state"],
        json!("FRESH_AT_SAMPLE"),
        "--fresh reconverges to fresh in quiescence: {fresh}"
    );
    assert_eq!(
        fresh["body"]["as_of_root"], fresh["body"]["live_root"],
        "--fresh FRESH_AT_SAMPLE ⇒ as_of == live: {fresh}"
    );
    assert_ne!(
        fresh["body"]["as_of_root"],
        json!(as_of_built),
        "--fresh rebuilt at the NEW fingerprint, not the stale one: {fresh}"
    );

    server.shutdown();
}

/// `fresh:true` under continuous churn is BOUNDED (§Q3): it reaches
/// `FRESH_AT_SAMPLE` or reports RACED, never STALE, never an error, never a loop.
/// A background thread rewrites the corpus with distinct content in a tight
/// loop, so the post-build sample keeps missing — RACED is observed within the
/// bounded retry. (Design gate 11: quiescent → FRESH, churn → RACED.)
#[test]
fn fresh_under_churn_is_bounded_and_reports_raced() {
    let tmp = TempDir::new().unwrap();
    // A multi-file corpus so each full-corpus fold takes measurable time — the
    // churn window the post-build sample races against.
    let ws = write_ws(
        &tmp,
        &[
            ("a.md", "# A\n\n0\n"),
            ("b.md", "# B\n\n0\n"),
            ("c.md", "# C\n\n0\n"),
            ("d.md", "# D\n\n0\n"),
        ],
    );
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    let stop = Arc::new(AtomicBool::new(false));
    let churn_ws = ws.clone();
    let churn_stop = stop.clone();
    let churn = std::thread::spawn(move || {
        let mut n: u64 = 0;
        while !churn_stop.load(Ordering::Relaxed) {
            n += 1;
            // Distinct content every write ⇒ the corpus fingerprint keeps moving.
            let _ = fs::write(churn_ws.join("a.md"), format!("# A\n\n{n}\n"));
        }
    });

    // Under continuous churn, a bounded --fresh call reaches RACED within its
    // one retry. Loop a bounded number of attempts to observe it deterministically
    // (each attempt is itself bounded — never a loop inside the daemon).
    let mut observed_raced = false;
    for _ in 0..64 {
        let reply = conn.view_path(&ws, Some(true));
        assert_reply_invariants(&reply);
        let state = reply["body"]["state"].as_str().expect("state");
        assert!(
            matches!(state, "FRESH_AT_SAMPLE" | "RACED"),
            "a --fresh reply is only ever FRESH_AT_SAMPLE or RACED, never STALE/error: {reply}"
        );
        if state == "RACED" {
            // RACED ⇒ the bounded retry could not reach equality: as_of != live.
            assert_ne!(
                reply["body"]["as_of_root"], reply["body"]["live_root"],
                "RACED carries both distinct fingerprints: {reply}"
            );
            observed_raced = true;
            break;
        }
    }
    stop.store(true, Ordering::Relaxed);
    churn.join().unwrap();

    assert!(
        observed_raced,
        "continuous churn drives --fresh to RACED within the bounded retry"
    );

    server.shutdown();
}

/// §3.2 discovery honesty + the self-resolving contract: the daemon advertises
/// `view_path` in its caps, and a `view_path` succeeds with NO prior `hello`
/// (the op carries its own `cwd`).
#[test]
fn view_path_is_advertised_and_needs_no_hello() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("a.md", "# A\n")]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    // A bare hello lists caps — `view_path` is served, so it is advertised.
    let ack = conn.call(&json!({"op": "hello", "proto": 1}));
    let caps: Vec<&str> = ack["body"]["caps"]
        .as_array()
        .expect("caps array")
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert!(
        caps.contains(&"view_path"),
        "the daemon advertises view_path (§3.2 discovery honesty): {ack}"
    );

    // A fresh connection sends view_path with NO hello — it self-resolves `cwd`.
    let mut bare = Conn::open(server.socket_path());
    let reply = bare.view_path(&ws, None);
    assert_reply_invariants(&reply);
    assert_eq!(
        reply["body"]["state"],
        json!("FRESH_AT_SAMPLE"),
        "an unbound view_path self-resolves and builds: {reply}"
    );

    server.shutdown();
}
