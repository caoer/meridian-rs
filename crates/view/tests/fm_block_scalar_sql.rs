//! `frontmatter.value` for a YAML BLOCK SCALAR (`>` / `|`) — card
//! `mrd-frontmatter-block-scalar-decoder-gap`, the `sql` half.
//!
//! Sibling of `fm_block_list.rs`, same defect class one shape over: the
//! projector read a block scalar's value off the key LINE, so a page carrying
//! `description: >` and six indented lines projected the INDICATOR BYTE `">"`
//! while `PyYAML` read the whole 459-character text. 45 key rows on 37 live
//! pages were mis-served. The pages are valid YAML — a decoder gap, not corpus
//! damage.
//!
//! **These assertions run a real query against the built view** (`duckdb`), not
//! `model::fm_value` in isolation. That distinction is the lesson of PR 189's
//! first round: the `read`-face matrix asserted one layer above the seam that
//! publishes and passed while the face served the wrong bytes. A gate one layer
//! off its door is not a gate.

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

fn value(conn: &Connection, path: &str, key: &str) -> String {
    conn.query_row(
        &format!("SELECT value FROM frontmatter WHERE path='{path}' AND key='{key}'"),
        [],
        |r| r.get(0),
    )
    .expect("scalar query")
}

fn view_of(raw: &str) -> Connection {
    let mut docs = BTreeMap::new();
    docs.insert("page.md".to_string(), doc(raw));
    build(&docs)
}

/// The card page's own shape: a folded scalar with clip chomping, which is what
/// all 37 live pages carry. The projected value is the folded TEXT, ending in
/// the newline clip chomping keeps — not `">"`, and not the text with its
/// newline shaved.
#[test]
fn a_folded_scalar_projects_its_folded_text() {
    let conn = view_of(
        "---\nname: gtv\ndescription: >\n  first line of the summary\n  second line of it\nmodel: opus\n---\n\n# Body\n",
    );
    assert_eq!(
        value(&conn, "page.md", "description"),
        "first line of the summary second line of it\n",
        "breaks fold to spaces; clip chomping keeps exactly one trailing newline"
    );
    // The keys around it are untouched.
    assert_eq!(value(&conn, "page.md", "name"), "gtv");
    assert_eq!(value(&conn, "page.md", "model"), "opus");
}

/// A literal scalar keeps its breaks, so the projected value carries real
/// newlines — the first `frontmatter.value` rows in this corpus that do.
#[test]
fn a_literal_scalar_projects_its_line_breaks() {
    let conn = view_of("---\nnotes: |\n  first\n  second\n  third\n---\n\n# Body\n");
    let v = value(&conn, "page.md", "notes");
    assert_eq!(v, "first\nsecond\nthird\n");
    assert_eq!(
        v[..v.len() - 1].matches('\n').count(),
        2,
        "two interior line feeds survive the projection"
    );
}

/// Chomping is honoured at the projection, not just in the decoder.
#[test]
fn chomping_reaches_the_projected_row() {
    let strip = view_of("---\nk: >-\n  a\n  b\n---\n\n# B\n");
    assert_eq!(value(&strip, "page.md", "k"), "a b");
    let keep = view_of("---\nk: |+\n  a\n\n\n---\n\n# B\n");
    assert_eq!(value(&keep, "page.md", "k"), "a\n\n\n");
}

/// The indicator byte is GONE from the plane: the census that would have read
/// `">"` for every one of the 45 live rows reads zero here.
#[test]
fn no_row_projects_a_bare_indicator_byte() {
    let conn = view_of(
        "---\nfolded: >\n  x\nliteral: |\n  y\nstripped: >-\n  z\nplain: ordinary\n---\n\n# B\n",
    );
    let indicators: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM frontmatter WHERE value IN ('>', '|', '>-', '|-', '>+', '|+')",
            [],
            |r| r.get(0),
        )
        .expect("census");
    assert_eq!(
        indicators, 0,
        "a header is not a value; the 45-row census reads zero"
    );
    assert_eq!(value(&conn, "page.md", "plain"), "ordinary");
}

/// The SUSPENDED neighbour, pinned so this card cannot drift into it: a block
/// LIST still projects flow-style text here, and the `read` face still serves
/// it empty by contract (`docs/wire-contract.md:2084-2111`). Widening that is
/// ZT's amendment, not this card.
#[test]
fn a_block_list_still_projects_flow_text() {
    let conn = view_of("---\nagents:\n  - 75fbab63\n  - 98cd905e\n---\n\n# B\n");
    assert_eq!(value(&conn, "page.md", "agents"), "[75fbab63, 98cd905e]");
}
