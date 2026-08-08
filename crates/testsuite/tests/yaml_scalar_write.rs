//! The frontmatter scalar write law (wire-contract § A.6): the write seam
//! encodes the caller's value into the fleet-canonical spelling, so a
//! `set_property` of an `[[id]]`-shaped value succeeds and lands
//! `reviewer: "[[id]]"` — and the I4 nested-frontmatter gate accepts it.
//!
//! The season-1 dogfood receipts measured the defect: the unquoted emit
//! turned `[[id]]` into a YAML flow sequence, the conformance gate refused it
//! as nested frontmatter, and the refusal misattributed the cause to the
//! caller's value (fail-CLOSED). This suite drives the real wire door
//! (`registry::serve_lines` — the socket's own line loop) so the refusal and
//! the committed bytes are the engine's, not a unit-level reconstruction.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// Drive one v3 serve session over `doc_dir`; one frame per line.
fn serve(doc_dir: &Path, requests: &[Value]) -> Vec<Value> {
    let mut input = crate::daemon_door::hello_line(Some("v3"), doc_dir);
    for r in requests {
        input.push_str(&serde_json::to_string(r).expect("request serializes"));
        input.push('\n');
    }
    crate::daemon_door::serve_frames(&input)
}

/// The `file_rev` a `set_property` row must carry (the wire door demands the
/// doc-root token at the grain the row writes).
fn file_rev(frames: &[Value], read_id: u64) -> String {
    frames
        .iter()
        .find(|f| f["id"] == json!(read_id))
        .expect("the read frame")
        .get("body")
        .and_then(|b| b.get("file_rev"))
        .and_then(Value::as_str)
        .expect("file_rev")
        .to_string()
}

/// A workspace with one card carrying the fleet-canonical quoted owner.
fn card_ws(home: &tempfile::TempDir) -> PathBuf {
    let ws = home.path().join("ws");
    fs::create_dir_all(ws.join("tasks")).expect("mkdir");
    fs::write(
        ws.join("tasks/0011-token-audit.md"),
        "---\nowner: \"\"\nstatus: todo\n---\n\n# Goals\n\nbody\n",
    )
    .expect("fixture");
    ws
}

/// One read + one guarded `set_property` splice; returns the frames
/// (`[hello, read, hello, splice]` — the hello answer leads every serve).
fn read_then_set(ws: &Path, key: &str, value: &str) -> Vec<Value> {
    let frames = serve(
        ws,
        &[json!({"id":1,"op":"read","path":"tasks/0011-token-audit.md"})],
    );
    let rev = file_rev(&frames, 1);
    let mut all = frames;
    all.extend(serve(
        ws,
        &[json!({"id":2,"op":"splice","path":"tasks/0011-token-audit.md",
                 "actor":"agent:8ab41c02",
                 "plan_edits":[{"set_property":{
                     "key": key, "value": value, "rev": rev}}]}),
        ],
    ));
    all
}

/// The splice frame of a [`read_then_set`] run.
fn splice_frame(frames: &[Value]) -> &Value {
    &frames[3]
}

/// **Negative proof, write half (gate 3):** a `set_property` of the
/// `[[id]]`-shaped value succeeds and emits the fleet-canonical form. On the
/// unfixed base the unquoted emit turns `[[id]]` into a flow sequence and the
/// I4 nested-frontmatter gate refuses the write.
#[test]
fn a_wikilink_shaped_value_commits_in_the_fleet_canonical_form() {
    let home = tempfile::tempdir().expect("workspace");
    let ws = card_ws(&home);
    let frames = read_then_set(&ws, "reviewer", "[[id]]");
    let splice = splice_frame(&frames);
    assert_eq!(
        splice["ok"], json!(true),
        "the [[id]]-shaped value must commit, not be refused as nested \
         frontmatter: {splice}"
    );
    let on_disk = fs::read_to_string(ws.join("tasks/0011-token-audit.md")).expect("read back");
    assert!(
        on_disk.contains("reviewer: \"[[id]]\"\n"),
        "the write must land the fleet-canonical double-quoted form: {on_disk:?}"
    );
}

/// The read→write loop closes: a quoted fleet-canonical value reads back
/// decoded, and the decoded value writes back into the fleet-canonical form.
#[test]
fn the_read_write_loop_round_trips_the_fleet_canonical_form() {
    let home = tempfile::tempdir().expect("workspace");
    let ws = home.path().join("ws");
    fs::create_dir_all(ws.join("tasks")).expect("mkdir");
    fs::write(
        ws.join("tasks/0011-token-audit.md"),
        "---\nowner: \"[[1ed98864]]\"\nstatus: doing\n---\n\n# Goals\n\nbody\n",
    )
    .expect("fixture");

    let frames = read_then_set(&ws, "reviewer", "[[id]]");
    // The first read serves the quoted wikilink DECODED (§ A.6).
    let props = frames[1]["body"]["props"].as_array().expect("props plane");
    let owner = props
        .iter()
        .find(|p| p["key"] == json!("owner"))
        .expect("owner row");
    assert_eq!(
        owner["value"], json!("[[1ed98864]]"),
        "a quoted fleet-canonical value reads back without quote bytes: {owner}"
    );
    // The splice commits the [[id]]-shaped value in the fleet-canonical form.
    assert_eq!(splice_frame(&frames)["ok"], json!(true), "splice: {}", splice_frame(&frames));
    let on_disk = fs::read_to_string(ws.join("tasks/0011-token-audit.md")).expect("read back");
    assert!(
        on_disk.contains("reviewer: \"[[id]]\"\n"),
        "the write lands the fleet-canonical form: {on_disk:?}"
    );
    // A second read serves the written value decoded — the loop is closed.
    let frames = serve(
        &ws,
        &[json!({"id":3,"op":"read","path":"tasks/0011-token-audit.md"})],
    );
    let props = frames[1]["body"]["props"].as_array().expect("props plane");
    let reviewer = props
        .iter()
        .find(|p| p["key"] == json!("reviewer"))
        .expect("reviewer row");
    assert_eq!(
        reviewer["value"], json!("[[id]]"),
        "the written value reads back decoded: {reviewer}"
    );
}

/// Season-1 G5 end state: a `put` of the empty owner lands the fleet-canonical
/// `owner: ""` — present-but-empty, never a bare `owner:` that reads as absent
/// to the def checker.
#[test]
fn an_empty_value_lands_the_fleet_canonical_quoted_empty() {
    let home = tempfile::tempdir().expect("workspace");
    let ws = card_ws(&home);
    let frames = read_then_set(&ws, "owner", "");
    assert_eq!(splice_frame(&frames)["ok"], json!(true), "splice: {}", splice_frame(&frames));
    let on_disk = fs::read_to_string(ws.join("tasks/0011-token-audit.md")).expect("read back");
    assert!(
        on_disk.contains("owner: \"\"\n"),
        "the empty value must land as the fleet-canonical quoted empty: {on_disk:?}"
    );
}

/// A one-level flow list stays unquoted — the fleet's own `tags: [a, b]`
/// spelling is preserved, and the I4 gate accepts it (one-level lists are not
/// the nested class).
#[test]
fn a_one_level_flow_list_stays_unquoted() {
    let home = tempfile::tempdir().expect("workspace");
    let ws = card_ws(&home);
    let frames = read_then_set(&ws, "tags", "[a, b]");
    assert_eq!(splice_frame(&frames)["ok"], json!(true), "splice: {}", splice_frame(&frames));
    let on_disk = fs::read_to_string(ws.join("tasks/0011-token-audit.md")).expect("read back");
    assert!(
        on_disk.contains("tags: [a, b]\n"),
        "a one-level flow list stays unquoted (the fleet's tags spelling): {on_disk:?}"
    );
}
