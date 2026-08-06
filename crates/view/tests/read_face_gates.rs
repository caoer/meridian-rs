//! C1 projections + locked read face gates:
//! - `gate_blocked_attach` / `gate_lock_read_face_primitive` — lock capability
//! - `gate_stale_projection` — `doc_rev` staleness triggers rebuild

use std::collections::BTreeMap;

use duckdb::Connection;
use model::Document;
use view::{lock_read_face, open_board, stale_paths};

fn doc(raw: &str) -> Document {
    model::build(raw.to_string(), syntax::parse(raw))
}

fn scalar_i64(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).expect("scalar query")
}

/// `DuckDB` `enable_external_access=false` refusal (phrase may vary by version).
fn is_external_access_refusal(msg: &str) -> bool {
    let m = msg.to_lowercase();
    m.contains("disabled by configuration")
        || m.contains("external access")
        || m.contains("enable_external_access")
        || m.contains("permission error")
}

/// Locked face refuses ATTACH/COPY/external; unlocked control proves the lock.
#[test]
fn gate_blocked_attach() {
    let tmp = tempfile::tempdir().unwrap();
    let sidecar = tmp.path().join("sidecar.duckdb");
    let attach = format!("ATTACH '{}' AS x", sidecar.display());

    // Positive control: without lock, ATTACH succeeds.
    let control = Connection::open_in_memory().unwrap();
    control
        .execute_batch(&attach)
        .expect("an unlocked connection CAN attach — the lock is what blocks it");

    let docs = BTreeMap::new();
    let conn = open_board(&docs).expect("open board");

    let attach_err = conn.execute_batch(&attach).unwrap_err().to_string();
    assert!(
        is_external_access_refusal(&attach_err),
        "ATTACH from a locked read face must refuse (external access disabled), got: {attach_err}"
    );

    let copy_err = conn
        .execute_batch(&format!(
            "COPY (SELECT 1 AS x) TO '{}'",
            tmp.path().join("leak.csv").display()
        ))
        .unwrap_err()
        .to_string();
    assert!(
        is_external_access_refusal(&copy_err),
        "COPY TO a file must refuse (no write path), got: {copy_err}"
    );

    let read_err = conn
        .execute_batch("SELECT * FROM read_csv('/etc/hosts')")
        .unwrap_err()
        .to_string();
    assert!(
        is_external_access_refusal(&read_err),
        "external file read must refuse, got: {read_err}"
    );

    // lock_configuration=true: cannot re-raise.
    let reraise_err = conn
        .execute_batch("SET enable_external_access=true")
        .unwrap_err()
        .to_string();
    assert!(
        reraise_err.to_lowercase().contains("lock")
            || reraise_err.to_lowercase().contains("cannot"),
        "re-enabling external access must refuse (configuration locked), got: {reraise_err}"
    );

    // Face is read-only, not dead.
    assert_eq!(scalar_i64(&conn, "SELECT count(*) FROM node"), 0);
}

/// `lock_read_face` alone: external read succeeds before lock, refuses after.
#[test]
fn gate_lock_read_face_primitive() {
    let tmp = tempfile::tempdir().unwrap();
    let csv = tmp.path().join("data.csv");
    std::fs::write(&csv, "a\n1\n").unwrap();
    let read = format!("SELECT * FROM read_csv('{}')", csv.display());

    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(&read)
        .expect("before the lock, an external file read succeeds");

    lock_read_face(&conn).expect("lock");
    assert!(
        conn.execute_batch(&read).is_err(),
        "after lock_read_face, the same external read refuses"
    );
}

/// Projection keyed on `doc_rev` — rev change is stale; rebuild refreshes (§2.1/§8).
#[test]
fn gate_stale_projection() {
    let subject_v1 = "# Subject\n\nThe reviewed claim body. ^claim\n";
    let subject_v2 = "# Subject\n\nThe body drifted. ^claim\n";

    let mut docs_v1 = BTreeMap::new();
    docs_v1.insert("subject.md".to_string(), doc(subject_v1));
    let conn = open_board(&docs_v1).expect("open board v1");

    assert!(
        stale_paths(&conn, &docs_v1)
            .expect("stale check")
            .is_empty(),
        "a projection is not stale against the docs it was built from"
    );

    let mut docs_v2 = BTreeMap::new();
    docs_v2.insert("subject.md".to_string(), doc(subject_v2));
    let stale = stale_paths(&conn, &docs_v2).expect("stale check");
    assert_eq!(
        stale,
        vec!["subject.md".to_string()],
        "a rev change is detected as stale (rev-compare invalidation)"
    );

    let conn2 = open_board(&docs_v2).expect("rebuild v2");
    assert!(
        stale_paths(&conn2, &docs_v2)
            .expect("stale check")
            .is_empty(),
        "the recomputed projection is fresh against v2"
    );
    let rev_v1 = docs_v1["subject.md"].root.node_rev.0.clone();
    let rev_v2 = docs_v2["subject.md"].root.node_rev.0.clone();
    assert_ne!(rev_v1, rev_v2, "the edit changed the doc_rev");
    let recorded: String = conn2
        .query_row(
            "SELECT DISTINCT doc_rev FROM node WHERE path='subject.md'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        recorded, rev_v2,
        "the recomputed node rows record the new doc_rev, not the stale one"
    );

    // An absent path is stale too (the projection cannot answer for it).
    let mut docs_new = BTreeMap::new();
    docs_new.insert("brand-new.md".to_string(), doc("# New\n"));
    assert_eq!(
        stale_paths(&conn2, &docs_new).expect("stale check"),
        vec!["brand-new.md".to_string()],
        "a path absent from the projection is stale"
    );
}
