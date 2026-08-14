//! Cards sql-record-rename + sql-task-text-marker — the SV4/CACHE4/s4 bump.
//!
//! `card` → `record`: the noun says what the rows are — one per
//! frontmatter-carrying record, corpus-wide — instead of promising a kanban
//! board (dogfood r6 U-S1). The retired name refuses through the catalog, and
//! nothing in the refusal may resurrect it. `task.text` carries the task text
//! alone: the `- [ ] ` marker duplicated the bit `checked` already encodes
//! (dogfood r6 S11), and grouping by text carried the marker into every key.

use std::collections::BTreeMap;

use duckdb::Connection;
use model::Document;

fn doc(raw: &str) -> Document {
    model::build(raw.to_string(), syntax::parse(raw))
}

/// Corpus fold stamp (version 0 — fixtures declare no domain).
fn fold(docs: &BTreeMap<String, Document>) -> String {
    let files: Vec<(&str, &[u8])> = docs
        .iter()
        .map(|(path, d)| (path.as_str(), d.raw.as_bytes()))
        .collect();
    model::merkle_root(&files, 0).0
}

fn build(docs: &BTreeMap<String, Document>) -> Connection {
    view::build_memory(docs, &fold(docs)).expect("build :memory: view")
}

fn scalar_i64(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).expect("scalar query")
}

/// `record` is the frontmatter pivot under its true name.
#[test]
fn record_serves_the_frontmatter_pivot() {
    let mut docs = BTreeMap::new();
    docs.insert(
        "tasks/t.md".to_string(),
        doc("---\ntype: task\nstatus: doing\nowner: w1\nsession: s\n---\n# T\n"),
    );
    docs.insert("plain.md".to_string(), doc("# No frontmatter\n"));
    let conn = build(&docs);

    let typed = scalar_i64(
        &conn,
        "SELECT count(*) FROM record WHERE type = 'task' AND status = 'doing'",
    );
    assert_eq!(typed, 1, "the typed row serves under `record`");
    let all = scalar_i64(&conn, "SELECT count(*) FROM record");
    assert_eq!(all, 2, "one row per document, untyped ones included");
}

/// The retired name is gone from the catalog, and the refusal does not
/// resurrect it: DuckDB's Did-you-mean draws candidates from the live
/// catalog, so `card` must not be offerable.
#[test]
fn card_refuses_without_resurrecting_the_old_name() {
    let mut docs = BTreeMap::new();
    docs.insert(
        "tasks/t.md".to_string(),
        doc("---\ntype: task\n---\n# T\n"),
    );
    let conn = build(&docs);

    let err = conn
        .query_row("SELECT count(*) FROM card", [], |r| r.get::<_, i64>(0))
        .expect_err("`card` must no longer exist");
    let msg = err.to_string();
    assert!(
        msg.contains("card") && msg.contains("does not exist"),
        "the refusal names the unknown table, got: {msg}"
    );
    assert!(
        !msg.contains("Did you mean \"card\""),
        "no catalog entry may offer the retired name back, got: {msg}"
    );
}

/// `task.text` is the text alone — every list-marker + checkbox spelling is
/// stripped, and `checked`/`depth` still carry their bits.
#[test]
fn task_text_is_marker_free() {
    let raw = "\
# Board

- [ ] buy milk
- [x] ship the fix
* [ ] star bullet
+ [X] plus bullet, caps marker
1. [ ] ordered item
    - [ ] nested child
- [ ]
";
    let mut docs = BTreeMap::new();
    docs.insert("t.md".to_string(), doc(raw));
    let conn = build(&docs);

    let mut stmt = conn
        .prepare("SELECT text, checked FROM task ORDER BY seq")
        .expect("prepare");
    let rows: Vec<(String, bool)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("rows");

    let texts: Vec<&str> = rows.iter().map(|(t, _)| t.as_str()).collect();
    let expected: Vec<&str> = vec![
        "buy milk",
        "ship the fix",
        "star bullet",
        "plus bullet, caps marker",
        "ordered item",
        "nested child",
        "",
    ];
    assert_eq!(texts, expected, "text is the task text, never the marker");

    let checked: Vec<bool> = rows.iter().map(|(_, c)| *c).collect();
    assert_eq!(
        checked,
        vec![false, true, false, true, false, false, false],
        "the bit still rides `checked` alone"
    );
}
