//! E2E gates for `mounts` — mount-table discovery (`docs/wire-contract.md`
//! § A.5, wire-map W20): the live root registry, machine-scoped, v3-only,
//! served workspace-less under the per-call config-hash freshness law.
//!
//! The env-dependent lifecycle rides ONE test fn by design (edition 2024:
//! env mutation is unsafe, and `MERIDIAN_CONFIG` is process-global); every
//! other gate here is env-free — it refuses before the serve path reads the
//! environment.

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
// `Duration::from_hours` is not const-stable at MSRV 1.96; the seconds form is
// the workspace precedent.
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

    /// A WORKSPACE-LESS v3 hello — the § A.5 caller: discovery exists for
    /// exactly the agent that does not know a root yet.
    fn hello_v3_bare(&mut self) -> Value {
        self.call(&json!({"op": "hello", "proto": 1, "contract": "v3"}))
    }
}

/// A root at `dir` carrying its own `meridian-root` self-declaration.
fn write_declared_root(dir: &Path, name: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(
        dir.join("MERIDIAN.md"),
        format!("---\ntype: meridian-root\nversion: 1\nname: {name}\n---\n"),
    )
    .unwrap();
}

/// A `meridian-config` binding the given pre-rendered mount blocks.
fn write_config(path: &Path, blocks: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        format!("---\ntype: meridian-config\nversion: 1\n---\n\n# Roots\n\n{blocks}"),
    )
    .unwrap();
}

fn vault_block(name: &str, path: &Path) -> String {
    format!(
        "```meridian-mount\nname: {name}\npath: {}\nvault: {name}\n```\n\n",
        path.display()
    )
}

fn rows(frame: &Value) -> &Vec<Value> {
    frame["body"]["mounts"]
        .as_array()
        .unwrap_or_else(|| panic!("mounts array: {frame}"))
}

/// The § A.5 lifecycle over one binding file, on a workspace-less v3
/// connection: the live table serves with its freshness token; a mount added
/// mid-process is named on the next call; a changed-but-invalid table refuses
/// `mount_table_invalid` (env) naming the offending entry — and never serves
/// the previous table as current; a repaired file recovers without redialing.
#[test]
#[allow(clippy::too_many_lines)] // one sequential lifecycle script by design
fn mounts_lifecycle_freshness_and_changed_invalid_refusal() {
    let tmp = TempDir::new().unwrap();
    let wiki = tmp.path().join("wiki");
    write_declared_root(&wiki, "wiki");
    let ghost = tmp.path().join("nowhere");

    let config_path = tmp.path().join("conf").join("MERIDIAN.md");
    // The wiki mount carries the declared-primary designation, so the row
    // projection is pinned on both sides: literal `true` on the designated
    // row, NO key anywhere else.
    let base = format!(
        "```meridian-mount\nname: wiki\npath: {}\nprimary: true\nvault: wiki\n```\n\n```meridian-mount\nname: ghost\npath: {}\n```\n",
        wiki.display(),
        ghost.display()
    );
    write_config(&config_path, &base);
    // One env-dependent test fn by design (see module docs).
    unsafe { std::env::set_var("MERIDIAN_CONFIG", &config_path) };

    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    let hi = conn.hello_v3_bare();
    assert_eq!(
        hi["ok"],
        json!(true),
        "workspace-less v3 hello binds nothing and succeeds: {hi}"
    );

    // The live table, machine-scoped: served with NO workspace bound.
    let first = conn.call(&json!({"id": 7, "op": "mounts"}));
    assert_eq!(
        first["ok"],
        json!(true),
        "mounts serves workspace-less: {first}"
    );
    assert_eq!(first["id"], json!(7));
    let rev1 = first["body"]["config_rev"]
        .as_str()
        .unwrap_or_else(|| panic!("config_rev present: {first}"));
    assert_eq!(
        rev1.len(),
        16,
        "config_rev is 16 hex (file_rev family): {first}"
    );
    assert!(
        rev1.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "config_rev is lowercase hex: {first}"
    );

    let table = rows(&first);
    assert_eq!(
        table.len(),
        2,
        "both declared mounts served, document order: {first}"
    );
    // Row §A.5: {name, state, workspace?, primary?} in the engine's
    // own words.
    assert_eq!(table[0]["name"], json!("wiki"));
    assert!(
        table[0].get("kind").is_none(),
        "kind left the wire row (kind-sweep 2026-08-13)"
    );
    assert_eq!(table[0]["state"], json!("bound"));
    assert_eq!(
        table[0]["primary"],
        json!(true),
        "the designated row carries literal true (§ A.5): {first}"
    );
    let canonical_wiki = fs::canonicalize(&wiki).unwrap();
    assert_eq!(
        table[0]["workspace"],
        json!(canonical_wiki.to_str().unwrap()),
        "workspace is the canonical bound path — the same handle hello returns: {first}"
    );
    assert_eq!(table[1]["name"], json!("ghost"));
    assert_eq!(
        table[1]["state"],
        json!("grey(path-unseeable)"),
        "an absent root is a served row carrying its state word, never a refusal: {first}"
    );
    assert!(
        table[1].get("workspace").is_none(),
        "workspace is absent where the binding did not canonicalize: {first}"
    );
    assert!(
        table[1].get("primary").is_none(),
        "an undesignated row carries NO primary key — absence is the only not-primary spelling on the wire: {first}"
    );

    // Unchanged bytes ⇒ the derived table serves again under the same token.
    let again = conn.call(&json!({"id": 8, "op": "mounts"}));
    assert_eq!(again["body"]["config_rev"], json!(rev1));
    assert_eq!(rows(&again).len(), 2);

    // A mount added mid-process is named on the NEXT call — no mtime, no TTL,
    // no hello-time snapshot.
    let sessions = tmp.path().join("sessions");
    write_declared_root(&sessions, "sessions");
    write_config(
        &config_path,
        &format!("{base}\n{}", vault_block("sessions", &sessions)),
    );
    let grown = conn.call(&json!({"id": 9, "op": "mounts"}));
    assert_eq!(grown["ok"], json!(true), "{grown}");
    let rev2 = grown["body"]["config_rev"].as_str().unwrap();
    assert_ne!(rev2, rev1, "changed bytes mint a new config_rev: {grown}");
    let named: Vec<&str> = rows(&grown)
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        named,
        vec!["wiki", "ghost", "sessions"],
        "the added mount is named on the next call, document order: {grown}"
    );
    assert_eq!(rows(&grown)[2]["state"], json!("bound"));

    // Changed-invalid REFUSES: two mounts binding one canonical tree is the
    // closed-schema duplicate-mount-path refusal — the op must not serve the
    // 3-row table as if current, and must not serve the broken one.
    write_config(
        &config_path,
        &format!(
            "{}{}",
            vault_block("wiki", &wiki),
            vault_block("wiki-two", &wiki)
        ),
    );
    let refused = conn.call(&json!({"id": 10, "op": "mounts"}));
    assert_eq!(
        refused["ok"],
        json!(false),
        "changed-invalid refuses: {refused}"
    );
    let err = &refused["error"];
    assert_eq!(err["code"], json!("mount_table_invalid"), "{refused}");
    assert_eq!(
        err["recovery"],
        json!("env"),
        "the binding file is an environment fact: {refused}"
    );
    assert_eq!(
        err["path"],
        json!(config_path.to_str().unwrap()),
        "the refusal names the binding file: {refused}"
    );
    let message = err["message"].as_str().unwrap();
    assert!(
        message.contains("wiki-two") && message.contains("the same tree"),
        "the refusal names the offending entry (Law A-3c): {message}"
    );

    // A repaired file recovers on the SAME connection — the refusal cached
    // nothing, so recovery is editing the file, not redialing.
    write_config(&config_path, &base);
    let healed = conn.call(&json!({"id": 11, "op": "mounts"}));
    assert_eq!(
        healed["ok"],
        json!(true),
        "a repaired table serves again: {healed}"
    );
    assert_eq!(
        healed["body"]["config_rev"],
        json!(rev1),
        "same bytes, same token: {healed}"
    );
    assert_eq!(rows(&healed).len(), 2);
}

/// v3 advertises `mounts` at op grain (the `create` precedent — no dotted
/// `mounts.<field>` at birth) plus exactly the field-only amendments § A.2
/// ships as dotted caps — today `mounts.primary` and `mounts.alias`, in that
/// order and nothing else; the frozen v2 caps stay byte-identical, and a v2
/// session's `mounts` answers `unknown_op`.
#[test]
fn v3_advertises_mounts_and_v2_answers_unknown_op() {
    let tmp = TempDir::new().unwrap();
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    let mut v3 = Conn::open(server.socket_path());
    let hi = v3.hello_v3_bare();
    let caps: Vec<&str> = hi["body"]["caps"]
        .as_array()
        .expect("caps array")
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert!(caps.contains(&"mounts"), "v3 caps advertise the op: {hi}");
    let dotted: Vec<&&str> = caps.iter().filter(|c| c.starts_with("mounts.")).collect();
    assert_eq!(
        dotted,
        vec![&"mounts.primary", &"mounts.alias"],
        "dotted mounts.<field> caps are exactly the § A.2 field-only amendments — today the declared-primary designation and the root alias, nothing else: {hi}"
    );

    let mut v2 = Conn::open(server.socket_path());
    let hi2 = v2.call(&json!({"op": "hello", "proto": 1}));
    let v2_caps: Vec<&str> = hi2["body"]["caps"]
        .as_array()
        .expect("caps array")
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert!(
        !v2_caps.contains(&"mounts"),
        "frozen v2 caps never grow the op: {hi2}"
    );
    let refused = v2.call(&json!({"id": 2, "op": "mounts"}));
    assert_eq!(refused["ok"], json!(false));
    assert_eq!(
        refused["error"]["code"],
        json!("unknown_op"),
        "a v2 session answers unknown_op (§3.2 discovery honesty): {refused}"
    );
}

/// The strict field wall (§3.2): `mounts` takes NO parameters, and an unknown
/// field refuses by name — before the serve path reads anything.
#[test]
fn strict_decode_refuses_an_unknown_field_by_name() {
    let tmp = TempDir::new().unwrap();
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    conn.hello_v3_bare();

    let refused = conn.call(&json!({"id": 3, "op": "mounts", "path": "notes"}));
    assert_eq!(refused["ok"], json!(false), "{refused}");
    assert_eq!(refused["error"]["code"], json!("bad_request"), "{refused}");
    let message = refused["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("unknown request field `path` on `mounts`"),
        "the wall names the field and the op: {message}"
    );
}
