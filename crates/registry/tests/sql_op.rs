//! E2E gates for `sql` — corpus SQL over the wire (`docs/wire-contract.md`
//! § A.11): one statement over the workspace's fingerprint-pinned projection
//! cache, served by the resident engine on the one execution path (the
//! NO-SANDBOX ruling, 2026-08-14), freshness folded post-result. v3-only,
//! advertised as cap `sql` (v3 projection); the daemon is the cache file's
//! single owner and its one append actor.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use registry::{Config, RunningServer};
use serde_json::{Value, json};
use tempfile::TempDir;

mod common;

/// A daemon config rooted under `tmp`, with reap horizons large enough that
/// the background reaper never evicts state mid-test.
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

/// A persistent connection speaking raw NDJSON: one frame in, one frame out.
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
        common::honour_retry(|| {
            let mut line = serde_json::to_string(request).unwrap();
            line.push('\n');
            self.writer.write_all(line.as_bytes()).unwrap();
            self.writer.flush().unwrap();
            let mut response = String::new();
            self.reader.read_line(&mut response).unwrap();
            serde_json::from_str(&response).unwrap()
        })
    }

    fn hello_v3(&mut self, workspace: &Path) -> Value {
        self.call(&json!({
            "op": "hello", "proto": 1, "contract": "v3",
            "workspace": workspace.to_str().unwrap(),
        }))
    }

    fn sql(&mut self, id: u64, query: &str) -> Value {
        self.call(&json!({"id": id, "op": "sql", "query": query}))
    }
}

fn write(ws: &Path, rel: &str, body: &str) {
    let p = ws.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

/// The § A.11 lifecycle, one workspace: cap advertised; a read answers
/// FRESH with rows at the engine's pin; a corpus move appends delta-grain
/// (pin ledger observable through the op itself); a failed query answers a
/// SUCCESS body with `state: UNVERIFIED` + the engine's words; view-DML
/// refuses with the OQ1 teaching; the wire runs the same one path as the
/// CLI — nothing locked, nothing disabled.
#[test]
fn sql_lifecycle_over_the_wire() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ws");
    write(&ws, "a.md", "# A\n\nsee [[b]]\n");
    write(&ws, "b.md", "# B\n");

    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    let hi = conn.hello_v3(&ws);
    assert_eq!(hi["ok"], true, "hello: {hi}");
    let caps: Vec<&str> = hi["body"]["caps"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(caps.contains(&"sql"), "v3 hello advertises sql: {caps:?}");

    // A read at the warm pin: FRESH, rows, typed columns.
    let read = conn.sql(1, "SELECT path FROM doc ORDER BY path");
    assert_eq!(read["ok"], true, "sql read: {read}");
    let body = &read["body"];
    assert_eq!(
        body["state"], "FRESH_AT_SAMPLE",
        "warm pin is fresh: {body}"
    );
    assert_eq!(body["rows"], json!([["a.md"], ["b.md"]]));
    assert_eq!(body["row_count"], 2);
    assert_eq!(
        body["columns"],
        json!([{"name": "path", "type": "VARCHAR"}])
    );
    let as_of = body["as_of_fingerprint"].as_str().unwrap();
    assert!(as_of.starts_with("b3"), "the pin is a real fold: {as_of}");
    assert_eq!(body["live"].as_str().unwrap(), as_of);

    // The file was cold-built once: pin ledger at gen 1.
    let pins = conn.sql(2, "SELECT count(*) FROM hist.pin");
    assert_eq!(pins["body"]["rows"], json!([[1]]), "{pins}");

    // Move the corpus: the next call re-warms and appends delta-grain.
    write(&ws, "a.md", "# A moved\n\nsee [[b]]\n");
    let after = conn.sql(
        3,
        "SELECT (SELECT count(*) FROM hist.pin), (SELECT count(*) FROM hist.doc WHERE gen = 2)",
    );
    assert_eq!(after["ok"], true, "{after}");
    assert_eq!(
        after["body"]["rows"],
        json!([[2, 1]]),
        "one moved file = one appended doc version: {after}"
    );
    assert_eq!(after["body"]["state"], "FRESH_AT_SAMPLE");

    // A failed query is a SUCCESS body: UNVERIFIED + the engine's words.
    let failed = conn.sql(4, "SELECT nope FROM missing");
    assert_eq!(
        failed["ok"], true,
        "the frame is the honest report: {failed}"
    );
    assert_eq!(failed["body"]["state"], "UNVERIFIED");
    assert!(
        failed["body"]["error"]
            .as_str()
            .unwrap()
            .contains("missing"),
        "engine words verbatim: {failed}"
    );

    // View-DML refuses with the OQ1 teaching (the hist lane named).
    let dml = conn.sql(5, "UPDATE doc SET bytes = 0");
    assert_eq!(dml["ok"], true);
    assert!(
        dml["body"]["error"].as_str().unwrap().contains("hist"),
        "the refusal teaches: {dml}"
    );

    // One execution path (NO-SANDBOX ruling): the wire serves exactly what
    // the CLI lane serves — an external file read answers rows, not a
    // refusal (the accepted trust posture; every caller already holds a
    // shell).
    let external = conn.sql(6, "SELECT count(*) FROM read_csv('/etc/hosts')");
    assert_eq!(external["ok"], true);
    assert!(
        external["body"]["error"].is_null(),
        "no lane-level refusal survives the ruling: {external}"
    );
    assert_eq!(external["body"]["row_count"], 1, "{external}");
}

/// v2 sessions never see the op (§3.2 discovery honesty): not in v2 caps,
/// and a v2 frame answers `unknown_op`.
#[test]
fn v2_session_answers_unknown_op_and_never_advertises_sql() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ws");
    write(&ws, "a.md", "# A\n");

    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut v2 = Conn::open(server.socket_path());
    let hi = v2.call(&json!({
        "op": "hello", "proto": 1,
        "workspace": ws.to_str().unwrap(),
    }));
    assert_eq!(hi["ok"], true, "v2 hello: {hi}");
    let caps: Vec<&str> = hi["body"]["caps"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        !caps.contains(&"sql"),
        "frozen v2 caps never grow: {caps:?}"
    );

    let refused = v2.sql(1, "SELECT 1");
    assert_eq!(refused["ok"], false);
    assert_eq!(refused["error"]["code"], "unknown_op", "{refused}");
}

/// A statement with no I/O and no corpus in it, whose only job is to occupy
/// one `sql` call for long enough that a sibling's whole round trip fits
/// inside it. DuckDB has no `sleep`, so the spin is a recursive CTE.
///
/// The count is sized to be seconds, not milliseconds — a recursive CTE is
/// inherently serial, and this one is far more expensive per row than its
/// shape suggests: 20,000,000 cost 6.5 minutes of wall clock at 210% CPU on a
/// 30-core Linux box (debug build), so the row count and the runtime are NOT
/// intuitively related. The assertion below is a happens-BEFORE fact rather
/// than a duration, so what this constant has to buy is only "long enough for
/// a sibling's whole round trip to fit inside it, on the slowest box that will
/// ever run it" — and [`HOLDER_MUST_EXCEED`] fails the test rather than pass it
/// quietly if a future box, or a faster DuckDB, ever makes that false.
const SLOW_SQL: &str = "WITH RECURSIVE spin(i) AS (\
     SELECT 1 UNION ALL SELECT i + 1 FROM spin WHERE i < 300000\
     ) SELECT count(*)::BIGINT FROM spin";

/// The floor under [`SLOW_SQL`]'s own runtime. Below this the holder is not
/// meaningfully occupying anything and the gate proves nothing, so it must
/// FAIL rather than pass — an instrument that has quietly stopped
/// discriminating is the failure mode this whole card exists to correct.
const HOLDER_MUST_EXCEED: Duration = Duration::from_millis(1500);

/// A trivial real-corpus read — the shape a seat actually issues.
const REAL_SQL: &str = "SELECT path FROM doc ORDER BY path";

/// **The convoy gate.** A slow `sql` on one connection must not cost a sibling
/// connection on the SAME workspace its turn.
///
/// Every `sql` on a workspace passes through one `Mutex<SqlStore>`. While that
/// mutex was held across the caller's query, a fresh connection's `sql` on the
/// same workspace waited the slow query out — measured at 350,574 ms against a
/// 351,582 ms holder, while the same probe on a DIFFERENT workspace served in
/// 70 ms through the same daemon. `sql_op::serve` now releases the store after
/// the append and the read's `BEGIN`, so the query runs outside it.
///
/// **The other-workspace leg is the control, and it is here because it CAN
/// fail.** If it came back slow too, the finding would be a process-global
/// lock or a saturated box, not this mutex — and the first reading of this
/// defect was taken on `mounts`, which never reaches the workspace engine and
/// therefore could never have come back slow. A control that cannot fail is
/// what hid this the first time.
///
/// The verdict is `holder_in_flight`: a happens-before fact, not a threshold,
/// so it keeps discriminating whatever `SQL_STORE_WAIT` is set to and whatever
/// the box costs. Serialized, the sibling can only answer after the holder
/// releases — later than the holder's own answer, or refused `lock_timeout` at
/// the bound. Either way this fails.
#[test]
fn a_slow_sql_does_not_convoy_a_sibling_connection_on_the_same_workspace() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Instant;

    let tmp = TempDir::new().unwrap();
    let held = tmp.path().join("ws-held");
    let other = tmp.path().join("ws-other");
    write(&held, "a.md", "# A\n\nsee [[b]]\n");
    write(&held, "b.md", "# B\n");
    write(&other, "a.md", "# A\n");

    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let socket = server.socket_path().to_path_buf();

    // Warm BOTH projections first. A cold projection cache is built inline
    // inside the append, under this same mutex, so an unwarmed workspace would
    // let a first-use build masquerade as the convoy this gate is about.
    for ws in [&held, &other] {
        let mut warm = Conn::open(&socket);
        assert_eq!(warm.hello_v3(ws)["ok"], true);
        assert_eq!(warm.sql(1, REAL_SQL)["ok"], true, "warm {}", ws.display());
    }

    let done = Arc::new(AtomicBool::new(false));
    let holder = {
        let (socket, ws, done) = (socket.clone(), held.clone(), Arc::clone(&done));
        std::thread::spawn(move || {
            let mut conn = Conn::open(&socket);
            conn.hello_v3(&ws);
            let started = Instant::now();
            let answer = conn.sql(1, SLOW_SQL);
            done.store(true, Ordering::SeqCst);
            (started.elapsed(), answer)
        })
    };

    // Let the holder get past its pre-lock work (domain load, mount corpus,
    // base walk) and actually take the store, so the siblings below are
    // genuinely contended rather than merely concurrent.
    std::thread::sleep(Duration::from_millis(750));

    let mut sibling = Conn::open(&socket);
    assert_eq!(sibling.hello_v3(&held)["ok"], true);
    let started = Instant::now();
    let same_ws = sibling.sql(2, REAL_SQL);
    let same_elapsed = started.elapsed();
    let holder_in_flight = !done.load(Ordering::SeqCst);

    let mut control = Conn::open(&socket);
    assert_eq!(control.hello_v3(&other)["ok"], true);
    let started = Instant::now();
    let other_ws = control.sql(3, REAL_SQL);
    let other_elapsed = started.elapsed();
    let control_in_flight = !done.load(Ordering::SeqCst);

    let (holder_elapsed, holder_answer) = holder.join().unwrap();
    let ledger = format!(
        "holder {holder_elapsed:?} (ok={}), same-workspace sibling {same_elapsed:?}, \
         other-workspace control {other_elapsed:?}",
        holder_answer["ok"]
    );
    // Hidden unless the run asks (`--nocapture`): this gate is a measurement,
    // and the three numbers are worth having on a PASS, not only on a failure.
    eprintln!("convoy gate: {ledger}");

    // The instrument first. A gate whose "slow" statement stopped being slow
    // would pass for the wrong reason, forever, and look exactly like a fix.
    assert!(
        holder_elapsed > HOLDER_MUST_EXCEED,
        "SLOW_SQL ran in {holder_elapsed:?}, under the {HOLDER_MUST_EXCEED:?} \
         floor — the holder is not occupying the store long enough for this \
         gate to discriminate. Raise the row count; do not relax this. {ledger}"
    );

    // Then the control: if the instrument cannot serve a fast `sql` at all
    // while the holder runs, nothing below it means anything.
    assert_eq!(
        other_ws["ok"], true,
        "the other-workspace control refused — this reading cannot separate \
         the per-workspace mutex from a process-global lock or a saturated \
         box: {other_ws}. {ledger}"
    );
    assert!(
        control_in_flight,
        "the other-workspace control answered only after the slow query \
         finished, so the slow query is not what this gate thinks it is. \
         {ledger}"
    );

    assert_eq!(
        same_ws["ok"], true,
        "a sibling connection's sql on the held workspace was refused while \
         another connection's query ran: {same_ws}. {ledger}"
    );
    assert!(
        holder_in_flight,
        "the same-workspace sibling answered only after the holder's own \
         query returned — every seat on this workspace is still serialized \
         behind the slowest one. {ledger}"
    );
    assert_eq!(
        same_ws["body"]["rows"],
        json!([["a.md"], ["b.md"]]),
        "the sibling's answer is its own workspace's rows: {ledger}"
    );
}

/// The strict field wall: a `sql` frame carrying any field beyond `query`
/// refuses `bad_request` — cwd and row bounds are host concerns.
#[test]
fn sql_strict_field_wall_refuses_extras() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("ws");
    write(&ws, "a.md", "# A\n");

    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    conn.hello_v3(&ws);

    let refused = conn.call(&json!({
        "id": 1, "op": "sql", "query": "SELECT 1", "max_rows": 5,
    }));
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "bad_request");
}
