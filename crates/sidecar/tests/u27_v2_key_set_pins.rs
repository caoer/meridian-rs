//! U27 — frozen-v2 key-set pins (LIVE plane).
//!
//! One exhaustive sorted-key `assert_eq!` per response shape in the frozen v2
//! op list, from the real serve loop on a v2 session (no `contract` in
//! `hello`). Full key lists only — subset/`contains` cannot catch a v3 field
//! on a v2 envelope. `Option`+`skip_serializing_if` is not a version gate;
//! value sweeps miss field growth.
//!
//! Live half of the pair with `crates/wire/tests/u27_frozen_key_sets.rs` (type
//! plane). Known dispositions: `armed.file_rev_after` pinned as served on v2
//! (ratified); `root_mismatch.changed` pinned as served + contract shape
//! `#[ignore]`d (never served). Pins record the wire; defects are not locked
//! green.

use policy::{PackFiles, RulesetPin};
use serde_json::Value;
use std::collections::HashMap;
use std::io::Write as _;

// ---------------------------------------------------------------------------
// Harness
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

/// Exhaustive sorted key-list pin — not subset/`contains`.
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

/// Frozen §4.4 guarded E3 write (armed + receipt + root advance).
fn e3_splice() -> String {
    format!(
        r#"{{"id":42,"op":"splice","path":"notes/plan.md","actor":"agent:b0864fb2","now":"2026-07-18T20:31:04Z","receipt":{{"path":"receipts/2026-07-18.md","anchor":"r-000042"}},"edits":[{{"target":{{"hpath":[{{"h":"Goals"}},{{"h":"Q3"}}]}},"edit":{{"match":{{"old":"ship by August","new":"ship by September"}}}},"if_node_rev":"{Q3_REV}"}}]}}"#
    )
}

// ---------------------------------------------------------------------------
// Frame envelope (§3.1) — every response, ok and error
// ---------------------------------------------------------------------------

/// §3.1: ok/error frames are exactly `{id, ok, body|error}` — no v3 `meta`.
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

/// Frozen §3.2 hello: `proto`/`server`/`caps`/`root` (no storage/workspace).
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

/// Frozen §4.1 toc body + row classes (no v3 addressing fields on v2).
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

/// §4.1 anchor row after receipt append mints the block.
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

/// Frozen §4.2 cat: `span`/`node_rev`/`content` (sectioned and whole-file).
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

/// Frozen §4.3 extract: no v3 host-face keys (`n`/`hpath_text`/`words`).
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

/// Frozen §4.5: location only (`dest`/`span`); no rev (D-C2).
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

/// Frozen §4.6: staleness triple + `files` (`resolved`/`unresolved` always).
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

/// Frozen §4.4 splice body (no v3-only `pin` on a v2 session).
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

/// `armed.file_rev_after` on committed v2 splice (ratified on v2). Absent on
/// dry — see [`splice_dry_body_key_set_is_frozen`]. Whole-file rev after write
/// so client skips a follow-up toc; correctness stays fingerprint/`root_after`.
#[test]
fn armed_key_set_as_served_on_v2() {
    let (_d, root) = s0();
    let got = one(&root, &e3_splice());
    pin_keys(
        &got["body"]["armed"],
        &["edits", "file_rev_after", "path"],
        "splice armed (§4.4 as amended, decision 21)",
    );
}

/// Frozen §4.4 dry: `root_after:null`, no receipt/`seq`; dry `armed` = path+edits.
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

/// Frozen §4.7: `{root, seq}` for `root` and `sub` ack.
#[test]
fn root_and_sub_ack_body_key_sets_are_frozen() {
    let (_d, root) = s0();
    let got = one(&root, r#"{"id":90,"op":"root"}"#);
    pin_keys(&got["body"], &["root", "seq"], "root body");
    let ack = one(&root, r#"{"id":91,"op":"sub","from_seq":0}"#);
    pin_keys(&ack["body"], &["root", "seq"], "sub ack body");
}

/// Frozen §4.7/§7.3: diff body is exactly `batches`.
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

/// Frozen §7.1 notification: `delta` (+ empty-omitted `effects`); Delta keys.
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

/// §5.2 `cas_mismatch`: `{code, recovery, expected, actual}` only on v2
/// (`rev::demote_v2` drops v3 ladder extras).
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

/// §5.2 `no_match` / `not_unique`: `matches` is the only extra.
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

/// `root_mismatch` as served: no `changed` (contract §5.1 spells three fields).
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

/// Contract §5.1 shape including `changed` — ignored until disposition.
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

/// §4.5 `ref_not_found`: stage-2 carries `dest`; stage-1 does not.
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

/// Remaining §8 envelopes: `unknown_kinds`, `id_raw`, overlap, proto, path classes.
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

// ---------------------------------------------------------------------------
// §11.1 Verdict — the rules-as-data surface, FROM THE WIRE
// ---------------------------------------------------------------------------
//
// The shape is pinned from the wire, not from a hand-built value: a pin over
// a value the test itself constructed tests its own construction. The pack
// below is the minimum that admits; `verdicts_e2e.rs` owns the §11.1 worked
// values, this one owns only the key set.

/// In-memory pack-file resolver (the `verdicts_e2e.rs` stand-in for the disk
/// reads the sidecar injects in production).
struct MemFiles(HashMap<String, String>);

impl PackFiles for MemFiles {
    fn read(&self, rel_path: &str) -> std::io::Result<String> {
        self.0
            .get(rel_path)
            .cloned()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, rel_path.to_string()))
    }
}

/// A node-class rule that fires on any heading opening directly onto a deeper
/// one — enough to put ONE verdict on the wire, which is all a key-set pin
/// needs.
const BLURB_RULE: &str = r#"# blurb-required

Every section must open with a blurb line.

```starlark
def check(doc):
    nodes = doc.nodes
    count = len(nodes)
    for i in range(count):
        node = nodes[i]
        if node.kind != "heading":
            continue
        if i + 1 < count:
            nxt = nodes[i + 1]
            if nxt.kind == "heading" and nxt.level > node.level:
                violation(
                    rule = "blurb-required",
                    severity = "warn",
                    span = node.span,
                    node_rev = node.node_rev,
                    hpath = node.hpath,
                    message = "section has no blurb line",
                )
```
"#;

const BLURB_MANIFEST: &str = "\
id: wiki-hygiene
api: rulepack-api@1
budgets: { steps: 10000, mem: 4194304 }
fixtures: [fixtures/blurb-pass.md, fixtures/blurb-fail.md]
rules: [rules/blurb-required.md]
";

const FX_PASS: &str = "---\nexpect: pass\n---\n# Goals\n\n[[intro]]\n\n## Sub\n\n[[more]]\n";
const FX_FAIL: &str = "---\nexpect: fail\n---\n# Goals\n## Sub\nmore\n";

fn blurb_pack() -> policy::CompiledRuleset {
    let files = MemFiles(
        [
            ("rules/blurb-required.md", BLURB_RULE),
            ("fixtures/blurb-pass.md", FX_PASS),
            ("fixtures/blurb-fail.md", FX_FAIL),
        ]
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect(),
    );
    let pin = RulesetPin {
        id: "wiki-hygiene".into(),
        path: "policy/wiki-hygiene.yaml".into(),
        sha256: None,
    };
    sidecar::admit(&pin, BLURB_MANIFEST, &files).expect("the pack admits (node-class)")
}

/// Frozen §11.1: a verdict is `crates/policy`'s `Violation` verbatim, projected
/// into THE §2.1 grammar — `{rule, severity, path, hpath, span, node_rev,
/// message}` and nothing else.
///
/// Taken FROM THE WIRE: a real pack, a real splice, the engine's own finding.
/// `hpath` is the one optional member, and it rides here, so this is the
/// maximal form — a verdict on a frontmatter or file-grain target would carry
/// one fewer key, never one more.
#[test]
fn verdict_key_set_is_frozen_on_the_wire() {
    let (_d, root) = s0();
    let q3 = toc_rev(&root, "Q3");
    let line = format!(
        r#"{{"id":1,"op":"splice","path":"notes/plan.md","edits":[{{"target":{{"hpath":[{{"h":"Goals"}},{{"h":"Q3"}}]}},"edit":{{"match":{{"old":"ship by August","new":"ship by September"}}}},"if_node_rev":"{q3}"}}]}}"#
    );
    let mut out = Vec::new();
    sidecar::serve(
        &root,
        format!("{line}\n").as_bytes(),
        &mut out,
        &[blurb_pack()],
    )
    .expect("serve");
    let frame: Value = serde_json::from_str(
        String::from_utf8(out)
            .expect("utf8")
            .lines()
            .next()
            .expect("frame"),
    )
    .expect("frame parses");
    let verdicts = frame["body"]["verdicts"]
        .as_array()
        .unwrap_or_else(|| panic!("the splice response carries verdicts: {frame}"));
    assert!(
        !verdicts.is_empty(),
        "POSITIVE CONTROL: the pack must actually fire, or this pin asserts \
         nothing about a shape it never saw: {frame}"
    );
    pin_keys(
        &verdicts[0],
        &[
            "hpath", "message", "node_rev", "path", "rule", "severity", "span",
        ],
        "verdict (from the wire)",
    );
}

/// The §2.1 node rev for one Goals subsection, read off a live `toc`.
fn toc_rev(root: &fs::WorkspaceRoot, sec: &str) -> String {
    let toc = one(root, r#"{"id":1,"op":"toc","path":"notes/plan.md"}"#);
    toc["body"]["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .find(|n| n["hpath"] == serde_json::json!([{"h":"Goals"},{"h":sec}]))
        .unwrap_or_else(|| panic!("no {sec} row"))["node_rev"]
        .as_str()
        .expect("rev")
        .to_string()
}

// ---------------------------------------------------------------------------
// pin_keys self-control (twin of wire/tests/u27)
// ---------------------------------------------------------------------------

/// `pin_keys` must reject a superset — a subset/`contains` weaken would disarm
/// every pin in this file. (Live-plane copy; type-plane twin is separate.)
#[test]
fn the_pin_primitive_rejects_a_superset() {
    let one_extra = serde_json::json!({"a": 1, "b": 2});
    let caught = std::panic::catch_unwind(|| {
        pin_keys(&one_extra, &["a"], "pin-primitive self-control");
    });
    assert!(
        caught.is_err(),
        "pin_keys accepted a key set with an EXTRA key — it is no longer \
         exhaustive, and every pin in this file is now a decoration"
    );
    // And it accepts the exact set, so the control can tell the two worlds
    // apart rather than only proving that something panics.
    pin_keys(&one_extra, &["a", "b"], "pin-primitive self-control");
}
