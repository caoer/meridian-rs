//! U21: cross-root link row. `dangling` is `dest_path IS NULL AND dest_root IS
//! NULL` — second clause stops resolved cross-root (`NULL` `dest_path` by FK) from
//! reading as broken.

use std::collections::BTreeMap;

use model::Document;

fn doc(raw: &str) -> std::sync::Arc<model::Document> {
    std::sync::Arc::new(model::build(raw.to_string(), syntax::parse(raw)))
}

fn fixture() -> (model::Docs, model::Docs) {
    let mut ambient = BTreeMap::new();
    ambient.insert(
        "claim.md".to_owned(),
        doc("# Claim\n\n[[sessions:notes.md]]\n[[local.md]]\n[[nowhere.md]]\n"),
    );
    ambient.insert("local.md".to_owned(), doc("# Local\n"));
    // Ambient decoy: wrong-bytes success if cross-root resolves here.
    ambient.insert(
        "notes.md".to_owned(),
        doc("# Ambient notes — the wrong one\n"),
    );

    let mut sessions = BTreeMap::new();
    sessions.insert("notes.md".to_owned(), doc("# Notes — the sessions root\n"));
    (ambient, sessions)
}

/// Corpus fold stamp (version 0 — fixtures declare no domain).
fn fold(docs: &model::Docs) -> String {
    let files: Vec<(&str, &[u8])> = docs
        .iter()
        .map(|(path, d)| (path.as_str(), d.raw.as_bytes()))
        .collect();
    model::merkle_root(&files, 0).0
}

fn one_text(conn: &duckdb::Connection, sql: &str) -> Vec<String> {
    let mut stmt = conn.prepare(sql).expect("prepare");
    stmt.query_map([], |r| r.get::<_, Option<String>>(0))
        .expect("query")
        .map(|r| r.expect("row").unwrap_or_else(|| "NULL".to_owned()))
        .collect()
}

/// Resolved cross-root is not dangling; lands in `dest_root`/`dest_root_path`.
#[test]
fn a_resolved_cross_root_edge_is_not_dangling_and_keeps_dest_path_free() {
    let (ambient, sessions) = fixture();
    let root = addr::MountName::parse("sessions").expect("a name");
    let corpus = model::RootedCorpus::ambient(&ambient).with_root(root.clone(), &sessions);
    let mounts = addr::MountSet::new([root]);
    let conn = view::build_memory_rooted(&ambient, &corpus, &mounts, &fold(&ambient), None, None)
        .expect("view");

    assert_eq!(
        one_text(
            &conn,
            "SELECT dest_root || '|' || dest_root_path FROM link \
             WHERE target_raw = 'sessions:notes.md'"
        ),
        vec!["sessions|notes.md"],
        "the cross-root destination is two columns, joined only for this assertion",
    );
    assert_eq!(
        one_text(
            &conn,
            "SELECT coalesce(dest_path,'NULL') FROM link WHERE target_raw = 'sessions:notes.md'"
        ),
        vec!["NULL"],
        "dest_path means 'a path in THIS corpus', always — never only sometimes",
    );

    // Clause under test: without `AND dest_root IS NULL` this row is false-dangling.
    assert_eq!(
        one_text(&conn, "SELECT target_raw FROM dangling ORDER BY target_raw"),
        vec!["nowhere.md"],
        "only the genuinely broken ambient ref is dangling; the resolved \
         cross-root edge is not",
    );

    // The generated column agrees — it is the other half of the same clause.
    assert_eq!(
        one_text(
            &conn,
            "SELECT CAST(resolved AS TEXT) FROM link WHERE target_raw = 'sessions:notes.md'"
        ),
        vec!["true"],
        "a cross-root edge that resolved IS resolved",
    );
}

/// The ambient projection is byte-unchanged: without this, the pin above is
/// satisfied by a projector that resolves nothing at all.
#[test]
fn the_ambient_projection_is_unchanged_by_the_cross_root_columns() {
    let (ambient, _) = fixture();
    let conn = view::build_memory(&ambient, &fold(&ambient)).expect("view");

    assert_eq!(
        one_text(
            &conn,
            "SELECT coalesce(dest_path,'NULL') FROM link WHERE target_raw = 'local.md'"
        ),
        vec!["local.md"],
        "an ordinary ambient link still resolves",
    );
    // With NO mount authority a rooted spelling stays dangling — the pre-U21
    // answer, and the daemon's discipline: not having looked is not a finding.
    assert_eq!(
        one_text(&conn, "SELECT target_raw FROM dangling ORDER BY target_raw"),
        vec!["nowhere.md", "sessions:notes.md"],
        "no mount authority ⇒ a rooted spelling is dangling, exactly as before",
    );
}

/// **Condition 1 — the illegal states are UNREPRESENTABLE, not merely avoided.**
/// The third column widens the error space; the CHECK constraints close it in
/// the schema rather than in the projector's discipline.
#[test]
fn the_schema_refuses_an_inconsistent_destination() {
    let docs = BTreeMap::new();
    let conn = view::build_memory(&docs, &fold(&docs)).expect("view");
    conn.execute_batch("INSERT INTO doc VALUES ('claim.md','r',1,10);")
        .expect("doc row");

    let insert = |dest_path: &str, dest_root: &str, dest_root_path: &str| {
        conn.execute_batch(&format!(
            "INSERT INTO link (src_path,seq,kind,target_raw,heading,block,alias,\
             dest_path,dest_root,dest_root_path,span_start,span_end,node_rev) \
             VALUES ('claim.md',0,'wikilink','t',NULL,NULL,NULL,\
             {dest_path},{dest_root},{dest_root_path},0,1,'r');"
        ))
    };

    assert!(
        insert("NULL", "'sessions'", "NULL").is_err(),
        "a root without its path names nothing",
    );
    assert!(
        insert("NULL", "NULL", "'notes.md'").is_err(),
        "a path inside no root names nothing",
    );
    assert!(
        insert("'claim.md'", "'sessions'", "'notes.md'").is_err(),
        "one destination, never two",
    );
    // The positive control: a well-formed cross-root row IS accepted, or the
    // three refusals above prove only that the table rejects everything.
    assert!(
        insert("NULL", "'sessions'", "'notes.md'").is_ok(),
        "a well-formed cross-root destination is accepted",
    );
}
