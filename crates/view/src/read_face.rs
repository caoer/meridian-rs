//! U2.9 — the C1 board-red projections + the locked read face (design 2 §2.1 /
//! §2.3 / §5.3; plan §3 decision C1 = YES; advisor checklist item 24).
//!
//! Three things land here, all mounted OVER U2.1's schema-v2 fact tables
//! ([`crate::facts`]) — never a fork of that contract:
//!
//! 1. **The pin projection** — a pure parse (source 1) of each page's declared
//!    pins into [`input_lock`](READ_FACE_SCHEMA_SQL) rows. ONE form: the
//!    engine's own `meridian-lock` block (see [`page_lock_items`]). The two
//!    legacy `^inputs` serializations were retired with the vocabulary (R1.3,
//!    amending 9.6).
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
//! One default-face view rides the locked read face (no face widening — the
//! blocked-`ATTACH` guard still holds with it loaded):
//!
//! - **`board`** — the colors layer: exactly one `green`/`red`/`grey` row per
//!   lock edge. Under R4 every row carries the color plane's verdict, so the
//!   board reads that verdict; the `node_rev` compare arms below it are
//!   unreachable and are the next unit's to collapse.
//!
//!   "An ungated close renders grey, never green" (d2 §5.3) still holds, and now
//!   holds BY CONSTRUCTION rather than by detection: green is COMPUTED from a
//!   fingerprint recomputed against live content, never granted by a row
//!   existing, so no row shape earns green without the content matching.
//!
//! A second view, `co_edit_trace`, carried the traces layer over the reserved
//! journal's mechanical write-facts. It died with the journal (ZT 2026-08-02,
//! remove-no-replacement): its every column came from `receipt_journal`, so with
//! that table gone there is no trace to project, and a co-edit is no longer
//! visible convention-free.

use std::collections::BTreeMap;

use duckdb::Connection;
use duckdb::types::Value;
use model::selector::Color;
use model::{CorpusIndex, Document};

use crate::ViewError;
use crate::facts;
use crate::walk;

/// The additive read-face DDL: the lock-pin parse projection + the board-red
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
-- NOTE (R1.3): the legacy `^inputs` plane is retired, so every projected row now
-- carries a verdict and the arms below fenced by `verdict_color IS NULL` — the
-- node_rev compare, board_drift, board_unresolved, and the green /
-- superseded-algo / declared-unpinned board arms — are UNREACHABLE. They are
-- left standing here deliberately: collapsing them changes what the board
-- SURFACE means, which is a separate unit's call, not a consequence of retiring
-- a parser. The comments below still describe the SQL as written.
--
-- input_lock — the parse projection of each page's `meridian-lock` block. One row
-- per lock item, exactly as written in the vault bytes (source 1). Distinct from
-- `edge` (owned by pin: the manifest LEFT-joined with live resolution). Every
-- row carries the containing page's doc_rev — the rev-compare invalidation key.
-- A `meridian-lock` (form-3) row additionally carries the fingerprint VERDICT
-- the color plane computed for it, so the board renders the SAME color the walk
-- renders for the same pin — one question, one answer, on both planes.
CREATE TABLE input_lock (
    src_path     TEXT     NOT NULL,   -- [1] page whose lock block declares the item
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
-- colors-amendment § Colors). Exactly ONE color row per lock edge, in the
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

/// Build a locked, board-ready read face over `docs`: project `node` (via
/// [`crate::facts`]), project the lock blocks into `input_lock`, create
/// the board-red views, then LOCK the face. The returned connection serves the
/// default-face board queries and **refuses `ATTACH`/`COPY`/external access**
/// (the C1 read-face capability).
///
/// # Errors
/// [`ViewError::Duckdb`] on any schema-creation, projection, or lock failure.
pub fn open_board(docs: &BTreeMap<String, Document>) -> Result<Connection, ViewError> {
    let conn = Connection::open_in_memory()?;
    facts::create_facts_schema(&conn)?;
    facts::project_nodes(&conn, docs)?;
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
// the lock-pin parse projection
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

/// Project every page's lock pins into `input_lock` — the `pins:` plane of the
/// page's own `meridian-lock` block, parsed from the vault bytes alone
/// (source 1), each row carrying the color plane's verdict ([`pin_verdicts`]).
///
/// The lock-REFUSAL row projects too. It is a PAGE-level fact, not a pin
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
            // Every R4 row carries a fingerprint or a refusal, so the guard is
            // now a belt-and-braces read rather than a partition; the arms it
            // used to fence off are the next unit's to collapse.
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

/// One parsed lock item (source 1). Passengers the engine ignores
/// (`claim`, `at:`) are not carried — only the columns board reds and the walk
/// plane compute on.
///
/// Public because the walk plane ([`crate::walk`]) consumes the SAME parser this
/// board projection uses: one owner for the lock grammar (design "one
/// owner per fact"), never a second reader that could drift.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockItem {
    /// The `ref` field, verbatim (the declared ref) — the bytes the projection
    /// writes and a verdict joins on. EMPTY on a [`LockItem::lock_refusal`] row
    /// — a refused lock declares no ref, and the listing names the page from
    /// its own context instead.
    pub declared_ref: String,
    /// The same ref, PARSED ([`addr::Addr`]) — **the structural owner** (U10).
    ///
    /// Every consumer that needs the root, the path or the selector reads THIS;
    /// nothing re-splits [`LockItem::declared_ref`]. `None` means there is no
    /// address to read: a refusal row (which declares no ref), or a spelling
    /// outside the address grammar (`docs/address-grammar.md` § 4). The option
    /// is what makes "not an address" impossible to mistake for one.
    pub declared_addr: Option<addr::Addr>,
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
    /// The `meridian-lock` `fingerprint` — a full `fp1.…` CID-token, verbatim.
    /// The typed slot a verdict computer reads:
    /// `model::fingerprint::verify_content` consumes THIS, never a re-parse of
    /// the lock block.
    pub fingerprint: Option<String>,
    /// **R4's structural selector — `path` XOR `properties`, verbatim.**
    ///
    /// This is the STRUCTURAL owner of the claim's address, and it exists because
    /// [`LockItem::to_sel`] cannot be one. `to_sel` is the `/`-joined display
    /// spelling, and joining is LOSSY in exactly the case R4's array was chosen
    /// to carry: a heading whose own text contains `/` (`["A/B", "leaf"]`) joins
    /// to `A/B/leaf` and re-splits into three segments naming a section that does
    /// not exist. Every verdict computer reads THIS and re-splits nothing.
    ///
    /// `None` only on a [`LockItem::lock_refusal`] row, which declares no claim.
    pub selector: Option<lock::Selector>,
    /// R4's `object` — the wiki-link inner text, verbatim, the target's vault
    /// path WITHOUT `.md`. Empty on a [`LockItem::lock_refusal`] row.
    ///
    /// Carried beside [`LockItem::to_path`] rather than re-derived from it
    /// because the transcript class needs it: a `session#seq-N` ref's session id
    /// is the OBJECT, and re-deriving it by stripping `.md` from `to_path` is the
    /// string surgery R4's structure exists to end.
    pub object: String,
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
    /// The canonical root this edge RESOLVED INTO, when it resolved into a
    /// mounted root rather than the ambient one. `None` = the ambient root (the
    /// majority case, unchanged).
    ///
    /// [`LockItem::to_path`] is the path INSIDE that root, so a consumer
    /// fetching bytes must read this to know which corpus to fetch from —
    /// resolving `to_path` against the ambient corpus is exactly FINDING 03's
    /// wrong-bytes success.
    pub to_root: Option<addr::MountName>,
    /// Set when this edge's ROOT could not be resolved to a readable corpus —
    /// carrying the computed grey reason itself, not merely the fact that
    /// something was wrong.
    ///
    /// **It carries the REASON because the causes must stay distinct (S3-R50).**
    /// A root nothing declares and a root that is declared but unreadable are
    /// different facts with different fixes, and a carrier that recorded only
    /// "unresolved" would force the renderer to guess which — which is how one
    /// of them ended up prescribing an action the user had already taken.
    ///
    /// Distinct from [`LockItem::lock_refusal`]: a refused lock is unreadable
    /// HERE, an unresolvable root is unreachable FROM here.
    pub root_refusal: Option<model::selector::GreyReason>,
}

/// Parse every lock pin declared in `doc`, document order (source 1). The SHARED
/// reader for the board projection ([`project_input_locks`]) and the walk plane
/// ([`crate::walk`]) — one owner for the lock grammar.
///
/// **One form, not three.** A page's pins are the `pins:` plane of its
/// ` ```meridian-lock ` block ([`collect_lock_pins`]) and nothing else. The two
/// legacy `^inputs` serializations this reader also carried — form-1 (the
/// `^inputs` fence-info token, `items:` + flow-mappings) and form-2 (a plain
/// yaml block trailed by an `^inputs` block-anchor, `- ref:`/`hash:` rows under a
/// `hash-algo:` header) — are **retired**: R1.3 kills `inputs` as vocabulary AND
/// as storage key, amending 9.6's storage-key clause.
///
/// With one form there is no disjointness to maintain. The double-count the old
/// reader guarded against — a `meridian-lock` fence trailed by an `^inputs`
/// anchor, projected once per pass with two different verdicts — is not fixed
/// here, it is unrepresentable: only one pass remains.
#[must_use]
pub fn page_lock_items(doc: &Document) -> Vec<LockItem> {
    let mut out = Vec::new();
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

/// Parse `doc`'s lock items, then RESOLVE each item's `to_path` against
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
    page_lock_items_in_rooted_corpus(
        src_path,
        doc,
        index,
        &model::RootedCorpus::ambient(docs),
        &addr::MountSet::default(),
    )
}

/// [`page_lock_items_in_corpus`] against a ROOT-KEYED corpus and a mount table —
/// **resolution is a mount lookup** (U11).
///
/// Each item's `to_path` is resolved through the one address owner, and the
/// outcome is recorded STRUCTURALLY rather than folded into the path string:
///
/// - resolved in a mounted root → `to_root` names it, `to_path` is the path
///   inside THAT root;
/// - the named root is not bound → `unmounted` names it and the row renders grey;
/// - anything else → the ambient behaviour, unchanged.
///
/// The unresolvable-ref fallback is unchanged: the spelling comes back verbatim
/// and the edge renders red `selector-unresolved`. **An unmounted root does NOT
/// take that path** — that is the conflation this function exists to prevent.
#[must_use]
pub fn page_lock_items_in_rooted_corpus(
    src_path: &str,
    doc: &Document,
    index: &CorpusIndex,
    corpus: &model::RootedCorpus<'_>,
    mounts: &addr::MountSet,
) -> Vec<LockItem> {
    let mut items = page_lock_items(doc);
    for item in &mut items {
        match index.resolve_ref(&item.to_path, src_path, corpus, mounts) {
            model::RefResolution::Ambient(path) => item.to_path = path,
            model::RefResolution::Rooted { root, path } => {
                item.to_root = Some(root);
                item.to_path = path;
            }
            model::RefResolution::Unmounted(root) => {
                item.root_refusal = Some(model::selector::GreyReason::Unmounted { root });
            }
            model::RefResolution::PathUnseeable { root, path, detail } => {
                item.root_refusal =
                    Some(model::selector::GreyReason::PathUnseeable { root, path, detail });
            }
            // Unresolved and malformed both keep the declared spelling, which is
            // what the red `selector-unresolved` render reports.
            model::RefResolution::NotFound | model::RefResolution::Malformed(_) => {}
        }
    }
    items
}

/// Resolve a lock item's `to_path` to a real corpus path through the ONE address
/// owner ([`CorpusIndex::resolve_ref`]) — exact key, then key + `.md`, then
/// `getFirstLinkpathDest` parity. This function holds no precedence of its own:
/// a second copy of the address grammar is how the pin plane and the decoration
/// plane came to hash two different documents for one ref.
///
/// An unresolvable ref returns its bare input unchanged — unresolved is
/// first-class (the edge then renders red `selector-unresolved`, never a false
/// green), exactly as a genuinely missing target should. That fallback is the
/// only thing this wrapper adds: the read face reports the spelling it could not
/// place, where a caller needing the absence itself reads the `Option`.
///
/// Public so the U3.4 supersede tool resolves a ref to the SAME path this reader
/// does — no reader/writer drift on which node a `[[ref]]` names (the swept
/// `node_rev` must match what the board reads).
#[must_use]
pub fn resolve_to_path(
    to_path: &str,
    src_path: &str,
    index: &CorpusIndex,
    docs: &BTreeMap<String, Document>,
) -> String {
    resolve_to_path_rooted(
        to_path,
        src_path,
        index,
        &model::RootedCorpus::ambient(docs),
        &addr::MountSet::default(),
    )
}

/// [`resolve_to_path`] against a ROOT-KEYED corpus and a mount table — the
/// cross-root form, and the one the walk plane uses.
///
/// The bare-input fallback is unchanged and stays deliberate: an unresolvable
/// ref reports the spelling it could not place. **A `root:`-bearing ref to an
/// UNMOUNTED root therefore reports its address verbatim rather than the ambient
/// root's same-basename file** — the caller distinguishes the two by asking
/// [`model::CorpusIndex::resolve_ref`] directly, which is what
/// `walk::steps_from` does to reach the grey.
#[must_use]
pub fn resolve_to_path_rooted(
    to_path: &str,
    src_path: &str,
    index: &CorpusIndex,
    corpus: &model::RootedCorpus<'_>,
    mounts: &addr::MountSet,
) -> String {
    index
        .resolve_ref(to_path, src_path, corpus, mounts)
        .path()
        .map_or_else(|| to_path.to_string(), str::to_owned)
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
/// shared nothing with the retired `^inputs` forms: no `^inputs` anchor, no
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
                declared_addr: None,
                to_path: String::new(),
                to_sel: String::new(),
                pinned_rev: None,
                rev_class: None,
                hash_algo: None,
                fingerprint: None,
                selector: None,
                object: String::new(),
                lock_refusal: Some(refusal.to_string()),
                // Set by resolution, never by the parser: a DECLARED ref carries
                // its root in the spelling, and which root it RESOLVED into is a
                // mount-table fact this seam has not consulted yet.
                to_root: None,
                root_refusal: None,
            });
            return;
        }
    };
    for pin in found.lock.pins {
        // **The one-time conversion door, read side.** R4 carries STRUCTURE —
        // `object` plus a selector ARRAY — and the joined `page#A/B` spelling
        // exists only for humans and the wire echo. So the joined form is minted
        // HERE, once, for display and for the address-grammar consumers that
        // still need it; every verdict computer reads `selector` instead and
        // re-splits nothing. This is U8's `pin_row` seam in reverse.
        let to_path = format!("{}.md", pin.object);
        let to_sel = display_selector(&pin.selector);
        let declared_ref = if to_sel.is_empty() {
            to_path.clone()
        } else {
            format!("{to_path}#{to_sel}")
        };
        out.push(LockItem {
            declared_addr: addr::Addr::parse(&declared_ref).ok(),
            declared_ref,
            to_path,
            to_sel,
            // The pinned value AS WRITTEN — the token; the typed slot below is
            // what a verdict computer reads.
            pinned_rev: Some(pin.fingerprint.clone()),
            // Structural, not a written field: `pins:` IS the claim plane, so a
            // pin is content-class.
            rev_class: Some("content".to_string()),
            hash_algo: Some(fingerprint_algo(&pin.fingerprint)),
            fingerprint: Some(pin.fingerprint),
            selector: Some(pin.selector),
            object: pin.object,
            lock_refusal: None, // this lock parsed — the refusal row is the Err arm
            // Set by resolution, never by the parser: a DECLARED ref carries
            // its root in the spelling, and which root it RESOLVED into is a
            // mount-table fact this seam has not consulted yet.
            to_root: None,
            root_refusal: None,
        });
    }
}

/// The `/`-joined DISPLAY spelling of an R4 selector — the human plane only.
///
/// `Path([])` is the whole body and spells empty (the page root), `Path(["^id"])`
/// spells the anchor, and a heading array joins on `/`. A `properties` selector
/// has no fragment spelling in the address grammar at all — the frontmatter is
/// not a body node — so it spells empty too, and the projection's `to_path`
/// carries the whole claim.
///
/// **This is lossy on purpose and must never be re-split.** A heading containing
/// `/` joins ambiguously here; [`LockItem::selector`] is the structural answer
/// and is what every verdict computer reads.
fn display_selector(selector: &lock::Selector) -> String {
    match selector {
        lock::Selector::Path(segments) => segments.join("/"),
        lock::Selector::Properties(_) => String::new(),
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

    /// The R4 blob hash every fixture pin carries. R4 makes `hash` MANDATORY —
    /// *"if hash is missing, we lost the explicit target meaning"* — so there is
    /// no fixture pin without one, and the retired `objects:` table these
    /// fixtures used to set has no successor to set.
    const FIXTURE_HASH: &str = "9ae3f1deadbeef";

    /// A lock page carrying one R4 pin per `(ref-spelling, fingerprint)` pair.
    ///
    /// The spelling is a CONVENIENCE for the fixture only — it is split into the
    /// `object` and the selector ARRAY here, at the fixture's own door, exactly
    /// as the production read door mints the joined form. Nothing downstream sees
    /// a joined string.
    fn lock_page(pins: &[(&str, &str)]) -> String {
        let mut l = lock::Lock::new();
        for (spelling, fingerprint) in pins {
            let (target, fragment) = match spelling.split_once('#') {
                Some((t, f)) => (t, f),
                None => (*spelling, ""),
            };
            let object = target.strip_suffix(".md").unwrap_or(target);
            let selector = if fragment.is_empty() {
                lock::Selector::Path(Vec::new())
            } else {
                lock::Selector::Path(fragment.split('/').map(str::to_string).collect())
            };
            l.upsert_pin(lock::PinEntry::new(
                object,
                FIXTURE_HASH,
                selector,
                fingerprint,
            ));
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
        let conn = open_board(&docs).expect("open board");

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
        let live = model::fingerprint::fingerprint(&doc(target), &doc(target).root)
            .expect("the fixture target has content")
            .into_string();
        let mut green_docs = BTreeMap::new();
        green_docs.insert(
            "effect.md".to_string(),
            doc(&lock_page(&[("sources/target-page.md", &live)])),
        );
        green_docs.insert("sources/target-page.md".to_string(), doc(target));
        let gconn = open_board(&green_docs).expect("open board green");
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

    #[test]
    fn refused_lock_block_projects_one_refusal_row_not_silence() {
        let malformed = "# A\n\n```meridian-lock\nversion: 2\ngarbage here\n```\n";
        // The precondition names WHICH refusal. `is_err()` could not tell the
        // MALFORMED rule from the version gate — and a pre-R4 `version: 1`
        // fixture refuses on VERSION, which is exactly the drift that would
        // leave the assertion below measuring a refusal it never named.
        //
        // The reason STRING is deliberately not repeated here: the assertion
        // immediately below already pins it in full, and a second spelling is a
        // second thing to drift. This pins the variant and the LINE.
        assert!(
            matches!(
                lock::find(&doc(malformed)),
                Err(lock::LockError::Malformed { line: 3, .. })
            ),
            "precondition: the fixture must reach the MALFORMED rule at line 3, \
             not the version gate — got {:?}",
            lock::find(&doc(malformed)),
        );
        let items = page_lock_items(&doc(malformed));
        assert_eq!(items.len(), 1, "the refusal is visible, not absent");
        assert_eq!(
            items[0].lock_refusal.as_deref(),
            Some("malformed at line 3: unrecognized line (canonical order: version, pins)"),
            "the row carries WHY the lock is unreadable",
        );
        assert_eq!(items[0].declared_ref, "", "a refusal declares no ref");
        assert_eq!(items[0].to_path, "", "a refusal names no target");
        assert!(items[0].pinned_rev.is_none() && items[0].fingerprint.is_none());

        let block = lock::render(&{
            let mut l = lock::Lock::new();
            l.upsert_pin(lock::PinEntry::new(
                "b",
                FIXTURE_HASH,
                lock::Selector::Path(Vec::new()),
                &fp("12"),
            ));
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
        let malformed = "# A\n\n```meridian-lock\nversion: 2\ngarbage here\n```\n";
        let mut docs = BTreeMap::new();
        docs.insert("a.md".to_string(), doc(malformed));
        let conn = open_board(&docs).expect("board");

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
            detail, "malformed at line 3: unrecognized line (canonical order: version, pins)",
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

    /// **NARROWED (U10).** This test was named
    /// `lock_ref_grammar_has_one_owner_and_is_root_prefix_learnable` and its
    /// comment claimed a later `root:` prefix would be *"a change to that one
    /// function"*. FINDING 02 measured sixteen sites re-splitting the same
    /// string, and this test never proved otherwise: it exercised exactly one
    /// other function, asserted only `declared_ref` for its colon-bearing
    /// fixture, and **never `to_path`** — had it asserted `to_path` it would
    /// have printed the literal-path misreading and contradicted its own
    /// comment. "ONE owner" was not tested at all.
    ///
    /// What is now true, and what this asserts: the address is a TYPE parsed at
    /// the ingress, so the root is STRUCTURAL rather than glued to a path, and
    /// the projection columns are byte-identical to what the string convention
    /// produced. The claim is exactly the size of its proof.
    #[test]
    fn a_lock_ref_is_a_parsed_address_and_the_projection_is_byte_identical() {
        // The projection columns, minted from R4's STRUCTURE at the read door —
        // what `split_address` used to do by string surgery over a joined
        // spelling that no longer exists on the pin row.
        for (spelling, want_path, want_sel) in [
            ("wiki/page.md", "wiki/page.md", ""),
            ("wiki/page.md#Design", "wiki/page.md", "Design"),
            // A block-id keeps its caret — it is the `node.selector` verbatim.
            ("wiki/page.md#^claim-1", "wiki/page.md", "^claim-1"),
        ] {
            let items = page_lock_items(&doc(&lock_page(&[(spelling, &fp("34"))])));
            assert_eq!(
                (items[0].to_path.as_str(), items[0].to_sel.as_str()),
                (want_path, want_sel),
                "{spelling}",
            );
        }

        // THE assertion the old test omitted. The row carries the ref verbatim
        // AND `to_path` — and `to_path` still prints the whole spelling, root
        // included. Peeling it here would resolve a cross-root ref onto the
        // ambient root's same-basename file, which is FINDING 03. U11 owns the
        // root-keyed lookup that makes peeling safe.
        let items = page_lock_items(&doc(&lock_page(&[("sessions:notes.md#Design", &fp("34"))])));
        assert_eq!(items[0].declared_ref, "sessions:notes.md#Design");
        assert_eq!(
            items[0].to_path, "sessions:notes.md",
            "the root stays ON the spelling until the lookup is root-aware",
        );
        assert_eq!(items[0].to_sel, "Design");

        // And the root is now readable as a VALUE — the thing sixteen sites
        // could not do, and the reason this unit exists.
        let root = items[0]
            .declared_addr
            .as_ref()
            .and_then(addr::Addr::root)
            .map(addr::MountName::as_str);
        assert_eq!(
            root,
            Some("sessions"),
            "the root is structural, not textual"
        );
    }
}
