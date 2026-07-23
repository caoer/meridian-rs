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
use model::{CorpusIndex, Document, Node, NodeKind};

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
    hash_algo    TEXT,                -- [1] the block's `hash-algo:` header (NULL = absent); != 'node-rev' -> grey superseded-algo
    src_doc_rev  TEXT     NOT NULL,   -- [1] containing doc_rev — the rev-compare invalidation key
    PRIMARY KEY (src_path, seq)
);

-- board_drift — RED in the default face: a pinned NATIVE-algo lock item whose
-- live target content rev differs from the pinned rev (§2.3 red; the
-- doctored-verdict trace, §5.3). A foreign-algo lock (hash_algo not in the
-- engine-native set {node-rev, v2}) cannot drift — the engine never computed its
-- rev — so it is excluded here and renders superseded-algo grey (U3.4). The
-- v1→v2 supersede keeps the node-rev value under the `v2` label, so a `v2` pin
-- drifts/greens through the SAME node_rev compare. Computed per query by joining
-- the parse projection against live nodes.
CREATE VIEW board_drift AS
    SELECT il.src_path, il.seq, il.declared_ref, il.to_path, il.to_sel,
           il.pinned_rev, n.node_rev AS live_rev, 'content-drifted' AS reason
    FROM input_lock il
    JOIN node n ON n.path = il.to_path AND n.selector = il.to_sel
    WHERE il.pinned_rev IS NOT NULL AND (il.hash_algo IS NULL OR il.hash_algo IN ('node-rev', 'v2'))
      AND n.node_rev <> il.pinned_rev;

-- board_unresolved — RED in the default face: a pinned NATIVE-algo lock item
-- whose `to` selector resolves to no live node (rename / delete / rewrite; §2.3,
-- §2.5). Foreign-algo locks (not in {node-rev, v2}) render superseded-algo grey,
-- never red — excluded.
CREATE VIEW board_unresolved AS
    SELECT il.src_path, il.seq, il.declared_ref, il.to_path, il.to_sel,
           il.pinned_rev
    FROM input_lock il
    LEFT JOIN node n ON n.path = il.to_path AND n.selector = il.to_sel
    WHERE il.pinned_rev IS NOT NULL AND (il.hash_algo IS NULL OR il.hash_algo IN ('node-rev', 'v2'))
      AND n.path IS NULL;

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
    -- green: pinned NATIVE-algo ({node-rev, v2}) + live rev still equals the
    -- frozen pinned rev. The v1→v2 supersede keeps the node-rev value under the
    -- `v2` label, so it greens through the SAME node_rev compare.
    SELECT il.src_path, il.seq, il.to_path, il.to_sel, il.pinned_rev,
           n.node_rev AS live_rev, 'green' AS color, 'attested' AS reason
        FROM input_lock il
        JOIN node n ON n.path = il.to_path AND n.selector = il.to_sel
        WHERE il.pinned_rev IS NOT NULL AND (il.hash_algo IS NULL OR il.hash_algo IN ('node-rev', 'v2'))
          AND n.node_rev = il.pinned_rev
    UNION ALL
    -- red: drift (doctored verdict) + unresolved (rename/delete of the pinned target).
    SELECT src_path, seq, to_path, to_sel, pinned_rev,
           live_rev, 'red' AS color, reason
        FROM board_red
    UNION ALL
    -- grey superseded-algo: pinned under a hash-algo this engine does not compute
    -- (v1/merkle-v1/foreign — anything outside the native {node-rev, v2} set).
    -- Readable, unverifiable here — never red, never green; an archived v1 block
    -- renders this forever (d2 §6.3; U0.2/U3.4).
    SELECT il.src_path, il.seq, il.to_path, il.to_sel, il.pinned_rev,
           NULL AS live_rev, 'grey' AS color, 'superseded-algo' AS reason
        FROM input_lock il
        WHERE il.pinned_rev IS NOT NULL AND il.hash_algo IS NOT NULL AND il.hash_algo NOT IN ('node-rev', 'v2')
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
    let index = corpus_index(docs);
    let mut stmt = conn.prepare(
        "INSERT INTO input_lock \
         (src_path, seq, declared_ref, to_path, to_sel, pinned_rev, rev_class, hash_algo, src_doc_rev) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )?;
    for (path, doc) in docs {
        let src_doc_rev = doc.root.node_rev.0.clone();
        for (seq, item) in page_lock_items_in_corpus(path, doc, &index, docs)
            .into_iter()
            .enumerate()
        {
            stmt.execute(duckdb::params_from_iter(
                [
                    Value::Text(path.clone()),
                    Value::UBigInt(u64c(seq)),
                    Value::Text(item.declared_ref),
                    Value::Text(item.to_path),
                    Value::Text(item.to_sel),
                    item.pinned_rev.map_or(Value::Null, Value::Text),
                    item.rev_class.map_or(Value::Null, Value::Text),
                    item.hash_algo.map_or(Value::Null, Value::Text),
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
    /// The containing block's `hash-algo:` header (`None` = absent). A value
    /// other than [`model::NODE_REV_ALGO`] means the pinned rev was minted under
    /// an algo this engine does not compute — the read face renders it grey
    /// `superseded-algo` (U3.4). Per-block, stamped onto every item of the block.
    pub hash_algo: Option<String>,
}

/// Parse every `^inputs` lock item declared in `doc`, document order (source 1).
/// The SHARED reader for the board projection ([`project_input_locks`]) and the
/// walk plane ([`crate::walk`]) — one owner for the lock grammar.
///
/// Reads BOTH serializations of the SAME `^inputs` chain (U3.4 compat reader):
/// - **form-1** (mrd native) — the `^inputs` token is in the fence info
///   (`` ```yaml ^inputs ``), body is `items:` + flow-mappings
///   ([`collect_lock_items`]);
/// - **form-2** (ratified SCHEMA.md effect-receipt) — a plain `` ```yaml `` block
///   whose TRAILING block-anchor line is `^inputs`, body a block-SEQUENCE of
///   `- ref:/hash:` items plus a bare `hash-algo:` header
///   ([`collect_form2_lock_items`]).
///
/// A page carries ONE form or the other, never both — the form-2 pass skips any
/// block the form-1 pass already owns (its fence info carries `^inputs`), so no
/// item is double-counted.
#[must_use]
pub fn page_lock_items(doc: &Document) -> Vec<LockItem> {
    let mut out = Vec::new();
    collect_lock_items(&doc.root, &doc.raw, &mut out);
    collect_form2_lock_items(doc, &mut out);
    out
}

/// Build the corpus [`CorpusIndex`] over `docs` — the vault name/alias index the
/// form-2 wikilink resolution needs (`getFirstLinkpathDest` parity, contract
/// §4.5). One owner: both the board projection ([`project_input_locks`]) and the
/// walk plane ([`crate::walk`]) build it from the SAME `docs`.
#[must_use]
pub fn corpus_index(docs: &BTreeMap<String, Document>) -> CorpusIndex {
    let mut index = CorpusIndex::new();
    for (path, doc) in docs {
        index.insert(path, doc);
    }
    index
}

/// Parse `doc`'s `^inputs` lock items, then RESOLVE each item's `to_path` against
/// the corpus (the U3.4 wikilink wiring). A form-2 ref is a `[[wikilink]]`-by-NAME
/// (`llm-wiki-skill-compilation`), not a vault path, so the raw `to_path` matches
/// no `node.path` and the board/walk cannot find the target. This maps the name to
/// a real path via [`resolve_to_path`], so a native-algo form-2 pin verifies
/// green against its live target (Go resolves these; mrd must too, or Gate B
/// agreement fails). A form-1 path (already a real `node.path`) passes through
/// unchanged; an unresolvable ref keeps its bare name and renders red/unresolved.
#[must_use]
pub fn page_lock_items_in_corpus(
    src_path: &str,
    doc: &Document,
    index: &CorpusIndex,
    docs: &BTreeMap<String, Document>,
) -> Vec<LockItem> {
    let mut items = page_lock_items(doc);
    for item in &mut items {
        item.to_path = resolve_to_path(&item.to_path, src_path, index, docs);
    }
    items
}

/// Resolve a lock item's `to_path` to a real corpus path. Precedence:
/// 1. already a real `node.path` (a form-1 path, or a full-path ref that carries
///    its `.md`) — kept verbatim;
/// 2. the ref + `.md` is a real path (a full-path wikilink `a/b/c` → `a/b/c.md`);
/// 3. otherwise `getFirstLinkpathDest` by basename/alias
///    ([`CorpusIndex::resolve_linkpath`]).
///
/// An unresolvable ref returns its bare input unchanged — unresolved is
/// first-class (the edge then renders red `selector-unresolved`, never a false
/// green), exactly as a genuinely missing target should.
///
/// Public so the U3.4 supersede tool resolves a ref to the SAME path this reader
/// does — one owner for wikilink resolution, no reader/writer drift on which
/// node a `[[ref]]` names (the swept `node_rev` must match what the board reads).
#[must_use]
pub fn resolve_to_path(
    to_path: &str,
    src_path: &str,
    index: &CorpusIndex,
    docs: &BTreeMap<String, Document>,
) -> String {
    if docs.contains_key(to_path) {
        return to_path.to_string();
    }
    let with_md = format!("{to_path}.md");
    if docs.contains_key(&with_md) {
        return with_md;
    }
    index
        .resolve_linkpath(to_path, src_path)
        .unwrap_or_else(|| to_path.to_string())
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
    let hash_algo = block_hash_algo(body);
    for line in body.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix('-') else {
            continue;
        };
        let rest = rest.trim();
        let Some(inner) = rest.strip_prefix('{').and_then(|s| s.strip_suffix('}')) else {
            continue;
        };
        if let Some(mut item) = lock_item_from_flow(inner) {
            item.hash_algo.clone_from(&hash_algo);
            out.push(item);
        }
    }
}

/// The block's `hash-algo:` header value (the scalar after the first bare
/// `hash-algo:` line), or `None` when the block omits it. A trailing `# comment`
/// is stripped (the corpus carries `hash-algo: v1  # spec version …`). The
/// engine writes exactly one header, before `items:`; item lines start with `-`
/// and are never matched here.
fn block_hash_algo(body: &str) -> Option<String> {
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('-') {
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("hash-algo:") {
            let value = value.split('#').next().unwrap_or(value).trim();
            if !value.is_empty() {
                return Some(unquote(value));
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// form-2 — the ratified SCHEMA.md effect-receipt chain block (U3.4 compat reader)
// ---------------------------------------------------------------------------

/// Append the form-2 chain-block lock items of `doc` to `out` (document order).
///
/// Form-2 is INVISIBLE to the form-1 reader ([`collect_lock_items`]): its
/// `^inputs` token is a TRAILING block-anchor line (a sibling [`NodeKind::Anchor`]
/// with `name == "inputs"`), not a fence-info token, and its body is a
/// block-SEQUENCE (`- ref:` / `claim:` / `hash:`) plus a bare `hash-algo:` header.
///
/// Detection walks `doc.root` collecting every [`NodeKind::CodeBlock`] and every
/// `^inputs` anchor. A plain `` ```yaml `` block is a form-2 chain block iff an
/// `^inputs` anchor IMMEDIATELY FOLLOWS it — the bytes between the block's end and
/// the anchor's start are whitespace-only. A block whose fence info already
/// carries `^inputs` is a form-1 block ([`is_inputs_lang`]) and is left to the
/// form-1 pass, so no item is double-counted. Detection is via the block+anchor
/// alone — never the frontmatter `inputs: '[[#^inputs]]'` discriminator (this is a
/// READER; the U3.4 ruling licenses no schema conversion, no write).
fn collect_form2_lock_items(doc: &Document, out: &mut Vec<LockItem>) {
    let mut blocks: Vec<&Node> = Vec::new();
    let mut anchors: Vec<&Node> = Vec::new();
    collect_blocks_and_inputs_anchors(&doc.root, &mut blocks, &mut anchors);
    for block in blocks {
        let NodeKind::CodeBlock { lang, .. } = &block.kind else {
            continue;
        };
        // A form-1 block (its fence info carries `^inputs`) is owned by the form-1
        // pass — never re-read here (no double-count for a page in either form).
        if is_inputs_lang(lang) {
            continue;
        }
        if !inputs_anchor_immediately_follows(&doc.raw, block, &anchors) {
            continue;
        }
        if let Some(body) = doc.raw.get(block.span.clone()) {
            parse_form2_body(body, out);
        }
    }
}

/// Collect (into `blocks`) every [`NodeKind::CodeBlock`] node and (into `anchors`)
/// every `^inputs` [`NodeKind::Anchor`] node reachable from `node`, pre-order.
fn collect_blocks_and_inputs_anchors<'a>(
    node: &'a Node,
    blocks: &mut Vec<&'a Node>,
    anchors: &mut Vec<&'a Node>,
) {
    match &node.kind {
        NodeKind::CodeBlock { .. } => blocks.push(node),
        NodeKind::Anchor { name } if name == "inputs" => anchors.push(node),
        _ => {}
    }
    for child in &node.children {
        collect_blocks_and_inputs_anchors(child, blocks, anchors);
    }
}

/// Whether any `^inputs` anchor immediately follows `block` — its span starts at
/// or after the block's end with only whitespace between (the form-2 marker). The
/// anchor's model span is its host line (`^inputs`); the gap is the blank line(s)
/// the writer leaves between the fenced block and the trailing anchor.
fn inputs_anchor_immediately_follows(raw: &str, block: &Node, anchors: &[&Node]) -> bool {
    anchors.iter().any(|anchor| {
        anchor.span.start >= block.span.end
            && raw
                .get(block.span.end..anchor.span.start)
                .is_some_and(|gap| gap.chars().all(char::is_whitespace))
    })
}

/// Parse a form-2 chain-block body into lock items, appended to `out`. The body
/// (the whole fenced block, fences included) is a block-SEQUENCE:
/// - a `- ref: <value>` line starts a NEW item — `declared_ref` is the unquoted
///   value with a surrounding `[[ ]]` wikilink stripped, its trailing `#sel` split
///   into `to_path` / `to_sel` (no `#` ⇒ `to_sel = ""`);
/// - a `hash: <value>` continuation line sets the current item's `pinned_rev`
///   verbatim (any `merkle-v1:` prefix kept — it is the pinned rev as written);
/// - `claim:` is an ignored passenger, `rev_class` stays `None`.
///
/// Every item's `hash_algo` is the block's bare `hash-algo:` header
/// ([`block_hash_algo`], e.g. `v1`) — a NAMED non-`node-rev` algo, so the read
/// face renders these grey `superseded-algo` (U3.4).
fn parse_form2_body(body: &str, out: &mut Vec<LockItem>) {
    let hash_algo = block_hash_algo(body);
    let mut current: Option<usize> = None;
    for line in body.lines() {
        let trimmed = line.trim();
        if let Some(after_dash) = trimmed.strip_prefix('-') {
            // A block-sequence item start; its first key must be `ref:`.
            if let Some(value) = after_dash.trim_start().strip_prefix("ref:") {
                let declared_ref = strip_wikilink(&unquote(value.trim()));
                let (to_path, to_sel) = split_selector(&declared_ref);
                out.push(LockItem {
                    declared_ref,
                    to_path,
                    to_sel,
                    pinned_rev: None,
                    rev_class: None,
                    hash_algo: hash_algo.clone(),
                });
                current = Some(out.len() - 1);
            }
        } else if let Some(value) = trimmed.strip_prefix("hash:")
            && let Some(i) = current
        {
            out[i].pinned_rev = Some(unquote(value.trim()));
        }
    }
}

/// Strip a surrounding `[[ ]]` wikilink from a ref value, best-effort
/// (`[[llm-wiki-skill-compilation]]` → `llm-wiki-skill-compilation`); a value with
/// no brackets is returned unchanged. An Obsidian display alias (`[[target|show]]`)
/// is dropped — only the LINK TARGET addresses a node, the `|show` text is render
/// sugar — so `[[usage|meridian usage]]` resolves as `usage` (a `#section`
/// survives for [`split_selector`]).
fn strip_wikilink(value: &str) -> String {
    let inner = value
        .strip_prefix("[[")
        .and_then(|s| s.strip_suffix("]]"))
        .unwrap_or(value);
    inner
        .split_once('|')
        .map_or(inner, |(target, _alias)| target)
        .to_string()
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
        hash_algo: None, // stamped per-block by parse_lock_body
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
        let raw = "# Doc\n\n```yaml ^inputs\nhash-algo: node-rev\nitems:\n  - {ref: 'a.md', to: 'a.md#^c', rev: 'deadbeef', rev_class: 'content'}\n  - {ref: 'b.md', claim: 'declared-only'}\n```\n";
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

    // -----------------------------------------------------------------------
    // form-2 — the ratified SCHEMA.md effect-receipt chain block (U3.4)
    // -----------------------------------------------------------------------

    /// The real effect-page form: a plain `` ```yaml `` block, block-SEQUENCE body
    /// (`- ref:` / `claim:` / `hash:`), a bare `hash-algo: v1` header, and a
    /// TRAILING `^inputs` anchor line. `page_lock_items` now SEES it (pre-U3.4:
    /// zero items). `declared_ref` is dewikilinked, `pinned_rev` is the `hash:`
    /// value verbatim (`merkle-v1:` kept), and `hash_algo` is the block header.
    #[test]
    fn form2_chain_block_parses_ref_hash_and_algo() {
        let raw = "## Chain\n\n```yaml\n- ref: '[[llm-wiki-skill-compilation]]'\n  claim:\n  hash: 'merkle-v1:247e292cc3c62e103424ad04cecb36517711cdfe42bc245ef516cfe54b83073d'\nhash-algo: v1\n```\n\n^inputs\n";
        let items = page_lock_items(&doc(raw));
        assert_eq!(
            items.len(),
            1,
            "form-2 chain parses (pre-U3.4: zero — blind)"
        );
        let item = &items[0];
        assert_eq!(item.declared_ref, "llm-wiki-skill-compilation");
        assert_eq!(item.to_path, "llm-wiki-skill-compilation");
        assert_eq!(item.to_sel, "");
        assert_eq!(
            item.pinned_rev.as_deref(),
            Some("merkle-v1:247e292cc3c62e103424ad04cecb36517711cdfe42bc245ef516cfe54b83073d"),
        );
        assert_eq!(item.rev_class, None);
        assert_eq!(item.hash_algo.as_deref(), Some("v1"));
    }

    /// A form-2 ref with an Obsidian display alias (`[[target|show]]`) resolves as
    /// the TARGET, dropping the `|show` render text — `[[usage|meridian usage]]`
    /// declares `usage`, not `usage|meridian usage` (which resolves to nothing).
    #[test]
    fn form2_ref_drops_display_alias() {
        let raw = "## Chain\n\n```yaml\n- ref: '[[usage|meridian usage]]'\n  hash: 'merkle-v1:abcd'\nhash-algo: v1\n```\n\n^inputs\n";
        let items = page_lock_items(&doc(raw));
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].declared_ref, "usage",
            "the `|meridian usage` display alias is dropped — only `usage` addresses a node",
        );
        assert_eq!(items[0].to_path, "usage");
    }

    /// A form-2 block with MULTIPLE `- ref:` items parses ALL of them, in order,
    /// each with its own `hash:` and the shared block `hash-algo:` header. A `#sel`
    /// on a ref splits into `to_path` / `to_sel`.
    #[test]
    fn form2_multiple_refs_all_parse() {
        let raw = "## Chain\n\n```yaml\n- ref: '[[alpha]]'\n  hash: 'merkle-v1:aaaa'\n- ref: '[[beta.md#^claim]]'\n  claim:\n  hash: 'merkle-v1:bbbb'\n- ref: 'gamma.md'\n  hash: 'merkle-v1:cccc'\nhash-algo: v1\n```\n\n^inputs\n";
        let items = page_lock_items(&doc(raw));
        assert_eq!(items.len(), 3, "all three block-sequence items parse");
        assert_eq!(
            items
                .iter()
                .map(|i| i.declared_ref.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "beta.md#^claim", "gamma.md"],
        );
        // The middle ref's `#^claim` splits into to_path / to_sel.
        assert_eq!(items[1].to_path, "beta.md");
        assert_eq!(items[1].to_sel, "^claim");
        assert_eq!(items[2].pinned_rev.as_deref(), Some("merkle-v1:cccc"));
        assert!(items.iter().all(|i| i.hash_algo.as_deref() == Some("v1")));
    }

    /// A `` ```yaml `` block NOT followed by an `^inputs` anchor (e.g. the sibling
    /// `## Receipt` block, trailing anchor `^receipt`) is NOT a form-2 chain — it
    /// projects zero lock items. Only the `^inputs`-anchored block is read.
    #[test]
    fn form2_non_inputs_anchored_block_is_ignored() {
        // A receipt-shaped block: plain yaml, trailing `^receipt` (not `^inputs`).
        let raw = "## Receipt\n\n```yaml\ncommit: abc123\nverdict: 'x'\n```\n\n^receipt\n";
        assert!(
            page_lock_items(&doc(raw)).is_empty(),
            "a `^receipt`-anchored block is not an `^inputs` chain",
        );
    }

    /// A form-1 page (the `^inputs` token in the fence info) is read by the form-1
    /// pass ONLY — the form-2 pass skips it, so items are never double-counted.
    #[test]
    fn form1_page_is_not_double_counted_by_form2_pass() {
        let raw = "# Doc\n\n```yaml ^inputs\nhash-algo: node-rev\nitems:\n  - {ref: 'a.md', rev: 'deadbeef'}\n```\n";
        assert_eq!(page_lock_items(&doc(raw)).len(), 1, "one item, read once");
    }
}
