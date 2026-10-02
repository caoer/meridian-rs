//! `link.candidates`: every page a written target matches, sorted by path,
//! when it matches more than one — NULL otherwise. `dest_path` is the pick
//! among them (`wire-contract.md` § A.11).

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

fn view() -> duckdb::Connection {
    let mut ambient = BTreeMap::new();
    for (path, raw) in [
        ("a/x.md", "# A x\n"),
        ("b/x.md", "# B x\n"),
        (
            "c/p.md",
            "# P\n\n[[x]]\n[[a/x]]\n[[nowhere]]\n[[sessions:x.md]]\n[[Shared]]\n",
        ),
        ("a/q.md", "# Q\n\n[[x]]\n"),
        ("b/q.md", "# Q\n\n[[x]]\n"),
        ("d/one.md", "---\naliases: [Shared]\n---\n# One\n"),
        ("e/two.md", "---\naliases: [Shared]\n---\n# Two\n"),
    ] {
        ambient.insert(path.to_owned(), doc(raw));
    }
    let mut sessions = BTreeMap::new();
    sessions.insert("x.md".to_owned(), doc("# Sessions x\n"));
    let root = addr::MountName::parse("sessions").expect("a name");
    let corpus = model::RootedCorpus::ambient(&ambient).with_root(root.clone(), &sessions);
    let mounts = addr::MountSet::new([root]);
    view::build_memory_rooted(&ambient, &corpus, &mounts, &fold(&ambient), None, None)
        .expect("view")
}

/// `(dest, candidates)` of the one link `src` writes as `target`, both as text.
fn link(conn: &duckdb::Connection, src: &str, target: &str) -> (String, String) {
    conn.query_row(
        "SELECT coalesce(dest_path, dest_root || ':' || dest_root_path, 'NULL'), \
                coalesce(candidates::VARCHAR, 'NULL') \
         FROM link WHERE src_path = ? AND target_raw = ?",
        [src, target],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .expect("one link row")
}

#[test]
fn a_bare_name_two_pages_carry_lists_both() {
    let conn = view();
    assert_eq!(
        link(&conn, "c/p.md", "x"),
        ("a/x.md".to_owned(), "[a/x.md, b/x.md]".to_owned())
    );
}

#[test]
fn a_qualified_name_one_page_matches_is_null() {
    let conn = view();
    assert_eq!(
        link(&conn, "c/p.md", "a/x"),
        ("a/x.md".to_owned(), "NULL".to_owned())
    );
}

#[test]
fn a_sibling_pick_still_lists_every_candidate() {
    let conn = view();
    assert_eq!(
        link(&conn, "a/q.md", "x"),
        ("a/x.md".to_owned(), "[a/x.md, b/x.md]".to_owned())
    );
    // The sibling beats the first-by-name; the list does not follow the pick.
    assert_eq!(
        link(&conn, "b/q.md", "x"),
        ("b/x.md".to_owned(), "[a/x.md, b/x.md]".to_owned())
    );
}

#[test]
fn an_alias_two_pages_carry_lists_both() {
    let conn = view();
    assert_eq!(
        link(&conn, "c/p.md", "Shared"),
        ("d/one.md".to_owned(), "[d/one.md, e/two.md]".to_owned())
    );
}

#[test]
fn dangling_and_rooted_targets_are_null() {
    let conn = view();
    assert_eq!(
        link(&conn, "c/p.md", "nowhere"),
        ("NULL".to_owned(), "NULL".to_owned())
    );
    assert_eq!(
        link(&conn, "c/p.md", "sessions:x.md"),
        ("sessions:x.md".to_owned(), "NULL".to_owned())
    );
}

#[test]
fn the_choice_predicate_and_unnest_answer_which_links_and_among_which_pages() {
    let conn = view();
    let mut stmt = conn
        .prepare(
            "SELECT src_path || ' ' || target_raw || ' -> ' || unnest(candidates) FROM link \
             WHERE candidates IS NOT NULL ORDER BY 1",
        )
        .expect("prepare");
    let rows: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();
    assert_eq!(
        rows,
        [
            "a/q.md x -> a/x.md",
            "a/q.md x -> b/x.md",
            "b/q.md x -> a/x.md",
            "b/q.md x -> b/x.md",
            "c/p.md Shared -> d/one.md",
            "c/p.md Shared -> e/two.md",
            "c/p.md x -> a/x.md",
            "c/p.md x -> b/x.md",
        ]
    );
}
