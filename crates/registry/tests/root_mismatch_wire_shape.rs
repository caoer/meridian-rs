//! Served `root_mismatch` wire shape — read from the daemon socket.
//!
//! Contract (`docs/wire-contract-v2.md` §5.1, §8, §18 row 2) names
//! `root_mismatch{expected,actual,changed}`. The engine serves `expected` and
//! `actual` only; `changed` is **implemented-absent**: `world_guard` sees two
//! root hashes, and a merkle root is not invertible. Serving "what drifted"
//! would need retained inter-lock history, which decision 19 forbids (engine
//! has no memory between locks).
//!
//! The frozen fixture in `wire/tests/contract_v2.rs` is left untouched (ZT
//! valve). This file pins the served shape from a real daemon over a real
//! socket (U27: an exhaustive key-set pin is a wire detector only when values
//! come from the wire).

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use registry::{Config, RunningServer};
use serde_json::{Value, json};
use tempfile::TempDir;

const PLAN: &str = "# Goals\n\nship by August\n";

/// Well-formed root that is not this workspace's — stale plan for `root_mismatch`.
const STALE_ROOT: &str = "b3:0000000000000000000000000000000000000000000000000000000000000000";

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
        // Lifetime is the test's; idle-exit would flake mid-assertion.
        idle_exit: None,
        push_write_timeout: registry::DEFAULT_PUSH_WRITE_TIMEOUT,
        // No build identity configured: this fixture is not testing the hello
        // identity field, and an absent sha is the honest state for a server
        // started from a test harness rather than a deployed binary.
        build_sha: None,
    }
}

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
}

/// Drive a real `root_mismatch` out of the daemon; return its `error` object.
fn served_root_mismatch(tmp: &TempDir) -> Value {
    let ws = tmp.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("plan.md"), PLAN).unwrap();
    let server = RunningServer::start(test_config(tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    let hello = conn.call(&json!({
        "op": "hello", "proto": 1, "workspace": ws.to_str().unwrap(),
    }));
    assert_eq!(hello["ok"], json!(true), "workspace binds: {hello}");

    let refused = conn.call(&json!({
        "op": "splice",
        "path": "plan.md",
        "if_root": STALE_ROOT,
        "edits": [{
            "target": {"hpath": [{"h": "Goals"}]},
            "edit": {"match": {"old": "ship by August", "new": "ship by September"}},
        }],
    }));
    server.shutdown();

    assert_eq!(
        refused["ok"],
        json!(false),
        "a stale plan is refused: {refused}"
    );
    assert_eq!(
        refused["error"]["code"],
        json!("root_mismatch"),
        "and it is refused as root_mismatch — the world guard, not a neighbour: {refused}"
    );
    refused["error"].clone()
}

/// Exhaustive key set of a served `root_mismatch` (not a subset check — that
/// would pass if `changed` appeared). `changed` absence is the
/// implemented-absent verdict, executable.
#[test]
fn a_served_root_mismatch_carries_expected_and_actual_and_no_changed() {
    let tmp = TempDir::new().unwrap();
    let error = served_root_mismatch(&tmp);

    let mut keys: Vec<&str> = error
        .as_object()
        .expect("the error is an object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["actual", "code", "expected", "recovery"],
        "the wire shape of a real root_mismatch: {error}"
    );

    // Named separately: absence is the engine fact pinned from its own output.
    assert!(
        error.get("changed").is_none(),
        "`changed` is specified in §5.1/§8/§18 row 2 and is unreachable by \
         construction — the world guard sees two hashes and a merkle root is \
         not invertible (U25 verdict: implemented-absent, ZT decision 19): {error}"
    );
}

/// §8 from the wire: `root_mismatch` carries `recovery: resync`. Also the
/// control that the pin above is not vacuous — code/recovery from the same
/// served frame proves a real world-guard refusal was produced.
#[test]
fn the_served_root_mismatch_is_a_real_one_carrying_its_ruled_recovery() {
    let tmp = TempDir::new().unwrap();
    let error = served_root_mismatch(&tmp);
    assert_eq!(error["code"], json!("root_mismatch"));
    assert_eq!(
        error["recovery"],
        json!("resync"),
        "§8: a failed world guard invalidates the plan — resync, not refresh: {error}"
    );
    assert!(
        error["expected"].is_string() && error["actual"].is_string(),
        "both comparison tokens are served, so the frame is the guard's own: {error}"
    );
}

/// Instrument control: the pin can distinguish a `changed` field from its
/// absence. Injects `changed` into the served frame (downstream of the engine)
/// so a future presence would redden the key-set pin. Not a production mutation
/// of `world_guard` — the pin already reads the real output path.
#[test]
fn the_pin_can_distinguish_a_changed_field_from_its_absence() {
    let tmp = TempDir::new().unwrap();
    let served = served_root_mismatch(&tmp);

    let key_set = |error: &Value| {
        let mut keys: Vec<String> = error.as_object().expect("object").keys().cloned().collect();
        keys.sort();
        keys
    };

    let as_served = key_set(&served);
    let mut leaked = served.clone();
    leaked
        .as_object_mut()
        .expect("object")
        .insert("changed".into(), json!(["plan.md"]));
    let with_changed = key_set(&leaked);

    assert_ne!(
        as_served, with_changed,
        "the instrument must tell the two worlds apart, or the absence it \
         reports is a decoration"
    );
    let extra: Vec<_> = with_changed
        .iter()
        .filter(|k| !as_served.contains(k))
        .collect();
    assert_eq!(
        extra,
        vec!["changed"],
        "and they differ by EXACTLY the field under test"
    );
}
