//! Stage-2 S6 gates: read IS the mint (D6/D9/D13/D16), driven single-session
//! **in-process** against the real resident daemon — ONE daemon holding the
//! receipt ledger across a read and the write that follows it.
//!
//! Every gate here goes over the daemon's own socket, so what is under test is
//! the PRODUCTION wiring (`dispatch_read` → `composed_read_warm` → the arm's
//! mint), never a hand-rolled re-call of the arm. Two CLI processes could not
//! share this ledger by design (it is daemon-session memory), so a
//! two-process shape would test the wrong thing.

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
    let dir = tmp.path().join("registry");
    Config {
        socket_path: dir.join("daemon.sock"),
        state_path: dir.join("state.json"),
        cache_root: tmp.path().join("cache"),
        idle_threshold: Duration::from_secs(365 * 24 * 60 * 60),
        reap_interval: Duration::from_secs(365 * 24 * 60 * 60),
        prewarm_interval: Duration::from_secs(365 * 24 * 60 * 60),
    }
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
/// rides the SAME connection (the per-connection binding the handshake sets up).
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

    /// The AGENT path: a composed sections read carrying the daemon-derived
    /// actor. (`actor` is a wire field the DAEMON stamps — D13; no MCP caller
    /// can set it, and nothing here widens that.)
    fn read_sections(&mut self, path: &str, selectors: &[&str], actor: Option<&str>) -> Value {
        let mut frame = json!({
            "op": "read",
            "path": path,
            // U14: the wire takes TAGGED selectors. The helper keeps its
            // ergonomic &[&str] and converts at its own door — which is
            // exactly the ingress discipline the wire now enforces.
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
        // U14: `sel` is echoed in the caller's own TAGGED grammar, so the
        // match is structure-to-structure rather than string-to-string.
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

/// Gate 1 — **the mint happens.** An agent-path read (a daemon-derived actor on
/// the wire) mints exactly one receipt per section it SERVED, carrying the three
/// D9 facts: actor, canonical selector, and the `sec_rev` of the bytes served.
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

/// Gate 2 — **the receipt survives the write** (H1, the failure mode this unit
/// exists to prevent). A commit through the daemon's own write path moves the
/// corpus content hash, so the next warm REBUILDS the engine — and a receipt
/// hung off that engine would evaporate exactly when a pin needs it. The write
/// here touches a DIFFERENT file (as a pin's lock write does), so the receipt
/// must not only survive but stay CURRENT: its rev still matches what the
/// target section serves after the rebuild.
#[test]
fn the_receipt_survives_the_write_that_rebuilds_the_warm_engine() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("plan.md", PLAN), ("notes.md", NOTES)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    conn.hello(&ws);

    let read = conn.read_sections("plan.md", &["Alpha"], Some("agent-7"));
    let read_rev = served_sec_rev(&read, "Alpha");

    // The write: a real commit through the shared `splice → commit`
    // choke-point, landing in a sibling file — the shape of a pin's own lock
    // write. U10: this arrives through the WIRE door, so it carries the
    // section's fingerprint — read first, then write what you read.
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

    // The rebuild is not assumed: `Built` is the parse-count proof that the warm
    // engine was thrown away and rebuilt at the new fingerprint.
    let canonical = workspace::canonicalize(&ws).unwrap();
    assert_eq!(
        server.registry().warm_or_build(&canonical).unwrap(),
        registry::WarmOutcome::Built { docs: 2 },
        "the write rebuilt the warm engine"
    );

    // The ledger is NOT the engine's: the receipt is still there…
    let mints = server.registry().read_mints(&canonical);
    let receipt = mints
        .lookup("agent-7", "plan.md", &wire::ReadSel::parse("Alpha"))
        .expect("the pin's own write must not evaporate the receipt that authorizes it");
    assert_eq!(receipt.sec_rev, read_rev, "…carrying its minted rev");

    // …and still CURRENT: the untouched section serves the same rev, so a gate
    // re-checking the receipt against disk finds it valid.
    let after = conn.read_sections("plan.md", &["Alpha"], None);
    assert_eq!(
        served_sec_rev(&after, "Alpha"),
        receipt.sec_rev,
        "the receipt's rev still matches the target section after the write"
    );

    server.shutdown();
}

/// Gate 3 — **selector grain (D6).** Reading section A does not satisfy a
/// lookup for section B. Doc-level receipts would let an agent attest bytes it
/// never saw.
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

/// Gate 4 — **actor isolation (D13).** A foreign actor's read mints for that
/// actor alone; my lookup does not see it. The ledger key starts with the
/// daemon-derived actor precisely so one agent's context never authorizes
/// another's attestation.
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

/// Gate 5 — **the CLI bypass (D16).** A read with no actor mints nothing: the
/// bare CLI is local-operator-trusted and bypasses the gate entirely, exactly
/// as `mrd put` skips the host's authz. A blank actor is treated as absent too
/// — an empty string is not an identity.
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

/// Gate 6 — a **toc-mode** read mints nothing. Toc serves the section MAP, not
/// content; minting there would gate an attestation over bytes nobody saw.
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

/// Gate 7 — **no persistence (D6).** The mint path writes NOTHING to disk: the
/// whole sandbox (workspace, cache drawer, daemon state file) is byte-for-byte
/// identical across an agent read that provably minted.
#[test]
fn the_mint_path_writes_nothing_to_disk() {
    let tmp = TempDir::new().unwrap();
    let ws = write_ws(&tmp, &[("plan.md", PLAN)]);
    let server = RunningServer::start(test_config(&tmp)).unwrap();
    let mut conn = Conn::open(server.socket_path());
    // Bind first: `hello` legitimately registers the workspace and writes the
    // drawer sentinel + state file, so the snapshot is taken AFTER it — what is
    // under test is the READ.
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
