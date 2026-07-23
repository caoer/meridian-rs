//! The pin-sourced `edge` fact projection (U2.1 deferred duty). U2.1 fixed the
//! `edge` DDL as contract with population "owned by the pin/lock reader"; this is
//! that reader. It COMPOSES over U2.9's `input_lock` projection (the parse of the
//! `^inputs` lock) rather than re-parsing the lock — one parse, never a fork.
//!
//! `edge` = the manifest LEFT-joined with the lock: one row per lock item, with
//! `pinned_rev` NULL rendering grey (declared-only). U2.9 already parses each
//! page's lock into `input_lock`; the edge projection maps those rows into the
//! `edge` columns and stamps the engine's content-class `hash_algo` for pinned
//! items (a grey item carries none).
//!
//! # The `claim` table stays run-plane-owned
//! The `claim` fact table (`facts::FACTS_SCHEMA_SQL`) is the RUN-PLANE projection
//! (program rev, output block, cap, state) — populated by the run plane, a later
//! unit. Pin sources no run-plane claim: a pin's `claim:` / `check:` are lock
//! passengers that ride the `edge`/`input_lock` surface, not run-plane rows. So
//! this module populates `edge` alone and leaves `claim` to its owner (stated).

use duckdb::Connection;
use model::Document;
use view::ViewError;

use crate::lock;

/// Build a locked board over `docs` (U2.9's `open_board`: `node` +
/// `receipt_journal` + `input_lock` + board views, read-face locked) and project
/// the pin-sourced `edge` rows over its `input_lock`. The returned connection
/// answers `edge` alongside every U2.9 surface.
///
/// # Errors
/// [`ViewError`] on any board-open or projection failure.
pub fn build_edges(
    docs: &std::collections::BTreeMap<String, Document>,
) -> Result<Connection, ViewError> {
    let conn = view::read_face::open_board(docs, &[])?;
    project_edges(&conn)?;
    Ok(conn)
}

/// Project the pin-sourced `edge` rows over an already-built `input_lock` (the
/// board `conn` from [`build_edges`] / U2.9's `open_board`). One `edge` row per
/// lock item, mapping `input_lock`'s parse columns into the `edge` contract;
/// `hash_algo` is the engine content-class label ([`lock::HASH_ALGO`]) for a
/// pinned item, NULL for a grey declared-only one.
///
/// # Errors
/// Propagates any `DuckDB` error from the projecting `INSERT`.
pub fn project_edges(conn: &Connection) -> duckdb::Result<()> {
    conn.execute_batch(&format!(
        "INSERT INTO edge (src_path, declared_ref, to_path, to_sel, pinned_rev, rev_class, hash_algo) \
         SELECT src_path, declared_ref, to_path, to_sel, pinned_rev, rev_class, \
                CASE WHEN pinned_rev IS NULL THEN NULL ELSE '{algo}' END \
         FROM input_lock",
        algo = lock::HASH_ALGO
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn doc(raw: &str) -> Document {
        model::build(raw.to_string(), syntax::parse(raw))
    }

    /// A page carrying a written `^inputs` lock projects one `edge` row per item,
    /// pinned items carry the engine `hash_algo`, and a grey declared-only item
    /// projects with NULL `pinned_rev` + NULL `hash_algo`.
    #[test]
    fn edge_projects_from_the_lock_over_u2_9_input_lock() {
        let block = lock::render_block(&[
            lock::item_line(&lock::Item {
                declared_ref: "b.md#Body".into(),
                to: "b.md#Body".into(),
                rev: Some("deadbeefdeadbeef".into()),
                rev_class: Some("content".into()),
                claim: None,
                check: None,
                check_rev: None,
            }),
            lock::item_line(&lock::Item {
                declared_ref: "substrate".into(),
                to: "substrate".into(),
                rev: None,
                rev_class: None,
                claim: Some("wiki ref, declared-only".into()),
                check: None,
                check_rev: None,
            }),
        ]);
        let mut docs = BTreeMap::new();
        docs.insert("a.md".to_string(), doc(&format!("# A\n\n{block}\n")));

        let conn = build_edges(&docs).expect("board + edge projection");
        let total: i64 = conn
            .query_row("SELECT count(*) FROM edge", [], |r| r.get(0))
            .unwrap();
        assert_eq!(total, 2, "one edge row per lock item");

        let pinned: i64 = conn
            .query_row(
                "SELECT count(*) FROM edge WHERE pinned_rev IS NOT NULL AND hash_algo = 'node-rev'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(pinned, 1, "the pinned item carries the engine hash_algo");

        let grey: i64 = conn
            .query_row(
                "SELECT count(*) FROM edge WHERE pinned_rev IS NULL AND hash_algo IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(grey, 1, "the declared-only item is grey (no rev, no algo)");
    }
}
