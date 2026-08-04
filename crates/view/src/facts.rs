//! The d2 §2.1 fact-table surface — the fact tables (`node`, `edge`, `claim`)
//! as `DuckDB` **projections over parse, never a second store**.
//!
//! # The admission test, encoded as columns
//! Every column carries its derivation-source class — the admission-test
//! classes, NOT design lineage (d2 §1.1):
//! - **1 parse** — computable from the vault's bytes alone.
//! - **2 rev-comparison** — equality of content-addressed revs (incl. repo
//!   refs of the vault's own repo).
//! - **3 engine's-own-acts** — facts the strict writer records about writes it
//!   itself performed (A5-bounded: write-mechanics only).
//!
//! Each column comment below is tagged `[1]`/`[2]`/`[3]`.
//!
//! # These are projections, not a store
//! `node` is projected from the parsed corpus. A fourth table, `receipt_journal`,
//! projected the ONE reserved journal page here until the journal was removed
//! (ZT 2026-08-02, remove-no-replacement); it is gone, not emptied — a table that
//! can never have a row is not a contract. `edge` and `claim` are defined here as
//! the contract; their
//! projections are populated by the units that own their source facts — `edge`
//! by the pin/lock manifest reader, `claim` by the run plane — and are left
//! empty here rather than half-projected (honest, stated).

use duckdb::Connection;
use duckdb::types::Value;
use model::{Document, Node, NodeKind};

/// The d2 §2.1 fact-table DDL: `node` · `edge` · `claim`.
/// A separate schema from the frozen round-1 view schema ([`crate::schema`]) —
/// additive, never an edit to what ships. Every column tags its source class.
pub const FACTS_SCHEMA_SQL: &str = r"
-- node — one row per addressable span (the selector grammar's spans: page,
-- page#Heading, page#^block-id, frontmatter). A projection of the parsed tree.
CREATE TABLE node (
    path       TEXT     NOT NULL,   -- [1] workspace-relative page path
    selector   TEXT     NOT NULL,   -- [1] addressable selector within the page ('' = the page/doc root)
    kind       TEXT     NOT NULL,   -- [1] span kind: document|section|frontmatter|anchor
    span_start UBIGINT  NOT NULL,   -- [1] byte span start
    span_end   UBIGINT  NOT NULL,   -- [1] byte span end
    node_rev   TEXT     NOT NULL,   -- [1] blake3(raw[span])[:16] — parse-computed
    doc_rev    TEXT     NOT NULL,   -- [1] containing document rev (the Document root's node_rev)
    PRIMARY KEY (path, selector, span_start)
);

-- edge — one row per pin row of a doc's `meridian-lock` block. Populated by the
-- pin/lock reader; the contract is fixed here.
--
-- R1.3 corrected the SOURCE named here, not the table: these rows came from the
-- legacy `^inputs` manifest when this comment was written. The
-- `pinned_rev NULL = declared-only (grey)` reading went with it — under R4 a pin
-- row carries a mandatory fingerprint, so `declared, never pinned` is not a
-- state a row can be in.
CREATE TABLE edge (
    src_path    TEXT     NOT NULL,  -- [1] the page whose lock declares the ref
    declared_ref TEXT    NOT NULL,  -- [1] the ref as written in the lock
    to_path     TEXT,               -- [2] pin-time resolved path (NULL = unresolved)
    to_sel      TEXT,               -- [2] pin-time resolved selector
    pinned_rev  TEXT,               -- [2] the rev recorded in the lock
    rev_class   TEXT,               -- [2] 'content' (fingerprint token) | 'object' (git object id)
    hash_algo   TEXT                -- [2] the hash algo the pin was taken under
);

-- claim — the run-plane projection. Populated by the run plane (a later unit);
-- the contract is fixed here.
CREATE TABLE claim (
    path         TEXT    NOT NULL,  -- [1] the page carrying the claim
    program_rev  TEXT,              -- [2] the pinned program rev the claim runs
    output_block TEXT,              -- [1] output-as-content block id
    declared_cap TEXT,              -- [1] the declared cap (deny-by-default)
    state        TEXT,              -- [3] engine-observed state (A4 mechanical classifier)
    last_receipt TEXT               -- [3] anchor of the last realise receipt
);

";

/// The facts-schema version. A reader whose version differs treats the facts
/// projection as ABSENT (delete-don't-migrate, mirroring [`crate::schema`]).
pub const FACTS_SCHEMA_VERSION: i32 = 1;

/// Create the fact tables against `conn`.
///
/// # Errors
/// Propagates any `DuckDB` error from executing the DDL batch.
pub fn create_facts_schema(conn: &Connection) -> duckdb::Result<()> {
    conn.execute_batch(FACTS_SCHEMA_SQL)
}

/// Build an in-memory fact-table projection over `docs`: the `node` table from
/// the parsed corpus. `edge`/`claim` exist (the contract) but are not populated
/// here (owned by later units — stated in the module docs).
///
/// # Errors
/// Propagates any `DuckDB` error from opening the database, creating the schema,
/// or inserting the projected rows.
pub fn build_facts_memory(
    docs: &std::collections::BTreeMap<String, Document>,
) -> duckdb::Result<Connection> {
    let conn = Connection::open_in_memory()?;
    create_facts_schema(&conn)?;
    project_nodes(&conn, docs)?;
    Ok(conn)
}

/// Project every document's addressable spans into `node` rows: the document
/// root, each section, the frontmatter node, and each block anchor — exactly
/// the spans the selector grammar (§2.2) addresses. Inline nodes (wikilinks,
/// tags) carry no independent selector and are not node rows.
///
/// `pub(crate)` so the U2.9 read face ([`crate::read_face`]) can project `node`
/// into the SAME connection as its lock-pin/board projections (board reds join
/// `input_lock` against `node`), never a second store.
pub(crate) fn project_nodes(
    conn: &Connection,
    docs: &std::collections::BTreeMap<String, Document>,
) -> duckdb::Result<()> {
    let mut stmt = conn.prepare(
        "INSERT INTO node (path, selector, kind, span_start, span_end, node_rev, doc_rev) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )?;
    for (path, doc) in docs {
        let doc_rev = doc.root.node_rev.0.clone();
        let mut rows = Vec::new();
        collect_nodes(&doc.root, path, &doc_rev, &mut rows);
        for row in rows {
            stmt.execute(duckdb::params_from_iter(row.iter()))?;
        }
    }
    Ok(())
}

/// Walk one node tree, emitting a `node` row for each addressable span.
fn collect_nodes(node: &Node, path: &str, doc_rev: &str, rows: &mut Vec<Vec<Value>>) {
    let entry = match &node.kind {
        NodeKind::Document { .. } => Some(("document", String::new())),
        NodeKind::Section { .. } => {
            Some(("section", node.hpath.clone().unwrap_or_default().join(">")))
        }
        NodeKind::Frontmatter { .. } => Some(("frontmatter", "frontmatter".to_string())),
        NodeKind::Anchor { name } => Some(("anchor", format!("^{name}"))),
        _ => None,
    };
    if let Some((kind, selector)) = entry {
        rows.push(vec![
            Value::Text(path.to_string()),
            Value::Text(selector),
            Value::Text(kind.to_string()),
            Value::UBigInt(u64c(node.span.start)),
            Value::UBigInt(u64c(node.span.end)),
            Value::Text(node.node_rev.0.clone()),
            Value::Text(doc_rev.to_string()),
        ]);
    }
    for child in &node.children {
        collect_nodes(child, path, doc_rev, rows);
    }
}

/// `usize` → `u64`, saturating (never truncates on a 32-bit target).
fn u64c(x: usize) -> u64 {
    u64::try_from(x).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn doc(raw: &str) -> Document {
        model::build(raw.to_string(), syntax::parse(raw))
    }

    /// The four fact tables exist as a projection (parse), and `node` carries
    /// one row per addressable span — the doc root, sections, and frontmatter.
    #[test]
    fn node_projects_addressable_spans() {
        let mut docs = BTreeMap::new();
        docs.insert(
            "notes/plan.md".to_string(),
            doc("---\ntitle: Plan\n---\n# Goals\n\ntext\n\n## Q3\n\nship\n"),
        );
        let conn = build_facts_memory(&docs).expect("facts build");

        // the fact tables exist (the contract).
        for t in ["node", "edge", "claim"] {
            let n: i64 = conn
                .query_row(&format!("SELECT count(*) FROM {t}"), [], |r| r.get(0))
                .expect("table exists");
            assert!(n >= 0);
        }

        // document + frontmatter + two sections (Goals, Q3).
        let mut stmt = conn
            .prepare("SELECT kind FROM node ORDER BY span_start")
            .unwrap();
        let kinds: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(kinds.contains(&"document".to_string()));
        assert!(kinds.contains(&"frontmatter".to_string()));
        assert_eq!(kinds.iter().filter(|k| *k == "section").count(), 2);
    }
}
