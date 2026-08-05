//! `DuckDB` **view-organ** — write-only leaf projecting the warm parsed corpus
//! into a disposable, fingerprint-stamped `DuckDB` file.
//!
//! # Charter
//! **Owns:** rung-5 SQL projection of the fact corpus: walk
//! `BTreeMap<path, Document>` into 8 tables + 4 views ([`schema`]). Disk and
//! fingerprints are correctness; the DB is advisory.
//!
//! **Never does:** re-parse, read its own output into a fact path
//! (view-never-store; C2 topology gate), or write on the daemonless path
//! ([`build_memory`] is `:memory:`-only).
//!
//! # Entry points (OD6 writer authority)
//! - [`build_memory`] — daemonless: in-process `:memory:`, epoch/seq NULL,
//!   **writes nothing to disk**.
//! - [`publish`] — daemon sole persistent writer: temp → `CHECKPOINT` → no
//!   `.wal` → `chmod 0444` → `fsync` → atomic rename. Publish mutex is the
//!   **caller's** (zero flock code here).

pub mod facts;
pub mod read_face;
pub mod schema;
pub mod walk;

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

/// Daemon `publish` stamp for the singleton `_meridian_view` row: authoritative
/// `as_of_fingerprint`, workspace, and epoch/seq (both set — never daemonless).
#[derive(Debug, Clone)]
pub struct PublishStamp {
    /// Canonical workspace path (advisory; identity is the drawer).
    pub workspace: String,
    /// Workspace content fingerprint at build (`MerkleRoot.0`) — freshness authority.
    pub as_of_fingerprint: String,
    /// Daemon epoch (pairs with `seq`).
    pub epoch: String,
    /// Wire `changes_seq` at build time (pairs with `epoch`).
    pub seq: u64,
}

/// View-organ failure. `publish` is the only fallible-on-I/O entry.
#[derive(Debug)]
pub enum ViewError {
    /// `DuckDB` error from schema, projection, or `CHECKPOINT`.
    Duckdb(duckdb::Error),
    /// Filesystem error (temp, `chmod`, `fsync`, rename, WAL removal).
    Io(std::io::Error),
    /// `.wal` survived clean `CHECKPOINT` or pre-rename cleanup — publish
    /// **aborted, last-good retained**.
    WalPresent(PathBuf),
    /// `publish` needs a disk drawer; use [`build_memory`] for ephemeral.
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

/// Ephemeral `:memory:` view over `docs` (daemonless `mrd`, OD6). Stamps
/// `_meridian_view` with epoch/seq **both NULL**. **Writes nothing to disk.**
///
/// `as_of` is the caller's authoritative corpus fold for these `docs` (from
/// `fs::domain_snapshot`). This crate must not refold: a refold is blind to
/// the domain filter and `version` prefix (§12.3), so it stamps `b3:` in a
/// `b3b:` workspace and freshness reports STALE over identical content (G14).
///
/// # Errors
/// Propagates any `DuckDB` error from open, schema, or insert.
pub fn build_memory(
    docs: &BTreeMap<String, Document>,
    as_of: &str,
) -> Result<Connection, ViewError> {
    let conn = Connection::open_in_memory()?;
    create_schema(&conn)?;
    project(&conn, docs, &model::RootedCorpus::ambient(docs), None)?;
    write_stamp(
        &conn,
        as_of,
        "",
        None,
        None,
        "view::build_memory",
        docs.len(),
    )?;
    Ok(conn)
}

/// [`build_memory`] against a root-keyed corpus and mount table (U21).
///
/// Cross-root edges land in `link.dest_root` + `link.dest_root_path`, never
/// `dest_path` (FK into `doc`; cross-root dest is not a key in this corpus).
///
/// Ambient callers pass **no** mount authority, not an empty table: empty
/// means "looked and bound nothing"; absent means "never consulted".
///
/// # Errors
/// As [`build_memory`].
pub fn build_memory_rooted(
    docs: &BTreeMap<String, Document>,
    corpus: &model::RootedCorpus<'_>,
    mounts: &addr::MountSet,
    as_of: &str,
) -> Result<Connection, ViewError> {
    let conn = Connection::open_in_memory()?;
    create_schema(&conn)?;
    project(&conn, docs, corpus, Some(mounts))?;
    write_stamp(
        &conn,
        as_of,
        "",
        None,
        None,
        "view::build_memory_rooted",
        docs.len(),
    )?;
    Ok(conn)
}

/// Publish a persistent view (daemon sole writer, OD6).
///
/// Atomic sequence: build `view.<epoch>.<seq>.building` → `CHECKPOINT`+close →
/// assert no temp `.wal` → `chmod 0444` → `fsync` → remove dest `.wal` + assert
/// absent → `rename` → `fsync(drawer)`. On WAL/I/O abort, last-good retained.
///
/// Publish mutex is the **caller's** (zero flock here). Returns published path.
///
/// # Errors
/// [`ViewError::EphemeralDrawer`] / [`ViewError::Duckdb`] /
/// [`ViewError::WalPresent`] / [`ViewError::Io`].
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

    // Stale temp from a crashed prior publish must never be reused.
    remove_if_present(&temp)?;
    remove_if_present(&temp_wal)?;

    // 1. Build candidate, CHECKPOINT + close (WAL folds in).
    {
        let conn = Connection::open(&temp)?;
        create_schema(&conn)?;
        project(&conn, docs, &model::RootedCorpus::ambient(docs), None)?;
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

    // 2. B1: surviving temp `.wal` aborts (keep last-good).
    if temp_wal.exists() {
        let _ = std::fs::remove_file(&temp);
        return Err(ViewError::WalPresent(temp_wal));
    }

    // 3. chmod 0444 + fsync candidate (managed readers check mode).
    chmod_readonly(&temp)?;
    fsync_path(&temp)?;

    // 4. Remove dest `.wal` (bare rename would strand it for replay), fsync
    //    drawer, assert absent — abort on failure.
    remove_if_present(&dest_wal)?;
    fsync_dir(dir)?;
    if dest_wal.exists() {
        return Err(ViewError::WalPresent(dest_wal));
    }

    // 5. Atomic same-dir rename + fsync drawer.
    std::fs::rename(&temp, &dest)?;
    fsync_dir(dir)?;
    Ok(dest)
}

/// Project documents into fact tables in FK order. Ordinals are document-order
/// `UBIGINT`s. Links resolve via `CorpusIndex::resolve_linkpath`; unresolved
/// vault ref ⇒ `dest_path = NULL` (first-class dangling).
fn project(
    conn: &Connection,
    docs: &BTreeMap<String, Document>,
    corpus: &model::RootedCorpus<'_>,
    mounts: Option<&addr::MountSet>,
) -> duckdb::Result<()> {
    let index = corpus_index(docs);
    let mut rows = Rows::default();
    for (path, doc) in docs {
        collect_doc(path, doc, &index, &mut rows, corpus, mounts);
    }
    rows.insert(conn)
}

/// Corpus name index (basename + frontmatter-alias) — same stage-1 resolver
/// the engine uses, so link resolution matches.
fn corpus_index(docs: &BTreeMap<String, Document>) -> CorpusIndex {
    let mut index = CorpusIndex::new();
    for (path, doc) in docs {
        index.insert(path, doc);
    }
    index
}

/// Document-order counters per fact table.
#[derive(Default)]
struct Counters {
    section: u64,
    link: u64,
    tag: u64,
    task: u64,
}

/// Projected rows, staged then bulk-inserted in FK order.
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

/// `section` row — `hpath` bound via dynamic `list_value`, apart from scalars.
struct SectionRow {
    scalars_before: Vec<Value>, // path, node_seq
    hpath: Vec<String>,
    scalars_after: Vec<Value>, // heading, level, node_rev, span_start, span_end
}

/// `task` row — `hpath` nullable (NULL when document-level).
struct TaskRow {
    scalars_before: Vec<Value>, // path, seq, checked, depth, section_seq
    hpath: Option<Vec<String>>,
    scalars_after: Vec<Value>, // text, span_start, span_end, node_rev
}

/// Emit `doc` row and walk the node tree.
fn collect_doc(
    path: &str,
    doc: &Document,
    index: &CorpusIndex,
    rows: &mut Rows,
    corpus: &model::RootedCorpus<'_>,
    mounts: Option<&addr::MountSet>,
) {
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
    walk(
        root,
        path,
        doc,
        index,
        None,
        &mut counters,
        rows,
        corpus,
        mounts,
    );
}

/// Walk one node (emit fact rows), recurse with governing `section_seq` for tasks.
#[allow(clippy::too_many_arguments)]
fn walk(
    node: &Node,
    path: &str,
    doc: &Document,
    index: &CorpusIndex,
    gov_section: Option<u64>,
    counters: &mut Counters,
    rows: &mut Rows,
    corpus: &model::RootedCorpus<'_>,
    mounts: Option<&addr::MountSet>,
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
            let dest = resolve_dest(
                index,
                target,
                heading.as_deref(),
                block.as_deref(),
                path,
                corpus,
                mounts,
            );
            emit_link(
                node,
                path,
                "wikilink",
                target,
                heading.as_deref(),
                block.as_deref(),
                alias.as_deref(),
                dest.as_ref(),
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
            let dest = resolve_dest(
                index,
                target,
                heading.as_deref(),
                block.as_deref(),
                path,
                corpus,
                mounts,
            );
            emit_link(
                node,
                path,
                "embed",
                target,
                heading.as_deref(),
                block.as_deref(),
                alias.as_deref(),
                dest.as_ref(),
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
        walk(
            child, path, doc, index, child_gov, counters, rows, corpus, mounts,
        );
    }
}

/// Emit `frontmatter` rows (first-occurrence-wins via `YamlMap`) and B2
/// `frontmatter_tag` for `tag`/`tags`. All share the Frontmatter C1 locator.
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

/// Emit one `section` row — identity `(path, node_seq)`; `hpath` advisory.
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

/// Emit one `link` row. `resolved` is generated, never inserted. Schema CHECKs
/// make "both dests set" and "root without path" unrepresentable.
#[allow(clippy::too_many_arguments)]
fn emit_link(
    node: &Node,
    path: &str,
    kind: &str,
    target_raw: &str,
    heading: Option<&str>,
    block: Option<&str>,
    alias: Option<&str>,
    dest: Option<&Dest>,
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
        // Three columns, one destination (schema CHECK enforces exclusivity).
        match dest {
            Some(Dest::Ambient(p)) => Value::Text(p.clone()),
            Some(Dest::Rooted { .. }) | None => Value::Null,
        },
        match dest {
            Some(Dest::Rooted { root, .. }) => Value::Text(root.clone()),
            _ => Value::Null,
        },
        match dest {
            Some(Dest::Rooted { path, .. }) => Value::Text(path.clone()),
            _ => Value::Null,
        },
        Value::UBigInt(u64c(node.span.start)),
        Value::UBigInt(u64c(node.span.end)),
        Value::Text(node.node_rev.0.clone()),
    ]);
}

/// Emit one inline `tag` row (`Tag.name`, no leading `#`).
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

/// Emit one `task` row. `section_seq` NULL = document-level; `text` is trimmed.
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

/// Resolve wikilink/embed to structural dest: `Ambient` / `Rooted` / `None`
/// (dangling). Self-ref `[[#H]]` / `[[#^blk]]` → source doc.
///
/// Split mirrors the link plane (`addr::head_carries_root_separator`). No mount
/// authority ⇒ rooted spelling stays dangling (not having looked ≠ finding).
fn resolve_dest(
    index: &CorpusIndex,
    target: &str,
    heading: Option<&str>,
    block: Option<&str>,
    src_path: &str,
    corpus: &model::RootedCorpus<'_>,
    mounts: Option<&addr::MountSet>,
) -> Option<Dest> {
    if target.trim().is_empty() {
        return (heading.is_some() || block.is_some()).then(|| Dest::Ambient(src_path.to_string()));
    }
    let Some(mounts) = mounts.filter(|_| addr::head_carries_root_separator(target)) else {
        return index.resolve_linkpath(target, src_path).map(Dest::Ambient);
    };
    match index.resolve_ref(target, src_path, corpus, mounts) {
        model::RefResolution::Ambient(path) => Some(Dest::Ambient(path)),
        model::RefResolution::Rooted { root, path } => Some(Dest::Rooted {
            root: root.to_string(),
            path,
        }),
        // Refusals project as dangling: this is a fact table, not a refusal surface.
        _ => None,
    }
}

/// Link destination — two facts when cross-root, never a joined `root:path` (U21 Q5).
enum Dest {
    Ambient(String),
    Rooted { root: String, path: String },
}

/// B2 scalar-parse of `tag`/`tags`: strip `[ ]`, split on `,`, trim/unquote/
/// strip `#`, drop empties. Empty / block-list ⇒ **0 rows** (fail-closed).
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
    /// Bulk-insert staged rows in FK order.
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
            "INSERT INTO link (src_path, seq, kind, target_raw, heading, block, alias, dest_path, dest_root, dest_root_path, span_start, span_end, node_rev) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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

/// `list_value(?, …)` with `n` placeholders, or empty `VARCHAR[]` when `n == 0`.
fn list_expr(n: usize) -> String {
    if n == 0 {
        return "[]::VARCHAR[]".to_string();
    }
    let marks = std::iter::repeat_n("?", n).collect::<Vec<_>>().join(", ");
    format!("list_value({marks})")
}

/// Scalar-before + list elements + scalar-after for a `list_value` INSERT.
fn chain_list(before: &[Value], list: &[String], after: &[Value]) -> Vec<Value> {
    let mut params = before.to_vec();
    params.extend(list.iter().map(|s| Value::Text(s.clone())));
    params.extend(after.iter().cloned());
    params
}

/// Prepare `sql` once; execute per staged row.
fn insert_rows(conn: &Connection, sql: &str, rows: &[Vec<Value>]) -> duckdb::Result<()> {
    let mut stmt = conn.prepare(sql)?;
    for row in rows {
        stmt.execute(duckdb::params_from_iter(row.iter()))?;
    }
    Ok(())
}

/// Insert singleton `_meridian_view` stamp. epoch/seq both set or both NULL
/// (DDL CHECK enforces pairing).
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

/// `<path>.wal` sidecar path `DuckDB` would write beside `path`.
fn wal_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".wal");
    PathBuf::from(s)
}

/// Remove `path` if present; missing is ok.
fn remove_if_present(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// `chmod 0444` — managed readers verify this mode.
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

fn fsync_path(path: &Path) -> std::io::Result<()> {
    std::fs::File::open(path)?.sync_all()
}

/// `fsync` a directory so create/rename/unlink within it is durable.
fn fsync_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::File::open(dir)?.sync_all()
}

/// Current unix seconds (`0` before epoch; never panics).
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `usize` → `u64`, saturating.
fn u64c(x: usize) -> u64 {
    u64::try_from(x).unwrap_or(u64::MAX)
}

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
        let conn = build_memory(&docs, "b3:empty").expect("build");
        let n: i64 = conn
            .query_row("SELECT doc_count FROM _meridian_view", [], |r| r.get(0))
            .expect("stamp");
        assert_eq!(n, 0);
    }

    /// Stamp is the caller's fold verbatim — not a refold (G14 / §12.3).
    #[test]
    fn build_memory_stamps_the_callers_fold_verbatim() {
        let mut docs = BTreeMap::new();
        docs.insert(
            "a.md".to_owned(),
            model::build("# A\n".to_owned(), syntax::parse("# A\n")),
        );
        // Version-2 fold (`b3b:`), as `fs::domain_snapshot` would hand over.
        let files = [("a.md", "# A\n".as_bytes())];
        let as_of = model::merkle_root(&files, 2).0;
        assert!(as_of.starts_with("b3b:"), "fixture is a version-2 fold");

        let conn = build_memory(&docs, &as_of).expect("build");
        let stamped: String = conn
            .query_row("SELECT as_of_fingerprint FROM _meridian_view", [], |r| {
                r.get(0)
            })
            .expect("stamp");
        assert_eq!(stamped, as_of, "the stamp is the fold it was handed");
    }
}
