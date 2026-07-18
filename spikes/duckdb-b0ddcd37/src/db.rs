//! Schema DDL, bulk insert (Appender), root-stamp helpers.

use crate::extract::Facts;
use duckdb::types::{TimeUnit, Value};
use duckdb::{params, Connection, Result};
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// PROJECTION = the rebuildable current-state tables. Dropped and recreated
/// wholesale on a stale-root rebuild (geography escalation clause).
pub const SCHEMA_PROJECTION: &str = r#"
CREATE TABLE files (
    path      TEXT PRIMARY KEY,
    norm_key  TEXT,
    file_rev  BIGINT,
    bytes     BIGINT
);
CREATE TABLE nodes (
    path       TEXT,
    kind       TEXT,
    span_start BIGINT,
    span_end   BIGINT,
    hpath      TEXT,
    node_rev   BIGINT
);
CREATE TABLE links (
    src_path TEXT,
    target   TEXT,
    resolved BOOLEAN
);
CREATE TABLE frontmatter (
    path  TEXT,
    key   TEXT,
    value TEXT
);
"#;

/// STAMP = root-stamp audit trail. Persists across rebuilds (it records WHICH
/// root each build/batch was projected from — the audit line, not derived data).
pub const SCHEMA_STAMP: &str = r#"
CREATE TABLE roots (
    root     TEXT,
    built_at TIMESTAMP
);
-- host position: every write batch records (root_before, root_after)
CREATE TABLE batches (
    batch_seq   BIGINT,
    root_before TEXT,
    root_after  TEXT,
    ts          TIMESTAMP,
    n_rows      BIGINT
);
"#;

/// CHANGES = append-only telemetry under the journal law. SURVIVES projection
/// rebuilds — an append log is not derived state (never authoritative, R5).
pub const SCHEMA_CHANGES: &str = r#"
CREATE TABLE changes (
    seq     BIGINT,
    ts      TIMESTAMP,
    path    TEXT,
    kind    TEXT,
    change  TEXT,
    old_rev BIGINT,
    new_rev BIGINT
);
"#;

/// Index on the projection (recreated after a rebuild).
pub const INDEX_PROJECTION: &str = "CREATE INDEX idx_links_target ON links(target);";
/// Index on the surviving changes journal (created once).
pub const INDEX_CHANGES: &str = "CREATE INDEX idx_changes_path ON changes(path);";

pub fn now_micros() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_micros() as i64
}

fn ts(micros: i64) -> Value {
    Value::Timestamp(TimeUnit::Microsecond, micros)
}

pub fn create_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA_PROJECTION)?;
    conn.execute_batch(SCHEMA_STAMP)?;
    conn.execute_batch(SCHEMA_CHANGES)?;
    Ok(())
}

/// Recreate only the PROJECTION tables — used on a stale-root rebuild while the
/// STAMP audit trail and CHANGES journal are left in place.
pub fn create_schema_projection(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA_PROJECTION)?;
    Ok(())
}

pub fn create_indexes(conn: &Connection) -> Result<()> {
    conn.execute_batch(INDEX_PROJECTION)?;
    conn.execute_batch(INDEX_CHANGES)?;
    Ok(())
}

pub fn create_index_projection(conn: &Connection) -> Result<()> {
    conn.execute_batch(INDEX_PROJECTION)?;
    Ok(())
}

/// Order-dependent fold over sorted files → hex root. Files must already be
/// sorted by path (extract_corpus sorts). Spike hash, not the production Merkle.
pub fn compute_root(facts: &Facts) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for f in &facts.files {
        f.path.hash(&mut h);
        f.file_rev.hash(&mut h);
    }
    format!("{:016x}", h.finish())
}

/// Bulk-insert CURRENT (rebuildable) tables via Appender. Returns DB wall time
/// ONLY (extraction already done).
pub fn insert_current(conn: &Connection, facts: &Facts) -> Result<Duration> {
    let t = Instant::now();
    {
        let mut a = conn.appender("files")?;
        for f in &facts.files {
            a.append_row(params![f.path, f.norm_key, f.file_rev, f.bytes])?;
        }
        a.flush()?;
    }
    {
        let mut a = conn.appender("nodes")?;
        for n in &facts.nodes {
            a.append_row(params![
                n.path,
                n.kind,
                n.span_start,
                n.span_end,
                n.hpath,
                n.node_rev
            ])?;
        }
        a.flush()?;
    }
    {
        let mut a = conn.appender("links")?;
        for l in &facts.links {
            a.append_row(params![l.src_path, l.target, l.resolved])?;
        }
        a.flush()?;
    }
    {
        let mut a = conn.appender("frontmatter")?;
        for f in &facts.frontmatter {
            a.append_row(params![f.path, f.key, f.value])?;
        }
        a.flush()?;
    }
    Ok(t.elapsed())
}

/// Append one 'created' change row per file into the CHANGES journal.
pub fn insert_changes_created(conn: &Connection, facts: &Facts) -> Result<Duration> {
    let t = Instant::now();
    let now = now_micros();
    let mut a = conn.appender("changes")?;
    let mut seq: i64 = 1;
    for f in &facts.files {
        a.append_row(params![
            seq,
            ts(now),
            f.path,
            "file",
            "created",
            Option::<i64>::None,
            f.file_rev
        ])?;
        seq += 1;
    }
    a.flush()?;
    Ok(t.elapsed())
}

/// Full-build insert: CURRENT tables + one 'created' change per file.
pub fn insert_facts(conn: &Connection, facts: &Facts) -> Result<Duration> {
    let d1 = insert_current(conn, facts)?;
    let d2 = insert_changes_created(conn, facts)?;
    Ok(d1 + d2)
}

/// Next batch sequence number (1-based).
pub fn next_batch_seq(conn: &Connection) -> Result<i64> {
    conn.query_row("SELECT coalesce(max(batch_seq),0)+1 FROM batches", [], |r| r.get(0))
}

/// Record a stamp batch: roots gets `root_after`; batches gets the full edge.
pub fn stamp_batch(
    conn: &Connection,
    batch_seq: i64,
    root_before: Option<&str>,
    root_after: &str,
    n_rows: i64,
) -> Result<()> {
    let now = now_micros();
    conn.execute("INSERT INTO roots VALUES (?, ?)", params![root_after, ts(now)])?;
    conn.execute(
        "INSERT INTO batches VALUES (?, ?, ?, ?, ?)",
        params![batch_seq, root_before, root_after, ts(now), n_rows],
    )?;
    Ok(())
}

/// Stamp a completed full build: record the root and a batch (NULL→root).
pub fn stamp_build(conn: &Connection, root: &str, n_rows: i64) -> Result<()> {
    let seq = next_batch_seq(conn)?;
    stamp_batch(conn, seq, None, root, n_rows)
}

/// Latest stamped root (the `root_after` of the most recent batch), if any.
pub fn latest_root(conn: &Connection) -> Result<Option<String>> {
    let mut stmt = conn.prepare(
        "SELECT root_after FROM batches ORDER BY batch_seq DESC LIMIT 1",
    )?;
    let mut rows = stmt.query([])?;
    if let Some(r) = rows.next()? {
        Ok(Some(r.get::<_, String>(0)?))
    } else {
        Ok(None)
    }
}

pub fn count(conn: &Connection, table: &str) -> Result<i64> {
    conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
}
