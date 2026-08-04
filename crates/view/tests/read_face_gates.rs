//! U2.9 acceptance gates — C1 projections + locked read face (plan U2.9 Test:
//! line). Three load-bearing gates, each named:
//!
//! - `gate_blocked_attach` — `ATTACH`/`COPY`/external access from a locked read
//!   face refuses; a positive control proves the LOCK is what refuses (the same
//!   `ATTACH` succeeds on an unlocked connection).
//! - `gate_doctored_verdict` — a doctored-verdict fixture renders red via
//!   default-face SQL only (`board_red`), no optional pack.
//! - `gate_stale_projection` — the projection is keyed on `doc_rev`: a rev change
//!   is detected as stale and recomputes, flipping the board color.

use std::collections::BTreeMap;

use duckdb::Connection;
use model::Document;
use view::{lock_read_face, open_board, stale_paths};

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn doc(raw: &str) -> Document {
    model::build(raw.to_string(), syntax::parse(raw))
}

fn scalar_i64(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).expect("scalar query")
}

/// Whether a `DuckDB` error is the `enable_external_access=false` refusal.
/// v1.5.2 phrases it "file system operations are disabled by configuration"
/// (a Permission Error); accept the setting name too for version drift.
fn is_external_access_refusal(msg: &str) -> bool {
    let m = msg.to_lowercase();
    m.contains("disabled by configuration")
        || m.contains("external access")
        || m.contains("enable_external_access")
        || m.contains("permission error")
}

// ---------------------------------------------------------------------------
// Gate — blocked-ATTACH assertion (load-bearing negative)
// ---------------------------------------------------------------------------

/// The locked read face refuses `ATTACH`, `COPY`, and external file reads — an
/// SQL view pack physically has no write path (A10; §2.1). The positive control
/// (an UNLOCKED connection attaches fine) proves the lock is load-bearing: the
/// refusal comes from `lock_read_face`, not from a universally broken `ATTACH`.
#[test]
fn gate_blocked_attach() {
    let tmp = tempfile::tempdir().unwrap();
    let sidecar = tmp.path().join("sidecar.duckdb");
    let attach = format!("ATTACH '{}' AS x", sidecar.display());

    // Positive control: WITHOUT the lock, ATTACH to a file succeeds. This is
    // what makes the negative below meaningful — remove the lock and the gate
    // fails (verified by hand: commenting out lock_read_face in open_board makes
    // every assertion below fail because ATTACH/COPY/read_csv all succeed).
    let control = Connection::open_in_memory().unwrap();
    control
        .execute_batch(&attach)
        .expect("an unlocked connection CAN attach — the lock is what blocks it");

    // The real read face: build + lock, then every external/write path refuses.
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

    // lock_configuration=true: untrusted SQL cannot re-raise the capability.
    let reraise_err = conn
        .execute_batch("SET enable_external_access=true")
        .unwrap_err()
        .to_string();
    assert!(
        reraise_err.to_lowercase().contains("lock")
            || reraise_err.to_lowercase().contains("cannot"),
        "re-enabling external access must refuse (configuration locked), got: {reraise_err}"
    );

    // A plain read still works — the face is read-only, not dead.
    assert_eq!(scalar_i64(&conn, "SELECT count(*) FROM node"), 0);
}

/// `lock_read_face` in isolation is a load-bearing capability primitive: an
/// external file read that SUCCEEDS before the lock refuses after it (positive
/// control), independent of the board projection.
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

// ---------------------------------------------------------------------------
// Gate — doctored-verdict: RETIRED HERE, carried by the fingerprint plane
// ---------------------------------------------------------------------------
//
// `gate_doctored_verdict` asserted "verdicts freeze at close" through
// `board_red` / `board_drift` — the legacy `node_rev` compare, fenced by
// `verdict_color IS NULL`. R1.3 retired the `^inputs` plane that was the only
// way to reach those arms, so the test could no longer fail and was deleted
// rather than left passing by construction.
//
// **The law is not retired, and it is still asserted.** A doctored verdict
// renders red through the FINGERPRINT compare instead:
// `board_pin_verdict_gates::gate_board_renders_the_same_verdict_the_walk_renders`
// pins `drifted.md -> red content-drifted` and `dangling.md -> red
// dangling-anchor` on the same `board` view, with a green control beside them.
// Same claim, surviving carrier, still falsified.

// ---------------------------------------------------------------------------
// Gate — stale projection recomputes on rev change
// ---------------------------------------------------------------------------

/// The projection is keyed on `doc_rev`: every row records the rev it was built
/// at, and a rev change is detected as stale so a recompute happens. The cache
/// obeys the staleness law it serves — no stored second truth (§2.1, §8).
#[test]
fn gate_stale_projection() {
    let subject_v1 = "# Subject\n\nThe reviewed claim body. ^claim\n";
    let subject_v2 = "# Subject\n\nThe body drifted. ^claim\n";

    let mut docs_v1 = BTreeMap::new();
    docs_v1.insert("subject.md".to_string(), doc(subject_v1));
    let conn = open_board(&docs_v1).expect("open board v1");

    // Fresh at v1: nothing is stale relative to the docs it was built from.
    assert!(
        stale_paths(&conn, &docs_v1)
            .expect("stale check")
            .is_empty(),
        "a projection is not stale against the docs it was built from"
    );

    // The subject changes to v2 (a new doc_rev). The v1 projection is now STALE
    // for subject.md — the recorded doc_rev no longer matches the live one.
    let mut docs_v2 = BTreeMap::new();
    docs_v2.insert("subject.md".to_string(), doc(subject_v2));
    let stale = stale_paths(&conn, &docs_v2).expect("stale check");
    assert_eq!(
        stale,
        vec!["subject.md".to_string()],
        "a rev change is detected as stale (rev-compare invalidation)"
    );

    // Recompute: rebuild over v2 and the projection carries the new doc_rev.
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
