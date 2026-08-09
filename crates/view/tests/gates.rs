//! DDL-enforced design validation gates (binding design §Validation).

use std::collections::BTreeMap;

use duckdb::Connection;
use model::Document;
use view::create_schema;

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

fn fresh_schema() -> Connection {
    let conn = Connection::open_in_memory().expect("open :memory:");
    create_schema(&conn).expect("create schema");
    conn
}

fn insert_doc(conn: &Connection, path: &str) {
    conn.execute(
        "INSERT INTO doc (path, file_rev, line_count, bytes) VALUES (?, 'rev', 1, 1)",
        [path],
    )
    .expect("insert doc");
}

fn scalar_i64(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).expect("scalar query")
}

/// Two-doc fixture (gate 18): dup hpath, doc-level task, link shapes.
fn fixture() -> BTreeMap<String, Document> {
    let a = "\
---
type: task
status: open
tags: [type/task, project/x]
---
- [ ] top-level task

# Steps

- [ ] first step
- [x] done step

Refs: [[b]] and [[missing-doc]] and [[#Steps]] and [ext](https://example.com).

# Steps

- [ ] second-steps task
";
    let b = "\
---
type: note
---
# B Doc

Body with an #inlinetag here.

- [ ] b task
";
    let mut docs = BTreeMap::new();
    docs.insert("b.md".to_string(), doc(b));
    docs.insert("a.md".to_string(), doc(a));
    docs
}

// ---------------------------------------------------------------------------
// Gate 1 — schema parses clean
// ---------------------------------------------------------------------------

#[test]
fn gate1_schema_parses_clean() {
    let conn = fresh_schema();
    let tables: i64 = scalar_i64(
        &conn,
        "SELECT count(*) FROM information_schema.tables \
         WHERE table_name IN ('_meridian_view','doc','frontmatter','section','link','tag','frontmatter_tag','task','backlink','dangling','card','tag_all')",
    );
    assert_eq!(tables, 12, "8 physical tables + 4 SQL views");
    println!("===SCHEMA PARSED OK===");
}

// ---------------------------------------------------------------------------
// Gate 4 — _meridian_view singleton
// ---------------------------------------------------------------------------

#[test]
fn gate4_singleton_second_row_is_pk_error() {
    let conn = fresh_schema();
    let stamp = "INSERT INTO _meridian_view \
        (singleton, schema_version, as_of_fingerprint, workspace, built_unix, builder, doc_count) \
        VALUES (true, 1, 'b3:x', '/ws', 0, 'test', 0)";
    conn.execute(stamp, []).expect("first stamp inserts");
    let err = conn.execute(stamp, []).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("primary key")
            || msg.contains("Duplicate")
            || msg.to_lowercase().contains("constraint"),
        "second singleton must violate the PK, got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Gate 5 — task -> section identity FK (never a heading-text join)
// ---------------------------------------------------------------------------

#[test]
fn gate5_task_section_identity_fk() {
    let conn = fresh_schema();
    insert_doc(&conn, "x.md");
    // two sections sharing the heading text 'Steps' (identity keeps them distinct)
    conn.execute(
        "INSERT INTO section VALUES ('x.md', 0, ['Steps'], 'Steps', 1, 'r', 0, 10)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO section VALUES ('x.md', 1, ['Steps'], 'Steps', 1, 'r', 10, 20)",
        [],
    )
    .unwrap();
    // one task under section node_seq 0
    conn.execute(
        "INSERT INTO task VALUES ('x.md', 0, false, 0, 0, ['Steps'], 'a task', 1, 5, 'r')",
        [],
    )
    .unwrap();

    // identity join: the task pairs with exactly ONE section (node_seq identity)
    let identity = scalar_i64(
        &conn,
        "SELECT count(*) FROM task t JOIN section s ON s.path=t.path AND s.node_seq=t.section_seq WHERE s.heading='Steps'",
    );
    assert_eq!(identity, 1, "identity join returns the task once");
    // heading-text join: pairs with BOTH 'Steps' sections -> double counts
    let by_text = scalar_i64(
        &conn,
        "SELECT count(*) FROM task t JOIN section s ON s.path=t.path AND s.heading='Steps'",
    );
    assert_eq!(
        by_text, 2,
        "heading-text join double-counts across duplicate headings"
    );

    // a headingless (document-level) task is legal: section_seq NULL, MATCH-SIMPLE
    conn.execute(
        "INSERT INTO task VALUES ('x.md', 1, false, 0, NULL, NULL, 'doc-level', 20, 25, 'r')",
        [],
    )
    .expect("headingless task legal");

    // a bad section_seq (no such section) violates the composite FK
    let err = conn
        .execute(
            "INSERT INTO task VALUES ('x.md', 2, false, 0, 99, ['Steps'], 'bad', 25, 30, 'r')",
            [],
        )
        .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("key")
            || err.to_string().to_lowercase().contains("constraint"),
        "bad section_seq must fail the FK, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// Gate 6 — resolved generated + link integrity
// ---------------------------------------------------------------------------

#[test]
fn gate6_resolved_generated_and_link_integrity() {
    let conn = fresh_schema();
    insert_doc(&conn, "x.md");

    // inserting the generated `resolved` column is a Binder Error
    let err = conn
        .execute(
            "INSERT INTO link (src_path, seq, kind, target_raw, resolved, span_start, span_end, node_rev) \
             VALUES ('x.md', 0, 'wikilink', 't', true, 0, 1, 'r')",
            [],
        )
        .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("generated"),
        "insert into generated column must be refused, got: {err}"
    );

    // a valid resolved link agrees with dest_path
    conn.execute(
        "INSERT INTO link (src_path, seq, kind, target_raw, dest_path, span_start, span_end, node_rev) \
         VALUES ('x.md', 1, 'wikilink', 't', 'x.md', 0, 1, 'r')",
        [],
    )
    .unwrap();
    let resolved = scalar_i64(&conn, "SELECT count(*) FROM link WHERE seq=1 AND resolved");
    assert_eq!(resolved, 1, "resolved is true when dest_path is set");

    // an external `link` carrying a non-NULL dest violates the CHECK
    let err = conn
        .execute(
            "INSERT INTO link (src_path, seq, kind, target_raw, dest_path, span_start, span_end, node_rev) \
             VALUES ('x.md', 2, 'link', 'http', 'x.md', 0, 1, 'r')",
            [],
        )
        .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("check"),
        "external link with a dest must fail CHECK, got: {err}"
    );

    // a dest_path to a non-doc violates the FK
    let err = conn
        .execute(
            "INSERT INTO link (src_path, seq, kind, target_raw, dest_path, span_start, span_end, node_rev) \
             VALUES ('x.md', 3, 'wikilink', 't', 'nope.md', 0, 1, 'r')",
            [],
        )
        .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("key")
            || err.to_string().to_lowercase().contains("constraint"),
        "dest to a non-doc must fail the FK, got: {err}"
    );

    // a bad kind violates the CHECK
    let err = conn
        .execute(
            "INSERT INTO link (src_path, seq, kind, target_raw, span_start, span_end, node_rev) \
             VALUES ('x.md', 4, 'bogus', 't', 0, 1, 'r')",
            [],
        )
        .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("check"),
        "bad kind must fail CHECK, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// Gate 7 — view-never-store: INSERT on a READ_ONLY attach refused
// ---------------------------------------------------------------------------

#[test]
fn gate7_readonly_attach_refuses_insert() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("ro.duckdb");
    {
        let conn = Connection::open(&path).unwrap();
        create_schema(&conn).unwrap();
        conn.close().unwrap();
    }
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(&format!("ATTACH '{}' AS ro (READ_ONLY)", path.display()))
        .unwrap();
    let err = conn
        .execute(
            "INSERT INTO ro.doc (path, file_rev, line_count, bytes) VALUES ('x.md','r',1,1)",
            [],
        )
        .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("read-only")
            || err.to_string().to_lowercase().contains("read only"),
        "INSERT on a READ_ONLY attach must be refused, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// Gate 8 — domain-prefix guard
// ---------------------------------------------------------------------------

#[test]
fn gate8_domain_prefix_guard() {
    let conn = Connection::open_in_memory().unwrap();
    let eq: bool = conn
        .query_row(
            "SELECT ('b3:' || repeat('a', 64)) = ('b3a:' || repeat('a', 64))",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        !eq,
        "a domain bump makes fingerprint tokens never compare equal even when hex repeats"
    );
}

// ---------------------------------------------------------------------------
// Gate 17 — frontmatter scalar -> tag rows (B2)
// ---------------------------------------------------------------------------

#[test]
fn gate17_frontmatter_scalar_to_tag_rows() {
    // inline-list scalar -> 2 rows
    let mut docs = BTreeMap::new();
    docs.insert(
        "inline.md".to_string(),
        doc("---\ntags: [type/agent, type/task]\n---\n# H\n"),
    );
    let conn = view::build_memory(&docs, &fold(&docs)).unwrap();
    let n = scalar_i64(
        &conn,
        "SELECT count(*) FROM frontmatter_tag WHERE path='inline.md'",
    );
    assert_eq!(
        n, 2,
        "inline-list [type/agent, type/task] -> 2 frontmatter_tag rows"
    );

    // both `tag` and `tags` keys coexist without a PK collision (separate seq spaces)
    let mut docs = BTreeMap::new();
    docs.insert(
        "both.md".to_string(),
        doc("---\ntag: solo\ntags: [one, two]\n---\n# H\n"),
    );
    let conn = view::build_memory(&docs, &fold(&docs)).unwrap();
    let singular_rows = scalar_i64(
        &conn,
        "SELECT count(*) FROM frontmatter_tag WHERE key='tag'",
    );
    let plural_rows = scalar_i64(
        &conn,
        "SELECT count(*) FROM frontmatter_tag WHERE key='tags'",
    );
    assert_eq!(singular_rows, 1, "`tag` key -> 1 row (seq 0)");
    assert_eq!(plural_rows, 2, "`tags` key -> 2 rows (its own seq space)");
    let coexist = scalar_i64(&conn, "SELECT count(*) FROM frontmatter_tag WHERE seq=0");
    assert_eq!(coexist, 2, "tag seq 0 and tags seq 0 coexist (key in PK)");
}

#[test]
fn gate17_block_form_frontmatter_is_zero_rows_fail_closed() {
    // a block-list frontmatter stores value '' -> 0 rows (fail-closed, never wrong rows)
    let mut docs = BTreeMap::new();
    docs.insert(
        "block.md".to_string(),
        doc("---\ntags:\n  - type/agent\n  - type/task\n---\n# H\n"),
    );
    let conn = view::build_memory(&docs, &fold(&docs)).unwrap();
    let n = scalar_i64(
        &conn,
        "SELECT count(*) FROM frontmatter_tag WHERE path='block.md'",
    );
    assert_eq!(
        n, 0,
        "block-list frontmatter projects 0 tag rows (fail-closed)"
    );
}

// ---------------------------------------------------------------------------
// Gate 18 — end-to-end fixture (V1 §Q5 gate)
// ---------------------------------------------------------------------------

#[test]
fn gate18_end_to_end_fixture() {
    let docs = fixture();
    let conn = view::build_memory(&docs, &fold(&docs)).expect("build_memory");

    // 2 docs
    assert_eq!(scalar_i64(&conn, "SELECT count(*) FROM doc"), 2);

    // every fact row carries a non-null node_rev
    for table in [
        "frontmatter",
        "section",
        "link",
        "tag",
        "frontmatter_tag",
        "task",
    ] {
        let nulls = scalar_i64(
            &conn,
            &format!("SELECT count(*) FROM {table} WHERE node_rev IS NULL"),
        );
        assert_eq!(nulls, 0, "{table} rows all carry a non-null node_rev");
    }

    // backlinks: b.md has exactly one inbound edge, from a.md ([[b]] resolved)
    let b_backlinks = scalar_i64(
        &conn,
        "SELECT count(*) FROM backlink WHERE path='b.md' AND src_path='a.md'",
    );
    assert_eq!(
        b_backlinks, 1,
        "resolved [[b]] is a backlink from a.md to b.md"
    );

    // dangling: [[missing-doc]] appears; the external link and the self-link do not
    let dangling = scalar_i64(
        &conn,
        "SELECT count(*) FROM dangling WHERE src_path='a.md' AND target_raw='missing-doc'",
    );
    assert_eq!(
        dangling, 1,
        "the broken vault ref [[missing-doc]] is dangling"
    );
    let ext_dangling = scalar_i64(
        &conn,
        "SELECT count(*) FROM dangling WHERE src_path='a.md' AND target_raw LIKE 'http%'",
    );
    assert_eq!(
        ext_dangling, 0,
        "an external markdown link is never dangling"
    );

    // a document-level task exists (section_seq NULL)
    let doc_level = scalar_i64(
        &conn,
        "SELECT count(*) FROM task WHERE path='a.md' AND section_seq IS NULL",
    );
    assert!(
        doc_level >= 1,
        "the pre-heading task is document-level (section_seq NULL)"
    );

    // Q4 identity join vs the double-counting heading-text join over 'Steps'
    let identity = scalar_i64(
        &conn,
        "SELECT count(*) FROM task t JOIN section s ON s.path=t.path AND s.node_seq=t.section_seq WHERE NOT t.checked AND s.heading='Steps'",
    );
    let by_text = scalar_i64(
        &conn,
        "SELECT count(*) FROM task t JOIN section s ON s.path=t.path AND s.heading='Steps' WHERE NOT t.checked",
    );
    assert!(
        by_text > identity,
        "the heading-text join double-counts across the duplicate 'Steps' sections ({by_text} > {identity})"
    );
}
