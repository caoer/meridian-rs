//! Board view over locked read face (d2 §5.3).
//!
//! Surviving gate: locked face holds with `board` loaded (no face widening).
//! Legacy-only arms retired with `^inputs`. Laws remain by construction under
//! R4 (green is computed, never granted by row existence). One-color-per-edge
//! lives in `board_pin_verdict_gates`.

use std::collections::BTreeMap;

use duckdb::Connection;
use model::Document;
use view::read_face::open_board;

fn doc(raw: &str) -> Document {
    model::build(raw.to_string(), syntax::parse(raw))
}

fn scalar_i64(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get::<_, i64>(0))
        .expect("scalar query")
}

/// Real R4 pin so board `count(*) == 1` is not vacuous.
fn pinned_page() -> String {
    let mut l = lock::Lock::new();
    l.upsert_pin(lock::PinEntry::new(
        "subject",
        "9ae3f1deadbeef",
        lock::Selector::Path(Vec::new()),
        &format!("fp1.span2.b3.{}", "0".repeat(64)),
    ));
    format!(
        "# Effect\n\ndraws from the subject\n\n{}\n",
        lock::render(&l)
    )
}

/// Board views over locked face: reads work; ATTACH still refuses (A10).
#[test]
fn gate_locked_face_holds_with_board() {
    let mut docs = BTreeMap::new();
    docs.insert("subject.md".to_string(), doc("# Subject\n\nbody. ^claim\n"));
    docs.insert("effect.md".to_string(), doc(&pinned_page()));
    let conn = open_board(&docs).expect("open board");

    assert_eq!(scalar_i64(&conn, "SELECT count(*) FROM board"), 1);

    let tmp = tempfile::tempdir().unwrap();
    let attach = format!("ATTACH '{}' AS x", tmp.path().join("side.duckdb").display());
    let err = conn.execute_batch(&attach).unwrap_err().to_string();
    let m = err.to_lowercase();
    assert!(
        m.contains("disabled by configuration")
            || m.contains("external access")
            || m.contains("enable_external_access")
            || m.contains("permission error"),
        "ATTACH must still refuse with the board view loaded, got: {err}",
    );
    assert!(
        conn.execute_batch("SET enable_external_access=true")
            .is_err(),
        "re-enabling external access must still refuse (configuration locked)",
    );
}
