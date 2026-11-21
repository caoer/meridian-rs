//! The §4.6 `links` op pays for a mounted root ONCE (`docs/laws.md` § C-1):
//! the daemon hands the link resolver its resident engine for that root
//! (`Registry::mounted_corpus`), so a second `links` call re-reads nothing of
//! it and never degrades to an in-process full rebuild.
//!
//! Measured in FULL FOLDS (`fs::fold_count`), mirroring `walk_mount_reuse.rs`.
//! One test fn, alone in this binary, so no sibling test moves the counter.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use registry::{Config, RunningServer};
use serde_json::{Value, json};
use tempfile::TempDir;

mod common;

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
}

fn write(ws: &Path, rel: &str, body: &str) {
    let p = ws.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

#[test]
fn a_second_links_call_folds_no_mounted_root() {
    let tmp = TempDir::new().unwrap();

    // The mounted "other" root, populated enough that a rebuild is a real sweep.
    let other = tmp.path().join("other");
    write(&other, "c.md", "# C\n\nleaf body\n");
    for i in 0..200 {
        write(
            &other,
            &format!("bulk/page-{i:03}.md"),
            &format!("# Page {i}\n\nbody {i}\n"),
        );
    }
    write(
        &other,
        "MERIDIAN.md",
        "---\ntype: meridian-root\nversion: 1\nname: other\n---\n",
    );

    // The ambient workspace: a.md links across roots and also has a table-external URL.
    let ws = tmp.path().join("ws");
    write(
        &ws,
        "a.md",
        "# A\n\nCross-root: [[other:c.md]]\nExternal: [[https://example.com]]\n",
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
    // SAFETY: single-test binary; process-global env mirrors `walk_mount_reuse.rs`.
    unsafe { std::env::set_var("MERIDIAN_CONFIG", &config_path) };

    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    let hi = conn.call(&json!({
        "op": "hello", "proto": 1, "contract": "v3",
        "workspace": ws.to_str().unwrap(),
    }));
    assert_eq!(hi["ok"], json!(true), "v3 hello binds the workspace: {hi}");

    let req = json!({"id": 1, "op": "links", "path": "a.md"});

    // First call warms the mounted root once.
    let before_first = ::fs::fold_count();
    let first = conn.call(&req);
    assert_eq!(first["ok"], json!(true), "first links serves: {first}");
    let entry = &first["body"]["files"]["a.md"];
    assert_eq!(
        entry["resolved_rooted"]["other"]["c.md"],
        json!(1),
        "cross-root link resolves into mounted root: {first}"
    );
    assert_eq!(
        entry["unresolved"]["https://example.com"],
        json!(1),
        "table-external scheme stays unresolved verbatim: {first}"
    );
    assert!(
        entry["refused"]["https://example.com"].is_null(),
        "table-external scheme is never refused: {first}"
    );
    let after_first = ::fs::fold_count();
    assert!(
        after_first > before_first,
        "first links call must warm the mounted root ({before_first} -> {after_first})"
    );

    // Second call: identical answer, zero full folds.
    let second = conn.call(&req);
    assert_eq!(second["ok"], json!(true), "second links serves: {second}");
    assert_eq!(
        second["body"]["files"], first["body"]["files"],
        "one answer across calls"
    );
    let after_second = ::fs::fold_count();
    assert_eq!(
        after_second,
        after_first,
        "a second links call over the same mounted root must pay zero full folds \
         (delta = {})",
        after_second - after_first
    );
}
