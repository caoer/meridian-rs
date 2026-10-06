//! The § A.11 `sql` arm resolves a link into a mounted root over the wire:
//! the daemon binds that root as its resident engine's generation catalog
//! (merkle-spec §6.8), and the rooted destination the projection stores is
//! the one the mounted root's own membership answers.
//!
//! One test fn, alone in this binary: `MERIDIAN_CONFIG` is process-global.

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
}

fn write(ws: &Path, rel: &str, body: &str) {
    let p = ws.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

#[test]
fn sql_resolves_a_link_into_a_mounted_root() {
    let tmp = TempDir::new().unwrap();

    let other = tmp.path().join("other");
    write(
        &other,
        "deep/c.md",
        "---\naliases: [sea]\n---\n# C\n\nleaf body\n",
    );
    write(
        &other,
        "MERIDIAN.md",
        "---\ntype: meridian-root\nversion: 1\nname: other\n---\n",
    );

    // One link that lands in the mounted root by path, one by alias, and one
    // that names a page the mounted root does not hold.
    let ws = tmp.path().join("ws");
    write(
        &ws,
        "a.md",
        "# A\n\n[[other:deep/c.md]], [[other:sea]] and [[other:deep/absent.md]]\n",
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
    // SAFETY: the only test in this binary, set before the daemon starts.
    unsafe { std::env::set_var("MERIDIAN_CONFIG", &config_path) };

    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    let hi = conn.call(&json!({
        "op": "hello", "proto": 1, "contract": "v3",
        "workspace": ws.to_str().unwrap(),
    }));
    assert_eq!(hi["ok"], json!(true), "v3 hello binds the workspace: {hi}");

    let query = "SELECT target_raw, dest_root, dest_root_path FROM link \
                 WHERE src_path = 'a.md' ORDER BY target_raw";
    let expected = json!([
        ["other:deep/absent.md", null, null],
        ["other:deep/c.md", "other", "deep/c.md"],
        ["other:sea", "other", "deep/c.md"],
    ]);
    // The first statement cold-builds the projection; the second answers
    // from the pinned one, after the rooted rows were re-resolved against
    // the mounted root and found unmoved.
    for id in 1..=2 {
        let answer = conn.call(&json!({"id": id, "op": "sql", "query": query}));
        assert_eq!(answer["ok"], json!(true), "statement {id} serves: {answer}");
        assert_eq!(
            answer["body"]["rows"], expected,
            "statement {id}: the rooted destination is the mounted root's own member: {answer}"
        );
    }
    let pins = conn.call(&json!({"id": 3, "op": "sql", "query": "SELECT count(*) FROM hist.pin"}));
    assert_eq!(
        pins["body"]["rows"],
        json!([[1]]),
        "an unmoved mounted root appends no generation: {pins}"
    );

    // The alias moves to a new page of the mounted root. The next statement
    // resolves it through that root's current name index, not the one an
    // earlier statement saw.
    write(&other, "deep/c.md", "# C\n\nleaf body\n");
    write(
        &other,
        "x/n2.md",
        "---\naliases: [sea]\n---\n# N2\n\nnew home\n",
    );
    let moved = conn.call(&json!({"id": 4, "op": "sql", "query": query}));
    assert_eq!(moved["ok"], json!(true), "statement 4 serves: {moved}");
    assert_eq!(
        moved["body"]["rows"],
        json!([
            ["other:deep/absent.md", null, null],
            ["other:deep/c.md", "other", "deep/c.md"],
            ["other:sea", "other", "x/n2.md"],
        ]),
        "the alias resolves in the mounted root's current generation: {moved}"
    );
    let pins = conn.call(&json!({"id": 5, "op": "sql", "query": "SELECT count(*) FROM hist.pin"}));
    assert_eq!(
        pins["body"]["rows"],
        json!([[2]]),
        "a moved mounted destination appends a generation: {pins}"
    );
}
