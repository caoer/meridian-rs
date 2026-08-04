//! U5.1 acceptance gate — the board view (colors), wired over the U2.9 locked
//! read face (plan U5.1; d2 §5.3).
//!
//! - `gate_locked_face_holds_with_board` — the U2.9 blocked-`ATTACH` assertion
//!   still refuses with the `board` view loaded (no face widening).
//!
//! # The other five gates retired with the legacy `^inputs` plane (R1.3)
//! `gate_doctored_verdict_board_red`, `gate_ungated_close_grey`,
//! `gate_board_one_color_per_edge`, `gate_archived_v1_pin_renders_superseded_algo_grey`,
//! `gate_form2_chain_renders_superseded_algo_grey` and
//! `gate_form2_v2_wikilink_pin_renders_green` each asserted a board arm that
//! **only a legacy row could reach** — the `node_rev` drift compare, the
//! declared-unpinned grey, and the superseded-algo grey are all fenced by
//! `verdict_color IS NULL`, and under R4 every row carries a verdict. They were
//! deleted rather than inverted into tripwires: a tripwire needs a representable
//! failure state, and R4 makes "declared without a minted pin" UNREPRESENTABLE —
//! the pin row IS the declaration. An inverted test would have passed by
//! construction, which is the defect it would have been written to prevent.
//!
//! **The laws themselves are not retired.** "An ungated close renders grey,
//! never green" holds by construction under R4: green is COMPUTED — the verdict
//! derives from recomputing the fingerprint against live content — never granted
//! by a row's existence, so no row shape earns green without the content
//! matching. Enforcement moved from board detection to schema refusal; the
//! carrier is the R4 grammar, gated by `crates/lock`'s parse refusals.
//!
//! The one-color-per-edge invariant survives and is asserted at full strength by
//! `board_pin_verdict_gates::gate_exactly_one_board_row_per_lock_row_across_both_planes`.

use std::collections::BTreeMap;

use duckdb::Connection;
use model::Document;
use view::read_face::open_board;

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn doc(raw: &str) -> Document {
    model::build(raw.to_string(), syntax::parse(raw))
}

fn scalar_i64(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get::<_, i64>(0))
        .expect("scalar query")
}

/// An effect page pinning `subject.md`'s body — a real R4 row, so the board has
/// something to render and the `count(*) == 1` below is not vacuous.
fn pinned_page() -> String {
    let mut l = lock::Lock::new();
    l.upsert_pin(lock::PinEntry::new(
        "subject",
        "9ae3f1deadbeef",
        lock::Selector::Path(Vec::new()),
        &format!("fp1.span2.b3.{}", "0".repeat(64)),
    ));
    format!("# Effect\n\ndraws from the subject\n\n{}\n", lock::render(&l))
}

// ---------------------------------------------------------------------------
// gate — the locked face is not widened by the board views
// ---------------------------------------------------------------------------

/// The board views are read-only SQL over the SAME locked read face: they read
/// fine, but `ATTACH`/`COPY`/external access STILL refuse (no face widening,
/// A10; §2.1). This is the U2.9 blocked-ATTACH guard re-asserted with the U5.1
/// views loaded.
#[test]
fn gate_locked_face_holds_with_board() {
    let mut docs = BTreeMap::new();
    docs.insert("subject.md".to_string(), doc("# Subject\n\nbody. ^claim\n"));
    docs.insert("effect.md".to_string(), doc(&pinned_page()));
    let conn = open_board(&docs).expect("open board");

    // The board views READ fine over the locked face — and there IS a row, so a
    // silently empty board could not pass this in place of a working one.
    assert_eq!(scalar_i64(&conn, "SELECT count(*) FROM board"), 1);

    // ATTACH still refuses — the face is not widened by the new views.
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
    // Re-raising the capability is still refused (configuration locked).
    assert!(
        conn.execute_batch("SET enable_external_access=true")
            .is_err(),
        "re-enabling external access must still refuse (configuration locked)",
    );
}
