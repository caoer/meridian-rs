//! The `contract_v3.rs` cases that had no daemon-socket twin, rehomed onto the
//! daemon's unix socket. Assertions re-derive their inputs from the live socket
//! session: cursors read off the wire, guards read off a `toc`.

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
/// background reaper never evicts a warm engine mid-test. Built by mutating the
/// production default so a new `Config` field cannot break this suite's compile.
#[allow(clippy::duration_suboptimal_units)]
fn test_config(tmp: &TempDir) -> Config {
    let forever = Duration::from_secs(365 * 24 * 60 * 60);
    let mut config = Config::for_cache_root(tmp.path().join("cache"));
    config.idle_threshold = forever;
    config.reap_interval = forever;
    config.prewarm_interval = forever;
    config.prewarm_quiet_max = forever;
    // No idle exit: a daemon that reaped itself mid-assertion would flake.
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

const PLAN: &str = "# Goals\n\nsee [[b]]\n\nship by August\n\n## Q3\n\nx y z\n";

const STALE: &str = "b3:0000000000000000000000000000000000000000000000000000000000000000";
const OTHER: &str = "b3:1111111111111111111111111111111111111111111111111111111111111111";

/// Recursively collect every object key in `v` (values are never collected — a
/// corpus path that spells "root" is a legitimate value).
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

/// One heading's live `node_rev` off a `toc` frame (U10: the guard a wire-origin
/// write must carry).
fn node_rev(toc: &Value, heading: &str) -> String {
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

// ---------------------------------------------------------------------------
// Replaces `contract_v3.rs::v3_error_codes_respell_root_family`
// ---------------------------------------------------------------------------

/// The v3 error vocabulary on the daemon socket: `root`-family refusal codes are
/// re-spelled to the `fingerprint` family, the recovery class is unchanged, and
/// the vocabulary-neutral extras (`expected`/`actual`) keep their names.
#[test]
fn v3_error_codes_respell_the_root_family_on_the_socket() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("plan.md", PLAN), ("b.md", "# B\n")]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    conn.hello(&ws, Some("v3"));

    let toc = conn.call(&json!({"op": "toc", "path": "plan.md"}));
    let q3 = node_rev(&toc, "Q3");

    // A write guarded on a fingerprint the world never had.
    let mismatch = conn.call(&json!({
        "op": "splice",
        "path": "plan.md",
        "if_fingerprint": STALE,
        "edits": [{
            "target": {"hpath": [{"h": "Goals"}, {"h": "Q3"}]},
            "edit": {"match": {"old": "x y z", "new": "a b c"}},
            "if_node_rev": q3,
        }],
    }));
    assert_eq!(
        mismatch["error"]["code"],
        json!("fingerprint_mismatch"),
        "a v3 session spells the whole-corpus guard failure in its own \
         vocabulary — `root_mismatch` is the v2 name for the same refusal: \
         {mismatch}"
    );
    assert_eq!(
        mismatch["error"]["recovery"],
        json!("resync"),
        "the recovery CLASS is vocabulary-neutral — a client ladder written \
         against v2 keeps working across the rename: {mismatch}"
    );
    assert!(
        mismatch["error"]["expected"].is_string(),
        "vocabulary-neutral extras keep their names: {mismatch}"
    );

    // A diff over a range this epoch never minted.
    let unknown = conn.call(&json!({
        "op": "diff", "from_fingerprint": STALE, "to_fingerprint": OTHER,
    }));
    assert_eq!(
        unknown["error"]["code"],
        json!("fingerprint_unknown"),
        "{unknown}"
    );
    assert_eq!(unknown["error"]["recovery"], json!("resync"), "{unknown}");

    // No `root`-family key or code spelling survives into a v3 refusal.
    for frame in [&mismatch, &unknown] {
        let raw = serde_json::to_string(frame).unwrap();
        assert!(
            !raw.contains("root_mismatch") && !raw.contains("root_unknown"),
            "a v3 refusal must not spell a root-family code: {raw}"
        );
        assert!(
            !all_keys(frame).iter().any(|k| k == "root"),
            "a v3 refusal carries no `root` key: {frame}"
        );
    }

    server.shutdown();
}

// ---------------------------------------------------------------------------
// Replaces `contract_v3.rs::v3_links_triple_and_require_knob`
// ---------------------------------------------------------------------------

/// The §10.1 staleness triple's request side under v3: `require_fingerprint` is
/// a real guard — met, it serves a view; stale, it refuses `stale_view` with all
/// three roots of the triple named in the fingerprint vocabulary.
#[test]
fn v3_links_require_fingerprint_is_a_real_guard() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("plan.md", PLAN), ("b.md", "# B\n")]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    assert_eq!(conn.hello(&ws, Some("v3"))["ok"], json!(true));
    // Hello binds at config cost (§3.2); the `fingerprint` op reads the live
    // cursor.
    let live = conn.call(&json!({"op": "fingerprint"}))["body"]["fingerprint"]
        .as_str()
        .expect("the fingerprint op carries the live cursor")
        .to_string();

    // Met: the demanded cursor is the world.
    let met = conn.call(&json!({
        "op": "links", "path": "plan.md", "require_fingerprint": live,
    }));
    assert_eq!(
        met["ok"],
        json!(true),
        "require_fingerprint met the world → a served view: {met}"
    );
    assert_eq!(met["body"]["as_of_fingerprint"], json!(live), "{met}");
    assert_eq!(met["body"]["live_fingerprint"], json!(live), "{met}");
    for dead in ["as_of_root", "live_root"] {
        assert!(
            !all_keys(&met).iter().any(|k| k == dead),
            "a v3 links view never spells `{dead}`: {met}"
        );
    }

    // Stale: a cursor from another world.
    let stale = conn.call(&json!({
        "op": "links", "path": "plan.md", "require_fingerprint": STALE,
    }));
    assert_eq!(stale["ok"], json!(false), "a stale demand refuses: {stale}");
    assert_eq!(stale["error"]["code"], json!("stale_view"), "{stale}");
    assert_eq!(stale["error"]["required"], json!(STALE), "{stale}");
    assert!(
        stale["error"]["as_of_fingerprint"].is_string()
            && stale["error"]["live_fingerprint"].is_string(),
        "the refusal names all three roots of the triple, re-spelled: {stale}"
    );
    for dead in ["as_of_root", "live_root", "require_root"] {
        assert!(
            !all_keys(&stale).iter().any(|k| k == dead),
            "a v3 stale_view never spells `{dead}`: {stale}"
        );
    }

    server.shutdown();
}

// ---------------------------------------------------------------------------
// Replaces the non-heading half of
// `contract_v3.rs::v3_extract_enriches_heading_nodes`
// ---------------------------------------------------------------------------

/// Enrichment is heading-scoped: a v3 `extract` adds `n`/`words` to heading
/// nodes and to nothing else. The positive and v2 halves live in
/// `wire_vocab_rev.rs::v3_extract_enriches_headings_v2_never`.
#[test]
fn v3_extract_leaves_non_heading_nodes_unenriched() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("plan.md", PLAN), ("b.md", "# B\n")]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    conn.hello(&ws, Some("v3"));

    let ex = conn.call(&json!({"op": "extract", "path": "plan.md"}));
    let nodes = ex["body"]["nodes"].as_array().expect("nodes");
    let non_headings: Vec<&Value> = nodes
        .iter()
        .filter(|n| n["kind"] != json!("heading"))
        .collect();
    assert!(
        !non_headings.is_empty(),
        "POSITIVE CONTROL: the fixture must produce non-heading nodes (it carries \
         a wikilink), or this test asserts nothing about a class it never saw: {ex}"
    );
    for n in non_headings {
        for key in ["n", "words"] {
            assert!(
                n.get(key).is_none(),
                "U2 enrichment is heading-scoped — a `{}` node must not carry \
                 `{key}`: {n}",
                n["kind"]
            );
        }
    }

    server.shutdown();
}

// ---------------------------------------------------------------------------
// Replaces `contract_v3.rs::explicit_v2_contract_is_the_frozen_vocabulary`
// ---------------------------------------------------------------------------

/// `contract:"v2"` ≡ `contract` absent: a v2 client that starts declaring its
/// rev sees no change at all.
#[test]
fn an_unnegotiated_session_is_exactly_the_declared_v2_session() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("plan.md", PLAN), ("b.md", "# B\n")]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    let mut declared = Conn::open(server.socket_path());
    let mut silent = Conn::open(server.socket_path());
    let hi_declared = declared.hello(&ws, Some("v2"));
    let hi_silent = silent.hello(&ws, None);
    assert_eq!(
        hi_declared["body"], hi_silent["body"],
        "the hello body is identical — including the ABSENCE of a `contract` \
         echo, which is what makes the declaration invisible: {hi_declared} vs \
         {hi_silent}"
    );

    for request in [
        json!({"op": "root"}),
        json!({"op": "toc", "path": "plan.md"}),
        json!({"op": "extract", "path": "plan.md"}),
        json!({"op": "links", "path": "plan.md"}),
        json!({"op": "cat", "path": "plan.md"}),
    ] {
        let a = declared.call(&request);
        let b = silent.call(&request);
        assert_eq!(
            a, b,
            "`contract:\"v2\"` and no contract are the SAME session — {request} \
             answered {a} vs {b}"
        );
    }

    server.shutdown();
}
