//! The § A.10 `walk` pays for a mounted root ONCE: the daemon hands the
//! colour computer its resident engine for that root
//! (`Registry::mounted_corpus`), so a second walk re-reads nothing of it.
//! The alternative this forbids — rebuilding every addressed mounted root
//! from disk per call (walk, hash, parse) — is the whole daemon's CPU and one
//! corpus-sized allocation per caller on a large root (measured on a 61k-file
//! sessions root: two concurrent `sql` calls at 17.8 GB resident).
//!
//! Measured in FULL FOLDS (`fs::fold_count`): the process-global count of
//! whole-corpus snapshots. A resident engine's currency pass never pays one
//! (it is O(dirty) through the leaf memo), so the second walk's delta is
//! exactly the work this test forbids — CPU or wall would measure the host.
//! One test fn, alone in this binary, so no sibling test moves the counter.
//!
//! Negative control: restore `load_mounts_for` (the in-process build) at the
//! `walk_op` call site and the second walk folds the mounted root again.

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

/// One lock block carrying a whole-body pin on `object` at `token`.
fn lock_block(object: &str, token: &str) -> String {
    let mut l = lock::Lock::new();
    l.upsert_pin(lock::PinEntry::new(
        object,
        "9ae3f1deadbeef",
        lock::Selector::Path(Vec::new()),
        token,
    ));
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

/// `(selector, color)` per row, so the two answers can be compared whole.
fn colors(frame: &Value) -> Vec<(String, String)> {
    frame["body"]["entries"]
        .as_array()
        .unwrap_or_else(|| panic!("entries array: {frame}"))
        .iter()
        .map(|r| {
            (
                r["selector"].as_str().unwrap().to_owned(),
                r["color"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[test]
fn a_second_walk_folds_no_mounted_root() {
    let tmp = TempDir::new().unwrap();

    // The mounted "other" root, populated enough that a rebuild is a real
    // sweep and not a rounding error: the pin target plus bulk pages.
    let other = tmp.path().join("other");
    let other_c = "# C\n\nleaf body\n";
    write(&other, "c.md", other_c);
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

    // The ambient workspace: a.md pins c across roots (green).
    let ws = tmp.path().join("ws");
    let a_raw = format!(
        "# A\n\ndraws from c across roots\n\n{}\n",
        lock_block("other:c", &live_token(other_c))
    );
    write(&ws, "a.md", &a_raw);

    // The machine's mount table binds exactly `other`. Process-global env,
    // one test fn in this binary by design (see module docs).
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
    let hi = conn.call(&json!({
        "op": "hello", "proto": 1, "contract": "v3",
        "workspace": ws.to_str().unwrap(),
    }));
    assert_eq!(hi["ok"], json!(true), "v3 hello binds the workspace: {hi}");

    let walk = json!({"id": 1, "op": "walk", "path": "a.md"});

    // The first walk may build the mounted root — once. The counter must be
    // seen to MOVE here, or a zero delta below would be the checkmark of a
    // counter that never counts.
    let before_first = ::fs::fold_count();
    let first = conn.call(&walk);
    assert_eq!(first["ok"], json!(true), "the first walk serves: {first}");
    assert!(
        colors(&first).contains(&("other:c.md".to_owned(), "green".to_owned())),
        "the cross-root pin colours green against the mounted root: {first}"
    );
    let after_first = ::fs::fold_count();
    assert!(
        after_first > before_first,
        "the first walk builds the mounted root, so the fold counter moves \
         ({before_first} -> {after_first}); it did not, so this instrument is not counting"
    );

    // The second walk: same answer, and NOT ONE full fold — the mounted root
    // is the daemon's resident engine now, refreshed O(dirty), never
    // re-walked, re-hashed or re-parsed.
    let second = conn.call(&walk);
    assert_eq!(
        second["ok"],
        json!(true),
        "the second walk serves: {second}"
    );
    assert_eq!(colors(&second), colors(&first), "one answer across calls");
    let after_second = ::fs::fold_count();
    assert_eq!(
        after_second,
        after_first,
        "a second walk re-reads no mounted root, yet it paid {} full corpus fold(s) — the \
         mounted root was rebuilt from disk instead of served from the resident engine",
        after_second - after_first
    );
}
