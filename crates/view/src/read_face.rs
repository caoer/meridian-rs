//! C1 board-red projections + locked read face (d2 §2.1 / §2.3 / §5.3), mounted
//! over [`crate::facts`] — never a fork of that contract.
//!
//! 1. **Pin projection** — pure parse of each page's `meridian-lock` into
//!    `input_lock` rows ([`page_lock_items`]). Every row carries `src_doc_rev`
//!    (cache self-invalidates by `doc_rev`; §2.1 / §8).
//! 2. **`board_red`** — the one red view, reading the colour plane's reds from
//!    the projected verdict. Default face, no optional pack. Legacy
//!    `board_drift` / `board_unresolved` retired with the `node_rev` compare.
//! 3. **[`lock_read_face`]** — `enable_external_access=false` +
//!    `lock_configuration=true`: SQL packs have no write path and cannot
//!    re-raise it (A10).
//!
//! `edge`/`claim` are not populated here.
//!
//! # Board colors + residue (d2 §5.3)
//! - **`board`** — one `green`/`red`/`grey` row per lock edge. Under R4 every
//!   row carries a verdict; the board reads it and computes none of its own.
//! - **`board_residue`** — rows `board` cannot colour (NULL verdict matches no
//!   arm). Count disclosed beside every board answer. Sits at zero with a test
//!   that it can move.
//!
//! "Ungated close renders grey, never green" holds by construction: green is
//! computed from a live fingerprint recompute, never granted by row existence.

use std::collections::BTreeMap;

use duckdb::Connection;
use duckdb::types::Value;
use model::selector::Color;
use model::{CorpusIndex, Document};

use crate::ViewError;
use crate::facts;
use crate::walk;

/// Additive read-face DDL (`input_lock` + board views) over frozen
/// [`crate::facts`]. Address columns are source-1 (parse). `verdict_*` are the
/// named exception: fingerprint colour is a blake3 re-hash SQL cannot express,
/// so [`crate::walk::lock_pin_colors`] computes it and [`project_input_locks`]
/// projects the answer — one colour plane, built per query and discarded.
pub const READ_FACE_SCHEMA_SQL: &str = r"
-- NOTE (R1.3 / U9c): the legacy `^inputs` plane is retired, and the arms that
-- were fenced by `verdict_color IS NULL` — the node_rev compare, board_drift,
-- board_unresolved, and the green / superseded-algo / declared-unpinned board
-- arms — ARE NOW COLLAPSED. Every projected row carries a verdict, so the board
-- reads the colour plane's answer and computes none of its own.
--
-- **THE COLLAPSE OPENED A HOLE AND `board_residue` IS WHAT CLOSES IT.** Deleting
-- those arms means a row with no verdict matches NO arm of `board` and would
-- VANISH from the surface rather than render — absence reading as
-- nothing-to-report, which is the same fail-open the walk plane's `uncolourable`
-- arm exists to prevent, arriving through a different door. A projection cannot
-- match-exhaustively the way a Rust enum can, so the compiler cannot guard this;
-- the substitute is a RESIDUE QUERY whose count is disclosed beside every board
-- answer and pinned by a named test. Nothing can vanish without moving a
-- disclosed counter that a test owns.
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
    hash_algo    TEXT,                -- [1] the algo the pinned value was minted under (NULL = absent): the block's `hash-algo:` header for the legacy forms, the fingerprint token's own version field for a meridian-lock pin. PROJECTED ONLY — no `board` arm reads it, and it colours nothing (D22)
    src_doc_rev  TEXT     NOT NULL,   -- [1] containing doc_rev — the rev-compare invalidation key
    verdict_color  TEXT,              -- [color plane] the row's verdict tone ('green'|'red'|'grey'). NULL is the RESIDUE case: no arm of `board` matches it, so `board_residue` counts it and the count is disclosed. Two doors reach NULL — a row failing `LockItem::is_colourable`, and a colourable row whose verdict lookup missed
    verdict_reason TEXT,              -- [color plane] the verdict's stable reason word ('content-drifted', 'dangling-anchor', 'unverifiable-fingerprint', 'malformed-fingerprint', 'lock-refused', …); NULL for green
    verdict_detail TEXT,              -- [color plane] the reason's own detail — WHICH fingerprint-triple member is unknown, or WHY the lock refused; NULL when the reason word says it all
    PRIMARY KEY (src_path, seq)
);

-- board_red — the ONE red surface: the colour plane's reds, carrying the plane's
-- own reason word (`content-drifted` / `dangling-anchor` / `selector-unresolved`,
-- never conflated). A reader renders red iff a row is present, in the DEFAULT
-- face, no pack — so a pin the colour plane measured as drift MUST be present
-- here. Absent, a reader asking `board_red` whether anything is red would be
-- told no while `board` and `mrd walk` both say red: the under-report, on the
-- surface most likely to be glanced at rather than read.
--
-- The `board_drift` and `board_unresolved` views this used to union were the
-- legacy `node_rev` compare, fenced to rows the colour plane had not answered.
-- R4 left no such rows, so they computed a second verdict for a population that
-- cannot exist. Deleted whole rather than left standing: a second compare that
-- can never run is not a safety net, it is a divergence waiting for an edit.
CREATE VIEW board_red AS
    SELECT src_path, seq, to_path, to_sel, pinned_rev,
           NULL AS live_rev, verdict_reason AS reason
        FROM input_lock
        WHERE verdict_color = 'red';

-- board_residue — THE DISCLOSURE, and the reason the collapse above is safe.
--
-- Every row `board` cannot colour. `board`'s two arms partition `input_lock` on
-- `verdict_color IS NOT NULL`, so a NULL-verdict row matches neither and would
-- silently LEAVE THE SURFACE. This view is that row's only trace, and the count
-- is disclosed beside every board answer — absence never has to be inferred.
--
-- **IT SITS AT ZERO TODAY AND THAT IS THE POINT.** A NULL verdict is unreachable
-- at this revision by two DIFFERENT guarantees, and only one is the R4 parser
-- invariant: a row failing `LockItem::is_colourable` is closed by the parser
-- rule carried by `lock::a_pin_row_missing_a_mandatory_field_refuses_at_parse`;
-- a COLOURABLE row whose verdict lookup missed is closed only by
-- `declared_ref` and `fingerprint` being minted once in `collect_lock_pins` and
-- never reassigned. That second guarantee has no parser and no ratified law
-- behind it — it holds because two functions currently agree.
--
-- A counter that has never moved and CANNOT move is a decoration. A counter at
-- zero with a test proving it can move is a detector. `board_residue_*` are that
-- test.
-- **IT NAMES NO CAUSE, DELIBERATELY.** The two doors leave IDENTICAL evidence in
-- this table: `input_lock` carries neither the fingerprint slot nor the refusal
-- slot as its own column, so SQL here cannot tell an uncolourable row from a
-- colourable row whose lookup missed. Labelling each row with a guessed cause
-- would attach a category the instrument never measured. The count is the
-- finding; the cause is the reader's next question, answered in Rust where the
-- evidence lives.
CREATE VIEW board_residue AS
    SELECT il.src_path, il.seq, il.declared_ref, il.to_path, il.to_sel,
           il.pinned_rev, il.src_doc_rev
        FROM input_lock il
        WHERE il.verdict_color IS NULL;

-- board — U5.1's colours layer (d2 §5.3 'colors = board view'; wire-contract-v2
-- colors-amendment § Colors). Exactly ONE colour row per lock edge, in the
-- DEFAULT face, no pack. The colour is 'traces read through workflow vocabulary'.
--
-- **The board computes NO verdict of its own.** Both arms read `verdict_color`,
-- which the colour plane already answered, so the board and `mrd walk` cannot
-- disagree about one pin — there is one compare, not two. Reds ride `board_red`;
-- everything else rides the projected verdict. No row is coloured twice, and a
-- row with no verdict is coloured ZERO times — which is what `board_residue`
-- exists to disclose rather than let pass as silence.
CREATE VIEW board AS
    SELECT src_path, seq, to_path, to_sel, pinned_rev,
           live_rev, 'red' AS color, reason
        FROM board_red
    UNION ALL
    SELECT il.src_path, il.seq, il.to_path, il.to_sel, il.pinned_rev,
           NULL AS live_rev, il.verdict_color AS color,
           COALESCE(il.verdict_reason, 'attested') AS reason
        FROM input_lock il
        WHERE il.verdict_color IS NOT NULL AND il.verdict_color <> 'red';

";

/// Create additive read-face schema. Caller must create [`crate::facts`] first
/// (board views reference `node`).
///
/// # Errors
/// Propagates any `DuckDB` error from the DDL batch.
pub fn create_read_face_schema(conn: &Connection) -> duckdb::Result<()> {
    conn.execute_batch(READ_FACE_SCHEMA_SQL)
}

/// Lock the read face as a capability (A10; §2.1):
/// `enable_external_access=false` + `lock_configuration=true` — no
/// `ATTACH`/`COPY`/external read, and settings cannot be re-raised.
///
/// Call AFTER schema + projections are loaded (ordinary INSERTs still work;
/// only the external/write surface is frozen).
///
/// # Errors
/// Propagates any `DuckDB` error from the two `SET`s.
pub fn lock_read_face(conn: &Connection) -> duckdb::Result<()> {
    conn.execute_batch("SET enable_external_access=false;\nSET lock_configuration=true;")
}

/// Locked, board-ready face: project `node` + `input_lock`, create board views,
/// then lock. Refuses `ATTACH`/`COPY`/external access.
///
/// # Errors
/// [`ViewError::Duckdb`] on schema, projection, or lock failure.
pub fn open_board(docs: &BTreeMap<String, Document>) -> Result<Connection, ViewError> {
    let conn = Connection::open_in_memory()?;
    facts::create_facts_schema(&conn)?;
    facts::project_nodes(&conn, docs)?;
    create_read_face_schema(&conn)?;
    project_input_locks(&conn, docs)?;
    lock_read_face(&conn)?;
    Ok(conn)
}

/// Paths whose live `doc_rev` no longer matches the projection (§2.1 / §8).
/// Stale iff recorded `node.doc_rev` ≠ live root rev, or path absent.
/// Non-empty ⇒ rebuild before answering.
///
/// # Errors
/// Propagates any `DuckDB` error reading recorded revs.
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

/// Pin identity: `(src_path, declared_ref, fingerprint)`. Colour is a pure
/// function of this key plus live corpus.
type PinKey = (String, String, Option<String>);

/// Colour of every `meridian-lock` row, keyed by [`PinKey`]. Asks
/// [`crate::walk::lock_pin_colors`] (same `edge_color` as walk) — this face
/// computes no colour of its own.
fn pin_verdicts(docs: &BTreeMap<String, Document>) -> BTreeMap<PinKey, Color> {
    walk::lock_pin_colors(docs)
        .into_iter()
        .map(|pin| ((pin.src_path, pin.declared_ref, pin.fingerprint), pin.color))
        .collect()
}

/// Project lock pins into `input_lock` (source-1 parse + colour-plane verdict).
/// Lock-refusal rows project too: grey `lock-refused`, empty `to_path` (leaf).
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
            // Shared predicate with walk ([`LockItem::is_colourable`]).
            // `.flatten()` is the second door to NULL verdict: colourable but
            // lookup-miss — key matches only because fields are minted once in
            // [`collect_lock_pins`]. `board_residue` counts that fallout.
            let verdict = item
                .is_colourable()
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

/// One parsed lock item (source 1). Public so walk and board share one parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockItem {
    /// Declared `ref`, verbatim. Empty on a [`LockItem::lock_refusal`] row.
    pub declared_ref: String,
    /// Parsed address ([`addr::Addr`]) — structural owner (U10). Consumers
    /// read this; nothing re-splits `declared_ref`. `None` = refusal or
    /// out-of-grammar spelling.
    pub declared_addr: Option<addr::Addr>,
    /// Target page path (`ref` path when `to` absent). Empty on refusal (leaf).
    pub to_path: String,
    /// Selector after first `#` (`""` = page/doc root).
    pub to_sel: String,
    /// Pinned rev — `None` = declared-only (grey under legacy; unused under R4).
    pub pinned_rev: Option<String>,
    /// `content` | `object` (`None` = unstated).
    pub rev_class: Option<String>,
    /// Algo label (header or derived from fingerprint token). Projected only —
    /// no board arm reads it; colour is `verdict_color` alone (D22).
    pub hash_algo: Option<String>,
    /// `fp1.…` CID-token. Verdict computers consume this, never a lock re-parse.
    pub fingerprint: Option<String>,
    /// R4 structural selector (`path` XOR `properties`). Owner of the claim
    /// address — `to_sel` is the lossy `/`-joined display. Verdict computers
    /// read this and re-split nothing. `None` only on refusal.
    pub selector: Option<lock::Selector>,
    /// Wiki-link object (path without `.md`). Needed for transcript
    /// `session#seq-N` classification; not re-derived from `to_path`.
    pub object: String,
    /// Page-level lock parse refusal ([`lock::LockError`]). Declares no edge —
    /// greys as `lock-refused` so corrupt ≠ "no pins".
    pub lock_refusal: Option<String>,
    /// Mounted root this edge resolved into (`None` = ambient). `to_path` is
    /// inside that root — ambient lookup is wrong-bytes success.
    pub to_root: Option<addr::MountName>,
    /// Root could not be resolved — carries the grey reason (causes stay
    /// distinct; S3-R50). Distinct from `lock_refusal` (unreadable HERE vs
    /// unreachable FROM here).
    pub root_refusal: Option<model::selector::GreyReason>,
    /// Root was reached; file not in it — measured absence (U21). Separate
    /// from `root_refusal` (grey refuse vs red claim).
    pub root_absence: Option<addr::MountName>,
}

impl LockItem {
    /// Whether the colour law can answer this row: has fingerprint evidence or
    /// a stated refusal. Walk skips non-colourable; board writes NULL verdict.
    /// **One definition** so residue counts exactly the rows walk skipped.
    #[must_use]
    pub fn is_colourable(&self) -> bool {
        self.fingerprint.is_some() || self.lock_refusal.is_some()
    }
}

/// Parse every lock pin in `doc` (source 1). Shared reader for board and walk —
/// one form: the `meridian-lock` `pins:` plane ([`collect_lock_pins`]).
#[must_use]
pub fn page_lock_items(doc: &Document) -> Vec<LockItem> {
    let mut out = Vec::new();
    collect_lock_pins(doc, &mut out);
    out
}

/// Corpus [`CorpusIndex`] over `docs` — shared by board and walk.
#[must_use]
pub fn corpus_index(docs: &BTreeMap<String, Document>) -> CorpusIndex {
    let mut index = CorpusIndex::new();
    for (path, doc) in docs {
        index.insert(path, doc);
    }
    index
}

/// Parse lock items then resolve each `to_path` against the corpus (wikilink
/// name → vault path). Unresolvable keeps bare name → red/unresolved.
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

/// Root-keyed form: resolution is a mount lookup (U11). Outcomes are structural:
/// mounted → `to_root` + in-root path; unbound → grey `unmounted`; else ambient.
/// Unmounted does **not** take the red `selector-unresolved` path.
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
            // Miss inside a mounted root: measured absence. Set `to_root` +
            // peeled `to_path` so naming is root-qualified (not ambient decoy)
            // and walk treats it as a leaf. Leaving declared spelling yields
            // doubled root prefix.
            model::RefResolution::NotFound {
                root: Some(root),
                path,
                ..
            } => {
                item.to_root = Some(root.clone());
                item.to_path = path;
                item.root_absence = Some(root);
            }
            // Ambient miss / malformed: keep declared spelling for red render.
            model::RefResolution::NotFound { root: None, .. }
            | model::RefResolution::Malformed(_) => {}
        }
    }
    items
}

/// Resolve `to_path` via the one address owner ([`CorpusIndex::resolve_ref`]).
/// Unresolvable returns bare input (first-class unresolved). Public so writers
/// resolve to the same path the board reads.
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

/// Root-keyed [`resolve_to_path`]. Unmounted `root:` refs report the address
/// verbatim (not ambient same-basename); caller uses `resolve_ref` for grey.
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

/// Algo label when a fingerprint token does not parse — outside native set so
/// it never enters a node-rev compare.
const FP_ALGO_UNKNOWN: &str = "fp-unknown";

/// Append `meridian-lock` `pins:` of `doc` to `out`. [`lock::find`] is the only
/// parser. Fail-loud: malformed / multi-block projects ONE grey
/// [`LockItem::lock_refusal`] row — corrupt must not read as "no pins".
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
                // Resolution fills to_root; parser has not consulted mounts.
                to_root: None,
                root_refusal: None,
                root_absence: None,
            });
            return;
        }
    };
    for pin in found.lock.pins {
        // One-time mint of display spelling; verdict computers read `selector`.
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
            pinned_rev: Some(pin.fingerprint.clone()),
            rev_class: Some("content".to_string()), // pins: is the claim plane
            hash_algo: Some(fingerprint_algo(&pin.fingerprint)),
            fingerprint: Some(pin.fingerprint),
            selector: Some(pin.selector),
            object: pin.object,
            lock_refusal: None,
            to_root: None,
            root_refusal: None,
            root_absence: None,
        });
    }
}

/// `/`-joined display spelling of an R4 selector — human plane only; **lossy,
/// never re-split**. Verdict computers read [`LockItem::selector`].
fn display_selector(selector: &lock::Selector) -> String {
    match selector {
        lock::Selector::Path(segments) => segments.join("/"),
        lock::Selector::Properties(_) => String::new(),
    }
}

/// Algo label from fingerprint token version. Outside native set so no node-rev
/// compare can call the row green (D22). Colour comes from projected verdict.
fn fingerprint_algo(token: &str) -> String {
    match model::fingerprint::parse_fingerprint(token) {
        // Collision with native version spelling → unknown (never node-rev green).
        Some(parts) if !model::is_native_algo(&parts.version) => parts.version,
        _ => FP_ALGO_UNKNOWN.to_string(),
    }
}

/// `usize` → `u64`, saturating.
fn u64c(x: usize) -> u64 {
    u64::try_from(x).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(raw: &str) -> Document {
        model::build(raw.to_string(), syntax::parse(raw))
    }

    /// R4 blob hash — mandatory on every fixture pin.
    const FIXTURE_HASH: &str = "9ae3f1deadbeef";

    /// Fixture page with one R4 pin per `(ref-spelling, fingerprint)`.
    /// Spelling is split to object + selector array at this door only.
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

    /// Well-formed CID-token (`fp1.span2.b3.<64hex>`).
    fn fp(nibble: &str) -> String {
        format!("fp1.span2.b3.{}", nibble.repeat(32))
    }

    /// meridian-lock pin projects a row (not silent absence).
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

    /// Board projects lock pin + colour-plane verdict; wrong digest is measured
    /// red, live token greens.
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
        // board_red agrees with board (no under-report).
        let reds: i64 = conn
            .query_row(
                "SELECT count(*) FROM board_red WHERE src_path='effect.md' AND reason='content-drifted'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(reds, 1, "a measured fingerprint drift IS a board red");

        // Live token greens — red above is measured, not blanket.
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
        // Pin Malformed@line 3 (not the version gate).
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

    /// Refusal reaches board as grey `lock-refused` (not red, not silent).
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

    /// Derived algo label stays outside native set — no false node-rev green.
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

    /// Address is a typed parse at ingress; projection columns match structure.
    #[test]
    fn a_lock_ref_is_a_parsed_address_and_the_projection_is_byte_identical() {
        for (spelling, want_path, want_sel) in [
            ("wiki/page.md", "wiki/page.md", ""),
            ("wiki/page.md#Design", "wiki/page.md", "Design"),
            // Block-id keeps caret (`node.selector` verbatim).
            ("wiki/page.md#^claim-1", "wiki/page.md", "^claim-1"),
        ] {
            let items = page_lock_items(&doc(&lock_page(&[(spelling, &fp("34"))])));
            assert_eq!(
                (items[0].to_path.as_str(), items[0].to_sel.as_str()),
                (want_path, want_sel),
                "{spelling}",
            );
        }

        // Cross-root: to_path keeps full spelling (no ambient peel here).
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
