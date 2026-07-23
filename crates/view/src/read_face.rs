//! U2.9 — the C1 board-red projections + the locked read face (design 2 §2.1 /
//! §2.3 / §5.3; plan §3 decision C1 = YES; advisor checklist item 24).
//!
//! Three things land here, all mounted OVER U2.1's schema-v2 fact tables
//! ([`crate::facts`]) — never a fork of that contract:
//!
//! 1. **The `^inputs` projection** — a pure parse (source 1) of each page's
//!    `^inputs` lock block into [`input_lock`](READ_FACE_SCHEMA_SQL) rows. Every
//!    row records the containing page's `doc_rev` (`src_doc_rev`), the
//!    rev-compare invalidation key: the projection cache self-invalidates by
//!    `doc_rev`, so no stored second truth exists (§2.1, §8).
//! 2. **The board-red views** — `board_drift` (a pinned lock item whose LIVE
//!    target rev ≠ its `pinned_rev`) and `board_unresolved` (a pinned item whose
//!    `to` selector resolves to no node), unioned as `board_red`. Board reds are
//!    computable in the DEFAULT read face with **no optional pack** (checklist
//!    24; §4.2 "live-rev ≠ armed-rev is ordinary red drift in the default face";
//!    §5.3 "a doctored verdict … verdicts freeze at close").
//! 3. **The locked read face** — [`lock_read_face`]: `enable_external_access =
//!    false` + `lock_configuration = true`, so an SQL view pack running over
//!    this connection **physically has no write path** — no `ATTACH`, no `COPY`,
//!    no external file read, and it cannot re-raise any of it (A10, adversary
//!    carry 1; §2.1: "locked down as a capability, not a convention").
//!
//! `edge`/`claim` are NOT populated here — their population is owned by their
//! source units (pin, run plane). U2.9 reads parse and locks the face only.
//!
//! # U5.1 — the board-view colors + the trace query (d2 §5.3)
//! Two default-face views ride the SAME locked read face (no face widening — the
//! blocked-`ATTACH` guard still holds with them loaded):
//!
//! - **`board`** — the colors layer: exactly one `green`/`red`/`grey` row per
//!   `^inputs` edge. `grey` = declared-unpinned (an ungated close, never green);
//!   `red` = drift or unresolved (a doctored verdict); `green` = the frozen pin
//!   still equals live. "Verdicts freeze at close" is the `pinned_rev` compare —
//!   the board reads the frozen pin, it never recomputes the verdict.
//! - **`co_edit_trace`** — the traces layer: the reserved journal's mechanical
//!   write-facts, per target path, convention-free. A doctored verdict shows the
//!   subject written at the close and edited again afterwards (a co-edit),
//!   visible before any pack reads it as red.

use std::collections::BTreeMap;

use duckdb::Connection;
use duckdb::types::Value;
use model::{Document, Node, NodeKind};

use crate::ViewError;
use crate::facts;

/// The additive read-face DDL: the `^inputs` parse projection + the board-red
/// views. A separate schema from the frozen [`crate::facts`] contract —
/// additive, never an edit to what U2.1 shipped. Every `input_lock` column is a
/// source-1 (parse) fact; the board views compute color per query, never stored.
pub const READ_FACE_SCHEMA_SQL: &str = r"
-- input_lock — the parse projection of each page's `^inputs` lock block. One row
-- per lock item, exactly as written in the vault bytes (source 1). Distinct from
-- `edge` (owned by pin: the manifest LEFT-joined with live resolution). Every
-- row carries the containing page's doc_rev — the rev-compare invalidation key.
CREATE TABLE input_lock (
    src_path     TEXT     NOT NULL,   -- [1] page whose ^inputs block declares the item
    seq          UBIGINT  NOT NULL,   -- [1] item order within the lock block
    declared_ref TEXT     NOT NULL,   -- [1] the `ref` field, verbatim
    to_path      TEXT     NOT NULL,   -- [1] `to` page path (the `ref` path when `to` is absent)
    to_sel       TEXT     NOT NULL,   -- [1] `to` selector ('' = the page/doc root)
    pinned_rev   TEXT,                -- [1] the `rev` field (NULL = declared-only -> grey)
    rev_class    TEXT,                -- [1] 'content' | 'object' (NULL = unstated)
    src_doc_rev  TEXT     NOT NULL,   -- [1] containing doc_rev — the rev-compare invalidation key
    PRIMARY KEY (src_path, seq)
);

-- board_drift — RED in the default face: a pinned lock item whose live target
-- content rev differs from the pinned rev (§2.3 red; the doctored-verdict trace,
-- §5.3). Computed per query by joining the parse projection against live nodes.
CREATE VIEW board_drift AS
    SELECT il.src_path, il.seq, il.declared_ref, il.to_path, il.to_sel,
           il.pinned_rev, n.node_rev AS live_rev, 'content-drifted' AS reason
    FROM input_lock il
    JOIN node n ON n.path = il.to_path AND n.selector = il.to_sel
    WHERE il.pinned_rev IS NOT NULL AND n.node_rev <> il.pinned_rev;

-- board_unresolved — RED in the default face: a pinned lock item whose `to`
-- selector resolves to no live node (rename / delete / rewrite; §2.3, §2.5).
CREATE VIEW board_unresolved AS
    SELECT il.src_path, il.seq, il.declared_ref, il.to_path, il.to_sel,
           il.pinned_rev
    FROM input_lock il
    LEFT JOIN node n ON n.path = il.to_path AND n.selector = il.to_sel
    WHERE il.pinned_rev IS NOT NULL AND n.path IS NULL;

-- board_red — the union board-red surface: drift + unresolved. Grey
-- (declared-only, pinned_rev NULL) never appears here; green never appears here.
-- A reader renders red iff a row is present, in the DEFAULT face, no pack.
CREATE VIEW board_red AS
    SELECT src_path, seq, to_path, to_sel, pinned_rev, live_rev, reason
        FROM board_drift
    UNION ALL
    SELECT src_path, seq, to_path, to_sel, pinned_rev,
           NULL AS live_rev, 'selector-unresolved' AS reason
        FROM board_unresolved;

-- board — U5.1's colors layer (d2 §5.3 'colors = board view'; wire-contract-v2
-- colors-amendment § Colors). Exactly ONE color row per `^inputs` edge, in the
-- DEFAULT face, no pack. The color is 'traces read through workflow vocabulary':
--   green  — the pinned rev the verdict FROZE AT CLOSE still equals the live rev
--            (nothing drifted since the close);
--   red    — the pinned rev no longer equals live (a DOCTORED VERDICT: edited
--            after close), OR the pinned target resolves to no live node;
--   grey   — declared-unpinned (pinned_rev NULL): an UNGATED close (a bare flip
--            that never froze a verdict rev). The ledger cannot verify it — grey,
--            NEVER green, never silently clean.
-- Verdicts-freeze-at-close: the pin (`pinned_rev`) IS the verdict frozen at
-- close; the board compares the LIVE rev against that frozen rev, it never
-- recomputes the verdict. A closed card color is a reading of the frozen pin.
CREATE VIEW board AS
    -- green: pinned + live rev still equals the frozen pinned rev.
    SELECT il.src_path, il.seq, il.to_path, il.to_sel, il.pinned_rev,
           n.node_rev AS live_rev, 'green' AS color, 'attested' AS reason
        FROM input_lock il
        JOIN node n ON n.path = il.to_path AND n.selector = il.to_sel
        WHERE il.pinned_rev IS NOT NULL AND n.node_rev = il.pinned_rev
    UNION ALL
    -- red: drift (doctored verdict) + unresolved (rename/delete of the pinned target).
    SELECT src_path, seq, to_path, to_sel, pinned_rev,
           live_rev, 'red' AS color, reason
        FROM board_red
    UNION ALL
    -- grey: declared-unpinned — the ungated close, never green.
    SELECT il.src_path, il.seq, il.to_path, il.to_sel, il.pinned_rev,
           NULL AS live_rev, 'grey' AS color, 'declared-unpinned' AS reason
        FROM input_lock il
        WHERE il.pinned_rev IS NULL;

-- co_edit_trace — U5.1's traces layer (d2 §5.3 'traces = core, source 3,
-- default-face visible'). The MECHANICAL write-facts of the reserved journal,
-- read convention-FREE: one row per guarded write, ordered per target path, with
-- the write's position on that path (`edit_ord`) and the total (`edits_on_path`).
-- A doctored verdict leaves the pack-free trace here: the subject was written at
-- the close (edit_ord 1) and EDITED AGAIN afterwards (edit_ord 2) — a co-edit
-- visible with no armed convention, before any board pack calls the later rev red.
CREATE VIEW co_edit_trace AS
    SELECT rj.path, rj.anchor, rj.op, rj.actor, rj.line_no,
           rj.root_before, rj.root_after, rj.edits,
           row_number() OVER (PARTITION BY rj.path ORDER BY rj.line_no) AS edit_ord,
           count(*)     OVER (PARTITION BY rj.path)                     AS edits_on_path
        FROM receipt_journal rj;
";

/// Create the additive read-face schema (`input_lock` + board views) against
/// `conn`. The caller must have created [`crate::facts`]'s schema first (the
/// board views reference `node`).
///
/// # Errors
/// Propagates any `DuckDB` error from executing the DDL batch.
pub fn create_read_face_schema(conn: &Connection) -> duckdb::Result<()> {
    conn.execute_batch(READ_FACE_SCHEMA_SQL)
}

/// Lock the read face as a capability (A10; §2.1). After this call the
/// connection can only READ: `enable_external_access = false` removes every
/// write/external path (`ATTACH`, `COPY`, `read_csv`/`read_parquet`,
/// `INSTALL`/`LOAD`), and `lock_configuration = true` freezes settings so
/// untrusted SQL cannot re-raise any of it. An SQL view pack over this
/// connection physically has no write path.
///
/// Ordering law: call this AFTER the schema is created and every projection is
/// inserted — the lock blocks nothing the projector does (ordinary in-memory
/// INSERTs), only the external/write surface a downstream view pack could reach.
///
/// # Errors
/// Propagates any `DuckDB` error from applying the two `SET` statements.
pub fn lock_read_face(conn: &Connection) -> duckdb::Result<()> {
    conn.execute_batch("SET enable_external_access=false;\nSET lock_configuration=true;")
}

/// Build a locked, board-ready read face over `docs` + `journal_rows`: project
/// `node` and `receipt_journal` (via [`crate::facts`]), project the `^inputs`
/// lock blocks into `input_lock`, create the board-red views, then LOCK the
/// face. The returned connection serves the default-face board queries and
/// **refuses `ATTACH`/`COPY`/external access** (the C1 read-face capability).
///
/// # Errors
/// [`ViewError::Duckdb`] on any schema-creation, projection, or lock failure.
pub fn open_board(
    docs: &BTreeMap<String, Document>,
    journal_rows: &[facts::JournalRowInput],
) -> Result<Connection, ViewError> {
    let conn = Connection::open_in_memory()?;
    facts::create_facts_schema(&conn)?;
    facts::project_nodes(&conn, docs)?;
    facts::project_journal(&conn, journal_rows)?;
    create_read_face_schema(&conn)?;
    project_input_locks(&conn, docs)?;
    lock_read_face(&conn)?;
    Ok(conn)
}

/// The paths whose live `doc_rev` no longer matches the rev the projection was
/// built at — the STALE set (design §2.1 / §8: the cache self-invalidates by
/// `doc_rev`). A path is stale iff its recorded `node.doc_rev` differs from the
/// live `docs[path].root.node_rev`, OR the path is absent from the projection.
/// A non-empty result means the projection must be rebuilt to answer honestly.
///
/// # Errors
/// Propagates any `DuckDB` error from reading the recorded `node` revs.
pub fn stale_paths(
    conn: &Connection,
    docs: &BTreeMap<String, Document>,
) -> duckdb::Result<Vec<String>> {
    let mut recorded: BTreeMap<String, String> = BTreeMap::new();
    let mut stmt = conn.prepare("SELECT DISTINCT path, doc_rev FROM node")?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        recorded.insert(row.get::<_, String>(0)?, row.get::<_, String>(1)?);
    }
    let mut stale = Vec::new();
    for (path, doc) in docs {
        let live = &doc.root.node_rev.0;
        if recorded.get(path) != Some(live) {
            stale.push(path.clone());
        }
    }
    Ok(stale)
}

// ---------------------------------------------------------------------------
// the `^inputs` parse projection
// ---------------------------------------------------------------------------

/// Project every page's `^inputs` lock items into `input_lock`. The lock block
/// is a fenced code block whose info string carries the `^inputs` anchor
/// (`` ```yaml ^inputs ``) — parsed here from the vault bytes alone (source 1).
fn project_input_locks(conn: &Connection, docs: &BTreeMap<String, Document>) -> duckdb::Result<()> {
    let mut stmt = conn.prepare(
        "INSERT INTO input_lock \
         (src_path, seq, declared_ref, to_path, to_sel, pinned_rev, rev_class, src_doc_rev) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )?;
    for (path, doc) in docs {
        let src_doc_rev = doc.root.node_rev.0.clone();
        for (seq, item) in page_lock_items(doc).into_iter().enumerate() {
            stmt.execute(duckdb::params_from_iter(
                [
                    Value::Text(path.clone()),
                    Value::UBigInt(u64c(seq)),
                    Value::Text(item.declared_ref),
                    Value::Text(item.to_path),
                    Value::Text(item.to_sel),
                    item.pinned_rev.map_or(Value::Null, Value::Text),
                    item.rev_class.map_or(Value::Null, Value::Text),
                    Value::Text(src_doc_rev.clone()),
                ]
                .iter(),
            ))?;
        }
    }
    Ok(())
}

/// One parsed `^inputs` lock item (source 1). Passengers the engine ignores
/// (`claim`, `at:`) are not carried — only the columns board reds and the walk
/// plane compute on.
///
/// Public because the walk plane ([`crate::walk`]) consumes the SAME parser this
/// board projection uses: one owner for the `^inputs` lock grammar (design "one
/// owner per fact"), never a second reader that could drift.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockItem {
    /// The `ref` field, verbatim (the declared ref).
    pub declared_ref: String,
    /// The `to` page path — the `ref` path when `to` is absent.
    pub to_path: String,
    /// The `to` selector, verbatim after the first `#` (`""` = the page/doc root).
    pub to_sel: String,
    /// The `rev` field — `None` = declared-only (grey).
    pub pinned_rev: Option<String>,
    /// The `rev_class` field — `content` | `object` (`None` = unstated).
    pub rev_class: Option<String>,
}

/// Parse every `^inputs` lock item declared in `doc`, document order (source 1).
/// The SHARED reader for the board projection ([`project_input_locks`]) and the
/// walk plane ([`crate::walk`]) — one owner for the lock grammar.
#[must_use]
pub fn page_lock_items(doc: &Document) -> Vec<LockItem> {
    let mut out = Vec::new();
    collect_lock_items(&doc.root, &doc.raw, &mut out);
    out
}

/// Walk the node tree, parsing the lock items of every `^inputs` code block into
/// `out` (document order — one lock block per page in practice, but any number
/// projects deterministically by walk order).
fn collect_lock_items(node: &Node, raw: &str, out: &mut Vec<LockItem>) {
    if let NodeKind::CodeBlock { lang, .. } = &node.kind
        && is_inputs_lang(lang)
        && let Some(body) = raw.get(node.span.clone())
    {
        parse_lock_body(body, out);
    }
    for child in &node.children {
        collect_lock_items(child, raw, out);
    }
}

/// Whether a fence info string addresses the `^inputs` lock — its whitespace
/// tokens include the `^inputs` anchor (`` ```yaml ^inputs `` → `["yaml",
/// "^inputs"]`). The anchor, not the language, is the marker.
fn is_inputs_lang(lang: &str) -> bool {
    lang.split_whitespace().any(|tok| tok == "^inputs")
}

/// Parse a fenced `^inputs` block body into lock items. Each `- {…}` flow
/// mapping is one item; fence-delimiter lines and scalar header lines
/// (`hash-algo:`, `items:`) are skipped. Deliberately narrow: the lock format is
/// engine-written and regular, so a bounded flow-mapping parse beats a full YAML
/// dependency (keeps `view` a dependency-light leaf).
fn parse_lock_body(body: &str, out: &mut Vec<LockItem>) {
    for line in body.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix('-') else {
            continue;
        };
        let rest = rest.trim();
        let Some(inner) = rest.strip_prefix('{').and_then(|s| s.strip_suffix('}')) else {
            continue;
        };
        if let Some(item) = lock_item_from_flow(inner) {
            out.push(item);
        }
    }
}

/// Parse one flow-mapping body (`ref: 'a.md', to: 'a.md#^c', rev: 'r1'`) into a
/// [`LockItem`]. A row without a `ref` is not a lock item (skipped). `to`
/// defaults to `ref`; the selector is the text after the first `#` (`` `` for a
/// bare page ref), matching `node.selector` (`^block-id` verbatim, heading hpath).
fn lock_item_from_flow(inner: &str) -> Option<LockItem> {
    let mut declared_ref: Option<String> = None;
    let mut to: Option<String> = None;
    let mut pinned_rev: Option<String> = None;
    let mut rev_class: Option<String> = None;
    for field in split_top_level_commas(inner) {
        let Some((key, value)) = field.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = unquote(value.trim());
        match key {
            "ref" => declared_ref = Some(value),
            "to" => to = Some(value),
            "rev" => pinned_rev = Some(value),
            "rev_class" => rev_class = Some(value),
            _ => {} // engine-ignored passengers (claim, at, hash-algo, …)
        }
    }
    let declared_ref = declared_ref?;
    let target = to.as_deref().unwrap_or(&declared_ref);
    let (to_path, to_sel) = split_selector(target);
    Some(LockItem {
        declared_ref,
        to_path,
        to_sel,
        pinned_rev,
        rev_class,
    })
}

/// Split a flow-mapping body on TOP-LEVEL commas only — a comma inside single or
/// double quotes stays with its field (a rev or claim may contain one).
fn split_top_level_commas(inner: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for ch in inner.chars() {
        match quote {
            Some(q) => {
                cur.push(ch);
                if ch == q {
                    quote = None;
                }
            }
            None => match ch {
                '\'' | '"' => {
                    quote = Some(ch);
                    cur.push(ch);
                }
                ',' => {
                    fields.push(std::mem::take(&mut cur));
                }
                _ => cur.push(ch),
            },
        }
    }
    if !cur.trim().is_empty() {
        fields.push(cur);
    }
    fields
}

/// Strip one layer of surrounding single or double quotes from a scalar value.
fn unquote(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        if (first == b'\'' || first == b'"') && bytes[bytes.len() - 1] == first {
            return value[1..value.len() - 1].to_string();
        }
    }
    value.to_string()
}

/// Split a `page#selector` target into `(path, selector)`. No `#` ⇒ the page
/// root (`selector = ""`). The selector is kept verbatim so a `^block-id`
/// carries its caret, matching the `node.selector` the projection stores.
fn split_selector(target: &str) -> (String, String) {
    match target.split_once('#') {
        Some((path, sel)) => (path.to_string(), sel.to_string()),
        None => (target.to_string(), String::new()),
    }
}

/// `usize` → `u64`, saturating (never truncates on a 32-bit target).
fn u64c(x: usize) -> u64 {
    u64::try_from(x).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(raw: &str) -> Document {
        model::build(raw.to_string(), syntax::parse(raw))
    }

    #[test]
    fn is_inputs_lang_matches_the_anchor_token_only() {
        assert!(is_inputs_lang("yaml ^inputs"));
        assert!(is_inputs_lang("^inputs"));
        assert!(!is_inputs_lang("yaml"));
        assert!(!is_inputs_lang("yaml ^outputs"));
    }

    #[test]
    fn flow_mapping_parses_ref_to_rev_and_class() {
        let item =
            lock_item_from_flow("ref: 'a.md', to: 'a.md#^claim', rev: 'r1', rev_class: 'content'")
                .expect("an item with a ref");
        assert_eq!(item.declared_ref, "a.md");
        assert_eq!(item.to_path, "a.md");
        assert_eq!(item.to_sel, "^claim");
        assert_eq!(item.pinned_rev.as_deref(), Some("r1"));
        assert_eq!(item.rev_class.as_deref(), Some("content"));
    }

    #[test]
    fn flow_mapping_without_to_falls_back_to_ref() {
        let item = lock_item_from_flow("ref: 'b.md', rev: 'r2'").expect("item");
        assert_eq!(item.to_path, "b.md");
        assert_eq!(item.to_sel, ""); // page root
        assert_eq!(item.pinned_rev.as_deref(), Some("r2"));
        assert!(item.rev_class.is_none());
    }

    #[test]
    fn declared_only_item_has_no_pinned_rev() {
        let item =
            lock_item_from_flow("ref: 'wiki/page.md', claim: 'declared-only, grey'").expect("item");
        assert!(item.pinned_rev.is_none(), "no rev -> declared-only -> grey");
    }

    #[test]
    fn top_level_comma_split_respects_quotes() {
        let fields = split_top_level_commas("ref: 'a,b.md', claim: 'x, y, z'");
        assert_eq!(fields.len(), 2, "a quoted comma does not split a field");
    }

    #[test]
    fn project_reads_the_inputs_block_from_parse() {
        let raw = "# Doc\n\n```yaml ^inputs\nhash-algo: statusd-file-rev\nitems:\n  - {ref: 'a.md', to: 'a.md#^c', rev: 'deadbeef', rev_class: 'content'}\n  - {ref: 'b.md', claim: 'declared-only'}\n```\n";
        let mut docs = BTreeMap::new();
        docs.insert("review.md".to_string(), doc(raw));
        let conn = open_board(&docs, &[]).expect("open board");
        let n: i64 = conn
            .query_row("SELECT count(*) FROM input_lock", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 2, "two lock items project");
        let pinned: i64 = conn
            .query_row(
                "SELECT count(*) FROM input_lock WHERE pinned_rev IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            pinned, 1,
            "only the item with a rev is pinned; the other is grey"
        );
        let doc_revs: i64 = conn
            .query_row(
                "SELECT count(*) FROM input_lock WHERE src_doc_rev = '' OR src_doc_rev IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            doc_revs, 0,
            "every projected row records its source doc_rev"
        );
    }
}
