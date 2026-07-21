//! The daemon-socket vocabulary gate (`docs/wire-contract-v3-amendment.md`),
//! executable end-to-end over the resident daemon's unified socket.
//!
//! The amendment's hard rule, on the daemon socket this time: a `hello` session
//! that negotiated `contract:v3` emits `fingerprint` and NEVER `root` — in every
//! answered class (`hello` caps + binding, `root`/`fingerprint`, `toc`, `links`,
//! `diff`, `splice`) — while a `v2` (or un-negotiated) session emits `root` and
//! NEVER `fingerprint`, byte-for-byte the frozen contract. One epoch, one rev.
//!
//! This closes the U2/U3 gap: the wire ops were unified onto the daemon socket
//! without carrying the per-session rev projection across, so the socket served
//! `root` even to a v3 client. The projection now lives in `wire-serve` and both
//! hosts drive it — this gate proves it on the daemon socket.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use registry::{Config, RunningServer};
use serde_json::{Value, json};
use tempfile::TempDir;

/// A daemon config rooted under `tmp`, reap horizons large enough that the
/// background reaper never evicts a warm engine mid-test.
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

/// A workspace `tmp/<name>` seeded with `files` — a sibling of the cache root, so
/// the corpus walk never sees the drawer.
fn write_ws(tmp: &TempDir, name: &str, files: &[(&str, &str)]) -> PathBuf {
    let ws = tmp.path().join(name);
    for (rel, content) in files {
        let path = ws.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }
    ws
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
        let mut line = serde_json::to_string(request).unwrap();
        line.push('\n');
        self.writer.write_all(line.as_bytes()).unwrap();
        self.writer.flush().unwrap();
        let mut response = String::new();
        self.reader.read_line(&mut response).unwrap();
        serde_json::from_str(&response).unwrap()
    }

    /// A `hello` binding `ws` under the given `contract` (`None` = un-negotiated,
    /// the frozen-v2 default).
    fn hello(&mut self, ws: &Path, contract: Option<&str>) -> Value {
        let mut frame = json!({
            "op": "hello",
            "proto": 1,
            "workspace": ws.to_str().unwrap(),
        });
        if let Some(rev) = contract {
            frame["contract"] = json!(rev);
        }
        self.call(&frame)
    }
}

const PLAN: &str = "# Goals\n\nsee [[b]]\n\nship by August\n";

/// The v2-root vocabulary keys — none may appear as an object KEY (recursively)
/// in a v3 frame, and every one of them must appear (where applicable) in a v2
/// frame.
const ROOT_KEYS: &[&str] = &[
    "root",
    "root_before",
    "root_after",
    "as_of_root",
    "live_root",
    "if_root",
    "from_root",
    "to_root",
    "require_root",
];

/// The v3-fingerprint vocabulary keys — the mirror image.
const FINGERPRINT_KEYS: &[&str] = &[
    "fingerprint",
    "fingerprint_before",
    "fingerprint_after",
    "as_of_fingerprint",
    "live_fingerprint",
];

/// Recursively collect every object KEY in `v` (values are never collected — a
/// corpus path or linkpath that spells "root" is a legitimate VALUE and must
/// never be re-keyed).
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

fn all_keys(frame: &Value) -> Vec<String> {
    let mut out = Vec::new();
    keys(frame, &mut out);
    out
}

/// The `hello` caps as owned strings.
fn caps(frame: &Value) -> Vec<String> {
    frame["body"]["caps"]
        .as_array()
        .expect("caps array")
        .iter()
        .map(|c| c.as_str().unwrap().to_string())
        .collect()
}

/// The primary gate: a v3-negotiated session emits `fingerprint` in every class
/// and ZERO `root` tokens — read frames (`toc`/`root`/`links`/`diff`), the write
/// frame (`splice`), and the `hello` caps + binding.
#[test]
fn v3_session_emits_fingerprint_and_zero_root_tokens() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, "ws", &[("plan.md", PLAN), ("b.md", "# B\n")]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    // hello: caps carry the fingerprint spelling, never root; the binding echoes
    // `fingerprint` (not `root`) and the negotiated `contract:v3`.
    let hi = conn.hello(&ws, Some("v3"));
    assert_eq!(hi["ok"], json!(true), "v3 hello ok: {hi}");
    assert_eq!(
        hi["body"]["contract"],
        json!("v3"),
        "v3 echoes contract: {hi}"
    );
    assert!(
        hi["body"]["fingerprint"].is_string(),
        "the v3 hello binding is `fingerprint`, not `root`: {hi}"
    );
    let hi_caps = caps(&hi);
    for want in [
        "fingerprint",
        "splice.if_fingerprint",
        "links.require_fingerprint",
    ] {
        assert!(
            hi_caps.contains(&want.to_string()),
            "v3 caps carry `{want}`: {hi}"
        );
    }
    for forbidden in ["root", "splice.if_root", "links.require_root"] {
        assert!(
            !hi_caps.contains(&forbidden.to_string()),
            "v3 caps must NOT carry `{forbidden}`: {hi}"
        );
    }

    // Every answered wire class, driven in the v3 vocabulary.
    let toc = conn.call(&json!({"op": "toc", "path": "plan.md"}));
    assert_eq!(toc["ok"], json!(true), "toc ok: {toc}");
    assert!(
        toc["body"]["fingerprint"].is_string(),
        "toc carries the ambient `fingerprint`: {toc}"
    );

    // `root` op in the v3 spelling is `fingerprint`; the response body is too.
    let fp = conn.call(&json!({"op": "fingerprint"}));
    assert_eq!(fp["ok"], json!(true), "fingerprint op ok: {fp}");
    assert!(
        fp["body"]["fingerprint"].is_string(),
        "the fingerprint op answers a `fingerprint` cursor: {fp}"
    );
    let cursor = fp["body"]["fingerprint"].as_str().unwrap().to_string();

    let links = conn.call(&json!({"op": "links", "path": "plan.md"}));
    assert_eq!(links["ok"], json!(true), "links ok: {links}");
    assert!(
        links["body"]["as_of_fingerprint"].is_string()
            && links["body"]["live_fingerprint"].is_string(),
        "links carries the fingerprint staleness triple: {links}"
    );
    assert_eq!(
        links["body"]["files"]["plan.md"]["resolved"]["b.md"],
        json!(1),
        "links still resolves [[b]] → b.md under v3: {links}"
    );

    // diff in the v3 request spelling (`from_fingerprint`/`to_fingerprint`).
    let diff = conn.call(&json!({
        "op": "diff", "from_fingerprint": cursor, "to_fingerprint": cursor,
    }));
    assert_eq!(diff["ok"], json!(true), "same-cursor diff ok: {diff}");
    assert_eq!(diff["body"]["batches"], json!([]), "diff empty: {diff}");

    // splice: the write frame carries `fingerprint_before`/`fingerprint_after`.
    let splice = conn.call(&json!({
        "id": 7,
        "op": "splice",
        "path": "plan.md",
        "edits": [{
            "target": {"hpath": [{"h": "Goals"}]},
            "edit": {"match": {"old": "ship by August", "new": "ship by September"}},
        }],
    }));
    assert_eq!(splice["ok"], json!(true), "splice ok: {splice}");
    assert!(
        splice["body"]["fingerprint_before"].is_string()
            && splice["body"]["fingerprint_after"].is_string(),
        "splice carries the fingerprint transition: {splice}"
    );

    // The hard rule, mechanized: ZERO root-vocabulary KEYS in ANY v3 frame.
    for frame in [&hi, &toc, &fp, &links, &diff, &splice] {
        let frame_keys = all_keys(frame);
        for root_key in ROOT_KEYS {
            assert!(
                !frame_keys.iter().any(|k| k == root_key),
                "a v3 frame must emit ZERO `{root_key}` keys: {frame}"
            );
        }
    }

    server.shutdown();
}

/// The mirror gate: a `v2` (declared) session emits `root` and NEVER
/// `fingerprint`, with no `contract` echo — byte-for-byte the frozen contract.
#[test]
fn v2_session_emits_root_and_never_fingerprint() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, "ws", &[("plan.md", PLAN), ("b.md", "# B\n")]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    let hi = conn.hello(&ws, Some("v2"));
    assert_eq!(hi["ok"], json!(true), "v2 hello ok: {hi}");
    assert!(
        hi["body"].get("contract").is_none(),
        "a v2 hello echoes NO contract (frozen shape): {hi}"
    );
    assert!(
        hi["body"]["root"].is_string(),
        "the v2 hello binding is `root`: {hi}"
    );
    let hi_caps = caps(&hi);
    for want in ["root", "splice.if_root", "links.require_root"] {
        assert!(
            hi_caps.contains(&want.to_string()),
            "v2 caps carry `{want}`: {hi}"
        );
    }

    let toc = conn.call(&json!({"op": "toc", "path": "plan.md"}));
    let root = conn.call(&json!({"op": "root"}));
    let links = conn.call(&json!({"op": "links", "path": "plan.md"}));
    assert!(
        root["body"]["root"].is_string(),
        "root op answers `root`: {root}"
    );
    assert!(
        links["body"]["as_of_root"].is_string() && links["body"]["live_root"].is_string(),
        "links carries the root staleness triple: {links}"
    );

    let splice = conn.call(&json!({
        "id": 7,
        "op": "splice",
        "path": "plan.md",
        "edits": [{
            "target": {"hpath": [{"h": "Goals"}]},
            "edit": {"match": {"old": "ship by August", "new": "ship by September"}},
        }],
    }));
    assert!(
        splice["body"]["root_before"].is_string() && splice["body"]["root_after"].is_string(),
        "splice carries the root transition: {splice}"
    );

    // ZERO fingerprint-vocabulary KEYS in ANY v2 frame.
    for frame in [&hi, &toc, &root, &links, &splice] {
        let frame_keys = all_keys(frame);
        for fp_key in FINGERPRINT_KEYS {
            assert!(
                !frame_keys.iter().any(|k| k == fp_key),
                "a v2 frame must emit ZERO `{fp_key}` keys: {frame}"
            );
        }
    }

    server.shutdown();
}

/// Byte-identical, mechanized: v3 is EXACTLY v2 re-keyed — for the read ops that
/// carry the cursor, a v2 connection and a v3 connection against the SAME warm
/// engine answer the same DATA under the two vocabularies. The v2 `root` value
/// equals the v3 `fingerprint` value, and neither leaks the other's spelling.
#[test]
fn v3_is_exactly_v2_rekeyed_over_the_same_engine() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, "ws", &[("plan.md", PLAN), ("b.md", "# B\n")]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    let mut v2 = Conn::open(server.socket_path());
    let mut v3 = Conn::open(server.socket_path());
    v2.hello(&ws, Some("v2"));
    v3.hello(&ws, Some("v3"));

    // The `root`/`fingerprint` cursor op: same seq, same hash value, mirrored key.
    let r2 = v2.call(&json!({"op": "root"}));
    let r3 = v3.call(&json!({"op": "fingerprint"}));
    assert_eq!(
        r2["body"]["root"], r3["body"]["fingerprint"],
        "the v2 root value equals the v3 fingerprint value: {r2} vs {r3}"
    );
    assert_eq!(
        r2["body"]["seq"], r3["body"]["seq"],
        "seq is vocabulary-neutral"
    );
    assert!(
        r2["body"]["fingerprint"].is_null(),
        "v2 cursor never spells fingerprint: {r2}"
    );
    assert!(
        r3["body"]["root"].is_null(),
        "v3 cursor never spells root: {r3}"
    );

    // The `links` staleness triple: same values, mirrored keys.
    let l2 = v2.call(&json!({"op": "links", "path": "plan.md"}));
    let l3 = v3.call(&json!({"op": "links", "path": "plan.md"}));
    assert_eq!(
        l2["body"]["as_of_root"], l3["body"]["as_of_fingerprint"],
        "links as_of value is identical across the two vocabularies"
    );
    assert_eq!(
        l2["body"]["live_root"], l3["body"]["live_fingerprint"],
        "links live value is identical across the two vocabularies"
    );
    assert_eq!(
        l2["body"]["files"], l3["body"]["files"],
        "the corpus edge map is byte-identical (never re-keyed)"
    );

    server.shutdown();
}
