//! The middleware door's `ctx.sql` backend (armed-plane Part A2, § The ctx
//! surface): one `:memory:` projection per overlay world the write door
//! hands in, answering every SELECT that reads that world, rows back as
//! `policy::SqlValue`s.
//!
//! Lives HERE because of the C2 topology law: `view` is a write-only leaf
//! `wire-serve` may not depend on, so the projection half is installed into
//! the door at process startup ([`install`]) by the hosts that already link
//! `view` — the resident daemon and the `mrd` CLI (which depends on this
//! crate). Snapshot-scoped by construction: the projection is built from the
//! given docs and dropped with the door run's world; no later writer is
//! visible.

use policy::{SqlRow, SqlValue};
use view::duckdb::Connection;
use wire_serve::middleware::SqlProjection;

/// Install the backend into the write door. Idempotent; call at startup.
pub fn install() {
    wire_serve::middleware::install_sql_backend(build);
}

/// One overlay world's projection: the built database, the name it is the
/// default database under, and the instance as a connection with no session
/// state sees it at build ([`instance`]).
struct Projection {
    conn: Connection,
    database: String,
    instance: Vec<String>,
    /// An earlier call left the database changed past its rollback.
    spent: bool,
}

/// Project `docs` once; the door keeps the answer for every `ctx.sql` that
/// reads the same world.
fn build(docs: &model::Docs) -> Result<Box<dyn SqlProjection>, String> {
    let projecting = timing::phase("mw.sql.project");
    let conn = view::build_memory(docs, "middleware-overlay").map_err(|e| e.to_string())?;
    // The same extension door as the two sql lanes (card
    // sql-extension-ddl-escapes-rollback-lane): a caller-SQL door that admits
    // third-party extension code while the others refuse it would make the
    // DOOR, not the statement, decide what the contract means. Three doors,
    // one law.
    view::store::apply_extension_gate(&conn).map_err(|e| e.to_string())?;
    // Every lane's spill budget (card sql-spill-config-lockout), at the
    // absolute home `mrd sql`'s `:memory:` lane uses: no drawer here either.
    view::store::apply_spill_containment(&conn, &std::env::temp_dir().join("mrd-sql-spill"))
        .map_err(|e| e.to_string())?;
    let database = conn
        .query_row("SELECT current_database()", [], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|e| e.to_string())?;
    let instance = instance(&conn)?;
    projecting.stop();
    Ok(Box::new(Projection {
        conn,
        database,
        instance,
        spent: false,
    }))
}

impl SqlProjection for Projection {
    /// One `ctx.sql` call on a fresh connection, so no session state of an
    /// earlier call is in force, inside `BEGIN → statement → ROLLBACK`, so its
    /// DML and DDL end with it.
    ///
    /// What a rollback cannot undo leaves the call's answer standing and
    /// marks the projection [`spent`](SqlProjection::spent):
    ///
    /// - text that ended the call's transaction itself — `prepare` runs every
    ///   statement before the last, so `COMMIT; DELETE …; BEGIN; SELECT 1`
    ///   commits the `DELETE`. The projection database's transaction id, read
    ///   before and after the text, tells.
    /// - a moved instance setting (`SET GLOBAL`, an extension's settings): the
    ///   settings a fresh connection sees differ from the build's.
    fn query(&mut self, query: &str) -> Result<Vec<SqlRow>, String> {
        let querying = timing::phase("mw.sql.query");
        let conn = self.conn.try_clone().map_err(|e| e.to_string())?;
        conn.execute_batch("BEGIN").map_err(|e| e.to_string())?;
        let opened = txid(&conn, &self.database);
        let rows = select(&conn, query)?;
        let held = opened.is_some() && txid(&conn, &self.database) == opened;
        let rolled_back = conn.execute_batch("ROLLBACK").is_ok();
        drop(conn);
        let unmoved = instance(&self.conn).is_ok_and(|now| now == self.instance);
        if !(held && rolled_back && unmoved) {
            self.spent = true;
        }
        querying.stop();
        Ok(rows)
    }

    fn spent(&self) -> bool {
        self.spent
    }
}

/// The projection database's open transaction id (`txid_current()`, its
/// start timestamp), read with that database as the default again — a `USE`
/// in the caller's text moves the default for this connection.
fn txid(conn: &Connection, database: &str) -> Option<u64> {
    conn.execute_batch(&format!("USE \"{database}\"")).ok()?;
    conn.query_row("SELECT txid_current()", [], |row| row.get::<_, u64>(0))
        .ok()
}

/// The instance state no transaction holds, as a new connection with no
/// session state sees it: every setting's value, whatever its scope label.
fn instance(conn: &Connection) -> Result<Vec<String>, String> {
    let probe = conn.try_clone().map_err(|e| e.to_string())?;
    let mut stmt = probe
        .prepare(
            "SELECT name || ' = ' || coalesce(value::VARCHAR, 'NULL') \
             FROM duckdb_settings() ORDER BY name",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<String>, _>>()
        .map_err(|e| e.to_string())
}

/// Run one statement and convert its rows.
fn select(conn: &Connection, query: &str) -> Result<Vec<SqlRow>, String> {
    let mut stmt = conn.prepare(query).map_err(|e| e.to_string())?;
    let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
    let names: Vec<String> = {
        let stmt_ref = rows
            .as_ref()
            .ok_or_else(|| "no result statement".to_owned())?;
        (0..stmt_ref.column_count())
            .map(|i| {
                stmt_ref
                    .column_name(i)
                    .map_or_else(|_| format!("col{i}"), String::clone)
            })
            .collect()
    };
    let mut out = Vec::new();
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let mut cells = Vec::with_capacity(names.len());
        for (i, name) in names.iter().enumerate() {
            let value = row.get_ref(i).map_or(SqlValue::Null, cell);
            cells.push((name.clone(), value));
        }
        out.push(cells);
    }
    Ok(out)
}

/// One cell to the closed [`SqlValue`] scalar surface. Anything without a
/// scalar arm (blobs, intervals, nested types) renders as text — rows are
/// facts for a predicate, not a serialization format.
fn cell(v: view::duckdb::types::ValueRef<'_>) -> SqlValue {
    use view::duckdb::types::ValueRef;
    match v {
        ValueRef::Null => SqlValue::Null,
        ValueRef::Boolean(b) => SqlValue::Bool(b),
        ValueRef::TinyInt(i) => SqlValue::Int(i64::from(i)),
        ValueRef::SmallInt(i) => SqlValue::Int(i64::from(i)),
        ValueRef::Int(i) => SqlValue::Int(i64::from(i)),
        ValueRef::BigInt(i) => SqlValue::Int(i),
        ValueRef::UTinyInt(i) => SqlValue::Int(i64::from(i)),
        ValueRef::USmallInt(i) => SqlValue::Int(i64::from(i)),
        ValueRef::UInt(i) => SqlValue::Int(i64::from(i)),
        ValueRef::UBigInt(i) => {
            i64::try_from(i).map_or(SqlValue::Text(i.to_string()), SqlValue::Int)
        }
        ValueRef::Float(f) => SqlValue::Float(f64::from(f)),
        ValueRef::Double(f) => SqlValue::Float(f),
        ValueRef::Text(bytes) => SqlValue::Text(String::from_utf8_lossy(bytes).into_owned()),
        other => SqlValue::Text(format!("{other:?}")),
    }
}

#[cfg(test)]
mod tests {
    //! One projection serves every `ctx.sql` of a door run, so a call must
    //! leave it as it found it or say it did not (armed-plane § The ctx
    //! surface).

    use std::sync::Arc;

    use policy::SqlValue;
    use wire_serve::middleware::SqlProjection;

    fn projection() -> Box<dyn SqlProjection> {
        let docs: model::Docs = [
            ("a.md", "---\nstatus: open\n---\n# A\n"),
            ("b.md", "---\nstatus: done\n---\n# B\n"),
        ]
        .into_iter()
        .map(|(path, raw)| {
            let doc = model::candidate_of_body(path, raw.to_owned()).into_document();
            (path.to_owned(), Arc::new(doc))
        })
        .collect();
        super::build(&docs).expect("the projection builds")
    }

    /// The first column of one answer, rendered, in row order.
    fn column(p: &mut dyn SqlProjection, query: &str) -> Vec<String> {
        p.query(query)
            .expect("the query answers")
            .into_iter()
            .map(|row| match &row[0].1 {
                SqlValue::Text(text) => text.clone(),
                other => format!("{other:?}"),
            })
            .collect()
    }

    #[test]
    fn what_a_rollback_undoes_keeps_the_projection() {
        let mut p = projection();
        let open = "SELECT path FROM frontmatter WHERE key = 'status' AND value = 'open'";
        assert_eq!(column(&mut *p, open), ["a.md"]);
        assert!(!p.spent(), "a SELECT leaves the projection as it found it");

        p.query("DELETE FROM frontmatter")
            .expect("DML runs in its call");
        assert_eq!(column(&mut *p, open), ["a.md"], "DML ends with its call");

        p.query("CREATE TABLE scratch AS SELECT 1 AS x")
            .expect("DDL runs in its call");
        assert!(
            p.query("SELECT x FROM scratch").is_err(),
            "DDL ends with its call"
        );

        p.query("SET integer_division = true; SELECT 1 AS x")
            .expect("a session SET runs in its call");
        assert_eq!(
            column(&mut *p, "SELECT 1 / 2 AS h"),
            ["Float(0.5)"],
            "a session SET ends with its call"
        );
        assert!(!p.spent(), "nothing outlived its call");
    }

    #[test]
    fn what_a_rollback_cannot_undo_spends_the_projection() {
        for text in [
            "SET GLOBAL integer_division = true; SELECT 1 / 2 AS h",
            "SET default_order = 'DESC'; SELECT 1 / 2 AS h",
            "COMMIT; DELETE FROM frontmatter; BEGIN; SELECT 1 / 2 AS h",
        ] {
            let mut p = projection();
            p.query(text).expect("the call still answers");
            assert!(p.spent(), "`{text}` spends the projection");
        }
    }
}
