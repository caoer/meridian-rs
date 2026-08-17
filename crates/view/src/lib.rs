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

mod base;
/// The pinned `DuckDB`, re-exported so a consumer converting result cells
/// (registry `mw_sql`) speaks THIS crate's engine version, never a second pin.
pub use duckdb;

pub mod facts;
pub mod read_face;
pub mod schema;
mod sqltext;
pub mod store;
pub mod walk;

use std::collections::{BTreeMap, HashMap};
use std::time::{SystemTime, UNIX_EPOCH};

use duckdb::Connection;
use duckdb::types::Value;
use model::{CorpusIndex, Document, Node, NodeKind};

/// The caller-injected exclusion probe: given a link target, the file it
/// resolved to and the word naming why the hash domain excludes it, or `None`
/// when it does not.
///
/// Named because the signature appears at three call depths and the concept —
/// *"ask the caller, who has the root and the disk, a question this crate cannot
/// answer"* — is what a reader needs at each of them.
///
/// It answers `(path, word)` rather than the word alone because the probe
/// computes both and the projection used to discard the path
/// (`base-projection.md` §5.1). The path is the join key `link.exclusion_path`
/// carries; the word is unchanged.
pub type ExclusionProbe<'a> = &'a dyn Fn(&str) -> Option<(String, String)>;

/// One `.base` member as a caller's walk found it — the shape [`BaseWalk`]
/// carries, and the projection's whole input for the base relations.
///
/// It is caller-injected for the same reason [`ExclusionProbe`] is: membership
/// is a directory-enumeration question over the workspace root, and this crate
/// reads nothing from disk. `fs::base::base_snapshot` is the walk that answers
/// it (`base-projection.md` §3).
pub struct BaseMember {
    /// Workspace-relative ON-DISK spelling.
    pub path: String,
    /// The member's raw bytes, or the message of whatever refused to read them
    /// (§4.4 — a member the walk SAW is a named row, never an absence).
    pub bytes: Result<Vec<u8>, String>,
}

/// A `.base` walk handed to a build: the members and the `bf:` witness the
/// walker folded over them (`base-projection.md` §6.2).
///
/// **Absent is not empty.** A build handed no walk stamps `base_fold` NULL —
/// "not asked" — while a walk that found nothing stamps the fold of the empty
/// sequence. §12.1's absence rule forecloses collapsing the two.
pub struct BaseWalk<'a> {
    /// The members, in path byte order.
    pub members: &'a [BaseMember],
    /// The `bf:` fold the WALKER computed, stamped verbatim. This crate folds
    /// nothing itself — the same discipline `as_of` is under (G14).
    pub fold: &'a str,
}

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
    project(
        &conn,
        docs,
        &model::RootedCorpus::ambient(docs),
        None,
        None,
        None,
    )?;
    write_stamp(
        &conn,
        as_of,
        "",
        None,
        None,
        "view::build_memory",
        docs.len(),
        None,
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
/// `base` is the same shape of claim: `None` stamps `base_fold` NULL — the
/// base plane was NOT WALKED — while a walk that found nothing stamps the fold
/// of the empty sequence (`base-projection.md` §6.2/§6.3).
///
/// # Errors
/// As [`build_memory`].
pub fn build_memory_rooted(
    docs: &BTreeMap<String, Document>,
    corpus: &model::RootedCorpus<'_>,
    mounts: &addr::MountSet,
    as_of: &str,
    exclusion: Option<ExclusionProbe<'_>>,
    base: Option<&BaseWalk<'_>>,
) -> Result<Connection, ViewError> {
    let conn = Connection::open_in_memory()?;
    create_schema(&conn)?;
    project(&conn, docs, corpus, Some(mounts), exclusion, base)?;
    write_stamp(
        &conn,
        as_of,
        "",
        None,
        None,
        "view::build_memory_rooted",
        docs.len(),
        base.map(|b| b.fold),
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
    base: Option<&BaseWalk<'_>>,
) -> duckdb::Result<()> {
    let index = corpus_index(docs);
    let mut rows = Rows::default();
    for (path, doc) in docs {
        collect_doc(path, doc, &index, &mut rows, corpus, mounts);
    }
    fill_exclusions(&mut rows, exclusion);
    if let Some(base) = base {
        collect_base(base.members, &mut rows);
    }
    rows.insert(conn)
}

/// Stage the three `.base` relations from a caller's walk
/// (`base-projection.md` §4). One `base` row per member; children only for a
/// member that parsed (§4.4: an error row has ZERO children).
pub(crate) fn collect_base(members: &[BaseMember], rows: &mut Rows) {
    for member in members {
        let (parsed, file_rev, bytes) = match &member.bytes {
            Ok(raw) => (
                base::parse(raw),
                // The merkle-spec §4 leaf truncated to 16 hex — the SAME shape
                // as `doc.file_rev`, so operators compare like with like. It
                // participates in no interior, no fingerprint, no receipt
                // (§6.1).
                Value::Text(hex16(&model::leaf_digest(raw))),
                Value::UBigInt(u64c(raw.len())),
            ),
            Err(message) => (base::unreadable(message), Value::Null, Value::Null),
        };
        rows.base.push(vec![
            Value::Text(member.path.clone()),
            file_rev,
            bytes,
            opt_text(parsed.error.as_deref()),
            opt_text(parsed.filters.as_deref()),
            opt_text(parsed.properties.as_deref()),
            opt_text(parsed.extra.as_deref()),
        ]);
        for (ord, view) in parsed.views.iter().enumerate() {
            rows.base_view.push(vec![
                Value::Text(member.path.clone()),
                Value::UBigInt(u64c(ord)),
                opt_text(view.name.as_deref()),
                opt_text(view.type_.as_deref()),
                opt_text(view.filters.as_deref()),
                opt_text(view.config.as_deref()),
            ]);
        }
        for (ord, formula) in parsed.formulas.iter().enumerate() {
            rows.base_formula.push(vec![
                Value::Text(member.path.clone()),
                Value::UBigInt(u64c(ord)),
                Value::Text(formula.name.clone()),
                Value::Text(formula.expr.clone()),
            ]);
        }
    }
}

/// A 32-byte digest as the 16-hex-character `file_rev` the corpus already
/// speaks (`node-rev-merkle-spec.md` §4 leaf, truncated).
fn hex16(digest: &[u8; 32]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(16);
    for byte in &digest[..8] {
        let _ = write!(out, "{byte:02x}");
    }
    out
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
        // The probe answers both facts at once; the projection stops
        // truncating it to the word (§5.1). The DDL CHECK holds them paired.
        if let Some((path, word)) = why(target) {
            row[LINK_COL_EXCLUSION] = Value::Text(word);
            row[LINK_COL_EXCLUSION_PATH] = Value::Text(path);
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
    body: u64,
}

/// Projected rows, staged then bulk-inserted in FK order. Shared with the
/// [`store`] append lane, which loads the same staging into hist tables.
#[derive(Default)]
pub(crate) struct Rows {
    pub(crate) doc: Vec<Vec<Value>>,
    pub(crate) frontmatter: Vec<Vec<Value>>,
    pub(crate) section: Vec<Vec<Value>>,
    pub(crate) link: Vec<Vec<Value>>,
    pub(crate) tag: Vec<Vec<Value>>,
    pub(crate) frontmatter_tag: Vec<Vec<Value>>,
    pub(crate) task: Vec<Vec<Value>>,
    pub(crate) base: Vec<Vec<Value>>,
    pub(crate) base_view: Vec<Vec<Value>>,
    pub(crate) base_formula: Vec<Vec<Value>>,
    pub(crate) body: Vec<Vec<Value>>,
}

/// One segment of a section's published machine address: raw heading text plus
/// the 1-based occurrence among same-parent siblings sharing that text —
/// present only where the text is ambiguous. The occurrence law is
/// `model::resolve_hpath_node`'s (position among same-raw-text Section children
/// of one parent, child order), the same law the read face's published
/// addresses and the policy rebuild index count by.
#[derive(Clone)]
pub(crate) struct AddrSeg {
    pub(crate) h: String,
    pub(crate) n: Option<u32>,
}

/// Occurrence-address the Section children of one node: `Some(seg)` for a
/// Section child (its `n` set only where its raw text repeats among the
/// siblings), `None` for every other child — the counting pass behind
/// [`AddrSeg`]'s law.
fn child_segs(node: &Node) -> Vec<Option<AddrSeg>> {
    let mut totals: HashMap<&str, u32> = HashMap::new();
    for child in &node.children {
        if let NodeKind::Section { heading_text, .. } = &child.kind {
            *totals.entry(heading_text.as_str()).or_insert(0) += 1;
        }
    }
    let mut seen: HashMap<&str, u32> = HashMap::new();
    node.children
        .iter()
        .map(|child| {
            let NodeKind::Section { heading_text, .. } = &child.kind else {
                return None;
            };
            let occ = seen.entry(heading_text.as_str()).or_insert(0);
            *occ += 1;
            let ambiguous = totals.get(heading_text.as_str()).is_some_and(|&t| t > 1);
            Some(AddrSeg {
                h: heading_text.clone(),
                n: ambiguous.then_some(*occ),
            })
        })
        .collect()
}

/// Render an address chain as the exact `[{"h":…},…]` machine address the read
/// face publishes per toc row and read/put accept verbatim (card
/// sql-hpath-read-grammar, dogfood r8 § D5): compact JSON, `n` riding only on
/// ambiguous segments. This is the `hpath` column's one spelling — the
/// space-padded ` / ` join could not address (read splits on `/` and refused
/// its padded segments), and duplicate siblings rendered identically.
pub(crate) fn hpath_json(chain: &[AddrSeg]) -> String {
    use std::fmt::Write as _;
    let mut out = String::from("[");
    for (i, seg) in chain.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"h\":");
        out.push_str(&serde_json::Value::String(seg.h.clone()).to_string());
        if let Some(n) = seg.n {
            let _ = write!(out, ",\"n\":{n}");
        }
        out.push('}');
    }
    out.push(']');
    out
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
    emit_preamble(root, path, doc, &mut counters, rows);
    walk(
        root,
        path,
        doc,
        index,
        None,
        &[],
        &mut counters,
        rows,
        corpus,
        mounts,
    );
}

/// Emit the preamble `body` chunk — frontmatter end (0 without frontmatter) to
/// the first section's span start (file end when the document has no
/// sections). Emitted only when non-empty; it takes `seq` 0 when present, so
/// it rides BEFORE the walk (`docs/body-projection.md` §2).
fn emit_preamble(
    root: &Node,
    path: &str,
    doc: &Document,
    counters: &mut Counters,
    rows: &mut Rows,
) {
    let start = root
        .children
        .iter()
        .find(|c| matches!(c.kind, NodeKind::Frontmatter { .. }))
        .map_or(0, |fm| fm.span.end);
    let end = root
        .children
        .iter()
        .find(|c| matches!(c.kind, NodeKind::Section { .. }))
        .map_or(doc.raw.len(), |s| s.span.start);
    if start >= end {
        return;
    }
    let seq = counters.body;
    counters.body += 1;
    push_body_chunk(path, seq, None, None, start..end, doc, rows);
}

/// Stage one `body` row. `section` carries the owning section's
/// `(node_seq, hpath-chain)` and its CAS token; `None` for preamble. The text
/// slice is byte-exact: chunk boundaries are line-aligned by construction, so
/// the expect is an engine-bug trap, never a data path.
fn push_body_chunk(
    path: &str,
    seq: u64,
    section: Option<(u64, &[AddrSeg])>,
    node_rev: Option<&str>,
    span: std::ops::Range<usize>,
    doc: &Document,
    rows: &mut Rows,
) {
    let text = doc
        .raw
        .get(span.start..span.end)
        .expect("body chunk boundaries are line-aligned");
    rows.body.push(vec![
        Value::Text(path.to_string()),
        Value::UBigInt(seq),
        section.map_or(Value::Null, |(n, _)| Value::UBigInt(n)),
        section.map_or(Value::Null, |(_, chain)| Value::Text(hpath_json(chain))),
        Value::Text(text.to_string()),
        Value::UBigInt(u64c(span.start)),
        Value::UBigInt(u64c(span.end)),
        node_rev.map_or(Value::Null, |r| Value::Text(r.to_string())),
    ]);
}

/// Walk one node (emit fact rows), recurse with governing `section_seq` for
/// tasks and the governing address `chain` (a Section's own segment included).
#[allow(clippy::too_many_arguments)]
fn walk(
    node: &Node,
    path: &str,
    doc: &Document,
    index: &CorpusIndex,
    gov_section: Option<u64>,
    chain: &[AddrSeg],
    counters: &mut Counters,
    rows: &mut Rows,
    corpus: &model::RootedCorpus<'_>,
    mounts: Option<&addr::MountSet>,
) {
    let mut child_gov = gov_section;
    match &node.kind {
        NodeKind::Frontmatter { map } => emit_frontmatter(node, path, doc, map, rows),
        NodeKind::Section {
            heading_text: _,
            level,
        } => {
            let node_seq = counters.section;
            counters.section += 1;
            emit_section(node, path, *level, node_seq, chain, rows);
            emit_section_chunk(node, path, node_seq, chain, doc, counters, rows);
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
                chain,
                doc,
                counters,
                rows,
            );
        }
        _ => {}
    }
    descend(
        node, path, doc, index, child_gov, chain, counters, rows, corpus, mounts,
    );
}

/// Recurse into `node`'s children: a Section child descends with its own
/// occurrence-addressed segment appended ([`child_segs`]); every other child
/// inherits the chain as is.
#[allow(clippy::too_many_arguments)]
fn descend(
    node: &Node,
    path: &str,
    doc: &Document,
    index: &CorpusIndex,
    child_gov: Option<u64>,
    chain: &[AddrSeg],
    counters: &mut Counters,
    rows: &mut Rows,
    corpus: &model::RootedCorpus<'_>,
    mounts: Option<&addr::MountSet>,
) {
    for (child, seg) in node.children.iter().zip(child_segs(node)) {
        if let Some(seg) = seg {
            let mut child_chain = chain.to_vec();
            child_chain.push(seg);
            walk(
                child,
                path,
                doc,
                index,
                child_gov,
                &child_chain,
                counters,
                rows,
                corpus,
                mounts,
            );
        } else {
            walk(
                child, path, doc, index, child_gov, chain, counters, rows, corpus, mounts,
            );
        }
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

/// Emit one `section` row — identity `(path, node_seq)`; `hpath` advisory,
/// rendered as the published machine address ([`hpath_json`]). `chain` includes
/// the section's own segment, so its last element carries the raw heading text.
fn emit_section(
    node: &Node,
    path: &str,
    level: u8,
    node_seq: u64,
    chain: &[AddrSeg],
    rows: &mut Rows,
) {
    let heading = chain.last().map_or(String::new(), |s| s.h.clone());
    // `n` is this row's OWN segment — the last of the chain — and stays NULL
    // where the published address omits it, so the column and `hpath` can never
    // disagree about whether the heading is ambiguous.
    let n = chain
        .last()
        .and_then(|s| s.n)
        .map_or(Value::Null, Value::UInt);
    rows.section.push(vec![
        Value::Text(path.to_string()),
        Value::UBigInt(node_seq),
        Value::Text(hpath_json(chain)),
        n,
        Value::Text(heading),
        Value::UTinyInt(level),
        Value::Text(node.node_rev.0.clone()),
        Value::UBigInt(u64c(node.span.start)),
        Value::UBigInt(u64c(node.span.end)),
    ]);
}

/// Emit one `body` chunk for a section — the exclusive-content law
/// (`docs/body-projection.md` §2): from the content span's start (heading line
/// stripped — `model::content_span`, the law's one owner) to the first child
/// section's span start, else the section's own span end. Always emitted,
/// empty text included: `COUNT(body WHERE section_seq IS NOT NULL) =
/// COUNT(section)` is the spec's invariant.
fn emit_section_chunk(
    node: &Node,
    path: &str,
    node_seq: u64,
    chain: &[AddrSeg],
    doc: &Document,
    counters: &mut Counters,
    rows: &mut Rows,
) {
    let start = model::content_span(node, doc.raw.as_bytes()).start;
    let end = node
        .children
        .iter()
        .find(|c| matches!(c.kind, NodeKind::Section { .. }))
        .map_or(node.span.end, |c| c.span.start);
    let seq = counters.body;
    counters.body += 1;
    push_body_chunk(
        path,
        seq,
        Some((node_seq, chain)),
        Some(&node.node_rev.0),
        start..end,
        doc,
        rows,
    );
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
        // `exclusion` + `exclusion_path` — filled by [`project`] after
        // collection, because the answer needs the workspace root and this
        // crate holds none.
        Value::Null,
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
const LINK_COL_EXCLUSION_PATH: usize = 11;

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

/// `- [x] text` → `text`: drop the list marker (bullet or ordered) and the
/// checkbox the parser recognised — `checked` already carries the bit, so the
/// marker in `text` only duplicated it into every GROUP BY key (card
/// sql-task-text-marker). A line that does not match the marker shape serves
/// unchanged rather than guessed at.
fn strip_task_marker(line: &str) -> &str {
    let after_bullet = if let Some(rest) = line.strip_prefix(['-', '*', '+']) {
        rest
    } else {
        let after_digits = line.trim_start_matches(|c: char| c.is_ascii_digit());
        if after_digits.len() == line.len() {
            return line;
        }
        let Some(rest) = after_digits.strip_prefix(['.', ')']) else {
            return line;
        };
        rest
    };
    let Some(after_open) = after_bullet.trim_start().strip_prefix('[') else {
        return line;
    };
    let mut chars = after_open.chars();
    let (Some(_state), Some(']')) = (chars.next(), chars.next()) else {
        return line;
    };
    chars.as_str().trim_start()
}

/// Emit one `task` row. `section_seq` NULL = document-level; `text` is the
/// trimmed task text with the marker stripped ([`strip_task_marker`]); `hpath`
/// is the governing section's machine address (NULL when document-level).
#[allow(clippy::too_many_arguments)]
fn emit_task(
    node: &Node,
    path: &str,
    checked: bool,
    depth: u32,
    gov_section: Option<u64>,
    chain: &[AddrSeg],
    doc: &Document,
    counters: &mut Counters,
    rows: &mut Rows,
) {
    let seq = counters.task;
    counters.task += 1;
    let text =
        strip_task_marker(doc.raw.get(node.span.clone()).unwrap_or_default().trim()).to_string();
    rows.task.push(vec![
        Value::Text(path.to_string()),
        Value::UBigInt(seq),
        Value::Boolean(checked),
        Value::UInt(depth),
        gov_section.map_or(Value::Null, Value::UBigInt),
        gov_section.map_or(Value::Null, |_| Value::Text(hpath_json(chain))),
        Value::Text(text),
        Value::UBigInt(u64c(node.span.start)),
        Value::UBigInt(u64c(node.span.end)),
        Value::Text(node.node_rev.0.clone()),
    ]);
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
        append_rows(conn, "section", &self.section)?;
        append_rows(conn, "link", &self.link)?;
        append_rows(conn, "tag", &self.tag)?;
        append_rows(conn, "frontmatter_tag", &self.frontmatter_tag)?;
        append_rows(conn, "task", &self.task)?;
        // The base relations, parents before children (the FK order every
        // other table already loads in).
        append_rows(conn, "base", &self.base)?;
        append_rows(conn, "base_view", &self.base_view)?;
        append_rows(conn, "base_formula", &self.base_formula)?;
        append_rows(conn, "body", &self.body)
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

/// Insert singleton `_meridian_view` stamp. epoch/seq both set or both NULL
/// (DDL CHECK enforces pairing).
#[allow(clippy::too_many_arguments)]
fn write_stamp(
    conn: &Connection,
    as_of_fingerprint: &str,
    workspace: &str,
    epoch: Option<&str>,
    seq: Option<u64>,
    builder: &str,
    doc_count: usize,
    base_fold: Option<&str>,
) -> duckdb::Result<()> {
    let built_unix = i64::try_from(now_secs()).unwrap_or(i64::MAX);
    conn.execute(
        "INSERT INTO _meridian_view \
         (singleton, schema_version, as_of_fingerprint, workspace, built_epoch, built_seq, built_unix, builder, doc_count, base_fold) \
         VALUES (true, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
                // NULL = the build was handed no base walk ("not asked",
                // never "empty" — §6.2).
                base_fold.map_or(Value::Null, |f| Value::Text(f.to_string())),
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

    /// The body-projection worked gate (`docs/body-projection.md` §8) plus its
    /// §7.1 negatives: exclusive-content chunks, preamble handling, CJK bytes
    /// verbatim, the empty-content row, and the section-count invariant.
    #[test]
    fn body_chunks_follow_the_exclusive_content_law() {
        /// One asserted chunk row: (`path`, `seq`, `section_seq`, `hpath`, `text`).
        type BodyRow = (String, u64, Option<u64>, Option<String>, String);
        let docs: BTreeMap<String, Document> = [
            (
                "a.md",
                "---\ntitle: Alpha\n---\npreamble line\n\n# Top\nintro\n\n## Sub\nsub body\n",
            ),
            (
                "cjk.md",
                "# \u{4e2d}\u{6587}\n\u{6b63}\u{6587}\u{5185}\u{5bb9}\n",
            ),
            ("nopre.md", "# Only\nbody\n"),
            ("fmonly.md", "---\nk: v\n---\n"),
            ("hollow.md", "# A\n## B\nb\n"),
        ]
        .into_iter()
        .map(|(p, raw)| {
            (
                p.to_string(),
                model::build(raw.to_string(), syntax::parse(raw)),
            )
        })
        .collect();
        let conn = build_memory(&docs, "b3:body-gate").expect("build");

        let rows: Vec<BodyRow> = {
            let mut stmt = conn
                .prepare("SELECT path, seq, section_seq, hpath, text FROM body ORDER BY path, seq")
                .expect("prepare");
            let got = stmt
                .query_map([], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                })
                .expect("query");
            got.collect::<Result<_, _>>().expect("rows")
        };

        let expect: Vec<BodyRow> = vec![
            // a.md: preamble, Top's exclusive content, Sub's content.
            ("a.md".into(), 0, None, None, "preamble line\n\n".into()),
            (
                "a.md".into(),
                1,
                Some(0),
                Some(r#"[{"h":"Top"}]"#.into()),
                "intro\n\n".into(),
            ),
            (
                "a.md".into(),
                2,
                Some(1),
                Some(r#"[{"h":"Top"},{"h":"Sub"}]"#.into()),
                "sub body\n".into(),
            ),
            // cjk.md: bytes verbatim.
            (
                "cjk.md".into(),
                0,
                Some(0),
                Some("[{\"h\":\"\u{4e2d}\u{6587}\"}]".into()),
                "\u{6b63}\u{6587}\u{5185}\u{5bb9}\n".into(),
            ),
            // fmonly.md: zero rows (no preamble byte, no section).
            // hollow.md: A's chunk is EMPTY but PRESENT; B carries the byte.
            (
                "hollow.md".into(),
                0,
                Some(0),
                Some(r#"[{"h":"A"}]"#.into()),
                String::new(),
            ),
            (
                "hollow.md".into(),
                1,
                Some(1),
                Some(r#"[{"h":"A"},{"h":"B"}]"#.into()),
                "b\n".into(),
            ),
            // nopre.md: no preamble row.
            (
                "nopre.md".into(),
                0,
                Some(0),
                Some(r#"[{"h":"Only"}]"#.into()),
                "body\n".into(),
            ),
        ];
        assert_eq!(rows, expect);

        // The §2 invariant: one section chunk per section, always.
        let (chunks, sections): (i64, i64) = conn
            .query_row(
                "SELECT (SELECT count(*) FROM body WHERE section_seq IS NOT NULL), \
                        (SELECT count(*) FROM section)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("counts");
        assert_eq!(chunks, sections);
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
    // row-at-a-time statement lane it replaced.

    /// The retired row-at-a-time statement lane, kept verbatim as the
    /// equivalence reference the appender lane is gated against.
    fn statement_lane_insert(rows: &Rows, conn: &Connection) -> duckdb::Result<()> {
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
        insert_rows(
            conn,
            "INSERT INTO section (path, node_seq, hpath, n, heading, level, node_rev, span_start, span_end) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            &rows.section,
        )?;
        insert_rows(
            conn,
            "INSERT INTO link (src_path, seq, kind, target_raw, heading, block, alias, dest_path, dest_root, dest_root_path, exclusion, exclusion_path, span_start, span_end, node_rev) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
        insert_rows(
            conn,
            "INSERT INTO task (path, seq, checked, depth, section_seq, hpath, text, span_start, span_end, node_rev) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            &rows.task,
        )?;
        insert_rows(
            conn,
            "INSERT INTO base (path, file_rev, bytes, error, filters, properties, extra) VALUES (?, ?, ?, ?, ?, ?, ?)",
            &rows.base,
        )?;
        insert_rows(
            conn,
            "INSERT INTO base_view (path, ord, name, type, filters, config) VALUES (?, ?, ?, ?, ?, ?)",
            &rows.base_view,
        )?;
        insert_rows(
            conn,
            "INSERT INTO base_formula (path, ord, name, expr) VALUES (?, ?, ?, ?)",
            &rows.base_formula,
        )?;
        insert_rows(
            conn,
            "INSERT INTO body (path, seq, section_seq, hpath, text, span_start, span_end, node_rev) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            &rows.body,
        )
    }

    /// Per-table digest over every column of every row (`resolved` is
    /// generated, never stored, so it is derived equal by construction). The
    /// hpath columns are TEXT (the JSON machine address — injective and
    /// self-delimiting), so they digest verbatim.
    const LANE_DIGESTS: [(&str, &str); 11] = [
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
            "SELECT coalesce(md5(string_agg(path || '|' || node_seq::VARCHAR || '|' || hpath || '|' || coalesce(n::VARCHAR,'~N~') || '|' || heading || '|' || level::VARCHAR || '|' || node_rev || '|' || span_start::VARCHAR || '|' || span_end::VARCHAR, chr(10) ORDER BY path, node_seq)), 'EMPTY') FROM section",
        ),
        (
            "link",
            "SELECT coalesce(md5(string_agg(src_path || '|' || seq::VARCHAR || '|' || kind || '|' || target_raw || '|' || coalesce(heading,'~N~') || '|' || coalesce(block,'~N~') || '|' || coalesce(alias,'~N~') || '|' || coalesce(dest_path,'~N~') || '|' || coalesce(dest_root,'~N~') || '|' || coalesce(dest_root_path,'~N~') || '|' || coalesce(exclusion,'~N~') || '|' || coalesce(exclusion_path,'~N~') || '|' || span_start::VARCHAR || '|' || span_end::VARCHAR || '|' || node_rev, chr(10) ORDER BY src_path, seq)), 'EMPTY') FROM link",
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
            "base",
            "SELECT coalesce(md5(string_agg(path || '|' || coalesce(file_rev,'~N~') || '|' || coalesce(bytes::VARCHAR,'~N~') || '|' || coalesce(error,'~N~') || '|' || coalesce(filters,'~N~') || '|' || coalesce(properties,'~N~') || '|' || coalesce(extra,'~N~'), chr(10) ORDER BY path)), 'EMPTY') FROM base",
        ),
        (
            "base_view",
            "SELECT coalesce(md5(string_agg(path || '|' || ord::VARCHAR || '|' || coalesce(name,'~N~') || '|' || coalesce(type,'~N~') || '|' || coalesce(filters,'~N~') || '|' || coalesce(config,'~N~'), chr(10) ORDER BY path, ord)), 'EMPTY') FROM base_view",
        ),
        (
            "base_formula",
            "SELECT coalesce(md5(string_agg(path || '|' || ord::VARCHAR || '|' || name || '|' || expr, chr(10) ORDER BY path, name)), 'EMPTY') FROM base_formula",
        ),
        (
            "body",
            "SELECT coalesce(md5(string_agg(path || '|' || seq::VARCHAR || '|' || coalesce(section_seq::VARCHAR,'~N~') || '|' || coalesce(hpath,'~N~') || '|' || text || '|' || span_start::VARCHAR || '|' || span_end::VARCHAR || '|' || coalesce(node_rev,'~N~'), chr(10) ORDER BY path, seq)), 'EMPTY') FROM body",
        ),
    ];

    /// Load `rows` into a fresh schema through `lane`; return the per-table digests.
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

    /// One filler row per scalar table, so the encoding-edge gate loads every
    /// lane and not only the hpath carriers.
    fn stage_scalar_fillers(rows: &mut Rows) {
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
            Value::Null,
            Value::UBigInt(0),
            Value::UBigInt(1),
            Value::Text("rev".to_string()),
        ]);
    }

    /// The encoding edges, hand-staged so the gate cannot silently lose them
    /// to parser drift: section hpath for an empty heading (`[{"h":""}]`) and
    /// for text carrying JSON-hostile bytes; task hpath NULL vs present.
    #[test]
    fn appender_lane_encoding_edges_match_statement_lane() {
        let mut rows = Rows::default();
        rows.doc.push(vec![
            Value::Text("d.md".to_string()),
            Value::Text("rev0".to_string()),
            Value::UInt(1),
            Value::UBigInt(2),
        ]);
        // Each shape carries the `n` its own last segment does, so both lanes
        // are exercised on a NULL occurrence and a present one.
        let shapes = [
            (
                hpath_json(&[AddrSeg {
                    h: String::new(),
                    n: None,
                }]),
                Value::Null,
            ),
            (
                hpath_json(&[
                    AddrSeg {
                        h: "a\"b\\c".to_string(),
                        n: None,
                    },
                    AddrSeg {
                        h: "Dup".to_string(),
                        n: Some(2),
                    },
                ]),
                Value::UInt(2),
            ),
        ];
        for (seq, (hpath, n)) in shapes.into_iter().enumerate() {
            rows.section.push(vec![
                Value::Text("d.md".to_string()),
                Value::UBigInt(seq as u64),
                Value::Text(hpath),
                n,
                Value::Text("h".to_string()),
                Value::UTinyInt(1),
                Value::Text("rev".to_string()),
                Value::UBigInt(0),
                Value::UBigInt(1),
            ]);
        }
        stage_scalar_fillers(&mut rows);
        let task_shapes = [
            (Value::Null, Value::Null),
            (
                Value::UBigInt(0),
                Value::Text(hpath_json(&[AddrSeg {
                    h: String::new(),
                    n: None,
                }])),
            ),
        ];
        for (seq, (section_seq, hpath)) in task_shapes.into_iter().enumerate() {
            rows.task.push(vec![
                Value::Text("d.md".to_string()),
                Value::UBigInt(seq as u64),
                Value::Boolean(false),
                Value::UInt(0),
                section_seq,
                hpath,
                Value::Text("t".to_string()),
                Value::UBigInt(0),
                Value::UBigInt(1),
                Value::Text("rev".to_string()),
            ]);
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
        let section_hpath = |row: &Vec<Value>| match &row[2] {
            Value::Text(s) => s.clone(),
            other => panic!("section hpath must stage as TEXT, got {other:?}"),
        };
        assert!(
            rows.section
                .iter()
                .any(|s| section_hpath(s) == r#"[{"h":""}]"#),
            "a bare `#` heading must stage hpath [{{\"h\":\"\"}}] — the empty-heading edge"
        );
        assert!(
            rows.task
                .iter()
                .any(|t| matches!(&t[5], Value::Text(s) if s == r#"[{"h":""}]"#)),
            "the task under the empty heading must stage its section's address"
        );
        assert!(
            rows.task.iter().any(|t| matches!(&t[5], Value::Null)),
            "the doc-level task must stage hpath NULL"
        );

        assert_lanes_match(&rows);
    }

    /// The render rule of card sql-hpath-read-grammar (dogfood r8 § D5),
    /// pinned on a parsed fixture: every hpath cell is the `[{"h":…},…]`
    /// machine address; duplicate siblings carry distinct `n`; a heading whose
    /// own text bears `/` stays one segment; the retired ` / ` join appears
    /// nowhere.
    #[test]
    fn hpath_cells_are_machine_addresses_with_n_on_collision() {
        let raw = "# R8 Fixture\n\n## Alpha\n\nbody\n\n## io/paths\n\n### mix a/b\n\nbody\n\n## Dup\n\n- [ ] task in first Dup\n\n## Dup\n\nbody\n";
        let mut docs = BTreeMap::new();
        docs.insert(
            "f.md".to_string(),
            model::build(raw.to_string(), syntax::parse(raw)),
        );
        let conn = build_memory(&docs, "b3:fixture").expect("build");

        let mut stmt = conn
            .prepare("SELECT hpath FROM section ORDER BY node_seq")
            .expect("prepare");
        let got: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("rows");
        assert_eq!(
            got,
            vec![
                r#"[{"h":"R8 Fixture"}]"#.to_string(),
                r#"[{"h":"R8 Fixture"},{"h":"Alpha"}]"#.to_string(),
                r#"[{"h":"R8 Fixture"},{"h":"io/paths"}]"#.to_string(),
                r#"[{"h":"R8 Fixture"},{"h":"io/paths"},{"h":"mix a/b"}]"#.to_string(),
                r#"[{"h":"R8 Fixture"},{"h":"Dup","n":1}]"#.to_string(),
                r#"[{"h":"R8 Fixture"},{"h":"Dup","n":2}]"#.to_string(),
            ],
            "every cell is the published machine address; dup siblings differ by n"
        );

        // The task under the FIRST Dup carries that occurrence's address.
        let task_hpath: String = conn
            .query_row("SELECT hpath FROM task WHERE path='f.md'", [], |r| r.get(0))
            .expect("task hpath");
        assert_eq!(task_hpath, r#"[{"h":"R8 Fixture"},{"h":"Dup","n":1}]"#);
    }

    /// The round-trip half of the same card: a dup-sibling cell taken from the
    /// projection, parsed as the read/put hpath grammar, resolves through
    /// `model::resolve` to the RIGHT occurrence — and the un-`n`'d spelling of
    /// the same text stays loudly ambiguous.
    #[test]
    fn dup_sibling_hpath_cell_resolves_to_the_right_occurrence() {
        let raw = "# T\n\n## Dup\n\nfirst body\n\n## Dup\n\nsecond body\n";
        let doc = model::build(raw.to_string(), syntax::parse(raw));
        let mut docs = BTreeMap::new();
        docs.insert("t.md".to_string(), doc);
        let conn = build_memory(&docs, "b3:roundtrip").expect("build");
        let cell: String = conn
            .query_row(
                "SELECT hpath FROM section WHERE heading='Dup' AND node_seq=2",
                [],
                |r| r.get(0),
            )
            .expect("dup cell");
        assert_eq!(cell, r#"[{"h":"T"},{"h":"Dup","n":2}]"#);

        let segs: Vec<serde_json::Value> = serde_json::from_str(&cell).expect("cell parses");
        let hpath: Vec<model::HpathSeg> = segs
            .iter()
            .map(|s| model::HpathSeg {
                h: s["h"].as_str().expect("h").to_string(),
                n: s.get("n")
                    .and_then(serde_json::Value::as_u64)
                    .map(|n| u32::try_from(n).expect("n fits")),
            })
            .collect();
        let doc = &docs["t.md"];
        let target =
            model::resolve(doc, &model::Ref::Hpath(hpath)).expect("cell resolves unambiguously");
        let resolved = &doc.raw[target.span.clone()];
        assert!(
            resolved.contains("second body") && !resolved.contains("first body"),
            "the n:2 cell resolves to the SECOND Dup, got: {resolved}"
        );

        let bare = model::Ref::Hpath(vec![
            model::HpathSeg {
                h: "T".to_string(),
                n: None,
            },
            model::HpathSeg {
                h: "Dup".to_string(),
                n: None,
            },
        ]);
        assert!(
            matches!(
                model::resolve(doc, &bare),
                Err(model::ResolveError::Ambiguous(_))
            ),
            "the un-n'd spelling stays loudly ambiguous — n is what the cell adds"
        );
    }

    /// One address law, two faces: the projection's hpath cell must be
    /// byte-identical to the machine address the read face publishes for the
    /// same section (wire-map's occurrence-addressed `ReadFact.hpath`,
    /// rendered as the same compact JSON). Guards the drift class the
    /// machine-form review named: two faces answering one question need one
    /// observable answer.
    #[test]
    fn hpath_cells_match_the_read_faces_published_addresses() {
        let raw = "# R8 Fixture\n\n## Alpha\n\nbody\n\n## io/paths\n\n### mix a/b\n\nbody\n\n## Dup\n\nfirst\n\n## Dup\n\nsecond\n\n## \u{4e2d}\u{6587} \"q\"\n\nbody\n";
        let doc = model::build(raw.to_string(), syntax::parse(raw));

        // The read face's published addresses, keyed by section span.
        let toc = wire_map::project_toc(&doc);
        let facts = wire_map::facts::read_facts(&toc, doc.raw.as_bytes());
        let mut published: BTreeMap<(u64, u64), String> = BTreeMap::new();
        for f in &facts {
            if f.anchor.is_none() && !f.hpath.is_empty() {
                let rendered = hpath_json(
                    &f.hpath
                        .iter()
                        .map(|s| AddrSeg {
                            h: s.h.clone(),
                            n: s.n,
                        })
                        .collect::<Vec<_>>(),
                );
                published.insert((f.span.0, f.span.1), rendered);
            }
        }

        let mut docs = BTreeMap::new();
        docs.insert("p.md".to_string(), doc);
        let conn = build_memory(&docs, "b3:parity").expect("build");
        let mut stmt = conn
            .prepare("SELECT hpath, span_start, span_end FROM section ORDER BY node_seq")
            .expect("prepare");
        let rows: Vec<(String, u64, u64)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("rows");
        assert!(!rows.is_empty(), "fixture projects sections");
        for (cell, start, end) in rows {
            let want = published.get(&(start, end)).unwrap_or_else(|| {
                panic!("read face publishes no address for span {start}..{end}")
            });
            assert_eq!(
                &cell, want,
                "projection and read face disagree on the address of span {start}..{end}"
            );
        }
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
