//! Read is the mint (D6/D9/D13/D16). Every gate goes over one live daemon's
//! socket: the ledger is daemon-session memory, so a two-process shape would
//! test the wrong thing.

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use registry::{Config, RunningServer};
use serde_json::{Value, json};
use tempfile::TempDir;

/// A daemon config rooted under `tmp`, with horizons large enough that neither
/// the reaper nor the pre-warm thread fires mid-test (a reap would legitimately
/// drop the ledger; a pre-warm would rebuild the engine behind the gate).
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
    // No idle exit: a daemon that reaped itself mid-assertion would flake.
    config.idle_exit = None;
    config
}

/// A workspace `tmp/ws` seeded with `files` — a sibling of the cache root, so
/// the corpus walk never sees the drawer.
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

/// A persistent NDJSON connection: `hello` binds the workspace, then every op
/// rides the same connection.
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

    fn hello(&mut self, ws: &Path) -> Value {
        self.call(&json!({
            "op": "hello",
            "proto": 1,
            "contract": "v3",
            "workspace": ws.to_str().unwrap(),
        }))
    }

    /// A composed sections read carrying the daemon-derived actor (D13).
    fn read_sections(&mut self, path: &str, selectors: &[&str], actor: Option<&str>) -> Value {
        let mut frame = json!({
            "op": "read",
            "path": path,
            // The wire takes tagged selectors; convert from the helper's &[&str].
            "sections": selectors
                .iter()
                .map(|s| wire::ReadSel::parse(s))
                .collect::<Vec<_>>(),
        });
        if let Some(actor) = actor {
            frame["actor"] = json!(actor);
        }
        let out = self.call(&frame);
        assert_eq!(out["ok"], json!(true), "read ok: {out}");
        out
    }
}

const PLAN: &str = "# Alpha\n\nalpha body\n\n# Beta\n\nbeta body\n";
const NOTES: &str = "# Notes\n\nnothing yet\n";

/// The `sec_rev` the read served for `sel` — the rev the receipt must carry.
fn served_sec_rev(read: &Value, sel: &str) -> String {
    read["body"]["sections"]
        .as_array()
        .expect("sections array")
        .iter()
        .find(|s| s["sel"] == json!(wire::ReadSel::parse(sel)))
        .unwrap_or_else(|| panic!("section {sel} served: {read}"))["sec_rev"]
        .as_str()
        .expect("sec_rev string")
        .to_string()
}

/// Every regular file under `dir` as (relative path → bytes). Sockets and
/// directories are skipped — opening the daemon socket would block.
fn file_tree(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        for entry in fs::read_dir(&next).expect("read_dir") {
            let entry = entry.expect("dir entry");
            let kind = entry.file_type().expect("file type");
            if kind.is_dir() {
                stack.push(entry.path());
            } else if kind.is_file() {
                let rel = entry
                    .path()
                    .strip_prefix(dir)
                    .expect("prefix")
                    .to_path_buf();
                out.insert(rel, fs::read(entry.path()).expect("read file"));
            }
        }
    }
    out
}

/// Gate 1 — an agent-path read mints one receipt per section served, carrying
/// the three D9 facts: actor, canonical selector, and served `sec_rev`.
#[test]
fn an_agent_read_mints_a_receipt_carrying_actor_selector_and_rev() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    conn.hello(&ws);

    let read = conn.read_sections("plan.md", &["Alpha"], Some("agent-7"));
    let read_rev = served_sec_rev(&read, "Alpha");

    let canonical = workspace::canonicalize(&ws).unwrap();
    let mints = server.registry().read_mints(&canonical);
    let receipt = mints
        .lookup("agent-7", "plan.md", &wire::ReadSel::parse("Alpha"))
        .expect("the read minted a receipt for the selector it served");
    assert_eq!(receipt.actor, "agent-7", "the receipt names the actor");
    assert_eq!(receipt.path, "plan.md", "…the path it read");
    assert_eq!(
        receipt.selector,
        wire::ReadSel::parse("Alpha"),
        "…the canonical selector, tagged since U14"
    );
    assert_eq!(
        receipt.sec_rev, read_rev,
        "…and the rev of the bytes the caller actually received"
    );
    assert_eq!(mints.len(), 1, "one section served, one receipt minted");

    server.shutdown();
}

/// Gate 2 — the receipt survives the rebuild a write forces, and stays current:
/// its rev still matches what the target section serves afterwards.
#[test]
fn the_receipt_survives_the_write_that_rebuilds_the_warm_engine() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("plan.md", PLAN), ("notes.md", NOTES)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    conn.hello(&ws);

    let read = conn.read_sections("plan.md", &["Alpha"], Some("agent-7"));
    let read_rev = served_sec_rev(&read, "Alpha");

    // A commit into a sibling file — the shape of a pin's own lock write.
    let notes = conn.read_sections("notes.md", &["Notes"], Some("agent-7"));
    let notes_rev = served_sec_rev(&notes, "Notes");
    let splice = conn.call(&json!({
        "id": 9,
        "op": "splice",
        "path": "notes.md",
        "edits": [{
            "target": {"hpath": [{"h": "Notes"}]},
            "edit": {"match": {"old": "nothing yet", "new": "pinned"}},
            "if_node_rev": notes_rev,
        }],
    }));
    assert_eq!(splice["ok"], json!(true), "splice ok: {splice}");
    assert_ne!(
        splice["body"]["fingerprint_after"], splice["body"]["fingerprint_before"],
        "the corpus content hash MOVED — the next warm must rebuild: {splice}"
    );

    // `Built` is the parse-count proof that the warm engine was rebuilt.
    let canonical = workspace::canonicalize(&ws).unwrap();
    assert_eq!(
        server.registry().warm_or_build(&canonical).unwrap(),
        registry::WarmOutcome::Built { docs: 2 },
        "the write rebuilt the warm engine"
    );

    let mints = server.registry().read_mints(&canonical);
    let receipt = mints
        .lookup("agent-7", "plan.md", &wire::ReadSel::parse("Alpha"))
        .expect("the pin's own write must not evaporate the receipt that authorizes it");
    assert_eq!(receipt.sec_rev, read_rev, "…carrying its minted rev");

    let after = conn.read_sections("plan.md", &["Alpha"], None);
    assert_eq!(
        served_sec_rev(&after, "Alpha"),
        receipt.sec_rev,
        "the receipt's rev still matches the target section after the write"
    );

    server.shutdown();
}

/// Gate 3 — selector grain (D6): reading section A does not satisfy a lookup
/// for section B.
#[test]
fn a_read_of_section_a_does_not_satisfy_a_lookup_for_section_b() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    conn.hello(&ws);

    conn.read_sections("plan.md", &["Alpha"], Some("agent-7"));

    let canonical = workspace::canonicalize(&ws).unwrap();
    let mints = server.registry().read_mints(&canonical);
    assert!(
        mints
            .lookup("agent-7", "plan.md", &wire::ReadSel::parse("Alpha"))
            .is_some(),
        "the section that WAS read is gated"
    );
    assert!(
        mints
            .lookup("agent-7", "plan.md", &wire::ReadSel::parse("Beta"))
            .is_none(),
        "the unread sibling section is NOT — the receipt is selector-grained"
    );

    server.shutdown();
}

/// Gate 4 — actor isolation (D13): a foreign actor's read mints for that actor
/// alone; my lookup does not see it.
#[test]
fn a_foreign_actors_read_does_not_mint_for_me() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    conn.hello(&ws);

    conn.read_sections("plan.md", &["Alpha"], Some("agent-other"));

    let canonical = workspace::canonicalize(&ws).unwrap();
    let mints = server.registry().read_mints(&canonical);
    assert!(
        mints
            .lookup("agent-other", "plan.md", &wire::ReadSel::parse("Alpha"))
            .is_some(),
        "the reading actor holds the receipt"
    );
    assert!(
        mints
            .lookup("agent-mine", "plan.md", &wire::ReadSel::parse("Alpha"))
            .is_none(),
        "another actor's read never gates MY pin"
    );

    server.shutdown();
}

/// Gate 5 — the CLI bypass (D16): a read with no actor mints nothing. A blank
/// actor counts as absent — an empty string is not an identity.
#[test]
fn a_read_with_no_actor_mints_nothing() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    conn.hello(&ws);

    let canonical = workspace::canonicalize(&ws).unwrap();

    conn.read_sections("plan.md", &["Alpha"], None);
    assert!(
        server.registry().read_mints(&canonical).is_empty(),
        "an actor-less read mints nothing (the CLI path)"
    );

    conn.read_sections("plan.md", &["Alpha"], Some("   "));
    assert!(
        server.registry().read_mints(&canonical).is_empty(),
        "a blank actor is not an identity — it opens no shared bucket"
    );

    server.shutdown();
}

/// Gate 6 — a toc-mode read mints nothing: it serves the section map, not
/// content.
#[test]
fn a_toc_mode_read_mints_nothing_it_served_no_content() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    conn.hello(&ws);

    let toc = conn.call(&json!({
        "op": "read", "path": "plan.md", "actor": "agent-7",
    }));
    assert_eq!(toc["ok"], json!(true), "toc read ok: {toc}");
    assert!(
        toc["body"]["toc"].as_array().is_some_and(|r| !r.is_empty()),
        "the toc read really served the section map: {toc}"
    );

    let canonical = workspace::canonicalize(&ws).unwrap();
    assert!(
        server.registry().read_mints(&canonical).is_empty(),
        "a map without content mints no attestation-bearing receipt"
    );

    server.shutdown();
}

/// Gate 7 — no persistence (D6): the whole sandbox is byte-for-byte identical
/// across an agent read that provably minted.
#[test]
fn the_mint_path_writes_nothing_to_disk() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    // Bind first: `hello` legitimately writes the drawer sentinel + state file,
    // so the snapshot is taken after it.
    conn.hello(&ws);

    let before = file_tree(tmp.path());
    conn.read_sections("plan.md", &["Alpha", "Beta"], Some("agent-7"));

    let canonical = workspace::canonicalize(&ws).unwrap();
    assert_eq!(
        server.registry().read_mints(&canonical).len(),
        2,
        "the read really minted (two sections served, two receipts)"
    );
    assert_eq!(
        file_tree(tmp.path()),
        before,
        "the mint is memory-only: not one byte of the sandbox changed"
    );

    server.shutdown();
}
