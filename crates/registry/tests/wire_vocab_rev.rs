//! The daemon-socket vocabulary gate (`docs/wire-contract.md`),
//! executable end-to-end over the resident daemon's unified socket.
//!
//! A `hello` session that negotiated `contract:v3` emits `fingerprint` and
//! never `root` — in every answered class (`hello` caps + binding,
//! `root`/`fingerprint`, `toc`, `links`, `diff`, `splice`) — while a `v2` (or
//! un-negotiated) session emits `root` and never `fingerprint`, byte-for-byte
//! the frozen contract. One epoch, one rev. The per-session rev projection
//! lives in `wire-serve`; this gate proves it on the daemon socket.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use registry::{Config, RunningServer};
use serde_json::{Value, json};
use tempfile::TempDir;

mod common;

/// A daemon config rooted under `tmp`, reap horizons large enough that the
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
    // Lifetime is the test's; idle-exit would flake mid-assertion.
    config.idle_exit = None;
    config.drain_cold_builds = Duration::from_secs(30);
    config
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

/// The v2-root vocabulary keys — none may appear as an object key (recursively)
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

/// Recursively collect every object key in `v` (values are never collected — a
/// corpus path or linkpath that spells "root" is a legitimate value and must
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
/// and zero `root` tokens — read frames (`toc`/`root`/`links`/`diff`), the write
/// frame (`splice`), and the `hello` caps + binding.
#[test]
fn v3_session_emits_fingerprint_and_zero_root_tokens() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, "ws", &[("plan.md", PLAN), ("b.md", "# B\n")]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    // hello: caps carry the fingerprint spelling, never root, and the
    // negotiated `contract:v3` is echoed. (The binding-cursor spelling is
    // asserted on a warm re-hello below — a cold hello carries no cursor at
    // all, §3.2 config-grade law.)
    let hi = conn.hello(&ws, Some("v3"));
    assert_eq!(hi["ok"], json!(true), "v3 hello ok: {hi}");
    assert_eq!(
        hi["body"]["contract"],
        json!("v3"),
        "v3 echoes contract: {hi}"
    );
    let hi_caps = caps(&hi);
    for want in [
        "fingerprint",
        "splice.if_fingerprint",
        "links.require_fingerprint",
        // v3-era splice amendments, advertised as dotted `op.field` caps.
        "splice.plan_edits",
        "splice.pin",
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

    // The toc warmed the engine; a re-hello now carries the binding cursor —
    // spelled `fingerprint`, never `root`.
    let rehi = conn.hello(&ws, Some("v3"));
    assert!(
        rehi["body"]["fingerprint"].is_string(),
        "the v3 hello binding is `fingerprint`, not `root`: {rehi}"
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
    // U10: a wire-origin content change carries its fingerprint — `if_node_rev`
    // is a frozen v2 field, so both vocabularies say the same thing here.
    let goals_rev = toc_node_rev(
        &conn.call(&json!({"op": "toc", "path": "plan.md"})),
        "Goals",
    );
    let splice = conn.call(&json!({
        "id": 7,
        "op": "splice",
        "path": "plan.md",
        "edits": [{
            "target": {"hpath": [{"h": "Goals"}]},
            "edit": {"match": {"old": "ship by August", "new": "ship by September"}},
            "if_node_rev": goals_rev,
        }],
    }));
    assert_eq!(splice["ok"], json!(true), "splice ok: {splice}");
    assert!(
        splice["body"]["fingerprint_before"].is_string()
            && splice["body"]["fingerprint_after"].is_string(),
        "splice carries the fingerprint transition: {splice}"
    );

    // The hard rule, mechanized: zero root-vocabulary keys in any v3 frame.
    for frame in [&hi, &rehi, &toc, &fp, &links, &diff, &splice] {
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

/// The mirror gate: a `v2` (declared) session emits `root` and never
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
    let hi_caps = caps(&hi);
    for want in ["root", "splice.if_root", "links.require_root"] {
        assert!(
            hi_caps.contains(&want.to_string()),
            "v2 caps carry `{want}`: {hi}"
        );
    }
    // Discovery honesty, the other direction: a v2 session refuses both v3-era
    // splice amendments at the field wall, so it must not advertise them.
    for forbidden in ["splice.pin", "splice.plan_edits"] {
        assert!(
            !hi_caps.contains(&forbidden.to_string()),
            "a v2 session refuses `{forbidden}`'s field, so v2 caps must NOT \
             advertise it: {hi}"
        );
    }

    let toc = conn.call(&json!({"op": "toc", "path": "plan.md"}));
    let root = conn.call(&json!({"op": "root"}));
    let links = conn.call(&json!({"op": "links", "path": "plan.md"}));
    assert!(
        root["body"]["root"].is_string(),
        "root op answers `root`: {root}"
    );
    // The toc warmed the engine; a re-hello now carries the binding cursor —
    // spelled `root` on the frozen v2 shape, never `fingerprint`. (A cold
    // hello carries no cursor at all: §3.2 config-grade law.)
    let rehi = conn.hello(&ws, Some("v2"));
    assert!(
        rehi["body"]["root"].is_string(),
        "the v2 hello binding is `root`: {rehi}"
    );
    assert!(
        rehi["body"].get("fingerprint").is_none(),
        "a v2 hello never spells `fingerprint`: {rehi}"
    );
    assert!(
        links["body"]["as_of_root"].is_string() && links["body"]["live_root"].is_string(),
        "links carries the root staleness triple: {links}"
    );

    // U10: a wire-origin content change carries its fingerprint — `if_node_rev`
    // is a frozen v2 field, so both vocabularies say the same thing here.
    let goals_rev = toc_node_rev(
        &conn.call(&json!({"op": "toc", "path": "plan.md"})),
        "Goals",
    );
    let splice = conn.call(&json!({
        "id": 7,
        "op": "splice",
        "path": "plan.md",
        "edits": [{
            "target": {"hpath": [{"h": "Goals"}]},
            "edit": {"match": {"old": "ship by August", "new": "ship by September"}},
            "if_node_rev": goals_rev,
        }],
    }));
    assert!(
        splice["body"]["root_before"].is_string() && splice["body"]["root_after"].is_string(),
        "splice carries the root transition: {splice}"
    );

    // Zero fingerprint-vocabulary keys in any v2 frame.
    for frame in [&hi, &rehi, &toc, &root, &links, &splice] {
        let frame_keys = all_keys(frame);
        for fp_key in FINGERPRINT_KEYS {
            assert!(
                !frame_keys.iter().any(|k| k == fp_key),
                "a v2 frame must emit ZERO `{fp_key}` keys: {frame}"
            );
        }
        // U7: the in-band timing block is a v3-only additive slot — the frozen
        // v2 contract never grows a `meta` key.
        assert!(
            !frame_keys.iter().any(|k| k == "meta" || k == "duration_us"),
            "a v2 frame must emit ZERO timing keys: {frame}"
        );
    }

    server.shutdown();
}

/// In-band timing on the daemon socket: a v3 session's dispatched frames —
/// reads, the write, and a refusal alike — carry `meta: {duration_us}` (integer
/// µs, the daemon measure point is `dispatch_read`). The `hello` handshake is
/// intercepted before the dispatch shell and carries none.
#[test]
fn v3_dispatch_frames_carry_meta_duration_us() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, "ws", &[("plan.md", PLAN), ("b.md", "# B\n")]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    let hi = conn.hello(&ws, Some("v3"));
    assert_eq!(hi["ok"], json!(true), "v3 hello ok: {hi}");
    assert!(
        hi.get("meta").is_none(),
        "hello is intercepted before dispatch_read — no timing: {hi}"
    );

    let toc = conn.call(&json!({"op": "toc", "path": "plan.md"}));
    let fp = conn.call(&json!({"op": "fingerprint"}));
    let missing = conn.call(&json!({"op": "toc", "path": "no/such/file.md"}));
    let splice = conn.call(&json!({
        "id": 9,
        "op": "splice",
        "path": "plan.md",
        "edits": [{
            "target": {"hpath": [{"h": "Goals"}]},
            "edit": {"match": {"old": "ship by August", "new": "ship by September"}},
        }],
    }));

    for (name, frame) in [
        ("toc", &toc),
        ("fingerprint", &fp),
        ("splice", &splice),
        ("refusal", &missing),
    ] {
        assert!(
            frame["meta"]["duration_us"].is_u64(),
            "v3 {name} frame carries meta.duration_us: {frame}"
        );
    }
    assert_eq!(
        missing["ok"],
        json!(false),
        "refusal is an error: {missing}"
    );

    server.shutdown();
}

/// A v3 `extract` enriches heading nodes with `n`/`words` and carries the
/// address as segments; a v2 `extract` against the same warm engine carries
/// zero such keys (frozen shape).
///
/// `hpath_text` is retired — the node carries the segment address that
/// round-trips into `put`. The v2 half still names `hpath_text` on purpose, so
/// it keeps failing if a future unit reintroduces the key.
#[test]
fn v3_extract_enriches_headings_v2_never() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(
        &tmp,
        "ws",
        &[("plan.md", "# Goals\n\nship by August\n\n## Q3\n\nx y z\n")],
    );
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    let mut v3 = Conn::open(server.socket_path());
    v3.hello(&ws, Some("v3"));
    let ex3 = v3.call(&json!({"op": "extract", "path": "plan.md"}));
    let heads: Vec<&Value> = ex3["body"]["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .filter(|n| n["kind"] == json!("heading"))
        .collect();
    assert_eq!(heads.len(), 2, "two headings: {ex3}");
    assert_eq!(heads[0]["n"], json!("1"));
    assert_eq!(heads[0]["hpath"], json!([{"h": "Goals"}]));
    assert!(
        heads[0].get("hpath_text").is_none(),
        "U14: the joined address is retired from this face: {ex3}"
    );
    assert_eq!(
        heads[0]["words"],
        json!(8),
        "subtree words incl ## Q3 tokens"
    );
    assert_eq!(heads[1]["n"], json!("1.1"));
    assert_eq!(heads[1]["hpath"], json!([{"h": "Goals"}, {"h": "Q3"}]));
    assert_eq!(heads[1]["words"], json!(3));

    let mut v2 = Conn::open(server.socket_path());
    v2.hello(&ws, Some("v2"));
    let ex2 = v2.call(&json!({"op": "extract", "path": "plan.md"}));
    for key in ["n", "hpath_text", "words"] {
        assert!(
            !all_keys(&ex2).iter().any(|k| k == key),
            "a v2 extract frame must emit ZERO `{key}` keys: {ex2}"
        );
    }

    server.shutdown();
}

/// Byte-identical, mechanized: v3 is exactly v2 re-keyed — for the read ops
/// that carry the cursor, a v2 connection and a v3 connection against the same
/// warm engine answer the same data under the two vocabularies. The v2 `root`
/// value equals the v3 `fingerprint` value, and neither leaks the other's
/// spelling.
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

/// One heading's live `node_rev` out of a `toc` frame — the fingerprint a
/// wire-origin write must carry. The v2 and v3 vocabularies spell the
/// surrounding frame differently; the per-node token is the same key in both.
fn toc_node_rev(toc: &Value, heading: &str) -> String {
    toc["body"]["nodes"]
        .as_array()
        .expect("toc nodes array")
        .iter()
        .find(|n| {
            n["hpath"]
                .as_array()
                .is_some_and(|h| h.last().is_some_and(|seg| seg["h"] == json!(heading)))
        })
        .unwrap_or_else(|| panic!("heading {heading} in toc: {toc}"))["node_rev"]
        .as_str()
        .expect("node_rev string")
        .to_string()
}
