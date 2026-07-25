//! U2.9 — the C1 board-red projections + the locked read face (design 2 §2.1 /
//! §2.3 / §5.3; plan §3 decision C1 = YES; advisor checklist item 24).
//!
//! Three things land here, all mounted OVER U2.1's schema-v2 fact tables
//! ([`crate::facts`]) — never a fork of that contract:
//!
//! 1. **The pin projection** — a pure parse (source 1) of each page's declared
//!    pins into [`input_lock`](READ_FACE_SCHEMA_SQL) rows, across all THREE
//!    forms the corpus carries (the legacy `^inputs` form-1/form-2 and the
//!    engine's own `meridian-lock` block, form-3 — see [`page_lock_items`]).
//!    Every row records the containing page's `doc_rev` (`src_doc_rev`), the
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
use model::selector::Color;
use model::{CorpusIndex, Document, Node, NodeKind};

use crate::ViewError;
use crate::facts;
use crate::walk;

/// The additive read-face DDL: the `^inputs` parse projection + the board-red
/// views. A separate schema from the frozen [`crate::facts`] contract —
/// additive, never an edit to what U2.1 shipped.
///
/// Every `input_lock` ADDRESS column is a source-1 (parse) fact. The three
/// `verdict_*` columns are the exception, and the exception is named: a
/// `meridian-lock` pin's color is a blake3 re-hash of the target's bytes, which
/// this face's SQL cannot express, so the ONE color plane
/// ([`crate::walk::lock_pin_colors`]) computes it and
/// [`project_input_locks`] carries the answer into the row. Not a second color
/// computer, and not a stored verdict: the whole read face is built per query
/// from the same `docs` snapshot and thrown away with the connection.
pub const READ_FACE_SCHEMA_SQL: &str = r"
-- input_lock — the parse projection of each page's `^inputs` lock block. One row
-- per lock item, exactly as written in the vault bytes (source 1). Distinct from
-- `edge` (owned by pin: the manifest LEFT-joined with live resolution). Every
-- row carries the containing page's doc_rev — the rev-compare invalidation key.
-- A `meridian-lock` (form-3) row additionally carries the fingerprint VERDICT
-- the color plane computed for it, so the board renders the SAME color the walk
-- renders for the same pin — one question, one answer, on both planes.
CREATE TABLE input_lock (
    src_path     TEXT     NOT NULL,   -- [1] page whose ^inputs block declares the item
    seq          UBIGINT  NOT NULL,   -- [1] item order within the lock block
    declared_ref TEXT     NOT NULL,   -- [1] the `ref` field, verbatim ('' on a lock-refusal row, which declares no ref)
    to_path      TEXT     NOT NULL,   -- [1] `to` page path (the `ref` path when `to` is absent; '' on a lock-refusal row, which names no target)
    to_sel       TEXT     NOT NULL,   -- [1] `to` selector ('' = the page/doc root)
    pinned_rev   TEXT,                -- [1] the pinned value as written (NULL = declared-only -> grey): a `rev` for the legacy forms, the `fingerprint` CID-token for a meridian-lock pin
    rev_class    TEXT,                -- [1] 'content' | 'object' (NULL = unstated)
    hash_algo    TEXT,                -- [1] the algo the pinned value was minted under (NULL = absent): the block's `hash-algo:` header for the legacy forms, the fingerprint token's own version field for a meridian-lock pin; != 'node-rev' -> grey superseded-algo
    src_doc_rev  TEXT     NOT NULL,   -- [1] containing doc_rev — the rev-compare invalidation key
    verdict_color  TEXT,              -- [color plane] a meridian-lock row's verdict tone ('green'|'red'|'grey'); NULL on EVERY legacy ^inputs row, which the board colors by the node_rev compare below
    verdict_reason TEXT,              -- [color plane] the verdict's stable reason word ('content-drifted', 'dangling-anchor', 'unverifiable-fingerprint', 'malformed-fingerprint', 'lock-refused', …); NULL for green
    verdict_detail TEXT,              -- [color plane] the reason's own detail — WHICH fingerprint-triple member is unknown, or WHY the lock refused; NULL when the reason word says it all
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
--
-- `verdict_color IS NULL` fences this compare to the LEGACY rows: a row that
-- already carries the color plane's verdict is answered there and here, and the
-- two answers could disagree. The partition is structural, not a convention.
CREATE VIEW board_drift AS
    SELECT il.src_path, il.seq, il.declared_ref, il.to_path, il.to_sel,
           il.pinned_rev, n.node_rev AS live_rev, 'content-drifted' AS reason
    FROM input_lock il
    JOIN node n ON n.path = il.to_path AND n.selector = il.to_sel
    WHERE il.verdict_color IS NULL
      AND il.pinned_rev IS NOT NULL AND (il.hash_algo IS NULL OR il.hash_algo IN ('node-rev', 'v2'))
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
    WHERE il.verdict_color IS NULL
      AND il.pinned_rev IS NOT NULL AND (il.hash_algo IS NULL OR il.hash_algo IN ('node-rev', 'v2'))
      AND n.path IS NULL;

-- board_red — the union board-red surface: drift + unresolved + the fingerprint
-- plane's reds. Grey (declared-only, pinned_rev NULL) never appears here; green
-- never appears here. A reader renders red iff a row is present, in the DEFAULT
-- face, no pack — so this is the ONE red surface, and a `meridian-lock` pin the
-- color plane measured as drift must be present here too. Absent, a reader
-- asking `board_red` whether anything is red would be told no while `board` and
-- `mrd walk` both say red: the under-report, on the surface most likely to be
-- glanced at rather than read.
CREATE VIEW board_red AS
    SELECT src_path, seq, to_path, to_sel, pinned_rev, live_rev, reason
        FROM board_drift
    UNION ALL
    SELECT src_path, seq, to_path, to_sel, pinned_rev,
           NULL AS live_rev, 'selector-unresolved' AS reason
        FROM board_unresolved
    UNION ALL
    -- the fingerprint plane's reds, carrying the color plane's own reason word
    -- (`content-drifted` / `dangling-anchor` / `selector-unresolved`, never
    -- conflated). No `live_rev`: a fingerprint pin has no node_rev to compare.
    SELECT src_path, seq, to_path, to_sel, pinned_rev,
           NULL AS live_rev, verdict_reason AS reason
        FROM input_lock
        WHERE verdict_color = 'red';

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
--
-- The arms partition `input_lock` on `verdict_color`: a row WITHOUT a verdict is
-- a legacy `^inputs` row, colored by the four node_rev arms below exactly as
-- U5.1 shipped; a row WITH one is a `meridian-lock` row, colored by the ONE
-- color plane. Still exactly one row per lock item, and no item can be colored
-- twice by two compares.
CREATE VIEW board AS
    -- green: pinned NATIVE-algo ({node-rev, v2}) + live rev still equals the
    -- frozen pinned rev. The v1→v2 supersede keeps the node-rev value under the
    -- `v2` label, so it greens through the SAME node_rev compare.
    SELECT il.src_path, il.seq, il.to_path, il.to_sel, il.pinned_rev,
           n.node_rev AS live_rev, 'green' AS color, 'attested' AS reason
        FROM input_lock il
        JOIN node n ON n.path = il.to_path AND n.selector = il.to_sel
        WHERE il.verdict_color IS NULL
          AND il.pinned_rev IS NOT NULL AND (il.hash_algo IS NULL OR il.hash_algo IN ('node-rev', 'v2'))
          AND n.node_rev = il.pinned_rev
    UNION ALL
    -- red: drift (doctored verdict) + unresolved (rename/delete of the pinned
    -- target) + the fingerprint plane's reds — all through the ONE red surface.
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
        WHERE il.verdict_color IS NULL
          AND il.pinned_rev IS NOT NULL AND il.hash_algo IS NOT NULL AND il.hash_algo NOT IN ('node-rev', 'v2')
    UNION ALL
    -- grey: declared-unpinned — the ungated close, never green.
    SELECT il.src_path, il.seq, il.to_path, il.to_sel, il.pinned_rev,
           NULL AS live_rev, 'grey' AS color, 'declared-unpinned' AS reason
        FROM input_lock il
        WHERE il.verdict_color IS NULL AND il.pinned_rev IS NULL
    UNION ALL
    -- the fingerprint plane, NON-red arms: a `meridian-lock` row's green and its
    -- greys (unverifiable-fingerprint / malformed-fingerprint / lock-refused),
    -- read straight off the projected verdict. The reds ride `board_red` above,
    -- so no row is colored twice. Green carries the board's own `attested`
    -- reason word, as every other green row does.
    SELECT il.src_path, il.seq, il.to_path, il.to_sel, il.pinned_rev,
           NULL AS live_rev, il.verdict_color AS color,
           COALESCE(il.verdict_reason, 'attested') AS reason
        FROM input_lock il
        WHERE il.verdict_color IS NOT NULL AND il.verdict_color <> 'red';

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

/// A `meridian-lock` row's identity — the page it is declared on, its ref
/// verbatim, and its pinned CID-token. The key both planes address the row by:
/// a color is a pure function of it (plus the live corpus), so a lookup can
/// never hand a row the wrong verdict.
type PinKey = (String, String, Option<String>);

/// The color of every `meridian-lock` row in `docs`, keyed by [`PinKey`].
///
/// This face computes NO color of its own: it asks [`crate::walk::lock_pin_colors`]
/// — the same `edge_color` a `mrd walk` listing renders — and carries the answer
/// into the SQL row. A fingerprint verdict is a blake3 re-hash of the target's
/// bytes, which this face's SQL cannot express, so projecting the one plane's
/// answer is the only way the board can agree with the walk instead of guessing
/// beside it.
fn pin_verdicts(docs: &BTreeMap<String, Document>) -> BTreeMap<PinKey, Color> {
    walk::lock_pin_colors(docs)
        .into_iter()
        .map(|pin| ((pin.src_path, pin.declared_ref, pin.fingerprint), pin.color))
        .collect()
}

/// Project every page's `^inputs` lock items into `input_lock`. The lock block
/// is a fenced code block whose info string carries the `^inputs` anchor
/// (`` ```yaml ^inputs ``) — parsed here from the vault bytes alone (source 1) —
/// or the engine's own `meridian-lock` block, whose rows additionally carry the
/// color plane's verdict ([`pin_verdicts`]).
///
/// The lock-REFUSAL row projects too. It is a PAGE-level fact, not an `^inputs`
/// edge — no ref, no target, no pinned rev — and before the `verdict_*` columns
/// existed the table could only spell it as a declared-unpinned EDGE, the wrong
/// reason on a row that declares no edge; so it was held back to the color plane
/// alone. It now spells itself: grey `lock-refused`, carrying the refusal. Its
/// `to_path` stays EMPTY, so it is still a leaf no walk traverses and no reverse
/// index reaches.
fn project_input_locks(conn: &Connection, docs: &BTreeMap<String, Document>) -> duckdb::Result<()> {
    let index = corpus_index(docs);
    let verdicts = pin_verdicts(docs);
    let mut stmt = conn.prepare(
        "INSERT INTO input_lock \
         (src_path, seq, declared_ref, to_path, to_sel, pinned_rev, rev_class, hash_algo, src_doc_rev, \
          verdict_color, verdict_reason, verdict_detail) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )?;
    for (path, doc) in docs {
        let src_doc_rev = doc.root.node_rev.0.clone();
        for (seq, item) in page_lock_items_in_corpus(path, doc, &index, docs)
            .into_iter()
            .enumerate()
        {
            // Look the verdict up for exactly the rows the color plane colors —
            // a legacy `^inputs` row carries neither a fingerprint nor a refusal
            // and is answered by the board's node_rev compare instead.
            let verdict = (item.fingerprint.is_some() || item.lock_refusal.is_some())
                .then(|| {
                    verdicts.get(&(
                        path.clone(),
                        item.declared_ref.clone(),
                        item.fingerprint.clone(),
                    ))
                })
                .flatten();
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
                    verdict.map_or(Value::Null, |c| {
                        Value::Text(walk::color_tone(c).to_string())
                    }),
                    verdict
                        .and_then(walk::color_reason)
                        .map_or(Value::Null, |r| Value::Text(r.to_string())),
                    verdict
                        .and_then(walk::color_detail)
                        .map_or(Value::Null, Value::Text),
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
    /// The `ref` field, verbatim (the declared ref). EMPTY on a
    /// [`LockItem::lock_refusal`] row — a refused lock declares no ref, and the
    /// listing names the page from its own context instead.
    pub declared_ref: String,
    /// The `to` page path — the `ref` path when `to` is absent. EMPTY on a
    /// [`LockItem::lock_refusal`] row: a refused lock names no target, so the
    /// row is a leaf the walk never traverses and never reverses into.
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
    /// A form-3 (`meridian-lock`) pin writes no header — the label is derived
    /// from its self-describing token ([`fingerprint_algo`]).
    pub hash_algo: Option<String>,
    /// The `meridian-lock` `fingerprint` — a full `fp1.…` CID-token, verbatim
    /// (`None` for the legacy `^inputs` forms, which pin a `node_rev`). The
    /// typed slot a verdict computer reads: `model::fingerprint::verify_content`
    /// consumes THIS, never a re-parse of the lock block.
    pub fingerprint: Option<String>,
    /// Set on the ONE row a page projects when its `meridian-lock` block
    /// REFUSED to parse ([`lock::LockError`], verbatim): malformed, or more than
    /// one block on the page. Every other field is the empty/absent case — this
    /// row declares no edge, it declares that the page's edges are UNREADABLE.
    ///
    /// It exists so a corrupt lock cannot read as "no pins": before it, a
    /// refusal projected ZERO rows and a damaged page was indistinguishable from
    /// a page that never pinned anything. The row renders grey `lock-refused`
    /// (grey = outside sight), never green and never red.
    pub lock_refusal: Option<String>,
}

/// Parse every `^inputs` lock item declared in `doc`, document order (source 1).
/// The SHARED reader for the board projection ([`project_input_locks`]) and the
/// walk plane ([`crate::walk`]) — one owner for the lock grammar.
///
/// Reads all THREE serializations a corpus page can carry:
/// - **form-1** (legacy, mrd native) — the `^inputs` token is in the fence info
///   (`` ```yaml ^inputs ``), body is `items:` + flow-mappings
///   ([`collect_lock_items`]);
/// - **form-2** (legacy, ratified SCHEMA.md effect-receipt) — a plain
///   `` ```yaml `` block whose TRAILING block-anchor line is `^inputs`, body a
///   block-SEQUENCE of `- ref:/hash:` items plus a bare `hash-algo:` header
///   ([`collect_form2_lock_items`]);
/// - **form-3** (`meridian-lock`, the engine's own lockfile) — the `pins:` plane
///   of the page's ` ```meridian-lock ` block ([`collect_lock_pins`]).
///
/// The three forms are disjoint by construction, so nothing is double-counted:
/// a page carries form-1 or form-2, never both (the form-2 pass skips any block
/// whose fence info already carries `^inputs`), and form-3 is a `meridian-lock`
/// fence that neither `^inputs` pass can match.
#[must_use]
pub fn page_lock_items(doc: &Document) -> Vec<LockItem> {
    let mut out = Vec::new();
    collect_lock_items(&doc.root, &doc.raw, &mut out);
    collect_form2_lock_items(doc, &mut out);
    collect_lock_pins(doc, &mut out);
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
                    fingerprint: None, // a legacy form pins a rev, never a CID-token
                    lock_refusal: None, // only form-3 has a lock block to refuse
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

// ---------------------------------------------------------------------------
// form-3 — the `meridian-lock` block (the engine's own lockfile)
// ---------------------------------------------------------------------------

/// The algo label for a form-3 pin whose token does not parse as a fingerprint.
/// Outside the engine-native set by construction, so an unreadable token can
/// never be compared against a live `node_rev` (no false green, no false red).
const FP_ALGO_UNKNOWN: &str = "fp-unknown";

/// Append the `meridian-lock` `pins:` of `doc` to `out` (block order).
///
/// Form-3 is the ENGINE's own lockfile (`crates/lock`, decision #8), and it
/// shares nothing with the legacy `^inputs` forms: no `^inputs` anchor, no
/// `hash-algo:` header, and its pinned value is a full `fp1.…` CID-token
/// (`docs/norm-v2-spec.md` §2), never a `node_rev`. Before this reader a real
/// meridian-lock pin was a SILENT absence here — the page projected zero rows
/// and read as if it declared no inputs.
///
/// [`lock::find`] is the only parser: one owner for the lock grammar, and the
/// crate's own predicate decides which fence is a lock (never a second
/// language list here). `find` is fail-loud — a malformed block, or more than
/// one on a page, is an `Err`; a READ face cannot refuse, so it projects the
/// refusal as ONE grey row ([`LockItem::lock_refusal`]) rather than swallowing
/// it. Projecting nothing would make a CORRUPT lock read as "no pins" —
/// indistinguishable from a page that never pinned anything, which is the one
/// answer a drift face must never give. The row carries the reason and colors
/// grey `lock-refused` (grey = outside sight); repairing the block is not this
/// face's job.
fn collect_lock_pins(doc: &Document, out: &mut Vec<LockItem>) {
    let found = match lock::find(doc) {
        Ok(Some(found)) => found,
        Ok(None) => return,
        Err(refusal) => {
            out.push(LockItem {
                declared_ref: String::new(),
                to_path: String::new(),
                to_sel: String::new(),
                pinned_rev: None,
                rev_class: None,
                hash_algo: None,
                fingerprint: None,
                lock_refusal: Some(refusal.to_string()),
            });
            return;
        }
    };
    for pin in found.lock.pins {
        let (to_path, to_sel) = split_lock_ref(&pin.declared_ref);
        out.push(LockItem {
            declared_ref: pin.declared_ref,
            to_path,
            to_sel,
            // The pinned value AS WRITTEN — for form-3 that is the token; the
            // typed slot below is what a verdict computer reads.
            pinned_rev: Some(pin.fingerprint.clone()),
            // Structural, not a written field: `pins:` IS the claim plane and
            // `objects:` is the object plane (#8 §2), so a pin is content-class.
            rev_class: Some("content".to_string()),
            hash_algo: Some(fingerprint_algo(&pin.fingerprint)),
            fingerprint: Some(pin.fingerprint),
            lock_refusal: None, // this lock parsed — the refusal row is the Err arm
        });
    }
}

/// The algo label a form-3 row carries: the fingerprint token's own VERSION
/// field (`fp1`). A `meridian-lock` block writes no `hash-algo:` header because
/// the token is self-describing (#4 §2), so the label is derived, never invented.
///
/// Both this label and [`FP_ALGO_UNKNOWN`] sit outside the engine-native set
/// ([`model::is_native_algo`] — `{node-rev, v2}`), and that is now the SAFE
/// FALLBACK rather than the answer: a form-3 row's color comes from its
/// projected verdict on both planes (the walk's `edge_color` routes a
/// fingerprint to `model::selector::classify_pin`; the `board` view reads
/// `verdict_color`). Should a row ever reach SQL without a verdict, the algo
/// label keeps it out of the `node_rev` compare — it renders grey
/// `superseded-algo`, never a false green.
fn fingerprint_algo(token: &str) -> String {
    match model::fingerprint::parse_fingerprint(token) {
        // A hand-written token could spell a version that collides with the
        // native set (`v2.span2.b3.…`). A fingerprint is never node-rev-
        // comparable, so a collision falls back to the unknown label rather
        // than inviting the node-rev compare to call it green.
        Some(parts) if !model::is_native_algo(&parts.version) => parts.version,
        _ => FP_ALGO_UNKNOWN.to_string(),
    }
}

/// Split a `meridian-lock` `ref` into `(to_path, to_sel)` — the SINGLE owner of
/// the form-3 ref grammar (D12).
///
/// Today the grammar is intra-root — `page[#selector]`, the same shape the
/// legacy forms use — so a form-3 row feeds the SAME corpus resolution
/// ([`resolve_to_path`]) and the same `node` join as every other row. The row
/// keeps `declared_ref` VERBATIM, so no consumer re-derives the address: a later
/// `root:` prefix is learned by teaching THIS function to peel a leading root,
/// and nothing else in the reader or the row shape moves.
fn split_lock_ref(declared_ref: &str) -> (String, String) {
    split_selector(declared_ref)
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
        hash_algo: None,    // stamped per-block by parse_lock_body
        fingerprint: None,  // a legacy form pins a rev, never a CID-token
        lock_refusal: None, // only form-3 has a lock block to refuse
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

    // -----------------------------------------------------------------------
    // form-3 — the `meridian-lock` block (S3: the silent absence, closed)
    // -----------------------------------------------------------------------

    /// A page carrying a `meridian-lock` block, rendered by the format's own
    /// sole writer (`lock::render`) so the fixture is the real byte form.
    fn lock_page(pins: &[(&str, &str)]) -> String {
        let mut l = lock::Lock::new();
        l.set_object("sources/target-page.md", "9ae3f1deadbeef");
        for (declared_ref, fingerprint) in pins {
            l.upsert_pin(lock::PinEntry {
                declared_ref: (*declared_ref).to_string(),
                fingerprint: (*fingerprint).to_string(),
            });
        }
        format!(
            "# Effect\n\ndraws from the target\n\n{}\n",
            lock::render(&l)
        )
    }

    /// A full, well-formed CID-token (`fp1.span2.b3.<64hex>`).
    fn fp(nibble: &str) -> String {
        format!("fp1.span2.b3.{}", nibble.repeat(32))
    }

    /// THE regression: a meridian-lock pin projects a row. Before S3 this page
    /// projected ZERO items — a real pin read as "this page declares no inputs"
    /// (a silent absence, `mrd walk` printed `(nothing)` and exited 0/clean).
    #[test]
    fn meridian_lock_pin_is_no_longer_a_silent_absence() {
        let token = fp("ab");
        let items = page_lock_items(&doc(&lock_page(&[(
            "sources/target-page.md#Design",
            &token,
        )])));

        assert_eq!(items.len(), 1, "the lock's `pins:` entry projects a row");
        let item = &items[0];
        assert_eq!(
            item.declared_ref, "sources/target-page.md#Design",
            "the declared ref is carried VERBATIM (D12: no consumer re-derives it)",
        );
        assert_eq!(item.to_path, "sources/target-page.md");
        assert_eq!(
            item.to_sel, "Design",
            "the selector grain survives the split"
        );
        assert_eq!(
            item.fingerprint.as_deref(),
            Some(token.as_str()),
            "the CID-token rides its own typed slot, ready for a verdict",
        );
        assert_eq!(item.pinned_rev.as_deref(), Some(token.as_str()));
        assert_eq!(
            item.rev_class.as_deref(),
            Some("content"),
            "pins: is the claim plane"
        );
        assert_eq!(
            item.hash_algo.as_deref(),
            Some("fp1"),
            "the algo label is DERIVED from the self-describing token",
        );
    }

    /// The board sees it AND judges it: a meridian-lock-only page projects an
    /// `input_lock` row carrying the color plane's verdict, and the board renders
    /// that verdict. `fp("cd")` is a well-formed token holding the WRONG digest,
    /// so the honest answer is `red content-drifted` — and the same pin at the
    /// target's LIVE token is green, proving the red is measured, not blanket.
    ///
    /// SUPERSEDES S3's `lock_only_page_is_visible_to_the_board_as_grey`, which
    /// asserted `grey superseded-algo` and `board_red` empty. That was honest
    /// while `input_lock` had no verdict column — the board could see the pin but
    /// not judge it — and dishonest the moment the walk plane could say red
    /// (S9): two planes, two colors, one question.
    #[test]
    fn lock_only_page_renders_its_pin_verdict_on_the_board() {
        let target = "# Target\n\nbody\n";
        let mut docs = BTreeMap::new();
        docs.insert(
            "effect.md".to_string(),
            doc(&lock_page(&[("sources/target-page.md", &fp("cd"))])),
        );
        docs.insert("sources/target-page.md".to_string(), doc(target));
        let conn = open_board(&docs, &[]).expect("open board");

        let n: i64 = conn
            .query_row(
                "SELECT count(*) FROM input_lock WHERE src_path='effect.md'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "the lock pin projects one input_lock row (pre-S3: 0)");

        let (color, reason): (String, String) = conn
            .query_row(
                "SELECT color, reason FROM board WHERE src_path='effect.md'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            (color.as_str(), reason.as_str()),
            ("red", "content-drifted"),
            "the wrong digest is measured drift — the same word `mrd walk` prints",
        );
        // The ONE red surface agrees: a reader asking board_red "is anything
        // red?" is not told no while `board` says red.
        let reds: i64 = conn
            .query_row(
                "SELECT count(*) FROM board_red WHERE src_path='effect.md' AND reason='content-drifted'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(reds, 1, "a measured fingerprint drift IS a board red");

        // The same pin at the target's LIVE token greens — the red above is a
        // measurement, never a blanket verdict on the form.
        let live = model::fingerprint::fingerprint(&doc(target), &doc(target).root).0;
        let mut green_docs = BTreeMap::new();
        green_docs.insert(
            "effect.md".to_string(),
            doc(&lock_page(&[("sources/target-page.md", &live)])),
        );
        green_docs.insert("sources/target-page.md".to_string(), doc(target));
        let gconn = open_board(&green_docs, &[]).expect("open board green");
        let (gcolor, greason): (String, String) = gconn
            .query_row(
                "SELECT color, reason FROM board WHERE src_path='effect.md'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((gcolor.as_str(), greason.as_str()), ("green", "attested"));
        let greds: i64 = gconn
            .query_row("SELECT count(*) FROM board_red", [], |r| r.get(0))
            .unwrap();
        assert_eq!(greds, 0, "a verifying pin is never a board red");
    }

    /// The three forms stay separated: a legacy row never grows a fingerprint,
    /// and a lock page never grows a legacy `rev` from thin air. A page carrying
    /// BOTH a legacy block and a lock block projects each exactly once.
    #[test]
    fn legacy_and_lock_forms_do_not_bleed_into_each_other() {
        let legacy = "# Doc\n\n```yaml ^inputs\nhash-algo: node-rev\nitems:\n  - {ref: 'a.md', rev: 'deadbeef'}\n```\n";
        let legacy_items = page_lock_items(&doc(legacy));
        assert_eq!(legacy_items.len(), 1);
        assert!(
            legacy_items[0].fingerprint.is_none(),
            "a legacy `^inputs` row pins a rev, never a CID-token",
        );
        assert_eq!(legacy_items[0].pinned_rev.as_deref(), Some("deadbeef"));

        let both = format!(
            "{legacy}\n{}\n",
            lock::render(&{
                let mut l = lock::Lock::new();
                l.upsert_pin(lock::PinEntry {
                    declared_ref: "b.md".to_string(),
                    fingerprint: fp("ef"),
                });
                l
            })
        );
        let items = page_lock_items(&doc(&both));
        assert_eq!(items.len(), 2, "one legacy item + one lock pin, each once");
        assert_eq!(items[0].declared_ref, "a.md");
        assert!(items[0].fingerprint.is_none());
        assert_eq!(items[1].declared_ref, "b.md");
        assert_eq!(items[1].fingerprint.as_deref(), Some(fp("ef").as_str()));
    }

    /// A lock block the format REFUSES (malformed, or two blocks on one page)
    /// projects ONE row carrying the refusal — never a pin guessed out of
    /// damage, and never NOTHING. `lock::find` stays the one parser and the one
    /// judge of the bytes; this face only reports its verdict.
    ///
    /// SUPERSEDES S3's `refused_lock_block_projects_no_pin_rows`: projecting
    /// zero rows made a CORRUPT lock byte-identical, to every reader, to a page
    /// that never pinned anything. The row exists so damage is visible; it
    /// carries no ref, no target and no rev, so it can never be read as an edge.
    #[test]
    fn refused_lock_block_projects_one_refusal_row_not_silence() {
        let malformed = "# A\n\n```meridian-lock\nversion: 1\ngarbage here\n```\n";
        assert!(
            lock::find(&doc(malformed)).is_err(),
            "precondition: the format refuses these bytes",
        );
        let items = page_lock_items(&doc(malformed));
        assert_eq!(items.len(), 1, "the refusal is visible, not absent");
        assert_eq!(
            items[0].lock_refusal.as_deref(),
            Some(
                "malformed at line 3: unrecognized line (canonical order: version, objects, pins)"
            ),
            "the row carries WHY the lock is unreadable",
        );
        assert_eq!(items[0].declared_ref, "", "a refusal declares no ref");
        assert_eq!(items[0].to_path, "", "a refusal names no target");
        assert!(items[0].pinned_rev.is_none() && items[0].fingerprint.is_none());

        let block = lock::render(&{
            let mut l = lock::Lock::new();
            l.upsert_pin(lock::PinEntry {
                declared_ref: "b.md".to_string(),
                fingerprint: fp("12"),
            });
            l
        });
        let two = format!("# A\n\n{block}\n\n{block}\n");
        assert_eq!(
            lock::find(&doc(&two)),
            Err(lock::LockError::MultipleBlocks),
            "precondition: two blocks is corruption, not two locks",
        );
        let items = page_lock_items(&doc(&two));
        assert_eq!(
            items.len(),
            1,
            "two blocks project ONE refusal, not two pins"
        );
        assert_eq!(
            items[0].lock_refusal.as_deref(),
            Some("more than one meridian-lock block on the page"),
        );
    }

    /// The refusal row reaches the SQL board and spells itself correctly: grey
    /// `lock-refused`, carrying WHY, with no ref and no target — so it is
    /// visible without ever being readable as an edge. It is NOT a board red
    /// (nothing was measured) and NOT `declared-unpinned` (the page did not
    /// decline to pin; its pins are unreadable).
    ///
    /// SUPERSEDES S3's `refusal_row_is_not_projected_into_the_input_lock_table`,
    /// which asserted zero rows. Holding the row back was right while the table
    /// could only spell it as a declared-unpinned EDGE — the wrong reason. With
    /// `verdict_*` the row spells itself, and zero rows would make a CORRUPT
    /// lock byte-identical, on the board, to a page that never pinned anything.
    #[test]
    fn refusal_row_projects_one_grey_lock_refused_row() {
        let malformed = "# A\n\n```meridian-lock\nversion: 1\ngarbage here\n```\n";
        let mut docs = BTreeMap::new();
        docs.insert("a.md".to_string(), doc(malformed));
        let conn = open_board(&docs, &[]).expect("board");

        let (declared_ref, to_path, color, reason, detail): (
            String,
            String,
            String,
            String,
            String,
        ) = conn
            .query_row(
                "SELECT declared_ref, to_path, verdict_color, verdict_reason, verdict_detail \
                 FROM input_lock",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .expect("exactly one refusal row");
        assert_eq!(declared_ref, "", "a refusal declares no ref");
        assert_eq!(to_path, "", "a refusal names no target — never a self-edge");
        assert_eq!((color.as_str(), reason.as_str()), ("grey", "lock-refused"));
        assert_eq!(
            detail,
            "malformed at line 3: unrecognized line (canonical order: version, objects, pins)",
            "the board carries WHY, the same words the walk plane prints",
        );

        let (bcolor, breason): (String, String) = conn
            .query_row("SELECT color, reason FROM board", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .expect("exactly one board row");
        assert_eq!(
            (bcolor.as_str(), breason.as_str()),
            ("grey", "lock-refused"),
            "never `declared-unpinned` — the page's pins are unreadable, not absent",
        );
        let reds: i64 = conn
            .query_row("SELECT count(*) FROM board_red", [], |r| r.get(0))
            .expect("count");
        assert_eq!(reds, 0, "an unreadable lock measures nothing — never red");
    }

    /// The algo label is derived, and it can never collide with the native set:
    /// an unparseable token, and a hand-written token spelling a native version,
    /// both fall back to the unknown label. Either way the row stays outside the
    /// `node_rev` compare — no false green.
    #[test]
    fn derived_algo_label_never_collides_with_the_native_set() {
        assert_eq!(fingerprint_algo(&fp("ab")), "fp1");
        assert_eq!(fingerprint_algo("not-a-token"), FP_ALGO_UNKNOWN);
        assert_eq!(
            fingerprint_algo(&format!("v2.span2.b3.{}", "ab".repeat(32))),
            FP_ALGO_UNKNOWN,
            "a token spelling a native version must not buy the node-rev compare",
        );
        for algo in [fingerprint_algo(&fp("ab")), FP_ALGO_UNKNOWN.to_string()] {
            assert!(
                !model::is_native_algo(&algo),
                "{algo} must stay outside the native set",
            );
        }
    }

    /// D12: the ref grammar has ONE owner, and the row carries the ref verbatim,
    /// so a later `root:` prefix is a change to that one function — the reader,
    /// the row shape, and the resolution path are untouched by it. Today the
    /// grammar is intra-root: `page`, `page#heading`, `page#^block-id`.
    #[test]
    fn lock_ref_grammar_has_one_owner_and_is_root_prefix_learnable() {
        assert_eq!(
            split_lock_ref("wiki/page.md"),
            ("wiki/page.md".to_string(), String::new()),
        );
        assert_eq!(
            split_lock_ref("wiki/page.md#Design"),
            ("wiki/page.md".to_string(), "Design".to_string()),
        );
        assert_eq!(
            split_lock_ref("wiki/page.md#^claim-1"),
            ("wiki/page.md".to_string(), "^claim-1".to_string()),
            "a block-id keeps its caret — it is the `node.selector` verbatim",
        );
        // The unpeeled `root:` prefix stays INSIDE the address today (stage 2 is
        // intra-root): it is carried, never silently reinterpreted as a path.
        let items = page_lock_items(&doc(&lock_page(&[("sessions:notes.md#Design", &fp("34"))])));
        assert_eq!(items[0].declared_ref, "sessions:notes.md#Design");
    }
}
