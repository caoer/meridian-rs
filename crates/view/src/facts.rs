//! The d2 §2.1 fact-table surface — the four fact tables (`node`, `edge`,
//! `claim`, `receipt_journal`) as `DuckDB` **projections over parse, never a
//! second store**.
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
//! `node` is projected from the parsed corpus; `receipt_journal` is projected
//! from the ONE reserved journal page (parsed upstream by
//! `receipt::journal::parse_rows`, handed in as rows — this crate never depends
//! on `receipt`, keeping Law 3: only `wire-map`/`sidecar` see both wire and
//! model). `edge` and `claim` are defined here as the contract; their
//! projections are populated by the units that own their source facts — `edge`
//! by the pin/lock manifest reader, `claim` by the run plane — and are left
//! empty here rather than half-projected (honest, stated).

use duckdb::Connection;
use duckdb::types::Value;
use model::{Document, Node, NodeKind};

/// The d2 §2.1 fact-table DDL: `node` · `edge` · `claim` · `receipt_journal`.
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

-- edge — one row per manifest item (a doc's `^inputs`). Manifest LEFT-joined
-- with the lock: pinned_rev NULL = declared-only (grey). Populated by the
-- pin/lock reader (a later Block-2 unit); the contract is fixed here.
CREATE TABLE edge (
    src_path    TEXT     NOT NULL,  -- [1] the page whose `^inputs` declares the ref
    declared_ref TEXT    NOT NULL,  -- [1] the ref as written in the manifest
    to_path     TEXT,               -- [2] pin-time resolved path (NULL = unresolved)
    to_sel      TEXT,               -- [2] pin-time resolved selector
    pinned_rev  TEXT,               -- [2] the rev recorded in the lock (NULL = declared-only -> grey)
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

-- receipt_journal — one row per guarded write, projected from the ONE reserved
-- root-EXCLUDED journal page. Every column is a source-3 fact (A5 bound).
CREATE TABLE receipt_journal (
    anchor      TEXT     NOT NULL,  -- [3] the row block anchor 'r-NNNNNN'
    op          TEXT     NOT NULL,  -- [3] guarded op: splice|create|remove
    path        TEXT     NOT NULL,  -- [3] the content target the write landed on
    actor       TEXT,               -- [3] recorded actor (nullable — never invented)
    now         TEXT,               -- [3] recorded timestamp (nullable — never invented)
    root_before TEXT     NOT NULL,  -- [3] workspace tree merkle BEFORE the write
    root_after  TEXT     NOT NULL,  -- [3] workspace tree merkle AFTER the write (recordable: journal is root-excluded)
    edits       UBIGINT  NOT NULL,  -- [3] edit count in the write
    line_no     UBIGINT  NOT NULL,  -- [3] 1-based position in the journal (chain/citation coordinate)
    PRIMARY KEY (anchor)
);
";

/// The facts-schema version. A reader whose version differs treats the facts
/// projection as ABSENT (delete-don't-migrate, mirroring [`crate::schema`]).
pub const FACTS_SCHEMA_VERSION: i32 = 1;

/// One receipt-journal row's scalar facts — the projection input. A plain
/// struct so this crate stays free of `receipt` (Law 3): the caller parses the
/// journal page with `receipt::journal::parse_rows` and maps each row here.
#[derive(Debug, Clone)]
pub struct JournalRowInput {
    pub anchor: String,
    pub op: String,
    pub path: String,
    pub actor: Option<String>,
    pub now: Option<String>,
    pub root_before: String,
    pub root_after: String,
    pub edits: u64,
    pub line_no: u64,
}

/// Create the four fact tables against `conn`.
///
/// # Errors
/// Propagates any `DuckDB` error from executing the DDL batch.
pub fn create_facts_schema(conn: &Connection) -> duckdb::Result<()> {
    conn.execute_batch(FACTS_SCHEMA_SQL)
}

/// Build an in-memory fact-table projection over `docs` + `journal_rows`: the
/// `node` table from the parsed corpus and `receipt_journal` from the supplied
/// journal rows. `edge`/`claim` exist (the contract) but are not populated here
/// (owned by later units — stated in the module docs).
///
/// # Errors
/// Propagates any `DuckDB` error from opening the database, creating the schema,
/// or inserting the projected rows.
pub fn build_facts_memory(
    docs: &std::collections::BTreeMap<String, Document>,
    journal_rows: &[JournalRowInput],
) -> duckdb::Result<Connection> {
    let conn = Connection::open_in_memory()?;
    create_facts_schema(&conn)?;
    project_nodes(&conn, docs)?;
    project_journal(&conn, journal_rows)?;
    Ok(conn)
}

/// Project every document's addressable spans into `node` rows: the document
/// root, each section, the frontmatter node, and each block anchor — exactly
/// the spans the selector grammar (§2.2) addresses. Inline nodes (wikilinks,
/// tags) carry no independent selector and are not node rows.
///
/// `pub(crate)` so the U2.9 read face ([`crate::read_face`]) can project `node`
/// into the SAME connection as its `^inputs`/board projections (board reds join
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

/// Project the supplied journal rows into `receipt_journal` (page order).
/// `pub(crate)` for the same reason as [`project_nodes`]: the read face mounts
/// the `^receipt` projection alongside `node` in one connection.
pub(crate) fn project_journal(conn: &Connection, rows: &[JournalRowInput]) -> duckdb::Result<()> {
    let mut stmt = conn.prepare(
        "INSERT INTO receipt_journal \
         (anchor, op, path, actor, now, root_before, root_after, edits, line_no) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )?;
    for r in rows {
        stmt.execute(duckdb::params_from_iter(
            [
                Value::Text(r.anchor.clone()),
                Value::Text(r.op.clone()),
                Value::Text(r.path.clone()),
                r.actor.clone().map_or(Value::Null, Value::Text),
                r.now.clone().map_or(Value::Null, Value::Text),
                Value::Text(r.root_before.clone()),
                Value::Text(r.root_after.clone()),
                Value::UBigInt(r.edits),
                Value::UBigInt(r.line_no),
            ]
            .iter(),
        ))?;
    }
    Ok(())
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
        let conn = build_facts_memory(&docs, &[]).expect("facts build");

        // all four fact tables exist (the contract).
        for t in ["node", "edge", "claim", "receipt_journal"] {
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

    /// `receipt_journal` is a projection over supplied journal rows; a nullable
    /// actor projects as SQL NULL (never invented).
    #[test]
    fn receipt_journal_projects_supplied_rows() {
        let rows = vec![
            JournalRowInput {
                anchor: "r-000001".into(),
                op: "splice".into(),
                path: "notes/plan.md".into(),
                actor: Some("alice".into()),
                now: Some("2026-07-23T12:00:00Z".into()),
                root_before: "b3:0".into(),
                root_after: "b3:1".into(),
                edits: 1,
                line_no: 2,
            },
            JournalRowInput {
                anchor: "r-000002".into(),
                op: "create".into(),
                path: "notes/new.md".into(),
                actor: None,
                now: None,
                root_before: "b3:1".into(),
                root_after: "b3:2".into(),
                edits: 0,
                line_no: 3,
            },
        ];
        let docs = BTreeMap::new();
        let conn = build_facts_memory(&docs, &rows).expect("facts build");

        let n: i64 = conn
            .query_row("SELECT count(*) FROM receipt_journal", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 2);
        let null_actors: i64 = conn
            .query_row(
                "SELECT count(*) FROM receipt_journal WHERE actor IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(null_actors, 1, "an absent actor projects as SQL NULL");
    }
}
