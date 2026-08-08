//! § A.6.1 — the view projection's `frontmatter.value` column is a published
//! value plane (wire-contract § A.6.1 seam table, the view row, 2026-08-08).
//!
//! **The defect this pins (d5654f18 non-scope follow-up, fail-INERT).** The
//! projector pushed the stored value bytes into the `value` column, so
//! `owner: "3f9a1c07"` reached every board pivot and analytics query with its
//! quote bytes on — a `WHERE owner = '3f9a1c07'` predicate was false against
//! fleet-canonical data and the query rendered a legitimate-looking empty
//! result. These gates assert the OUTCOME shape where they can: the predicate
//! matches, not just the cell text.
//!
//! § A.6.2 differential: the locator/rev plane stays raw-computed — a
//! quoting-only edit moves `node_rev` (the stored form changed) while the
//! value plane holds still.

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

fn scalar_text(conn: &Connection, sql: &str) -> String {
    conn.query_row(sql, [], |r| r.get(0)).expect("scalar query")
}

fn scalar_i64(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).expect("scalar query")
}

/// The fleet-canonical corpus spellings — the dogfood season-1 read table,
/// which measured every one of these arriving WITH its quote bytes.
#[test]
fn fleet_canonical_spellings_serve_decoded_values() {
    let mut docs = BTreeMap::new();
    docs.insert(
        "card.md".to_string(),
        doc("---\nowner: \"3f9a1c07\"\nparent: \"[[1ed98864]]\"\nstatus: doing\nnote: \"\"\nbare:\nnul: ~\nnulword: null\nsingle: 'it''s'\nmalformed: 'a' and 'b'\n---\n# H\n"),
    );
    let conn = build(&docs);
    let value = |key: &str| {
        scalar_text(
            &conn,
            &format!("SELECT value FROM frontmatter WHERE path='card.md' AND key='{key}'"),
        )
    };

    // Well-formed quoted scalars lose their quote bytes.
    assert_eq!(value("owner"), "3f9a1c07");
    assert_eq!(value("parent"), "[[1ed98864]]");
    assert_eq!(value("single"), "it's");
    // A quoted empty and a bare key both publish the empty string ('' never NULL).
    assert_eq!(value("note"), "");
    assert_eq!(value("bare"), "");
    // Plain scalars are verbatim: the decode is the quoting layer only, never
    // type inference — null spellings stay spelled.
    assert_eq!(value("status"), "doing");
    assert_eq!(value("nul"), "~");
    assert_eq!(value("nulword"), "null");
    // Malformed quoting decodes to itself: no reader may guess at it.
    assert_eq!(value("malformed"), "'a' and 'b'");
}

/// The outcome the defect silenced: a board predicate over the `card` pivot
/// matches fleet-canonical quoted data.
#[test]
fn card_pivot_predicate_matches_quoted_corpus() {
    let mut docs = BTreeMap::new();
    docs.insert(
        "t.md".to_string(),
        doc("---\ntype: \"task\"\nstatus: \"doing\"\nowner: \"3f9a1c07\"\n---\n# T\n"),
    );
    let conn = build(&docs);
    let hits = scalar_i64(
        &conn,
        "SELECT count(*) FROM card WHERE owner = '3f9a1c07' AND status = 'doing'",
    );
    assert_eq!(hits, 1, "the board predicate must see the decoded value");
    let raw_hits = scalar_i64(&conn, "SELECT count(*) FROM card WHERE owner = '\"3f9a1c07\"'");
    assert_eq!(raw_hits, 0, "no quote-tolerant row remains to compare against");
}

/// § A.6.2 differential: a quoting-only edit IS a change to the stored form.
/// The value plane serves the same string for both spellings; the rev plane
/// (raw-computed) still moves.
#[test]
fn quoting_only_edit_moves_node_rev_not_value() {
    let quoted = {
        let mut docs = BTreeMap::new();
        docs.insert("d.md".to_string(), doc("---\nowner: \"3f9a1c07\"\n---\n# H\n"));
        let conn = build(&docs);
        conn.query_row(
            "SELECT value, node_rev FROM frontmatter WHERE path='d.md' AND key='owner'",
            [],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .expect("quoted row")
    };
    let plain = {
        let mut docs = BTreeMap::new();
        docs.insert("d.md".to_string(), doc("---\nowner: 3f9a1c07\n---\n# H\n"));
        let conn = build(&docs);
        conn.query_row(
            "SELECT value, node_rev FROM frontmatter WHERE path='d.md' AND key='owner'",
            [],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .expect("plain row")
    };
    assert_eq!(quoted.0, plain.0, "one value, two spellings");
    assert_ne!(
        quoted.1, plain.1,
        "the rev plane is raw-computed: a quoting-only edit moves the stored form"
    );
}

/// B2 tag parse rides the decoded value: a whole-value-quoted flow list
/// yields clean items, not quote-mangled fragments.
#[test]
fn quoted_flow_list_yields_clean_tag_rows() {
    let mut docs = BTreeMap::new();
    docs.insert(
        "q.md".to_string(),
        doc("---\ntags: \"[type/task, domain/x]\"\n---\n# H\n"),
    );
    let conn = build(&docs);
    let clean = scalar_i64(
        &conn,
        "SELECT count(*) FROM frontmatter_tag WHERE path='q.md' AND tag IN ('type/task','domain/x')",
    );
    assert_eq!(clean, 2, "decoded flow list -> clean items");
    let mangled = scalar_i64(
        &conn,
        "SELECT count(*) FROM frontmatter_tag WHERE path='q.md' AND (tag LIKE '[%' OR tag LIKE '%]')",
    );
    assert_eq!(mangled, 0, "no bracket fragments from parsing through quote bytes");
}
