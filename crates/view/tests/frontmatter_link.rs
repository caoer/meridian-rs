//! `frontmatter_link`: one row per wikilink in a frontmatter value that names
//! a page, whatever the value's form, resolved by the body link's call
//! (`wire-contract.md` § A.11). `link` stays body-only.

use std::collections::BTreeMap;

fn doc(raw: &str) -> std::sync::Arc<model::Document> {
    std::sync::Arc::new(model::build(raw.to_string(), syntax::parse(raw)))
}

fn fold(docs: &model::Docs) -> String {
    let files: Vec<(&str, &[u8])> = docs
        .iter()
        .map(|(path, d)| (path.as_str(), d.raw.as_bytes()))
        .collect();
    model::merkle_root(&files, 0).0
}

const SHAPES: &str = "---
source: \"[[a/x]]\"
see: [\"[[y]]\", \"[[a/x#Top|the top]]\"]
related:
  - \"[[y]]\"
also: \"[[y]], [[nowhere]]\"
description: written at [[y]] by hand
task.fetch: \"[[#^fetch]]\"
remote: \"[[sessions:x.md]]\"
view: \"[[TAGS.base]]\"
---
# P

[[y]]
";

fn view() -> duckdb::Connection {
    let mut ambient = BTreeMap::new();
    for (path, raw) in [
        ("a/x.md", "# A x\n"),
        ("b/x.md", "# B x\n"),
        ("y.md", "# Y\n"),
        ("c/shapes.md", SHAPES),
        ("c/p.md", "---\nsource: \"[[x]]\"\n---\n# P\n"),
        ("a/q.md", "---\nsource: \"[[x]]\"\n---\n# Q\n"),
    ] {
        ambient.insert(path.to_owned(), doc(raw));
    }
    // A mount the rooted target names: a body link would resolve through it;
    // a frontmatter link consults none.
    let mut sessions = BTreeMap::new();
    sessions.insert("x.md".to_owned(), doc("# Sessions x\n"));
    let root = addr::MountName::parse("sessions").expect("a name");
    let corpus = model::RootedCorpus::ambient(&ambient).with_root(root.clone(), &sessions);
    let mounts = addr::MountSet::new([root]);
    view::build_memory_rooted(&ambient, &corpus, &mounts, &fold(&ambient), None, None)
        .expect("view")
}

/// `seq|key|target_raw|dest_path|candidates` per row of `src`, in `seq` order.
fn rows(conn: &duckdb::Connection, src: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(
            "SELECT seq::VARCHAR || '|' || key || '|' || target_raw || '|' || \
                    coalesce(dest_path, 'NULL') || '|' || coalesce(candidates::VARCHAR, 'NULL') \
             FROM frontmatter_link WHERE src_path = ? ORDER BY seq",
        )
        .expect("prepare");
    stmt.query_map([src], |r| r.get(0))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect()
}

#[test]
fn every_value_form_gives_its_rows_and_an_anchor_gives_none() {
    let conn = view();
    assert_eq!(
        rows(&conn, "c/shapes.md"),
        [
            "0|source|a/x|a/x.md|NULL",
            "1|see|y|y.md|NULL",
            "2|see|a/x|a/x.md|NULL",
            "3|related|y|y.md|NULL",
            "4|also|y|y.md|NULL",
            "5|also|nowhere|NULL|NULL",
            "6|description|y|y.md|NULL",
            "7|remote|sessions:x.md|NULL|NULL",
            "8|view|TAGS.base|NULL|NULL",
        ]
    );
}

#[test]
fn link_keeps_the_body_links_alone() {
    let conn = view();
    let targets: Vec<String> = conn
        .prepare("SELECT target_raw FROM link WHERE src_path = 'c/shapes.md' ORDER BY seq")
        .expect("prepare")
        .query_map([], |r| r.get(0))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();
    assert_eq!(targets, ["y"]);
}

#[test]
fn a_bare_name_two_pages_carry_lists_both() {
    let conn = view();
    assert_eq!(
        rows(&conn, "c/p.md"),
        ["0|source|x|a/x.md|[a/x.md, b/x.md]"]
    );
}

#[test]
fn a_sibling_pick_still_lists_every_candidate() {
    let conn = view();
    assert_eq!(
        rows(&conn, "a/q.md"),
        ["0|source|x|a/x.md|[a/x.md, b/x.md]"]
    );
}

#[test]
fn every_key_is_a_key_of_the_frontmatter_relation() {
    let conn = view();
    let orphans: i64 = conn
        .query_row(
            "SELECT count(*) FROM frontmatter_link l \
             ANTI JOIN frontmatter f ON f.path = l.src_path AND f.key = l.key",
            [],
            |r| r.get(0),
        )
        .expect("count");
    assert_eq!(orphans, 0);
}
