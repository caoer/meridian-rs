//! The middleware door's `ctx.sql` backend (armed-plane Part A2): one
//! ephemeral `:memory:` projection per call over the overlay world the write
//! door hands in, one SELECT, rows back as `policy::SqlValue`s.
//!
//! Lives HERE because of the C2 topology law: `view` is a write-only leaf
//! `wire-serve` may not depend on, so the projection half is installed into
//! the door at process startup ([`install`]) by the hosts that already link
//! `view` — the resident daemon and the `mrd` CLI (which depends on this
//! crate). Snapshot-scoped by construction: the projection is built from the
//! given docs and discarded; no later writer is visible.

use policy::{SqlRow, SqlValue};

/// Install the backend into the write door. Idempotent; call at startup.
pub fn install() {
    wire_serve::middleware::install_sql_backend(backend);
}

/// One `ctx.sql` call: project `docs`, run `query`, convert rows.
fn backend(docs: &model::Docs, query: &str) -> Result<Vec<SqlRow>, String> {
    let conn = view::build_memory(docs, "middleware-overlay").map_err(|e| e.to_string())?;
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
