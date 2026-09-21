//! §10.1's counter, at the doors that declare it (wire-contract §18 row 12).
//!
//! The counter was minted and retained on every daemon-served write — the CLI
//! lane included, its write verbs being wire clients — and published on the
//! `sub` ack, the push frames and the writer's own splice response and nowhere
//! else: `links` answered `changes_seq: 0` and the §4.7 mint answered `seq: 0`,
//! before and after a write that moved the fingerprint. A consumer polling the
//! counter as a change monotone read a corpus that never changed.
//!
//! Every gate here drives the real daemon over its socket: the ring is a
//! per-workspace daemon property, and an in-process re-call would test a shape
//! the wiring does not have.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use registry::{Config, RunningServer};
use serde_json::{Value, json};
use tempfile::TempDir;

mod common;

/// Longer than any test's life: the horizon a gate parks when it is not the
/// thing under test.
#[allow(clippy::duration_suboptimal_units)]
const FOREVER: Duration = Duration::from_secs(365 * 24 * 60 * 60);

const PLAN: &str = "# Goals\n\nship by August\n\n# Notes\n\nnothing yet\n";

fn test_config(tmp: &TempDir) -> Config {
    let dir = tmp.path().join("registry");
    let mut config = Config::for_cache_root(tmp.path().join("cache"));
    config.socket_path = dir.join("daemon.sock");
    config.state_path = dir.join("state.json");
    config.idle_threshold = FOREVER;
    config.reap_interval = FOREVER;
    config.prewarm_interval = FOREVER;
    config.prewarm_quiet_max = FOREVER;
    // Lifetime is the test's; idle-exit would flake mid-assertion.
    config.idle_exit = None;
    config.drain_cold_builds = Duration::from_secs(30);
    config
}

fn write_ws(root: &Path, files: &[(&str, &str)]) -> PathBuf {
    for (rel, content) in files {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }
    root.to_path_buf()
}

/// One NDJSON request channel.
struct Conn {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Conn {
    fn open(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .unwrap();
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
            serde_json::from_str(&response).unwrap_or_else(|e| panic!("frame {response:?}: {e}"))
        })
    }

    fn hello_v3(&mut self, ws: &Path) -> Value {
        self.call(&json!({
            "op": "hello", "proto": 1, "contract": "v3",
            "workspace": ws.to_str().unwrap(),
        }))
    }

    fn hello_v2(&mut self, ws: &Path) -> Value {
        self.call(&json!({"op": "hello", "proto": 1, "workspace": ws.to_str().unwrap()}))
    }

    fn links(&mut self) -> Value {
        let resp = self.call(&json!({"id": 80, "op": "links"}));
        assert_eq!(resp["ok"], json!(true), "links is served: {resp}");
        resp["body"].clone()
    }

    fn mint(&mut self) -> Value {
        self.mint_op("fingerprint")
    }

    /// The §4.7 mint under the session's own spelling: a frozen v2 session
    /// asks `root`, the v3 rename never having reached it.
    fn mint_op(&mut self, op: &str) -> Value {
        let resp = self.call(&json!({"id": 90, "op": op}));
        assert_eq!(resp["ok"], json!(true), "the mint is served: {resp}");
        resp["body"].clone()
    }

    /// A forced wire splice — the lane `mrd put` writes through.
    fn splice(&mut self, id: u64, old: &str, new: &str) -> Value {
        let resp = self.call(&json!({
            "id": id, "op": "splice", "path": "plan.md", "force": true,
            "actor": "agent:probe", "now": "2026-09-18T00:00:00Z",
            "edits": [{
                "target": {"hpath": [{"h": "Goals"}]},
                "edit": {"match": {"old": old, "new": new}},
            }],
        }));
        assert_eq!(resp["ok"], json!(true), "the write commits: {resp}");
        resp["body"].clone()
    }
}

/// The defect itself: a write mints a Delta, and the doors that declare the
/// counter publish it — in the tense of the fingerprint they answered at.
///
/// *Mutation:* pass a literal `0` to `read::links` / the §4.7 mint again and
/// both post-write assertions read 0 while the write's own response reads 1.
#[test]
fn a_daemon_lane_write_reaches_the_doors_that_declare_the_counter() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp.path().join("ws"), &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    let mut c = Conn::open(server.socket_path());
    assert_eq!(c.hello_v3(&ws)["ok"], json!(true));

    // A fresh epoch has numbered nothing, and says so as `0` — which is the
    // "aligned to no retained frame" reading, not a claim about the corpus.
    let before = c.links();
    assert_eq!(before["changes_seq"], json!(0), "fresh epoch: {before}");
    let mint_before = c.mint();
    assert_eq!(mint_before["seq"], json!(0), "fresh epoch: {mint_before}");

    let put = c.splice(3, "ship by August", "ship by September");
    assert_eq!(put["seq"], json!(1), "the write minted its Delta: {put}");

    let after = c.links();
    assert_eq!(
        after["changes_seq"],
        json!(1),
        "the counter reaches the view door: {after}"
    );
    assert_eq!(
        after["as_of_fingerprint"], put["fingerprint_after"],
        "and it is the counter in THAT fingerprint's tense: {after}"
    );
    let mint_after = c.mint();
    assert_eq!(
        mint_after["seq"],
        json!(1),
        "and the §4.7 mint: {mint_after}"
    );
    assert_eq!(mint_after["fingerprint"], put["fingerprint_after"]);

    // One epoch, one identity: the number is useless to a client that cannot
    // tell which numbering it belongs to (B-01).
    let instance = mint_after["tree_instance"].as_str().expect("the epoch id");
    assert!(!instance.is_empty());
    assert_eq!(
        after["tree_instance"], mint_after["tree_instance"],
        "both doors name one epoch"
    );

    let second = c.splice(4, "ship by September", "ship by October");
    assert_eq!(second["seq"], json!(2));
    assert_eq!(c.links()["changes_seq"], json!(2), "the counter advances");

    server.shutdown();
}

/// §7.1's detection clause, first half: detection ran only under a live
/// subscription, so an out-of-band edit with nobody watching was numbered by
/// nobody — and the read door said `0` while meaning "nothing changed".
///
/// A read door now runs a detection cycle, but it never *primes*: adopting a
/// baseline costs a full corpus snapshot, and one read op is one fold here.
/// So an epoch no subscriber ever opened numbers nothing, and `0` keeps its
/// only honest reading — "aligned to no retained frame" — which the epoch
/// identity beside it is what makes readable (§10.1).
#[test]
fn an_epoch_no_one_ever_subscribed_to_numbers_nothing_and_says_so() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp.path().join("ws"), &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    let mut c = Conn::open(server.socket_path());
    assert_eq!(c.hello_v3(&ws)["ok"], json!(true));
    let baseline = c.links();
    assert_eq!(baseline["changes_seq"], json!(0));

    // The door nobody watches: a human in their editor.
    fs::write(
        ws.join("plan.md"),
        "# Goals\n\nship by September\n\n# Notes\n\nnothing yet\n",
    )
    .unwrap();

    let after = c.links();
    assert_ne!(
        after["as_of_fingerprint"], baseline["as_of_fingerprint"],
        "the edit landed: {after}"
    );
    assert_eq!(
        after["changes_seq"],
        json!(0),
        "and an unprimed epoch numbered it with nothing: {after}"
    );
    assert_eq!(
        after["tree_instance"], baseline["tree_instance"],
        "one epoch throughout, so the two zeroes are comparable: {after}"
    );

    server.shutdown();
}

/// §7.1's detection clause, second half: once an epoch HAS a baseline, the
/// cycle a view-shaped read runs is the one that numbers out-of-band change —
/// so a reader is served a counter that covers an edit no subscriber was
/// listening for. The subscription here is the baseline's origin, not the
/// detector: it is gone before the edit is made.
///
/// *Mutation:* leave the read doors calling `detect` (cadence-coalesced) or
/// nothing at all and the post-edit counter stays at the write's own `1` while
/// the fingerprint moves past it.
#[test]
fn a_primed_epoch_numbers_an_out_of_band_edit_for_a_reader() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp.path().join("ws"), &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    let mut c = Conn::open(server.socket_path());
    assert_eq!(c.hello_v3(&ws)["ok"], json!(true));

    // A subscriber opens the epoch's baseline and leaves. Its own connection is
    // push-only after the ack, so it is dropped here and watches nothing after.
    {
        let mut watcher = Conn::open(server.socket_path());
        assert_eq!(watcher.hello_v3(&ws)["ok"], json!(true));
        let ack = watcher.call(&json!({"id": 70, "op": "sub"}));
        assert_eq!(ack["ok"], json!(true), "the subscription is acked: {ack}");
    }

    let before = c.links();
    let numbered_before = before["changes_seq"].as_u64().expect("a counter");

    fs::write(
        ws.join("plan.md"),
        "# Goals\n\nship by September\n\n# Notes\n\nnothing yet\n",
    )
    .unwrap();

    let after = c.links();
    assert_ne!(
        after["as_of_fingerprint"], before["as_of_fingerprint"],
        "the edit landed: {after}"
    );
    assert_eq!(
        after["changes_seq"].as_u64(),
        Some(numbered_before + 1),
        "and the read numbered it, with no subscriber anywhere: {after}"
    );
    assert_eq!(
        c.mint()["seq"].as_u64(),
        Some(numbered_before + 1),
        "the mint reads the same ring as the view door"
    );

    server.shutdown();
}

/// The counter is a workspace-grain fact. A scoped mint names a NODE, so no
/// retained frame ends at its token: `seq: 0`, no epoch named (§4.7).
#[test]
fn a_scoped_mint_names_a_node_so_it_carries_no_counter() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp.path().join("ws"), &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    let mut c = Conn::open(server.socket_path());
    assert_eq!(c.hello_v3(&ws)["ok"], json!(true));
    let put = c.splice(3, "ship by August", "ship by September");
    assert_eq!(put["seq"], json!(1));

    let scoped = c.call(&json!({"id": 91, "op": "fingerprint", "scope": "plan.md"}));
    assert_eq!(scoped["ok"], json!(true), "{scoped}");
    assert_eq!(scoped["body"]["seq"], json!(0), "node grain: {scoped}");
    assert!(
        scoped["body"].get("tree_instance").is_none(),
        "and it names no epoch: {scoped}"
    );
    // The world mint, same connection, still carries both.
    assert_eq!(c.mint()["seq"], json!(1));

    server.shutdown();
}

/// Frozen v2 never grows a field: the §4.6 key set predates the counter's
/// identity, so a v2 session is served the number alone. The number itself is
/// v2-legal — `changes_seq` and the mint's `seq` are frozen fields whose
/// value was the defect.
///
/// *Mutation:* drop the `tree_instance` row from `rev::V2_RESERVED_FIELDS`
/// and the v2 `links` body grows a key its session was never promised.
#[test]
fn a_frozen_v2_session_is_served_the_number_without_the_epoch() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp.path().join("ws"), &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();

    let mut c = Conn::open(server.socket_path());
    assert_eq!(c.hello_v2(&ws)["ok"], json!(true));
    let put = c.splice(3, "ship by August", "ship by September");
    assert_eq!(put["seq"], json!(1));

    let links = c.links();
    assert_eq!(links["changes_seq"], json!(1), "the counter is served");
    assert!(
        links.get("tree_instance").is_none(),
        "and its v3 identity is not: {links}"
    );
    let mut keys: Vec<&str> = links
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec!["as_of_root", "changes_seq", "files", "live_root"],
        "the frozen §4.6 key set: {links}"
    );

    let mint = c.mint_op("root");
    let mut mint_keys: Vec<&str> = mint
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    mint_keys.sort_unstable();
    assert_eq!(
        mint_keys,
        vec!["root", "seq"],
        "the frozen §4.7 pair: {mint}"
    );
    assert_eq!(mint["seq"], json!(1));

    server.shutdown();
}
