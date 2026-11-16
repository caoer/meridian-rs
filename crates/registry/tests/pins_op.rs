//! E2E gates for `pins` — the lock plane of one workspace over the wire
//! (`docs/wire-contract.md` § A.13): every declared pin coloured, the whole
//! pin population, fresh at the call. Read-only, workspace-bound, v3-only,
//! advertised as cap `pins`.
//!
//! The env-dependent lifecycle rides ONE test fn by design (edition 2024:
//! env mutation is unsafe, and `MERIDIAN_CONFIG` is process-global).

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
}

fn write(ws: &Path, rel: &str, body: &str) {
    let p = ws.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

/// One lock block carrying a whole-body pin per `(object, token)` row
/// (`path: []` = document root), rendered through the lock crate — the one
/// owner of the lock grammar. One block per page: a second block is the
/// lock-refused grey.
fn chain_block(pins: &[(&str, &str)]) -> String {
    let mut l = lock::Lock::new();
    for (object, token) in pins {
        l.upsert_pin(lock::PinEntry::new(
            object,
            "9ae3f1deadbeef",
            lock::Selector::Path(Vec::new()),
            token,
        ));
    }
    lock::render(&l)
}

/// The live whole-document fingerprint token for `raw` — what a green pin
/// carries.
fn live_token(raw: &str) -> String {
    let d = model::build(raw.to_string(), syntax::parse(raw));
    model::fingerprint::fingerprint(&d, &d.root)
        .expect("the fixture page has content")
        .into_string()
}

fn pins(frame: &Value) -> &Vec<Value> {
    frame["body"]["pins"]
        .as_array()
        .unwrap_or_else(|| panic!("pins array: {frame}"))
}

/// `(holder, object, color, reason)` of every row, in wire order.
fn summary(frame: &Value) -> Vec<(String, String, String, String)> {
    let text = |v: &Value| v.as_str().unwrap_or("").to_owned();
    pins(frame)
        .iter()
        .map(|row| {
            (
                text(&row["holder"]),
                text(&row["object"]),
                text(&row["color"]),
                text(&row["reason"]),
            )
        })
        .collect()
}

/// The § A.13 lifecycle, one env pin (`MERIDIAN_CONFIG`), one workspace: the
/// rows of an in-domain holder (ambient green, mounted green, unmounted grey)
/// and of a holder the hash domain excludes; the fingerprint of the served
/// generation; the excluded census; and the gate case — a same-size in-place
/// edit of a pinned target made right after the daemon observed it turns its
/// rows red on the next call.
#[test]
fn pins_lifecycle_population_colours_and_freshness() {
    let tmp = TempDir::new().unwrap();

    let other = tmp.path().join("other");
    let other_c = "# C\n\nleaf body\n";
    write(&other, "c.md", other_c);
    write(
        &other,
        "MERIDIAN.md",
        "---\ntype: meridian-root\nversion: 1\nname: other\n---\n",
    );

    let ws = tmp.path().join("ws");
    let b_raw = "# B\n\nthe pinned body v1\n";
    let a_raw = format!(
        "# A\n\n{}\n",
        chain_block(&[
            ("b", &live_token(b_raw)),
            ("ghost:z", "fp1.span2.b3.00000000"),
            ("other:c", &live_token(other_c)),
        ])
    );
    write(&ws, "a.md", &a_raw);
    write(&ws, "b.md", b_raw);
    write(
        &ws,
        "meridian/domain.md",
        "---\nversion: 1\nignore:\n  - \"bulk/**\"\n---\n",
    );
    write(
        &ws,
        "bulk/holder.md",
        &format!("# Out\n\n{}\n", chain_block(&[("b", &live_token(b_raw))])),
    );

    let config_path = tmp.path().join("conf").join("MERIDIAN.md");
    fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    fs::write(
        &config_path,
        format!(
            "---\ntype: meridian-config\nversion: 1\n---\n\n# Roots\n\n```meridian-mount\nname: other\npath: {}\nvault: other\n```\n",
            other.display()
        ),
    )
    .unwrap();
    unsafe { std::env::set_var("MERIDIAN_CONFIG", &config_path) };

    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    let hi = conn.hello_v3(&ws);
    assert_eq!(hi["ok"], json!(true), "v3 hello binds the workspace: {hi}");

    let first = conn.call(&json!({"id": 1, "op": "pins"}));
    assert_eq!(first["ok"], json!(true), "pins serves: {first}");
    let row = |holder: &str, object: &str, color: &str, reason: &str| {
        (
            holder.to_owned(),
            object.to_owned(),
            color.to_owned(),
            reason.to_owned(),
        )
    };
    assert_eq!(
        summary(&first),
        [
            row("a.md", "b", "green", ""),
            row("a.md", "ghost:z", "grey", "unmounted"),
            row("a.md", "other:c", "green", ""),
            row("bulk/holder.md", "b", "green", ""),
        ],
        "holder path order then lock order, the excluded holder last: {first}"
    );
    let green = &pins(&first)[0];
    assert_eq!(green["hash"], json!("9ae3f1deadbeef"), "{first}");
    assert!(green.get("selector").is_none(), "whole-file row: {first}");
    assert!(
        green.get("reason").is_none(),
        "green carries no reason: {first}"
    );
    assert_eq!(
        first["body"]["excluded"],
        json!(["bulk/holder.md"]),
        "{first}"
    );
    let root = conn.call(&json!({"id": 2, "op": "fingerprint"}));
    assert_eq!(
        first["body"]["fingerprint"], root["body"]["fingerprint"],
        "the served generation's fingerprint: {first} / {root}"
    );

    // The gate case: same size, in place, at once.
    fs::write(ws.join("b.md"), "# B\n\nthe pinned body v2\n").unwrap();
    let second = conn.call(&json!({"id": 3, "op": "pins"}));
    assert_eq!(
        summary(&second),
        [
            row("a.md", "b", "red", "content-drifted"),
            row("a.md", "ghost:z", "grey", "unmounted"),
            row("a.md", "other:c", "green", ""),
            row("bulk/holder.md", "b", "red", "content-drifted"),
        ],
        "an edit made before the call is in the answer: {second}"
    );
    assert_ne!(
        second["body"]["fingerprint"], first["body"]["fingerprint"],
        "{second}"
    );
}

/// v3 advertises `pins` at op grain; the frozen v2 caps never grow it, a v2
/// session's `pins` answers `unknown_op`, and the op takes no field.
#[test]
fn v3_advertises_pins_v2_answers_unknown_op_and_no_field_is_taken() {
    let tmp = TempDir::new().unwrap();
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let caps = |hi: &Value| -> Vec<String> {
        hi["body"]["caps"]
            .as_array()
            .expect("caps array")
            .iter()
            .map(|c| c.as_str().unwrap().to_owned())
            .collect()
    };

    let mut v3 = Conn::open(server.socket_path());
    let hi = v3.call(&json!({"op": "hello", "proto": 1, "contract": "v3"}));
    assert!(caps(&hi).contains(&"pins".to_owned()), "{hi}");
    let walled = v3.call(&json!({"id": 1, "op": "pins", "paths": ["a.md"]}));
    assert_eq!(walled["error"]["code"], json!("bad_request"), "{walled}");
    assert!(
        walled["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unknown request field `paths` on `pins`"),
        "{walled}"
    );

    let ws = tmp.path().join("ws");
    fs::create_dir_all(&ws).unwrap();
    let mut v2 = Conn::open(server.socket_path());
    let hi2 = v2.call(&json!({
        "op": "hello", "proto": 1, "workspace": ws.to_str().unwrap(),
    }));
    assert!(!caps(&hi2).contains(&"pins".to_owned()), "{hi2}");
    let refused = v2.call(&json!({"id": 2, "op": "pins"}));
    assert_eq!(refused["error"]["code"], json!("unknown_op"), "{refused}");
}
