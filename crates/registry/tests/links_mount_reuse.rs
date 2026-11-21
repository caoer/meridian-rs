//! The §4.6 `links` op resolves a declared cross-root link in the daemon and
//! pays for the mounted root ONCE: the daemon hands the link resolver its
//! resident engine for that root (`Registry::mounted_corpus`), so a second
//! `links` call re-reads nothing of it.
//!
//! Measured in FULL FOLDS (`fs::fold_count`), as `walk_mount_reuse.rs` measures
//! `walk`. One test fn, alone in this binary, so no sibling test moves the
//! counter and the process-global `MERIDIAN_CONFIG` has one writer.
//!
//! Negative control: build the mounted root in process at the `links_op` call
//! site (`load_mounts_for`) and the second call folds it again.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use registry::raw_store::RawStore;
use registry::residency::Host;
use registry::residency::controller::Controller;
use registry::{Config, RunningServer};
use serde_json::{Value, json};
use tempfile::TempDir;

mod common;

/// A daemon config under `tmp/name`, with reap horizons large enough that the
/// background reaper never evicts state mid-test.
#[allow(clippy::duration_suboptimal_units)]
fn test_config(tmp: &TempDir, name: &str, residency: Option<Arc<Host>>) -> Config {
    let forever = Duration::from_secs(365 * 24 * 60 * 60);
    let dir = tmp.path().join(name);
    let mut config = Config::for_cache_root(dir.join("cache"));
    config.socket_path = dir.join("daemon.sock");
    config.state_path = dir.join("state.json");
    config.idle_threshold = forever;
    config.reap_interval = forever;
    config.prewarm_interval = forever;
    config.prewarm_quiet_max = forever;
    config.idle_exit = None;
    config.drain_cold_builds = Duration::from_secs(30);
    config.residency = residency;
    config
}

/// A persistent connection speaking raw NDJSON: one frame in, one frame out.
struct Conn {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Conn {
    /// Connect and bind `ws` on the v3 dialect.
    fn bind(socket: &Path, ws: &Path) -> Self {
        let stream = UnixStream::connect(socket).unwrap();
        let mut conn = Conn {
            writer: stream.try_clone().unwrap(),
            reader: BufReader::new(stream),
        };
        let hi = conn.call(&json!({
            "op": "hello", "proto": 1, "contract": "v3",
            "workspace": ws.to_str().unwrap(),
        }));
        assert_eq!(hi["ok"], json!(true), "v3 hello binds the workspace: {hi}");
        conn
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

    /// One served `links` call's `files` map.
    fn links(&mut self, path: Option<&str>) -> Value {
        let mut request = json!({"id": 1, "op": "links"});
        if let Some(path) = path {
            request["path"] = json!(path);
        }
        let response = self.call(&request);
        assert_eq!(response["ok"], json!(true), "links {path:?}: {response}");
        response["body"]["files"].clone()
    }
}

fn write(ws: &Path, rel: &str, body: &str) {
    let p = ws.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

/// The four askings: a member page, a member whose only link is a declared
/// head with a malformed remainder, a page outside the hash domain, and the
/// whole corpus.
const ASKINGS: [Option<&str>; 4] = [Some("a.md"), Some("b.md"), Some(".drafts/x.md"), None];

/// A mounted root `other`, the ambient workspace that links into it, and the
/// mount table binding the two, published to this process's environment.
/// Returns the workspace.
fn fixture(tmp: &TempDir) -> PathBuf {
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

    // The ambient workspace. `a.md` links across roots beside a table-external
    // URL; `b.md` names the declared root and nothing a corpus is needed for;
    // `.drafts/x.md` is outside the hash domain (a dot segment).
    let ws = tmp.path().join("ws");
    write(
        &ws,
        "a.md",
        "# A\n\nCross-root: [[other:c.md]]\nExternal: [[https://example.com]]\n",
    );
    write(&ws, "b.md", "# B\n\nMalformed: [[other:a:b]]\n");
    write(&ws, ".drafts/x.md", "# X\n\nDraft: [[other:c.md]]\n");

    let config_path = tmp.path().join("conf").join("MERIDIAN.md");
    write(
        tmp.path(),
        "conf/MERIDIAN.md",
        &format!(
            "---\ntype: meridian-config\nversion: 1\n---\n\n# Roots\n\n```meridian-mount\nname: other\npath: {}\nvault: other\n```\n",
            other.display()
        ),
    );
    // SAFETY: single-test binary; the daemons read the mount table from this
    // process's environment, as `walk_mount_reuse.rs` arranges it.
    unsafe { std::env::set_var("MERIDIAN_CONFIG", &config_path) };
    ws
}

#[test]
fn a_second_links_call_folds_no_mounted_root() {
    let tmp = TempDir::new().unwrap();
    let ws = fixture(&tmp);

    let server = RunningServer::start(test_config(&tmp, "eager", None)).unwrap();
    let mut conn = Conn::bind(server.socket_path(), &ws);

    // The first call warms the mounted root, once.
    let before_first = ::fs::fold_count();
    let first = conn.links(Some("a.md"));
    let entry = &first["a.md"];
    assert_eq!(
        entry["resolved_rooted"]["other"]["c.md"],
        json!(1),
        "a declared cross-root link resolves into the mounted root: {first}"
    );
    assert_eq!(
        entry["unresolved"]["https://example.com"],
        json!(1),
        "a table-external head stays unresolved, verbatim: {first}"
    );
    assert!(
        entry["refused"].is_null(),
        "and nothing on this page is refused: {first}"
    );
    let after_first = ::fs::fold_count();
    assert!(
        after_first > before_first,
        "the first links call warms the mounted root ({before_first} -> {after_first})"
    );

    // The same asking again: one answer, zero full folds.
    assert_eq!(conn.links(Some("a.md")), first, "one answer across calls");

    // A declared head whose remainder does not parse names no corpus, and is
    // still the daemon's to refuse: the table is read whatever the page names.
    let malformed = conn.links(Some("b.md"));
    let refusal = &malformed["b.md"]["refused"]["other:a:b"];
    assert_eq!(
        (&refusal["color"], &refusal["reason"]),
        (&json!("red"), &json!("bad-ref")),
        "a declared head with a malformed remainder is refused: {malformed}"
    );
    assert!(
        malformed["b.md"]["unresolved"]["other:a:b"].is_null(),
        "and is not left unresolved: {malformed}"
    );

    // A page outside the hash domain, named: its rooted link resolves too.
    let draft = conn.links(Some(".drafts/x.md"));
    assert_eq!(
        draft[".drafts/x.md"]["resolved_rooted"]["other"]["c.md"],
        json!(1),
        "an out-of-domain page's cross-root link resolves: {draft}"
    );

    // The whole corpus: the roots come from the generation's descriptors, and
    // each member answers as it does when named.
    let whole = conn.links(None);
    assert_eq!(whole["a.md"], first["a.md"], "whole corpus: {whole}");
    assert_eq!(whole["b.md"], malformed["b.md"], "whole corpus: {whole}");
    assert!(
        whole[".drafts/x.md"].is_null(),
        "the enumeration holds members only: {whole}"
    );

    assert_eq!(
        ::fs::fold_count(),
        after_first,
        "no links call after the first pays a full fold"
    );

    // A daemon whose generations are leased through a residency host names the
    // same roots from its descriptors and gives the same four answers.
    let host = Arc::new(Host::new(
        Controller::new(1 << 30),
        RawStore::new(&tmp.path().join("backing")).unwrap(),
    ));
    let leased = RunningServer::start(test_config(&tmp, "leased", Some(host))).unwrap();
    let mut over_leases = Conn::bind(leased.socket_path(), &ws);
    for asking in ASKINGS {
        assert_eq!(
            over_leases.links(asking),
            conn.links(asking),
            "leased and eager daemons agree on {asking:?}"
        );
    }
}
