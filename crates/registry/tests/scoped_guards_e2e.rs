//! Served `scoped-guards` family — decode + mint + fold through the
//! daemon socket (fp-grain PR-4).
//!
//! Acceptance is the ruling's dogfood shape: capture the FILE's token,
//! birth a disjoint sibling, put the untouched file pinned on that token
//! — commits. A write that moves the planned scope still refuses,
//! naming the scope.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use registry::{Config, RunningServer};
use serde_json::{Value, json};
use tempfile::TempDir;

mod common;
const TARGET: &str = "# Target\n\nbody line one.\n";

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
    config
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

    fn hello_v3(&mut self, ws: &Path) -> Value {
        self.call(&json!({
            "op": "hello", "proto": 1, "contract": "v3",
            "workspace": ws.to_str().unwrap(),
        }))
    }
}

fn seeded(tmp: &TempDir) -> PathBuf {
    let ws = tmp.path().join("ws");
    std::fs::create_dir_all(ws.join("a")).unwrap();
    std::fs::write(ws.join("a/target.md"), TARGET).unwrap();
    ws
}

fn edit() -> Value {
    json!([{
        "target": {"hpath": [{"h": "Target"}]},
        "edit": {"match": {"old": "body line one.", "new": "body line two."}}
    }])
}

#[test]
fn v3_hello_advertises_scoped_guards_and_v2_never_does() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());

    let v3 = conn.hello_v3(&ws);
    assert_eq!(v3["ok"], json!(true), "{v3}");
    let caps: Vec<&str> = v3["body"]["caps"]
        .as_array()
        .expect("caps")
        .iter()
        .map(|c| c.as_str().expect("cap"))
        .collect();
    assert!(
        caps.contains(&"scoped-guards"),
        "v3 advertises the family: {caps:?}"
    );

    let mut v2 = Conn::open(server.socket_path());
    let hi = v2.call(&json!({
        "op": "hello", "proto": 1,
        "workspace": ws.to_str().unwrap(),
    }));
    let v2_caps: Vec<&str> = hi["body"]["caps"]
        .as_array()
        .expect("caps")
        .iter()
        .map(|c| c.as_str().expect("cap"))
        .collect();
    assert!(
        !v2_caps.contains(&"scoped-guards"),
        "frozen v2 is never pushed the family: {v2_caps:?}"
    );
    server.shutdown();
}

#[test]
fn scoped_mint_echoes_the_scope_and_absent_is_a_value() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    assert_eq!(conn.hello_v3(&ws)["ok"], json!(true));

    let minted = conn.call(&json!({
        "op": "fingerprint", "scope": "a/target.md"
    }));
    assert_eq!(minted["ok"], json!(true), "{minted}");
    let token = minted["body"]["fingerprint"].as_str().expect("token");
    assert!(
        token.starts_with("b3"),
        "a live file mints a Root-family token: {token}"
    );
    assert_ne!(token, "absent");
    assert_eq!(
        minted["body"]["scope"],
        json!("a/target.md"),
        "the mint echoes its scope (§4.7 desync guard): {minted}"
    );

    let missing = conn.call(&json!({
        "op": "fingerprint", "scope": "no/such.md"
    }));
    assert_eq!(missing["ok"], json!(true), "{missing}");
    assert_eq!(
        missing["body"]["fingerprint"],
        json!("absent"),
        "a lawful empty path mints absent, never scope_unresolved: {missing}"
    );
    assert_eq!(missing["body"]["scope"], json!("no/such.md"));
    server.shutdown();
}

/// The ruling's acceptance shape through the served wire.
#[test]
fn a_file_premise_survives_a_disjoint_sibling_birth() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    assert_eq!(conn.hello_v3(&ws)["ok"], json!(true));

    let minted = conn.call(&json!({
        "op": "fingerprint", "scope": "a/target.md"
    }));
    let token = minted["body"]["fingerprint"].as_str().unwrap();

    std::fs::create_dir_all(ws.join("b")).unwrap();
    std::fs::write(ws.join("b/mover.md"), "# Mover\n\ndisjoint.\n").unwrap();

    let put = conn.call(&json!({
        "op": "splice",
        "path": "a/target.md",
        "if_fingerprint": token,
        "scope": "a/target.md",
        "edits": edit(),
    }));
    assert_eq!(
        put["ok"],
        json!(true),
        "a disjoint sibling must not refuse a file-scoped plan: {put}"
    );
    let bytes = std::fs::read_to_string(ws.join("a/target.md")).unwrap();
    assert!(
        bytes.contains("body line two."),
        "the edit reached disk: {bytes}"
    );
    server.shutdown();
}

/// The counter-proof: the same shape refuses when the planned scope moved.
#[test]
fn a_file_premise_refuses_when_the_scope_itself_moved() {
    let tmp = TempDir::new().unwrap();
    let ws = seeded(&tmp);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    assert_eq!(conn.hello_v3(&ws)["ok"], json!(true));

    let minted = conn.call(&json!({
        "op": "fingerprint", "scope": "a/target.md"
    }));
    let token = minted["body"]["fingerprint"].as_str().unwrap();

    std::fs::write(
        ws.join("a/target.md"),
        format!("{TARGET}\nout-of-band tail.\n"),
    )
    .unwrap();

    let put = conn.call(&json!({
        "op": "splice",
        "path": "a/target.md",
        "if_fingerprint": token,
        "scope": "a/target.md",
        "edits": edit(),
    }));
    assert_eq!(put["ok"], json!(false), "the premise moved: {put}");
    assert_eq!(put["error"]["code"], json!("fingerprint_mismatch"), "{put}");
    assert_eq!(
        put["error"]["scope"],
        json!("a/target.md"),
        "a SCOPED mismatch names its premise (§5.7): {put}"
    );
    let message = put["error"]["message"].as_str().unwrap_or("");
    assert!(
        message.contains("the premise at a/target.md moved"),
        "the §8.2 register text speaks: {message}"
    );
    server.shutdown();
}
