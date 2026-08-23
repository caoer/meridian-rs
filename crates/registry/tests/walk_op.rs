//! E2E gates for `walk` — pin-graph context assembly over the wire
//! (`docs/wire-contract.md` § A.10): up = what the page draws from, down =
//! who pins the page (blast radius), every answer citing the doc revs it
//! read. Read-only, workspace-bound, v3-only, advertised as cap `walk`.
//!
//! The env-dependent lifecycle rides ONE test fn by design (edition 2024:
//! env mutation is unsafe, and `MERIDIAN_CONFIG` is process-global); the
//! caps and strict-wall gates are env-free — they refuse before the serve
//! path reads the environment.

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

fn entries(frame: &Value) -> &Vec<Value> {
    frame["body"]["entries"]
        .as_array()
        .unwrap_or_else(|| panic!("entries array: {frame}"))
}

/// The § A.10 lifecycle, one env pin (`MERIDIAN_CONFIG`), one workspace:
/// up-walk over a green chain with rev citations; bounded down-walk naming
/// the hash-domain-excluded population (§12.1 enumerator clause); cross-root
/// rows colored against the mounted root (green) and grey(unmounted) for an
/// undeclared one; a domain-excluded NAMED page still walks (§12.1 door
/// clause); a missing root refuses `file_not_found`; an in-snapshot cycle
/// refuses `walk_cycle`.
#[test]
#[allow(clippy::too_many_lines)] // one sequential lifecycle script by design
fn walk_lifecycle_up_down_rooted_and_refusals() {
    let tmp = TempDir::new().unwrap();

    // The mounted "other" root: c.md is the cross-root pin target.
    let other = tmp.path().join("other");
    let other_c = "# C\n\nleaf body\n";
    write(&other, "c.md", other_c);
    write(
        &other,
        "MERIDIAN.md",
        "---\ntype: meridian-root\nversion: 1\nname: other\n---\n",
    );

    // The ambient workspace: a green chain a → b, b pinning two rooted
    // targets — one mounted (green), one undeclared (grey unmounted).
    let ws = tmp.path().join("ws");
    let b_raw = format!(
        "# B\n\ndraws from c across roots\n\n{}\n",
        chain_block(&[
            ("other:c", &live_token(other_c)),
            ("ghost:z", "fp1.span2.b3.00000000"),
        ])
    );
    let a_raw = format!(
        "# A\n\ndraws from b\n\n{}\n",
        chain_block(&[("b", &live_token(&b_raw))])
    );
    write(&ws, "b.md", &b_raw);
    write(&ws, "a.md", &a_raw);
    // The hash domain ignores bulk/**: vendored.md is real, on disk, outside.
    write(
        &ws,
        "meridian/domain.md",
        "---\nversion: 1\nignore:\n  - \"bulk/**\"\n---\n",
    );
    write(
        &ws,
        "bulk/vendored.md",
        "# bulk\n\nexcluded but walkable.\n",
    );

    // The machine's mount table binds exactly `other`. One env-dependent
    // test fn by design (see module docs).
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

    // Up: the context walk — depth-tagged entries, colors computed live,
    // every answer citing the doc revs it read (§2.4 honesty citation).
    let up = conn.call(&json!({"id": 1, "op": "walk", "path": "a.md"}));
    assert_eq!(up["ok"], json!(true), "up walk serves: {up}");
    assert_eq!(up["body"]["direction"], json!("up"), "{up}");
    assert_eq!(up["body"]["page"], json!("a.md"), "{up}");
    assert!(
        up["body"].get("depth_bound").is_none(),
        "unbounded walk carries no depth_bound key: {up}"
    );
    let rows = entries(&up);
    assert_eq!(rows[0]["selector"], json!("b.md"), "{up}");
    assert_eq!(rows[0]["depth"], json!(1), "{up}");
    assert_eq!(rows[0]["color"], json!("green"), "{up}");
    assert!(
        rows[0]["rev"].is_string(),
        "a green row cites its rev: {up}"
    );
    assert!(
        rows[0].get("reason").is_none() || rows[0]["reason"].is_null(),
        "green carries no reason: {up}"
    );
    // The cross-root rows ride at depth 2, colored against the mount table:
    // the bound root verifies green; the undeclared one is grey(unmounted).
    let colors: Vec<(&str, &str)> = rows
        .iter()
        .map(|r| {
            (
                r["selector"].as_str().unwrap(),
                r["color"].as_str().unwrap(),
            )
        })
        .collect();
    assert!(
        colors.contains(&("other:c.md", "green")),
        "the mounted cross-root pin colors against the target root: {up}"
    );
    let grey = rows
        .iter()
        .find(|r| r["selector"] == json!("ghost:z.md"))
        .unwrap_or_else(|| panic!("the undeclared-root row is served, never dropped: {up}"));
    assert_eq!(grey["color"], json!("grey"), "{up}");
    assert_eq!(grey["reason"], json!("unmounted"), "{up}");
    let cited: Vec<&str> = up["body"]["revs_read"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["path"].as_str().unwrap())
        .collect();
    assert!(
        cited.contains(&"a.md") && cited.contains(&"b.md"),
        "the walk cites the doc revs it read: {up}"
    );

    // Down, bounded: exactly the direct dependents, the bound echoed, and
    // the §12.1 enumerator clause — a population answer names what the hash
    // domain left out instead of publishing a partial census as whole.
    let down = conn.call(&json!({"id": 2, "op": "walk", "path": "b.md", "down": true, "depth": 1}));
    assert_eq!(down["ok"], json!(true), "down walk serves: {down}");
    assert_eq!(down["body"]["direction"], json!("down"), "{down}");
    assert_eq!(down["body"]["depth_bound"], json!(1), "{down}");
    let dep: Vec<&str> = entries(&down)
        .iter()
        .map(|r| r["selector"].as_str().unwrap())
        .collect();
    assert_eq!(dep, vec!["a.md"], "depth 1 = the direct dependents: {down}");
    let excluded: Vec<&str> = down["body"]["excluded"]
        .as_array()
        .unwrap_or_else(|| panic!("down walk voices the excluded population: {down}"))
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(
        excluded,
        vec!["bulk/vendored.md"],
        "the complete excluded list rides the machine answer (§12.1): {down}"
    );
    assert!(
        up["body"].get("excluded").is_none(),
        "up drops nothing, so it carries no excluded key: {up}"
    );

    // §12.1 door clause: a NAMED page the hash domain excludes is served —
    // membership gates enumerations, never what a named path is entitled to.
    let door = conn.call(&json!({"id": 3, "op": "walk", "path": "bulk/vendored.md"}));
    assert_eq!(
        door["ok"],
        json!(true),
        "a named excluded page still walks (§12.1 door-family clause): {door}"
    );
    assert_eq!(door["body"]["page"], json!("bulk/vendored.md"), "{door}");

    // A root with no file refuses `file_not_found`, echoing the path.
    let miss = conn.call(&json!({"id": 4, "op": "walk", "path": "nope.md"}));
    assert_eq!(miss["ok"], json!(false), "{miss}");
    assert_eq!(miss["error"]["code"], json!("file_not_found"), "{miss}");
    assert_eq!(miss["error"]["path"], json!("nope.md"), "{miss}");

    // An in-snapshot cycle refuses `walk_cycle` (env class), naming the loop.
    let loop_ws = tmp.path().join("loop");
    write(
        &loop_ws,
        "x.md",
        &format!(
            "# X\n\n{}\n",
            chain_block(&[("y", "fp1.span2.b3.00000000")])
        ),
    );
    write(
        &loop_ws,
        "y.md",
        &format!(
            "# Y\n\n{}\n",
            chain_block(&[("x", "fp1.span2.b3.00000000")])
        ),
    );
    let mut loop_conn = Conn::open(server.socket_path());
    loop_conn.hello_v3(&loop_ws);
    let cycle = loop_conn.call(&json!({"id": 5, "op": "walk", "path": "x.md"}));
    assert_eq!(cycle["ok"], json!(false), "{cycle}");
    assert_eq!(cycle["error"]["code"], json!("walk_cycle"), "{cycle}");
    assert_eq!(
        cycle["error"]["recovery"],
        json!("env"),
        "the workspace's own pin graph is broken, not one request: {cycle}"
    );
    let message = cycle["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("x.md") && message.contains("y.md"),
        "the refusal names the loop: {message}"
    );
}

/// v3 advertises `walk` at op grain; the frozen v2 caps stay byte-identical,
/// and a v2 session's `walk` answers `unknown_op` (§3.2 discovery honesty).
#[test]
fn v3_advertises_walk_and_v2_answers_unknown_op() {
    let tmp = TempDir::new().unwrap();
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    let mut v3 = Conn::open(server.socket_path());
    let hi = v3.call(&json!({"op": "hello", "proto": 1, "contract": "v3"}));
    let caps: Vec<&str> = hi["body"]["caps"]
        .as_array()
        .expect("caps array")
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert!(caps.contains(&"walk"), "v3 caps advertise the op: {hi}");

    // The v2 session BINDS a workspace: the op-name honesty arm sits behind
    // the binding guard (the `read`/`create` precedent), so a workspace-less
    // v2 call would answer the guard, not the vocabulary.
    let ws = tmp.path().join("ws");
    fs::create_dir_all(&ws).unwrap();
    let mut v2 = Conn::open(server.socket_path());
    let hi2 = v2.call(&json!({
        "op": "hello", "proto": 1, "workspace": ws.to_str().unwrap(),
    }));
    let v2_caps: Vec<&str> = hi2["body"]["caps"]
        .as_array()
        .expect("caps array")
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert!(
        !v2_caps.contains(&"walk"),
        "frozen v2 caps never grow the op: {hi2}"
    );
    let refused = v2.call(&json!({"id": 2, "op": "walk", "path": "a.md"}));
    assert_eq!(refused["ok"], json!(false));
    assert_eq!(
        refused["error"]["code"],
        json!("unknown_op"),
        "a v2 session answers unknown_op: {refused}"
    );
}

/// The strict field wall (§3.2): `walk` takes `path`, `down`, `depth`, and an
/// unknown field refuses by name — before the serve path reads anything.
#[test]
fn strict_decode_refuses_an_unknown_field_by_name() {
    let tmp = TempDir::new().unwrap();
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    conn.call(&json!({"op": "hello", "proto": 1, "contract": "v3"}));

    let refused = conn.call(&json!({"id": 3, "op": "walk", "path": "a.md", "direction": "up"}));
    assert_eq!(refused["ok"], json!(false), "{refused}");
    assert_eq!(refused["error"]["code"], json!("bad_request"), "{refused}");
    let message = refused["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("unknown request field `direction` on `walk`"),
        "the wall names the field and the op: {message}"
    );
}
