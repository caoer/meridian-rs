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
//! A transaction is not the whole contract, because two things do not live
//! inside one.
//!
//! **Extension code.** `LOAD` lands on the shared `DatabaseInstance`, and a
//! loaded extension may then write through its own path (duckpgq's `CREATE
//! PROPERTY GRAPH` wrote `sql.main.__duckpgq_internal` durably into the
//! drawer — card `sql-extension-ddl-escapes-rollback-lane`). So the lane also
//! runs no unaudited third-party code: [`apply_extension_gate`] shuts
//! community extensions one-way at open, leaving core extensions, external
//! access and caller `SET`s exactly where the NO-SANDBOX ruling put them.
//! What the rollback cannot undo, the door never admits.
//!
//! **Engine configuration.** A GLOBAL-scope `SET` re-tunes the `DBConfig` the
//! whole instance shares, so a caller's `SET memory_limit='1MB'` used to
//! starve every LATER caller of that root until the daemon restarted (card
//! `sql-set-config-cross-caller-starvation`). The lane now closes each call
//! with [`restore_global_config`]: the caller's `SET` still succeeds and still
//! governs its own statement, and the values the next caller finds are the
//! ones the engine set. What the rollback cannot undo, the lane puts back.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

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

/// Shut the query lane to COMMUNITY extension code (card
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
/// core DDL rolls back exactly as the contract says. A bare `LOAD duckpgq`
/// with no DDL at all also survived the call, leaking the extension (parser
/// override included) onto every later caller of that root.
///
/// **The line is third-party code, and it is drawn where this codebase
/// already draws it.** The NO-SANDBOX ruling (2026-08-14) accepted the
/// caller's own reach — `read_csv('/etc/hosts')` answers rows, not a refusal,
/// because every caller already holds a shell (`registry/tests/sql_op.rs`
/// `sql_lifecycle_over_the_wire`). That posture is about what a caller may do
/// TO ITSELF. It says nothing about unaudited third-party code writing the
/// DRAWER THAT EVERY OTHER CALLER SHARES, which is the defect here — and
/// which no caller's shell could do on the others' behalf. So the gate is
/// exactly one `SET`, and it is the narrowest one that closes the class:
///
/// - `duckpgq` is a **community** extension (`installed_from = community`);
///   `fts`, `vss`, `json`, `parquet`, `icu` are **core**. `LOAD fts` and
///   `LOAD vss` keep working, and both were measured transactional:
///   `PRAGMA create_fts_index` over a durable `hist` table left no
///   `fts_main_*` schema behind, and an HNSW index built under
///   `hnsw_enable_experimental_persistence` left neither index nor table.
/// - A community extension cannot slip in by the back door either: they do
///   not autoload (a bare `SELECT quack('hello')` answered `Did you mean
///   "quarter"?` with `quack` installed and both autoload settings on), and
///   `CREATE PROPERTY GRAPH` with no prior `LOAD` is a parser error.
/// - The `SET` is one-way **by code, not by observation**: at the vendored
///   v1.5.4, `AllowCommunityExtensionsSetting::Scope =
///   SettingScopeTarget::GLOBAL_ONLY` (`settings.hpp:144-153`, so it is
///   DBConfig-level and survives every `try_clone`), and its `OnSet` throws
///   `Cannot change allow_community_extensions setting while database is
///   running` for any `true` input once `info.db` is set
///   (`custom_settings.cpp:153-157`). Tightening at runtime is allowed;
///   loosening is not.
/// - **The gate's precondition holds by the same mechanism.** It bites inside
///   `extension_load.cpp:482-497`, which only runs while
///   `allow_unsigned_extensions` is false — and that setting is likewise
///   `GLOBAL_ONLY`, defaults false, and refuses a runtime `true`
///   (`settings.hpp:198-207`, `custom_settings.cpp:197-201`). We never set it,
///   so no caller can unlock the branch this gate lives in.
/// - The knob touches nothing else: its `OnSet` is those four lines. Contrast
///   `EnableExternalAccessSetting::OnSet` (`custom_settings.cpp:744-767`),
///   which whitelists the temp dir as it closes and therefore FREEZES
///   `temp_directory` — the ordering coupling an earlier draft of this gate
///   introduced, and which this one simply does not have.
///
/// This is a door, not a sandbox: one execution path for every caller, no
/// per-caller profile, external access untouched, and the configuration NOT
/// locked — a caller's `SET memory_limit` still succeeds.
///
/// **What it does not close**, named rather than implied: a core extension
/// that wrote outside the caller's transaction would still reach the drawer.
/// That would be a `DuckDB` bug in `DuckDB`'s own code, reportable upstream —
/// not unaudited third-party code the engine chose to run. A caller's global
/// `SET` was the other open leak here; it is closed by
/// [`restore_global_config`], not by this gate (card
/// `sql-set-config-cross-caller-starvation`, a ruling of its own).
///
/// **Every caller-SQL door takes it, with no exemptions** — [`SqlStore::open`]
/// and [`SqlStore::recreate`] (so `SqlStore::query`'s `try_clone` inherits it
/// by `GLOBAL_ONLY` scope), `mrd sql`'s `:memory:` lane, and
/// `registry::mw_sql`'s middleware projection. The last two have no drawer to
/// leak into; they are gated anyway, because a door that admits third-party
/// extension code while its siblings refuse it would make the DOOR, not the
/// statement, decide what the contract means.
///
/// # Errors
/// Propagates the `DuckDB` error from the `SET`.
pub fn apply_extension_gate(conn: &Connection) -> duckdb::Result<()> {
    conn.execute_batch("SET allow_community_extensions=false;")
}

/// Every GLOBAL-scope setting `DuckDB` reports, `(name, value)`, sorted by
/// name. LOCAL-scope settings are deliberately absent: they live on the
/// connection, and `SqlStore::query`'s clone dies at the end of the call, so
/// they cannot reach the next caller (measured 2026-08-23 on
/// `hnsw_enable_experimental_persistence`, card
/// `sql-set-config-cross-caller-starvation`). A `NULL` value is skipped —
/// there is nothing to put back and `SET x=NULL` is not the way to say it.
fn global_config_snapshot(conn: &Connection) -> duckdb::Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT name, value::VARCHAR FROM duckdb_settings() \
         WHERE scope::VARCHAR = 'GLOBAL' AND value IS NOT NULL ORDER BY name",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    rows.collect()
}

/// How many restore passes [`restore_global_config`] runs before it reports
/// what it could not put back. Two, because `DuckDB` DERIVES some settings
/// from others: restoring `memory_limit` re-derives
/// `write_buffer_row_group_memory_limit`, which a single alphabetical pass
/// would then leave at the caller's value on a host where the derived name
/// sorts first.
const RESTORE_PASSES: usize = 2;

/// Put back every GLOBAL-scope setting a caller's statement moved, so the
/// caller's `SET` governs its own call and nothing after it (card
/// `sql-set-config-cross-caller-starvation`).
///
/// **Why this shape and not the other two.** The leak is real and it is the
/// ROOT's, not the caller's: the resident daemon holds one `DatabaseInstance`
/// per workspace for its whole life, `SqlStore::query`'s `try_clone` is a new
/// connection on that SAME instance, and a GLOBAL `SET` writes the `DBConfig`
/// they share — so `SET memory_limit='1MB'` starved every later caller of a
/// fleet-shared root until the daemon restarted. `BEGIN … ROLLBACK` cannot
/// reach it: that is a transaction, and engine config is not in one. The
/// advisor's ruling (2026-08-24) took this shape over locking the
/// configuration precisely because it takes NOTHING away — the NO-SANDBOX
/// ruling (2026-08-14) answered "may a caller do X?", and this defect asks
/// "may a caller's X outlive its call and bind every OTHER caller?".
///
/// **Snapshot-and-re-`SET`, never `RESET`.** `RESET temp_directory` would
/// restore `DuckDB`'s default, not [`apply_spill_containment`]'s value, and
/// silently unbound the spill that card `sql-spill-config-lockout` exists to
/// bound. The `before` snapshot is taken with the engine's own values already
/// in force, so putting it back is putting the ENGINE's value back. Measured
/// 2026-08-24: a caller `SET temp_directory='/tmp/mrd-hostile-probe'` came
/// back to the drawer-derived spill dir, and `max_temp_directory_size` to
/// `8.0 GiB`.
///
/// Returns the names it could NOT put back — empty on the ordinary path. Two
/// classes can appear there, both measured on `DuckDB` v1.5.4:
///
/// - A setting `DuckDB` refuses to re-`SET` to the value it just reported
///   (`force_variant_shredding` reads `INVALID`, which is not an accepted
///   input). Eight of the 134 GLOBAL settings are in this class; seven of
///   them also refuse a caller's `SET` ("Cannot change … while database is
///   running", "not adjustable by a user"), so they can never drift in the
///   first place.
/// - `lock_configuration`, which is one-way BY DESIGN: once a caller sets it
///   true, every later `SET` on that instance refuses — this one included.
///   That residue is a denial of the DOOR, not starvation: the restore pass
///   has already run at the end of the previous call, so the values it
///   freezes are the engine's. Pinned by
///   `a_caller_lock_is_the_one_residue_and_it_freezes_engine_values`.
///
/// The return value is advisory: a caller's query has already answered by the
/// time this runs, and failing it here would turn a successful query into an
/// error over state the caller cannot see. It is what the tests assert on.
pub fn restore_global_config(conn: &Connection, before: &[(String, String)]) -> Vec<String> {
    let mut drifted: Vec<(String, String)> = Vec::new();
    for pass in 0..=RESTORE_PASSES {
        let Ok(now) = global_config_snapshot(conn) else {
            // The snapshot itself failed (a locked or dying instance): say so
            // by name rather than reporting a clean restore we never made.
            return vec!["<config snapshot unavailable>".to_owned()];
        };
        drifted = before
            .iter()
            .filter(|(name, was)| {
                now.iter()
                    .any(|(seen, is_now)| seen == name && is_now != was)
            })
            .cloned()
            .collect();
        // The last iteration only MEASURES: it reports what the passes before
        // it failed to put back.
        if drifted.is_empty() || pass == RESTORE_PASSES {
            break;
        }
        for (name, was) in &drifted {
            let value = was.replace('\'', "''");
            // Best effort by construction — the names that refuse are this
            // function's return value, not its error.
            let _ = conn.execute_batch(&format!("SET \"{name}\"='{value}';"));
        }
    }
    drifted.into_iter().map(|(name, _)| name).collect()
}

/// Bound the connection's disk spill (card sql-spill-config-lockout):
/// `spill_dir` becomes the ABSOLUTE `temp_directory`, with [`SPILL_BUDGET`]
/// as `max_temp_directory_size`. Plain config, never a sandbox (the
/// NO-SANDBOX ruling, 2026-08-14) — nothing is locked, and a caller's own
/// `SET` can re-point it FOR THE LENGTH OF THAT CALL
/// ([`restore_global_config`] puts the engine's value back at the end of it,
/// card `sql-set-config-cross-caller-starvation`; the `SET` still succeeds,
/// which is what the NO-SANDBOX ruling asserts). The `DuckDB` defaults were
/// the defect this repairs:
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
    // sql-extension-ddl-escapes-rollback-lane). DuckDB blames the WRONG
    // setting here — it reports the signature check and names
    // `allow_unsigned_extensions`, because a community extension is unsigned
    // by construction — so the caller is left hunting a knob that is not the
    // one that stopped it. Say which gate it hit, and why.
    if error.contains("signature is either missing or invalid") {
        return format!(
            "{error}\nNeither shut door is the knob that message names. A \
             COMMUNITY extension cannot load here: this engine sets \
             allow_community_extensions=false at open. An UNSIGNED local build \
             cannot load anywhere: DuckDB's own default keeps \
             allow_unsigned_extensions=false. Both refuse to loosen while the \
             database is running. Why the first one exists: an extension loads \
             onto the shared engine instance OUTSIDE your statement's \
             transaction and is then free to write the drawer that every other \
             caller shares (duckpgq's CREATE PROPERTY GRAPH wrote \
             sql.main.__duckpgq_internal durably). Signed core extensions are \
             not gated, external files are not gated, and your own SET still \
             works."
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
/// Appends ride the base connection; caller queries run on a [`SqlRead`]'s own
/// clone through the rollback lane. Spill containment
/// ([`apply_spill_containment`]) is applied once at open — the `SET`s are
/// instance-global in `DuckDB`, so every connection inherits it.
///
/// **The store is `!Sync`, and that is load-bearing.** `duckdb::Connection` is
/// `Send` but not `Sync` (`unsafe impl Send for Connection`, and its interior
/// is a `RefCell`), so the store can only be shared between threads behind a
/// `Mutex`. What that mutex must cover is the APPEND and the base connection —
/// not the caller's query, which owns a separate connection of its own. See
/// [`SqlStore::begin_read`] for the split, and why the read's snapshot is
/// pinned before the lock is released.
pub struct SqlStore {
    conn: Connection,
    file: PathBuf,
    /// The engine's own GLOBAL-scope configuration, captured at open, before
    /// any caller could reach this instance — [`SqlRead`]'s restore target.
    ///
    /// **Why captured once and not per call.** The restore has to put back the
    /// ENGINE's values, and it can only do that if its `before` snapshot was
    /// taken when no caller's `SET` was in force. A per-call snapshot could
    /// only promise that while every query ran under the store mutex; the
    /// moment queries overlap, one caller's `SET` becomes another caller's
    /// "engine value" and the restore would make the drift permanent — the
    /// exact starvation of card `sql-set-config-cross-caller-starvation`,
    /// re-opened by the concurrency. Open is the one instant where no caller
    /// exists, so it is the only honest place to read the baseline. It also
    /// drops a 134-row `duckdb_settings()` scan from every served query.
    baseline: Arc<Vec<(String, String)>>,
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
        let store = Self::adopt(conn, file)?;
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
        Self::adopt(conn, file)
    }

    /// Put the engine's instance-global configuration in force on a
    /// freshly-opened connection, then record it as every later read's restore
    /// baseline (see [`SqlStore::baseline`]). The one instant at which no
    /// caller can be holding a `SET` on this instance is before the store is
    /// published, which is here.
    fn adopt(conn: Connection, file: &Path) -> Result<SqlStore, ViewError> {
        let mut store = SqlStore {
            conn,
            file: file.to_path_buf(),
            baseline: Arc::new(Vec::new()),
        };
        apply_spill_containment(&store.conn, &store.spill_dir())?;
        apply_extension_gate(&store.conn)?;
        store.baseline = Arc::new(global_config_snapshot(&store.conn)?);
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

    /// Open a caller-owned read on this store's `DuckDB` instance: a fresh
    /// connection with its `BEGIN` already issued, so the snapshot it will
    /// answer from is the one visible AT THIS CALL.
    ///
    /// **This is the seam that un-serializes `sql`.** The returned [`SqlRead`]
    /// borrows nothing from the store — it owns its connection, it is `Send`,
    /// and running it needs no access to the base connection at all. A caller
    /// that holds the store's mutex may therefore `sync`, open a read here,
    /// and RELEASE THE MUTEX before running the query; the query then costs
    /// other callers nothing but `DuckDB`'s own MVCC.
    ///
    /// **Why `BEGIN` here and not in [`SqlRead::run`].** The transaction pins
    /// the read's snapshot, and it must be pinned while the caller still holds
    /// the store — otherwise a concurrent caller's append could commit between
    /// the release and the query, and the answer would carry rows NEWER than
    /// the `as_of` fingerprint the served body reports. Taking the snapshot
    /// under the lock keeps the served answer exactly as fresh as it claims to
    /// be (§Q3 honest tense), while the expensive part — the query — runs
    /// outside it. Pinned by
    /// `a_read_opened_before_an_append_does_not_see_that_append`.
    ///
    /// # Errors
    /// The connection cannot be cloned, or `BEGIN` fails.
    pub fn begin_read(&self) -> Result<SqlRead, ViewError> {
        let conn = self.conn.try_clone()?;
        conn.execute_batch("BEGIN")?;
        Ok(SqlRead {
            conn,
            baseline: Arc::clone(&self.baseline),
        })
    }

    /// Run one caller query through the rollback lane, start to finish, on a
    /// connection of its own — [`SqlStore::begin_read`] then [`SqlRead::run`].
    ///
    /// The whole call holds `&self`, so this is the lane for callers that are
    /// not contending for the store (the `mrd` CLI, tests). The served daemon
    /// door splits the two halves across the store mutex instead; see
    /// [`SqlStore::begin_read`].
    ///
    /// # Errors
    /// A connection failure is a `ViewError`; the caller's own SQL failing is
    /// `Ok` with the error string in the result (their register, not ours) —
    /// including `DuckDB`'s MVCC transaction conflicts, verbatim. A setting
    /// that will not go back is NOT an error (see [`restore_global_config`]).
    #[allow(clippy::type_complexity)]
    pub fn query(
        &self,
        sql: &str,
    ) -> Result<Result<(Vec<ColMeta>, Vec<Vec<Json>>), String>, ViewError> {
        self.begin_read()?.run(sql)
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

/// One caller's read on a [`SqlStore`]'s `DuckDB` instance: its own connection,
/// its snapshot already pinned by an open transaction, and the engine's config
/// baseline to put back when it is done.
///
/// It owns everything it needs, so it is `Send` and outlives the store guard
/// that made it — which is the whole point (see [`SqlStore::begin_read`]).
/// Dropping one without calling [`SqlRead::run`] leaves the transaction to be
/// closed by the connection's own drop.
pub struct SqlRead {
    conn: Connection,
    baseline: Arc<Vec<(String, String)>>,
}

impl std::fmt::Debug for SqlRead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqlRead").finish_non_exhaustive()
    }
}

impl SqlRead {
    /// Finish the rollback lane: `statement → collect → ROLLBACK → restore
    /// config`. The `BEGIN` was issued by [`SqlStore::begin_read`]. One
    /// execution path for every caller (the NO-SANDBOX ruling, 2026-08-14): no
    /// profile, no lock — spill containment is already in force instance-wide
    /// from open.
    ///
    /// The config restore is the last step, not a further guarantee: a
    /// GLOBAL-scope `SET` writes the `DBConfig` this connection shares with
    /// every other caller of the root, which no `ROLLBACK` reaches (card
    /// `sql-set-config-cross-caller-starvation`). It runs whether the caller's
    /// statement succeeded or failed — a failed statement can still have moved
    /// config before it failed — and it restores toward the ENGINE's values
    /// recorded at open, never toward whatever a concurrent caller happens to
    /// have in force ([`SqlStore::baseline`]).
    ///
    /// **What overlapping reads do NOT get.** GLOBAL settings belong to the
    /// shared `DatabaseInstance`, not to a connection, so while two reads
    /// overlap each can see the other's `SET` and either one's restore can put
    /// the engine's value back under the other. A caller's `SET` still governs
    /// its own statement whenever that statement runs alone, and no caller's
    /// value survives its call either way — the leak the restore exists to
    /// stop. Making a `SET` private to an overlapping caller is not something
    /// one instance can offer; serializing every query to fake it is the
    /// convoy this split removes.
    ///
    /// # Errors
    /// A connection failure is a `ViewError`; the caller's own SQL failing is
    /// `Ok` with the error string in the result (their register, not ours) —
    /// including `DuckDB`'s MVCC transaction conflicts, verbatim. A setting
    /// that will not go back is NOT an error (see [`restore_global_config`]).
    #[allow(clippy::type_complexity)]
    pub fn run(
        self,
        sql: &str,
    ) -> Result<Result<(Vec<ColMeta>, Vec<Vec<Json>>), String>, ViewError> {
        let result = run_query(&self.conn, sql);
        // Always roll back — reads are unaffected; DML dies here (P3/P6).
        let rolled_back = self.conn.execute_batch("ROLLBACK");
        // Always restore — the transaction never held the engine's config, so
        // a ROLLBACK that itself failed is no reason to leave the next caller
        // with this caller's `memory_limit`. Restore first, then answer for
        // the rollback.
        let _ = restore_global_config(&self.conn, &self.baseline);
        rolled_back?;
        Ok(result)
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
/// A held lock must degrade down the caller's ladder, never delete the file —
/// except for the repair verb, which refuses instead of degrading
/// ([`crate::ViewError::is_held`], the one caller outside this module).
pub(crate) fn is_lock_error(e: &duckdb::Error) -> bool {
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
                    Arc::new(model::build((*raw).to_string(), syntax::parse(raw))),
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

    /// One doc count out of a store, through a caller-owned read.
    fn count_docs(read: SqlRead) -> i64 {
        let (_, rows) = read
            .run("SELECT count(*)::BIGINT FROM doc")
            .expect("lane")
            .expect("query");
        rows[0][0].as_i64().expect("bigint")
    }

    /// **The assumption the whole split rests on**, asserted rather than
    /// trusted: `begin_read`'s `BEGIN` pins the snapshot AT THAT CALL, so an
    /// append committed afterwards is invisible to it.
    ///
    /// If DuckDB deferred the snapshot to the first statement instead, the
    /// serve path would hand back rows NEWER than the `as_of` fingerprint it
    /// reports — a freshness lie (§Q3 honest tense) that no other test on this
    /// path would catch, because single-caller runs never have a second
    /// appender.
    #[test]
    fn a_read_opened_before_an_append_does_not_see_that_append() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut store = SqlStore::open(&tmp_store(&dir)).expect("open");
        let v1 = fixture_v1();
        sync_ambient(&mut store, &v1, "b3b:v1").expect("cold");
        let before = count_docs(store.begin_read().expect("read"));

        // The read is opened FIRST, then the corpus grows under it.
        let read = store.begin_read().expect("read");
        let mut v2 = v1.clone();
        v2.insert(
            "late.md".to_owned(),
            Arc::new(model::build(
                "# Late\nappended after the read opened\n".to_owned(),
                syntax::parse("# Late\nappended after the read opened\n"),
            )),
        );
        sync_ambient(&mut store, &v2, "b3b:v2").expect("append");

        assert_eq!(
            count_docs(store.begin_read().expect("read")),
            before + 1,
            "a read opened AFTER the append must see it — otherwise this test \
             proves nothing about the one opened before"
        );
        assert_eq!(
            count_docs(read),
            before,
            "the read's snapshot was pinned by its BEGIN, so the later append \
             is invisible to it"
        );
    }

    /// The property that un-serializes `sql`: a live read borrows nothing from
    /// the store, so the store is free the instant the caller lets go of it.
    ///
    /// This is the serve shape (`registry::sql_op::serve`) in miniature —
    /// lock, sync, `begin_read`, RELEASE, then run — and it is asserted
    /// structurally rather than by wall clock, so it cannot pass on a fast box
    /// and fail on a slow one.
    #[test]
    fn a_read_in_flight_does_not_hold_the_store() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let store = std::sync::Mutex::new(SqlStore::open(&tmp_store(&dir)).expect("open"));

        let read = {
            let mut guard = store.lock().expect("lock");
            sync_ambient(&mut guard, &fixture_v1(), "b3b:v1").expect("cold");
            guard.begin_read().expect("read")
        };

        // The read outlives the guard and the store is uncontended while it is
        // alive — which is exactly what a sibling seat needs to be true.
        let sibling = store
            .try_lock()
            .expect("an open read must not hold the store: a sibling's sql would convoy behind it");
        drop(sibling);

        // And it still answers correctly after the store has moved on.
        assert_eq!(count_docs(read), 3, "the read still serves its snapshot");
    }

    /// The cross-thread form of the property above, on one store: while one
    /// caller's query is genuinely in flight, another caller takes the store,
    /// opens its own read and answers — start to finish — and the first is
    /// still running when it does.
    ///
    /// The slow statement is a recursive CTE (no `sleep` in DuckDB), so the
    /// margin is generous on purpose: the assertion is "the sibling got
    /// through while the holder ran", never a duration.
    #[test]
    fn a_sibling_reads_the_same_store_while_a_slow_query_is_in_flight() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let dir = tempfile::tempdir().expect("tmpdir");
        let store = Arc::new(std::sync::Mutex::new(
            SqlStore::open(&tmp_store(&dir)).expect("open"),
        ));
        {
            let mut guard = store.lock().expect("lock");
            sync_ambient(&mut guard, &fixture_v1(), "b3b:v1").expect("cold");
        }

        let running = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        let holder = {
            let store = std::sync::Arc::clone(&store);
            let running = std::sync::Arc::clone(&running);
            let finished = std::sync::Arc::clone(&finished);
            std::thread::spawn(move || {
                let read = {
                    let guard = store.lock().expect("lock");
                    guard.begin_read().expect("read")
                };
                running.store(true, Ordering::SeqCst);
                let _ = read.run(
                    "WITH RECURSIVE t(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM t \
                     WHERE i < 20000000) SELECT count(*)::BIGINT FROM t",
                );
                finished.store(true, Ordering::SeqCst);
            })
        };

        while !running.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
        let sibling = {
            let guard = store
                .lock()
                .expect("the store is not held by the in-flight query");
            guard.begin_read().expect("read")
        };
        let docs = count_docs(sibling);
        let holder_still_running = !finished.load(Ordering::SeqCst);

        holder.join().expect("holder");
        assert_eq!(docs, 3, "the sibling's own answer is correct");
        assert!(
            holder_still_running,
            "the sibling answered only after the slow query had already \
             finished — either the store is still serialized, or the slow \
             statement was not slow enough on this box to prove anything"
        );
    }

    /// A caller's GLOBAL `SET` is put back to the value the ENGINE set at
    /// open, and the restore does not depend on the store having been
    /// exclusive for the length of the query.
    ///
    /// The baseline is read at open, before any caller exists, so a `SET` in
    /// force on the instance can never be mistaken for an engine value and
    /// made permanent — which is what a per-call `before` snapshot would do
    /// the moment two reads overlap (card
    /// `sql-set-config-cross-caller-starvation`, re-opened by concurrency).
    #[test]
    fn the_restore_baseline_is_the_engines_values_not_a_concurrent_callers() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut store = SqlStore::open(&tmp_store(&dir)).expect("open");
        sync_ambient(&mut store, &fixture_v1(), "b3b:v1").expect("cold");

        let engine_value = global_setting(&store, "memory_limit");

        // A caller moves it, and — the concurrency case — a SECOND read is
        // opened while that value is the one in force on the instance.
        let hostile = store.begin_read().expect("read");
        let overlapping = store.begin_read().expect("read");
        hostile
            .run("SET memory_limit='123MB'")
            .expect("lane")
            .expect("set");

        // The overlapping read was opened under the hostile value; its own
        // restore must still land on the ENGINE's, never on '123MB'.
        overlapping.run("SELECT 1").expect("lane").expect("select");

        assert_eq!(
            global_setting(&store, "memory_limit"),
            engine_value,
            "an overlapping read restored a concurrent caller's SET as if it \
             were the engine's value"
        );
    }

    /// One GLOBAL setting's live value, read on the store's own base
    /// connection so it is never the read lane's own answer about itself.
    fn global_setting(store: &SqlStore, name: &str) -> String {
        store
            .connection()
            .query_row(
                &format!("SELECT current_setting('{name}')::VARCHAR"),
                [],
                |r| r.get::<_, String>(0),
            )
            .unwrap_or_else(|e| panic!("setting {name}: {e}"))
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
            // `::VARCHAR` because the boolean settings answer BOOLEAN, and one
            // reader for every setting is the point of this helper.
            .query_row(
                &format!("SELECT current_setting('{name}')::VARCHAR"),
                [],
                |r| r.get::<_, String>(0),
            )
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
            setting(&store, "allow_community_extensions"),
            "false",
            "the door must be shut at open, not at first query"
        );
        // The gate lives inside DuckDB's signature branch
        // (extension_load.cpp:482-497), which only runs while this is false.
        // If a future change ever turns it on at open, the gate above becomes
        // decorative and nothing else would say so.
        assert_eq!(
            setting(&store, "allow_unsigned_extensions"),
            "false",
            "the gate's precondition: unsigned loading must stay off, or the \
             signature branch the gate lives in never runs"
        );
        // F4's ordering coupling, inverted into a guard: the broad knob
        // (enable_external_access) freezes temp_directory as it closes, this
        // one does not. If someone swaps the mechanism back, this fails.
        store
            .connection()
            .execute_batch("SET temp_directory='/tmp/mrd-gate-probe';")
            .expect("the gate must not freeze temp_directory (card sql-spill-config-lockout)");
    }

    /// The gate is one-way: a caller cannot re-open the door it found shut.
    /// `DuckDB` refuses the loosening `SET` itself, which is what makes a
    /// runtime gate as strong as an open-time one.
    #[test]
    fn a_caller_cannot_reopen_the_extension_gate() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut store = SqlStore::open(&tmp_store(&dir)).expect("open");
        sync_ambient(&mut store, &fixture_v1(), "b3b:v1").expect("cold");

        let refusal = store
            .query("SET allow_community_extensions=true")
            .expect("lane")
            .expect_err("loosening refuses");
        assert!(
            refusal.contains("Cannot change allow_community_extensions"),
            "the refusal is DuckDB's own one-way rule: {refusal}"
        );
        assert_eq!(
            setting(&store, "allow_community_extensions"),
            "false",
            "the door is still shut after the attempt"
        );
    }

    /// `DuckDB` blames `allow_unsigned_extensions` for a gate the caller never
    /// touched, so the teaching has to say which door it actually hit. The
    /// input is `DuckDB` v1.5.4's verbatim message, captured 2026-08-23 — this
    /// test is what tells us upstream reworded it.
    #[test]
    fn the_community_gate_refusal_teaches_which_door() {
        let verbatim = "IO Error: Extension \"/home/x/.duckdb/extensions/v1.5.4/linux_amd64/\
                        duckpgq.duckdb_extension\" could not be loaded because its signature is \
                        either missing or invalid and unsigned extensions are disabled by \
                        configuration (allow_unsigned_extensions)";
        let taught = teach(verbatim);
        assert!(
            taught.starts_with(verbatim),
            "DuckDB's own words survive first: {taught}"
        );
        assert!(
            taught.contains("Neither shut door is the knob that message names"),
            "the caller is not sent hunting the wrong setting: {taught}"
        );
        // r2 F-D: this message is DuckDB's for ANY extension failing the
        // signature check, including a caller's own unsigned local build —
        // which is not a community extension. The teaching must name both
        // doors, or it tells that caller something false about their own.
        assert!(
            taught.contains("UNSIGNED local build") && taught.contains("COMMUNITY extension"),
            "both shut doors are named, not just ours: {taught}"
        );
        assert!(
            taught.contains("OUTSIDE your statement's transaction"),
            "the refusal teaches WHY: {taught}"
        );
        // F2: the sentence must describe the GATE, never what this process
        // happened to load — a capability claim built from a constant is false
        // on any host that does not have the extension.
        assert!(
            taught.contains("core extensions are not gated"),
            "and what is NOT taken away: {taught}"
        );
        assert!(
            !taught.contains("Loaded at open"),
            "no capability claim the process cannot back: {taught}"
        );
    }

    /// The card's leak at its door. Best-effort by construction: `DuckDB`
    /// checks the extension FILE before the community flag, so a host without
    /// duckpgq installed answers `not found` instead of the gate's words. The
    /// assertion that holds on every host is the one that matters — the load
    /// never SUCCEEDS, and no side table is left behind.
    #[test]
    fn a_caller_cannot_load_a_community_extension() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut store = SqlStore::open(&tmp_store(&dir)).expect("open");
        sync_ambient(&mut store, &fixture_v1(), "b3b:v1").expect("cold");

        let refusal = store
            .query("LOAD duckpgq")
            .expect("lane")
            .expect_err("a community extension never loads through this lane");
        if refusal.contains("signature is either missing or invalid") {
            // This host HAS duckpgq, so the gate — not a missing file — is what
            // refused, and the teaching must ride.
            assert!(
                refusal.contains("Neither shut door is the knob that message names"),
                "this host HAS duckpgq, so the teaching must ride: {refusal}"
            );
            assert!(
                refusal.contains("allow_community_extensions=false"),
                "and it must name OUR door specifically: {refusal}"
            );
        }

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

    /// F3, by mechanism rather than by name: the invariant the card is
    /// actually about — **a caller query leaves the drawer's catalog exactly as
    /// it found it.**
    ///
    /// **What it does and does not guard, because an earlier version of this
    /// comment overclaimed and review r2 (F-A) caught it.** The first draft
    /// said "THIS catches whatever the next duckpgq turns out to be" while the
    /// statement list held no `CREATE PROPERTY GRAPH` at all — so deleting
    /// [`apply_extension_gate`] left it green on every host, and it was a
    /// rollback test wearing a gate test's label. The leaking statement is now
    /// in the list, in the single-statement form the lane actually runs
    /// (`run_query` prepares one statement), which the reviewer measured to
    /// leak `sql.main.__duckpgq_internal` through `BEGIN … ROLLBACK` with the
    /// gate open.
    ///
    /// So: on a host that HAS duckpgq this fails if the gate is removed. On a
    /// host without it (CI), that statement is a parser error and this degrades
    /// to what it always was — a rollback test. The gate's host-independent
    /// guards are [`the_extension_gate_is_in_force_at_open`] and
    /// [`a_caller_cannot_reopen_the_extension_gate`], which read a setting
    /// whose `DefaultValue` is `"true"`; do not let this test be counted twice.
    ///
    /// **Verified by mutation, not asserted** (2026-08-24, duckpgq-stocked
    /// build box): commenting out both `apply_extension_gate` call sites in
    /// [`SqlStore::open`] and [`SqlStore::recreate`] fails FOUR tests — this
    /// one, `the_extension_gate_is_in_force_at_open`,
    /// `a_caller_cannot_reopen_the_extension_gate` and
    /// `a_caller_cannot_load_a_community_extension` — while the two over-reach
    /// guards stay green, which is the correct signature (they guard the
    /// opposite failure). The claim in this comment is a measurement; if you
    /// change the statement list, re-run that mutation rather than trusting
    /// this paragraph.
    ///
    /// Schemas are snapshotted beside tables because `duckdb_tables()` cannot
    /// see an EMPTY schema — without that, the `CREATE SCHEMA` line below was
    /// unasserted (r2, same finding).
    #[test]
    fn a_caller_query_leaves_the_drawer_catalog_identical() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut store = SqlStore::open(&tmp_store(&dir)).expect("open");
        sync_ambient(&mut store, &fixture_v1(), "b3b:v1").expect("cold");

        // Tables AND schemas: duckdb_tables() cannot see an empty schema.
        let catalog = |s: &SqlStore| -> Vec<String> {
            let mut stmt = s
                .connection()
                .prepare(
                    "SELECT 'T ' || database_name || '.' || schema_name || '.' || table_name \
                       FROM duckdb_tables() \
                     UNION ALL \
                     SELECT 'S ' || database_name || '.' || schema_name FROM duckdb_schemas() \
                     ORDER BY 1",
                )
                .expect("prepare");
            stmt.query_map([], |r| r.get::<_, String>(0))
                .expect("query")
                .collect::<Result<Vec<_>, _>>()
                .expect("rows")
        };

        let before = catalog(&store);
        assert!(!before.is_empty(), "the fixture built a catalog to compare");

        // Everything a caller can throw at it: the extension door, the LEAKING
        // STATEMENT ITSELF (single-statement, as the lane runs it — this is the
        // one that would move the snapshot if the gate were gone), ordinary
        // DDL, DML, and a plain read.
        for statement in [
            "LOAD duckpgq",
            "CREATE PROPERTY GRAPH g VERTEX TABLES (doc)",
            "CREATE OR REPLACE TABLE n AS SELECT path AS id FROM doc",
            "CREATE SCHEMA IF NOT EXISTS sneaky",
            "UPDATE hist.doc SET bytes = 0",
            "SELECT count(*) FROM doc",
        ] {
            let _ = store.query(statement).expect("lane");
        }

        assert_eq!(
            before,
            catalog(&store),
            "a caller query changed the drawer's catalog — that is the whole defect"
        );
    }

    /// The gate must NOT reach past third-party extension code. The NO-SANDBOX
    /// ruling accepted the caller's own file reach — `registry`'s
    /// `sql_lifecycle_over_the_wire` asserts `read_csv('/etc/hosts')` answers
    /// rows — and an earlier draft of this gate broke exactly that by shutting
    /// `enable_external_access`. This is that regression, nailed down on the
    /// write side too, where it is cheap to observe.
    #[test]
    fn the_gate_leaves_external_file_access_alone() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut store = SqlStore::open(&tmp_store(&dir)).expect("open");
        sync_ambient(&mut store, &fixture_v1(), "b3b:v1").expect("cold");

        let out = dir.path().join("caller-owned.csv");
        store
            .query(&format!(
                "COPY (SELECT 1 AS a) TO '{}' (FORMAT CSV)",
                out.display()
            ))
            .expect("lane")
            .expect("the caller's own reach is the accepted posture, not this card's defect");
        assert!(
            out.exists(),
            "the write happened: {} — if this fails, the gate over-reached again",
            out.display()
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

/// Card `sql-set-config-cross-caller-starvation`: a caller's GLOBAL `SET`
/// governs its own call and nothing after it.
///
/// **These tests need TWO calls on ONE instance, never one connection.** A
/// single-connection assertion cannot see this defect at all: the leak IS the
/// second caller, and `SqlStore::query` makes a fresh `try_clone` per call —
/// which is exactly the shape the resident daemon serves (`registry`'s
/// `sql_op::serve` holds one `Arc<Mutex<SqlStore>>` per workspace for the
/// daemon's whole life).
#[cfg(test)]
mod config_scope_tests {
    use super::tests::{fixture_v1, sync_ambient, tmp_store};
    use super::*;

    /// One setting as the NEXT caller would find it — through the query lane,
    /// so the reader is a different connection from the writer.
    fn next_caller_sees(store: &SqlStore, name: &str) -> String {
        let (_, rows) = store
            .query(&format!("SELECT current_setting('{name}')::VARCHAR"))
            .expect("lane")
            .expect("readback");
        rows[0][0].as_str().expect("value").to_owned()
    }

    /// The defect itself, at the grain the card measured it: call 1 sets, call
    /// 2 must not see it. `memory_limit` AND `threads`, so this is about the
    /// GLOBAL class and not one name — `SET memory_limit='1MB'` on a
    /// fleet-shared root was the starvation.
    #[test]
    fn a_caller_set_does_not_reach_the_next_caller() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut store = SqlStore::open(&tmp_store(&dir)).expect("open");
        sync_ambient(&mut store, &fixture_v1(), "b3b:v1").expect("cold");

        let engine_memory = next_caller_sees(&store, "memory_limit");
        let engine_threads = next_caller_sees(&store, "threads");
        assert_ne!(
            engine_memory, "953.6 MiB",
            "the fixture must not already sit at the value call 1 sets, or \
             this test cannot fail"
        );

        // Call 1: the hostile (or merely careless) caller.
        store
            .query("SET memory_limit='1GB'")
            .expect("lane")
            .expect("the caller's own SET still succeeds — NO-SANDBOX");
        assert_eq!(
            next_caller_sees(&store, "memory_limit"),
            engine_memory,
            "call 1's memory_limit reached call 2 — that is the starvation"
        );

        store
            .query("SET threads=2")
            .expect("lane")
            .expect("the caller's own SET still succeeds");
        assert_eq!(
            next_caller_sees(&store, "threads"),
            engine_threads,
            "the leak is the GLOBAL class, not one setting name"
        );
    }

    /// The trap the card named explicitly: the restore must reapply the
    /// ENGINE's `temp_directory`/`max_temp_directory_size`
    /// ([`apply_spill_containment`], card `sql-spill-config-lockout`), never
    /// `DuckDB`'s defaults — a `RESET`-shaped fix would silently unbound the
    /// spill that ENOSPC'ed a host.
    #[test]
    fn restoring_config_reapplies_the_engines_spill_bound_not_duckdbs_default() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut store = SqlStore::open(&tmp_store(&dir)).expect("open");
        sync_ambient(&mut store, &fixture_v1(), "b3b:v1").expect("cold");

        let engine_temp = next_caller_sees(&store, "temp_directory");
        let engine_budget = next_caller_sees(&store, "max_temp_directory_size");
        assert!(
            engine_temp.starts_with(dir.path().to_str().expect("utf8 dir")),
            "precondition: the engine owns temp_directory, drawer-derived: {engine_temp}"
        );

        store
            .query("SET temp_directory='/tmp/mrd-caller-owned-spill'")
            .expect("lane")
            .expect("the caller's own SET succeeds");

        assert_eq!(
            next_caller_sees(&store, "temp_directory"),
            engine_temp,
            "the next caller must spill into the ENGINE's drawer-derived dir, \
             not the previous caller's path and not DuckDB's cwd-relative default"
        );
        assert_eq!(
            next_caller_sees(&store, "max_temp_directory_size"),
            engine_budget,
            "and under the ENGINE's bound, not DuckDB's %-of-disk default"
        );
        assert!(
            !engine_budget.contains('%'),
            "the bound restored is a real size: {engine_budget}"
        );
    }

    /// `DuckDB` derives settings from settings: `memory_limit` re-derives
    /// `write_buffer_row_group_memory_limit`, and `threads` re-derives
    /// `worker_threads`. A single restore pass would leave a derived name at
    /// the caller's value depending on sort order — this pins that the derived
    /// ones come back too.
    #[test]
    fn derived_settings_come_back_with_the_setting_they_derive_from() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut store = SqlStore::open(&tmp_store(&dir)).expect("open");
        sync_ambient(&mut store, &fixture_v1(), "b3b:v1").expect("cold");

        let before: Vec<(String, String)> = ["max_memory", "write_buffer_row_group_memory_limit"]
            .iter()
            .map(|n| ((*n).to_owned(), next_caller_sees(&store, n)))
            .collect();

        store
            .query("SET memory_limit='1GB'")
            .expect("lane")
            .expect("the caller's own SET succeeds");

        for (name, was) in &before {
            assert_eq!(
                &next_caller_sees(&store, name),
                was,
                "{name} is derived from memory_limit and must come back with it"
            );
        }
    }

    /// The error path is not a place config can hide.
    ///
    /// **Why this is two halves and not one.** A caller who moved config and
    /// THEN failed is the case worth guarding, but the lane cannot be handed
    /// one: `run_query` prepares exactly ONE statement, so `SET x; <bad>` never
    /// reaches the `SET`. So this pins the two halves that ARE reachable —
    /// (1) through the lane, a failing statement still answers the caller's own
    /// error, which is the return shape the restore must not disturb; (2) at
    /// function grain, `restore_global_config` puts back a `SET` made on a
    /// connection whose statement then errored, which is exactly the state
    /// `SqlStore::query` hands it when `run_query` returns `Err`.
    ///
    /// This test is deliberately NOT the guard for the lane wiring — it stays
    /// green if the restore call is deleted from
    /// [`SqlStore::query`] (measured 2026-08-24). The wiring is guarded by
    /// `a_caller_set_does_not_reach_the_next_caller` and the two beside it,
    /// which go RED under that mutation.
    #[test]
    fn the_error_path_is_not_a_place_config_can_hide() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut store = SqlStore::open(&tmp_store(&dir)).expect("open");
        sync_ambient(&mut store, &fixture_v1(), "b3b:v1").expect("cold");
        let engine_threads = next_caller_sees(&store, "threads");

        // (1) Through the lane: the caller's own error survives the restore.
        let refusal = store
            .query("SELECT * FROM no_such_table")
            .expect("lane")
            .expect_err("the caller's own SQL failed");
        assert!(
            refusal.contains("no_such_table"),
            "the caller's error is theirs, verbatim, restore or no restore: {refusal}"
        );

        // (2) At function grain: the state the lane hands the restore on Err.
        let conn = store.connection().try_clone().expect("clone");
        let before = global_config_snapshot(&conn).expect("snapshot");
        conn.execute_batch("SET threads=2;").expect("caller SET");
        run_query(&conn, "SELECT * FROM no_such_table")
            .expect_err("the statement fails after the SET");
        assert!(
            restore_global_config(&conn, &before).is_empty(),
            "the restore puts everything back on the error path too"
        );
        drop(conn);

        assert_eq!(
            next_caller_sees(&store, "threads"),
            engine_threads,
            "a failed statement's config change must not outlive it either"
        );
    }

    /// The residue (c) provably cannot close, pinned rather than implied:
    /// `lock_configuration` is one-way BY `DuckDB`'s DESIGN, so no restore can
    /// undo it. What this test asserts is that the residue is a denial of the
    /// DOOR, not starvation — the values it freezes are the ENGINE's, because
    /// the restore pass already ran at the end of the previous call.
    ///
    /// If `DuckDB` ever makes the lock reversible, this test fails and the
    /// residue paragraph on [`restore_global_config`] is what to delete.
    #[test]
    fn a_caller_lock_is_the_one_residue_and_it_freezes_engine_values() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut store = SqlStore::open(&tmp_store(&dir)).expect("open");
        sync_ambient(&mut store, &fixture_v1(), "b3b:v1").expect("cold");

        let engine_memory = next_caller_sees(&store, "memory_limit");

        // Call 1 leaks, and is restored — this is what makes the freeze land
        // on good values rather than the leaked one.
        store.query("SET memory_limit='1GB'").expect("lane").ok();
        // Call 2 locks. One statement per call: `run_query` prepares one, so a
        // caller cannot leak and lock in the same breath.
        store
            .query("SET lock_configuration=true")
            .expect("lane")
            .expect("DuckDB accepts the lock — this is the residue, measured");

        assert_eq!(
            next_caller_sees(&store, "memory_limit"),
            engine_memory,
            "the lock froze the ENGINE's value, not the previous caller's — \
             the residue is a denied door, never starvation"
        );
        let refusal = store
            .query("SET memory_limit='2GB'")
            .expect("lane")
            .expect_err("after a caller's lock, every SET refuses");
        assert!(
            refusal.contains("configuration has been locked"),
            "DuckDB's own words name the residue: {refusal}"
        );
    }

    /// The restore is honest about what it could not put back: it names the
    /// setting instead of reporting a clean pass. Uses a value `DuckDB` will
    /// not accept back (`force_variant_shredding` reports `INVALID`, which is
    /// not a legal input) to drive the unrestorable branch deliberately.
    #[test]
    fn the_restore_names_what_it_could_not_put_back() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let store = SqlStore::open(&tmp_store(&dir)).expect("open");
        let conn = store.connection().try_clone().expect("clone");

        // A snapshot claiming a value DuckDB refuses as input. This is the
        // shape `force_variant_shredding` really has (value `INVALID`), staged
        // here without depending on that one setting keeping that value.
        let before = vec![("memory_limit".to_owned(), "not-a-size".to_owned())];
        let unrestored = restore_global_config(&conn, &before);
        assert_eq!(
            unrestored,
            vec!["memory_limit".to_owned()],
            "a setting that will not go back is NAMED, not silently dropped"
        );

        // And the ordinary path reports nothing.
        let clean = global_config_snapshot(&conn).expect("snapshot");
        assert!(
            restore_global_config(&conn, &clean).is_empty(),
            "an undisturbed instance restores clean"
        );
    }
}
