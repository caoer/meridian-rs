//! **U21 Q7(B) — the cross-root link row, and the two-place clause that decides
//! whether a working cross-vault link reads as broken.**
//!
//! The `dangling` view is `dest_path IS NULL AND dest_root IS NULL`. The second
//! clause is the whole point: a resolved cross-root edge has `dest_path = NULL`
//! BY CONSTRUCTION — its target is not a path in this corpus, and `dest_path`
//! carries an enforced foreign key into `doc` — so without that clause every
//! working cross-vault link is reported broken.
//!
//! The Leader's condition 2: that clause gets a RED TEST rather than a comment.

use std::collections::BTreeMap;

use model::Document;

fn doc(raw: &str) -> Document {
    model::build(raw.to_string(), syntax::parse(raw))
}

/// The ambient workspace, the mounted root's corpus, and the mount table.
fn fixture() -> (BTreeMap<String, Document>, BTreeMap<String, Document>) {
    let mut ambient = BTreeMap::new();
    ambient.insert(
        "claim.md".to_owned(),
        doc("# Claim\n\n[[sessions:notes.md]]\n[[local.md]]\n[[nowhere.md]]\n"),
    );
    ambient.insert("local.md".to_owned(), doc("# Local\n"));
    // THE DECOY: the ambient corpus holds its own `notes.md`. A cross-root edge
    // that resolved onto THIS file is FINDING 03's wrong-bytes success.
    ambient.insert(
        "notes.md".to_owned(),
        doc("# Ambient notes — the wrong one\n"),
    );

    let mut sessions = BTreeMap::new();
    sessions.insert("notes.md".to_owned(), doc("# Notes — the sessions root\n"));
    (ambient, sessions)
}

/// The corpus fold an ephemeral build is stamped with — in production the
/// caller's domain fold (`fs::domain_snapshot`); these fixtures declare no
/// domain, so version 0 is that domain.
fn fold(docs: &BTreeMap<String, Document>) -> String {
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

/// **A resolved cross-root edge is NOT dangling** — the clause under test — and
/// it lands in its own two columns rather than in `dest_path`.
#[test]
fn a_resolved_cross_root_edge_is_not_dangling_and_keeps_dest_path_free() {
    let (ambient, sessions) = fixture();
    let root = addr::MountName::parse("sessions").expect("a name");
    let corpus = model::RootedCorpus::ambient(&ambient).with_root(
        root.clone(),
        model::RootKind::Vault,
        &sessions,
    );
    let mounts = addr::MountSet::new([root]);
    let conn =
        view::build_memory_rooted(&ambient, &corpus, &mounts, &fold(&ambient)).expect("view");

    // The cross-root edge resolved, into its own columns.
    assert_eq!(
        one_text(
            &conn,
            "SELECT dest_root || '|' || dest_root_path FROM link \
             WHERE target_raw = 'sessions:notes.md'"
        ),
        vec!["sessions|notes.md"],
        "the cross-root destination is two columns, joined only for this assertion",
    );
    // And NOT into dest_path — which would have pointed at the ambient decoy.
    assert_eq!(
        one_text(
            &conn,
            "SELECT coalesce(dest_path,'NULL') FROM link WHERE target_raw = 'sessions:notes.md'"
        ),
        vec!["NULL"],
        "dest_path means 'a path in THIS corpus', always — never only sometimes",
    );

    // **THE CLAUSE UNDER TEST.** Delete `AND dest_root IS NULL` from the
    // `dangling` view and this row appears — a working cross-vault link
    // reported broken.
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
