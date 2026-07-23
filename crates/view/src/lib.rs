//! The `DuckDB` **view-organ** — a write-only leaf that projects the warm parsed
//! corpus into a disposable, fingerprint-stamped `DuckDB` file.
//!
//! # Charter (design: `tournament-duckdb/team-a/design.md`)
//! **Owns:** the rung-5 SQL *projection* of the fact corpus. A pure walk of the
//! resident engine's `BTreeMap<path, Document>` into 8 physical tables + 4 SQL
//! views ([`schema`]). Nothing reads it as truth — disk and fingerprints are
//! correctness, the DB is advisory ([[0003-extension-kernel]] §6).
//!
//! **Never does:** re-parse (it walks already-parsed `model::Document`s), read
//! its own output back into a fact path (view-never-store, enforced by the C2
//! crate-topology test), or write anything on the daemonless path
//! ([`build_memory`] is `:memory:`-only).
//!
//! # Two entry points, split by writer authority (OD6)
//! - [`build_memory`] — daemonless `mrd`: an in-process `:memory:` build for one
//!   query, `built_epoch`/`built_seq` both NULL, **writes nothing to disk**.
//! - [`publish`] — the daemon (sole persistent writer): build a temp file →
//!   `CHECKPOINT` → assert no `.wal` → `chmod 0444` → `fsync` → atomic rename into
//!   the shared drawer. The per-workspace publish mutex is the **caller's**
//!   responsibility (OD6: zero flock code here).

pub mod facts;
pub mod read_face;
pub mod schema;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use cache::CacheDrawer;
use duckdb::Connection;
use duckdb::types::Value;
use model::{CorpusIndex, Document, Node, NodeKind};

pub use read_face::{
    READ_FACE_SCHEMA_SQL, create_read_face_schema, lock_read_face, open_board, stale_paths,
};
pub use schema::{SCHEMA_SQL, SCHEMA_VERSION, create_schema};

/// The fingerprint domain version the ephemeral build folds under. `0` ⇒ the
/// `b3:` token space (the current corpus domain); `model::merkle_root` prefixes
/// the hex accordingly. A daemon `publish` instead carries the daemon's own
/// authoritative `at_fingerprint`, so this constant only serves the daemonless
/// ephemeral stamp.
const FINGERPRINT_VERSION: u32 = 0;

/// The stamp a daemon `publish` writes into the singleton `_meridian_view` row.
/// The daemon is the sole persistent writer, so it carries the authoritative
/// `as_of_fingerprint` (its warm `at_fingerprint`), the workspace path, and the
/// epoch/seq pair (both set — a daemon build, never a daemonless `:memory:` one).
#[derive(Debug, Clone)]
pub struct PublishStamp {
    /// Canonical workspace path (advisory in the stamp; identity is the drawer).
    pub workspace: String,
    /// The workspace content fingerprint the projection was built at
    /// (`MerkleRoot.0`, `"b3:"+hex`) — the durable freshness authority.
    pub as_of_fingerprint: String,
    /// The daemon epoch (pairs with `seq`; a daemon build sets both).
    pub epoch: String,
    /// The wire `changes_seq` at build time (pairs with `epoch`).
    pub seq: u64,
}

/// A view-organ failure. `publish` is the only fallible-on-I/O entry; the
/// variants map the design's abort points (a surviving `.wal`, an ephemeral
/// drawer that cannot persist).
#[derive(Debug)]
pub enum ViewError {
    /// A `DuckDB` error from schema creation, projection, or `CHECKPOINT`.
    Duckdb(duckdb::Error),
    /// A filesystem error (temp create, `chmod`, `fsync`, rename, WAL removal).
    Io(std::io::Error),
    /// A `.wal` sidecar survived the clean `CHECKPOINT` (build-side) or beside
    /// the destination (pre-rename) and could not be removed — the publish is
    /// **aborted, last-good retained**, never a torn/replayed generation.
    WalPresent(PathBuf),
    /// `publish` requires a disk drawer; an ephemeral drawer writes nothing.
    /// Daemonless callers must use [`build_memory`] instead.
    EphemeralDrawer,
}

impl std::fmt::Display for ViewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ViewError::Duckdb(e) => write!(f, "duckdb: {e}"),
            ViewError::Io(e) => write!(f, "io: {e}"),
            ViewError::WalPresent(p) => {
                write!(f, "wal sidecar present, publish aborted: {}", p.display())
            }
            ViewError::EphemeralDrawer => {
                write!(
                    f,
                    "publish requires a disk drawer; use build_memory for an ephemeral build"
                )
            }
        }
    }
}

impl std::error::Error for ViewError {}

impl From<duckdb::Error> for ViewError {
    fn from(e: duckdb::Error) -> Self {
        ViewError::Duckdb(e)
    }
}

impl From<std::io::Error> for ViewError {
    fn from(e: std::io::Error) -> Self {
        ViewError::Io(e)
    }
}

/// Build an ephemeral `:memory:` view over `docs` for one query (daemonless
/// `mrd`, OD6). Creates the schema, projects the corpus, and stamps the
/// singleton `_meridian_view` with `built_epoch`/`built_seq` **both NULL**. The
/// `as_of_fingerprint` is derived from `docs` via `model::merkle_root`, so the
/// stamp reflects exactly the projected content. **Writes nothing to disk.**
///
/// # Errors
/// Propagates any `DuckDB` error from opening the in-memory database, creating the
/// schema, or inserting the projected rows.
pub fn build_memory(docs: &BTreeMap<String, Document>) -> Result<Connection, ViewError> {
    let conn = Connection::open_in_memory()?;
    create_schema(&conn)?;
    project(&conn, docs)?;
    let as_of = ephemeral_fingerprint(docs);
    write_stamp(
        &conn,
        &as_of,
        "",
        None,
        None,
        "view::build_memory",
        docs.len(),
    )?;
    Ok(conn)
}

/// Publish a persistent view for the daemon (the sole persistent writer, OD6).
///
/// Builds a fresh `DuckDB` at `view.<epoch>.<seq>.building` in the drawer, then
/// runs the design's atomic-publication sequence (§Atomic publication + B1):
/// `CHECKPOINT` + close → **assert `<temp>.wal` absent** → `chmod 0444` → `fsync`
/// → **remove any pre-existing destination `view.duckdb.wal`** (`fsync` drawer +
/// assert absent) → `rename(temp, view.duckdb)` → `fsync(drawer)`. On any WAL or
/// I/O abort the last-good `view.duckdb` is retained.
///
/// **The per-workspace publish mutex is the CALLER's responsibility** (daemon
/// side, V2). `publish` assumes it runs under that mutex and adds no flock
/// (OD6: zero flock code). Returns the published `view.duckdb` path.
///
/// # Errors
/// [`ViewError::EphemeralDrawer`] if `drawer` has no disk directory;
/// [`ViewError::Duckdb`] on schema/projection/checkpoint failure;
/// [`ViewError::WalPresent`] if a `.wal` sidecar survives; [`ViewError::Io`] on
/// any filesystem step (create, `chmod`, `fsync`, WAL removal, rename).
pub fn publish(
    docs: &BTreeMap<String, Document>,
    drawer: &CacheDrawer,
    stamp: &PublishStamp,
) -> Result<PathBuf, ViewError> {
    let dir = drawer.dir().ok_or(ViewError::EphemeralDrawer)?;
    let temp = dir.join(format!("view.{}.{}.building", stamp.epoch, stamp.seq));
    let dest = dir.join("view.duckdb");
    let temp_wal = wal_path(&temp);
    let dest_wal = wal_path(&dest);

    // A stale temp from a crashed prior publish must never be reused.
    remove_if_present(&temp)?;
    remove_if_present(&temp_wal)?;

    // 1. Build the candidate into the temp file, then CHECKPOINT + close so it
    //    is self-contained (the WAL folds in).
    {
        let conn = Connection::open(&temp)?;
        create_schema(&conn)?;
        project(&conn, docs)?;
        write_stamp(
            &conn,
            &stamp.as_of_fingerprint,
            &stamp.workspace,
            Some(&stamp.epoch),
            Some(stamp.seq),
            "view::publish",
            docs.len(),
        )?;
        conn.execute_batch("CHECKPOINT")?;
        conn.close().map_err(|(_, e)| ViewError::Duckdb(e))?;
    }

    // 2. B1 build-side: a clean CHECKPOINT leaves no `<temp>.wal`. A surviving
    //    sidecar aborts the publish (keep last-good), never a replayed generation.
    if temp_wal.exists() {
        let _ = std::fs::remove_file(&temp);
        return Err(ViewError::WalPresent(temp_wal));
    }

    // 3. chmod 0444 (read-only, no writable bit) + fsync the candidate. A managed
    //    reader proves no in-place writer can mutate it via this mode check.
    chmod_readonly(&temp)?;
    fsync_path(&temp)?;

    // 4. Remove any pre-existing destination `view.duckdb.wal` (a stale sidecar a
    //    bare rename of `view.duckdb` would strand for the next reader to replay),
    //    fsync the drawer, and assert it is absent — abort + expose on failure.
    remove_if_present(&dest_wal)?;
    fsync_dir(dir)?;
    if dest_wal.exists() {
        return Err(ViewError::WalPresent(dest_wal));
    }

    // 5. Atomic same-dir rename, then fsync the drawer so the new inode is durable.
    std::fs::rename(&temp, &dest)?;
    fsync_dir(dir)?;
    Ok(dest)
}

// ---------------------------------------------------------------------------
// projection — the pure walk of docs into rows
// ---------------------------------------------------------------------------

/// Project every document into the fact tables in FK order: all `doc` rows
/// first, then `frontmatter`, `section`, `link`, `tag`, `frontmatter_tag`,
/// `task`. Ordinals are document-order `UBIGINT`s (a pre-order walk of the
/// span-sorted containment tree yields ascending `span.start`). Links resolve
/// same-corpus via `model::CorpusIndex::resolve_linkpath`; an unresolved vault
/// ref is `dest_path = NULL` (first-class dangling).
fn project(conn: &Connection, docs: &BTreeMap<String, Document>) -> duckdb::Result<()> {
    let index = corpus_index(docs);
    let mut rows = Rows::default();
    for (path, doc) in docs {
        collect_doc(path, doc, &index, &mut rows);
    }
    rows.insert(conn)
}

/// Build the corpus name index (basename + frontmatter-alias) over `docs` — the
/// same stage-1 resolver the engine uses, so round-1 link resolution matches.
fn corpus_index(docs: &BTreeMap<String, Document>) -> CorpusIndex {
    let mut index = CorpusIndex::new();
    for (path, doc) in docs {
        index.insert(path, doc);
    }
    index
}

/// A document-order counter set: each fact table's next ordinal.
#[derive(Default)]
struct Counters {
    section: u64,
    link: u64,
    tag: u64,
    task: u64,
}

/// The projected rows, staged per table then bulk-inserted in FK order.
#[derive(Default)]
struct Rows {
    doc: Vec<Vec<Value>>,
    frontmatter: Vec<Vec<Value>>,
    section: Vec<SectionRow>,
    link: Vec<Vec<Value>>,
    tag: Vec<Vec<Value>>,
    frontmatter_tag: Vec<Vec<Value>>,
    task: Vec<TaskRow>,
}

/// A `section` row — the `hpath` list column is bound via a dynamic `list_value`
/// expression, so it is carried apart from the scalar params.
struct SectionRow {
    scalars_before: Vec<Value>, // path, node_seq
    hpath: Vec<String>,
    scalars_after: Vec<Value>, // heading, level, node_rev, span_start, span_end
}

/// A `task` row — `hpath` is a nullable list (NULL when document-level).
struct TaskRow {
    scalars_before: Vec<Value>, // path, seq, checked, depth, section_seq
    hpath: Option<Vec<String>>,
    scalars_after: Vec<Value>, // text, span_start, span_end, node_rev
}

/// Emit the `doc` row and walk the node tree for one document.
fn collect_doc(path: &str, doc: &Document, index: &CorpusIndex, rows: &mut Rows) {
    let root = &doc.root;
    let line_count = match &root.kind {
        NodeKind::Document { line_count, .. } => *line_count,
        _ => u32::try_from(doc.raw.lines().count()).unwrap_or(u32::MAX),
    };
    rows.doc.push(vec![
        Value::Text(path.to_string()),
        Value::Text(root.node_rev.0.clone()),
        Value::UInt(line_count),
        Value::UBigInt(u64c(doc.raw.len())),
    ]);

    let mut counters = Counters::default();
    walk(root, path, doc, index, None, &mut counters, rows);
}

/// Walk one node, emitting its fact row(s), then recurse into children carrying
/// the governing section identity (`section_seq`) down to any task descendant.
fn walk(
    node: &Node,
    path: &str,
    doc: &Document,
    index: &CorpusIndex,
    gov_section: Option<u64>,
    counters: &mut Counters,
    rows: &mut Rows,
) {
    let mut child_gov = gov_section;
    match &node.kind {
        NodeKind::Frontmatter { map } => emit_frontmatter(node, path, map, rows),
        NodeKind::Section {
            heading_text,
            level,
        } => {
            let node_seq = counters.section;
            counters.section += 1;
            emit_section(node, path, heading_text, *level, node_seq, rows);
            child_gov = Some(node_seq);
        }
        NodeKind::Wikilink {
            target,
            heading,
            block,
            alias,
        } => {
            let dest = resolve_dest(index, target, heading.as_deref(), block.as_deref(), path);
            emit_link(
                node,
                path,
                "wikilink",
                target,
                heading.as_deref(),
                block.as_deref(),
                alias.as_deref(),
                dest,
                counters,
                rows,
            );
        }
        NodeKind::Embed {
            target,
            heading,
            block,
            alias,
        } => {
            let dest = resolve_dest(index, target, heading.as_deref(), block.as_deref(), path);
            emit_link(
                node,
                path,
                "embed",
                target,
                heading.as_deref(),
                block.as_deref(),
                alias.as_deref(),
                dest,
                counters,
                rows,
            );
        }
        NodeKind::Link { target } => {
            emit_link(
                node, path, "link", target, None, None, None, None, counters, rows,
            );
        }
        NodeKind::Tag { name } => emit_tag(node, path, name, counters, rows),
        NodeKind::TaskItem { checked, depth } => {
            emit_task(
                node,
                path,
                *checked,
                *depth,
                gov_section,
                doc,
                counters,
                rows,
            );
        }
        _ => {}
    }
    for child in &node.children {
        walk(child, path, doc, index, child_gov, counters, rows);
    }
}

/// Emit `frontmatter` rows (one per key, first-occurrence-wins via `YamlMap`) and
/// the B2 `frontmatter_tag` rows for the `tag`/`tags` keys. All rows of a doc
/// share the enclosing Frontmatter node's C1 locator triple.
fn emit_frontmatter(node: &Node, path: &str, map: &model::YamlMap, rows: &mut Rows) {
    let (span_start, span_end) = (u64c(node.span.start), u64c(node.span.end));
    let node_rev = node.node_rev.0.clone();
    for (ord, (key, value)) in map.0.iter().enumerate() {
        rows.frontmatter.push(vec![
            Value::Text(path.to_string()),
            Value::UBigInt(u64c(ord)),
            Value::Text(key.clone()),
            Value::Text(value.clone()),
            Value::UBigInt(span_start),
            Value::UBigInt(span_end),
            Value::Text(node_rev.clone()),
        ]);
        if key == "tag" || key == "tags" {
            for (seq, tag) in parse_fm_tags(value).into_iter().enumerate() {
                rows.frontmatter_tag.push(vec![
                    Value::Text(path.to_string()),
                    Value::UBigInt(u64c(seq)),
                    Value::Text(tag),
                    Value::Text(key.clone()),
                    Value::UBigInt(span_start),
                    Value::UBigInt(span_end),
                    Value::Text(node_rev.clone()),
                ]);
            }
        }
    }
}

/// Emit one `section` row (identity `(path, node_seq)`; `hpath` advisory).
fn emit_section(node: &Node, path: &str, heading: &str, level: u8, node_seq: u64, rows: &mut Rows) {
    rows.section.push(SectionRow {
        scalars_before: vec![Value::Text(path.to_string()), Value::UBigInt(node_seq)],
        hpath: node.hpath.clone().unwrap_or_default(),
        scalars_after: vec![
            Value::Text(heading.to_string()),
            Value::UTinyInt(level),
            Value::Text(node.node_rev.0.clone()),
            Value::UBigInt(u64c(node.span.start)),
            Value::UBigInt(u64c(node.span.end)),
        ],
    });
}

/// Emit one `link` row (kind ∈ wikilink/embed/link; `resolved` is generated,
/// never inserted). `dest_path` is the same-corpus resolution or NULL.
#[allow(clippy::too_many_arguments)]
fn emit_link(
    node: &Node,
    path: &str,
    kind: &str,
    target_raw: &str,
    heading: Option<&str>,
    block: Option<&str>,
    alias: Option<&str>,
    dest_path: Option<String>,
    counters: &mut Counters,
    rows: &mut Rows,
) {
    let seq = counters.link;
    counters.link += 1;
    rows.link.push(vec![
        Value::Text(path.to_string()),
        Value::UBigInt(seq),
        Value::Text(kind.to_string()),
        Value::Text(target_raw.to_string()),
        opt_text(heading),
        opt_text(block),
        opt_text(alias),
        dest_path.map_or(Value::Null, Value::Text),
        Value::UBigInt(u64c(node.span.start)),
        Value::UBigInt(u64c(node.span.end)),
        Value::Text(node.node_rev.0.clone()),
    ]);
}

/// Emit one inline `tag` row (`Tag.name`, no leading `#`; a real node span).
fn emit_tag(node: &Node, path: &str, name: &str, counters: &mut Counters, rows: &mut Rows) {
    let seq = counters.tag;
    counters.tag += 1;
    rows.tag.push(vec![
        Value::Text(path.to_string()),
        Value::UBigInt(seq),
        Value::Text(name.to_string()),
        Value::UBigInt(u64c(node.span.start)),
        Value::UBigInt(u64c(node.span.end)),
        Value::Text(node.node_rev.0.clone()),
    ]);
}

/// Emit one `task` row. `section_seq` is the governing section identity (NULL =
/// document-level); `text` is the trimmed task line (identity-bearing, bounded).
#[allow(clippy::too_many_arguments)]
fn emit_task(
    node: &Node,
    path: &str,
    checked: bool,
    depth: u32,
    gov_section: Option<u64>,
    doc: &Document,
    counters: &mut Counters,
    rows: &mut Rows,
) {
    let seq = counters.task;
    counters.task += 1;
    let text = doc
        .raw
        .get(node.span.clone())
        .unwrap_or_default()
        .trim()
        .to_string();
    rows.task.push(TaskRow {
        scalars_before: vec![
            Value::Text(path.to_string()),
            Value::UBigInt(seq),
            Value::Boolean(checked),
            Value::UInt(depth),
            gov_section.map_or(Value::Null, Value::UBigInt),
        ],
        hpath: gov_section.and(node.hpath.clone()),
        scalars_after: vec![
            Value::Text(text),
            Value::UBigInt(u64c(node.span.start)),
            Value::UBigInt(u64c(node.span.end)),
            Value::Text(node.node_rev.0.clone()),
        ],
    });
}

/// Resolve a wikilink/embed target to a same-corpus doc path, or `None`
/// (first-class dangling). A self-ref `[[#H]]` / `[[#^blk]]` (empty target with a
/// heading or block) resolves to the source doc.
fn resolve_dest(
    index: &CorpusIndex,
    target: &str,
    heading: Option<&str>,
    block: Option<&str>,
    src_path: &str,
) -> Option<String> {
    if target.trim().is_empty() {
        return (heading.is_some() || block.is_some()).then(|| src_path.to_string());
    }
    index.resolve_linkpath(target, src_path)
}

/// Scalar-parse a `tag`/`tags` frontmatter value into normalized tags (B2): strip
/// surrounding `[ ]`, split on `,`, then per item trim, strip surrounding quotes
/// and a leading `#`, and drop empties. A block-list value (`''`) yields **0
/// rows** — fail-closed (never wrong rows).
fn parse_fm_tags(value: &str) -> Vec<String> {
    let trimmed = value.trim();
    let inner = trimmed
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(trimmed);
    inner
        .split(',')
        .filter_map(|item| {
            let s = item.trim().trim_matches(['"', '\'']);
            let s = s.strip_prefix('#').unwrap_or(s).trim();
            (!s.is_empty()).then(|| s.to_string())
        })
        .collect()
}

impl Rows {
    /// Bulk-insert all staged rows in FK order (parent `doc` before children).
    fn insert(&self, conn: &Connection) -> duckdb::Result<()> {
        insert_rows(
            conn,
            "INSERT INTO doc (path, file_rev, line_count, bytes) VALUES (?, ?, ?, ?)",
            &self.doc,
        )?;
        insert_rows(
            conn,
            "INSERT INTO frontmatter (path, ord, key, value, span_start, span_end, node_rev) VALUES (?, ?, ?, ?, ?, ?, ?)",
            &self.frontmatter,
        )?;
        for row in &self.section {
            let hpath_expr = list_expr(row.hpath.len());
            let sql = format!(
                "INSERT INTO section (path, node_seq, hpath, heading, level, node_rev, span_start, span_end) VALUES (?, ?, {hpath_expr}, ?, ?, ?, ?, ?)"
            );
            let params = chain_list(&row.scalars_before, &row.hpath, &row.scalars_after);
            conn.execute(&sql, duckdb::params_from_iter(params.iter()))?;
        }
        insert_rows(
            conn,
            "INSERT INTO link (src_path, seq, kind, target_raw, heading, block, alias, dest_path, span_start, span_end, node_rev) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            &self.link,
        )?;
        insert_rows(
            conn,
            "INSERT INTO tag (path, seq, tag, span_start, span_end, node_rev) VALUES (?, ?, ?, ?, ?, ?)",
            &self.tag,
        )?;
        insert_rows(
            conn,
            "INSERT INTO frontmatter_tag (path, seq, tag, key, span_start, span_end, node_rev) VALUES (?, ?, ?, ?, ?, ?, ?)",
            &self.frontmatter_tag,
        )?;
        for row in &self.task {
            let hpath_expr = match &row.hpath {
                Some(h) => list_expr(h.len()),
                None => "NULL".to_string(),
            };
            let sql = format!(
                "INSERT INTO task (path, seq, checked, depth, section_seq, hpath, text, span_start, span_end, node_rev) VALUES (?, ?, ?, ?, ?, {hpath_expr}, ?, ?, ?, ?)"
            );
            let empty: Vec<String> = Vec::new();
            let hpath = row.hpath.as_ref().unwrap_or(&empty);
            let params = chain_list(&row.scalars_before, hpath, &row.scalars_after);
            conn.execute(&sql, duckdb::params_from_iter(params.iter()))?;
        }
        Ok(())
    }
}

/// A `list_value(?, ?, …)` expression with `n` placeholders, or an empty
/// `VARCHAR[]` literal for `n == 0`.
fn list_expr(n: usize) -> String {
    if n == 0 {
        return "[]::VARCHAR[]".to_string();
    }
    let marks = std::iter::repeat_n("?", n).collect::<Vec<_>>().join(", ");
    format!("list_value({marks})")
}

/// Splice scalar-before params, the list elements, and scalar-after params into
/// one positional param vector matching a `list_value` INSERT.
fn chain_list(before: &[Value], list: &[String], after: &[Value]) -> Vec<Value> {
    let mut params = before.to_vec();
    params.extend(list.iter().map(|s| Value::Text(s.clone())));
    params.extend(after.iter().cloned());
    params
}

/// Prepare `sql` once and execute it per staged row.
fn insert_rows(conn: &Connection, sql: &str, rows: &[Vec<Value>]) -> duckdb::Result<()> {
    let mut stmt = conn.prepare(sql)?;
    for row in rows {
        stmt.execute(duckdb::params_from_iter(row.iter()))?;
    }
    Ok(())
}

/// Insert the singleton `_meridian_view` stamp row. `epoch`/`seq` are both set
/// (daemon build) or both NULL (daemonless `:memory:` build) — the DDL CHECK
/// enforces the pairing.
fn write_stamp(
    conn: &Connection,
    as_of_fingerprint: &str,
    workspace: &str,
    epoch: Option<&str>,
    seq: Option<u64>,
    builder: &str,
    doc_count: usize,
) -> duckdb::Result<()> {
    let built_unix = i64::try_from(now_secs()).unwrap_or(i64::MAX);
    conn.execute(
        "INSERT INTO _meridian_view \
         (singleton, schema_version, as_of_fingerprint, workspace, built_epoch, built_seq, built_unix, builder, doc_count) \
         VALUES (true, ?, ?, ?, ?, ?, ?, ?, ?)",
        duckdb::params_from_iter(
            [
                Value::Int(SCHEMA_VERSION),
                Value::Text(as_of_fingerprint.to_string()),
                Value::Text(workspace.to_string()),
                epoch.map_or(Value::Null, |e| Value::Text(e.to_string())),
                seq.map_or(Value::Null, Value::UBigInt),
                Value::BigInt(built_unix),
                Value::Text(builder.to_string()),
                Value::UBigInt(u64c(doc_count)),
            ]
            .iter(),
        ),
    )?;
    Ok(())
}

/// The ephemeral build's `as_of` fingerprint — `merkle_root` over the projected
/// `docs` (their raw bytes), a self-consistent content hash in the `b3:` domain.
fn ephemeral_fingerprint(docs: &BTreeMap<String, Document>) -> String {
    let files: Vec<(&str, &[u8])> = docs
        .iter()
        .map(|(path, doc)| (path.as_str(), doc.raw.as_bytes()))
        .collect();
    model::merkle_root(&files, FINGERPRINT_VERSION).0
}

// ---------------------------------------------------------------------------
// filesystem helpers (publish side)
// ---------------------------------------------------------------------------

/// The `<path>.wal` sidecar path `DuckDB` would write beside `path`.
fn wal_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".wal");
    PathBuf::from(s)
}

/// Remove `path` if it exists; a missing file is not an error.
fn remove_if_present(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// `chmod 0444` — read-only, no writable bit (managed readers verify this mode).
fn chmod_readonly(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o444))
    }
    #[cfg(not(unix))]
    {
        let mut perms = std::fs::metadata(path)?.permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(path, perms)
    }
}

/// `fsync` a file's contents to disk.
fn fsync_path(path: &Path) -> std::io::Result<()> {
    std::fs::File::open(path)?.sync_all()
}

/// `fsync` a directory so a create/rename/unlink within it is durable.
fn fsync_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::File::open(dir)?.sync_all()
}

/// Current unix time in whole seconds (never panics; `0` before the epoch).
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `usize` → `u64`, saturating (never truncates on a 32-bit target).
fn u64c(x: usize) -> u64 {
    u64::try_from(x).unwrap_or(u64::MAX)
}

/// An optional string as a `DuckDB` `Value` (`Text` or `Null`).
fn opt_text(s: Option<&str>) -> Value {
    s.map_or(Value::Null, |s| Value::Text(s.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fm_tags_inline_list_two_rows() {
        assert_eq!(
            parse_fm_tags("[type/agent, type/task]"),
            vec!["type/agent", "type/task"]
        );
    }

    #[test]
    fn parse_fm_tags_block_form_empty_is_zero_rows() {
        assert!(parse_fm_tags("").is_empty());
    }

    #[test]
    fn parse_fm_tags_strips_hash_and_quotes() {
        assert_eq!(parse_fm_tags("['#foo', \"bar\"]"), vec!["foo", "bar"]);
    }

    #[test]
    fn build_memory_over_empty_corpus_stamps_singleton() {
        let docs = BTreeMap::new();
        let conn = build_memory(&docs).expect("build");
        let n: i64 = conn
            .query_row("SELECT doc_count FROM _meridian_view", [], |r| r.get(0))
            .expect("stamp");
        assert_eq!(n, 0);
    }
}
