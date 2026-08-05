//! Exhaustive key-set pins for the daemon socket's served shapes (v3 session).
//!
//! The LIVE half of the U27 frozen-key-set pair lives in
//! `crates/sidecar/tests/u27_v2_key_set_pins.rs`, which drives `sidecar::serve`.
//! Unit R3b deletes that crate, and with it the only live-plane place where a
//! response shape is pinned EXHAUSTIVELY. This suite rehomes that mechanism onto
//! the daemon socket — the surface that outlives the sidecar.
//!
//! # What "outlives v2" means here
//!
//! The u27 pins are stated in the FROZEN V2 vocabulary, and R3b also removes v2
//! rev support, so a verbatim copy would pin a wire nobody will serve. The
//! criterion applied, stated once so a reader can check every pin below against
//! it:
//!
//! > **An assertion outlives v2 iff its subject survives the v2→v3 projection in
//! > `crates/wire-serve/src/rev.rs`.** Concretely: a pin over keys the rename
//! > table does not touch is rehomed verbatim; a pin over a renamed cursor key
//! > (`root*` → `fingerprint*`) is rehomed re-spelled; and a pin asserting the
//! > ABSENCE of a v3-only key on a v2 session (no `meta`, no `fingerprint`, no
//! > `n`/`words`) dies with v2 — it has no v3 statement, because on v3 the key is
//! > present by design.
//!
//! Three u27 pins are deliberately NOT rehomed here, each for a stated reason;
//! see § Not rehomed at the foot of this file.
//!
//! # Why exhaustive
//!
//! Every pin is a full sorted key LIST, never a subset or a `contains`. A subset
//! check cannot catch the failure these pins exist for: a field that leaks onto a
//! surface it was never designed for. `Option` + `skip_serializing_if` is not a
//! version gate, and a value sweep misses field growth entirely.
//!
//! Pins record the wire AS SERVED. Where the served shape and a document disagree
//! the pin follows the wire and says so in its comment — a pin that encodes what
//! the wire ought to be is a wish, and it fails on the day someone fixes nothing.

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
/// background reaper never evicts a warm engine mid-test.
///
/// Built by MUTATING the production layout rather than by a struct literal: the
/// literal form (copied across the older registry suites) stops compiling the day
/// `Config` grows a field, turning one unit's change into a compile break in
/// another unit's test file. This form names only what the test cares about.
#[allow(clippy::duration_suboptimal_units)]
fn test_config(tmp: &TempDir) -> Config {
    let forever = Duration::from_secs(365 * 24 * 60 * 60);
    let mut config = Config::for_cache_root(tmp.path().join("cache"));
    config.idle_threshold = forever;
    config.reap_interval = forever;
    config.prewarm_interval = forever;
    config.prewarm_quiet_max = forever;
    // No idle exit: this server's lifetime is the test's, and a daemon that
    // reaped itself mid-assertion would fail as a flake, not a finding.
    config.idle_exit = None;
    // R1: a build sha, so the hello pin covers the shape a DEPLOYED daemon
    // emits. Left unset, `identity` would be absent and the pin would freeze
    // the identity-less variant — exhaustive over a body no production host
    // sends (`docs/wire-contract-v3-identity-amendment.md`).
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

/// Rehomes `u27_v2_key_set_pins.rs::frame_envelope_key_set_is_id_ok_and_exactly_one_payload`.
///
/// The v2 pin was `{id, ok, body|error}` and named the ABSENCE of `meta` as the
/// point. Under v3 `meta` is present by design (U7 in-band timing), so the
/// surviving statement is the one that still bites: exactly ONE payload member
/// rides — never `body` and `error` together.
///
/// The daemon has TWO refusal envelopes, and the difference is not cosmetic: a
/// frame refused BEFORE the dispatch shell (an unknown op — the daemon never
/// reached the engine) carries no `meta`, while a refusal the engine itself
/// produced is timed like any other engine work. Both are pinned, because a
/// future refactor that moved the unknown-op check behind dispatch would change
/// which frames a latency consumer can read, silently.
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

    // U7 placement: `meta` rides BESIDE the payload, never inside it. A timing
    // block that sank into `body` would be a payload field on every read.
    assert!(
        ok["body"].as_object().unwrap().get("meta").is_none(),
        "meta is not a body field: {ok}"
    );
    assert!(
        dispatched["error"].as_object().unwrap().get("meta").is_none(),
        "meta is not an error field: {dispatched}"
    );
    pin_keys(&ok["meta"], &["duration_us"], "meta block");

    fx.shutdown();
}

// ---------------------------------------------------------------------------
// hello (§3.2)
// ---------------------------------------------------------------------------

/// Rehomes `u27_v2_key_set_pins.rs::hello_body_key_set_is_the_frozen_four`.
///
/// The sidecar's hello body was the frozen four (`caps`/`proto`/`root`/`server`).
/// The daemon's is a DIFFERENT shape by design — it also pins the drawer
/// (`storage`) and echoes the negotiated `contract`, because a resident daemon
/// serving many workspaces must tell the client which one it bound. That is a
/// socket-plane fact the sidecar never had, so this pin is the socket's own, not
/// a copy: it exists to stop the handshake growing a seventh field unnoticed.
/// `workspace` is the resolved binding echoed back — the client asked with a path
/// and the daemon answers with the canonical one it actually bound.
///
/// **R1 (2026-08-05) — the expected set grew an eighth key, `identity`,
/// deliberately.** The daemon now echoes its build identity on a v3 handshake
/// so a client can tell a resident daemon from the binary it just deployed
/// (`docs/wire-contract-v3-identity-amendment.md`). The pin stays exhaustive;
/// only the expected list moved, and the fixture configures a sha precisely so
/// this assertion covers the shape a deployed daemon emits rather than the
/// identity-less one a bare fixture would produce.
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

/// Rehomes `u27_v2_key_set_pins.rs::toc_body_and_row_key_sets_are_frozen`.
///
/// The body's cursor key is re-spelled (`root` → `fingerprint`); the ROW shapes
/// are vocabulary-neutral and rehome verbatim, including the §2.1 `hpath`
/// segment, whose object form is pinned in both directions (decision 20).
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

/// Rehomes `u27_v2_key_set_pins.rs::toc_anchor_row_key_set_is_frozen`.
///
/// Vocabulary-neutral: the anchor row is minted by the receipt append and carries
/// no cursor key at all.
#[test]
fn the_toc_anchor_row_key_set_is_pinned() {
    let (fx, mut conn) = Fixture::start();
    let write = guarded_write(&mut conn);
    let splice = conn.call(&write);
    assert_eq!(splice["ok"], json!(true), "the guarded write lands: {splice}");

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

/// Rehomes `u27_v2_key_set_pins.rs::cat_body_key_set_is_frozen_both_forms`.
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

/// Rehomes `u27_v2_key_set_pins.rs::extract_body_and_node_key_sets_are_frozen`
/// AND `contract_v3.rs::a_frozen_v2_extract_node_never_grows_a_field`.
///
/// The v2 statement was an allow-list proving the node had NOT grown the v3 host
/// face. Its v3 statement is the mirror and the stronger one: on a v3 session the
/// enriched keys are present, EXACTLY, and a heading node carries `n`/`words`
/// while a wikilink node carries neither. `hpath_text` is absent from both — U14
/// (ZT decision 14) retired the joined string address from machine surfaces, and
/// an exhaustive pin is what keeps it retired.
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

/// Rehomes `u27_v2_key_set_pins.rs::resolve_body_key_sets_are_frozen_both_forms`.
/// Vocabulary-neutral: §4.5 answers a LOCATION, never a rev (D-C2), so the
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

/// Rehomes `u27_v2_key_set_pins.rs::links_body_and_file_key_sets_are_frozen`.
/// The staleness triple is re-spelled; the corpus edge map is never re-keyed, so
/// the file entry rehomes verbatim.
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

/// Rehomes `u27_v2_key_set_pins.rs::splice_body_receipt_and_edit_key_sets_are_frozen`
/// and `::armed_key_set_as_served_on_v2`.
///
/// Only the transition slots are re-spelled; `armed`, the receipt fact, and the
/// armed edit are vocabulary-neutral and rehome verbatim. `armed.file_rev_after`
/// is pinned AS SERVED (decision 21): the whole-file rev after the write, so a
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

/// Rehomes `u27_v2_key_set_pins.rs::splice_dry_body_key_set_is_frozen`.
///
/// The dry rehearsal is a DIFFERENT key set from the committed write, not the
/// same one with null values: no receipt, no `seq`, a `dry` flag, and a dry
/// `armed` that carries no `file_rev_after` (nothing was written, so the
/// file-grain post-rev does not exist). `fingerprint_after` is NULL, never
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
    assert_eq!(got["ok"], json!(true), "the dry rehearsal answers ok: {got}");
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

/// Rehomes `u27_v2_key_set_pins.rs::root_and_sub_ack_body_key_sets_are_frozen`
/// and `::diff_body_key_set_is_frozen`. Cursor keys re-spelled; `seq` and
/// `batches` are vocabulary-neutral.
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

    // `sub` turns its connection push-only, so it rides its own.
    let mut sub_conn = fx.conn();
    assert_eq!(sub_conn.hello(&fx.ws)["ok"], json!(true));
    let ack = sub_conn.call(&json!({"id": 91, "op": "sub", "from_seq": 0}));
    pin_keys(&ack["body"], &["fingerprint", "seq"], "sub ack body");

    fx.shutdown();
}

// ---------------------------------------------------------------------------
// The §8 error envelopes — per worked code
// ---------------------------------------------------------------------------

/// Rehomes `u27_v2_key_set_pins.rs::cas_mismatch_error_key_set_is_frozen`.
///
/// The v2 pin's point was that `rev::demote_v2` DROPS the v3 ladder extras, so
/// its key set was the short one (`{code, recovery, expected, actual}`). The v3
/// statement is the other side of that same fact, and it is the one worth
/// keeping: this is the shape BEFORE any demotion — the full A-K5 retry ladder.
/// Porting the v2 key set verbatim would have pinned the demoted shape on a wire
/// that never demotes, and it would have passed on nothing.
#[test]
fn the_cas_mismatch_error_key_set_is_pinned() {
    let (fx, mut conn) = Fixture::start();
    let write = guarded_write(&mut conn);
    assert_eq!(conn.call(&write)["ok"], json!(true), "the first write lands");

    // The SAME guard replayed: the section rev moved under it.
    let stale_retry = conn.call(&write);
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
    // The extras are the LADDER, and the ladder is the point: the refusal hands
    // back the current rev and the diff, so a client re-sends without re-reading.
    // `recovery` is `refresh` here, not `resync` — a section-grain conflict is
    // recoverable in place.
    assert_eq!(stale_retry["error"]["recovery"], json!("refresh"), "{stale_retry}");
    assert_eq!(
        stale_retry["error"]["new_fingerprint"], stale_retry["error"]["actual"],
        "the ladder hands back the rev the client must guard on next: {stale_retry}"
    );
    fx.shutdown();
}

/// Rehomes `u27_v2_key_set_pins.rs::no_match_and_not_unique_error_key_sets_are_frozen`.
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

    // `not_unique` needs a second occurrence: append "- new item" into Q4 so
    // "item" then occurs twice inside that section.
    let append = conn.call(&json!({
        "id": 57, "op": "splice", "path": "plan.md",
        "edits": [{
            "target": {"hpath": [{"h": "Goals"}, {"h": "Q4"}]},
            "edit": {"put": {"at": "end", "text": "- new item\n"}},
            "if_node_rev": q4,
        }],
    }));
    assert_eq!(append["ok"], json!(true), "the append lands: {append}");
    let q4_after = node_rev(
        &conn.call(&json!({"op": "toc", "path": "plan.md"})),
        "Q4",
    );
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

/// Rehomes `u27_v2_key_set_pins.rs::ref_not_found_error_key_sets_are_frozen_both_stages`.
/// Vocabulary-neutral: stage 2 got far enough to name a `dest`; stage 1 did not.
/// The two stages are separate shapes and both are pinned, because collapsing
/// them is exactly how the walk loses its diagnostic.
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

/// Rehomes `u27_v2_key_set_pins.rs::remaining_frozen_error_key_sets_are_pinned`.
/// Every one of these is vocabulary-neutral — they refuse the REQUEST, before any
/// cursor is in play.
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
    let overlap = conn.call(&json!({
        "id": 22, "op": "splice", "path": "plan.md",
        "edits": [
            {"target": {"hpath": [{"h": "Goals"}, {"h": "Q3"}]},
             "edit": {"match": {"old": "ship", "new": "a"}}, "if_node_rev": q3},
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
    pin_keys(
        &missing["error"],
        &["code", "path", "recovery"],
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
    fx.shutdown();
}

/// Rehomes `u27_v2_key_set_pins.rs::guard_required_error_key_set_is_pinned`.
///
/// Vocabulary-neutral, and the most-served refusal on this wire: it is what a
/// guardless write meets first. Its teaching `message`/`path` are pinned so the
/// refusal cannot grow a sibling field unseen.
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
// §3.1 id discipline — a divergence, recorded rather than papered over
// ---------------------------------------------------------------------------

/// §3.1 says a request whose `id` is not an integer lexeme is `bad_request`, and
/// the refusal echoes the offending lexeme back as `id_raw` so the client can
/// correlate a frame it can no longer address. `u27_v2_key_set_pins.rs::remaining_frozen_error_key_sets_are_pinned`
/// pins that refusal on the sidecar plane.
///
/// **The daemon does not serve it.** A string `id` is accepted: the request is
/// executed, and the response carries `id: null` with `ok: true` — so the caller
/// gets an answer it cannot correlate and no signal that anything was wrong.
/// This test pins WHAT IS SERVED. The contract shape gets the `#[ignore]`d twin
/// below, exactly as u27 did for `root_mismatch.changed`: a pin that encodes the
/// contract instead of the wire would fail on day one and teach a future reader
/// that the suite is broken rather than that the daemon is.
///
/// Deleting `crates/sidecar` removes the only plane where §3.1 id discipline is
/// enforced by a test. R3b should carry a disposition card for this.
#[test]
fn a_non_integer_id_is_accepted_as_served_on_the_daemon() {
    let (fx, mut conn) = Fixture::start();
    let raw_id = conn.call(&json!({"id": "7", "op": "fingerprint"}));
    assert_eq!(
        raw_id["ok"],
        json!(true),
        "as served: the daemon executes the request rather than refusing it: {raw_id}"
    );
    assert_eq!(
        raw_id["id"],
        Value::Null,
        "the un-parseable lexeme becomes a null id — the correlation is lost \
         silently, which is the half of §3.1 the daemon does implement: {raw_id}"
    );
    pin_keys(
        &raw_id,
        &["body", "id", "meta", "ok"],
        "non-integer-id response frame (as served)",
    );
    fx.shutdown();
}

/// Contract §3.1: the same request is a `bad_request` echoing `id_raw`.
#[test]
#[ignore = "R3a finding — §3.1 id_raw is never served by the daemon; needs a disposition card before R3b deletes the sidecar plane that does serve it"]
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

// ---------------------------------------------------------------------------
// pin_keys self-control
// ---------------------------------------------------------------------------

/// Rehomes `u27_v2_key_set_pins.rs::the_pin_primitive_rejects_a_superset`.
///
/// The primitive must travel with its own control. A future weakening of
/// `pin_keys` to a subset check would leave every pin in this file compiling,
/// passing, and asserting nothing — the exact failure mode these pins exist to
/// prevent, committed inside the suite that names the law.
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
// § Not rehomed
// ---------------------------------------------------------------------------
//
// Three u27 live-plane pins are absent from this file on purpose:
//
// 1. `root_mismatch_error_key_set_as_served_on_v2_today` and
//    `contract_root_mismatch_carries_changed` (the `#[ignore]`d disposition
//    twin). Both already have a socket-plane home:
//    `crates/registry/tests/root_mismatch_wire_shape.rs` pins the served shape,
//    proves the refusal is a real one carrying its ruled recovery, and carries
//    its own control that the pin can distinguish a `changed` field from its
//    absence. Duplicating them here would give the U27 finding two homes and no
//    owner.
//
// 2. `delta_notification_key_sets_are_frozen`. The Delta noun on the push plane
//    has a socket home in `crates/registry/tests/sub_push.rs`, which drives the
//    real subscription. That suite asserts Delta VALUES, not an exhaustive key
//    set — so the pin mechanism is genuinely thinner there than it was in u27.
//    It is named here rather than silently dropped: the honest statement is that
//    the push plane keeps its coverage and loses its exhaustive pin.
//
// 3. `verdict_key_set_is_frozen_on_the_wire`. This one has NO socket home and
//    cannot be given one from a test: the daemon serves with no rule packs
//    (`crates/registry/src/server.rs` — "No rule packs (`&[]` ⇒ `verdicts: []`)"),
//    so a §11.1 Verdict is UNREACHABLE on this wire. u27's own comment records
//    that a pin over a hand-built value tests its own construction, which is why
//    it insisted on a wire pin. Deleting `crates/sidecar` therefore removes the
//    only place a Verdict is pinned as it leaves a real server, and nothing in
//    this unit can replace it. That is a REAL coverage loss and it belongs on
//    R3b's checklist, not in a comment that reads as if it were handled.
