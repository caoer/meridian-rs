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
//!
//! # The v3 vocabulary half
//!
//! The §Q5 gates above run on an un-negotiated (frozen v2) session, so they read
//! `as_of_root`/`live_root`. The last test in this file drives the SAME op on a
//! session that negotiated `contract:v3` and asserts the projected spelling —
//! `as_of_fingerprint`/`live_fingerprint` present, and the v2 `root` names
//! **absent**. The absent half is the load-bearing one: a rename that COPIED
//! instead of moving would leave both spellings on the wire and a present-only
//! assertion would pass over it.

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
    let forever = Duration::from_secs(365 * 24 * 60 * 60);
    let dir = tmp.path().join("registry");
    let mut config = Config::for_cache_root(tmp.path().join("cache"));
    config.socket_path = dir.join("daemon.sock");
    config.state_path = dir.join("state.json");
    config.idle_threshold = forever;
    config.reap_interval = forever;
    config.prewarm_interval = forever;
    config.prewarm_quiet_max = forever;
    // No idle exit: this server's lifetime is the test's, and a daemon that
    // reaped itself mid-assertion would fail as a flake, not a finding.
    config.idle_exit = None;
    config
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

/// A persistent connection speaking raw NDJSON. Un-negotiated by default (no
/// `contract`), so the reply carries the frozen `root` vocabulary
/// (`as_of_root`/`live_root`); [`Conn::hello`] negotiates a rev when a gate needs
/// the v3 spelling instead.
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

    /// A `hello` binding `ws` under `contract` — the only way a session moves off
    /// the frozen v2 default, since the rev is negotiated per connection.
    fn hello(&mut self, ws: &Path, contract: &str) -> Value {
        self.call(&json!({
            "op": "hello",
            "proto": 1,
            "workspace": ws.to_str().unwrap(),
            "contract": contract,
        }))
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

/// Recursively collect every object KEY in `v`. Values are never collected — a
/// corpus path that spells `root` is a legitimate VALUE and must not be re-keyed.
/// (Same mechanism as `wire_vocab_rev.rs`, kept local so this file stays a
/// standalone suite.)
fn keys(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            for (k, child) in map {
                out.push(k.clone());
                keys(child, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|c| keys(c, out)),
        _ => {}
    }
}

/// The v3 vocabulary gate for the view organ's path forwarder, end to end over
/// the socket: a session that negotiated `contract:v3` asks for a `view_path` and
/// the reply carries the fingerprint spelling **and only** the fingerprint
/// spelling.
///
/// # Why the absent-clause is the point
///
/// The Rust wire type spells `as_of_root`/`live_root` deliberately — the v3
/// projection re-keys the frozen v2 slots through the ONE rename table
/// (`wire-serve::rev`), never a second dialect. That table's contract is that it
/// **moves** a key. A rename that COPIED would leave a frame carrying BOTH
/// spellings, which a present-only assertion accepts happily. Pinning the v2
/// names as absent is what proves the move, and it is why this test exists at all
/// — every other `view_path` gate in this file runs on a v2 session.
///
/// Same shape of reasoning as G5's refusal arm asserting that zero ops reached
/// the daemon rather than merely that the caller saw an error: assert the
/// mechanism, not the symptom.
#[test]
fn a_v3_session_view_path_carries_fingerprint_slots_and_never_the_root_spelling() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("a.md", "# A\n\nsee [[b]]\n"), ("b.md", "# B\n")]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    // Negotiate v3 — the projection is per-session, so this handshake is the
    // whole difference between this gate and the ones above.
    let hi = conn.hello(&ws, "v3");
    assert_eq!(hi["ok"], json!(true), "v3 hello ok: {hi}");
    assert_eq!(
        hi["body"]["contract"],
        json!("v3"),
        "the session negotiated v3 — without this echo the assertions below \
         would be testing a v2 session that happens to look right: {hi}"
    );

    let reply = conn.view_path(&ws, None);
    assert_eq!(reply["ok"], json!(true), "v3 view_path ok: {reply}");
    let body = &reply["body"];
    let path = body["path"].as_str().expect("path is a string");
    assert!(
        path.ends_with("view.duckdb") && Path::new(path).is_file(),
        "v3 forwards the same stamped view.duckdb PATH — the projection is a \
         vocabulary re-key, never a different answer: {reply}"
    );
    assert_eq!(
        body["state"],
        json!("FRESH_AT_SAMPLE"),
        "the first build is fresh at its sample: {reply}"
    );

    // (1) PRESENT: both fingerprint slots, b3-shaped.
    for key in ["as_of_fingerprint", "live_fingerprint"] {
        assert!(
            body[key].as_str().is_some_and(|s| s.starts_with("b3:")),
            "a v3 view_path carries `{key}` as a b3 fingerprint hint: {reply}"
        );
    }

    // (2) ABSENT: the v2 `root` spellings, ANYWHERE in the frame. This is the
    // load-bearing half — a copy-instead-of-move rename passes (1) and fails
    // here, and a whole-frame key sweep catches the slot wherever it lands.
    let mut frame_keys = Vec::new();
    keys(&reply, &mut frame_keys);
    for v2_key in ["as_of_root", "live_root", "root"] {
        assert!(
            !frame_keys.iter().any(|k| k == v2_key),
            "a v3 view_path frame must emit ZERO `{v2_key}` keys — the rename \
             table MOVES the slot, and both spellings on one wire is exactly \
             what a copying projection would produce: {reply}"
        );
    }

    // The frozen invariants still hold under the v3 spelling: a pre-open hint is
    // never a verdict, and the forwarder still marshals no rows.
    assert!(
        body.as_object().unwrap().contains_key("stale"),
        "stale is always present: {reply}"
    );
    assert_eq!(body["stale"], Value::Null, "stale is ALWAYS null: {reply}");
    for tabular in ["rows", "columns", "row_count", "result"] {
        assert!(
            body.get(tabular).is_none(),
            "view_path marshals no rows (`{tabular}` absent): {reply}"
        );
    }

    server.shutdown();
}
