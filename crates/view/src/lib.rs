//! `DuckDB` ephemeral projection + lock-aware read face (`wire-contract.md`
//! §10.3–§10.4; not agent core).
//!
//! # Charter
//! **Owns:** the `:memory:` SQL projection of the fact corpus — walk
//! `BTreeMap<path, Document>` into 8 tables + 4 views ([`schema`]) — and the
//! lock-aware read face for walk/status colour ([`walk`], [`read_face`]).
//! The projection is advisory; disk and fingerprints are correctness.
//!
//! **Never does:** re-parse, or read its own output into a fact path
//! (view-never-store; C2 topology gate). [`build_memory`] is `:memory:`-only.
//! The persistent published-file organ (`publish`, `view.duckdb`, the
//! `view_path` wire op) was DROPPED by ruling (§10.4, 2026-08-06); the ONE
//! disk write this crate performs is [`store`]'s fingerprint-pinned
//! append-only `sql.duckdb` cache — an operator surface over its own build,
//! never a wire-served file path (sql lifecycle-B ruling, 2026-08-14, which
//! knowingly supersedes §10.4 for sql; session design
//! `results/sql-duckdb-append-cache-design.md` § Ruling interaction).

pub mod facts;
pub mod read_face;
pub mod schema;
pub mod store;
pub mod walk;

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use duckdb::Connection;
use duckdb::types::Value;
use model::{CorpusIndex, Document, Node, NodeKind};

/// The caller-injected exclusion probe: given a link target, the word naming why
/// the hash domain excludes it, or `None` when it does not.
///
/// Named because the signature appears at three call depths and the concept —
/// *"ask the caller, who has the root and the disk, a question this crate cannot
/// answer"* — is what a reader needs at each of them.
pub type ExclusionProbe<'a> = &'a dyn Fn(&str) -> Option<String>;

pub use read_face::{READ_FACE_SCHEMA_SQL, create_read_face_schema, open_board, stale_paths};
pub use schema::{SCHEMA_SQL, SCHEMA_VERSION, create_schema};

/// View-projection failure.
#[derive(Debug)]
pub enum ViewError {
    /// `DuckDB` error from schema or projection.
    Duckdb(duckdb::Error),
    /// Filesystem error from the [`store`] cache file's housekeeping.
    Io(std::io::Error),
}

impl std::fmt::Display for ViewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ViewError::Duckdb(e) => write!(f, "duckdb: {e}"),
            ViewError::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for ViewError {}

impl From<duckdb::Error> for ViewError {
    fn from(e: duckdb::Error) -> Self {
        ViewError::Duckdb(e)
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
    project(&conn, docs, &model::RootedCorpus::ambient(docs), None, None)?;
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
    exclusion: Option<ExclusionProbe<'_>>,
) -> Result<Connection, ViewError> {
    let conn = Connection::open_in_memory()?;
    create_schema(&conn)?;
    project(&conn, docs, corpus, Some(mounts), exclusion)?;
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

/// Project documents into fact tables in FK order. Ordinals are document-order
/// `UBIGINT`s. Links resolve via `CorpusIndex::resolve_linkpath`; unresolved
/// vault ref ⇒ `dest_path = NULL` (first-class dangling).
fn project(
    conn: &Connection,
    docs: &BTreeMap<String, Document>,
    corpus: &model::RootedCorpus<'_>,
    mounts: Option<&addr::MountSet>,
    exclusion: Option<ExclusionProbe<'_>>,
) -> duckdb::Result<()> {
    let index = corpus_index(docs);
    let mut rows = Rows::default();
    for (path, doc) in docs {
        collect_doc(path, doc, &index, &mut rows, corpus, mounts);
    }
    fill_exclusions(&mut rows, exclusion);
    rows.insert(conn)
}

/// Fill `link.exclusion` for the DANGLING rows whose target is a real file the
/// hash domain does not carry (session decision 0034).
///
/// CALLER-INJECTED, the way `policy::change` injects its one-hop resolver, and
/// for the same reason: the answer needs the workspace root and a disk probe,
/// and this crate writes nothing and reads nothing from disk. `None` means the
/// caller had no root to answer with — every row then keeps the NULL it was
/// emitted with, which is the honest value for "not asked", not a claim that
/// nothing is excluded.
///
/// Only rows with NO destination are considered: a resolved edge is in the
/// domain by construction, so asking about it would be asking a question whose
/// answer cannot be true.
pub(crate) fn fill_exclusions(rows: &mut Rows, exclusion: Option<ExclusionProbe<'_>>) {
    let Some(why) = exclusion else { return };
    for row in &mut rows.link {
        let dangling = matches!(row[LINK_COL_DEST_PATH], Value::Null)
            && matches!(row[LINK_COL_DEST_ROOT], Value::Null);
        if !dangling {
            continue;
        }
        let Value::Text(target) = &row[LINK_COL_TARGET_RAW] else {
            continue;
        };
        if let Some(word) = why(target) {
            row[LINK_COL_EXCLUSION] = Value::Text(word);
        }
    }
}

/// Corpus name index (basename + frontmatter-alias) — same stage-1 resolver
/// the engine uses, so link resolution matches.
pub(crate) fn corpus_index(docs: &BTreeMap<String, Document>) -> CorpusIndex {
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

/// Projected rows, staged then bulk-inserted in FK order. Shared with the
/// [`store`] append lane, which loads the same staging into hist tables.
#[derive(Default)]
pub(crate) struct Rows {
    pub(crate) doc: Vec<Vec<Value>>,
    pub(crate) frontmatter: Vec<Vec<Value>>,
    pub(crate) section: Vec<SectionRow>,
    pub(crate) link: Vec<Vec<Value>>,
    pub(crate) tag: Vec<Vec<Value>>,
    pub(crate) frontmatter_tag: Vec<Vec<Value>>,
    pub(crate) task: Vec<TaskRow>,
}

/// `section` row — `hpath` bound via dynamic `list_value`, apart from scalars.
pub(crate) struct SectionRow {
    pub(crate) scalars_before: Vec<Value>, // path, node_seq
    pub(crate) hpath: Vec<String>,
    pub(crate) scalars_after: Vec<Value>, // heading, level, node_rev, span_start, span_end
}

/// `task` row — `hpath` nullable (NULL when document-level).
pub(crate) struct TaskRow {
    pub(crate) scalars_before: Vec<Value>, // path, seq, checked, depth, section_seq
    pub(crate) hpath: Option<Vec<String>>,
    pub(crate) scalars_after: Vec<Value>, // text, span_start, span_end, node_rev
}

/// Emit `doc` row and walk the node tree.
pub(crate) fn collect_doc(
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
        NodeKind::Frontmatter { map } => emit_frontmatter(node, path, doc, map, rows),
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
/// The `value` column is a published value plane: the stored scalar decodes
/// through § A.6.1 (`model::scalar`); the locator/rev columns stay
/// raw-computed (§ A.6.2).
///
/// `prop_rev` is the per-key CAS token (`node-rev-merkle-spec.md` §2.1), taken
/// off `model::resolve` — the same owner the write door compares `if_node_rev`
/// against and the read face serves as `props[].prop_rev`. It is READ here,
/// never recomputed: a second derivation of one hash drifts silently, since
/// both spellings produce 16 plausible hex characters.
fn emit_frontmatter(
    node: &Node,
    path: &str,
    doc: &Document,
    map: &model::YamlMap,
    rows: &mut Rows,
) {
    let (span_start, span_end) = (u64c(node.span.start), u64c(node.span.end));
    let node_rev = node.node_rev.0.clone();
    for (ord, (key, value)) in map.0.iter().enumerate() {
        // A map key resolves by construction — `model::parse_frontmatter` and
        // the `fm_key` resolver scan the same block with the same column-0,
        // first-colon, quote-trimmed key rule, first occurrence wins. A miss is
        // an engine bug; a fallback to the block `node_rev` would serve a token
        // that silently refuses every guarded write, which is the defect this
        // column exists to remove.
        let prop_rev = model::resolve(doc, &model::Ref::FmKey(key.clone()))
            .expect("frontmatter map key resolves against its own document")
            .node_rev
            .0;
        let value = model::scalar::text(value);
        rows.frontmatter.push(vec![
            Value::Text(path.to_string()),
            Value::UBigInt(u64c(ord)),
            Value::Text(key.clone()),
            Value::Text(value.clone()),
            Value::UBigInt(span_start),
            Value::UBigInt(span_end),
            Value::Text(node_rev.clone()),
            Value::Text(prop_rev),
        ]);
        if key == "tag" || key == "tags" {
            for (seq, tag) in parse_fm_tags(&value).into_iter().enumerate() {
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
        // `exclusion` — filled by [`project`] after collection, because the
        // answer needs the workspace root and this crate holds none.
        Value::Null,
        Value::UBigInt(u64c(node.span.start)),
        Value::UBigInt(u64c(node.span.end)),
        Value::Text(node.node_rev.0.clone()),
    ]);
}

/// Column index of `exclusion` in a `link` row, and of the two destination
/// columns that decide whether a row is dangling. Named once so the row shape
/// and the fill cannot drift apart silently.
const LINK_COL_TARGET_RAW: usize = 3;
const LINK_COL_DEST_PATH: usize = 7;
const LINK_COL_DEST_ROOT: usize = 8;
const LINK_COL_EXCLUSION: usize = 10;

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
    /// Bulk-insert staged rows in FK order through the `DuckDB` Appender API.
    ///
    /// The row-at-a-time statement lane this replaced executed one INSERT per
    /// staged row (134k executes on a 6.7k-doc corpus — ~98% of the sql build
    /// wall); the appender loads the same rows ~500x faster. The statement
    /// lane survives as the equivalence reference in
    /// `tests::statement_lane_insert`, gated bit-identical per table.
    fn insert(&self, conn: &Connection) -> duckdb::Result<()> {
        append_rows(conn, "doc", &self.doc)?;
        append_rows(conn, "frontmatter", &self.frontmatter)?;
        self.append_section(conn)?;
        append_rows(conn, "link", &self.link)?;
        append_rows(conn, "tag", &self.tag)?;
        append_rows(conn, "frontmatter_tag", &self.frontmatter_tag)?;
        self.append_task(conn)
    }

    /// `section` — the `hpath` list column rides the stage-table workaround
    /// ([`hpath_join`]).
    fn append_section(&self, conn: &Connection) -> duckdb::Result<()> {
        conn.execute_batch(
            "CREATE TABLE _stage_section (path TEXT, node_seq UBIGINT, hpath_j TEXT, heading TEXT, level UTINYINT, node_rev TEXT, span_start UBIGINT, span_end UBIGINT);",
        )?;
        {
            let mut app = conn.appender("_stage_section")?;
            for row in &self.section {
                let mut params = row.scalars_before.clone();
                params.push(hpath_join(&row.hpath));
                params.extend(row.scalars_after.iter().cloned());
                app.append_row(duckdb::appender_params_from_iter(params.iter()))?;
            }
            app.flush()?;
        }
        conn.execute_batch(
            "INSERT INTO section SELECT path, node_seq, CASE WHEN hpath_j = '' THEN []::TEXT[] ELSE string_split(hpath_j, chr(31))[2:] END, heading, level, node_rev, span_start, span_end FROM _stage_section; \
             DROP TABLE _stage_section;",
        )
    }

    /// `task` — as `section`, with NULL `hpath` (document-level task) kept NULL.
    fn append_task(&self, conn: &Connection) -> duckdb::Result<()> {
        conn.execute_batch(
            "CREATE TABLE _stage_task (path TEXT, seq UBIGINT, checked BOOLEAN, depth UINTEGER, section_seq UBIGINT, hpath_j TEXT, text TEXT, span_start UBIGINT, span_end UBIGINT, node_rev TEXT);",
        )?;
        {
            let mut app = conn.appender("_stage_task")?;
            for row in &self.task {
                let mut params = row.scalars_before.clone();
                params.push(row.hpath.as_deref().map_or(Value::Null, hpath_join));
                params.extend(row.scalars_after.iter().cloned());
                app.append_row(duckdb::appender_params_from_iter(params.iter()))?;
            }
            app.flush()?;
        }
        conn.execute_batch(
            "INSERT INTO task SELECT path, seq, checked, depth, section_seq, CASE WHEN hpath_j IS NULL THEN NULL WHEN hpath_j = '' THEN []::TEXT[] ELSE string_split(hpath_j, chr(31))[2:] END, text, span_start, span_end, node_rev FROM _stage_task; \
             DROP TABLE _stage_task;",
        )
    }
}

/// Appender load of one all-scalar table. Value order = DDL column order.
fn append_rows(conn: &Connection, table: &str, rows: &[Vec<Value>]) -> duckdb::Result<()> {
    let mut app = conn.appender(table)?;
    for row in rows {
        app.append_row(duckdb::appender_params_from_iter(row.iter()))?;
    }
    app.flush()
}

/// TEXT encoding of an `hpath` list for the stage tables. The `TEXT[]`
/// columns cannot go through the appender — duckdb-rs does not support
/// binding List values in `append_row` yet ("binding List parameters is not
/// yet supported"; retire the stage tables when upstream adds it) — so the
/// list rides an appender-loaded stage TEXT column, decoded set-based by one
/// `INSERT..SELECT string_split(...)[2:]`.
///
/// Every element is PREFIXED with the unit separator (chr(31)), not joined by
/// it: a plain join encodes both `[]` and `['']` as `''`, and a real `['']`
/// hpath exists in live corpora. Prefixed, `[]` → `''` and `['']` → `"\u{1f}"`
/// stay distinct; the decode drops the leading empty split element via `[2:]`.
pub(crate) fn hpath_join(hpath: &[String]) -> Value {
    let mut joined = String::new();
    for element in hpath {
        joined.push('\u{1f}');
        joined.push_str(element);
    }
    Value::Text(joined)
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

    // ---- Appender-lane equivalence gate ----------------------------------
    //
    // `Rows::insert` loads through the DuckDB Appender; this gate holds it
    // bit-identical, per table over every column of every row, to the
    // row-at-a-time statement lane it replaced. The hazard the gate exists
    // for: the stage-table TEXT encoding of `hpath` must keep `[]` and `['']`
    // distinct — a real `['']` hpath exists in live corpora.

    /// The retired row-at-a-time statement lane, kept verbatim as the
    /// equivalence reference the appender lane is gated against.
    fn statement_lane_insert(rows: &Rows, conn: &Connection) -> duckdb::Result<()> {
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
        insert_rows(
            conn,
            "INSERT INTO doc (path, file_rev, line_count, bytes) VALUES (?, ?, ?, ?)",
            &rows.doc,
        )?;
        insert_rows(
            conn,
            "INSERT INTO frontmatter (path, ord, key, value, span_start, span_end, node_rev, prop_rev) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            &rows.frontmatter,
        )?;
        for row in &rows.section {
            let hpath_expr = list_expr(row.hpath.len());
            let sql = format!(
                "INSERT INTO section (path, node_seq, hpath, heading, level, node_rev, span_start, span_end) VALUES (?, ?, {hpath_expr}, ?, ?, ?, ?, ?)"
            );
            let params = chain_list(&row.scalars_before, &row.hpath, &row.scalars_after);
            conn.execute(&sql, duckdb::params_from_iter(params.iter()))?;
        }
        insert_rows(
            conn,
            "INSERT INTO link (src_path, seq, kind, target_raw, heading, block, alias, dest_path, dest_root, dest_root_path, exclusion, span_start, span_end, node_rev) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            &rows.link,
        )?;
        insert_rows(
            conn,
            "INSERT INTO tag (path, seq, tag, span_start, span_end, node_rev) VALUES (?, ?, ?, ?, ?, ?)",
            &rows.tag,
        )?;
        insert_rows(
            conn,
            "INSERT INTO frontmatter_tag (path, seq, tag, key, span_start, span_end, node_rev) VALUES (?, ?, ?, ?, ?, ?, ?)",
            &rows.frontmatter_tag,
        )?;
        for row in &rows.task {
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

    /// Per-table digest over every column of every row (`resolved` is
    /// generated, never stored, so it is derived equal by construction). The
    /// section/task hpath term is length-prefixed (`len#join`) so `[]` and
    /// `['']` digest differently.
    const LANE_DIGESTS: [(&str, &str); 7] = [
        (
            "doc",
            "SELECT coalesce(md5(string_agg(path || '|' || file_rev || '|' || line_count::VARCHAR || '|' || bytes::VARCHAR, chr(10) ORDER BY path)), 'EMPTY') FROM doc",
        ),
        (
            "frontmatter",
            "SELECT coalesce(md5(string_agg(path || '|' || ord::VARCHAR || '|' || key || '|' || value || '|' || span_start::VARCHAR || '|' || span_end::VARCHAR || '|' || node_rev || '|' || prop_rev, chr(10) ORDER BY path, key)), 'EMPTY') FROM frontmatter",
        ),
        (
            "section",
            "SELECT coalesce(md5(string_agg(path || '|' || node_seq::VARCHAR || '|' || len(hpath)::VARCHAR || '#' || array_to_string(hpath, chr(31)) || '|' || heading || '|' || level::VARCHAR || '|' || node_rev || '|' || span_start::VARCHAR || '|' || span_end::VARCHAR, chr(10) ORDER BY path, node_seq)), 'EMPTY') FROM section",
        ),
        (
            "link",
            "SELECT coalesce(md5(string_agg(src_path || '|' || seq::VARCHAR || '|' || kind || '|' || target_raw || '|' || coalesce(heading,'~N~') || '|' || coalesce(block,'~N~') || '|' || coalesce(alias,'~N~') || '|' || coalesce(dest_path,'~N~') || '|' || coalesce(dest_root,'~N~') || '|' || coalesce(dest_root_path,'~N~') || '|' || coalesce(exclusion,'~N~') || '|' || span_start::VARCHAR || '|' || span_end::VARCHAR || '|' || node_rev, chr(10) ORDER BY src_path, seq)), 'EMPTY') FROM link",
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
    ];

    /// Load `rows` into a fresh schema through `lane`; return the 7 digests.
    fn lane_digests(
        rows: &Rows,
        lane: impl Fn(&Rows, &Connection) -> duckdb::Result<()>,
    ) -> Vec<(&'static str, String)> {
        let conn = Connection::open_in_memory().expect("open");
        create_schema(&conn).expect("schema");
        lane(rows, &conn).expect("lane insert");
        LANE_DIGESTS
            .iter()
            .map(|(table, sql)| {
                let d: String = conn.query_row(sql, [], |r| r.get(0)).expect("digest");
                (*table, d)
            })
            .collect()
    }

    fn assert_lanes_match(rows: &Rows) {
        assert_eq!(
            lane_digests(rows, Rows::insert),
            lane_digests(rows, statement_lane_insert),
            "appender lane must project bit-identically to the statement lane"
        );
    }

    /// Stage `docs` the way [`project`] does (ambient corpus, no mounts).
    fn stage(docs: &BTreeMap<String, Document>) -> Rows {
        let index = corpus_index(docs);
        let mut rows = Rows::default();
        let corpus = model::RootedCorpus::ambient(docs);
        for (path, doc) in docs {
            collect_doc(path, doc, &index, &mut rows, &corpus, None);
        }
        rows
    }

    /// The encoding edges, hand-staged so the gate cannot silently lose them
    /// to parser drift: section hpath `[]` vs `['']` vs a list holding an
    /// empty element; task hpath NULL vs `[]` vs `['']`.
    #[test]
    fn appender_lane_encoding_edges_match_statement_lane() {
        let mut rows = Rows::default();
        rows.doc.push(vec![
            Value::Text("d.md".to_string()),
            Value::Text("rev0".to_string()),
            Value::UInt(1),
            Value::UBigInt(2),
        ]);
        let shapes = [
            vec![],
            vec![String::new()],
            vec!["a".to_string(), String::new(), "b".to_string()],
        ];
        for (seq, hpath) in shapes.into_iter().enumerate() {
            rows.section.push(SectionRow {
                scalars_before: vec![Value::Text("d.md".to_string()), Value::UBigInt(seq as u64)],
                hpath,
                scalars_after: vec![
                    Value::Text("h".to_string()),
                    Value::UTinyInt(1),
                    Value::Text("rev".to_string()),
                    Value::UBigInt(0),
                    Value::UBigInt(1),
                ],
            });
        }
        rows.frontmatter.push(vec![
            Value::Text("d.md".to_string()),
            Value::UBigInt(0),
            Value::Text("tags".to_string()),
            Value::Text("[x]".to_string()),
            Value::UBigInt(0),
            Value::UBigInt(1),
            Value::Text("rev".to_string()),
            Value::Text("prev".to_string()),
        ]);
        rows.frontmatter_tag.push(vec![
            Value::Text("d.md".to_string()),
            Value::UBigInt(0),
            Value::Text("x".to_string()),
            Value::Text("tags".to_string()),
            Value::UBigInt(0),
            Value::UBigInt(1),
            Value::Text("rev".to_string()),
        ]);
        rows.tag.push(vec![
            Value::Text("d.md".to_string()),
            Value::UBigInt(0),
            Value::Text("x".to_string()),
            Value::UBigInt(0),
            Value::UBigInt(1),
            Value::Text("rev".to_string()),
        ]);
        rows.link.push(vec![
            Value::Text("d.md".to_string()),
            Value::UBigInt(0),
            Value::Text("wikilink".to_string()),
            Value::Text("d".to_string()),
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Text("d.md".to_string()),
            Value::Null,
            Value::Null,
            Value::Null,
            Value::UBigInt(0),
            Value::UBigInt(1),
            Value::Text("rev".to_string()),
        ]);
        let task_shapes = [
            (Value::Null, None),
            (Value::UBigInt(0), Some(vec![])),
            (Value::UBigInt(1), Some(vec![String::new()])),
        ];
        for (seq, (section_seq, hpath)) in task_shapes.into_iter().enumerate() {
            rows.task.push(TaskRow {
                scalars_before: vec![
                    Value::Text("d.md".to_string()),
                    Value::UBigInt(seq as u64),
                    Value::Boolean(false),
                    Value::UInt(0),
                    section_seq,
                ],
                hpath,
                scalars_after: vec![
                    Value::Text("t".to_string()),
                    Value::UBigInt(0),
                    Value::UBigInt(1),
                    Value::Text("rev".to_string()),
                ],
            });
        }
        assert_lanes_match(&rows);
    }

    /// Full-walk equivalence on a parsed fixture corpus that populates all 7
    /// tables and carries the live-corpus `['']` hazard (a bare `#` heading).
    #[test]
    fn appender_lane_matches_statement_lane_on_parsed_corpus() {
        let a = "---\ntitle: Alpha\ntags: [x/one, x/two]\n---\n- [ ] doc-level task\n\n# Top\nintro #inline\n## Sub\n- [x] task under Sub\n\nsee [[b]] and [[missing]] and ![[b]]\n";
        let b = "#\n- [ ] task under the empty heading\n## Sub\nbody\n";
        let mut docs = BTreeMap::new();
        docs.insert(
            "a.md".to_string(),
            model::build(a.to_string(), syntax::parse(a)),
        );
        docs.insert(
            "b.md".to_string(),
            model::build(b.to_string(), syntax::parse(b)),
        );
        let rows = stage(&docs);

        // The fixture must be non-degenerate: every parser-reachable table
        // populated, and the hazard shapes actually staged. `tag` stays empty
        // — the parser emits no inline `NodeKind::Tag` nodes today (the live
        // corpus stages tag=0 too); its lane is covered by the hand-staged
        // gate above.
        assert!(!rows.doc.is_empty(), "doc rows");
        assert!(!rows.frontmatter.is_empty(), "frontmatter rows");
        assert!(!rows.section.is_empty(), "section rows");
        assert!(!rows.link.is_empty(), "link rows");
        assert!(!rows.frontmatter_tag.is_empty(), "frontmatter_tag rows");
        assert!(!rows.task.is_empty(), "task rows");
        assert!(
            rows.section.iter().any(|s| s.hpath == vec![String::new()]),
            "a bare `#` heading must stage hpath [''] — the encoding hazard"
        );
        assert!(
            rows.task
                .iter()
                .any(|t| t.hpath.as_deref() == Some(&[String::new()][..])),
            "the task under the empty heading must stage hpath ['']"
        );
        assert!(
            rows.task.iter().any(|t| t.hpath.is_none()),
            "the doc-level task must stage hpath NULL"
        );

        assert_lanes_match(&rows);
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
