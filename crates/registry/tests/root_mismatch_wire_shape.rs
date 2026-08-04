//! **What a real `root_mismatch` actually serves** — read FROM THE WIRE.
//!
//! `docs/wire-contract-v2.md` promises `root_mismatch{expected,actual,changed}`
//! in three places (§5.1, the §8 table, §18 row 2). The engine has never served
//! `changed`, and the disposition is **`implemented-absent`** (leader ruling
//! 2026-08-04): the law survives, the proof premise is dead.
//!
//! # Why the field is unreachable, not merely unwired
//! The sole producer is `wire_serve::write::world_guard`, whose ENTIRE INPUT is
//! two root hashes — `if_root` and the ambient `root_before`. `changed` means
//! "the files that drifted under the plan" (`wire::ErrorBody::changed`), a set
//! difference between the CLIENT's corpus state and the current one. The engine
//! holds the current corpus and a HASH of the client's, and **a merkle root is
//! not invertible.** No amount of plumbing answers it, because plumbing is not
//! what is missing.
//!
//! The only route to an answer is retained history — and answering "what
//! drifted between your root and mine" for an arbitrary stale client root is a
//! claim about the interval between locks, which **ZT decision 19** forbids:
//! *"Engine does not have memory. It should not have. History is pin to git
//! when we lock. Anything between locks is not history."*
//!
//! Even via the delta ring it would be ABSENT EXACTLY WHEN THE CLIENT IS MOST
//! STALE — the case the field exists to serve. A feature that works only when
//! you do not need it is not a feature, and that holds even without decision 19.
//!
//! # Why this file exists rather than an edit
//! `wire/tests/contract_v2.rs::root_mismatch_scope_drop_deviation_fixture` is a
//! FROZEN worked-value fixture (§18 ZT valve) and is deliberately untouched. It
//! is also not a liar: its stated subject is the `scope` DROP and it is honest
//! about being a type-shape assertion — only the `changed` line inside it
//! implies a behaviour. A true assertion with a false neighbour sharing its
//! body. So: add a per-case wire-sourced assertion beside it, never loosen or
//! edit the frozen one.
//!
//! # The general law this file is an instance of (U27)
//! **An exhaustive key-set pin is only a WIRE detector if the value came FROM
//! the wire.** A pin over a hand-built value tests its own construction. This
//! one drives the real daemon over its real socket, so what it pins is what a
//! client receives.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use registry::{Config, RunningServer};
use serde_json::{Value, json};
use tempfile::TempDir;

const PLAN: &str = "# Goals\n\nship by August\n";

/// A root that is well-formed and is NOT this workspace's — the stale plan a
/// `root_mismatch` exists to refuse.
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

/// Drive a REAL `root_mismatch` out of the daemon and return its `error` object.
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

/// **THE PIN.** The exhaustive key set of a served `root_mismatch`.
///
/// Exhaustive, not `contains`: a subset check would pass while `changed`
/// appeared, which is the whole class this docket keeps finding.
///
/// The assertion that `changed` is ABSENT is the `implemented-absent` verdict
/// made executable. If someone later implements the field, this pin reddens and
/// sends them to the verdict and to decision 19 — which is what a verdict
/// should do, rather than sitting in a document nobody re-reads.
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

    // Named separately from the key set, because THIS is the verdict: the
    // absence is a fact about the engine, pinned from the engine's own output.
    assert!(
        error.get("changed").is_none(),
        "`changed` is specified in §5.1/§8/§18 row 2 and is unreachable by \
         construction — the world guard sees two hashes and a merkle root is \
         not invertible (U25 verdict: implemented-absent, ZT decision 19): {error}"
    );
}

/// The §8 binding, from the wire rather than from the type: `root_mismatch`
/// carries `recovery: resync` — a failed world guard invalidates the whole
/// plan, not one node's picture.
///
/// This is the control that keeps the pin above from passing vacuously. If the
/// harness ever stopped producing a real `root_mismatch` — a typo'd op, a
/// refusal from a neighbouring guard, a frame that never reached the engine —
/// the key set would collapse to something small and `changed` would be absent
/// for a reason that has nothing to do with the verdict. Asserting the
/// code/recovery pair from the SAME served frame is what distinguishes "the
/// engine served a `root_mismatch` without `changed`" from "no `root_mismatch` was
/// served at all".
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

/// **THE INSTRUMENT CONTROL — can this pin SEE a `changed` field at all?**
///
/// The pin above asserts an ABSENCE, and an absence assertion is worthless
/// unless the instrument can produce the presence (All-Hands #3, and the
/// docket's standing rule: before you trust a negative result, prove the
/// instrument can produce a positive one).
///
/// The two worlds:
/// - **as served** — key set `{actual, code, expected, recovery}`.
/// - **the same frame with `changed` present** — key set gains `changed`.
///
/// Their outputs DIFFER, and they differ by exactly the field under test. So a
/// `changed` that appeared on the wire tomorrow would redden the pin rather
/// than slip past it.
///
/// **What this control is NOT.** It is not a production mutation. The true
/// mutation — making `world_guard` set the field — lives in
/// `crates/wire-serve/src/write.rs`, which is a serialized-car file this worker
/// is not cleared to touch, so it was not run. This control proves the
/// ASSERTION distinguishes the two worlds; arm A coming from the real socket is
/// what proves the pin is wired to the engine. Together they cover what a
/// production mutation would have shown, minus the proof that `world_guard`
/// itself is the only producer — which is established by reading instead
/// (`write.rs::world_guard` and `model::splice_verdict` are the two sites, and
/// both take two roots and set exactly `expected`/`actual`).
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
