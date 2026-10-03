//! `lock`: one row per pin in a page's `meridian-lock` block, read by
//! `lock::find`, its object resolved as the walk resolves it and a heading
//! path spelled as `section.hpath` (`wire-contract.md` § A.11). A block the
//! parser refuses projects no rows.

use std::collections::BTreeMap;

use lock::Selector::{Path, Properties};

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

fn segs(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_string()).collect()
}

/// One pin per selector shape, plus a cross-root object and an unresolved one.
fn pinning_page() -> String {
    let mut l = lock::Lock::new();
    for (object, selector) in [
        ("notes", Path(segs(&["Top", "Dup#2"]))),
        ("notes", Path(segs(&["^claim"]))),
        ("notes", Properties(segs(&["tags"]))),
        ("notes", Path(Vec::new())),
        ("t/session", Path(segs(&["seq-3"]))),
        ("sessions:x", Path(segs(&["Top"]))),
        ("nowhere", Path(segs(&["Top"]))),
    ] {
        l.upsert_pin(lock::PinEntry::new(
            object,
            "blob1",
            selector,
            "fp1.span2.b3.aa",
        ));
    }
    format!("# Effect\n\ndraws from notes\n\n{}\n", lock::render(&l))
}

/// A one-pin block whose pin would project a row if the block parsed.
fn one_pin_block() -> String {
    let mut l = lock::Lock::new();
    l.upsert_pin(lock::PinEntry::new(
        "notes",
        "blob1",
        Path(Vec::new()),
        "fp1.span2.b3.aa",
    ));
    lock::render(&l)
}

fn view() -> duckdb::Connection {
    let block = one_pin_block();
    let mut ambient = BTreeMap::new();
    for (path, raw) in [
        ("effect.md", pinning_page()),
        (
            "notes.md",
            "---\ntags: [a]\n---\n# Top\n\n## Dup\n\nfirst\n\n## Dup\n\nsecond\n".to_string(),
        ),
        ("t/session.md", "# S\n".to_string()),
        // A well-formed pin row followed by a line the grammar refuses.
        (
            "bad.md",
            format!(
                "# Bad\n\n{}\n",
                block.replace("\n```", "\ngarbage here\n```")
            ),
        ),
        ("two.md", format!("# Two\n\n{block}\n\n{block}\n")),
        ("one.md", format!("# One\n\n{block}\n")),
    ] {
        ambient.insert(path.to_owned(), doc(&raw));
    }
    // A mount the cross-root object names: the walk would colour through it;
    // the relation consults none.
    let mut sessions = BTreeMap::new();
    sessions.insert("x.md".to_owned(), doc("# Top\n"));
    let root = addr::MountName::parse("sessions").expect("a name");
    let corpus = model::RootedCorpus::ambient(&ambient).with_root(root.clone(), &sessions);
    let mounts = addr::MountSet::new([root]);
    view::build_memory_rooted(&ambient, &corpus, &mounts, &fold(&ambient), None, None)
        .expect("view")
}

/// `seq|object|dest_path|arm|selector|hpath` per row of `src`, in `seq` order.
fn rows(conn: &duckdb::Connection, src: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(
            "SELECT seq::VARCHAR || '|' || object || '|' || coalesce(dest_path, 'NULL') || '|' || \
                    arm || '|' || selector || '|' || coalesce(hpath, 'NULL') \
             FROM lock WHERE src_path = ? ORDER BY seq",
        )
        .expect("prepare");
    stmt.query_map([src], |r| r.get(0))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect()
}

#[test]
fn every_pin_is_a_row_and_only_a_heading_path_has_an_hpath() {
    let conn = view();
    assert_eq!(
        rows(&conn, "effect.md"),
        [
            r#"0|notes|notes.md|path|["Top","Dup#2"]|[{"h":"Top"},{"h":"Dup","n":2}]"#,
            r#"1|notes|notes.md|path|["^claim"]|NULL"#,
            r#"2|notes|notes.md|properties|["tags"]|NULL"#,
            "3|notes|notes.md|path|[]|NULL",
            r#"4|t/session|t/session.md|path|["seq-3"]|NULL"#,
            r#"5|sessions:x|NULL|path|["Top"]|[{"h":"Top"}]"#,
            r#"6|nowhere|NULL|path|["Top"]|[{"h":"Top"}]"#,
        ]
    );
    let tokens: (String, String) = conn
        .query_row(
            "SELECT fingerprint, hash FROM lock WHERE src_path = 'effect.md' AND seq = 0",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("the heading pin");
    assert_eq!(tokens, ("fp1.span2.b3.aa".to_owned(), "blob1".to_owned()));
}

#[test]
fn a_heading_pin_joins_the_section_it_names() {
    let conn = view();
    let mut stmt = conn
        .prepare(
            "SELECT l.seq, s.node_seq, s.n FROM lock l \
             JOIN section s ON s.path = l.dest_path AND s.hpath = l.hpath \
             WHERE l.src_path = 'effect.md'",
        )
        .expect("prepare");
    let joined: Vec<(u64, u64, u32)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();
    assert_eq!(
        joined,
        [(0, 2, 2)],
        "the heading pin joins the second Dup; no other pin joins"
    );
}

#[test]
fn a_refused_lock_projects_no_rows() {
    let conn = view();
    assert_eq!(
        rows(&conn, "one.md"),
        ["0|notes|notes.md|path|[]|NULL"],
        "the block parses alone"
    );
    for src in ["bad.md", "two.md"] {
        assert!(rows(&conn, src).is_empty(), "{src} projects no pin rows");
    }
}
