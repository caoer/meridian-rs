//! The in-memory world model: governed node tree (`kind/span/node_rev/hpath`),
//! resolve, CAS-splice validation, Merkle roots — deliberately non-serializable.
//!
//! # Charter
//! **Owns:** the derived world model. From `syntax`'s flat dialect stream it
//! builds the governed tree (sections govern children, hpath chains — the node
//! model policy-schema-design §2 guarantees; that doc is the contract, not
//! restated here), computes `node_rev` hashes and Merkle roots, resolves
//! `#hpath` / `#^anchor` refs to targets, validates CAS splices, and diffs at
//! node level. The corpus name index lives here too — derived, disposable —
//! and `query`/`policy` *borrow* it, never own it.
//!
//! **Never does:** file I/O (that's `fs`), persistence of any kind (law 2: Rust
//! memory is disposable; cold rebuild is the recovery path), protocol types
//! (law 3), body formatting.
//!
//! # Law enforcement (candidate thesis, this crate's part)
//! **No serde derives on any public type in this crate — by design, permanently**
//! (rust-analyzer's "ide types are not serializable by design"). The wire cannot
//! leak inward: whoever wants a model fact on the wire must go through `sidecar`,
//! the one crate that sees `wire` and `model` together. That makes law 3
//! ("nothing Go-facing beyond the wire") a compile error, not a convention.
//!
//! # Rungs
//! Rung 1: tree build + hpath. Rung 2: `resolve` + splice validation + revs.
//! Rung 3: roots + guard. Rung 4: node-level diff feeds the change feed.
//! Rung 5: the corpus index `query` borrows.

use std::collections::BTreeMap;
use std::ops::Range;

pub mod walk;

/// Half-open byte range into a file's raw bytes. Distinct from the wire's
/// serializable span on purpose — converting between them is `sidecar`'s job.
pub type ByteSpan = Range<usize>;

/// Node content hash — the CAS token's model-side form. Opaque; algorithm is a
/// rung-2 wire amendment (meridian's xxhash64 `sec_rev` is the migration donor;
/// the mapping must be stated when the amendment lands).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NodeRev(pub String);

/// Merkle root over a file or the corpus (rung 3). Go keeps one of these — a
/// 32-byte cursor — as its only markdown-derived state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MerkleRoot(pub String);

/// Parsed frontmatter, DOCUMENT ORDER preserved (the M2-PROJECT ordered-keys
/// amendment: wire `keys` must echo document order, B1 predicate 4 — a sorted
/// map betrayed the order). Flat `(key, value)` pairs, first occurrence wins;
/// no YAML library (no-serde crate law; nesting deferred to the rung that
/// needs it).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct YamlMap(pub Vec<(String, String)>);

impl YamlMap {
    /// Keys in document order.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(|(k, _)| k.as_str())
    }
}

/// The governed tree node. Every node carries kind + span + `node_rev` + hpath
/// (`None` for document/frontmatter), per policy-schema §2's guaranteed surface.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub kind: NodeKind,
    pub span: ByteSpan,
    pub node_rev: NodeRev,
    /// Chain of heading texts root → governing heading; delimiter-free array
    /// form (a literal `/` in a heading needs no escaping).
    pub hpath: Option<Vec<String>>,
    pub children: Vec<Node>,
}

/// Model node kinds — the policy-schema §2 vocabulary. Richer than the wire's
/// flat kind enum (sections, paragraphs, lists exist here); the wire projection
/// is `sidecar`'s.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind {
    Document {
        path: String,
        line_count: u32,
    },
    Frontmatter {
        map: YamlMap,
    },
    Section {
        heading_text: String,
        level: u8,
    },
    Heading {
        text: String,
        level: u8,
    },
    Paragraph,
    List,
    ListItem,
    TaskItem {
        checked: bool,
        depth: u32,
    },
    CodeBlock {
        lang: String,
        unterminated: bool,
    },
    Callout {
        r#type: String,
        fold: String,
    },
    Table,
    Wikilink {
        target: String,
        heading: Option<String>,
        block: Option<String>,
        alias: Option<String>,
    },
    Link {
        target: String,
    },
    Embed {
        target: String,
        heading: Option<String>,
        block: Option<String>,
        alias: Option<String>,
    },
    Anchor {
        name: String,
    },
    Tag {
        name: String,
    },
    /// Wire-observable leaf (M2-PROJECT vocabulary amendment — the B1
    /// predicate-1 gap, closed: every dialect construct is representable).
    InlineCode,
    /// Wire-observable leaf (same amendment).
    Comment,
}

/// One parsed file: the tree plus the raw bytes it was derived from (spans
/// index into `raw`; keeping them together makes span-law violations testable).
#[derive(Debug, Clone)]
pub struct Document {
    pub raw: String,
    pub root: Node,
}

/// Build the governed tree from `syntax`'s dialect stream: assemble sections,
/// parse frontmatter, compute hpath chains and revs. The syntax→model seam.
///
/// Structure: `Document` root (span = whole file) → an optional `Frontmatter`
/// node then heading-nested `Section` nodes; leaf dialect constructs (fences,
/// anchors, wikilinks, …) attach to the deepest section that spans them. Every
/// node carries `node_rev = blake3-256(span bytes)[:16]` (contract §1); the
/// root node's span is the whole file, so its `node_rev` equals `file_rev`
/// by construction (`blake3-256(whole file)[:16]`).
#[must_use]
pub fn build(raw: String, nodes: Vec<syntax::DialectNode>) -> Document {
    use syntax::DialectKind as D;

    let mut frontmatter: Option<Node> = None;
    // (start, level, heading_text) in document order; section spans derived below.
    let mut headings: Vec<(usize, u8, String)> = Vec::new();
    let mut leaves: Vec<Node> = Vec::new();

    for node in nodes {
        let syntax::DialectNode { kind, span } = node;
        match kind {
            D::Frontmatter { .. } => {
                // Fence-to-fence, terminator-inclusive (§18 row 3): `syntax` trims
                // the closing fence's terminator (leaf-block trim), but the model
                // frontmatter node is span-lawed with the section (newline-inclusive)
                // family — extend the end over that one terminator.
                let span = span.start..extend_terminator(raw.as_bytes(), span.end);
                let map = parse_frontmatter(&raw, &span);
                frontmatter = Some(leaf_node(&raw, NodeKind::Frontmatter { map }, span));
            }
            D::Heading { level, text } => headings.push((span.start, level, text)),
            other => {
                if let Some(kind) = leaf_kind(other) {
                    leaves.push(leaf_node(&raw, kind, span));
                }
            }
        }
    }

    // Sections, leaves, and the frontmatter node fold into one containment forest
    // under the document root.
    let mut items = section_nodes(&raw, &headings);
    items.extend(leaves);
    items.extend(frontmatter);

    let root_span = 0..raw.len();
    let mut root = Node {
        kind: NodeKind::Document {
            path: String::new(),
            line_count: u32::try_from(raw.lines().count()).unwrap_or(u32::MAX),
        },
        node_rev: node_rev(raw.as_bytes(), &root_span),
        span: root_span,
        hpath: None,
        children: nest_by_containment(items),
    };
    sort_tree(&mut root);
    fill_hpath(&mut root, &mut Vec::new());

    Document { raw, root }
}

/// A leaf/standalone node with its span-derived `node_rev` and no children yet.
fn leaf_node(raw: &str, kind: NodeKind, span: ByteSpan) -> Node {
    Node {
        node_rev: node_rev(raw.as_bytes(), &span),
        kind,
        span,
        hpath: None,
        children: Vec::new(),
    }
}

/// Section nodes from the heading list: a section runs from its heading start to
/// the next heading of level ≤ its own, else EOF — newline-inclusive,
/// heading-inclusive (contract §1 span sub-laws / compute.py `sections`).
fn section_nodes(raw: &str, headings: &[(usize, u8, String)]) -> Vec<Node> {
    let len = raw.len();
    headings
        .iter()
        .enumerate()
        .map(|(i, (start, level, text))| {
            let end = headings[i + 1..]
                .iter()
                .find(|(_, l, _)| *l <= *level)
                .map_or(len, |(s, _, _)| *s);
            leaf_node(
                raw,
                NodeKind::Section {
                    heading_text: text.clone(),
                    level: *level,
                },
                *start..end,
            )
        })
        .collect()
}

/// Fold a flat node list into a containment forest: sort so a container precedes
/// everything it spans (start asc, end desc), then stack — an item the stack top
/// spans becomes its child, otherwise the top closes and joins its own parent.
/// Handles frontmatter, sections, and leaves uniformly, with no panicking `unwrap`.
fn nest_by_containment(mut items: Vec<Node>) -> Vec<Node> {
    items.sort_by(span_order);
    let mut stack: Vec<Node> = Vec::new();
    let mut forest: Vec<Node> = Vec::new();
    let close = |done: Node, stack: &mut Vec<Node>, forest: &mut Vec<Node>| match stack.last_mut() {
        Some(parent) => parent.children.push(done),
        None => forest.push(done),
    };
    for item in items {
        while let Some(top) = stack.last() {
            if top.span.start <= item.span.start && item.span.end <= top.span.end {
                break; // top contains item
            }
            if let Some(done) = stack.pop() {
                close(done, &mut stack, &mut forest);
            }
        }
        stack.push(item);
    }
    while let Some(done) = stack.pop() {
        close(done, &mut stack, &mut forest);
    }
    forest
}

/// `node_rev = blake3-256(span bytes)[:16]` — 16 lowercase hex (contract §1).
/// Slices raw bytes (not `&str`) so a span is never asked to be char-aligned.
fn node_rev(bytes: &[u8], span: &ByteSpan) -> NodeRev {
    NodeRev(blake3::hash(&bytes[span.clone()]).to_hex().as_str()[..16].to_string())
}

/// The §1 total order: span.start asc, span.end desc (container before contained),
/// then kind ordinal.
fn span_order(a: &Node, b: &Node) -> std::cmp::Ordering {
    a.span
        .start
        .cmp(&b.span.start)
        .then(b.span.end.cmp(&a.span.end))
        .then(kind_ordinal(&a.kind).cmp(&kind_ordinal(&b.kind)))
}

/// Include a single trailing line terminator (`\n` or `\r\n`) — the closing
/// fence's own terminator, making the frontmatter node fence-to-fence (§18 row 3).
fn extend_terminator(bytes: &[u8], mut end: usize) -> usize {
    if end < bytes.len() && bytes[end] == b'\r' {
        end += 1;
    }
    if end < bytes.len() && bytes[end] == b'\n' {
        end += 1;
    }
    end
}

/// Model's OWN flat-frontmatter parse (top-level `key: value`, column 0) — the
/// keys authority per the R1 forward-note, NOT `syntax`'s best-effort list. No
/// serde/YAML crate: the no-serde crate law forbids it and the corpus frontmatter
/// is flat; a full YAML library is deferred to the rung that needs nesting.
/// Key order is document order (first occurrence wins) — the wire `keys`
/// surface echoes it verbatim (B1 predicate 4).
fn parse_frontmatter(raw: &str, span: &ByteSpan) -> YamlMap {
    let block = raw.get(span.clone()).unwrap_or_default();
    let mut pairs: Vec<(String, String)> = Vec::new();
    for line in block.lines() {
        let trimmed = line.trim();
        if trimmed == "---" || trimmed.is_empty() {
            continue;
        }
        // top-level keys sit at column 0; indented lines are values/nesting
        if line.starts_with([' ', '\t']) || trimmed.starts_with('#') {
            continue;
        }
        if let Some(colon) = line.find(':') {
            let key = line[..colon].trim().trim_matches(['"', '\'']).to_string();
            if key.is_empty() {
                continue;
            }
            if pairs.iter().all(|(k, _)| *k != key) {
                pairs.push((key, line[colon + 1..].trim().to_string()));
            }
        }
    }
    YamlMap(pairs)
}

/// Map a leaf dialect construct to its model node kind. `Heading`/`Frontmatter`
/// are handled structurally (sections / fm node); `InlineCode`/`Comment` have no
/// model kind yet (B1-SUPERSET's fail-first surface) and are dropped.
fn leaf_kind(dk: syntax::DialectKind) -> Option<NodeKind> {
    use syntax::DialectKind as D;
    Some(match dk {
        D::Fence {
            info_string,
            unterminated,
        } => NodeKind::CodeBlock {
            lang: info_string,
            unterminated,
        },
        D::Anchor { id } => NodeKind::Anchor { name: id },
        D::Wikilink {
            target,
            heading,
            block,
            alias,
        } => NodeKind::Wikilink {
            target,
            heading,
            block,
            alias,
        },
        D::Embed {
            target,
            heading,
            block,
            alias,
        } => NodeKind::Embed {
            target,
            heading,
            block,
            alias,
        },
        D::Callout { r#type, fold } => NodeKind::Callout { r#type, fold },
        D::Task { checked, depth } => NodeKind::TaskItem { checked, depth },
        D::Table => NodeKind::Table,
        D::InlineCode => NodeKind::InlineCode,
        D::Comment => NodeKind::Comment,
        D::Frontmatter { .. } | D::Heading { .. } => return None,
    })
}

/// Deterministic child order (the §1 total order), applied recursively.
fn sort_tree(node: &mut Node) {
    node.children.sort_by(span_order);
    for c in &mut node.children {
        sort_tree(c);
    }
}

/// Assign `hpath` — the chain of governing heading texts. Sections carry their
/// own heading; descendants inherit the governing chain; document/frontmatter and
/// pre-heading nodes have none.
fn fill_hpath(node: &mut Node, chain: &mut Vec<String>) {
    let pushed = if let NodeKind::Section { heading_text, .. } = &node.kind {
        chain.push(heading_text.clone());
        true
    } else {
        false
    };
    node.hpath = match &node.kind {
        NodeKind::Document { .. } | NodeKind::Frontmatter { .. } => None,
        NodeKind::Section { .. } => Some(chain.clone()),
        _ if chain.is_empty() => None,
        _ => Some(chain.clone()),
    };
    for c in &mut node.children {
        fill_hpath(c, chain);
    }
    if pushed {
        chain.pop();
    }
}

/// Kind ordinal for the total-order tiebreak (declaration order).
fn kind_ordinal(kind: &NodeKind) -> u8 {
    match kind {
        NodeKind::Document { .. } => 0,
        NodeKind::Frontmatter { .. } => 1,
        NodeKind::Section { .. } => 2,
        NodeKind::Heading { .. } => 3,
        NodeKind::Paragraph => 4,
        NodeKind::List => 5,
        NodeKind::ListItem => 6,
        NodeKind::TaskItem { .. } => 7,
        NodeKind::CodeBlock { .. } => 8,
        NodeKind::Callout { .. } => 9,
        NodeKind::Table => 10,
        NodeKind::Wikilink { .. } => 11,
        NodeKind::Link { .. } => 12,
        NodeKind::Embed { .. } => 13,
        NodeKind::Anchor { .. } => 14,
        NodeKind::Tag { .. } => 15,
        NodeKind::InlineCode => 16,
        NodeKind::Comment => 17,
    }
}

// ---------------------------------------------------------------------------
// resolve (rung 2)
// ---------------------------------------------------------------------------

/// One hpath segment — a heading text plus an optional 1-based occurrence index
/// among identical raw texts at that containment position (contract §2.1). The
/// model-side twin of `wire::HpathSeg`; the crates never share a type (no-serde
/// law), the sidecar converts between them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HpathSeg {
    /// Heading text, matched **byte-exactly** against the containment tree
    /// (the mint plane never case-folds — that is the walk plane's job).
    pub h: String,
    /// 1-based occurrence among identical sibling heading texts. `None` demands
    /// uniqueness: a duplicate with no disambiguator resolves `Ambiguous` (loud),
    /// never silently picked (contract §2.1, the mint plane's never-silently-picks
    /// law).
    pub n: Option<u32>,
}

/// A mint-plane ref — the strict fleet grammar (contract §2.1). Three forms,
/// per-segment byte-equality (`hpath`), exact block id (`anchor`), or top-level
/// frontmatter key (`fm_key`). No join string: `#A#a/b` vs `#A#a#b` is
/// unrepresentable here. The Obsidian walk algebra is a **separate** grammar,
/// carried only by [`walk`] — never by this type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ref {
    /// `{"hpath":[{"h":"Goals"},{"h":"Q3"}]}` — descend the containment tree
    /// segment by segment, byte-exact.
    Hpath(Vec<HpathSeg>),
    /// `{"anchor":"r-000042"}` — a block id, exact match; mint via [`Ref::anchor`]
    /// so the one block-id charset (§2.4) is enforced.
    Anchor(String),
    /// `{"fm_key":"title"}` — a top-level frontmatter key; the node is the full
    /// key line (the frontmatter plane is nodes, never ref grammar — §2.1).
    FmKey(String),
}

/// A mint-plane anchor id outside the one block-id charset (`[A-Za-z0-9-]`,
/// decision 011 / contract §2.4) — e.g. a legacy `_`-bearing id. `model` is
/// wire-blind, so this is the model-side refusal; the dispatch boundary maps it
/// to the wire `bad_request` (§2.4). The walk plane does NOT use this — it
/// follows the app (the pack-pinned answer), where the same id is silently
/// dropped (decision 013 consequence 3: both dispositions conform).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadAnchorId {
    pub id: String,
}

impl Ref {
    /// Build a mint-plane anchor ref, enforcing the one block-id charset — the
    /// mint-guard (CHARSET-GUARD). Every mint position that constructs an anchor
    /// ref from untrusted input (dispatch strict-decode, splice targets, receipt
    /// anchors) routes through here rather than `Ref::Anchor` directly.
    ///
    /// # Errors
    /// [`BadAnchorId`] when `id` is empty or bears a char outside `[A-Za-z0-9-]`
    /// (the dead `_` superset is refused here → `bad_request`).
    pub fn anchor(id: impl Into<String>) -> Result<Self, BadAnchorId> {
        let id = id.into();
        if syntax::is_block_id(&id) {
            Ok(Self::Anchor(id))
        } else {
            Err(BadAnchorId { id })
        }
    }
}

/// A resolved splice target: the section (heading line through end of subtree)
/// or the anchor's host block, plus its CAS token.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    pub span: ByteSpan,
    pub node_rev: NodeRev,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResolveError {
    NotFound,
    /// Duplicate hpaths — carries the candidate list (meridian's duplicate-hpath
    /// candidate-list error, preserved verbatim; wire code reserved, undecided).
    Ambiguous(Vec<Target>),
}

/// The strict mint-plane lookup (contract §2.1) backing `cat`/`toc`/splice
/// targets: an `hpath` walked byte-exactly down the containment tree, an exact
/// block `anchor`, or a top-level `fm_key`. Section span = heading line through
/// end of subtree (what makes the vision's splice example coherent). Never
/// case-folds, never silently picks — that is the [`walk`] plane's job.
///
/// # Errors
/// [`ResolveError::NotFound`] for a missing (or empty) ref; [`ResolveError::Ambiguous`]
/// with the candidate list when a duplicate is not disambiguated (an `hpath`
/// segment with no occurrence index, or a duplicate anchor id) → `ambiguous_ref`.
pub fn resolve(doc: &Document, r#ref: &Ref) -> Result<Target, ResolveError> {
    match r#ref {
        Ref::Hpath(segs) => resolve_hpath(&doc.root, segs),
        Ref::Anchor(id) => resolve_anchor(doc, id),
        Ref::FmKey(key) => resolve_fm_key(doc, key),
    }
}

/// A node's splice target — span + CAS token.
fn target_of(node: &Node) -> Target {
    Target {
        span: node.span.clone(),
        node_rev: node.node_rev.clone(),
    }
}

/// Descend the containment tree one hpath segment at a time, byte-exact on each
/// heading text. A segment matching multiple sibling sections resolves by its
/// 1-based occurrence `n`; without `n`, a duplicate is `Ambiguous` (loud).
fn resolve_hpath(root: &Node, segs: &[HpathSeg]) -> Result<Target, ResolveError> {
    if segs.is_empty() {
        return Err(ResolveError::NotFound);
    }
    let mut current = root;
    for seg in segs {
        let matches: Vec<&Node> = current
            .children
            .iter()
            .filter(|c| matches!(&c.kind, NodeKind::Section { heading_text, .. } if *heading_text == seg.h))
            .collect();
        current = match (matches.as_slice(), seg.n) {
            ([], _) => return Err(ResolveError::NotFound),
            (_, Some(n)) => {
                let idx = usize::try_from(n).unwrap_or(usize::MAX);
                match idx.checked_sub(1).and_then(|i| matches.get(i)) {
                    Some(node) => node,
                    None => return Err(ResolveError::NotFound),
                }
            }
            ([only], None) => only,
            (many, None) => {
                return Err(ResolveError::Ambiguous(
                    many.iter().map(|n| target_of(n)).collect(),
                ));
            }
        };
    }
    Ok(target_of(current))
}

/// Exact block-id lookup over the tree's anchor nodes. A duplicate id in one
/// file is `Ambiguous` (loud) — the mint plane never silently picks; the walk
/// plane follows the app instead (last-wins, silent — [`walk`]).
fn resolve_anchor(doc: &Document, id: &str) -> Result<Target, ResolveError> {
    let mut hits: Vec<&Node> = Vec::new();
    collect_anchors(&doc.root, id, &mut hits);
    hits.sort_by_key(|n| n.span.start);
    match hits.as_slice() {
        [] => Err(ResolveError::NotFound),
        [only] => Ok(target_of(only)),
        many => Err(ResolveError::Ambiguous(
            many.iter().map(|n| target_of(n)).collect(),
        )),
    }
}

/// Anchor nodes whose id matches `id` byte-exactly (mint plane), document order.
fn collect_anchors<'a>(node: &'a Node, id: &str, hits: &mut Vec<&'a Node>) {
    if matches!(&node.kind, NodeKind::Anchor { name } if name == id) {
        hits.push(node);
    }
    for c in &node.children {
        collect_anchors(c, id, hits);
    }
}

/// A top-level frontmatter key → the full key line (span + rev). The node the
/// contract names is the key line inside the frontmatter block, not the whole
/// block.
fn resolve_fm_key(doc: &Document, key: &str) -> Result<Target, ResolveError> {
    let Some(fm) = find_frontmatter(&doc.root) else {
        return Err(ResolveError::NotFound);
    };
    let bytes = doc.raw.as_bytes();
    let block = &fm.span;
    let mut line_start = block.start;
    while line_start < block.end {
        let line_end = bytes[line_start..block.end]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(block.end, |p| line_start + p + 1);
        let line = &doc.raw[line_start..line_end];
        // top-level keys sit at column 0 (the fm-parse convention); match the
        // key byte-exactly up to its colon.
        if !line.starts_with([' ', '\t'])
            && line
                .split_once(':')
                .is_some_and(|(k, _)| k.trim().trim_matches(['"', '\'']) == key)
        {
            let span = line_start..line_end;
            return Ok(Target {
                node_rev: node_rev(bytes, &span),
                span,
            });
        }
        line_start = line_end;
    }
    Err(ResolveError::NotFound)
}

/// The document's frontmatter node, if any.
fn find_frontmatter(node: &Node) -> Option<&Node> {
    if matches!(node.kind, NodeKind::Frontmatter { .. }) {
        return Some(node);
    }
    node.children.iter().find_map(find_frontmatter)
}

// ---------------------------------------------------------------------------
// CAS-splice validation (rung 2) — validation here, execution in `fs`
// ---------------------------------------------------------------------------

/// A proposed node-level CAS write: replace exactly `span`, guarded by
/// `if_node_rev`.
#[derive(Debug, Clone, PartialEq)]
pub struct SpliceRequest {
    pub span: ByteSpan,
    pub if_node_rev: NodeRev,
    pub text: String,
}

/// Validation verdict. `Validated` is the capability token `fs` demands for
/// execution — an unvalidated splice cannot reach disk by construction.
#[derive(Debug, Clone, PartialEq)]
pub enum SpliceVerdict {
    Validated(ValidatedSplice),
    /// The retryable one: re-resolve, re-derive, splice again.
    CasMismatch {
        expected: NodeRev,
        actual: NodeRev,
    },
}

/// A splice that passed CAS validation against a live `Document`. Only `model`
/// can mint one (private field), and `fs::apply_splice` only accepts one.
#[expect(
    clippy::manual_non_exhaustive,
    reason = "_sealed is a capability seal (only model mints), not future-proofing"
)]
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedSplice {
    pub span: ByteSpan,
    pub text: String,
    _sealed: (),
}

/// Validate a splice against the current tree: span integrity + rev ladder
/// (meridian I1/I2 anchor resolution + fresh/stale/omitted semantics relocate
/// here as the validation core).
#[must_use]
pub fn validate_splice(doc: &Document, req: &SpliceRequest) -> SpliceVerdict {
    let _ = (doc, req);
    todo!("rung 2: CAS validation + rev ladder")
}

// ---------------------------------------------------------------------------
// integrity (rung 3) + corpus index (rung 5 borrow surface)
// ---------------------------------------------------------------------------

/// Current root over one document (rung 3); corpus root composes over these.
#[must_use]
pub fn merkle_root(doc: &Document) -> MerkleRoot {
    let _ = doc;
    todo!("rung 3: per-node hash fold")
}

/// The resident corpus name index (rung 4+ daemon state). Model state —
/// derived, disposable (law 2). `query` and `policy` borrow it as a capability
/// parameter (`Option<&CorpusIndex>`); neither owns it, and there is no
/// policy→query dependency — both are siblings over model.
///
/// It houses the vault name/alias index the walk plane's stage 1 needs
/// (`getFirstLinkpathDest` parity, contract §4.5): file basename → paths and
/// frontmatter alias → paths, both lowercased (stage 1 is case-insensitive).
#[derive(Debug, Default)]
pub struct CorpusIndex {
    by_basename: BTreeMap<String, Vec<String>>,
    by_alias: BTreeMap<String, Vec<String>>,
}

impl CorpusIndex {
    /// An empty index.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Index one document under its vault `path`: its file basename and every
    /// frontmatter alias, both lowercased (stage-1 case-insensitivity, §4.5).
    pub fn insert(&mut self, path: &str, doc: &Document) {
        push_unique(self.by_basename.entry(basename_lc(path)).or_default(), path);
        for alias in doc_aliases(doc) {
            push_unique(self.by_alias.entry(alias).or_default(), path);
        }
    }

    /// Stage 1 — `getFirstLinkpathDest(linkpath, from)` parity: resolve a
    /// linkpath (a basename, optionally with subdirs and/or a `.md` suffix) to a
    /// vault path, case-insensitively, preferring the source-relative
    /// shortest-unambiguous match, frontmatter aliases included. An unresolved
    /// linkpath returns `None` — unresolved is first-class (§4.5).
    ///
    /// The multi-candidate tie-break is a §13.4 pack-pinned unknown (harness
    /// probe WX-3): this records a deterministic, source-relative-then-shortest
    /// pick — it is never asserted against an assumed oracle answer.
    #[must_use]
    pub fn resolve_linkpath(&self, linkpath: &str, from: &str) -> Option<String> {
        let key = linkpath.trim().trim_end_matches(".md").to_lowercase();
        let base = key.rsplit('/').next().unwrap_or(key.as_str()).to_string();
        let candidates = self
            .by_basename
            .get(&base)
            .or_else(|| self.by_alias.get(&key))?;
        pick_source_relative(candidates, from)
    }
}

/// Lowercased file basename without its `.md` suffix — the stage-1 index key.
fn basename_lc(path: &str) -> String {
    path.rsplit('/')
        .next()
        .unwrap_or(path)
        .trim_end_matches(".md")
        .to_lowercase()
}

/// Push `value` only if absent (the index is a set per key).
fn push_unique(bucket: &mut Vec<String>, value: &str) {
    if !bucket.iter().any(|v| v == value) {
        bucket.push(value.to_string());
    }
}

/// Frontmatter aliases (lowercased) for stage-1 alias resolution. The flat
/// frontmatter parse keeps the `aliases` value as one string; parse the inline
/// list `[a, b]` (or a bare single value) — the corpus alias forms.
fn doc_aliases(doc: &Document) -> Vec<String> {
    let Some(fm) = find_frontmatter(&doc.root) else {
        return Vec::new();
    };
    let NodeKind::Frontmatter { map } = &fm.kind else {
        return Vec::new();
    };
    map.0
        .iter()
        .find(|(k, _)| k == "aliases" || k == "alias")
        .map(|(_, v)| parse_alias_list(v))
        .unwrap_or_default()
}

/// Parse an inline frontmatter alias value: `[a, b]`, `a, b`, or a bare `a`.
fn parse_alias_list(value: &str) -> Vec<String> {
    value
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|s| s.trim().trim_matches(['"', '\'']).to_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Source-relative shortest-unambiguous pick over stage-1 candidates: prefer a
/// match in the source's own directory, then the shortest path, then the
/// lexicographically first — deterministic (see `resolve_linkpath`'s WX-3 note).
fn pick_source_relative(candidates: &[String], from: &str) -> Option<String> {
    let from_dir = from.rsplit_once('/').map(|(dir, _)| dir);
    candidates
        .iter()
        .min_by(|a, b| {
            let a_same = from_dir.is_some_and(|d| a.starts_with(d));
            let b_same = from_dir.is_some_and(|d| b.starts_with(d));
            b_same
                .cmp(&a_same)
                .then(a.len().cmp(&b.len()))
                .then(a.cmp(b))
        })
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The §0.3 fixture `notes/plan.md` at S0 (136 bytes, LF, trailing newline).
    const PLAN_S0: &str = "---\ntitle: Plan\n---\n# Goals\n\nShip the contract.\n\n## Q3\n\nship by August\n\n## Q4\n\n- item one\n- see [[2026-07-18]]\n- blocked on [[roadmap]]\n";

    fn build_plan() -> Document {
        let raw = PLAN_S0.to_string();
        build(raw.clone(), syntax::parse(&raw))
    }

    fn find<'a>(node: &'a Node, pred: &dyn Fn(&Node) -> bool) -> Option<&'a Node> {
        if pred(node) {
            return Some(node);
        }
        node.children.iter().find_map(|c| find(c, pred))
    }

    #[test]
    fn frontmatter_node_is_fence_to_fence_terminator_inclusive() {
        // §18 row 3: the frontmatter node is span-lawed with the section
        // (newline-inclusive) family — [0,20], NOT syntax's trimmed [0,19).
        assert_eq!(PLAN_S0.len(), 136);
        let doc = build_plan();
        let fm = find(&doc.root, &|n| {
            matches!(n.kind, NodeKind::Frontmatter { .. })
        })
        .expect("frontmatter node");
        assert_eq!(fm.span, 0..20, "fence-to-fence, terminator-inclusive");
        assert_eq!(&doc.raw[fm.span.clone()], "---\ntitle: Plan\n---\n");
        assert_eq!(fm.node_rev.0, "26796ebec5d0bf1a");
        let NodeKind::Frontmatter { map } = &fm.kind else {
            unreachable!()
        };
        assert_eq!(
            map.0
                .iter()
                .find_map(|(k, v)| (k == "title").then_some(v.as_str())),
            Some("Plan")
        );
    }

    #[test]
    fn root_node_rev_equals_file_rev_over_whole_file_bytes() {
        // file_rev is DEFINED over whole-file bytes, independent of tree shape;
        // the root node's span is the whole file, so the identity must hold.
        let doc = build_plan();
        let independent = blake3::hash(doc.raw.as_bytes()).to_hex().as_str()[..16].to_string();
        assert_eq!(independent, "e3c4acaceb75b907", "oracle file_rev(plan_v0)");
        assert_eq!(doc.root.node_rev.0, independent);
        assert_eq!(doc.root.span, 0..136);
    }

    #[test]
    fn sections_nest_by_level_with_hpath_chains() {
        let doc = build_plan();
        let goals = find(&doc.root, &|n| {
            matches!(&n.kind, NodeKind::Section { heading_text, .. } if heading_text == "Goals")
        })
        .expect("Goals section");
        assert_eq!(goals.span, 20..136, "L1 section runs to EOF (Q3/Q4 deeper)");
        assert_eq!(goals.node_rev.0, "a6665baff294bd04");
        assert_eq!(
            goals.hpath.as_deref(),
            Some(["Goals".to_string()].as_slice())
        );
        // Q3 and Q4 are children of Goals (level nesting), not siblings.
        let q3 = goals
            .children
            .iter()
            .find(|n| matches!(&n.kind, NodeKind::Section { heading_text, .. } if heading_text == "Q3"))
            .expect("Q3 under Goals");
        assert_eq!(q3.span, 49..72);
        assert_eq!(q3.node_rev.0, "33d5b0e1b27cb48b");
        assert_eq!(
            q3.hpath.as_deref(),
            Some(["Goals".to_string(), "Q3".to_string()].as_slice())
        );
    }

    #[test]
    fn ref_anchor_guard_refuses_underscore_ids() {
        // CHARSET-GUARD: the mint-plane anchor constructor enforces the one
        // block-id charset (ruling 011 / contract §2.4). Clean ids mint; a
        // `_`-bearing (or otherwise out-of-charset) id is refused, carrying the
        // offending lexeme for the wire `bad_request` frame downstream.
        assert_eq!(Ref::anchor("r-000042"), Ok(Ref::Anchor("r-000042".into())));
        assert_eq!(Ref::anchor("clean-1"), Ok(Ref::Anchor("clean-1".into())));
        assert_eq!(
            Ref::anchor("under-probe_x"),
            Err(BadAnchorId {
                id: "under-probe_x".into()
            })
        );
        assert_eq!(Ref::anchor("a_04"), Err(BadAnchorId { id: "a_04".into() }));
        assert_eq!(Ref::anchor(""), Err(BadAnchorId { id: String::new() }));
    }

    fn seg(h: &str) -> HpathSeg {
        HpathSeg { h: h.to_string(), n: None }
    }

    fn seg_n(h: &str, n: u32) -> HpathSeg {
        HpathSeg { h: h.to_string(), n: Some(n) }
    }

    #[test]
    fn resolve_hpath_descends_containment_byte_exact() {
        let doc = build_plan();
        let goals = resolve(&doc, &Ref::Hpath(vec![seg("Goals")])).expect("Goals");
        assert_eq!(goals.span, 20..136);
        assert_eq!(goals.node_rev.0, "a6665baff294bd04");
        let q3 = resolve(&doc, &Ref::Hpath(vec![seg("Goals"), seg("Q3")])).expect("Q3");
        assert_eq!(q3.span, 49..72);
        assert_eq!(q3.node_rev.0, "33d5b0e1b27cb48b");
        // byte-exact: the mint plane never case-folds (that is the walk plane).
        assert_eq!(
            resolve(&doc, &Ref::Hpath(vec![seg("goals")])),
            Err(ResolveError::NotFound)
        );
        // a missing deeper segment, and the empty ref, both miss.
        assert_eq!(
            resolve(&doc, &Ref::Hpath(vec![seg("Goals"), seg("Q9")])),
            Err(ResolveError::NotFound)
        );
        assert_eq!(
            resolve(&doc, &Ref::Hpath(vec![])),
            Err(ResolveError::NotFound)
        );
    }

    #[test]
    fn resolve_fm_key_targets_the_key_line() {
        let doc = build_plan();
        let title = resolve(&doc, &Ref::FmKey("title".to_string())).expect("title fm_key");
        assert_eq!(title.span, 4..16);
        assert_eq!(&doc.raw[title.span.clone()], "title: Plan\n");
        let independent = blake3::hash(&doc.raw.as_bytes()[4..16]).to_hex().as_str()[..16].to_string();
        assert_eq!(title.node_rev.0, independent);
        assert_eq!(
            resolve(&doc, &Ref::FmKey("nope".to_string())),
            Err(ResolveError::NotFound)
        );
    }

    #[test]
    fn resolve_hpath_duplicate_siblings_ambiguous_unless_occurrence_given() {
        // Two identical `## Beta` siblings under `# A`: the mint plane refuses to
        // silently pick (contract §2.1) — Ambiguous unless an occurrence `n`
        // disambiguates (1-based, document order).
        let raw = "# A\n\n## Beta\n\nfirst\n\n## Beta\n\nsecond\n".to_string();
        let doc = build(raw.clone(), syntax::parse(&raw));
        let ResolveError::Ambiguous(cands) =
            resolve(&doc, &Ref::Hpath(vec![seg("A"), seg("Beta")])).expect_err("duplicate is ambiguous")
        else {
            panic!("duplicate sibling hpath must resolve Ambiguous");
        };
        assert_eq!(cands.len(), 2);
        let first = resolve(&doc, &Ref::Hpath(vec![seg("A"), seg_n("Beta", 1)])).expect("Beta#1");
        let second = resolve(&doc, &Ref::Hpath(vec![seg("A"), seg_n("Beta", 2)])).expect("Beta#2");
        assert!(first.span.start < second.span.start, "n follows document order");
        assert_eq!(cands[0].span, first.span);
        assert_eq!(cands[1].span, second.span);
        assert_eq!(
            resolve(&doc, &Ref::Hpath(vec![seg("A"), seg_n("Beta", 3)])),
            Err(ResolveError::NotFound),
            "occurrence past the last match misses"
        );
    }

    #[test]
    fn resolve_anchor_duplicate_is_ambiguous_single_is_target() {
        let raw = "para one ^dup1\n\npara two ^dup1\n\nlone ^solo\n".to_string();
        let doc = build(raw.clone(), syntax::parse(&raw));
        let ResolveError::Ambiguous(cands) =
            resolve(&doc, &Ref::anchor("dup1").unwrap()).expect_err("dup anchor is ambiguous")
        else {
            panic!("duplicate anchor must resolve Ambiguous");
        };
        assert_eq!(cands.len(), 2);
        assert!(cands[0].span.start < cands[1].span.start, "candidates in document order");
        let solo = resolve(&doc, &Ref::anchor("solo").unwrap()).expect("solo anchor");
        assert!(!solo.node_rev.0.is_empty());
        assert_eq!(
            resolve(&doc, &Ref::anchor("absent").unwrap()),
            Err(ResolveError::NotFound)
        );
    }
}
