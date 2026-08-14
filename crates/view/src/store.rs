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
//!   covers, so the file is always at exactly one fingerprint.
//! - **`main.*`** — the caller-facing latest views, keeping the ephemeral
//!   projection's exact names and column shapes (`doc`, `section`, `link`,
//!   `backlink`, `dangling`, `card`, `tag_all`, `task`, `frontmatter`,
//!   `frontmatter_tag`, `tag`, `_meridian_view`): one `QUALIFY` window over
//!   `hist.doc` picks each path's newest version and drops tombstones; child
//!   views follow by `(path, gen)` semi join.
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
//! Disclosed approximations (repair = the rebuild verb): motion OUTSIDE the
//! pinned corpus — mount-table edits, other roots' contents (cross-root
//! `dest_root_path`), and non-md files entering/leaving the `exclusion`
//! domain — moves no fingerprint and therefore triggers no append.
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

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use duckdb::Connection;
use duckdb::types::Value;
use model::Document;
use serde_json::Value as Json;

use crate::sqltext;
use crate::{ExclusionProbe, Rows, ViewError, collect_doc, corpus_index, fill_exclusions};

/// The cache file's own payload schema version, recorded in every pin row.
/// Bump it — together with the drawer path's `SCHEMA_SALT` — whenever the
/// hist DDL changes; a mismatched file is deleted and cold-rebuilt, never
/// migrated (the cache is a pure function of the corpus).
///
/// `2`: `main.dangling` gained `AND exclusion IS NULL`, and the exclusion
/// mint's bare-name fallback changed stamped content for identical corpora —
/// a v1 file would serve pre-ruling rows at the same fingerprint.
pub const CACHE_SCHEMA_VERSION: i64 = 2;

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
    span_start UBIGINT, span_end UBIGINT, node_rev TEXT
);
CREATE TABLE hist.section (
    path TEXT, gen BIGINT, node_seq UBIGINT, hpath TEXT[], heading TEXT,
    level UTINYINT, node_rev TEXT, span_start UBIGINT, span_end UBIGINT
);
CREATE TABLE hist.link (
    src_path TEXT, gen BIGINT, seq UBIGINT, kind TEXT, target_raw TEXT,
    heading TEXT, block TEXT, alias TEXT,
    dest_path TEXT, dest_root TEXT, dest_root_path TEXT, exclusion TEXT,
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
    section_seq UBIGINT, hpath TEXT[], text TEXT,
    span_start UBIGINT, span_end UBIGINT, node_rev TEXT
);
CREATE TABLE hist.pin (
    gen BIGINT, fingerprint VARCHAR, applied_at TIMESTAMP,
    files_added BIGINT, files_changed BIGINT, files_removed BIGINT,
    engine_version VARCHAR, cache_schema_version BIGINT
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
    SELECT f.path, f.ord, f.key, f.value, f.span_start, f.span_end, f.node_rev
    FROM hist.frontmatter f
    SEMI JOIN hist.doc_latest d ON f.path = d.path AND f.gen = d.gen;
CREATE VIEW main.section AS
    SELECT s.path, s.node_seq, s.hpath, s.heading, s.level, s.node_rev,
           s.span_start, s.span_end
    FROM hist.section s
    SEMI JOIN hist.doc_latest d ON s.path = d.path AND s.gen = d.gen;
CREATE VIEW main.link AS
    SELECT l.src_path, l.seq, l.kind, l.target_raw, l.heading, l.block,
           l.alias, l.dest_path, l.dest_root, l.dest_root_path, l.exclusion,
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

-- Convenience views: today's definitions verbatim, now over the latest views.
CREATE VIEW main.backlink AS
    SELECT dest_path AS path, src_path, kind, alias FROM main.link
    WHERE dest_path IS NOT NULL;
CREATE VIEW main.dangling AS
    SELECT src_path, target_raw FROM main.link
    WHERE kind IN ('wikilink','embed') AND dest_path IS NULL AND dest_root IS NULL
      AND exclusion IS NULL;
CREATE VIEW main.card AS
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
    SELECT fingerprint AS as_of_fingerprint, gen, applied_at, cache_schema_version
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

/// The sandbox profile a query connection runs under (B5). `Local` is the
/// trusted CLI: memory cap only. `Agent` is every untrusted lane (the MCP
/// face, the wire op): caps set, external access off, configuration locked.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ExecProfile {
    /// Trusted local CLI: `memory_limit` only, no sandbox.
    Local,
    /// Untrusted agent / proxy: the locked `DuckDB` sandbox (B5 order).
    Agent,
}

impl ExecProfile {
    /// The wire/CLI spelling of the profile.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            ExecProfile::Local => "local",
            ExecProfile::Agent => "agent",
        }
    }
}

/// The `local` (trusted) profile RAM cap — generous, only to avoid exhausting
/// host RAM; the user already has a shell (B5).
const LOCAL_MEMORY_LIMIT: &str = "4GB";
/// The `agent` (untrusted) profile RAM cap (B5).
const AGENT_MEMORY_LIMIT: &str = "512MB";
/// The `agent` profile CPU fan-out bound (B5).
const AGENT_THREADS: i64 = 1;
/// The `agent` profile deep-parse-bomb bound (B5; `DuckDB` default is 1000).
const AGENT_MAX_EXPRESSION_DEPTH: u32 = 1000;
/// Both profiles' spill budget (card sql-spill-config-lockout): the `DuckDB`
/// default is 90% of available disk — effectively unbounded, and it `ENOSPC`ed
/// a host (one query spilled >9 GiB into the seat's cwd). Bounded for local
/// too: the operator's shell is trusted, the disk is shared.
const SPILL_BUDGET: &str = "8GiB";

/// Apply the profile's resource limits to `conn` (B5). Order is load-bearing
/// for `agent`: set the caps, disable external access, then lock the
/// configuration so untrusted SQL cannot re-raise any of it. The cache file
/// must already be open on this connection — `enable_external_access=false`
/// blocks opening new files, never the database already attached. There is no
/// `statement_timeout` pragma — a wall-clock cap is the parent's process kill.
///
/// `spill_dir` (card sql-spill-config-lockout) becomes the ABSOLUTE
/// `temp_directory`, with [`SPILL_BUDGET`] as `max_temp_directory_size`, on
/// BOTH profiles and BEFORE any lock: the locked defaults were a small
/// memory cap (spills early), a 90%-of-disk budget, and a `.tmp` path
/// RELATIVE to the shell cwd — no flag changes the process cwd, so spills
/// landed wherever the seat happened to be, and the agent lock froze it all.
///
/// # Errors
/// Propagates the `DuckDB` error from the pragma batch.
pub fn apply_profile(
    conn: &Connection,
    profile: ExecProfile,
    spill_dir: &Path,
) -> duckdb::Result<()> {
    // Best-effort: DuckDB creates the directory on first spill too.
    let _ = std::fs::create_dir_all(spill_dir);
    let spill = spill_dir.display().to_string().replace('\'', "''");
    let containment = format!(
        "SET temp_directory='{spill}';\n\
         SET max_temp_directory_size='{SPILL_BUDGET}';\n"
    );
    let sql = match profile {
        ExecProfile::Local => format!("{containment}SET memory_limit='{LOCAL_MEMORY_LIMIT}';"),
        ExecProfile::Agent => format!(
            "{containment}\
             SET memory_limit='{AGENT_MEMORY_LIMIT}';\n\
             SET threads={AGENT_THREADS};\n\
             SET max_expression_depth={AGENT_MAX_EXPRESSION_DEPTH};\n\
             SET enable_external_access=false;\n\
             SET lock_configuration=true;"
        ),
    };
    conn.execute_batch(&sql)
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

/// Extend a view-DML refusal with its remedy (ruling OQ1: the refusal
/// teaches). `DuckDB`'s words stay verbatim and first; the teaching follows.
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
    error.to_owned()
}

/// The fingerprint-pinned append-only cache file, open read-write.
///
/// The holder is the file's single writer for its lifetime (`DuckDB`'s own
/// lock excludes every other process, read-only included — receipt P4). The
/// base connection stays unsandboxed for appends; caller queries run on
/// [`SqlStore::query`]'s profile-sandboxed clone.
pub struct SqlStore {
    conn: Connection,
    file: PathBuf,
    /// The profile whose pragmas are in force on the database instance.
    /// `lock_configuration` (and every other `SET`) is instance-global in
    /// `DuckDB`, not per-connection — measured: a second `apply_profile`
    /// after an `agent` lock refuses. So the sandbox is applied once and
    /// recorded here; an `agent` lock can never be relaxed for the file's
    /// lifetime.
    sandbox: std::cell::Cell<Option<ExecProfile>>,
}

impl std::fmt::Debug for SqlStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqlStore")
            .field("file", &self.file)
            .field("sandbox", &self.sandbox.get().map(ExecProfile::label))
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
            sandbox: std::cell::Cell::new(None),
        };
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
        Ok(SqlStore {
            conn,
            file: file.to_path_buf(),
            sandbox: std::cell::Cell::new(None),
        })
    }

    /// The file's current pin, `None` for a fresh (never-appended) file.
    ///
    /// # Errors
    /// The pin ledger cannot be read (treated by [`SqlStore::open`] as a
    /// delete-and-recreate condition).
    pub fn pin(&self) -> Result<Option<Pin>, ViewError> {
        let mut stmt = self.conn.prepare(
            "SELECT gen, fingerprint, cache_schema_version \
             FROM hist.pin ORDER BY gen DESC LIMIT 1",
        )?;
        let mut rows = stmt.query([])?;
        match rows.next()? {
            Some(row) => Ok(Some(Pin {
                generation: row.get(0)?,
                fingerprint: row.get(1)?,
                cache_schema_version: row.get(2)?,
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
        docs: &BTreeMap<String, Document>,
        corpus: &model::RootedCorpus<'_>,
        mounts: Option<&addr::MountSet>,
        exclusion: Option<ExclusionProbe<'_>>,
        fingerprint: &str,
    ) -> Result<Option<AppendCounts>, ViewError> {
        let mut pin = self.pin()?;
        if let Some(p) = &pin {
            if p.fingerprint == fingerprint {
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

        // Unchanged docs whose link resolution the delta can move.
        let affected = if pin.is_none() {
            BTreeSet::new()
        } else {
            self.resolution_affected(docs, &added, &changed, &removed)?
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

        self.append(&rows, &removed, counts, fingerprint)?;
        Ok(Some(counts))
    }

    /// Unchanged docs holding a link whose target names a moved name: the
    /// name keys of added/removed paths (stem + full key) plus the aliases
    /// entering or leaving through added/changed/removed docs, matched
    /// against every latest link row's target keys (module docs § Delta
    /// grain — a superset of the resolver's keys, never a subset).
    fn resolution_affected(
        &self,
        docs: &BTreeMap<String, Document>,
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
    ) -> Result<(), ViewError> {
        self.conn.execute_batch("BEGIN")?;
        let result = self.append_body(rows, removed, counts, fingerprint);
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
        self.append_section(rows, &generation)?;
        self.append_task(rows, &generation)?;

        self.conn.execute(
            "INSERT INTO hist.pin \
             (gen, fingerprint, applied_at, files_added, files_changed, files_removed, \
              engine_version, cache_schema_version) \
             VALUES (?, ?, now(), ?, ?, ?, ?, ?)",
            duckdb::params_from_iter(
                [
                    generation,
                    Value::Text(fingerprint.to_owned()),
                    Value::BigInt(i64_len(counts.added)),
                    Value::BigInt(i64_len(counts.changed)),
                    Value::BigInt(i64_len(counts.removed)),
                    Value::Text(env!("CARGO_PKG_VERSION").to_owned()),
                    Value::BigInt(CACHE_SCHEMA_VERSION),
                ]
                .iter(),
            ),
        )?;
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

    /// `hist.section` — the `hpath` list column rides the same stage-table
    /// workaround as the ephemeral lane ([`crate::hpath_join`]): a TEMP stage
    /// table (memory-backed, never file churn), appender-loaded, decoded by
    /// one set-based `INSERT..SELECT`.
    fn append_section(&self, rows: &Rows, generation: &Value) -> Result<(), ViewError> {
        self.conn.execute_batch(
            "CREATE TEMP TABLE _stage_section (path TEXT, gen BIGINT, node_seq UBIGINT, hpath_j TEXT, heading TEXT, level UTINYINT, node_rev TEXT, span_start UBIGINT, span_end UBIGINT);",
        )?;
        {
            let mut app = self
                .conn
                .appender_to_catalog_and_db("_stage_section", "temp", "main")?;
            for row in &rows.section {
                let mut params: Vec<Value> = Vec::with_capacity(9);
                params.push(row.scalars_before[0].clone());
                params.push(generation.clone());
                params.extend(row.scalars_before[1..].iter().cloned());
                params.push(crate::hpath_join(&row.hpath));
                params.extend(row.scalars_after.iter().cloned());
                app.append_row(duckdb::appender_params_from_iter(params.iter()))?;
            }
            app.flush()?;
        }
        self.conn.execute_batch(
            "INSERT INTO hist.section SELECT path, gen, node_seq, CASE WHEN hpath_j = '' THEN []::TEXT[] ELSE string_split(hpath_j, chr(31))[2:] END, heading, level, node_rev, span_start, span_end FROM _stage_section; \
             DROP TABLE _stage_section;",
        )?;
        Ok(())
    }

    /// `hist.task` — as `hist.section`, with NULL `hpath` kept NULL.
    fn append_task(&self, rows: &Rows, generation: &Value) -> Result<(), ViewError> {
        self.conn.execute_batch(
            "CREATE TEMP TABLE _stage_task (path TEXT, gen BIGINT, seq UBIGINT, checked BOOLEAN, depth UINTEGER, section_seq UBIGINT, hpath_j TEXT, text TEXT, span_start UBIGINT, span_end UBIGINT, node_rev TEXT);",
        )?;
        {
            let mut app = self
                .conn
                .appender_to_catalog_and_db("_stage_task", "temp", "main")?;
            for row in &rows.task {
                let mut params: Vec<Value> = Vec::with_capacity(11);
                params.push(row.scalars_before[0].clone());
                params.push(generation.clone());
                params.extend(row.scalars_before[1..].iter().cloned());
                params.push(row.hpath.as_deref().map_or(Value::Null, crate::hpath_join));
                params.extend(row.scalars_after.iter().cloned());
                app.append_row(duckdb::appender_params_from_iter(params.iter()))?;
            }
            app.flush()?;
        }
        self.conn.execute_batch(
            "INSERT INTO hist.task SELECT path, gen, seq, checked, depth, section_seq, CASE WHEN hpath_j IS NULL THEN NULL WHEN hpath_j = '' THEN []::TEXT[] ELSE string_split(hpath_j, chr(31))[2:] END, text, span_start, span_end, node_rev FROM _stage_task; \
             DROP TABLE _stage_task;",
        )?;
        Ok(())
    }

    /// Run one caller query through the rollback lane on a clone of the base
    /// connection: `BEGIN → statement → collect → ROLLBACK`. The profile's
    /// pragmas are applied ONCE per store — they are instance-global, so the
    /// first `agent` query locks the whole file's configuration for this
    /// holder's lifetime (open-before-sandbox-pragmas by construction; the
    /// lock never blocks appends — inserts on the open database need no
    /// external access and no configuration change).
    ///
    /// # Errors
    /// A connection/sandbox failure is a `ViewError`; the caller's own SQL
    /// failing is `Ok` with the error string in the result (their register,
    /// not ours) — including `DuckDB`'s MVCC transaction conflicts, verbatim.
    #[allow(clippy::type_complexity)]
    pub fn query(
        &self,
        profile: ExecProfile,
        sql: &str,
    ) -> Result<Result<(Vec<ColMeta>, Vec<Vec<Json>>), String>, ViewError> {
        let conn = self.conn.try_clone()?;
        match self.sandbox.get() {
            Some(applied) if applied == profile => {} // already in force, instance-wide
            Some(ExecProfile::Agent) => {
                // The agent lock cannot be relaxed (lock_configuration).
                return Err(ViewError::Io(std::io::Error::other(
                    "the agent sandbox is locked on this cache instance; a local-profile query needs its own process",
                )));
            }
            _ => {
                // The drawer-derived spill home: absolute, beside the cache
                // file, reaped with the drawer — never the shell cwd.
                apply_profile(&conn, profile, &self.spill_dir())?;
                self.sandbox.set(Some(profile));
            }
        }
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

    /// The drawer-derived spill directory query connections are pointed at
    /// (card sql-spill-config-lockout).
    #[must_use]
    pub fn spill_dir(&self) -> PathBuf {
        self.file
            .parent()
            .map_or_else(|| PathBuf::from("sql-spill"), |p| p.join("sql-spill"))
    }

    /// The base (unsandboxed) connection — appends, pin reads, tests.
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
    pub(crate) fn corpus(files: &[(&str, &str)]) -> BTreeMap<String, Document> {
        files
            .iter()
            .map(|(p, raw)| {
                (
                    (*p).to_string(),
                    model::build((*raw).to_string(), syntax::parse(raw)),
                )
            })
            .collect()
    }

    /// Sync `docs` into `store` at `fingerprint` (ambient corpus, no mounts —
    /// mirrors the [`crate::build_memory`] reference build).
    pub(crate) fn sync_ambient(
        store: &mut SqlStore,
        docs: &BTreeMap<String, Document>,
        fingerprint: &str,
    ) -> Option<AppendCounts> {
        let ambient = RootedCorpus::ambient(docs);
        store
            .sync(docs, &ambient, None, None, fingerprint)
            .expect("sync")
    }

    /// Per-surface digest over every column of every row — the 7 base tables
    /// plus the 4 convenience views, so the acceptance is the whole caller
    /// surface, not just storage.
    const SURFACE_DIGESTS: [(&str, &str); 11] = [
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
            "SELECT coalesce(md5(string_agg(path || '|' || node_seq::VARCHAR || '|' || len(hpath)::VARCHAR || '#' || array_to_string(hpath, chr(31)) || '|' || heading || '|' || level::VARCHAR || '|' || node_rev || '|' || span_start::VARCHAR || '|' || span_end::VARCHAR, chr(10) ORDER BY path, node_seq)), 'EMPTY') FROM section",
        ),
        (
            "link",
            "SELECT coalesce(md5(string_agg(src_path || '|' || seq::VARCHAR || '|' || kind || '|' || target_raw || '|' || coalesce(heading,'~N~') || '|' || coalesce(block,'~N~') || '|' || coalesce(alias,'~N~') || '|' || coalesce(dest_path,'~N~') || '|' || coalesce(dest_root,'~N~') || '|' || coalesce(dest_root_path,'~N~') || '|' || coalesce(exclusion,'~N~') || '|' || resolved::VARCHAR || '|' || span_start::VARCHAR || '|' || span_end::VARCHAR || '|' || node_rev, chr(10) ORDER BY src_path, seq)), 'EMPTY') FROM link",
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
            "SELECT coalesce(md5(string_agg(path || '|' || seq::VARCHAR || '|' || checked::VARCHAR || '|' || depth::VARCHAR || '|' || coalesce(section_seq::VARCHAR,'~N~') || '|' || CASE WHEN hpath IS NULL THEN '~N~' ELSE len(hpath)::VARCHAR || '#' || array_to_string(hpath, chr(31)) END || '|' || text || '|' || span_start::VARCHAR || '|' || span_end::VARCHAR || '|' || node_rev, chr(10) ORDER BY path, seq)), 'EMPTY') FROM task",
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
            "card",
            "SELECT coalesce(md5(string_agg(path || '|' || coalesce(type,'~N~') || '|' || coalesce(status,'~N~') || '|' || coalesce(owner,'~N~') || '|' || coalesce(session,'~N~'), chr(10) ORDER BY path)), 'EMPTY') FROM card",
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
    fn assert_surface_matches_fresh(store: &SqlStore, docs: &BTreeMap<String, Document>) {
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
    pub(crate) fn fixture_v1() -> BTreeMap<String, Document> {
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
                "INSERT INTO hist.pin VALUES (2, 'b3b:v1', now(), 0, 0, 0, 'future', 999)",
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
                ExecProfile::Local,
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
                ExecProfile::Local,
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
            .query(ExecProfile::Local, "UPDATE hist.doc SET bytes = 0")
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
            .query(ExecProfile::Local, "UPDATE doc SET bytes = 0")
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
    fn agent_profile_queries_are_sandboxed_and_appends_stay_unrestricted() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut store = SqlStore::open(&tmp_store(&dir)).expect("open");
        sync_ambient(&mut store, &fixture_v1(), "b3b:v1").expect("cold");

        let err = store
            .query(ExecProfile::Agent, "SELECT * FROM read_csv('/etc/hosts')")
            .expect("lane")
            .expect_err("external access is off under agent");
        assert!(!err.is_empty());
        let err = store
            .query(ExecProfile::Agent, "SET memory_limit='8GB'")
            .expect("lane")
            .expect_err("configuration is locked under agent");
        assert!(!err.is_empty());

        // The sandbox rode the clone, never the base connection: a later
        // append still writes.
        let v2 = corpus(&[("a.md", "# Only\n")]);
        sync_ambient(&mut store, &v2, "b3b:v2").expect("append after sandboxed queries");
        assert_surface_matches_fresh(&store, &v2);
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
    fn spill_config_is_absolute_and_bounded_before_the_lock() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut store = SqlStore::open(&tmp_store(&dir)).expect("open");
        sync_ambient(&mut store, &fixture_v1(), "b3b:v1").expect("cold");

        let (_, rows) = store
            .query(
                ExecProfile::Agent,
                "SELECT name, value FROM duckdb_settings() \
                 WHERE name IN ('temp_directory','max_temp_directory_size','lock_configuration') \
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
        assert_eq!(value("lock_configuration"), "true");
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

        // And the lock still refuses caller overrides (T1/T2 stay true).
        let refused = store
            .query(ExecProfile::Agent, "SET temp_directory='/tmp/elsewhere'")
            .expect("lane")
            .expect_err("locked");
        assert!(refused.contains("locked"), "{refused}");
    }

    /// BOTH profiles get the containment — local is unlocked but must not
    /// default to the loaded gun either.
    #[test]
    fn local_profile_carries_the_same_spill_containment() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let mut store = SqlStore::open(&tmp_store(&dir)).expect("open");
        sync_ambient(&mut store, &fixture_v1(), "b3b:v1").expect("cold");
        let (_, rows) = store
            .query(
                ExecProfile::Local,
                "SELECT value FROM duckdb_settings() WHERE name = 'max_temp_directory_size'",
            )
            .expect("lane")
            .expect("readback");
        let budget = rows[0][0].as_str().expect("value");
        assert!(!budget.contains('%'), "local is bounded too: {budget}");
    }
}
