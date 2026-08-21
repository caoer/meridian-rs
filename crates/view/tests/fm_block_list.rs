//! `frontmatter.value` for a YAML block sequence — the generic list plane
//! (card `fm-block-list-sql-empty`).
//!
//! **The defect this pins (fail-INERT, measured 2026-08-21 on engine
//! `39dd8ccc8`).** Every `agents:` on the fleet corpus is a block sequence
//! (`agents:` then indented `- <id>` lines), and the projector read its value
//! off the flat [`model::YamlMap`], which keeps only the key LINE's remainder:
//!
//! ```sql
//! SELECT COUNT(*), SUM(CASE WHEN value = '' THEN 1 ELSE 0 END)
//! FROM frontmatter WHERE key='agents'   -- → 50, 50
//! ```
//!
//! Fifty rosters, fifty empty strings, while every one named its ids on the
//! following lines. A sql absence claim over a list-valued key read clean and
//! proved nothing. `model::fm_tags` closed the same blindness for `tag`/`tags`
//! only (card `tag-all-block-form-blindness`); this plane is EVERY key.
//!
//! The published shape is the flow-style text the same list would carry on
//! the key line — `[a, b]`, items verbatim — so a consumer splits a block list
//! and a flow list the same way (`model::parse_alias_list` /
//! `model::parse_tag_list` already do).

use std::collections::BTreeMap;

use duckdb::Connection;

fn doc(raw: &str) -> std::sync::Arc<model::Document> {
    std::sync::Arc::new(model::build(raw.to_string(), syntax::parse(raw)))
}

/// Corpus fold stamp (version 0 — fixtures declare no domain).
fn fold(docs: &model::Docs) -> String {
    let files: Vec<(&str, &[u8])> = docs
        .iter()
        .map(|(path, d)| (path.as_str(), d.raw.as_bytes()))
        .collect();
    model::merkle_root(&files, 0).0
}

fn build(docs: &model::Docs) -> Connection {
    view::build_memory(docs, &fold(docs)).expect("build :memory: view")
}

fn scalar_text(conn: &Connection, sql: &str) -> String {
    conn.query_row(sql, [], |r| r.get(0)).expect("scalar query")
}

fn scalar_i64(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).expect("scalar query")
}

fn value(conn: &Connection, path: &str, key: &str) -> String {
    scalar_text(
        conn,
        &format!("SELECT value FROM frontmatter WHERE path='{path}' AND key='{key}'"),
    )
}

/// The fleet roster shape, verbatim from
/// `rosters/0016-worker-engine-run-put-defects.md` (the card's first gate).
const ROSTER: &str = "\
---
type: roster
tags: [type/roster]
role: worker
agents:
  - 75fbab63
  - 98cd905e
  - d6e4a57a
created: 2026-08-21T15:54:24-0400
---

# worker-engine-run-put-defects
";

/// The 50/50 probe, as a gate: a block-sequence `agents:` projects its ids,
/// and the empty-string census that read 50/50 on the fleet reads zero.
#[test]
fn block_sequence_agents_project_their_ids() {
    let mut docs = BTreeMap::new();
    docs.insert("rosters/0016.md".to_string(), doc(ROSTER));
    let conn = build(&docs);

    assert_eq!(
        value(&conn, "rosters/0016.md", "agents"),
        "[75fbab63, 98cd905e, d6e4a57a]",
        "a block sequence publishes the flow-style text it spells"
    );
    let empties = scalar_i64(
        &conn,
        "SELECT SUM(CASE WHEN value = '' THEN 1 ELSE 0 END) FROM frontmatter WHERE key='agents'",
    );
    assert_eq!(empties, 0, "the 50/50 census reads zero");

    // The keys around it are untouched: scalars and a flow list on the key
    // line serve exactly what they served before.
    assert_eq!(value(&conn, "rosters/0016.md", "role"), "worker");
    assert_eq!(value(&conn, "rosters/0016.md", "tags"), "[type/roster]");
    assert_eq!(
        value(&conn, "rosters/0016.md", "created"),
        "2026-08-21T15:54:24-0400"
    );
}

/// Universal, not an `agents:` special case: any key, and the items keep
/// their spelling — a quoted item renders quoted, as the flow form on the key
/// line would have been served verbatim (§ A.6.1: a flow collection is plain).
#[test]
fn every_list_valued_key_renders_flow_style() {
    let mut docs = BTreeMap::new();
    docs.insert(
        "card.md".to_string(),
        doc("---\nhandoff-to:\n  - \"[[a1b2c3d4]]\"\n  - '[[e5f6a7b8]]'\ninputs:\n  # pinned floors\n  - floor/one\n\n  - floor/two\n---\n# H\n"),
    );
    let conn = build(&docs);
    assert_eq!(
        value(&conn, "card.md", "handoff-to"),
        "[\"[[a1b2c3d4]]\", '[[e5f6a7b8]]']"
    );
    assert_eq!(
        value(&conn, "card.md", "inputs"),
        "[floor/one, floor/two]",
        "comment and blank lines inside a sequence are skipped, never items"
    );
}

/// The four shapes the card names, side by side on one page: block list,
/// flow list, empty list, scalar — plus the bare key, which is NOT a list.
#[test]
fn block_flow_empty_scalar_and_bare() {
    let mut docs = BTreeMap::new();
    docs.insert(
        "shapes.md".to_string(),
        doc("---\nblock:\n  - a\n  - b\nflow: [a, b]\nempty: []\nscalar: a\nbare:\nquoted: \"[a, b]\"\nsplit: [a,\n  b]\n---\n# H\n"),
    );
    let conn = build(&docs);
    assert_eq!(value(&conn, "shapes.md", "block"), "[a, b]");
    assert_eq!(value(&conn, "shapes.md", "flow"), "[a, b]");
    assert_eq!(value(&conn, "shapes.md", "empty"), "[]");
    assert_eq!(value(&conn, "shapes.md", "scalar"), "a");
    assert_eq!(
        value(&conn, "shapes.md", "bare"),
        "",
        "a key line with nothing after it and no sequence below is the empty string — never an invented `[]`"
    );
    assert_eq!(
        value(&conn, "shapes.md", "quoted"),
        "[a, b]",
        "§ A.6.1 decode still runs first: a quoted scalar is served unquoted"
    );
    assert_eq!(
        value(&conn, "shapes.md", "split"),
        "[a, b]",
        "a multi-line flow sequence joins, the way the tag lane already joined it"
    );
}

/// Fail-closed still governs the shape the walk cannot read: an indented
/// non-item stops it, serving what it read and never a guess.
#[test]
fn a_nested_shape_ends_the_walk() {
    let mut docs = BTreeMap::new();
    docs.insert(
        "nested.md".to_string(),
        doc("---\nagents:\n  - a\n  nested:\n    - b\n---\n# H\n"),
    );
    let conn = build(&docs);
    assert_eq!(value(&conn, "nested.md", "agents"), "[a]");
}

/// The tag lane is unchanged by sharing its walk: `frontmatter_tag` still
/// projects a block-sequence `tags:` item by item (gate 17's block form).
#[test]
fn tag_rows_still_project_from_the_shared_walk() {
    let mut docs = BTreeMap::new();
    docs.insert(
        "block.md".to_string(),
        doc("---\ntags:\n  - type/agent\n  - type/task\n---\n# H\n"),
    );
    let conn = build(&docs);
    let tags: Vec<String> = conn
        .prepare("SELECT tag FROM frontmatter_tag WHERE path='block.md' ORDER BY seq")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        tags,
        vec!["type/agent".to_string(), "type/task".to_string()]
    );
    assert_eq!(
        value(&conn, "block.md", "tags"),
        "[type/agent, type/task]",
        "and the value column carries the same list as flow text"
    );
}
