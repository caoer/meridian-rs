//! `sql.duckdb` — the fingerprint-pinned, append-only `DuckDB` cache of the
//! sql projection (session design `results/sql-duckdb-append-cache-design.md`,
//! ZT-ruled 2026-08-14).
//!
//! One file per root, living in the engine's per-workspace cache drawer. Two
//! namespaces inside it:
//!
//! - **`hist.*`** — append-only base tables: every projection table plus a
//!   `gen BIGINT` (which append wrote the row) and, on `hist.doc` only, a
//!   `tombstone` (the path was removed at that generation). The pin ledger
//!   `hist.pin` is itself append-only; **the highest-generation pin row IS the
//!   file's fingerprint pin**, written in the same transaction as the rows it
//!   covers, so the file is always at exactly one fingerprint. One table is
//!   NOT generation-keyed: `hist.body_text (body_key, text)` is the
//!   content-addressed chunk store (`docs/body-projection.md` §4) —
//!   insert-if-absent, so unchanged body text is stored once across
//!   generations and paths while `hist.body` carries the narrow per-gen rows
//!   that reference it.
//! - **`main.*`** — the caller-facing latest views, keeping the ephemeral
//!   projection's exact names and column shapes (`doc`, `section`, `link`,
//!   `backlink`, `dangling`, `record`, `tag_all`, `task`, `frontmatter`,
//!   `frontmatter_tag`, `tag`) — plus `_meridian_view`, the ONE relation whose
//!   shape deliberately differs (there a singleton table, here a view over the
//!   pin ledger). One `QUALIFY` window over `hist.doc` picks each path's newest
//!   version and drops tombstones; child views follow by `(path, gen)` semi
//!   join.
//!
//! Both namespaces are real schemas in ONE catalog, so a caller reading
//! `information_schema` sees `doc`/`section`/`task` TWICE — once per schema.
//! Discovery must stay schema-qualified (`WHERE table_schema = 'main'`, or
//! `GROUP BY table_schema, table_name`); grouping by `table_name` alone merges
//! the two relations into one falsely-doubled column list (card
//! sql-information-schema-doubling).
//!
//! Only `INSERT` ever runs — no UPDATE, no DELETE, on any path including
//! repair (`DuckDB` punishes edits). The only compaction is rebuild-and-swap:
//! the cache is a pure function of the corpus, so deleting the file is always
//! a correct repair.
//!
//! # Delta grain (append protocol)
//!
//! The cache is its own manifest: the latest `doc(path, file_rev)` map is a
//! complete `(path, rev)` picture of the pinned state (`file_rev` is the
//! Merkle leaf truncated), so diffing it against the live parsed corpus names
//! added / changed / removed exactly. On top of the content delta, the append
//! re-projects unchanged docs whose **link resolution** the delta can move: a
//! doc appearing, vanishing, or changing its aliases changes what `[[name]]`
//! resolves to in docs that did not move. The affected set is matched by
//! name key (basename stem + full path key + aliases, lowercased — a
//! deliberate superset of [`model::CorpusIndex`]'s resolution keys), so the
//! latest views stay equal to a fresh build after every append.
//!
//! # The base plane (`docs/base-projection.md` §7)
//!
//! `.base` members ride the same protocol under their OWN witness: the latest
//! `base(path, file_rev)` map is diffed against the live base walk exactly as
//! `doc(path, file_rev)` is diffed against the live parsed corpus, and an
//! append triggers on EITHER delta — so base motion appends even when the
//! fingerprint did not move. The pin ledger therefore carries `base_fold`
//! beside the fingerprint, and the no-op check reads one row for both.
//!
//! Its affected-set rule runs over ALL link rows, not the dangling ones, and
//! its keys are **case-exact** — separate from the deliberately-lowercased md
//! keys above, because folding them together would reintroduce the
//! case-folding the base floor and the §5.1 mint rule forbid.
//!
//! Disclosed approximations (repair = the rebuild verb): motion OUTSIDE the
//! pinned corpus — mount-table edits, other roots' contents (cross-root
//! `dest_root_path`), and non-md files entering/leaving the `exclusion`
//! domain — moves no fingerprint and therefore triggers no append. **`.base`
//! motion has LEFT that list** (§7); every other non-md class (`.svg`,
//! `.xlsx`, …) remains approximated, since no snapshot of those exists to
//! diff, and rebuild remains the repair. The `:memory:` lane has no
//! approximation: it re-walks everything per query.
//!
//! # Query lane
//!
//! Every caller query runs `BEGIN → statement → collect → ROLLBACK`
//! ([`SqlStore::query`]): reads are unaffected; DML against hist tables
//! executes, is visible to the caller's own statement, and touches nothing
//! durable (the pinned "writes nothing durable" contract carried into the
//! persistent file — rolled-back DML does not even grow it); DML against a
//! latest view refuses through `DuckDB`'s own `Binder Error`, extended with
//! the remedy (ruling OQ1: the refusal teaches).
//!
//! A transaction is not the whole contract, because extension code does not
//! live inside one: `LOAD` lands on the shared `DatabaseInstance`, and a
//! loaded extension may then write through its own path (duckpgq's `CREATE
//! PROPERTY GRAPH` wrote `sql.main.__duckpgq_internal` durably into the
//! drawer — card `sql-extension-ddl-escapes-rollback-lane`). So the lane is
//! also gated at the only door extension code has:
//! [`apply_extension_gate`] loads the measured-clean [`EXTENSION_ALLOW_LIST`]
//! at open and then closes `LOAD`/`INSTALL` one-way. What the rollback cannot
//! undo, the door never admits.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use duckdb::Connection;
use duckdb::types::Value;
use serde_json::Value as Json;

use crate::sqltext;
use crate::{
    BaseWalk, ExclusionProbe, Rows, ViewError, collect_doc, corpus_index, fill_exclusions,
};

/// The cache file's own payload schema version, recorded in every pin row.
/// Bump it — together with the drawer path's `SCHEMA_SALT` — whenever the
/// hist DDL changes; a mismatched file is deleted and cold-rebuilt, never
/// migrated (the cache is a pure function of the corpus).
///
/// `2`: `main.dangling` gained `AND exclusion IS NULL`, and the exclusion
/// mint's bare-name fallback changed stamped content for identical corpora —
/// a v1 file would serve pre-ruling rows at the same fingerprint.
///
/// `3`: `hist.frontmatter` gained `prop_rev`; a v2 file's rows cannot answer
/// a key-grain guard question at all.
///
/// `4`: the `card` view became `record`, and `task.text` dropped its
/// list-marker + checkbox prefix — a v3 file's `hist.task` rows carry the
/// marker bytes and would digest differently from a fresh build.
///
/// `5`: `hist.section.hpath` / `hist.task.hpath` became TEXT — the published
/// `[{"h":…},…]` machine address (card sql-hpath-read-grammar); a v4 file's
/// TEXT[] rows would serve the retired spelling.
///
/// `6`: `hist.section.n` added — the occurrence index served as its own column
/// (`wire-contract.md` § A.11); a v5 file's rows carry no such column, so the
/// appender's positional load would land every later column one slot left.
///
/// `7`: the `.base` projection (`docs/base-projection.md`) — three `hist.base*`
/// tables, `hist.link.exclusion_path`, and `hist.pin.base_fold`. A v6 file
/// carries no base rows at all and its `exclusion` content predates the §5.1
/// mint rule.
///
/// `8`: `hist.body` + `hist.body_text` + `main.body` — the content-addressed
/// body projection (`docs/body-projection.md` §4); a v7 file has no chunk
/// rows to serve.
///
/// `9`: `hist.frontmatter_tag` rows for block-sequence `tags:` — the parse
/// moved to `model::fm_tags` and a v8 file holds ZERO rows for every page
/// written that way, pinned to a fingerprint that has not moved and never
/// will. The append delta cannot repair it: the files are unchanged, so
/// nothing re-stages them (card `tag-all-block-form-blindness`).
///
/// `10`: `hist.frontmatter.value` for a block-sequence value under ANY key —
/// `model::fm_value` renders it as flow-style text, and a v9 file holds `''`
/// for every such row (50 of 50 `agents` rows on the fleet corpus), pinned
/// to fingerprints that will not move. Same repair-by-rebuild reasoning as
/// `9` (card `fm-block-list-sql-empty`).
pub const CACHE_SCHEMA_VERSION: i64 = 10;

/// The cache file's basename inside the workspace cache drawer (ruling OQ4).
pub const SQL_CACHE_FILENAME: &str = "sql.duckdb";

/// Append-only hist tables + pin ledger + the caller-facing latest views.
///
/// No PRIMARY KEY / UNIQUE / FK constraints on hist tables: uniqueness of
/// `(path, gen)` is the single-appender protocol's invariant, not a
/// constraint every bulk load pays for (measured in duckdb-expert: bulk load
/// with a PK 461s vs 121s without).
const CACHE_SCHEMA_SQL: &str = r"
CREATE SCHEMA hist;
CREATE TABLE hist.doc (
    path       TEXT NOT NULL,
    gen        BIGINT NOT NULL,
    tombstone  BOOLEAN NOT NULL DEFAULT false,
    file_rev   TEXT,                -- NULL on a tombstone row
    line_count UINTEGER,
    bytes      UBIGINT
);
CREATE TABLE hist.frontmatter (
    path TEXT, gen BIGINT, ord UBIGINT, key TEXT, value TEXT,
    span_start UBIGINT, span_end UBIGINT, node_rev TEXT, prop_rev TEXT
);
CREATE TABLE hist.section (
    path TEXT, gen BIGINT, node_seq UBIGINT, hpath TEXT, n UINTEGER, heading TEXT,
    level UTINYINT, node_rev TEXT, span_start UBIGINT, span_end UBIGINT
);
CREATE TABLE hist.link (
    src_path TEXT, gen BIGINT, seq UBIGINT, kind TEXT, target_raw TEXT,
    heading TEXT, block TEXT, alias TEXT,
    dest_path TEXT, dest_root TEXT, dest_root_path TEXT, exclusion TEXT,
    exclusion_path TEXT,
    span_start UBIGINT, span_end UBIGINT, node_rev TEXT
);
CREATE TABLE hist.tag (
    path TEXT, gen BIGINT, seq UBIGINT, tag TEXT,
    span_start UBIGINT, span_end UBIGINT, node_rev TEXT
);
CREATE TABLE hist.frontmatter_tag (
    path TEXT, gen BIGINT, seq UBIGINT, tag TEXT, key TEXT,
    span_start UBIGINT, span_end UBIGINT, node_rev TEXT
);
CREATE TABLE hist.task (
    path TEXT, gen BIGINT, seq UBIGINT, checked BOOLEAN, depth UINTEGER,
    section_seq UBIGINT, hpath TEXT, text TEXT,
    span_start UBIGINT, span_end UBIGINT, node_rev TEXT
);
CREATE TABLE hist.base (
    path TEXT, gen BIGINT, tombstone BOOLEAN DEFAULT false,
    file_rev TEXT, bytes UBIGINT, error TEXT,
    filters TEXT, properties TEXT, extra TEXT
);
CREATE TABLE hist.base_view (
    path TEXT, gen BIGINT, ord UBIGINT, name TEXT, type TEXT,
    filters TEXT, config TEXT
);
CREATE TABLE hist.base_formula (
    path TEXT, gen BIGINT, ord UBIGINT, name TEXT, expr TEXT
);
-- The body split (docs/body-projection.md §4): narrow per-gen rows reference
-- content-addressed text, so an edit re-stores only chunks whose bytes are
-- new to this file's history.
CREATE TABLE hist.body (
    path TEXT, gen BIGINT, seq UBIGINT, section_seq UBIGINT, hpath TEXT,
    span_start UBIGINT, span_end UBIGINT, node_rev TEXT, body_key TEXT
);
CREATE TABLE hist.body_text (
    body_key TEXT,   -- full 64-hex blake3 of the chunk bytes (spec §4: corpus-wide address, full width)
    text     TEXT
);
CREATE TABLE hist.pin (
    gen BIGINT, fingerprint VARCHAR, applied_at TIMESTAMP,
    files_added BIGINT, files_changed BIGINT, files_removed BIGINT,
    engine_version VARCHAR, cache_schema_version BIGINT,
    -- The SECOND witness beside the fingerprint (`base-projection.md` §7):
    -- base motion appends even when the fingerprint did not move, so the pin
    -- ledger carries what the append covered on BOTH planes. NULL = the
    -- append was handed no base walk.
    base_fold VARCHAR
);

-- The latest pick, ONE window over hist.doc only (receipt P1); children
-- resolve by (path, generation) semi join — a tombstone generation has no child rows.
CREATE VIEW hist.doc_latest AS
    SELECT * FROM hist.doc
    QUALIFY row_number() OVER (PARTITION BY path ORDER BY gen DESC) = 1;

CREATE VIEW main.doc AS
    SELECT path, file_rev, line_count, bytes
    FROM hist.doc_latest WHERE NOT tombstone;
CREATE VIEW main.frontmatter AS
    SELECT f.path, f.ord, f.key, f.value, f.span_start, f.span_end, f.node_rev,
           f.prop_rev
    FROM hist.frontmatter f
    SEMI JOIN hist.doc_latest d ON f.path = d.path AND f.gen = d.gen;
CREATE VIEW main.section AS
    SELECT s.path, s.node_seq, s.hpath, s.n, s.heading, s.level, s.node_rev,
           s.span_start, s.span_end
    FROM hist.section s
    SEMI JOIN hist.doc_latest d ON s.path = d.path AND s.gen = d.gen;
CREATE VIEW main.link AS
    SELECT l.src_path, l.seq, l.kind, l.target_raw, l.heading, l.block,
           l.alias, l.dest_path, l.dest_root, l.dest_root_path, l.exclusion,
           l.exclusion_path,
           (l.dest_path IS NOT NULL OR l.dest_root IS NOT NULL) AS resolved,
           l.span_start, l.span_end, l.node_rev
    FROM hist.link l
    SEMI JOIN hist.doc_latest d ON l.src_path = d.path AND l.gen = d.gen;
CREATE VIEW main.tag AS
    SELECT t.path, t.seq, t.tag, t.span_start, t.span_end, t.node_rev
    FROM hist.tag t
    SEMI JOIN hist.doc_latest d ON t.path = d.path AND t.gen = d.gen;
CREATE VIEW main.frontmatter_tag AS
    SELECT f.path, f.seq, f.tag, f.key, f.span_start, f.span_end, f.node_rev
    FROM hist.frontmatter_tag f
    SEMI JOIN hist.doc_latest d ON f.path = d.path AND f.gen = d.gen;
CREATE VIEW main.task AS
    SELECT t.path, t.seq, t.checked, t.depth, t.section_seq, t.hpath, t.text,
           t.span_start, t.span_end, t.node_rev
    FROM hist.task t
    SEMI JOIN hist.doc_latest d ON t.path = d.path AND t.gen = d.gen;
CREATE VIEW main.body AS
    SELECT b.path, b.seq, b.section_seq, b.hpath, t.text,
           b.span_start, b.span_end, b.node_rev
    FROM hist.body b
    SEMI JOIN hist.doc_latest d ON b.path = d.path AND b.gen = d.gen
    JOIN hist.body_text t USING (body_key);

-- The base relations, same protocol: ONE window over hist.base picks each
-- member's newest generation and drops tombstones; children follow by
-- (path, gen) semi join (`base-projection.md` §7).
CREATE VIEW hist.base_latest AS
    SELECT * FROM hist.base
    QUALIFY row_number() OVER (PARTITION BY path ORDER BY gen DESC) = 1;
CREATE VIEW main.base AS
    SELECT path, file_rev, bytes, error, filters, properties, extra
    FROM hist.base_latest WHERE NOT tombstone;
CREATE VIEW main.base_view AS
    SELECT v.path, v.ord, v.name, v.type, v.filters, v.config
    FROM hist.base_view v
    SEMI JOIN hist.base_latest b ON v.path = b.path AND v.gen = b.gen;
CREATE VIEW main.base_formula AS
    SELECT f.path, f.ord, f.name, f.expr
    FROM hist.base_formula f
    SEMI JOIN hist.base_latest b ON f.path = b.path AND f.gen = b.gen;

-- Convenience views: today's definitions verbatim, now over the latest views.
CREATE VIEW main.backlink AS
    SELECT dest_path AS path, src_path, kind, alias FROM main.link
    WHERE dest_path IS NOT NULL;
CREATE VIEW main.dangling AS
    SELECT src_path, target_raw FROM main.link
    WHERE kind IN ('wikilink','embed') AND dest_path IS NULL AND dest_root IS NULL
      AND exclusion IS NULL;
CREATE VIEW main.record AS
    SELECT d.path,
        max(fm.value) FILTER (fm.key = 'type')    AS type,
        max(fm.value) FILTER (fm.key = 'status')  AS status,
        max(fm.value) FILTER (fm.key = 'owner')   AS owner,
        max(fm.value) FILTER (fm.key = 'session') AS session
    FROM main.doc d LEFT JOIN main.frontmatter fm USING (path) GROUP BY d.path;
CREATE VIEW main.tag_all AS
    SELECT path, tag, 'inline'      AS source, span_start, span_end, node_rev FROM main.tag
    UNION ALL
    SELECT path, tag, 'frontmatter' AS source, span_start, span_end, node_rev FROM main.frontmatter_tag;

-- The stamp is a view over the pin ledger: the highest-generation pin row IS the pin.
CREATE VIEW main._meridian_view AS
    SELECT fingerprint AS as_of_fingerprint, gen, applied_at, cache_schema_version,
           base_fold
    FROM hist.pin
    QUALIFY row_number() OVER (ORDER BY gen DESC) = 1;
";

/// The file's current pin — the highest-generation `hist.pin` row.
#[derive(Debug, Clone)]
pub struct Pin {
    /// The append counter the pin row rode in on.
    pub generation: i64,
    /// The corpus fingerprint the file is at.
    pub fingerprint: String,
    /// The payload schema version the file was written under.
    pub cache_schema_version: i64,
    /// The `.base` witness the same append covered (`base-projection.md` §7);
    /// `None` = that append was handed no base walk.
    pub base_fold: Option<String>,
}

/// What one [`SqlStore::sync`] append did (the pin-ledger counts).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppendCounts {
    /// The generation the append wrote.
    pub generation: i64,
    /// Paths new to the corpus.
    pub added: u64,
    /// Paths re-projected: content moved, or link resolution moved under them.
    pub changed: u64,
    /// Paths tombstoned.
    pub removed: u64,
}

/// One sync's `.base` plane delta (`base-projection.md` §7).
///
/// Default is the NOT-ASKED delta: no walk was handed in, so nothing
/// re-projects and nothing is tombstoned. It is deliberately indistinguishable
/// from a walk that found every member unmoved, because in both cases the base
/// rows the file already holds stay exactly as they are.
#[derive(Default)]
struct BaseDelta {
    /// Members to re-project (added or changed).
    reproject: Vec<crate::BaseMember>,
    /// Member paths to tombstone.
    removed: Vec<String>,
    /// Every moved member's path — the affected-set key source.
    moved_keys: BTreeSet<String>,
}

/// Every lane's spill budget (card sql-spill-config-lockout): the `DuckDB`
/// default is 90% of available disk — effectively unbounded, and it `ENOSPC`ed
/// a host (one query spilled >9 GiB into the seat's cwd).
const SPILL_BUDGET: &str = "8GiB";

/// The extensions loaded into every engine instance at open (card
/// sql-extension-ddl-escapes-rollback-lane). The list is the gate: after
/// [`apply_extension_gate`] runs, nothing else can ever be loaded, so an
/// extension is reachable from the query lane only by standing here.
///
/// Membership is earned by MEASUREMENT — the extension's DDL must stay inside
/// the caller's transaction. `fts` and `vss` were measured clean on DuckDB
/// v1.5.4 (2026-08-23): `PRAGMA create_fts_index` over a durable `hist` table
/// left no `fts_main_*` schema behind, and an HNSW index built under
/// `hnsw_enable_experimental_persistence` left neither index nor table.
/// `duckpgq` is the counter-example that opened the card and is NOT here: its
/// `CREATE PROPERTY GRAPH` wrote two durable rows into
/// `sql.main.__duckpgq_internal`, outside the caller's transaction.
const EXTENSION_ALLOW_LIST: &[&str] = &["fts", "vss"];

/// Close the query lane to extension code (card
/// sql-extension-ddl-escapes-rollback-lane).
///
/// `BEGIN → statement → ROLLBACK` is a TRANSACTION, and a transaction covers
/// only what `DuckDB`'s own catalog and storage put inside it. An extension
/// loads onto the shared `DatabaseInstance`, not into the caller's
/// transaction, and is then free to write through its own path: measured
/// 2026-08-23, `LOAD duckpgq; …; CREATE PROPERTY GRAPH g …` left
/// `sql.main.__duckpgq_internal` holding 2 rows in the drawer AFTER the call,
/// and a second `CREATE PROPERTY GRAPH g` answered `Property graph table with
/// name g already exists`. The same call's `CREATE TABLE n`/`e` were gone —
/// core DDL rolls back exactly as the contract says.
///
/// So the lane is gated at the only door extension code has. Explicit `LOAD`
/// and `INSTALL` are the whole door: community extensions do not autoload
/// (measured — a bare `SELECT quack('hello')` answered `Did you mean
/// "quarter"?` with `quack` installed and both autoload settings on), and
/// `CREATE PROPERTY GRAPH` without a prior `LOAD` is a parser error.
///
/// Three statements, in this order:
/// 1. `autoinstall_known_extensions=false` — the engine never reaches the
///    network; a `LOAD` below can only find what the host already has.
/// 2. the [`EXTENSION_ALLOW_LIST`], best effort — a host with the extension
///    absent keeps serving the projection, and the capability is simply
///    missing (the caller's own `Catalog Error` says so).
/// 3. `enable_external_access=false` — one-way in `DuckDB`: a caller's own
///    `SET` can tighten it, never loosen it (`Cannot enable external access
///    while database is running`). After it, `LOAD`/`INSTALL` answers
///    `Permission Error: Loading external extensions is disabled through
///    configuration` and no external file can be written either.
///
/// This is a door, not a sandbox (the NO-SANDBOX ruling, 2026-08-14): one
/// execution path for every caller, no per-caller profile, and the
/// configuration is NOT locked — a caller's `SET memory_limit` still
/// succeeds.
///
/// # Errors
/// Propagates the `DuckDB` error from the two gate `SET`s. A failed allow-list
/// `LOAD` is not an error — see step 2.
pub fn apply_extension_gate(conn: &Connection) -> duckdb::Result<()> {
    conn.execute_batch("SET autoinstall_known_extensions=false;")?;
    for ext in EXTENSION_ALLOW_LIST {
        let _ = conn.execute_batch(&format!("LOAD {ext};"));
    }
    conn.execute_batch("SET enable_external_access=false;")
}

/// Bound the connection's disk spill (card sql-spill-config-lockout):
/// `spill_dir` becomes the ABSOLUTE `temp_directory`, with [`SPILL_BUDGET`]
/// as `max_temp_directory_size`. Plain config, never a sandbox (the
/// NO-SANDBOX ruling, 2026-08-14) — nothing is locked, and a caller's own
/// `SET` can re-point it. The `DuckDB` defaults were the defect this repairs:
/// a `.tmp` path RELATIVE to the shell cwd (no flag changes the process cwd,
/// so spills landed wherever the seat happened to be) and a %-of-disk budget.
///
/// # Errors
/// Propagates the `DuckDB` error from the two `SET`s.
pub fn apply_spill_containment(conn: &Connection, spill_dir: &Path) -> duckdb::Result<()> {
    // Best-effort: DuckDB creates the directory on first spill too.
    let _ = std::fs::create_dir_all(spill_dir);
    let spill = spill_dir.display().to_string().replace('\'', "''");
    conn.execute_batch(&format!(
        "SET temp_directory='{spill}';\n\
         SET max_temp_directory_size='{SPILL_BUDGET}';"
    ))
}

/// One result column of a served query.
#[derive(Debug)]
pub struct ColMeta {
    /// The column name as `DuckDB` reports it.
    pub name: String,
    /// A friendly type name (best effort, from the arrow schema).
    pub ty: String,
}

/// Execute one caller query on `conn` and materialise all rows + column
/// metadata. Returns the SQL error string (never a structured error) so the
/// caller can buffer it into its answer — the engine's words flow verbatim.
///
/// # Errors
/// The error is the `DuckDB` message, extended with the OQ1 teaching when the
/// statement was DML against a latest view (`Binder Error: Can only update
/// base table` — the remedy names the hist lane).
pub fn run_query(conn: &Connection, query: &str) -> Result<(Vec<ColMeta>, Vec<Vec<Json>>), String> {
    let mut stmt = conn.prepare(query).map_err(|e| teach(&e.to_string()))?;
    let mut rows = stmt.query([]).map_err(|e| teach(&e.to_string()))?;

    // Column metadata is available once the query has executed (`query` binds +
    // executes); collect owned copies before stepping the rows. The tz flags
    // ride beside the names: `ValueRef` drops the arrow timestamp's timezone,
    // and a tz-aware column renders with the `+00` marker (§ S1).
    let (columns, tz_cols): (Vec<ColMeta>, Vec<bool>) = {
        let stmt_ref = rows
            .as_ref()
            .ok_or_else(|| "no result statement".to_owned())?;
        let n = stmt_ref.column_count();
        let mut cols = Vec::with_capacity(n);
        let mut tz = Vec::with_capacity(n);
        for i in 0..n {
            let name = stmt_ref
                .column_name(i)
                .map_or_else(|_| format!("col{i}"), String::clone);
            let dt = stmt_ref.column_type(i);
            tz.push(matches!(
                dt,
                duckdb::arrow::datatypes::DataType::Timestamp(_, Some(_))
            ));
            cols.push(ColMeta {
                name,
                ty: arrow_type_name(&dt),
            });
        }
        (cols, tz)
    };

    let mut out = Vec::new();
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let mut r = Vec::with_capacity(tz_cols.len());
        for (i, tz_col) in tz_cols.iter().enumerate() {
            let v = row
                .get_ref(i)
                .map_or(Json::Null, |v| value_ref_to_json(v, *tz_col));
            r.push(v);
        }
        out.push(r);
    }
    Ok((columns, out))
}

/// Face names that were retired by a rename, and the name that replaced each.
/// The renames shipped with no compat alias, so the catalog refusal IS the
/// whole migration path — a caller who learned the old face last week must
/// read the new word out of the error itself.
const RETIRED_NAMES: &[(&str, &str)] = &[("card", "record")];

/// `DuckDB`'s built-in metadata surfaces. Its Did-you-mean is pure edit
/// distance over the WHOLE catalog, so a retired face name can fit one of
/// these by accident (`card` → `pg_attrdef`, `board_drift` →
/// `duckdb_constraints`). None of them is ever the answer to a face question.
const CATALOG_INTERNAL_PREFIXES: &[&str] = &["pg_", "duckdb_", "sqlite_"];

/// Whether a Did-you-mean line offers a catalog internal as the fit.
fn suggests_catalog_internal(line: &str) -> bool {
    let Some(rest) = line.trim_start().strip_prefix("Did you mean \"") else {
        return false;
    };
    CATALOG_INTERNAL_PREFIXES
        .iter()
        .any(|p| rest.starts_with(p))
}

/// The unknown table name out of `DuckDB`'s catalog refusal, e.g.
/// `Catalog Error: Table with name card does not exist!` → `card`.
fn unknown_table(error: &str) -> Option<&str> {
    let rest = error.split_once("Table with name ")?.1;
    rest.split_once(" does not exist").map(|(name, _)| name)
}

/// Extend a view-DML refusal with its remedy (ruling OQ1: the refusal
/// teaches). `DuckDB`'s words stay verbatim and first; the teaching follows.
///
/// Two more arms ride the same register — reason first, then a suggestion
/// that fits: a retired face name names its replacement
/// ([`RETIRED_NAMES`]), and a Did-you-mean fitted to a catalog internal is
/// dropped ([`CATALOG_INTERNAL_PREFIXES`]). Near-miss face suggestions
/// (`records` → `record`) are untouched.
fn teach(error: &str) -> String {
    // DuckDB's three view-DML spellings: UPDATE/DELETE answer a Binder
    // Error naming base tables; INSERT answers a Catalog Error ("doc is not
    // an table" — a name that exists but is not a base table).
    if error.contains("Can only update base table")
        || error.contains("Can only delete from base table")
        || error.contains("is not an table")
        || error.contains("is not a table")
    {
        return format!(
            "{error}\nThe latest layer is views over append-only history; \
             ephemeral DML is accepted against the hist tables instead \
             (e.g. UPDATE hist.doc ...) — visible to your own statement, \
             rolled back at call end, never durable."
        );
    }

    // The extension gate's own refusal (card
    // sql-extension-ddl-escapes-rollback-lane). DuckDB names the setting; the
    // caller needs the reason and the allow-list.
    if error.contains("Loading external extensions is disabled") {
        let allowed = EXTENSION_ALLOW_LIST.join(", ");
        return format!(
            "{error}\nThe query lane loads no extension on demand: an extension \
             loads onto the shared engine instance, OUTSIDE your statement's \
             transaction, so its catalog writes are not rolled back at call end \
             (duckpgq's CREATE PROPERTY GRAPH wrote sql.main.__duckpgq_internal \
             durably into the drawer). Loaded at open, and all there is: \
             {allowed}."
        );
    }

    // A suggestion fitted to a catalog internal is worse than none: it sends
    // the caller at metadata. Drop the clause, keep every other word.
    let out: String = error
        .lines()
        .filter(|line| !suggests_catalog_internal(line))
        .collect::<Vec<_>>()
        .join("\n");

    if let Some(name) = unknown_table(&out)
        && let Some((_, new)) = RETIRED_NAMES.iter().find(|(old, _)| *old == name)
    {
        return format!("{out}\n`{name}` was renamed to `{new}` — query `{new}` instead.");
    }
    out
}

/// The fingerprint-pinned append-only cache file, open read-write.
///
/// The holder is the file's single writer for its lifetime (`DuckDB`'s own
/// lock excludes every other process, read-only included — receipt P4).
/// Appends ride the base connection; caller queries run on
/// [`SqlStore::query`]'s clone through the rollback lane. Spill containment
/// ([`apply_spill_containment`]) is applied once at open — the `SET`s are
/// instance-global in `DuckDB`, so every connection inherits it.
pub struct SqlStore {
    conn: Connection,
    file: PathBuf,
}

impl std::fmt::Debug for SqlStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqlStore")
            .field("file", &self.file)
            .finish_non_exhaustive()
    }
}

impl SqlStore {
    /// Open the cache file at `file`, creating and initialising it when
    /// absent. A file that cannot be opened, has no readable pin ledger, or
    /// was written under a different [`CACHE_SCHEMA_VERSION`] is deleted and
    /// recreated empty — the cache is a pure function of the corpus, so
    /// deletion is always a correct repair. A file held by another process
    /// (the `DuckDB` lock) is an error — the caller degrades down its ladder.
    ///
    /// # Errors
    /// The file (or its recreation) cannot be opened — most commonly the
    /// `DuckDB` inter-process lock held elsewhere — or the DDL fails.
    pub fn open(file: &Path) -> Result<SqlStore, ViewError> {
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).map_err(ViewError::Io)?;
        }
        let fresh = !file.exists();
        let conn = match Connection::open(file) {
            Ok(conn) => conn,
            // Unopenable and absent-lock: a torn/corrupt file. The DuckDB
            // lock error must NOT be repaired by deletion (a live holder
            // owns the file) — it is detectable by its message.
            Err(e) => {
                if is_lock_error(&e) {
                    return Err(ViewError::Duckdb(e));
                }
                return Self::recreate(file);
            }
        };
        let store = SqlStore {
            conn,
            file: file.to_path_buf(),
        };
        apply_spill_containment(&store.conn, &store.spill_dir())?;
        apply_extension_gate(&store.conn)?;
        if fresh {
            store.conn.execute_batch(CACHE_SCHEMA_SQL)?;
            return Ok(store);
        }
        // Existing file: the pin ledger must be readable and this version.
        match store.pin() {
            Ok(Some(pin)) if pin.cache_schema_version == CACHE_SCHEMA_VERSION => Ok(store),
            // Empty-but-initialised (a cold build never landed): schema
            // readable, no pin row — usable as-is.
            Ok(None) => Ok(store),
            // Foreign schema version or unreadable ledger: delete, cold start.
            _ => {
                drop(store);
                Self::recreate(file)
            }
        }
    }

    /// Delete the file (and its WAL) and open a fresh, initialised one.
    fn recreate(file: &Path) -> Result<SqlStore, ViewError> {
        let _ = std::fs::remove_file(file);
        let _ = std::fs::remove_file(wal_path(file));
        let conn = Connection::open(file)?;
        conn.execute_batch(CACHE_SCHEMA_SQL)?;
        let store = SqlStore {
            conn,
            file: file.to_path_buf(),
        };
        apply_spill_containment(&store.conn, &store.spill_dir())?;
        apply_extension_gate(&store.conn)?;
        Ok(store)
    }

    /// The file's current pin, `None` for a fresh (never-appended) file.
    ///
    /// # Errors
    /// The pin ledger cannot be read (treated by [`SqlStore::open`] as a
    /// delete-and-recreate condition).
    pub fn pin(&self) -> Result<Option<Pin>, ViewError> {
        let mut stmt = self.conn.prepare(
            "SELECT gen, fingerprint, cache_schema_version, base_fold \
             FROM hist.pin ORDER BY gen DESC LIMIT 1",
        )?;
        let mut rows = stmt.query([])?;
        match rows.next()? {
            Some(row) => Ok(Some(Pin {
                generation: row.get(0)?,
                fingerprint: row.get(1)?,
                cache_schema_version: row.get(2)?,
                base_fold: row.get(3)?,
            })),
            None => Ok(None),
        }
    }

    /// Bring the file to `fingerprint`: no-op when already pinned there
    /// (`Ok(None)`), else ONE append transaction — added/changed docs' rows
    /// plus resolution-affected re-projections at `gen+1`, tombstones for
    /// removed paths, the pin row last (module docs § Delta grain). A crash
    /// mid-append rolls back whole: the file is at the old pin or the new
    /// pin, never between, and the retry re-derives the same delta.
    ///
    /// A pin whose fingerprint version prefix (`b3b:` …) differs from the
    /// live fingerprint's names a domain-rule change: the file is deleted and
    /// cold-rebuilt (same hex under a different prefix must never compare
    /// equal, so an append over it would version-mix).
    ///
    /// `docs`/`corpus`/`mounts`/`exclusion` are [`crate::build_memory_rooted`]'s
    /// inputs, whole — the delta needs the full corpus index for link
    /// resolution even though it re-projects only the delta docs.
    ///
    /// # Errors
    /// Any `DuckDB` failure (the transaction is rolled back first).
    pub fn sync(
        &mut self,
        docs: &model::Docs,
        corpus: &model::RootedCorpus<'_>,
        mounts: Option<&addr::MountSet>,
        exclusion: Option<ExclusionProbe<'_>>,
        fingerprint: &str,
        base: Option<&BaseWalk<'_>>,
    ) -> Result<Option<AppendCounts>, ViewError> {
        let base_fold = base.map(|b| b.fold);
        let mut pin = self.pin()?;
        if let Some(p) = &pin {
            // TWO witnesses, one no-op check (`base-projection.md` §7): base
            // motion appends even when the fingerprint did not move, so a
            // pinned fingerprint alone no longer proves the file is current.
            if p.fingerprint == fingerprint && p.base_fold.as_deref() == base_fold {
                return Ok(None);
            }
            if fingerprint_version(&p.fingerprint) != fingerprint_version(fingerprint) {
                *self = Self::recreate(&self.file.clone())?;
                pin = None;
            }
        }

        // The cache as its own manifest: latest live (path → file_rev).
        let manifest: BTreeMap<String, String> = {
            let mut stmt = self.conn.prepare("SELECT path, file_rev FROM doc")?;
            let mut rows = stmt.query([])?;
            let mut m = BTreeMap::new();
            while let Some(row) = rows.next()? {
                m.insert(row.get::<_, String>(0)?, row.get::<_, String>(1)?);
            }
            m
        };

        let mut added: BTreeSet<&str> = BTreeSet::new();
        let mut changed: BTreeSet<&str> = BTreeSet::new();
        for (path, doc) in docs {
            match manifest.get(path) {
                None => {
                    added.insert(path);
                }
                Some(rev) if *rev != doc.root.node_rev.0 => {
                    changed.insert(path);
                }
                Some(_) => {}
            }
        }
        let removed: Vec<&str> = manifest
            .keys()
            .filter(|p| !docs.contains_key(*p))
            .map(String::as_str)
            .collect();

        // The base plane's own delta, against the cache's own base manifest.
        let base_delta = self.base_delta(base)?;

        // Unchanged docs whose link resolution the delta can move — on either
        // plane. A base member appearing, vanishing, or shifting a tie-break
        // moves what `exclusion`/`exclusion_path` say in docs that did not
        // move (§7).
        let affected = if pin.is_none() {
            BTreeSet::new()
        } else {
            let mut affected = self.resolution_affected(docs, &added, &changed, &removed)?;
            affected.extend(self.base_affected(docs, &base_delta)?);
            affected.retain(|p| !added.contains(p.as_str()) && !changed.contains(p.as_str()));
            affected
        };

        let counts = AppendCounts {
            generation: pin.as_ref().map_or(0, |p| p.generation) + 1,
            added: u64_len(added.len()),
            changed: u64_len(changed.len() + affected.len()),
            removed: u64_len(removed.len()),
        };

        // Stage the re-projection set with the FULL corpus index (link
        // resolution is corpus-global even when the projected set is not).
        let mut reproject: BTreeSet<String> = added.iter().map(|p| (*p).to_owned()).collect();
        reproject.extend(changed.iter().map(|p| (*p).to_owned()));
        reproject.extend(affected.iter().cloned());
        let index = corpus_index(docs);
        let mut rows = Rows::default();
        for path in &reproject {
            if let Some(doc) = docs.get(path) {
                collect_doc(path, doc, &index, &mut rows, corpus, mounts);
            }
        }
        fill_exclusions(&mut rows, exclusion);
        // Only the moved members re-project; unmoved ones keep their pinned
        // generation, exactly as an unmoved doc does.
        crate::collect_base(&base_delta.reproject, &mut rows);

        self.append(&rows, &removed, counts, fingerprint, &base_delta, base_fold)?;
        Ok(Some(counts))
    }

    /// The base plane's delta against the cache's own base manifest
    /// (`base-projection.md` §7): the latest `base(path, file_rev)` map diffed
    /// against the live walk, exactly as `doc(path, file_rev)` is diffed
    /// against the live parsed corpus.
    ///
    /// A member with a NULL `file_rev` (unreadable, §4.4) compares unequal to
    /// everything, so it re-reads at every sync until it heals — deliberate,
    /// and the reason `file_rev` is `Option` here rather than a sentinel
    /// string that could accidentally match.
    ///
    /// Handed no walk, the base plane is NOT ASKED: nothing is re-projected and
    /// nothing is tombstoned. An absent walk must never read as an empty one,
    /// or the next append would tombstone every member the workspace still has.
    fn base_delta(&self, base: Option<&BaseWalk<'_>>) -> Result<BaseDelta, ViewError> {
        let Some(base) = base else {
            return Ok(BaseDelta::default());
        };
        let manifest: BTreeMap<String, Option<String>> = {
            let mut stmt = self.conn.prepare("SELECT path, file_rev FROM base")?;
            let mut rows = stmt.query([])?;
            let mut m = BTreeMap::new();
            while let Some(row) = rows.next()? {
                m.insert(row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?);
            }
            m
        };

        let mut delta = BaseDelta::default();
        for member in base.members {
            let live_rev = member
                .bytes
                .as_ref()
                .ok()
                .map(|raw| crate::hex16(&model::leaf_digest(raw)));
            let moved = match manifest.get(&member.path) {
                None => true,
                // NULL on either side never compares equal: an unreadable
                // member re-reads until it heals.
                Some(pinned) => match (pinned, &live_rev) {
                    (Some(pinned), Some(live)) => pinned != live,
                    _ => true,
                },
            };
            if moved {
                delta.moved_keys.insert(member.path.clone());
                delta.reproject.push(crate::BaseMember {
                    path: member.path.clone(),
                    bytes: member.bytes.clone(),
                });
            }
        }
        let live: BTreeSet<&str> = base.members.iter().map(|m| m.path.as_str()).collect();
        for path in manifest.keys() {
            if !live.contains(path.as_str()) {
                delta.moved_keys.insert(path.clone());
                delta.removed.push(path.clone());
            }
        }
        Ok(delta)
    }

    /// Docs whose `exclusion` / `exclusion_path` the base delta can move
    /// (`base-projection.md` §7) — over ALL link rows, not the dangling ones.
    ///
    /// The rows a base delta moves divide in two, and the second class is by
    /// definition NOT dangling: (i) unresolved, UNEXPLAINED rows an appearing
    /// member can newly stamp; (ii) already-STAMPED rows whose stamp a removal
    /// must clear or a tie-break shift must re-point. Deleting
    /// `bases/TAG-FILES.base` must un-stamp its 367 embed rows — under a
    /// dangling-only predicate they would stay stamped forever while the cache
    /// reported itself fresh, repairable only by rebuild.
    ///
    /// So a doc re-projects when any of its link rows matches, **case-exact**,
    /// on EITHER the target's own key (bare basename, or literal path) OR the
    /// row's `exclusion_path` (full path, or its basename). This key set is
    /// SEPARATE from [`Self::resolution_affected`]'s deliberately-lowercased md
    /// keys: folding them together would reintroduce through the back door
    /// exactly the case-folding §3 and §5.1 forbid.
    fn base_affected(
        &self,
        docs: &model::Docs,
        delta: &BaseDelta,
    ) -> Result<BTreeSet<String>, ViewError> {
        if delta.moved_keys.is_empty() {
            return Ok(BTreeSet::new());
        }
        // Each moved member contributes both of its name-keys, case-exact.
        let mut keys: BTreeSet<&str> = BTreeSet::new();
        for path in &delta.moved_keys {
            keys.insert(path.as_str());
            if let Some(stem) = path.rsplit('/').next() {
                keys.insert(stem);
            }
        }

        let mut affected = BTreeSet::new();
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT src_path, target_raw, exclusion_path FROM link")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let src: String = row.get(0)?;
            if affected.contains(&src) || !docs.contains_key(&src) {
                continue;
            }
            let target: String = row.get(1)?;
            let target = target.trim();
            let stamped: Option<String> = row.get(2)?;
            let hit = keys.contains(target)
                || target.rsplit('/').next().is_some_and(|b| keys.contains(b))
                || stamped.is_some_and(|p| {
                    keys.contains(p.as_str())
                        || p.rsplit('/').next().is_some_and(|b| keys.contains(b))
                });
            if hit {
                affected.insert(src);
            }
        }
        Ok(affected)
    }

    /// Unchanged docs holding a link whose target names a moved name: the
    /// name keys of added/removed paths (stem + full key) plus the aliases
    /// entering or leaving through added/changed/removed docs, matched
    /// against every latest link row's target keys (module docs § Delta
    /// grain — a superset of the resolver's keys, never a subset).
    fn resolution_affected(
        &self,
        docs: &model::Docs,
        added: &BTreeSet<&str>,
        changed: &BTreeSet<&str>,
        removed: &[&str],
    ) -> Result<BTreeSet<String>, ViewError> {
        let mut names: BTreeSet<String> = BTreeSet::new();
        for path in added.iter().chain(removed.iter()) {
            let key = path.trim_end_matches(".md").to_lowercase();
            if let Some(stem) = key.rsplit('/').next() {
                names.insert(stem.to_owned());
            }
            names.insert(key);
        }
        // New aliases: added + changed docs, from the parsed corpus.
        for path in added.iter().chain(changed.iter()) {
            if let Some(doc) = docs.get(*path) {
                names.extend(model::doc_aliases(doc));
            }
        }
        // Old aliases: changed + removed docs, from the cache's latest layer.
        {
            let mut stmt = self
                .conn
                .prepare("SELECT path, value FROM frontmatter WHERE key IN ('alias','aliases')")?;
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                let path: String = row.get(0)?;
                if changed.contains(path.as_str()) || removed.contains(&path.as_str()) {
                    let value: String = row.get(1)?;
                    names.extend(model::parse_alias_list(&value));
                }
            }
        }
        if names.is_empty() {
            return Ok(BTreeSet::new());
        }

        let mut affected = BTreeSet::new();
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT src_path, target_raw FROM link")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let src: String = row.get(0)?;
            if added.contains(src.as_str())
                || changed.contains(src.as_str())
                || removed.contains(&src.as_str())
                || !docs.contains_key(&src)
            {
                continue;
            }
            let target: String = row.get(1)?;
            let key = target.trim().trim_end_matches(".md").to_lowercase();
            let base = key.rsplit('/').next().unwrap_or(key.as_str());
            if names.contains(base) || names.contains(&key) {
                affected.insert(src);
            }
        }
        Ok(affected)
    }

    /// One append transaction: hist rows at `counts.gen`, tombstones,
    /// then the pin row — commit, or roll back whole.
    fn append(
        &self,
        rows: &Rows,
        removed: &[&str],
        counts: AppendCounts,
        fingerprint: &str,
        base_delta: &BaseDelta,
        base_fold: Option<&str>,
    ) -> Result<(), ViewError> {
        self.conn.execute_batch("BEGIN")?;
        let result = self.append_body(rows, removed, counts, fingerprint, base_delta, base_fold);
        match result {
            Ok(()) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(())
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    fn append_body(
        &self,
        rows: &Rows,
        removed: &[&str],
        counts: AppendCounts,
        fingerprint: &str,
        base_delta: &BaseDelta,
        base_fold: Option<&str>,
    ) -> Result<(), ViewError> {
        let generation = Value::BigInt(counts.generation);

        // doc: path, generation, tombstone, rest. Live rows first, then tombstones.
        {
            let mut app = self.conn.appender_to_db("doc", "hist")?;
            for row in &rows.doc {
                let mut r: Vec<Value> = Vec::with_capacity(row.len() + 2);
                r.push(row[0].clone());
                r.push(generation.clone());
                r.push(Value::Boolean(false));
                r.extend(row[1..].iter().cloned());
                app.append_row(duckdb::appender_params_from_iter(r.iter()))?;
            }
            for path in removed {
                let r = [
                    Value::Text((*path).to_owned()),
                    generation.clone(),
                    Value::Boolean(true),
                    Value::Null,
                    Value::Null,
                    Value::Null,
                ];
                app.append_row(duckdb::appender_params_from_iter(r.iter()))?;
            }
            app.flush()?;
        }

        self.append_scalar(&rows.frontmatter, "frontmatter", &generation)?;
        self.append_scalar(&rows.link, "link", &generation)?;
        self.append_scalar(&rows.tag, "tag", &generation)?;
        self.append_scalar(&rows.frontmatter_tag, "frontmatter_tag", &generation)?;
        self.append_scalar(&rows.section, "section", &generation)?;
        self.append_scalar(&rows.task, "task", &generation)?;
        self.append_body_chunks(&rows.body, &generation)?;

        // base: path, generation, tombstone, rest — the hist.doc shape, so the
        // latest-pick window and the tombstone rule read identically.
        {
            let mut app = self.conn.appender_to_db("base", "hist")?;
            for row in &rows.base {
                let mut r: Vec<Value> = Vec::with_capacity(row.len() + 2);
                r.push(row[0].clone());
                r.push(generation.clone());
                r.push(Value::Boolean(false));
                r.extend(row[1..].iter().cloned());
                app.append_row(duckdb::appender_params_from_iter(r.iter()))?;
            }
            for path in &base_delta.removed {
                let r = [
                    Value::Text(path.clone()),
                    generation.clone(),
                    Value::Boolean(true),
                    Value::Null,
                    Value::Null,
                    Value::Null,
                    Value::Null,
                    Value::Null,
                    Value::Null,
                ];
                app.append_row(duckdb::appender_params_from_iter(r.iter()))?;
            }
            app.flush()?;
        }
        self.append_scalar(&rows.base_view, "base_view", &generation)?;
        self.append_scalar(&rows.base_formula, "base_formula", &generation)?;

        self.conn.execute(
            "INSERT INTO hist.pin \
             (gen, fingerprint, applied_at, files_added, files_changed, files_removed, \
              engine_version, cache_schema_version, base_fold) \
             VALUES (?, ?, now(), ?, ?, ?, ?, ?, ?)",
            duckdb::params_from_iter(
                [
                    generation,
                    Value::Text(fingerprint.to_owned()),
                    Value::BigInt(i64_len(counts.added)),
                    Value::BigInt(i64_len(counts.changed)),
                    Value::BigInt(i64_len(counts.removed)),
                    Value::Text(env!("CARGO_PKG_VERSION").to_owned()),
                    Value::BigInt(CACHE_SCHEMA_VERSION),
                    base_fold.map_or(Value::Null, |f| Value::Text(f.to_owned())),
                ]
                .iter(),
            ),
        )?;
        Ok(())
    }

    /// Appender load of the body split (`docs/body-projection.md` §4): each
    /// staged row `(path, seq, section_seq, hpath, text, span_start, span_end,
    /// node_rev)` becomes one narrow `hist.body` row carrying `body_key`
    /// (full-hex blake3 of the chunk bytes) in place of `text`, plus one
    /// `hist.body_text` row when the key is new to this file's history. The
    /// read-then-insert is race-free by the single-appender protocol — the
    /// holder is the file's only writer — and stays INSERT-only, so the
    /// never-edit law holds.
    fn append_body_chunks(&self, rows: &[Vec<Value>], generation: &Value) -> Result<(), ViewError> {
        if rows.is_empty() {
            return Ok(());
        }
        let mut known: BTreeSet<String> = {
            let mut stmt = self.conn.prepare("SELECT body_key FROM hist.body_text")?;
            let mut got = stmt.query([])?;
            let mut keys = BTreeSet::new();
            while let Some(row) = got.next()? {
                keys.insert(row.get::<_, String>(0)?);
            }
            keys
        };
        let mut narrow = self.conn.appender_to_db("body", "hist")?;
        let mut texts = self.conn.appender_to_db("body_text", "hist")?;
        for row in rows {
            let Value::Text(text) = &row[4] else {
                unreachable!("body row text column is TEXT by construction");
            };
            let key = blake3::hash(text.as_bytes()).to_hex().to_string();
            let r = [
                row[0].clone(),     // path
                generation.clone(), // gen
                row[1].clone(),     // seq
                row[2].clone(),     // section_seq
                row[3].clone(),     // hpath
                row[5].clone(),     // span_start
                row[6].clone(),     // span_end
                row[7].clone(),     // node_rev
                Value::Text(key.clone()),
            ];
            narrow.append_row(duckdb::appender_params_from_iter(r.iter()))?;
            if known.insert(key.clone()) {
                let t = [Value::Text(key), Value::Text(text.clone())];
                texts.append_row(duckdb::appender_params_from_iter(t.iter()))?;
            }
        }
        narrow.flush()?;
        texts.flush()?;
        Ok(())
    }

    /// Appender load of one all-scalar hist table: first column, generation, rest.
    fn append_scalar(
        &self,
        rows: &[Vec<Value>],
        table: &str,
        generation: &Value,
    ) -> Result<(), ViewError> {
        let mut app = self.conn.appender_to_db(table, "hist")?;
        for row in rows {
            let mut r: Vec<Value> = Vec::with_capacity(row.len() + 1);
            r.push(row[0].clone());
            r.push(generation.clone());
            r.extend(row[1..].iter().cloned());
            app.append_row(duckdb::appender_params_from_iter(r.iter()))?;
        }
        app.flush()?;
        Ok(())
    }

    /// Run one caller query through the rollback lane on a clone of the base
    /// connection: `BEGIN → statement → collect → ROLLBACK`. One execution
    /// path for every caller (the NO-SANDBOX ruling, 2026-08-14): no profile,
    /// no lock — spill containment is already in force instance-wide from
    /// open.
    ///
    /// # Errors
    /// A connection failure is a `ViewError`; the caller's own SQL failing is
    /// `Ok` with the error string in the result (their register, not ours) —
    /// including `DuckDB`'s MVCC transaction conflicts, verbatim.
    #[allow(clippy::type_complexity)]
    pub fn query(
        &self,
        sql: &str,
    ) -> Result<Result<(Vec<ColMeta>, Vec<Vec<Json>>), String>, ViewError> {
        let conn = self.conn.try_clone()?;
        conn.execute_batch("BEGIN")?;
        let result = run_query(&conn, sql);
        // Always roll back — reads are unaffected; DML dies here (P3/P6).
        conn.execute_batch("ROLLBACK")?;
        Ok(result)
    }

    /// The explicit rebuild verb (ruling OQ3), which doubles as the repair
    /// path: delete the file and open a fresh, initialised one. The next
    /// [`SqlStore::sync`] is the cold build. Rebuild-and-swap is the ONLY
    /// compaction — in-place vacuum would violate never-edit.
    ///
    /// # Errors
    /// As [`SqlStore::open`] — and a file HELD by another process (the
    /// resident daemon) refuses instead of unlinking under it: unlink would
    /// succeed on unix, forking the holder onto a dead inode while a new
    /// file grows beside it.
    pub fn rebuild(file: &Path) -> Result<SqlStore, ViewError> {
        if file.exists() {
            // Prove the file is unheld before deleting it.
            drop(Connection::open(file)?);
        }
        Self::recreate(file)
    }

    /// The cache file this store holds open.
    #[must_use]
    pub fn file(&self) -> &Path {
        &self.file
    }

    /// The drawer-derived spill directory this store's instance is pointed at
    /// (card sql-spill-config-lockout).
    #[must_use]
    pub fn spill_dir(&self) -> PathBuf {
        self.file
            .parent()
            .map_or_else(|| PathBuf::from("sql-spill"), |p| p.join("sql-spill"))
    }

    /// The base connection — appends, pin reads, tests.
    #[must_use]
    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}

/// The `<file>.wal` sibling `DuckDB` keeps beside a database file.
fn wal_path(file: &Path) -> PathBuf {
    let mut os = file.as_os_str().to_owned();
    os.push(".wal");
    PathBuf::from(os)
}

/// The fingerprint's domain-rule version prefix (`b3b:` → `b3b`). A missing
/// separator yields the whole string, which then simply never matches a
/// prefixed one.
fn fingerprint_version(fingerprint: &str) -> &str {
    fingerprint.split(':').next().unwrap_or(fingerprint)
}

/// Is this `DuckDB` open error the inter-process file lock (receipt P4)?
/// A held lock must degrade down the caller's ladder, never delete the file.
fn is_lock_error(e: &duckdb::Error) -> bool {
    e.to_string().contains("Conflicting lock")
}

fn u64_len(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

fn i64_len(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// A friendly type name for an arrow result column (best effort). The card
/// sql-scalar-type-render families name themselves in `DuckDB`'s own words —
/// the `Debug` fallback survives only for shapes `DuckDB`'s arrow export
/// cannot produce.
fn arrow_type_name(dt: &duckdb::arrow::datatypes::DataType) -> String {
    use duckdb::arrow::datatypes::{DataType as D, TimeUnit as U};
    match dt {
        D::Boolean => "BOOLEAN".to_owned(),
        D::Int8 => "TINYINT".to_owned(),
        D::Int16 => "SMALLINT".to_owned(),
        D::Int32 => "INTEGER".to_owned(),
        D::Int64 => "BIGINT".to_owned(),
        D::UInt8 => "UTINYINT".to_owned(),
        D::UInt16 => "USMALLINT".to_owned(),
        D::UInt32 => "UINTEGER".to_owned(),
        D::UInt64 => "UBIGINT".to_owned(),
        D::Float32 => "FLOAT".to_owned(),
        D::Float64 => "DOUBLE".to_owned(),
        D::Utf8 | D::LargeUtf8 => "VARCHAR".to_owned(),
        D::Binary | D::LargeBinary => "BLOB".to_owned(),
        D::List(_) | D::LargeList(_) => "LIST".to_owned(),
        D::Timestamp(_, Some(_)) => "TIMESTAMP WITH TIME ZONE".to_owned(),
        D::Timestamp(U::Second, None) => "TIMESTAMP_S".to_owned(),
        D::Timestamp(U::Millisecond, None) => "TIMESTAMP_MS".to_owned(),
        D::Timestamp(U::Microsecond, None) => "TIMESTAMP".to_owned(),
        D::Timestamp(U::Nanosecond, None) => "TIMESTAMP_NS".to_owned(),
        D::Date32 => "DATE".to_owned(),
        D::Time64(_) => "TIME".to_owned(),
        D::Interval(_) => "INTERVAL".to_owned(),
        // HUGEINT crosses arrow as exactly Decimal128(38, 0).
        D::Decimal128(38, 0) => "HUGEINT".to_owned(),
        D::Decimal128(w, s) | D::Decimal256(w, s) => format!("DECIMAL({w},{s})"),
        D::Struct(_) => "STRUCT".to_owned(),
        D::Map(..) => "MAP".to_owned(),
        D::Dictionary(..) => "ENUM".to_owned(),
        D::Union(..) => "UNION".to_owned(),
        D::FixedSizeList(..) => "ARRAY".to_owned(),
        other => format!("{other:?}"),
    }
}

/// Render one result cell into JSON. Scalars are exact; the temporal,
/// decimal, and container families speak `DuckDB`'s own SQL text
/// ([`crate::sqltext`], card sql-scalar-type-render — no `Debug` fallback
/// survives: the match is exhaustive so a new `ValueRef` variant fails the
/// build instead of leaking a repr); list/array cells slice out their own
/// row's elements (F1) and stay JSON arrays for the face's ruled ` / ` join.
/// `tz_col` marks a `TIMESTAMP WITH TIME ZONE` column ([`run_query`] reads
/// it off the arrow schema): the value is UTC and renders with the `+00`
/// marker, exactly as this ICU-less engine's own `::VARCHAR` cast would.
fn value_ref_to_json(v: duckdb::types::ValueRef<'_>, tz_col: bool) -> Json {
    use duckdb::types::ValueRef;
    match v {
        ValueRef::Null => Json::Null,
        ValueRef::Boolean(b) => Json::Bool(b),
        ValueRef::TinyInt(n) => Json::from(i64::from(n)),
        ValueRef::SmallInt(n) => Json::from(i64::from(n)),
        ValueRef::Int(n) => Json::from(i64::from(n)),
        ValueRef::BigInt(n) => Json::from(n),
        ValueRef::HugeInt(n) => {
            i64::try_from(n).map_or_else(|_| Json::String(n.to_string()), Json::from)
        }
        ValueRef::UTinyInt(n) => Json::from(u64::from(n)),
        ValueRef::USmallInt(n) => Json::from(u64::from(n)),
        ValueRef::UInt(n) => Json::from(u64::from(n)),
        ValueRef::UBigInt(n) => Json::from(n),
        ValueRef::Float(f) => json_f64(f64::from(f)),
        ValueRef::Double(f) => json_f64(f),
        ValueRef::Decimal(d) => Json::String(d.to_string()),
        ValueRef::Timestamp(unit, t) => Json::String(sqltext::timestamp_text(unit, t, tz_col)),
        ValueRef::Date32(d) => Json::String(sqltext::date_text(d)),
        ValueRef::Time64(unit, t) => Json::String(sqltext::time_text(unit, t)),
        ValueRef::Interval {
            months,
            days,
            nanos,
        } => Json::String(sqltext::interval_text(months, days, nanos)),
        ValueRef::Text(bytes) => Json::String(String::from_utf8_lossy(bytes).into_owned()),
        ValueRef::Blob(bytes) => Json::String(hex(bytes)),
        // A list cell's ValueRef carries the WHOLE column array + a row index;
        // converting per-cell would dump every row's values into every cell
        // (F1). The owned conversion slices out this row's elements — and the
        // enum/struct/map/union families ride the same owned lane.
        ValueRef::List(..)
        | ValueRef::Array(..)
        | ValueRef::Enum(..)
        | ValueRef::Struct(..)
        | ValueRef::Map(..)
        | ValueRef::Union(..) => duck_value_to_json(&Value::from(v)),
    }
}

/// An owned duckdb value into JSON — list/array elements and union members
/// after the row slice. Scalars render in `DuckDB`'s RAW form (a nested
/// timestamptz has lost its column's tz flag, so it renders unmarked — the
/// one documented divergence); struct/map cells become their `DuckDB` text.
/// Exhaustive for the same reason as [`value_ref_to_json`].
fn duck_value_to_json(v: &Value) -> Json {
    match v {
        Value::Null => Json::Null,
        Value::Boolean(b) => Json::Bool(*b),
        Value::TinyInt(n) => Json::from(i64::from(*n)),
        Value::SmallInt(n) => Json::from(i64::from(*n)),
        Value::Int(n) => Json::from(i64::from(*n)),
        Value::BigInt(n) => Json::from(*n),
        Value::HugeInt(n) => {
            i64::try_from(*n).map_or_else(|_| Json::String(n.to_string()), Json::from)
        }
        Value::UTinyInt(n) => Json::from(u64::from(*n)),
        Value::USmallInt(n) => Json::from(u64::from(*n)),
        Value::UInt(n) => Json::from(u64::from(*n)),
        Value::UBigInt(n) => Json::from(*n),
        Value::Float(f) => json_f64(f64::from(*f)),
        Value::Double(f) => json_f64(*f),
        Value::Decimal(d) => Json::String(d.to_string()),
        Value::Timestamp(unit, t) => Json::String(sqltext::timestamp_text(*unit, *t, false)),
        Value::Date32(d) => Json::String(sqltext::date_text(*d)),
        Value::Time64(unit, t) => Json::String(sqltext::time_text(*unit, *t)),
        Value::Interval {
            months,
            days,
            nanos,
        } => Json::String(sqltext::interval_text(*months, *days, *nanos)),
        Value::Text(s) | Value::Enum(s) => Json::String(s.clone()),
        Value::Blob(bytes) => Json::String(hex(bytes)),
        Value::List(items) | Value::Array(items) => {
            Json::Array(items.iter().map(duck_value_to_json).collect())
        }
        Value::Struct(_) | Value::Map(_) => Json::String(sqltext::raw_text(v)),
        Value::Union(member) => duck_value_to_json(member),
    }
}

/// A finite `f64` as a JSON number; NaN/±Inf render as `null`.
fn json_f64(f: f64) -> Json {
    serde_json::Number::from_f64(f).map_or(Json::Null, Json::Number)
}

/// Lowercase hex of a byte slice (BLOB rendering).
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use model::RootedCorpus;

    /// Parse a fixture corpus.
    pub(crate) fn corpus(files: &[(&str, &str)]) -> model::Docs {
        files
            .iter()
            .map(|(p, raw)| {
                (
                    (*p).to_string(),
                    std::sync::Arc::new(model::build((*raw).to_string(), syntax::parse(raw))),
                )
            })
            .collect()
    }

    /// Sync `docs` into `store` at `fingerprint` (ambient corpus, no mounts —
    /// mirrors the [`crate::build_memory`] reference build).
    pub(crate) fn sync_ambient(
        store: &mut SqlStore,
        docs: &model::Docs,
        fingerprint: &str,
    ) -> Option<AppendCounts> {
        let ambient = RootedCorpus::ambient(docs);
        store
            .sync(docs, &ambient, None, None, fingerprint, None)
            .expect("sync")
    }

    /// Per-surface digest over every column of every row — the 8 base tables
    /// plus the 4 convenience views, so the acceptance is the whole caller
    /// surface, not just storage.
    const SURFACE_DIGESTS: [(&str, &str); 12] = [
        (
            "doc",
            "SELECT coalesce(md5(string_agg(path || '|' || file_rev || '|' || line_count::VARCHAR || '|' || bytes::VARCHAR, chr(10) ORDER BY path)), 'EMPTY') FROM doc",
        ),
        (
            "frontmatter",
            "SELECT coalesce(md5(string_agg(path || '|' || ord::VARCHAR || '|' || key || '|' || value || '|' || span_start::VARCHAR || '|' || span_end::VARCHAR || '|' || node_rev, chr(10) ORDER BY path, key)), 'EMPTY') FROM frontmatter",
        ),
        (
            "section",
            "SELECT coalesce(md5(string_agg(path || '|' || node_seq::VARCHAR || '|' || hpath || '|' || coalesce(n::VARCHAR,'~N~') || '|' || heading || '|' || level::VARCHAR || '|' || node_rev || '|' || span_start::VARCHAR || '|' || span_end::VARCHAR, chr(10) ORDER BY path, node_seq)), 'EMPTY') FROM section",
        ),
        (
            "link",
            "SELECT coalesce(md5(string_agg(src_path || '|' || seq::VARCHAR || '|' || kind || '|' || target_raw || '|' || coalesce(heading,'~N~') || '|' || coalesce(block,'~N~') || '|' || coalesce(alias,'~N~') || '|' || coalesce(dest_path,'~N~') || '|' || coalesce(dest_root,'~N~') || '|' || coalesce(dest_root_path,'~N~') || '|' || coalesce(exclusion,'~N~') || '|' || coalesce(exclusion_path,'~N~') || '|' || resolved::VARCHAR || '|' || span_start::VARCHAR || '|' || span_end::VARCHAR || '|' || node_rev, chr(10) ORDER BY src_path, seq)), 'EMPTY') FROM link",
        ),
        (
            "tag",
            "SELECT coalesce(md5(string_agg(path || '|' || seq::VARCHAR || '|' || tag || '|' || span_start::VARCHAR || '|' || span_end::VARCHAR || '|' || node_rev, chr(10) ORDER BY path, seq)), 'EMPTY') FROM tag",
        ),
        (
            "frontmatter_tag",
            "SELECT coalesce(md5(string_agg(path || '|' || seq::VARCHAR || '|' || tag || '|' || key || '|' || span_start::VARCHAR || '|' || span_end::VARCHAR || '|' || node_rev, chr(10) ORDER BY path, key, seq)), 'EMPTY') FROM frontmatter_tag",
        ),
        (
            "task",
            "SELECT coalesce(md5(string_agg(path || '|' || seq::VARCHAR || '|' || checked::VARCHAR || '|' || depth::VARCHAR || '|' || coalesce(section_seq::VARCHAR,'~N~') || '|' || coalesce(hpath,'~N~') || '|' || text || '|' || span_start::VARCHAR || '|' || span_end::VARCHAR || '|' || node_rev, chr(10) ORDER BY path, seq)), 'EMPTY') FROM task",
        ),
        (
            "body",
            "SELECT coalesce(md5(string_agg(path || '|' || seq::VARCHAR || '|' || coalesce(section_seq::VARCHAR,'~N~') || '|' || coalesce(hpath,'~N~') || '|' || text || '|' || span_start::VARCHAR || '|' || span_end::VARCHAR || '|' || coalesce(node_rev,'~N~'), chr(10) ORDER BY path, seq)), 'EMPTY') FROM body",
        ),
        (
            "backlink",
            "SELECT coalesce(md5(string_agg(path || '|' || src_path || '|' || kind || '|' || coalesce(alias,'~N~'), chr(10) ORDER BY path, src_path, kind)), 'EMPTY') FROM backlink",
        ),
        (
            "dangling",
            "SELECT coalesce(md5(string_agg(src_path || '|' || target_raw, chr(10) ORDER BY src_path, target_raw)), 'EMPTY') FROM dangling",
        ),
        (
            "record",
            "SELECT coalesce(md5(string_agg(path || '|' || coalesce(type,'~N~') || '|' || coalesce(status,'~N~') || '|' || coalesce(owner,'~N~') || '|' || coalesce(session,'~N~'), chr(10) ORDER BY path)), 'EMPTY') FROM record",
        ),
        (
            "tag_all",
            "SELECT coalesce(md5(string_agg(path || '|' || tag || '|' || source || '|' || span_start::VARCHAR || '|' || span_end::VARCHAR || '|' || node_rev, chr(10) ORDER BY path, tag, source, span_start)), 'EMPTY') FROM tag_all",
        ),
    ];

    fn surface_digests(conn: &Connection) -> Vec<(&'static str, String)> {
        SURFACE_DIGESTS
            .iter()
            .map(|(name, sql)| {
                let d: String = conn
                    .query_row(sql, [], |r| r.get(0))
                    .unwrap_or_else(|e| panic!("digest {name}: {e}"));
                (*name, d)
            })
            .collect()
    }

    /// Assert the cache's caller surface equals a fresh ephemeral build of the
    /// same docs — the card-1/-2 acceptance, per surface.
    fn assert_surface_matches_fresh(store: &SqlStore, docs: &model::Docs) {
        let fresh = crate::build_memory(docs, "b3b:reference").expect("fresh build");
        assert_eq!(
            surface_digests(store.connection()),
            surface_digests(&fresh),
            "cache latest views must be bit-identical to a fresh ephemeral build"
        );
    }

    pub(crate) fn tmp_store(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join(SQL_CACHE_FILENAME)
    }

    /// The dogfood-shaped fixture: frontmatter tags + alias, links (resolved,
    /// dangling, embed), tasks (document-level and governed), the `['']`
    /// heading hazard, and a card-shaped doc.
    pub(crate) fn fixture_v1() -> model::Docs {
        corpus(&[
            (
                "a.md",
                "---\ntitle: Alpha\ntags: [x/one, x/two]\n---\n- [ ] doc-level task\n\n# Top\nintro\n## Sub\n- [x] task under Sub\n\nsee [[b]] and [[missing]] and ![[b]]\n",
            ),
            (
                "b.md",
                "#\n- [ ] task under the empty heading\nsee [[Fancy Name]] and [[newdoc]]\n",
            ),
            (
                "cards/c.md",
                "---\ntype: task\nstatus: todo\nowner: w1\nsession: s\n---\n# C\nbody [[a]]\n",
            ),
        ])
    }

    /// Every relation's column list, schema-qualified, as `DuckDB`'s catalog
    /// reports it — the caller's self-service discovery route.
    fn columns_by_relation(conn: &Connection) -> Vec<(String, String, String)> {
        let mut stmt = conn
            .prepare(
                "SELECT table_schema, table_name, \
                        string_agg(column_name, ', ' ORDER BY ordinal_position) \
                 FROM information_schema.columns \
                 GROUP BY table_schema, table_name \
                 ORDER BY table_schema, table_name",
            )
            .expect("prepare catalog query");
        let mut rows = stmt.query([]).expect("catalog query");
        let mut out = Vec::new();
        while let Some(r) = rows.next().expect("catalog row") {
            out.push((
                r.get(0).expect("schema"),
                r.get(1).expect("name"),
                r.get(2).expect("columns"),
            ));
        }
        out
    }

    /// Just the caller-facing `main` schema, as `(relation, column list)`.
    fn main_face(conn: &Connection) -> Vec<(String, String)> {
        columns_by_relation(conn)
            .into_iter()
            .filter(|(schema, ..)| schema == "main")
            .map(|(_, name, columns)| (name, columns))
            .collect()
    }

    /// The cache file's caller-facing `main` face is exactly the ephemeral
    /// build's face — same relations, same column lists, in order. The stamp
    /// `_meridian_view` is the one deliberate divergence (a view over the pin
    /// ledger here, a singleton table there), so it is compared by name only.
    ///
    /// The guard the card sql-information-schema-doubling asked for: a
    /// projection that arrived with a column twice, dropped one, or drifted
    /// from the ephemeral lane fails here. It also pins the discriminator
    /// behind the reported "doubling": `hist` is a SEPARATE schema carrying
    /// same-named tables (`doc`, `section`, `task`), so a caller aggregate
    /// that groups by `table_name` alone merges two real relations into one
    /// false list. Qualified by schema — the route below — every column is
    /// reported exactly once.
    #[test]
    fn main_face_columns_match_the_ephemeral_build_exactly() {
        let docs = fixture_v1();
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut store = SqlStore::open(&tmp_store(&dir)).expect("open");
        sync_ambient(&mut store, &docs, "b3b:v1").expect("cold build");

        let cached = main_face(store.connection());
        let fresh = crate::build_memory(&docs, "b3b:reference").expect("fresh build");
        let ephemeral = main_face(&fresh);

        // The ephemeral lane has no hist twin, so its face IS the contract.
        assert_eq!(
            cached.iter().map(|(n, _)| n).collect::<Vec<_>>(),
            ephemeral.iter().map(|(n, _)| n).collect::<Vec<_>>(),
            "the cache must serve the ephemeral build's relations"
        );
        for ((name, cols), (_, ref_cols)) in cached.iter().zip(&ephemeral) {
            if name == "_meridian_view" {
                continue; // the ruled divergence (see doc comment)
            }
            assert_eq!(cols, ref_cols, "main.{name} columns drifted from the build");
        }
        // 12 md relations + the three `.base` relations (`base-projection.md`
        // §4) + `body` (`body-projection.md`), all under the same
        // latest-view protocol.
        assert_eq!(cached.len(), 16, "16 caller-facing relations");

        // Each column once, per relation — the literal card assertion.
        for (name, columns) in &cached {
            let listed: Vec<&str> = columns.split(", ").collect();
            let unique: BTreeSet<&str> = listed.iter().copied().collect();
            assert_eq!(
                listed.len(),
                unique.len(),
                "main.{name} reports a column more than once: {columns}"
            );
        }
    }

    #[test]
    fn cold_build_matches_ephemeral_projection_bit_identically() {
        let docs = fixture_v1();
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut store = SqlStore::open(&tmp_store(&dir)).expect("open");

        let counts = sync_ambient(&mut store, &docs, "b3b:v1").expect("cold build appends");
        assert_eq!(counts.generation, 1);
        assert_eq!(counts.added, 3);
        assert_eq!((counts.changed, counts.removed), (0, 0));

        assert_surface_matches_fresh(&store, &docs);

        // The stamp view serves the pin.
        let as_of: String = store
            .connection()
            .query_row("SELECT as_of_fingerprint FROM _meridian_view", [], |r| {
                r.get(0)
            })
            .expect("stamp");
        assert_eq!(as_of, "b3b:v1");
    }

    #[test]
    fn sync_at_the_pinned_fingerprint_is_a_noop() {
        let docs = fixture_v1();
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut store = SqlStore::open(&tmp_store(&dir)).expect("open");
        sync_ambient(&mut store, &docs, "b3b:v1").expect("cold");
        assert!(sync_ambient(&mut store, &docs, "b3b:v1").is_none());
        assert_eq!(store.pin().expect("pin").expect("pinned").generation, 1);
    }

    #[test]
    fn append_is_delta_grain_and_latest_views_match_fresh_build() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut store = SqlStore::open(&tmp_store(&dir)).expect("open");
        let v1 = fixture_v1();
        sync_ambient(&mut store, &v1, "b3b:v1").expect("cold");

        // Move the corpus: change a.md, add newdoc.md (b.md's [[newdoc]] was
        // dangling and must now resolve WITHOUT b.md changing), remove
        // cards/c.md.
        let v2 = corpus(&[
            (
                "a.md",
                "---\ntitle: Alpha\ntags: [x/one]\n---\n# Top\nchanged body\n\nsee [[b]] and [[missing]]\n",
            ),
            (
                "b.md",
                "#\n- [ ] task under the empty heading\nsee [[Fancy Name]] and [[newdoc]]\n",
            ),
            ("newdoc.md", "# New\nfresh page\n"),
        ]);
        let counts = sync_ambient(&mut store, &v2, "b3b:v2").expect("append");
        assert_eq!(counts.generation, 2);
        assert_eq!(counts.added, 1, "newdoc.md");
        assert_eq!(counts.removed, 1, "cards/c.md");
        // changed = a.md (content) + b.md (resolution-affected by newdoc).
        assert_eq!(counts.changed, 2);

        // O(k): gen-2 doc rows = re-projected + tombstones, never the corpus.
        let gen2_rows: i64 = store
            .connection()
            .query_row("SELECT count(*) FROM hist.doc WHERE gen = 2", [], |r| {
                r.get(0)
            })
            .expect("count");
        assert_eq!(gen2_rows, 4, "a.md + b.md + newdoc.md + c.md tombstone");

        assert_surface_matches_fresh(&store, &v2);

        // The previously-dangling [[newdoc]] resolved without b.md changing.
        let resolved: i64 = store
            .connection()
            .query_row(
                "SELECT count(*) FROM link WHERE src_path = 'b.md' AND dest_path = 'newdoc.md'",
                [],
                |r| r.get(0),
            )
            .expect("resolved");
        assert_eq!(resolved, 1);
    }

    /// Spec §7.1 test 3: an identical chunk in two docs stores ONE
    /// `hist.body_text` row, and an edit that leaves a chunk's bytes unchanged
    /// appends no new text row for it — the content-address dedup that keeps
    /// the append-only file from re-storing unchanged text.
    #[test]
    fn body_text_dedups_across_paths_and_generations() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut store = SqlStore::open(&tmp_store(&dir)).expect("open");
        let shared = "# H\nshared body\n";
        let v1 = corpus(&[
            ("x.md", shared),
            ("y.md", shared),
            ("z.md", "# Z\nown body\n"),
        ]);
        sync_ambient(&mut store, &v1, "b3b:v1").expect("cold");

        let count = |store: &SqlStore, sql: &str| -> i64 {
            store
                .connection()
                .query_row(sql, [], |r| r.get(0))
                .expect("count")
        };
        // x.md and y.md share one chunk ("shared body\n"); z.md adds one.
        let texts_v1 = count(&store, "SELECT count(*) FROM hist.body_text");
        assert_eq!(texts_v1, 2, "identical chunks share one text row");

        // Edit z.md only; x.md/y.md untouched, their chunk text unchanged.
        let v2 = corpus(&[
            ("x.md", shared),
            ("y.md", shared),
            ("z.md", "# Z\nedited body\n"),
        ]);
        sync_ambient(&mut store, &v2, "b3b:v2").expect("append");
        let texts_v2 = count(&store, "SELECT count(*) FROM hist.body_text");
        assert_eq!(
            texts_v2,
            texts_v1 + 1,
            "only the edited chunk's new bytes store a text row"
        );
        assert_surface_matches_fresh(&store, &v2);
    }

    /// Spec §7.1 test 4: a heading-only rename moves the section's CAS token
    /// but stores no new chunk text — heading lines are in no chunk, so the
    /// dedup survives the rename.
    #[test]
    fn heading_rename_leaves_body_text_count_unchanged() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut store = SqlStore::open(&tmp_store(&dir)).expect("open");
        let v1 = corpus(&[("a.md", "# Alpha\nstable body\n")]);
        sync_ambient(&mut store, &v1, "b3b:v1").expect("cold");
        let rev_v1: String = store
            .connection()
            .query_row("SELECT node_rev FROM body WHERE path = 'a.md'", [], |r| {
                r.get(0)
            })
            .expect("rev");

        let v2 = corpus(&[("a.md", "# Alpha2\nstable body\n")]);
        sync_ambient(&mut store, &v2, "b3b:v2").expect("append");
        let texts: i64 = store
            .connection()
            .query_row("SELECT count(*) FROM hist.body_text", [], |r| r.get(0))
            .expect("count");
        assert_eq!(texts, 1, "the rename stored no new chunk text");
        let rev_v2: String = store
            .connection()
            .query_row("SELECT node_rev FROM body WHERE path = 'a.md'", [], |r| {
                r.get(0)
            })
            .expect("rev");
        assert_ne!(rev_v1, rev_v2, "the CAS token moved with the heading");
        assert_surface_matches_fresh(&store, &v2);
    }

    #[test]
    fn alias_appearing_on_a_changed_doc_re_resolves_unchanged_docs() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut store = SqlStore::open(&tmp_store(&dir)).expect("open");
        let v1 = fixture_v1();
        sync_ambient(&mut store, &v1, "b3b:v1").expect("cold");

        // a.md gains the alias b.md's [[Fancy Name]] has been dangling on.
        let mut files = vec![
            (
                "a.md",
                "---\ntitle: Alpha\naliases: [Fancy Name]\n---\n# Top\n",
            ),
            (
                "b.md",
                "#\n- [ ] task under the empty heading\nsee [[Fancy Name]] and [[newdoc]]\n",
            ),
            (
                "cards/c.md",
                "---\ntype: task\nstatus: todo\nowner: w1\nsession: s\n---\n# C\nbody [[a]]\n",
            ),
        ];
        let v2 = corpus(&files);
        sync_ambient(&mut store, &v2, "b3b:v2").expect("append");
        assert_surface_matches_fresh(&store, &v2);
        let dest: String = store
            .connection()
            .query_row(
                "SELECT dest_path FROM link WHERE src_path = 'b.md' AND target_raw = 'Fancy Name'",
                [],
                |r| r.get(0),
            )
            .expect("alias resolution");
        assert_eq!(dest, "a.md");

        // And the alias leaving re-danglifies it (old alias read from the
        // cache's own frontmatter rows).
        files[0] = ("a.md", "---\ntitle: Alpha\n---\n# Top\n");
        let v3 = corpus(&files);
        sync_ambient(&mut store, &v3, "b3b:v3").expect("append 2");
        assert_surface_matches_fresh(&store, &v3);
        let dangling: i64 = store
            .connection()
            .query_row(
                "SELECT count(*) FROM dangling WHERE src_path = 'b.md' AND target_raw = 'Fancy Name'",
                [],
                |r| r.get(0),
            )
            .expect("dangling again");
        assert_eq!(dangling, 1);
    }

    #[test]
    fn uncommitted_append_rolls_back_whole_and_retry_converges() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let file = tmp_store(&dir);
        let mut store = SqlStore::open(&file).expect("open");
        let v1 = fixture_v1();
        sync_ambient(&mut store, &v1, "b3b:v1").expect("cold");

        // A torn append: data rows land in an open transaction, no pin row,
        // and the holder dies (drop = no COMMIT).
        store
            .connection()
            .execute_batch(
                "BEGIN; INSERT INTO hist.doc (path, gen, tombstone) VALUES ('torn.md', 99, false);",
            )
            .expect("torn insert");
        drop(store);

        // The file is at the old pin — or the new one; never between.
        let mut store = SqlStore::open(&file).expect("reopen");
        let pin = store.pin().expect("pin").expect("pinned");
        assert_eq!((pin.generation, pin.fingerprint.as_str()), (1, "b3b:v1"));
        let torn: i64 = store
            .connection()
            .query_row(
                "SELECT count(*) FROM hist.doc WHERE path = 'torn.md'",
                [],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(torn, 0, "the torn transaction left nothing");

        // The retry re-derives the same delta and converges.
        let v2 = corpus(&[("a.md", "# Only\n")]);
        sync_ambient(&mut store, &v2, "b3b:v2").expect("retry");
        assert_surface_matches_fresh(&store, &v2);
    }

    #[test]
    fn foreign_cache_schema_version_recreates_the_file() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let file = tmp_store(&dir);
        let mut store = SqlStore::open(&file).expect("open");
        sync_ambient(&mut store, &fixture_v1(), "b3b:v1").expect("cold");
        // A future schema lands a pin this build does not understand.
        store
            .connection()
            .execute(
                "INSERT INTO hist.pin VALUES (2, 'b3b:v1', now(), 0, 0, 0, 'future', 999, NULL)",
                [],
            )
            .expect("future pin");
        drop(store);
        let store = SqlStore::open(&file).expect("reopen");
        assert!(
            store.pin().expect("pin").is_none(),
            "a foreign-schema file is deleted and cold-started, never migrated"
        );
    }

    #[test]
    fn domain_rule_prefix_change_cold_rebuilds_never_appends() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut store = SqlStore::open(&tmp_store(&dir)).expect("open");
        let docs = fixture_v1();
        sync_ambient(&mut store, &docs, "b3b:v1").expect("cold");
        let counts = sync_ambient(&mut store, &docs, "b3c:v1").expect("rule change");
        assert_eq!(
            counts.generation, 1,
            "same hex under a different prefix must never compare equal — the file cold-rebuilds"
        );
        assert_eq!(
            store.pin().expect("pin").expect("pinned").fingerprint,
            "b3c:v1"
        );
    }

    // ---- Query lane (card 3) ---------------------------------------------

    // Card sql-scalar-type-render (dogfood r6 § S1): every scalar family
    // renders as its SQL text, never a Rust-Arrow Debug repr. Expected
    // strings are pinned against raw DuckDB v1.5.4 (`SET TimeZone='UTC'`
    // for the tz families) — the round's second instrument.
    #[test]
    fn scalar_families_render_as_duckdb_text_never_debug_reprs() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut store = SqlStore::open(&tmp_store(&dir)).expect("open");
        sync_ambient(&mut store, &fixture_v1(), "b3b:v1").expect("cold");

        let (cols, rows) = store
            .query(
                "SELECT TIMESTAMP '2026-08-14 12:00:00' AS ts, \
                        TIMESTAMP '2026-08-14 12:00:00.120000' AS ts_frac, \
                        TIMESTAMPTZ '2026-08-14 12:00:00+00' AS tstz, \
                        DATE '2026-08-14' AS d, \
                        TIME '13:02:03.500000' AS t, \
                        INTERVAL 3 DAY AS iv, \
                        1.5::DECIMAL(4,2) AS dec, \
                        {'k': 1} AS strct, \
                        'small'::ENUM('small','big') AS e, \
                        union_value(num := 2) AS u, \
                        [TIMESTAMP '2026-08-14 12:00:00', NULL] AS arr",
            )
            .expect("lane")
            .expect("select");
        let row = &rows[0];

        // The column register speaks DuckDB's names for the same families —
        // never an arrow Debug repr like `Timestamp(Microsecond, None)`.
        let tys: Vec<&str> = cols.iter().map(|c| c.ty.as_str()).collect();
        assert_eq!(
            tys,
            [
                "TIMESTAMP",
                "TIMESTAMP",
                "TIMESTAMP WITH TIME ZONE",
                "DATE",
                "TIME",
                "INTERVAL",
                "DECIMAL(4,2)",
                "STRUCT",
                "ENUM",
                "UNION",
                "LIST",
            ],
        );

        assert_eq!(
            row[0],
            Json::String("2026-08-14 12:00:00".into()),
            "TIMESTAMP"
        );
        assert_eq!(
            row[1],
            Json::String("2026-08-14 12:00:00.12".into()),
            "fractional seconds trim trailing zeros"
        );
        assert_eq!(
            row[2],
            Json::String("2026-08-14 12:00:00+00".into()),
            "TIMESTAMPTZ carries the UTC marker (the engine has no ICU; values are UTC)"
        );
        assert_eq!(row[3], Json::String("2026-08-14".into()), "DATE");
        assert_eq!(row[4], Json::String("13:02:03.5".into()), "TIME");
        assert_eq!(row[5], Json::String("3 days".into()), "INTERVAL");
        assert_eq!(
            row[6],
            Json::String("1.50".into()),
            "DECIMAL keeps its declared scale"
        );
        assert_eq!(row[7], Json::String("{'k': 1}".into()), "STRUCT");
        assert_eq!(row[8], Json::String("small".into()), "ENUM");
        assert_eq!(row[9], Json::from(2_i64), "UNION renders its member");
        assert_eq!(
            row[10],
            Json::Array(vec![Json::String("2026-08-14 12:00:00".into()), Json::Null]),
            "list elements are SQL text; the ruled ` / ` join stays with the face"
        );
    }

    // The nested-context quoting is DuckDB's own nested-to-varchar rule
    // (LOOKUP_TABLE specials + empty/leading-trailing-space/ci-NULL; struct
    // keys always quoted; union members never) — pinned against v1.5.4.
    #[test]
    fn nested_values_quote_exactly_as_duckdb_would() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut store = SqlStore::open(&tmp_store(&dir)).expect("open");
        sync_ambient(&mut store, &fixture_v1(), "b3b:v1").expect("cold");

        let (_, rows) = store
            .query(
                "SELECT {'k': 'abc'} AS plain, \
                        {'s': 'it''s'} AS esc, \
                        {'k': NULL} AS n, \
                        {'k': TIMESTAMP '2026-08-14 12:00:00'} AS ts, \
                        {'a': [1,2], 'b': {'x': 'y y'}} AS deep, \
                        MAP([1,2],['a','b']) AS m, \
                        MAP([TIMESTAMP '2026-08-14 12:00:00'],[1]) AS mts, \
                        [union_value(s := 'a,b')] AS uraw",
            )
            .expect("lane")
            .expect("select");
        let row = &rows[0];

        assert_eq!(
            row[0],
            Json::String("{'k': abc}".into()),
            "plain strings stay bare"
        );
        assert_eq!(
            row[1],
            Json::String(r"{'s': 'it\'s'}".into()),
            "a quote forces quoting; escape is backslash"
        );
        assert_eq!(
            row[2],
            Json::String("{'k': NULL}".into()),
            "NULL is the bare word"
        );
        assert_eq!(
            row[3],
            Json::String("{'k': '2026-08-14 12:00:00'}".into()),
            "a nested timestamp quotes (its text carries `:`)"
        );
        assert_eq!(
            row[4],
            Json::String("{'a': [1, 2], 'b': {'x': y y}}".into()),
            "containers nest with `, ` joins; inner space alone never quotes"
        );
        assert_eq!(row[5], Json::String("{1=a, 2=b}".into()), "MAP is `k=v`");
        assert_eq!(
            row[6],
            Json::String("{'2026-08-14 12:00:00'=1}".into()),
            "map keys follow the same quoting rule"
        );
        assert_eq!(
            row[7],
            Json::Array(vec![Json::String("a,b".into())]),
            "a union member never quotes, even where the rule would"
        );
    }

    #[test]
    fn query_lane_rolls_back_dml_and_the_file_stays_durable_free() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut store = SqlStore::open(&tmp_store(&dir)).expect("open");
        sync_ambient(&mut store, &fixture_v1(), "b3b:v1").expect("cold");

        let before: i64 = store
            .connection()
            .query_row("SELECT sum(bytes)::BIGINT FROM doc", [], |r| r.get(0))
            .expect("sum");

        // DML against hist executes (visible to its own statement) …
        let (cols, rows) = store
            .query("UPDATE hist.doc SET bytes = 0")
            .expect("lane")
            .expect("dml accepted");
        assert_eq!(cols[0].name, "Count");
        assert!(rows[0][0].as_i64().unwrap_or(0) > 0, "rows were touched");

        // … and nothing durable moved.
        let after: i64 = store
            .connection()
            .query_row("SELECT sum(bytes)::BIGINT FROM doc", [], |r| r.get(0))
            .expect("sum after");
        assert_eq!(before, after, "the rollback lane keeps DML ephemeral");
    }

    #[test]
    fn update_on_a_latest_view_refuses_with_the_teaching() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut store = SqlStore::open(&tmp_store(&dir)).expect("open");
        sync_ambient(&mut store, &fixture_v1(), "b3b:v1").expect("cold");
        let err = store
            .query("UPDATE doc SET bytes = 0")
            .expect("lane")
            .expect_err("views refuse DML");
        assert!(
            err.contains("Can only update base table"),
            "DuckDB's own words stay verbatim: {err}"
        );
        assert!(
            err.contains("hist"),
            "the refusal teaches the hist lane (OQ1): {err}"
        );
    }

    #[test]
    fn concurrent_writers_surface_duckdbs_own_conflict_verbatim() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut store = SqlStore::open(&tmp_store(&dir)).expect("open");
        sync_ambient(&mut store, &fixture_v1(), "b3b:v1").expect("cold");

        let c1 = store.connection().try_clone().expect("clone 1");
        let c2 = store.connection().try_clone().expect("clone 2");
        c1.execute_batch("BEGIN").expect("begin 1");
        c2.execute_batch("BEGIN").expect("begin 2");
        c1.execute("UPDATE hist.doc SET bytes = 1", [])
            .expect("first writer wins");
        let conflict = c2
            .execute("UPDATE hist.doc SET bytes = 2", [])
            .expect_err("optimistic MVCC aborts the second writer");
        assert!(
            !conflict.to_string().is_empty(),
            "the conflict is surfaced verbatim, retryable"
        );
        let _ = c1.execute_batch("ROLLBACK");
        let _ = c2.execute_batch("ROLLBACK");
    }
}

#[cfg(test)]
mod spill_tests {
    use super::tests::{fixture_v1, sync_ambient, tmp_store};
    use super::*;

    /// The spill-config lockout (card sql-spill-config-lockout): both
    /// profiles hold an ABSOLUTE drawer-derived `temp_directory` and a
    /// bounded `max_temp_directory_size` BEFORE the lock — the locked
    /// DEFAULTS were a loaded gun (488 MiB memory spills early; budget 90%
    /// of disk; path `.tmp` RELATIVE to the shell cwd; caller locked out).
    /// It `ENOSPC`ed a host: one query spilled >9 GiB into the seat's cwd.
    #[test]
    fn spill_config_is_absolute_and_bounded() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut store = SqlStore::open(&tmp_store(&dir)).expect("open");
        sync_ambient(&mut store, &fixture_v1(), "b3b:v1").expect("cold");

        let (_, rows) = store
            .query(
                "SELECT name, value FROM duckdb_settings() \
                 WHERE name IN ('temp_directory','max_temp_directory_size') \
                 ORDER BY name",
            )
            .expect("lane")
            .expect("readback");
        let value = |name: &str| -> String {
            rows.iter()
                .find(|r| r[0].as_str() == Some(name))
                .unwrap_or_else(|| panic!("setting {name} missing"))[1]
                .as_str()
                .expect("value")
                .to_owned()
        };
        let temp = value("temp_directory");
        assert!(
            temp.starts_with('/'),
            "temp_directory must be ABSOLUTE — a relative path spills into the shell cwd: {temp}"
        );
        assert!(
            temp.starts_with(dir.path().to_str().expect("utf8 dir")),
            "the spill dir derives from the drawer, never the cwd: {temp}"
        );
        let budget = value("max_temp_directory_size");
        assert!(
            !budget.contains('%'),
            "the spill budget must be a bounded size, never a %%-of-disk default: {budget}"
        );
    }

    /// One setting's live value on the store's own connection.
    fn setting(store: &SqlStore, name: &str) -> String {
        store
            .connection()
            .query_row(&format!("SELECT current_setting('{name}')"), [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap_or_else(|e| panic!("setting {name}: {e}"))
    }

    /// The gate is in force on every store the moment it opens — no caller
    /// action, no first-query lazy path (card
    /// sql-extension-ddl-escapes-rollback-lane).
    #[test]
    fn the_extension_gate_is_in_force_at_open() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let store = SqlStore::open(&tmp_store(&dir)).expect("open");
        assert_eq!(
            setting(&store, "enable_external_access"),
            "false",
            "the door must be shut at open, not at first query"
        );
        assert_eq!(
            setting(&store, "autoinstall_known_extensions"),
            "false",
            "a LOAD must never reach the network from the engine"
        );
    }

    /// The card's leak, at its door: `LOAD` is what let duckpgq write
    /// `sql.main.__duckpgq_internal` past the rollback, and `LOAD` is what
    /// refuses now. The refusal is DuckDB's, extended with the reason and the
    /// allow-list. Hermetic: the Permission Error precedes the extension-file
    /// lookup, so the test does not need duckpgq installed.
    #[test]
    fn a_caller_cannot_load_an_extension() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut store = SqlStore::open(&tmp_store(&dir)).expect("open");
        sync_ambient(&mut store, &fixture_v1(), "b3b:v1").expect("cold");

        for statement in ["LOAD duckpgq", "INSTALL duckpgq"] {
            let refusal = store
                .query(statement)
                .expect("lane")
                .expect_err("the gate refuses extension loading");
            assert!(
                refusal.contains("Loading external extensions is disabled"),
                "{statement}: DuckDB's own words survive: {refusal}"
            );
            assert!(
                refusal.contains("OUTSIDE your statement's transaction"),
                "{statement}: the refusal teaches WHY: {refusal}"
            );
            assert!(
                refusal.contains("fts, vss"),
                "{statement}: the refusal names the allow-list: {refusal}"
            );
        }

        // And the leak's own artefact never appears.
        let leaked: i64 = store
            .connection()
            .query_row(
                "SELECT count(*)::BIGINT FROM duckdb_tables() \
                 WHERE table_name = '__duckpgq_internal'",
                [],
                |r| r.get(0),
            )
            .expect("count");
        assert_eq!(leaked, 0, "the drawer holds no extension side table");
    }

    /// The gate is one-way: a caller cannot re-open the door it found shut.
    /// DuckDB refuses the loosening `SET` itself — `Cannot enable external
    /// access while database is running`.
    #[test]
    fn a_caller_cannot_reopen_the_extension_gate() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut store = SqlStore::open(&tmp_store(&dir)).expect("open");
        sync_ambient(&mut store, &fixture_v1(), "b3b:v1").expect("cold");

        let refusal = store
            .query("SET enable_external_access=true")
            .expect("lane")
            .expect_err("loosening refuses");
        assert!(
            refusal.contains("Cannot enable external access"),
            "the refusal is DuckDB's own one-way rule: {refusal}"
        );
        assert_eq!(
            setting(&store, "enable_external_access"),
            "false",
            "the door is still shut after the attempt"
        );
    }

    /// The gate closes the lane to extension code, not to the caller's own
    /// reads: an ordinary projection query is untouched by it.
    #[test]
    fn the_gate_leaves_the_projection_readable() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut store = SqlStore::open(&tmp_store(&dir)).expect("open");
        sync_ambient(&mut store, &fixture_v1(), "b3b:v1").expect("cold");

        let (_, rows) = store
            .query("SELECT count(*)::BIGINT FROM doc")
            .expect("lane")
            .expect("the projection still answers");
        assert!(
            rows[0][0].as_i64().unwrap_or(0) > 0,
            "the fixture's docs project"
        );
    }

    /// The containment is plain config, not a lock (NO-SANDBOX ruling): a
    /// caller's own `SET` through the query lane succeeds — nothing refuses,
    /// nothing is frozen. The extension gate does not change that: it shuts
    /// ONE door, it does not lock the configuration.
    #[test]
    fn spill_containment_is_plain_config_not_a_lock() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut store = SqlStore::open(&tmp_store(&dir)).expect("open");
        sync_ambient(&mut store, &fixture_v1(), "b3b:v1").expect("cold");
        store
            .query("SET memory_limit='1GB'")
            .expect("lane")
            .expect("configuration is not locked — a caller SET succeeds");
    }
}
