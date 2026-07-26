//! E2E gates for the resident-engine handshake (`[[0002-resident-daemon]]` §4,
//! U3). One `hello` round trip asserts the contract rev (single v3), resolves
//! the workspace (the ancestor walk, folded in from `Registry::resolve`), pins
//! its storage (the canonicalize → deny-ceiling → sentinel path, reused — R2),
//! warms its resident engine, binds the connection, and lists the served caps.
//! An unknown declared rev is a loud refusal. `hello` subsumes the deleted
//! `attach` op — no parallel binding path (§5).

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use registry::{Config, RunningServer};
use serde_json::{Value, json};
use tempfile::TempDir;

/// A daemon config rooted under `tmp`, with reap horizons large enough that the
/// background reaper never evicts a warm engine mid-test.
// `Duration::from_hours` is not const-stable at MSRV 1.96; the seconds form is
// the workspace precedent.
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

/// A workspace `tmp/<name>` seeded with `files` — a sibling of the cache root,
/// so the corpus walk never sees the drawer.
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

    /// A v3 hello binding `ws`.
    fn hello(&mut self, ws: &Path) -> Value {
        self.call(&json!({
            "op": "hello",
            "proto": 1,
            "contract": "v3",
            "workspace": ws.to_str().unwrap(),
        }))
    }
}

/// The U3 spine gate: one `hello` resolves the workspace, pins storage,
/// negotiates v3, and lists the served caps — one round trip.
#[test]
fn hello_resolves_pins_negotiates_and_lists_caps_in_one_round_trip() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(
        &tmp,
        "ws",
        &[("a.md", "# A\n\nsee [[b]]\n"), ("b.md", "# B\n")],
    );
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    let hi = conn.hello(&ws);
    assert_eq!(hi["ok"], json!(true), "hello ok: {hi}");
    let body = &hi["body"];

    // Rev negotiated: proto 1, a named server.
    assert_eq!(body["proto"], json!(1), "hello negotiates proto 1: {hi}");
    assert!(body["server"].is_string(), "hello names the server: {hi}");

    // Caps listed — and honest: the served read ops AND the served write op
    // (splice, W1) are present; the unserved push op (sub, P2) and `hello` itself
    // are absent (§3.2 discovery honesty). This is a v3 session, so the root
    // capability is spelled `fingerprint` (never `root` — the amendment's rule).
    let caps: Vec<String> = body["caps"]
        .as_array()
        .expect("caps array")
        .iter()
        .map(|c| c.as_str().unwrap().to_string())
        .collect();
    for served in [
        "toc",
        "cat",
        "extract",
        "resolve",
        "links",
        "fingerprint",
        "diff",
        "splice",
        // S3 U1 (R23): the served write op's v3-era sibling FIELDS are listed
        // too, as dotted `op.field` caps — discovery honesty at field grain,
        // not just op grain.
        "splice.plan_edits",
        "splice.pin",
    ] {
        assert!(
            caps.contains(&served.to_string()),
            "caps list `{served}`: {hi}"
        );
    }
    for absent in ["hello", "sub", "root"] {
        assert!(
            !caps.contains(&absent.to_string()),
            "caps must not list `{absent}` (§3.2 discovery honesty; v3 hides `root`): {hi}"
        );
    }

    // Storage pinned: the drawer sits under this daemon's cache root...
    let storage = body["storage"].as_str().expect("hello pins storage");
    let cache_root = tmp.path().join("cache");
    assert!(
        Path::new(storage).starts_with(&cache_root),
        "the pinned drawer is under the cache root {cache_root:?}: {storage}"
    );
    // ...and the pin registered the workspace — admin `list` now reports it (R2:
    // the same canonicalize → deny → sentinel path the admin `register` drives).
    let listed = conn.call(&json!({"op": "list"}));
    assert_eq!(
        listed["entries"].as_array().map(Vec::len),
        Some(1),
        "hello's storage pin registered the workspace: {listed}"
    );

    // The bind is live: a read op on the SAME connection serves from the warm
    // engine without any separate attach.
    let toc = conn.call(&json!({"op": "toc", "path": "a.md"}));
    assert_eq!(
        toc["ok"],
        json!(true),
        "bound read serves from the engine: {toc}"
    );
    assert_eq!(toc["body"]["path"], json!("a.md"));

    server.shutdown();
}

/// An unknown DECLARED contract rev is refused LOUD at the handshake — never a
/// silent downgrade (the v3-amendment negotiation law).
#[test]
fn hello_with_an_unknown_rev_is_a_loud_refusal() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, "ws", &[("a.md", "# A\n")]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    let refused = conn.call(&json!({
        "op": "hello",
        "proto": 1,
        "contract": "v9",
        "workspace": ws.to_str().unwrap(),
    }));
    assert_eq!(
        refused["ok"],
        json!(false),
        "unknown rev refused: {refused}"
    );
    assert_eq!(
        refused["error"]["code"],
        json!("bad_request"),
        "an unknown declared rev is bad_request: {refused}"
    );
    assert_eq!(
        refused["error"]["recovery"],
        json!("fix"),
        "bad_request recovers by fix: {refused}"
    );
    assert!(
        refused["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("v9")),
        "the refusal names the bad rev: {refused}"
    );
}

/// `hello` folds `Registry::resolve`'s ancestor walk: a hello from a SUBDIR of
/// an already-pinned workspace binds to (and pins) the registered ancestor, not
/// a second nested workspace. The storage pin is the ancestor's drawer, and the
/// registry still holds exactly one entry.
#[test]
fn hello_folds_the_ancestor_walk_resolve() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(
        &tmp,
        "ws",
        &[("a.md", "# A\n\nsee [[b]]\n"), ("b.md", "# B\n")],
    );
    fs::create_dir_all(ws.join("sub/deep")).unwrap();
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    // Pin the workspace root.
    let root_pin = conn.hello(&ws);
    let root_storage = root_pin["body"]["storage"]
        .as_str()
        .expect("root hello pins storage")
        .to_string();

    // Hello from a subdir resolves UP to the registered ancestor: same drawer.
    let sub_pin = conn.hello(&ws.join("sub/deep"));
    assert_eq!(
        sub_pin["body"]["storage"].as_str(),
        Some(root_storage.as_str()),
        "a subdir hello pins the ANCESTOR's drawer (ancestor walk): {sub_pin}"
    );

    // No nested registration: the ancestor walk kept the registry at one entry.
    let listed = conn.call(&json!({"op": "list"}));
    assert_eq!(
        listed["entries"].as_array().map(Vec::len),
        Some(1),
        "the ancestor walk avoids a nested registration: {listed}"
    );

    // The subdir connection is bound to the ancestor's corpus.
    let links = conn.call(&json!({"op": "links", "path": "a.md"}));
    assert_eq!(
        links["body"]["files"]["a.md"]["resolved"]["b.md"],
        json!(1),
        "the bound corpus is the ancestor's: {links}"
    );

    server.shutdown();
}

/// A workspace-less `hello` is a pure version handshake: it negotiates the rev
/// and lists caps, but binds and pins nothing — no `storage`, no `root`, and a
/// following read op is refused for want of a binding.
#[test]
fn workspace_less_hello_is_a_pure_version_handshake() {
    let tmp = TempDir::new().unwrap();
    write_ws(&tmp, "ws", &[("a.md", "# A\n")]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    let hi = conn.call(&json!({"op": "hello", "proto": 1, "contract": "v3"}));
    assert_eq!(hi["ok"], json!(true), "bare hello ok: {hi}");
    assert_eq!(hi["body"]["proto"], json!(1));
    assert!(
        !hi["body"]["caps"].as_array().unwrap().is_empty(),
        "a bare hello still lists caps: {hi}"
    );
    assert!(
        hi["body"]["storage"].is_null(),
        "a workspace-less hello pins nothing: {hi}"
    );
    // v3 session: the binding cursor is spelled `fingerprint`, and it is absent
    // (nothing bound) — never `root`.
    assert!(
        hi["body"]["fingerprint"].is_null(),
        "a workspace-less hello binds nothing: {hi}"
    );
    assert!(
        hi["body"]["root"].is_null(),
        "a v3 hello never spells `root`: {hi}"
    );

    // Nothing bound → a read op is refused loudly.
    let toc = conn.call(&json!({"op": "toc", "path": "a.md"}));
    assert_eq!(toc["ok"], json!(false), "unbound read is refused: {toc}");

    // And it registered nothing.
    let listed = conn.call(&json!({"op": "list"}));
    assert_eq!(
        listed["entries"].as_array().map(Vec::len),
        Some(0),
        "a bare hello registers no workspace: {listed}"
    );

    server.shutdown();
}
