//! Exhaustive key-set pins for the daemon socket's served shapes (v3 session).
//!
//! Every pin is a full sorted key list, never a subset or a `contains` — a
//! subset check cannot catch a field leaking onto a surface it was never
//! designed for. Pins record the wire as served; where the served shape and a
//! document disagree the pin follows the wire and says so in its comment.
//! Deliberate coverage gaps are listed at the foot of this file.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use registry::{Config, RunningServer};
use serde_json::{Value, json};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A daemon config rooted under `tmp`, reap horizons large enough that the
/// background reaper never evicts a warm engine mid-test. Built by mutating the
/// production default so a new `Config` field cannot break this suite's compile.
#[allow(clippy::duration_suboptimal_units)]
fn test_config(tmp: &TempDir) -> Config {
    let forever = Duration::from_secs(365 * 24 * 60 * 60);
    let mut config = Config::for_cache_root(tmp.path().join("cache"));
    config.idle_threshold = forever;
    config.reap_interval = forever;
    config.prewarm_interval = forever;
    config.prewarm_quiet_max = forever;
    // Lifetime is the test's; idle-exit would flake mid-assertion.
    config.idle_exit = None;
    // A build sha, so the hello pin covers the shape a deployed daemon emits
    // rather than the identity-less variant.
    config.build_sha = Some("pinfixturebuild01".to_owned());
    config
}

/// A workspace `tmp/ws` seeded with `files` — a sibling of the cache root, so the
/// corpus walk never sees the drawer.
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
        let mut line = serde_json::to_string(request).unwrap();
        line.push('\n');
        self.call_line(&line)
    }

    /// One raw frame in, one frame out — for lexemes no `Value` can carry
    /// (a `2^64` id overflows `u64`, and re-serializing it as f64 would
    /// change the lexeme under test).
    fn call_line(&mut self, line: &str) -> Value {
        self.writer.write_all(line.as_bytes()).unwrap();
        self.writer.flush().unwrap();
        let mut response = String::new();
        self.reader.read_line(&mut response).unwrap();
        serde_json::from_str(&response).unwrap()
    }

    /// Bind `ws` on a v3 session — the vocabulary this daemon will still speak
    /// after R3b removes v2.
    fn hello(&mut self, ws: &Path) -> Value {
        self.call(&json!({
            "op": "hello", "proto": 1, "contract": "v3",
            "workspace": ws.to_str().unwrap(),
        }))
    }
}

/// The corpus every pin below is computed against: one plan with a wikilink and
/// two sections, plus the link target and a receipt file for the guarded write.
const PLAN: &str = "# Goals\n\nsee [[b]]\n\n## Q3\n\nship by August\n\n## Q4\n\nitem one\n";
const RECEIPTS: &str = "# Receipts\n";

fn corpus() -> [(&'static str, &'static str); 3] {
    [
        ("plan.md", PLAN),
        ("b.md", "# B\n"),
        ("receipts/2026-07-18.md", RECEIPTS),
    ]
}

/// A live daemon bound to a fresh corpus, plus its first connection.
struct Fixture {
    _tmp: TempDir,
    ws: PathBuf,
    server: RunningServer,
}

impl Fixture {
    fn start() -> (Self, Conn) {
        let tmp = TempDir::new().unwrap();
        let ws = write_ws(&tmp, &corpus());
        let server = RunningServer::start(test_config(&tmp)).unwrap();
        let mut conn = Conn::open(server.socket_path());
        assert_eq!(conn.hello(&ws)["ok"], json!(true), "the fixture binds");
        let fixture = Fixture {
            _tmp: tmp,
            ws,
            server,
        };
        (fixture, conn)
    }

    fn conn(&self) -> Conn {
        Conn::open(self.server.socket_path())
    }

    fn shutdown(self) {
        self.server.shutdown();
    }
}

/// Exhaustive sorted key-list pin — not subset, not `contains`.
#[track_caller]
fn pin_keys(value: &Value, expected: &[&str], what: &str) {
    let obj = value
        .as_object()
        .unwrap_or_else(|| panic!("{what} is not a JSON object: {value}"));
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys, expected,
        "served {what} key set drifted — a key was added, removed, or its \
         population rule changed. Frame: {value}"
    );
}

/// One heading's live `node_rev` off a `toc` frame (U10: the guard a wire-origin
/// write must carry).
fn node_rev(toc: &Value, heading: &str) -> String {
    toc["body"]["nodes"]
        .as_array()
        .expect("toc nodes array")
        .iter()
        .find(|n| {
            n["hpath"]
                .as_array()
                .is_some_and(|h| h.last().is_some_and(|seg| seg["h"] == json!(heading)))
        })
        .unwrap_or_else(|| panic!("heading {heading} in toc: {toc}"))["node_rev"]
        .as_str()
        .expect("node_rev string")
        .to_string()
}

/// The §4.4 guarded receipt-bearing write: the one request that puts `armed`,
/// `receipt`, an anchor row, and a delta notification on the wire at once.
fn guarded_write(conn: &mut Conn) -> Value {
    let toc = conn.call(&json!({"op": "toc", "path": "plan.md"}));
    let q3 = node_rev(&toc, "Q3");
    json!({
        "id": 42, "op": "splice", "path": "plan.md",
        "actor": "agent:b0864fb2", "now": "2026-07-18T20:31:04Z",
        "receipt": {"path": "receipts/2026-07-18.md", "anchor": "r-000042"},
        "edits": [{
            "target": {"hpath": [{"h": "Goals"}, {"h": "Q3"}]},
            "edit": {"match": {"old": "ship by August", "new": "ship by September"}},
            "if_node_rev": q3,
        }],
    })
}

// ---------------------------------------------------------------------------
// The frame envelope (§3.1)
// ---------------------------------------------------------------------------

/// Exactly one payload member rides — never `body` and `error` together — and
/// `meta` is present by design (in-band timing).
///
/// Both refusal envelopes are pinned: a frame refused before the dispatch shell
/// (an unknown op) carries no `meta`, while a refusal the engine itself
/// produced is timed like any other engine work.
#[test]
fn the_frame_envelope_carries_exactly_one_payload() {
    let (fx, mut conn) = Fixture::start();

    let ok = conn.call(&json!({"id": 11, "op": "fingerprint"}));
    pin_keys(&ok, &["body", "id", "meta", "ok"], "ok response frame");

    let pre_dispatch = conn.call(&json!({"id": 12, "op": "nope"}));
    pin_keys(
        &pre_dispatch,
        &["error", "id", "ok"],
        "pre-dispatch error response frame",
    );

    let dispatched = conn.call(&json!({"id": 13, "op": "toc", "path": "missing.md"}));
    pin_keys(
        &dispatched,
        &["error", "id", "meta", "ok"],
        "dispatched error response frame",
    );

    // `meta` rides beside the payload, never inside it. A timing block that
    // sank into `body` would be a payload field on every read.
    assert!(
        ok["body"].as_object().unwrap().get("meta").is_none(),
        "meta is not a body field: {ok}"
    );
    assert!(
        dispatched["error"]
            .as_object()
            .unwrap()
            .get("meta")
            .is_none(),
        "meta is not an error field: {dispatched}"
    );
    pin_keys(&ok["meta"], &["duration_us"], "meta block");

    fx.shutdown();
}

// ---------------------------------------------------------------------------
// hello (§3.2)
// ---------------------------------------------------------------------------

/// The daemon's hello body pins the drawer (`storage`) and echoes the
/// negotiated `contract`, the resolved `workspace`, and the build `identity` —
/// a resident daemon serving many workspaces must tell the client which one it
/// bound, and from which binary. The pin stops the handshake growing another
/// field unnoticed.
#[test]
fn the_hello_body_key_set_is_pinned() {
    let (fx, mut conn) = Fixture::start();
    let hi = conn.call(&json!({
        "op": "hello", "proto": 1, "contract": "v3",
        "workspace": fx.ws.to_str().unwrap(), "client": "md-cli/0.3",
    }));
    pin_keys(
        &hi["body"],
        &[
            "caps",
            "contract",
            "fingerprint",
            "identity",
            "proto",
            "server",
            "storage",
            "workspace",
        ],
        "hello body",
    );
    fx.shutdown();
}

// ---------------------------------------------------------------------------
// toc (§4.1) — body + every row class the contract works
// ---------------------------------------------------------------------------

/// The body's cursor key is `fingerprint`; the row shapes are
/// vocabulary-neutral, including the §2.1 `hpath` segment, whose object form is
/// pinned in both directions (decision 20).
#[test]
fn the_toc_body_and_row_key_sets_are_pinned() {
    let (fx, mut conn) = Fixture::start();
    let got = conn.call(&json!({"id": 2, "op": "toc", "path": "plan.md"}));
    pin_keys(
        &got["body"],
        &["file_rev", "fingerprint", "nodes", "path"],
        "toc body",
    );
    let nodes = got["body"]["nodes"].as_array().expect("nodes array");
    let heading = nodes
        .iter()
        .find(|n| n["kind"] == json!("heading"))
        .expect("the fixture carries headings");
    pin_keys(
        heading,
        &[
            "content_span",
            "hpath",
            "kind",
            "level",
            "node_rev",
            "span",
            "text_prefix_16b",
        ],
        "toc heading row",
    );
    pin_keys(
        &heading["hpath"].as_array().expect("hpath")[0],
        &["h"],
        "toc heading row hpath segment",
    );
    fx.shutdown();
}

/// Vocabulary-neutral: the anchor row is minted by the receipt append and
/// carries no cursor key at all.
#[test]
fn the_toc_anchor_row_key_set_is_pinned() {
    let (fx, mut conn) = Fixture::start();
    let write = guarded_write(&mut conn);
    let splice = conn.call(&write);
    assert_eq!(
        splice["ok"],
        json!(true),
        "the guarded write lands: {splice}"
    );

    let toc = conn.call(&json!({"id": 4, "op": "toc", "path": "receipts/2026-07-18.md"}));
    let anchor_row = toc["body"]["nodes"]
        .as_array()
        .expect("nodes array")
        .iter()
        .find(|n| n.get("anchor").is_some())
        .unwrap_or_else(|| panic!("the receipt append minted an anchor row: {toc}"));
    pin_keys(
        anchor_row,
        &["anchor", "kind", "node_rev", "span", "text_prefix_16b"],
        "toc anchor row",
    );
    fx.shutdown();
}

// ---------------------------------------------------------------------------
// cat (§4.2) · extract (§4.3) · resolve (§4.5)
// ---------------------------------------------------------------------------

/// Vocabulary-neutral — `cat` carries no cursor key in either form.
#[test]
fn the_cat_body_key_sets_are_pinned_both_forms() {
    let (fx, mut conn) = Fixture::start();
    let sec = conn.call(&json!({
        "id": 3, "op": "cat", "path": "plan.md",
        "sec": {"hpath": [{"h": "Goals"}, {"h": "Q3"}]},
    }));
    pin_keys(&sec["body"], &["content", "node_rev", "span"], "cat body");
    let whole = conn.call(&json!({"id": 9, "op": "cat", "path": "plan.md"}));
    pin_keys(
        &whole["body"],
        &["content", "node_rev", "span"],
        "cat whole-file body",
    );
    fx.shutdown();
}

/// On a v3 session the enriched keys are present, exactly: a heading node
/// carries `n`/`words`, a wikilink node carries neither. `hpath_text` is absent
/// from both — the joined string address is retired from machine surfaces.
#[test]
fn the_extract_body_and_node_key_sets_are_pinned() {
    let (fx, mut conn) = Fixture::start();
    let got = conn.call(&json!({"id": 5, "op": "extract", "path": "plan.md"}));
    pin_keys(&got["body"], &["nodes", "path"], "extract body");
    let nodes = got["body"]["nodes"].as_array().expect("nodes array");

    let heading = nodes
        .iter()
        .find(|n| n["kind"] == json!("heading"))
        .expect("the fixture carries headings");
    pin_keys(
        heading,
        &[
            "hpath",
            "kind",
            "n",
            "node_rev",
            "span",
            "text_prefix_16b",
            "words",
        ],
        "extract heading node (v3 host face)",
    );

    let wikilink = nodes
        .iter()
        .find(|n| n["kind"] == json!("wikilink"))
        .expect("the fixture carries wikilinks");
    pin_keys(
        wikilink,
        &["info", "kind", "node_rev", "span", "text_prefix_16b"],
        "extract wikilink node",
    );
    pin_keys(&wikilink["info"], &["target"], "extract wikilink info");
    fx.shutdown();
}

/// Vocabulary-neutral: §4.5 answers a location, never a rev (D-C2), so the
/// rename table never touches it.
#[test]
fn the_resolve_body_key_sets_are_pinned_both_forms() {
    let (fx, mut conn) = Fixture::start();
    let plain = conn.call(&json!({
        "id": 70, "op": "resolve", "from": "plan.md", "ref": "plan#Goals#Q3",
    }));
    pin_keys(&plain["body"], &["dest", "span"], "resolve body");
    let with_content = conn.call(&json!({
        "id": 71, "op": "resolve", "from": "plan.md", "ref": "plan#Goals#Q3",
        "content": true,
    }));
    pin_keys(
        &with_content["body"],
        &["content", "dest", "span"],
        "resolve body (content:true)",
    );
    fx.shutdown();
}

// ---------------------------------------------------------------------------
// links (§4.6) + the §10.1 staleness triple
// ---------------------------------------------------------------------------

/// The staleness triple is re-spelled; the corpus edge map is never re-keyed.
#[test]
fn the_links_body_and_file_key_sets_are_pinned() {
    let (fx, mut conn) = Fixture::start();
    let got = conn.call(&json!({"id": 80, "op": "links", "path": "plan.md"}));
    pin_keys(
        &got["body"],
        &[
            "as_of_fingerprint",
            "changes_seq",
            "files",
            "live_fingerprint",
        ],
        "links body",
    );
    pin_keys(
        &got["body"]["files"]["plan.md"],
        &["resolved", "unresolved"],
        "links file entry",
    );

    // Whole-corpus form: the same body shape, every domain file keyed — including
    // the link-LESS one, whose entry still carries both members.
    let all = conn.call(&json!({"id": 81, "op": "links"}));
    pin_keys(
        &all["body"],
        &[
            "as_of_fingerprint",
            "changes_seq",
            "files",
            "live_fingerprint",
        ],
        "links whole-corpus body",
    );
    pin_keys(
        &all["body"]["files"]["receipts/2026-07-18.md"],
        &["resolved", "unresolved"],
        "links link-less file entry",
    );
    fx.shutdown();
}

// ---------------------------------------------------------------------------
// splice (§4.4) — the ONE write response shape, real and dry
// ---------------------------------------------------------------------------

/// Only the transition slots are re-spelled; `armed`, the receipt fact, and the
/// armed edit are vocabulary-neutral. `armed.file_rev_after`
/// is pinned as served (decision 21): the whole-file rev after the write, so a
/// client skips a follow-up `toc`. Correctness still rides the fingerprint.
#[test]
fn the_splice_body_receipt_and_armed_key_sets_are_pinned() {
    let (fx, mut conn) = Fixture::start();
    let write = guarded_write(&mut conn);
    let got = conn.call(&write);
    assert_eq!(got["ok"], json!(true), "the guarded write lands: {got}");

    pin_keys(
        &got["body"],
        &[
            "armed",
            "fingerprint_after",
            "fingerprint_before",
            "receipt",
            "seq",
            "verdicts",
        ],
        "splice body",
    );
    pin_keys(
        &got["body"]["receipt"],
        &["anchor", "node_rev", "path", "span_after"],
        "splice receipt fact",
    );
    pin_keys(
        &got["body"]["armed"],
        &["edits", "file_rev_after", "path"],
        "splice armed (§4.4 as amended, decision 21)",
    );
    let edits = got["body"]["armed"]["edits"].as_array().expect("edits");
    pin_keys(
        &edits[0],
        &["node_rev_after", "node_rev_before", "span_after", "target"],
        "splice armed edit",
    );
    pin_keys(&edits[0]["target"], &["hpath"], "splice armed edit target");
    fx.shutdown();
}

/// The dry rehearsal is a different key set from the committed write, not the
/// same one with null values: no receipt, no `seq`, a `dry` flag, and a dry
/// `armed` that carries no `file_rev_after` (nothing was written, so the
/// file-grain post-rev does not exist). `fingerprint_after` is null, never
/// absent — a client reading the transition slot must see the slot.
#[test]
fn the_splice_dry_body_key_set_is_pinned() {
    let (fx, mut conn) = Fixture::start();
    let toc = conn.call(&json!({"op": "toc", "path": "plan.md"}));
    let q3 = node_rev(&toc, "Q3");
    let got = conn.call(&json!({
        "id": 60, "op": "splice", "path": "plan.md", "dry": true,
        "edits": [{
            "target": {"hpath": [{"h": "Goals"}, {"h": "Q3"}]},
            "edit": {"match": {"old": "ship by August", "new": "ship by September"}},
            "if_node_rev": q3,
        }],
    }));
    assert_eq!(
        got["ok"],
        json!(true),
        "the dry rehearsal answers ok: {got}"
    );
    pin_keys(
        &got["body"],
        &[
            "armed",
            "dry",
            "fingerprint_after",
            "fingerprint_before",
            "verdicts",
        ],
        "splice dry body",
    );
    assert_eq!(
        got["body"]["fingerprint_after"],
        Value::Null,
        "§4.4: the transition slot is NULL on a dry run, never absent: {got}"
    );
    pin_keys(
        &got["body"]["armed"],
        &["edits", "path"],
        "splice dry armed",
    );
    fx.shutdown();
}

// ---------------------------------------------------------------------------
// fingerprint · sub · diff (§4.7)
// ---------------------------------------------------------------------------

/// Cursor keys re-spelled; `seq` and `batches` are vocabulary-neutral.
#[test]
fn the_cursor_sub_ack_and_diff_body_key_sets_are_pinned() {
    let (fx, mut conn) = Fixture::start();
    let cursor = conn.call(&json!({"id": 90, "op": "fingerprint"}));
    pin_keys(&cursor["body"], &["fingerprint", "seq"], "fingerprint body");
    let live = cursor["body"]["fingerprint"].as_str().expect("cursor");

    let diff = conn.call(&json!({
        "id": 95, "op": "diff", "from_fingerprint": live, "to_fingerprint": live,
    }));
    pin_keys(&diff["body"], &["batches"], "diff body");

    // `sub` turns its connection push-only, so it rides its own. Live
    // subscribe (B-01): no cursor; the ack teaches the cursor identity.
    let mut sub_conn = fx.conn();
    assert_eq!(sub_conn.hello(&fx.ws)["ok"], json!(true));
    let ack = sub_conn.call(&json!({"id": 91, "op": "sub"}));
    pin_keys(
        &ack["body"],
        &["fingerprint", "seq", "tree_instance"],
        "sub ack body",
    );

    fx.shutdown();
}

// ---------------------------------------------------------------------------
// The §8 error envelopes — per worked code
// ---------------------------------------------------------------------------

/// The shape before any demotion — the full retry ladder. `rev::demote_v2`
/// drops the ladder extras; this wire never demotes.
#[test]
fn the_cas_mismatch_error_key_set_is_pinned() {
    let (fx, mut conn) = Fixture::start();
    let write = guarded_write(&mut conn);
    assert_eq!(
        conn.call(&write)["ok"],
        json!(true),
        "the first write lands"
    );

    // The same guard replayed: the section rev moved under it.
    //
    // The anchor is FRESH on the retry, and that is load-bearing rather than
    // cosmetic. `guarded_write` hardcodes `r-000042`, which the first call above
    // committed into the receipt file — so replaying the request verbatim also
    // replays the anchor, and §6.6 resolves that collision FIRST and refuses
    // `bad_request` before the stale rev this test exists to pin is ever
    // reached. A fresh anchor per write is the engine's own rule
    // (`crates/realise/src/lib.rs` mints one per attempt) and `r-000043` is
    // `crates/wire/tests/contract_v2.rs:559`'s convention for exactly this.
    // The ordering itself is pinned by
    // `crates/wire-serve/tests/s13_88_receipt_anchor_collision.rs`, not here.
    let mut stale = write.clone();
    stale["receipt"]["anchor"] = json!("r-000043");
    let stale_retry = conn.call(&stale);
    assert_eq!(
        stale_retry["error"]["code"],
        json!("cas_mismatch"),
        "the stale-rev retry refuses: {stale_retry}"
    );
    pin_keys(
        &stale_retry["error"],
        &[
            "actual",
            "code",
            "diff",
            "expected",
            "message",
            "new_fingerprint",
            "path",
            "recovery",
            "rung",
        ],
        "cas_mismatch error (v3, ladder extras present)",
    );
    // The extras are the ladder: the refusal hands back the current rev and
    // the diff, so a client re-sends without re-reading. `recovery` is
    // `refresh`, not `resync` — a section-grain conflict is recoverable in place.
    assert_eq!(
        stale_retry["error"]["recovery"],
        json!("refresh"),
        "{stale_retry}"
    );
    assert_eq!(
        stale_retry["error"]["new_fingerprint"], stale_retry["error"]["actual"],
        "the ladder hands back the rev the client must guard on next: {stale_retry}"
    );
    fx.shutdown();
}

/// Vocabulary-neutral: `matches` is the only extra either refusal carries.
#[test]
fn the_no_match_and_not_unique_error_key_sets_are_pinned() {
    let (fx, mut conn) = Fixture::start();
    let toc = conn.call(&json!({"op": "toc", "path": "plan.md"}));
    let q3 = node_rev(&toc, "Q3");
    let q4 = node_rev(&toc, "Q4");

    let no_match = conn.call(&json!({
        "id": 89, "op": "splice", "path": "plan.md",
        "edits": [{
            "target": {"hpath": [{"h": "Goals"}, {"h": "Q3"}]},
            "edit": {"match": {"old": "ship by October", "new": "x"}},
            "if_node_rev": q3,
        }],
    }));
    assert_eq!(no_match["error"]["code"], json!("no_match"), "{no_match}");
    pin_keys(
        &no_match["error"],
        &["code", "matches", "recovery"],
        "no_match error",
    );

    // `not_unique` needs a second occurrence of "item" inside Q4.
    let append = conn.call(&json!({
        "id": 57, "op": "splice", "path": "plan.md",
        "edits": [{
            "target": {"hpath": [{"h": "Goals"}, {"h": "Q4"}]},
            "edit": {"put": {"at": "end", "text": "- new item\n"}},
            "if_node_rev": q4,
        }],
    }));
    assert_eq!(append["ok"], json!(true), "the append lands: {append}");
    let q4_after = node_rev(&conn.call(&json!({"op": "toc", "path": "plan.md"})), "Q4");
    let not_unique = conn.call(&json!({
        "id": 92, "op": "splice", "path": "plan.md",
        "edits": [{
            "target": {"hpath": [{"h": "Goals"}, {"h": "Q4"}]},
            "edit": {"match": {"old": "item", "new": "entry"}},
            "if_node_rev": q4_after,
        }],
    }));
    assert_eq!(
        not_unique["error"]["code"],
        json!("not_unique"),
        "{not_unique}"
    );
    assert_eq!(
        not_unique["error"]["matches"],
        json!(2),
        "the refusal counts what it saw: {not_unique}"
    );
    pin_keys(
        &not_unique["error"],
        &["code", "matches", "recovery"],
        "not_unique error",
    );
    fx.shutdown();
}

/// Vocabulary-neutral: stage 2 got far enough to name a `dest`; stage 1 did
/// not. Both shapes are pinned — collapsing them loses the walk's diagnostic.
#[test]
fn the_ref_not_found_error_key_sets_are_pinned_both_stages() {
    let (fx, mut conn) = Fixture::start();
    let stage2 = conn.call(&json!({
        "id": 73, "op": "resolve", "from": "plan.md", "ref": "plan#Goals#Q9",
    }));
    pin_keys(
        &stage2["error"],
        &["code", "dest", "recovery", "stage"],
        "ref_not_found stage-2 error",
    );
    let stage1 = conn.call(&json!({
        "id": 74, "op": "resolve", "from": "plan.md", "ref": "roadmap",
    }));
    pin_keys(
        &stage1["error"],
        &["code", "recovery", "stage"],
        "ref_not_found stage-1 error",
    );
    fx.shutdown();
}

/// All vocabulary-neutral — they refuse the request before any cursor is in
/// play.
#[test]
fn the_remaining_error_key_sets_are_pinned() {
    let (fx, mut conn) = Fixture::start();

    let kinds = conn.call(&json!({
        "id": 21, "op": "extract", "path": "plan.md", "kinds": ["bogus"],
    }));
    pin_keys(
        &kinds["error"],
        &["code", "recovery", "unknown_kinds"],
        "bad_request{unknown_kinds} error",
    );

    let toc = conn.call(&json!({"op": "toc", "path": "plan.md"}));
    let q3 = node_rev(&toc, "Q3");
    // Region-grain overlap (§4.4): the second needle's bytes sit inside the
    // first's matched region — same-target edits on DISJOINT bytes are legal.
    let overlap = conn.call(&json!({
        "id": 22, "op": "splice", "path": "plan.md",
        "edits": [
            {"target": {"hpath": [{"h": "Goals"}, {"h": "Q3"}]},
             "edit": {"match": {"old": "ship by August", "new": "a"}}, "if_node_rev": q3},
            {"target": {"hpath": [{"h": "Goals"}, {"h": "Q3"}]},
             "edit": {"match": {"old": "August", "new": "b"}}, "if_node_rev": q3},
        ],
    }));
    pin_keys(
        &overlap["error"],
        &["code", "message", "overlap", "recovery"],
        "bad_request{overlap} error",
    );

    let proto = conn.call(&json!({"id": 23, "op": "hello", "proto": 99}));
    pin_keys(
        &proto["error"],
        &["code", "recovery", "supported"],
        "unsupported_proto error",
    );

    let missing = conn.call(&json!({"id": 24, "op": "toc", "path": "missing.md"}));
    // `message` joined deliberately (card p2-dogfood-refusal-teaching): the
    // domain-scoped miss teaches instead of echoing a bare token.
    pin_keys(
        &missing["error"],
        &["code", "message", "path", "recovery"],
        "file_not_found error",
    );

    let bad_path = conn.call(&json!({"id": 25, "op": "toc", "path": "../escape.md"}));
    pin_keys(
        &bad_path["error"],
        &["code", "path", "recovery"],
        "bad_path error",
    );

    let unknown_op = conn.call(&json!({"id": 26, "op": "nope"}));
    pin_keys(
        &unknown_op["error"],
        &["code", "recovery"],
        "unknown_op error",
    );

    // The daemon served `view_path` until it was DROPPED by ruling
    // (wire-contract §10.4, 2026-08-06). The dropped name must answer
    // unknown_op like any stranger — never a partial dispatch remnant.
    let dropped = conn.call(&json!({"id": 27, "op": "view_path"}));
    assert_eq!(
        dropped["error"]["code"],
        json!("unknown_op"),
        "the dropped view_path op refuses clean: {dropped}"
    );
    pin_keys(
        &dropped["error"],
        &["code", "recovery"],
        "unknown_op (dropped view_path) error",
    );
    fx.shutdown();
}

/// Vocabulary-neutral, and the most-served refusal on this wire: what a
/// guardless write meets first. Its `message`/`path` are pinned so the refusal
/// cannot grow a sibling field unseen.
#[test]
fn the_guard_required_error_key_set_is_pinned() {
    let (fx, mut conn) = Fixture::start();
    let got = conn.call(&json!({
        "id": 27, "op": "splice", "path": "plan.md",
        "edits": [{
            "target": {"hpath": [{"h": "Goals"}, {"h": "Q3"}]},
            "edit": {"match": {"old": "ship by August", "new": "x"}},
        }],
    }));
    assert_eq!(
        got["error"]["code"],
        json!("guard_required"),
        "a guardless write is refused: {got}"
    );
    pin_keys(
        &got["error"],
        &["code", "message", "path", "recovery"],
        "guard_required error",
    );
    fx.shutdown();
}

// ---------------------------------------------------------------------------
// §3.1 id discipline — the raw-lexeme law, served at the daemon door
// ---------------------------------------------------------------------------

/// §3.1: a non-integer `id` lexeme is refused `bad_request` echoing `id_raw` —
/// the daemon answers the law at frame classification, BEFORE decode/dispatch
/// (`transport::scan_id` at the door), and never serves the request. The error
/// frame carries `id:null`; a single-shot client reads `id_raw` for the
/// offending lexeme verbatim.
#[test]
fn a_non_integer_id_is_refused_with_id_raw_at_the_daemon_door() {
    let (fx, mut conn) = Fixture::start();
    let raw_id = conn.call(&json!({"id": "7", "op": "fingerprint"}));
    assert_eq!(
        raw_id["ok"],
        json!(false),
        "the daemon refuses the frame rather than serving it: {raw_id}"
    );
    assert_eq!(
        raw_id["id"],
        Value::Null,
        "the non-conforming lexeme is never echoed as a valid id: {raw_id}"
    );
    assert_eq!(
        raw_id["error"]["code"],
        json!("bad_request"),
        "one malformed-envelope code (§8 W4: bad_id folded into bad_request): {raw_id}"
    );
    assert_eq!(
        raw_id["error"]["id_raw"],
        json!("\"7\""),
        "the offending lexeme, verbatim — quotes kept: {raw_id}"
    );
    assert_eq!(
        raw_id["error"]["recovery"],
        json!("fix"),
        "the §8 binding: the envelope is the caller's to fix (the respawn \
         consequence is the client-side null-id corruption law, §3.1, keyed \
         off the frame header — not this field): {raw_id}"
    );
    pin_keys(
        &raw_id,
        &["error", "id", "ok"],
        "non-integer-id refusal frame",
    );
    pin_keys(
        &raw_id["error"],
        &["code", "id_raw", "recovery"],
        "bad_request{id_raw} error",
    );
    fx.shutdown();
}

/// Contract §3.1: the same request is a `bad_request` echoing `id_raw`.
/// Served since the door scan landed (2026-08-12) — the R3a gap this test was
/// ignored for is closed; row 9's disposition is this refusal.
#[test]
fn contract_3_1_a_non_integer_id_is_refused_with_id_raw() {
    let (fx, mut conn) = Fixture::start();
    let raw_id = conn.call(&json!({"id": "7", "op": "fingerprint"}));
    assert_eq!(raw_id["ok"], json!(false), "{raw_id}");
    pin_keys(
        &raw_id["error"],
        &["code", "id_raw", "recovery"],
        "bad_request{id_raw} error",
    );
    fx.shutdown();
}

/// Contract §3.1: an oversized numeric id (`2^64`, overflowing `u64`) is the
/// same refusal — a bad request, never silently reclassified as a notification.
#[test]
fn an_out_of_range_id_is_refused_never_reclassified_as_notification() {
    let (fx, mut conn) = Fixture::start();
    let raw_id = conn.call_line("{\"id\":18446744073709551616,\"op\":\"fingerprint\"}\n");
    assert_eq!(raw_id["ok"], json!(false), "{raw_id}");
    assert_eq!(raw_id["id"], Value::Null, "{raw_id}");
    assert_eq!(
        raw_id["error"]["id_raw"],
        json!("18446744073709551616"),
        "2^64 refuses as itself: {raw_id}"
    );
    pin_keys(
        &raw_id["error"],
        &["code", "id_raw", "recovery"],
        "bad_request{id_raw} error (2^64)",
    );
    fx.shutdown();
}

// ---------------------------------------------------------------------------
// pin_keys self-control
// ---------------------------------------------------------------------------

/// The primitive travels with its own control: weakening `pin_keys` to a subset
/// check would leave every pin in this file compiling, passing, and asserting
/// nothing.
#[test]
fn the_pin_primitive_rejects_a_superset() {
    let one_extra = json!({"a": 1, "b": 2});
    let caught = std::panic::catch_unwind(|| {
        pin_keys(&one_extra, &["a"], "pin-primitive self-control");
    });
    assert!(
        caught.is_err(),
        "pin_keys accepted a key set with an EXTRA key — it is no longer \
         exhaustive, and every pin in this file is now a decoration"
    );
    // And it accepts the exact set, so the control tells the two worlds apart
    // rather than only proving that something panics.
    pin_keys(&one_extra, &["a", "b"], "pin-primitive self-control");
}

// ---------------------------------------------------------------------------
// script (§ A.7) — the trace body, its row classes, and the embedded leg
// ---------------------------------------------------------------------------

/// The committed script body: the `ScriptTrace` as served, with the §4.4 splice
/// response EMBEDDED verbatim — so the embedded leg's key set is the splice
/// body's own (minus `receipt`: none was named), and the fingerprint keys
/// inside it ride the v3 vocabulary.
#[test]
fn the_script_body_trace_rows_and_embedded_leg_key_sets_are_pinned() {
    let (fx, mut conn) = Fixture::start();
    // An append rather than a props write: the fixture's plan.md carries no
    // frontmatter, and the plan lowering refuses a property with nothing to
    // anchor it — the section append is the corpus's own committed shape.
    let got = conn.call(&json!({
        "id": 41, "op": "script",
        "source": "t = read(\"plan.md\")\nput(\"plan.md\", section=\"Goals/Q4\", append=\"- pinned item\\n\")\n",
        "actor": "agent:pin", "now": "2026-08-12T00:00:00Z",
    }));
    assert_eq!(got["ok"], json!(true), "the script commits: {got}");
    let body = &got["body"];
    assert_eq!(body["outcome"], json!("committed"), "{body}");
    pin_keys(
        body,
        &[
            "armed_digest",
            "commit",
            "entry_fingerprint",
            "outcome",
            "telemetry",
            "trace",
        ],
        "script body (committed)",
    );
    pin_keys(
        &body["telemetry"],
        &["fuel_used", "mem_used", "reads_used", "wall_ms"],
        "script telemetry",
    );
    let rows = body["trace"].as_array().expect("trace rows");
    let echo = rows
        .iter()
        .find(|r| r["kind"] == json!("echo"))
        .expect("the top-level read echoes");
    pin_keys(echo, &["face", "kind", "line", "path"], "script echo row");
    let armed = rows
        .iter()
        .find(|r| r["kind"] == json!("armed"))
        .expect("the put arms");
    pin_keys(
        armed,
        &["committed", "depth", "edit", "kind", "line", "path"],
        "script armed row",
    );
    // The embedded commit leg IS the §4.4 splice response: same key set as
    // the splice pin above (no `receipt` — none was named), v3 fingerprint
    // spelling inside the embedded bytes.
    pin_keys(
        &body["commit"],
        &[
            "armed",
            "fingerprint_after",
            "fingerprint_before",
            "seq",
            "verdicts",
        ],
        "script embedded commit leg",
    );
    fx.shutdown();
}

/// The read-class script body: no commit leg, no armed digest, no fault — the
/// key set is exactly the premise triple plus the trace.
#[test]
fn the_script_read_class_body_key_set_is_pinned() {
    let (fx, mut conn) = Fixture::start();
    let got = conn.call(&json!({
        "id": 42, "op": "script", "source": "t = read(\"plan.md\")\n",
    }));
    assert_eq!(got["ok"], json!(true), "{got}");
    assert_eq!(got["body"]["outcome"], json!("no_effect"));
    pin_keys(
        &got["body"],
        &["entry_fingerprint", "outcome", "telemetry", "trace"],
        "script body (read-class)",
    );
    fx.shutdown();
}

// ---------------------------------------------------------------------------
// § Deliberate gaps
// ---------------------------------------------------------------------------
//
// - root_mismatch: pinned on the socket plane by
//   `crates/registry/tests/root_mismatch_wire_shape.rs`, not duplicated here.
// - Delta notifications: `crates/registry/tests/sub_push.rs` drives the real
//   subscription and asserts Delta values, not an exhaustive key set.
// - Verdict: unreachable on this wire — the daemon serves with no rule packs
//   (`&[]` ⇒ `verdicts: []`), so no §11.1 Verdict can be pinned from a test.
