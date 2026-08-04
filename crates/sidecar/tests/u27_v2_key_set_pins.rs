//! **U27 — the frozen-v2 key-set pin suite (LIVE plane).**
//!
//! One exhaustive key-set pin per response shape in the frozen v2 op list
//! (`docs/wire-contract-v2.md` §3.2 caps + §4 op definitions), taken from the
//! REAL serve loop on a v2 session (no `contract` in `hello`, so `rev::Rev::V2`
//! by default).
//!
//! # Why this file exists
//!
//! A v3-additive response field riding a v2 envelope has been found three times
//! (All-Hands #1: `read`/`extract` fields; U20b: `NotificationRoot`; U20b again:
//! `body.armed.effects`) — every time by a worker reading a surface, never by a
//! gate. The standing instruments cannot see the class:
//!
//! - `Option` + `skip_serializing_if` is NOT a version gate. It skips on a none
//!   VALUE, never on a v2 SESSION, so any path that POPULATES a v3-additive
//!   field serializes it onto a v2 wire.
//! - `pf_frozen_sweep` pins worked VALUES (spans, revs, roots). Sweep-green is
//!   true for values and FALSE for fields: a v3-only key in a v2 envelope passes
//!   it untouched.
//!
//! # What makes these pins detectors and not decorations
//!
//! Every assertion is `assert_eq!` on the FULL sorted key list. A `contains`
//! or subset check would pass while a v3 field rode the envelope — a test that
//! cannot fail for the reason it exists (All-Hands #3: a control that cannot
//! distinguish the two worlds is a decoration). The two worlds these pins tell
//! apart are "the v2 envelope carries exactly its frozen keys" and "the v2
//! envelope grew (or lost) one" — an added key, a removed key, and a changed
//! POPULATION RULE on an existing `Option` all move the list.
//!
//! This is the LIVE half of the pair. [`crates/wire/tests/u27_frozen_key_sets.rs`]
//! pins the same shapes at the TYPE with every optional field populated: it
//! catches a field added to the struct even when no live path populates it yet,
//! which this file cannot see. Neither half subsumes the other.
//!
//! # Deviations found and NOT pinned as correct
//!
//! Where the live key set disagrees with the frozen contract, the pin records
//! what the wire does (so the shape cannot drift further unseen) and a paired
//! `#[ignore]`d test states what the CONTRACT says, so nothing here converts a
//! defect into a regression lock. See `armed_key_set_*` and
//! `root_mismatch_key_set_*` below.

use serde_json::Value;
use std::io::Write as _;

// ---------------------------------------------------------------------------
// Harness (dispatch_v2.rs conventions, verbatim)
// ---------------------------------------------------------------------------

fn workspace(files: &[(&str, &str)]) -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    for (rel, bytes) in files {
        let abs = dir.path().join(rel);
        std::fs::create_dir_all(abs.parent().expect("parent")).expect("mkdir");
        let mut f = std::fs::File::create(&abs).expect("create");
        f.write_all(bytes.as_bytes()).expect("write");
    }
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

fn wsfix(rel: &str) -> String {
    std::fs::read_to_string(testsuite::wsfix_dir().join(rel))
        .unwrap_or_else(|e| panic!("wsfix fixture {rel}: {e}"))
}

/// The §0.3 S0 workspace every worked contract value is computed against.
fn s0() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let plan = wsfix("s0/notes/plan.md");
    let receipts = wsfix("s0/receipts/2026-07-18.md");
    workspace(&[
        ("notes/plan.md", &plan),
        ("receipts/2026-07-18.md", &receipts),
    ])
}

/// Feed `input` through the live serve loop as a V2 SESSION; one parsed frame
/// per output line (notification frames included, in emission order).
fn serve(root: &fs::WorkspaceRoot, input: &str) -> Vec<Value> {
    let mut out = Vec::new();
    sidecar::serve(root, input.as_bytes(), &mut out, &[]).expect("serve");
    String::from_utf8(out)
        .expect("frames are UTF-8")
        .lines()
        .map(|l| serde_json::from_str(l).expect("frame parses"))
        .collect()
}

fn one(root: &fs::WorkspaceRoot, line: &str) -> Value {
    let mut frames = serve(root, &format!("{line}\n"));
    assert_eq!(frames.len(), 1, "exactly one response per request");
    frames.remove(0)
}

/// **The pin primitive.** EXHAUSTIVE `assert_eq!` on the full sorted key list of
/// one JSON object — never a subset or `contains` check, which is the only form
/// that can fail for the reason this suite exists.
#[track_caller]
fn pin_keys(value: &Value, expected: &[&str], what: &str) {
    let obj = value
        .as_object()
        .unwrap_or_else(|| panic!("{what} is not a JSON object: {value}"));
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys, expected,
        "frozen v2 {what} key set drifted — a key was added, removed, or its \
         population rule changed. Frame: {value}"
    );
}

/// The S0 node revs the guarded fixtures resend (frozen §4.1 worked toc).
const Q3_REV: &str = "33d5b0e1b27cb48b";
const Q4_REV: &str = "4b8bc385a58da0e0";
const R0: &str = "b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9";

/// The frozen §4.4 guarded E3 write, receipt included — the one exchange that
/// exercises the whole splice response shape (armed + receipt + root advance).
fn e3_splice() -> String {
    format!(
        r#"{{"id":42,"op":"splice","path":"notes/plan.md","actor":"agent:b0864fb2","now":"2026-07-18T20:31:04Z","receipt":{{"path":"receipts/2026-07-18.md","anchor":"r-000042"}},"edits":[{{"target":{{"hpath":[{{"h":"Goals"}},{{"h":"Q3"}}]}},"edit":{{"match":{{"old":"ship by August","new":"ship by September"}}}},"if_node_rev":"{Q3_REV}"}}]}}"#
    )
}

// ---------------------------------------------------------------------------
// Frame envelope (§3.1) — every response, ok and error
// ---------------------------------------------------------------------------

/// §3.1: a response frame is exactly `{id, ok, body}` or `{id, ok, error}`.
/// A v3 session additionally carries `meta` (U7 timing); a v2 session must
/// never grow it, and this pin is what says so for EVERY frame in the suite.
#[test]
fn frame_envelope_key_set_is_id_ok_and_exactly_one_payload() {
    let (_d, root) = s0();
    let ok = one(&root, r#"{"id":11,"op":"root"}"#);
    pin_keys(&ok, &["body", "id", "ok"], "ok response frame");
    let err = one(&root, r#"{"id":12,"op":"nope"}"#);
    pin_keys(&err, &["error", "id", "ok"], "error response frame");
}

// ---------------------------------------------------------------------------
// hello (§3.2)
// ---------------------------------------------------------------------------

/// Frozen §3.2 hello body: `proto`, `server`, `caps`, `root`. The type admits
/// two more OPTIONAL additive fields (`storage`, `workspace`, both daemon-only
/// bindings); the sidecar populates neither, and this pin is what fails if a
/// future path starts emitting one onto a v2 handshake.
#[test]
fn hello_body_key_set_is_the_frozen_four() {
    let (_d, root) = s0();
    let got = one(
        &root,
        r#"{"id":1,"op":"hello","proto":1,"client":"md-cli/0.3"}"#,
    );
    pin_keys(
        &got["body"],
        &["caps", "proto", "root", "server"],
        "hello body",
    );
}

// ---------------------------------------------------------------------------
// toc (§4.1) — body + every row class the contract works
// ---------------------------------------------------------------------------

/// Frozen §4.1: the toc body is `path` + `file_rev` + ambient `root` + `nodes`,
/// and each row class carries exactly the kit the contract prints —
/// frontmatter rows carry `keys`, heading rows carry `level`/`hpath`/
/// `content_span`, anchor rows carry `anchor` and their HOST kind.
///
/// The row pins are where a v3-additive addressing field (`n`, `hpath_text`,
/// `words` — all present on the node types) would surface on a v2 wire.
#[test]
fn toc_body_and_row_key_sets_are_frozen() {
    let (_d, root) = s0();
    let got = one(&root, r#"{"id":2,"op":"toc","path":"notes/plan.md"}"#);
    pin_keys(
        &got["body"],
        &["file_rev", "nodes", "path", "root"],
        "toc body",
    );
    let nodes = got["body"]["nodes"].as_array().expect("nodes array");
    pin_keys(
        &nodes[0],
        &["keys", "kind", "node_rev", "span", "text_prefix_16b"],
        "toc frontmatter row",
    );
    pin_keys(
        &nodes[1],
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
    // The §2.1 hpath segment — object form only, both directions (decision 20).
    let seg = &nodes[1]["hpath"].as_array().expect("hpath")[0];
    pin_keys(seg, &["h"], "toc heading row hpath segment");
}

/// The §4.1 worked ANCHOR row: a block id echoes as its HOST kind keyed by
/// `anchor`, carrying its own `node_rev` over the block-leaf span. Reached the
/// only way it exists — after a receipt append mints the block.
#[test]
fn toc_anchor_row_key_set_is_frozen() {
    let (_d, root) = s0();
    let frames = serve(
        &root,
        &format!(
            "{}\n{}\n",
            e3_splice(),
            r#"{"id":4,"op":"toc","path":"receipts/2026-07-18.md"}"#
        ),
    );
    let toc = frames.last().expect("toc frame");
    let nodes = toc["body"]["nodes"].as_array().expect("nodes array");
    let anchor_row = nodes
        .iter()
        .find(|n| n.get("anchor").is_some())
        .expect("the receipt append minted an anchor row");
    pin_keys(
        anchor_row,
        &["anchor", "kind", "node_rev", "span", "text_prefix_16b"],
        "toc anchor row",
    );
}

// ---------------------------------------------------------------------------
// cat (§4.2)
// ---------------------------------------------------------------------------

/// Frozen §4.2: `span` + `node_rev` + `content`, sectioned and whole-file
/// alike — one shape, no `path`, no `root`.
#[test]
fn cat_body_key_set_is_frozen_both_forms() {
    let (_d, root) = s0();
    let sec = one(
        &root,
        r#"{"id":3,"op":"cat","path":"notes/plan.md","sec":{"hpath":[{"h":"Goals"},{"h":"Q3"}]}}"#,
    );
    pin_keys(&sec["body"], &["content", "node_rev", "span"], "cat body");
    let whole = one(&root, r#"{"id":9,"op":"cat","path":"notes/plan.md"}"#);
    pin_keys(
        &whole["body"],
        &["content", "node_rev", "span"],
        "cat whole-file body",
    );
}

// ---------------------------------------------------------------------------
// extract (§4.3)
// ---------------------------------------------------------------------------

/// Frozen §4.3: the node inventory is `path` + `nodes`, and a node is
/// `kind`/`span`/`text_prefix_16b`/`node_rev` plus per-kind extras — NEVER the
/// v3-additive host-face trio (`n`, `hpath_text`, `words`) that lives on the
/// same struct.
#[test]
fn extract_body_and_node_key_sets_are_frozen() {
    let (_d, root) = s0();
    let got = one(&root, r#"{"id":5,"op":"extract","path":"notes/plan.md"}"#);
    pin_keys(&got["body"], &["nodes", "path"], "extract body");
    let nodes = got["body"]["nodes"].as_array().expect("nodes array");
    pin_keys(
        &nodes[0],
        &["info", "kind", "node_rev", "span", "text_prefix_16b"],
        "extract frontmatter node",
    );
    pin_keys(&nodes[0]["info"], &["keys"], "extract frontmatter info");
    pin_keys(
        &nodes[1],
        &["hpath", "kind", "node_rev", "span", "text_prefix_16b"],
        "extract heading node",
    );
    let wikilink = nodes
        .iter()
        .find(|n| n["kind"] == "wikilink")
        .expect("the fixture carries wikilinks");
    pin_keys(
        wikilink,
        &["info", "kind", "node_rev", "span", "text_prefix_16b"],
        "extract wikilink node",
    );
    pin_keys(&wikilink["info"], &["target"], "extract wikilink info");
}

// ---------------------------------------------------------------------------
// resolve (§4.5)
// ---------------------------------------------------------------------------

/// Frozen §4.5: location facts only — `dest` + `span`, and NO rev field exists
/// to return (D-C2, the mint partition as a type-level fact). `content` rides
/// only when the request asked for it.
#[test]
fn resolve_body_key_sets_are_frozen_both_forms() {
    let (_d, root) = s0();
    let plain = one(
        &root,
        r#"{"id":70,"op":"resolve","from":"notes/plan.md","ref":"plan#Goals#Q3"}"#,
    );
    pin_keys(&plain["body"], &["dest", "span"], "resolve body");
    let with_content = one(
        &root,
        r#"{"id":71,"op":"resolve","from":"notes/plan.md","ref":"plan#Goals#Q3","content":true}"#,
    );
    pin_keys(
        &with_content["body"],
        &["content", "dest", "span"],
        "resolve body (content:true)",
    );
}

// ---------------------------------------------------------------------------
// links (§4.6) + the §10.1 staleness triple
// ---------------------------------------------------------------------------

/// Frozen §4.6: the staleness triple (`as_of_root`, `live_root`,
/// `changes_seq`) plus `files`, whose values are the `resolved`/`unresolved`
/// edge maps — both always serialized, a link-less file being `{}`/`{}`.
#[test]
fn links_body_and_file_key_sets_are_frozen() {
    let (_d, root) = s0();
    let got = one(&root, r#"{"id":80,"op":"links","path":"notes/plan.md"}"#);
    pin_keys(
        &got["body"],
        &["as_of_root", "changes_seq", "files", "live_root"],
        "links body",
    );
    pin_keys(
        &got["body"]["files"]["notes/plan.md"],
        &["resolved", "unresolved"],
        "links file entry",
    );
    // Whole-corpus form: same body shape, every domain file keyed.
    let all = one(&root, r#"{"id":81,"op":"links"}"#);
    pin_keys(
        &all["body"],
        &["as_of_root", "changes_seq", "files", "live_root"],
        "links whole-corpus body",
    );
    pin_keys(
        &all["body"]["files"]["receipts/2026-07-18.md"],
        &["resolved", "unresolved"],
        "links link-less file entry",
    );
}

// ---------------------------------------------------------------------------
// splice (§4.4) — the ONE write response shape, real and dry
// ---------------------------------------------------------------------------

/// Frozen §4.4 splice body: `armed`, `receipt` (iff the request named one and
/// the batch hit disk), `root_before`, `root_after`, `seq`, `verdicts`.
///
/// `pin` (stage-2 S7) also lives on this type and is v3-only at decode — a v2
/// session cannot mint one, and this pin is what fails if that ever changes.
#[test]
fn splice_body_receipt_and_edit_key_sets_are_frozen() {
    let (_d, root) = s0();
    let got = one(&root, &e3_splice());
    assert_eq!(got["ok"], Value::Bool(true), "the guarded E3 write lands");
    pin_keys(
        &got["body"],
        &[
            "armed",
            "receipt",
            "root_after",
            "root_before",
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
    let edits = got["body"]["armed"]["edits"].as_array().expect("edits");
    pin_keys(
        &edits[0],
        &["node_rev_after", "node_rev_before", "span_after", "target"],
        "splice armed edit",
    );
    pin_keys(&edits[0]["target"], &["hpath"], "splice armed edit target");
}

/// **BOTH armed pins are WITHHELD — the oracle contradicts itself here.**
///
/// The two halves of the frozen-v2 oracle disagree on exactly one field:
///
/// - `docs/wire-contract-v2.md` §4.4 prints `armed` as `{path, edits}`.
/// - `crates/sidecar/tests/splice_e2e.rs` asserts frozen frames that INCLUDE
///   `armed.file_rev_after`, by exact-frame `assert_eq!`, on sessions that send
///   no `hello` — i.e. v2 sessions.
///
/// Which half is authoritative is above this unit's line: it is escalated to
/// the advisor (found by C3 / worker `0d110aa0`; the field arrived in commit
/// `9365455a`, ZT-authored, touching no `.md`). Minting EITHER pin now decides
/// the question by fiat — pinning the served shape would convert a possible
/// fourth leak into a permanent regression lock, and pinning the doc shape
/// would red-flag a field ZT may have amended in deliberately.
///
/// So both arms are landed `#[ignore]`d and neither is deleted: whichever way
/// the ruling goes, the pin that survives is already written and one attribute
/// away from live. Every OTHER shape in this file is minted normally — this is
/// the only contested one.
#[test]
#[ignore = "U27 — armed shape BLOCKED by Leader 160c2d32: doc §4.4 and splice_e2e frozen frames disagree on `file_rev_after`; advisor ruling pending"]
fn armed_key_set_as_served_on_v2_today() {
    let (_d, root) = s0();
    let got = one(&root, &e3_splice());
    pin_keys(
        &got["body"]["armed"],
        &["edits", "file_rev_after", "path"],
        "splice armed (as served)",
    );
}

/// **The DOC half's answer for `armed`** — frozen §4.4 prints `{path, edits}`,
/// no `file_rev_after`. The other arm of the withheld pair above; see that
/// doc comment for why neither is live.
#[test]
#[ignore = "U27 — armed shape BLOCKED by Leader 160c2d32: this arm asserts the doc half of a self-contradicting oracle; advisor ruling pending"]
fn contract_armed_key_set_is_path_and_edits_only() {
    let (_d, root) = s0();
    let got = one(&root, &e3_splice());
    pin_keys(
        &got["body"]["armed"],
        &["edits", "path"],
        "splice armed (frozen §4.4)",
    );
}

/// Frozen §4.4 dry law: same response shape, `root_after:null`, no receipt
/// written — and `dry:true` rides. `seq` is absent because no batch committed.
#[test]
fn splice_dry_body_key_set_is_frozen() {
    let (_d, root) = s0();
    let got = one(
        &root,
        &format!(
            r#"{{"id":60,"op":"splice","path":"notes/plan.md","dry":true,"edits":[{{"target":{{"hpath":[{{"h":"Goals"}},{{"h":"Q3"}}]}},"edit":{{"match":{{"old":"ship by August","new":"ship by September"}}}},"if_node_rev":"{Q3_REV}"}}]}}"#
        ),
    );
    assert_eq!(got["ok"], Value::Bool(true), "the dry rehearsal answers ok");
    pin_keys(
        &got["body"],
        &["armed", "dry", "root_after", "root_before", "verdicts"],
        "splice dry body",
    );
    assert_eq!(
        got["body"]["root_after"],
        Value::Null,
        "§4.4: root_after is NULL on a dry run, never absent"
    );
    // Nothing was written, so the file-grain post-rev does not exist either.
    pin_keys(
        &got["body"]["armed"],
        &["edits", "path"],
        "splice dry armed",
    );
}

// ---------------------------------------------------------------------------
// root · sub · diff (§4.7)
// ---------------------------------------------------------------------------

/// Frozen §4.7: `{root, seq}` — the world-grain cursor. `sub`'s ack reuses the
/// same body (the subscription's anchor tense), so both are pinned here.
#[test]
fn root_and_sub_ack_body_key_sets_are_frozen() {
    let (_d, root) = s0();
    let got = one(&root, r#"{"id":90,"op":"root"}"#);
    pin_keys(&got["body"], &["root", "seq"], "root body");
    let ack = one(&root, r#"{"id":91,"op":"sub","from_seq":0}"#);
    pin_keys(&ack["body"], &["root", "seq"], "sub ack body");
}

/// Frozen §4.7/§7.3: the replay body is exactly `batches`, each batch being a
/// notification frame body — there is no second diff dialect.
#[test]
fn diff_body_key_set_is_frozen() {
    let (_d, root) = s0();
    let got = one(
        &root,
        &format!(r#"{{"id":95,"op":"diff","from_root":"{R0}","to_root":"{R0}"}}"#),
    );
    pin_keys(&got["body"], &["batches"], "diff body");
}

// ---------------------------------------------------------------------------
// The Delta noun on the notification plane (§7.1)
// ---------------------------------------------------------------------------

/// Frozen §7.1: the notification frame carries `delta` (plus the
/// amendment-declared `effects` sibling, omitted when empty), and the Delta is
/// `seq`/`root_before`/`root_after`/`actor`/`now`/`files` with node-grain
/// entries. This is the surface U20b's `NotificationRoot` sighting rode.
#[test]
fn delta_notification_key_sets_are_frozen() {
    let (_d, root) = s0();
    let frames = serve(
        &root,
        &format!(
            "{}\n{}\n",
            r#"{"id":1,"op":"sub","from_seq":0}"#,
            e3_splice()
        ),
    );
    let note = frames
        .iter()
        .find(|f| f.get("delta").is_some())
        .expect("the landed write pushed a delta notification");
    pin_keys(note, &["delta"], "delta notification frame");
    pin_keys(
        &note["delta"],
        &["actor", "files", "now", "root_after", "root_before", "seq"],
        "delta",
    );
    let files = note["delta"]["files"].as_array().expect("files array");
    let edited = files
        .iter()
        .find(|f| f["path"] == "notes/plan.md")
        .expect("the content file changed");
    pin_keys(
        edited,
        &[
            "change",
            "file_rev_after",
            "file_rev_before",
            "nodes",
            "path",
        ],
        "delta file (modified)",
    );
    pin_keys(
        &edited["nodes"].as_array().expect("nodes")[0],
        &[
            "change",
            "hpath",
            "node_rev_after",
            "node_rev_before",
            "span_after",
        ],
        "delta node (edited)",
    );
    let receipt_file = files
        .iter()
        .find(|f| f["path"] == "receipts/2026-07-18.md")
        .expect("the receipt file changed");
    pin_keys(
        &receipt_file["nodes"].as_array().expect("nodes")[0],
        &["anchor", "change", "node_rev_after", "span_after"],
        "delta node (added anchor)",
    );
}

// ---------------------------------------------------------------------------
// The §8 error envelope — per worked code
// ---------------------------------------------------------------------------

/// §5.2 the failure split: `cas_mismatch` carries `{code, recovery, expected,
/// actual}` on a v2 session and NOTHING else.
///
/// This is the standing detector for All-Hands #1's own sighting: U11's
/// mismatch-recovery ladder authors four v3-additive extras (`rung`, `diff`,
/// `new_content`, `new_fingerprint`) plus a teaching `message`/`path` on this
/// very envelope, and `rev::demote_v2` is the only thing keeping them off a v2
/// wire. If that demotion is bypassed, this pin reddens.
#[test]
fn cas_mismatch_error_key_set_is_frozen() {
    let (_d, root) = s0();
    let frames = serve(
        &root,
        &format!(
            "{}\n{}\n",
            e3_splice(),
            format_args!(
                r#"{{"id":88,"op":"splice","path":"notes/plan.md","edits":[{{"target":{{"hpath":[{{"h":"Goals"}},{{"h":"Q3"}}]}},"edit":{{"match":{{"old":"ship by September","new":"ship by October"}}}},"if_node_rev":"{Q3_REV}"}}]}}"#
            )
        ),
    );
    let err = frames.last().expect("the stale-rev retry refuses");
    assert_eq!(err["error"]["code"], "cas_mismatch");
    pin_keys(
        &err["error"],
        &["actual", "code", "expected", "recovery"],
        "cas_mismatch error",
    );
}

/// §5.2: rev PASSED and the old string did not — provably the caller's typo.
/// `matches` is the occurrence count and the only extra.
#[test]
fn no_match_and_not_unique_error_key_sets_are_frozen() {
    let (_d, root) = s0();
    let no_match = one(
        &root,
        &format!(
            r#"{{"id":89,"op":"splice","path":"notes/plan.md","edits":[{{"target":{{"hpath":[{{"h":"Goals"}},{{"h":"Q3"}}]}},"edit":{{"match":{{"old":"ship by October","new":"x"}}}},"if_node_rev":"{Q3_REV}"}}]}}"#
        ),
    );
    assert_eq!(no_match["error"]["code"], "no_match");
    pin_keys(
        &no_match["error"],
        &["code", "matches", "recovery"],
        "no_match error",
    );

    // not_unique needs the §0.3 S2 state: E4 appends "- new item", so "item"
    // then occurs twice inside Q4 — the contract's own worked count of 2.
    let (_d2, root2) = s0();
    let frames = serve(
        &root2,
        &format!(
            "{}\n{}\n",
            format_args!(
                r#"{{"id":57,"op":"splice","path":"notes/plan.md","edits":[{{"target":{{"hpath":[{{"h":"Goals"}},{{"h":"Q4"}}]}},"edit":{{"put":{{"at":"end","text":"- new item\n"}}}},"if_node_rev":"{Q4_REV}"}}]}}"#
            ),
            r#"{"id":91,"op":"toc","path":"notes/plan.md"}"#
        ),
    );
    let q4_after = frames.last().expect("toc")["body"]["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .find(|n| n["hpath"] == serde_json::json!([{"h":"Goals"},{"h":"Q4"}]))
        .expect("Q4 row")["node_rev"]
        .as_str()
        .expect("rev")
        .to_string();
    let not_unique = one(
        &root2,
        &format!(
            r#"{{"id":92,"op":"splice","path":"notes/plan.md","edits":[{{"target":{{"hpath":[{{"h":"Goals"}},{{"h":"Q4"}}]}},"edit":{{"match":{{"old":"item","new":"entry"}}}},"if_node_rev":"{q4_after}"}}]}}"#
        ),
    );
    assert_eq!(not_unique["error"]["code"], "not_unique");
    assert_eq!(not_unique["error"]["matches"], 2, "§5.2 worked count");
    pin_keys(
        &not_unique["error"],
        &["code", "matches", "recovery"],
        "not_unique error",
    );
}

/// **`root_mismatch` as the v2 wire ACTUALLY serves it.** §5.1 and §18 ledger
/// row 2 both spell `{expected, actual, changed}`; the live envelope omits
/// `changed` (U27 finding 2 — a contract field ABSENT, the mirror image of a
/// leak, and equally invisible to a value sweep).
#[test]
fn root_mismatch_error_key_set_as_served_on_v2_today() {
    let (_d, root) = s0();
    let frames = serve(
        &root,
        &format!(
            "{}\n{}\n",
            e3_splice(),
            format_args!(
                r#"{{"id":93,"op":"splice","path":"notes/plan.md","if_root":"{R0}","edits":[{{"target":{{"hpath":[{{"h":"Goals"}},{{"h":"Q4"}}]}},"edit":{{"match":{{"old":"item one","new":"entry"}}}},"if_node_rev":"{Q4_REV}"}}]}}"#
            )
        ),
    );
    let err = frames.last().expect("the world moved under if_root");
    assert_eq!(err["error"]["code"], "root_mismatch");
    assert_eq!(err["error"]["recovery"], "resync", "§18 row 4 rebind");
    pin_keys(
        &err["error"],
        &["actual", "code", "expected", "recovery"],
        "root_mismatch error (as served)",
    );
}

/// **The CONTRACT's answer for `root_mismatch`, red today.** §5.1 prints
/// `root_mismatch{expected,actual,changed}` and §18 row 2 re-states the
/// three-field shape while waiving only the `scope` drop. Ignored, not deleted.
#[test]
#[ignore = "U27 finding 2 — frozen §5.1 `changed` never served; needs a disposition card"]
fn contract_root_mismatch_carries_changed() {
    let (_d, root) = s0();
    let frames = serve(
        &root,
        &format!(
            "{}\n{}\n",
            e3_splice(),
            format_args!(
                r#"{{"id":93,"op":"splice","path":"notes/plan.md","if_root":"{R0}","edits":[{{"target":{{"hpath":[{{"h":"Goals"}},{{"h":"Q4"}}]}},"edit":{{"match":{{"old":"item one","new":"entry"}}}},"if_node_rev":"{Q4_REV}"}}]}}"#
            )
        ),
    );
    let err = frames.last().expect("root_mismatch frame");
    pin_keys(
        &err["error"],
        &["actual", "changed", "code", "expected", "recovery"],
        "root_mismatch error (frozen §5.1)",
    );
}

/// §4.5: `dest` rides every stage-2 outcome, success or failure; a stage-1 miss
/// has no dest to name.
#[test]
fn ref_not_found_error_key_sets_are_frozen_both_stages() {
    let (_d, root) = s0();
    let stage2 = one(
        &root,
        r#"{"id":73,"op":"resolve","from":"notes/plan.md","ref":"plan#Goals#Q9"}"#,
    );
    pin_keys(
        &stage2["error"],
        &["code", "dest", "recovery", "stage"],
        "ref_not_found stage-2 error",
    );
    let stage1 = one(
        &root,
        r#"{"id":74,"op":"resolve","from":"notes/plan.md","ref":"roadmap"}"#,
    );
    pin_keys(
        &stage1["error"],
        &["code", "recovery", "stage"],
        "ref_not_found stage-1 error",
    );
}

/// The remaining §8 envelopes the frozen contract works: the D-C5 loud
/// `unknown_kinds`, the §3.1 raw-lexeme `id_raw` (beside `id:null`), the §4.4
/// disjointness refusal, `unsupported_proto`, and the two path classes.
#[test]
fn remaining_frozen_error_key_sets_are_pinned() {
    let (_d, root) = s0();

    let kinds = one(
        &root,
        r#"{"id":21,"op":"extract","path":"notes/plan.md","kinds":["bogus"]}"#,
    );
    pin_keys(
        &kinds["error"],
        &["code", "recovery", "unknown_kinds"],
        "bad_request{unknown_kinds} error",
    );

    let raw_id = one(&root, r#"{"id":"7","op":"root"}"#);
    assert_eq!(
        raw_id["id"],
        Value::Null,
        "§3.1: a bad lexeme echoes id:null"
    );
    pin_keys(
        &raw_id["error"],
        &["code", "id_raw", "recovery"],
        "bad_request{id_raw} error",
    );

    let overlap = one(
        &root,
        &format!(
            r#"{{"id":22,"op":"splice","path":"notes/plan.md","edits":[{{"target":{{"hpath":[{{"h":"Goals"}},{{"h":"Q3"}}]}},"edit":{{"match":{{"old":"ship","new":"a"}}}},"if_node_rev":"{Q3_REV}"}},{{"target":{{"hpath":[{{"h":"Goals"}},{{"h":"Q3"}}]}},"edit":{{"match":{{"old":"August","new":"b"}}}},"if_node_rev":"{Q3_REV}"}}]}}"#
        ),
    );
    pin_keys(
        &overlap["error"],
        &["code", "message", "overlap", "recovery"],
        "bad_request{overlap} error",
    );

    let proto = one(&root, r#"{"id":23,"op":"hello","proto":99}"#);
    pin_keys(
        &proto["error"],
        &["code", "recovery", "supported"],
        "unsupported_proto error",
    );

    let missing = one(&root, r#"{"id":24,"op":"toc","path":"missing.md"}"#);
    pin_keys(
        &missing["error"],
        &["code", "path", "recovery"],
        "file_not_found error",
    );

    let bad_path = one(&root, r#"{"id":25,"op":"toc","path":"../escape.md"}"#);
    pin_keys(
        &bad_path["error"],
        &["code", "path", "recovery"],
        "bad_path error",
    );

    let unknown_op = one(&root, r#"{"id":26,"op":"nope"}"#);
    pin_keys(
        &unknown_op["error"],
        &["code", "recovery"],
        "unknown_op error",
    );
}

/// §10.2: the `require_root` refusal names all three roots of the staleness
/// triple — what was demanded, what the answer would have been computed at,
/// and what is live.
#[test]
fn stale_view_error_key_set_is_frozen() {
    let (_d, root) = s0();
    let frames = serve(
        &root,
        &format!(
            "{}\n{}\n",
            e3_splice(),
            format_args!(
                r#"{{"id":82,"op":"links","path":"notes/plan.md","require_root":"{R0}"}}"#
            )
        ),
    );
    let err = frames.last().expect("the demanded root is stale");
    assert_eq!(err["error"]["code"], "stale_view");
    pin_keys(
        &err["error"],
        &["as_of_root", "code", "live_root", "recovery", "required"],
        "stale_view error",
    );
}

/// The refusal-amendment's `guard_required` (a v2-era refusal code, not a
/// frozen-§8 row): pinned so its teaching `message`/`path` cannot grow a
/// v3-additive sibling unseen — it is the refusal a guardless v2 write meets
/// first, so it is the most-served error on this wire.
#[test]
fn guard_required_error_key_set_is_pinned() {
    let (_d, root) = s0();
    let got = one(
        &root,
        r#"{"id":27,"op":"splice","path":"notes/plan.md","edits":[{"target":{"hpath":[{"h":"Goals"},{"h":"Q3"}]},"edit":{"match":{"old":"ship by August","new":"x"}}}]}"#,
    );
    assert_eq!(got["error"]["code"], "guard_required");
    pin_keys(
        &got["error"],
        &["code", "message", "path", "recovery"],
        "guard_required error",
    );
}
