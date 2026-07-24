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

pub mod delta;
pub mod fingerprint;
pub mod selector;
pub mod walk;

/// Half-open byte range into a file's raw bytes. Distinct from the wire's
/// serializable span on purpose — converting between them is `sidecar`'s job.
pub type ByteSpan = Range<usize>;

/// The `hash-algo` label this engine mints and computes — `blake3-256(span
/// bytes)[:16]` (contract v2 §1; node-rev-merkle-spec §1). An `^inputs` lock
/// whose `hash-algo` header is anything else carries a rev this engine cannot
/// recompute, so the read face renders it grey `superseded-algo` — never green,
/// never red (wire-contract-v2-colors-amendment § Colors; U0.2/U3.4). One owner
/// for the label so `pin` (writer) and `view` (reader) never drift.
pub const NODE_REV_ALGO: &str = "node-rev";

/// The effect-page `hash-algo` label the v1→v2 supersede stamps (design-2 §6.3:
/// "`hash-algo: v2` = merkle-v1's composition law verbatim, two swaps: sha256 →
/// blake3; norm-v1 → raw bytes"). For a whole-page ref that swap yields exactly
/// this engine's `node_rev` (`blake3(raw bytes)[:16]`), so a `v2` pin is
/// verified through the SAME node-rev compare as a native `node-rev` pin — the
/// supersede keeps the value and re-labels it under the effect-page `vN`
/// data-contract convention (SCHEMA.md). One owner for the label.
pub const SUPERSEDE_ALGO_V2: &str = "v2";

/// Whether `algo` is an engine-native `hash-algo` — one whose rev this engine's
/// node-rev compare verifies to green/red, NOT grey `superseded-algo`. The
/// native set is `{node-rev, v2}`: `node-rev` is what `pin` mints; `v2` is the
/// design-2 §6.3 supersede label carrying the same node-rev value under the
/// effect-page `vN` convention. Any other label (v1/merkle-v1, statusd-file-rev,
/// read-rev, …) is a rev this engine cannot recompute → grey.
#[must_use]
pub fn is_native_algo(algo: &str) -> bool {
    algo == NODE_REV_ALGO || algo == SUPERSEDE_ALGO_V2
}

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
                    // An anchor's model node carries its HOST BLOCK-LEAF span, not
                    // the inline `^id` marker span `syntax` emits (§2.1 write-target
                    // / §4.1 / §6.3): mint `resolve_anchor` + walk `block_span` read
                    // block grain from one node. Every other leaf keeps its span.
                    let span = match kind {
                        NodeKind::Anchor { .. } => anchor_host_span(raw.as_bytes(), &span),
                        _ => span,
                    };
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

/// The host block-leaf span for an anchor whose inline `^id` marker occupies
/// `marker`: the marker's own line, start-of-line → line end with the final
/// terminator excluded (contract §1 leaf-block law; §2.1 write-target; the §4.1 /
/// §6.3 worked `^r-000042` → `[26,248]`). `syntax` emits the inline marker span
/// (delimiter-included — a legitimate §1 inline fact, untouched); the model
/// write-target is the host block the anchor keys, so mint `resolve_anchor` and
/// walk `block_span` read block grain from one node, no per-consumer derivation.
///
/// A heading-line anchor keys the heading LINE leaf (terminator excluded),
/// distinct from the newline-inclusive heading SECTION span — a rule-derived
/// consequence of the same line rule + §1 leaf law, not new law.
fn anchor_host_span(bytes: &[u8], marker: &ByteSpan) -> ByteSpan {
    // start of the marker's line: the byte after the previous `\n`, or 0.
    let start = bytes[..marker.start]
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(0, |p| p + 1);
    // line end, final terminator excluded: the next `\n` at/after the marker end
    // (EOF if none), dropping a preceding `\r` of a CRLF pair.
    let mut end = bytes[marker.end..]
        .iter()
        .position(|&b| b == b'\n')
        .map_or(bytes.len(), |p| marker.end + p);
    if end > start && bytes[end - 1] == b'\r' {
        end -= 1;
    }
    start..end
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

/// Exclude a single trailing line terminator (`\n` or the full `\r\n` pair) —
/// the §1 leaf-block law (a leaf node excludes its final terminator). The exact
/// inverse of [`extend_terminator`]: `extend` grows the fence-to-fence container
/// over its terminator (§18 row 3), `trim` holds the `fm_key` leaf inside it at
/// terminator-excluded grain (`[4,15]` = `title: Plan`, §4.4). Full terminator,
/// never a naive `-1`: `\n` ⇒ 1, `\r\n` ⇒ 2, a terminator-less line ⇒ 0.
fn trim_terminator(bytes: &[u8], start: usize, mut end: usize) -> usize {
    if end > start && bytes[end - 1] == b'\n' {
        end -= 1;
        if end > start && bytes[end - 1] == b'\r' {
            end -= 1;
        }
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
    resolve_full(doc, r#ref).map(|r| Target {
        span: r.span,
        node_rev: r.node_rev,
    })
}

/// A fully resolved target: the full node span, its content span (heading
/// stripped for sections; == full span for headingless nodes), and the CAS
/// token. The content span backs `put{at:"content"}` (§4.4); the full span
/// backs `put{at:"all"}`, `put{at:"end"}` (at its end byte), and `match`
/// (search over the full span bytes). Internal to validation — the public
/// [`resolve`] projects it to [`Target`].
struct Resolved {
    span: ByteSpan,
    content_span: ByteSpan,
    node_rev: NodeRev,
}

/// Resolve a ref to its full validation surface (span + content span + rev).
fn resolve_full(doc: &Document, r#ref: &Ref) -> Result<Resolved, ResolveError> {
    let bytes = doc.raw.as_bytes();
    match r#ref {
        Ref::Hpath(segs) => resolve_hpath_node(&doc.root, segs).map(|n| resolved_of(n, bytes)),
        Ref::Anchor(id) => resolve_anchor_node(doc, id).map(|n| resolved_of(n, bytes)),
        Ref::FmKey(key) => resolve_fm_key_resolved(doc, key),
    }
}

/// A node's full validation surface.
fn resolved_of(node: &Node, bytes: &[u8]) -> Resolved {
    Resolved {
        content_span: content_span(node, bytes),
        span: node.span.clone(),
        node_rev: node.node_rev.clone(),
    }
}

/// The content span of a node (contract §1 rev sub-laws / §4.4 `put{at:"content"}`):
/// for a `Section`, the bytes after the heading line's terminator to the section
/// end (heading preserved). Every other node has no heading to preserve, so its
/// content span IS its full span — rule-derived (`at:"content"` ≡ `at:"all"`
/// where "heading preserved" is vacuous; no bytes are lost either way).
fn content_span(node: &Node, bytes: &[u8]) -> ByteSpan {
    match &node.kind {
        NodeKind::Section { .. } => {
            let start = bytes[node.span.start..node.span.end]
                .iter()
                .position(|&b| b == b'\n')
                .map_or(node.span.end, |p| node.span.start + p + 1);
            start..node.span.end
        }
        _ => node.span.clone(),
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
fn resolve_hpath_node<'a>(root: &'a Node, segs: &[HpathSeg]) -> Result<&'a Node, ResolveError> {
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
    Ok(current)
}

/// Exact block-id lookup over the tree's anchor nodes. A duplicate id in one
/// file is `Ambiguous` (loud) — the mint plane never silently picks; the walk
/// plane follows the app instead (last-wins, silent — [`walk`]).
fn resolve_anchor_node<'a>(doc: &'a Document, id: &str) -> Result<&'a Node, ResolveError> {
    let mut hits: Vec<&Node> = Vec::new();
    collect_anchors(&doc.root, id, &mut hits);
    hits.sort_by_key(|n| n.span.start);
    match hits.as_slice() {
        [] => Err(ResolveError::NotFound),
        [only] => Ok(only),
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

/// A top-level frontmatter key → its full VALUE grain (span + rev). The node the
/// contract names is the key line PLUS any multi-line block value that hangs off
/// it (indented continuation lines), never the whole fence-to-fence block.
///
/// A scalar or flow value (`title: Plan`, `tags: [a, b]`) has no continuation,
/// so the grain is the single key line — the frozen §4.4 leaf (`[4,15]` =
/// `title: Plan`, §1 leaf law: terminator-excluded). A block value (`inputs:`
/// plus indented `- item` lines) extends the grain over every item, so an
/// upsert/replace addresses the WHOLE value and can never orphan the tail
/// (U2.11 — the line-oriented edit could formerly reach only the key line,
/// minting a rev over a silently corrupted block sequence). The frontmatter
/// CONTAINER node stays fence-to-fence, terminator-inclusive (`[0,20]`, §18 row
/// 3) and is minted elsewhere; nothing here touches it.
fn resolve_fm_key_resolved(doc: &Document, key: &str) -> Result<Resolved, ResolveError> {
    let Some(fm) = find_frontmatter(&doc.root) else {
        return Err(ResolveError::NotFound);
    };
    let bytes = doc.raw.as_bytes();
    let block = &fm.span;
    let mut line_start = block.start;
    while line_start < block.end {
        let line_end = fm_line_end(bytes, line_start, block.end);
        let line = &doc.raw[line_start..line_end];
        // top-level keys sit at column 0 (the fm-parse convention); match the
        // key byte-exactly up to its colon.
        if !line.starts_with([' ', '\t'])
            && line
                .split_once(':')
                .is_some_and(|(k, _)| k.trim().trim_matches(['"', '\'']) == key)
        {
            let span = fm_key_grain_span(bytes, line_start, block.end);
            return Ok(Resolved {
                node_rev: node_rev(bytes, &span),
                content_span: span.clone(),
                span,
            });
        }
        line_start = line_end;
    }
    Err(ResolveError::NotFound)
}

/// The end byte (terminator INCLUDED) of the frontmatter line starting at
/// `start`, clamped to the block end.
fn fm_line_end(bytes: &[u8], start: usize, block_end: usize) -> usize {
    bytes[start..block_end]
        .iter()
        .position(|&b| b == b'\n')
        .map_or(block_end, |p| start + p + 1)
}

/// A frontmatter line with no non-whitespace content (spaces/tabs then its
/// terminator).
fn fm_line_is_blank(bytes: &[u8], start: usize, end: usize) -> bool {
    bytes[start..end]
        .iter()
        .all(|&b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
}

/// The full grain span of a top-level frontmatter key whose LINE starts at
/// `key_line_start`: the key line, extended over every INDENTED continuation
/// line of a block value (block sequence or block mapping). A blank line is
/// carried only when a later indented line follows — trailing blanks belong to
/// the inter-key gap, not the value. The scan stops at the next column-0
/// non-blank line (the next key or the closing fence) or the block end. The
/// returned end EXCLUDES the last content line's terminator (§1 leaf law), so a
/// single-line key keeps its frozen terminator-excluded grain.
fn fm_key_grain_span(bytes: &[u8], key_line_start: usize, block_end: usize) -> ByteSpan {
    let key_line_end = fm_line_end(bytes, key_line_start, block_end);
    let mut grain_end = trim_terminator(bytes, key_line_start, key_line_end);
    let mut cursor = key_line_end;
    while cursor < block_end {
        let line_end = fm_line_end(bytes, cursor, block_end);
        if fm_line_is_blank(bytes, cursor, line_end) {
            // tentative — a blank line joins the value only if a later indented
            // line extends the grain past it.
        } else if matches!(bytes.get(cursor), Some(b' ' | b'\t')) {
            // an indented continuation line: the block value extends over it
            // (and any blank lines skipped since the last content line, since
            // the grain is the contiguous `[key_line_start, grain_end)` region).
            grain_end = trim_terminator(bytes, cursor, line_end);
        } else {
            // column-0 non-blank: the next key or the closing fence — value ends.
            break;
        }
        cursor = line_end;
    }
    key_line_start..grain_end
}

/// The document's frontmatter node, if any.
fn find_frontmatter(node: &Node) -> Option<&Node> {
    if matches!(node.kind, NodeKind::Frontmatter { .. }) {
        return Some(node);
    }
    node.children.iter().find_map(find_frontmatter)
}

/// A planned `fm_key` upsert: the pre-batch target span (disjointness grain),
/// the byte region to replace (zero-width for a create), the composed
/// replacement text, and the honest before-rev the CAS guard compares.
struct FmUpsertPlan {
    target_span: ByteSpan,
    region: ByteSpan,
    text: String,
    before_rev: NodeRev,
}

/// Plan a frontmatter-key upsert against the PRE-batch document — the
/// create-or-replace site for `{key}: {value}` (`PutAt::Upsert`). The insertion
/// offset is SERVER-derived (never a client byte offset, D-C1):
/// - key present → replace its full line span; before-rev is the line's rev.
/// - key absent, frontmatter present → insert `{key}: {value}\n` right after the
///   opening `---\n` fence (first-key position); before-rev is the empty
///   insertion point's rev.
/// - no frontmatter → synthesize `---\n{key}: {value}\n---\n` at byte 0.
fn plan_fm_upsert(doc: &Document, key: &str, value: &str) -> FmUpsertPlan {
    let line = format!("{key}: {value}");
    // key present → replace the whole key line (== `at:all` on the fm_key leaf).
    if let Ok(existing) = resolve_fm_key_resolved(doc, key) {
        return FmUpsertPlan {
            target_span: existing.span.clone(),
            region: existing.span,
            text: line,
            before_rev: existing.node_rev,
        };
    }
    // A create has no prior node: the honest before-rev is the empty span's rev.
    let empty_rev = node_rev(doc.raw.as_bytes(), &(0..0));
    match find_frontmatter(&doc.root) {
        Some(fm) => {
            let at = fm_insert_offset(doc.raw.as_bytes(), &fm.span);
            FmUpsertPlan {
                target_span: at..at,
                region: at..at,
                text: format!("{line}\n"),
                before_rev: empty_rev,
            }
        }
        None => FmUpsertPlan {
            target_span: 0..0,
            region: 0..0,
            text: format!("---\n{line}\n---\n"),
            before_rev: empty_rev,
        },
    }
}

/// The byte offset just past a frontmatter block's opening `---\n` fence — where
/// a new key line inserts (first-key position, keeping the block well-formed).
/// The block always opens with `---` + a terminator (the syntax fm gate); a
/// terminator-less block (never emitted by the parser) falls back to the block
/// start.
fn fm_insert_offset(bytes: &[u8], block: &ByteSpan) -> usize {
    let mut i = block.start;
    while i < block.end && bytes[i] != b'\n' {
        i += 1;
    }
    if i < block.end { i + 1 } else { block.start }
}

/// The pre-batch armed BEFORE fact for an `fm_key` upsert target: the existing
/// key line's span + rev when present, else the empty insertion point's span +
/// the empty-span rev (`blake3("")[:16]` — a create has no prior node). The
/// dispatch write path uses this to arm `node_rev_before` without re-deriving the
/// synthesis site ([`plan_fm_upsert`]); `value` does not affect the before fact.
#[must_use]
pub fn fm_upsert_before(doc: &Document, key: &str) -> Target {
    let plan = plan_fm_upsert(doc, key, "");
    Target {
        span: plan.target_span,
        node_rev: plan.before_rev,
    }
}

// ---------------------------------------------------------------------------
// CAS-splice validation (rung 2) — validation here, execution in `fs`
// ---------------------------------------------------------------------------

/// Where a `put` writes within its resolved target (contract §4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PutAt {
    /// Replace the full span, heading included.
    All,
    /// Replace the content span, heading preserved (§1 content-span law; ==
    /// full span for a headingless target — see [`content_span`]).
    Content,
    /// Insert `text` at the span-end byte — the append verb: RAW byte
    /// concatenation, NO synthesized separator (§4.4 `at:"end"` law). `text`
    /// that must begin a new line carries its own leading `\n`; a result that
    /// loses containment refuses [`SpliceVerdict::WouldCorrupt`].
    End,
    /// Set a frontmatter key (create-or-replace) — valid ONLY on a
    /// [`Ref::FmKey`] target. `text` is the VALUE; the server composes
    /// `{key}: {value}` and either replaces the key's existing line, inserts a
    /// new line into the frontmatter block, or synthesizes a `---` block when
    /// the file has none (see [`plan_fm_upsert`]). The insertion offset is
    /// derived from the document — never client-supplied (D-C1).
    Upsert,
}

/// The two edit shapes (contract §4.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditKind {
    /// Edit-exact: `old` must occur exactly ONCE in the target's full span
    /// bytes; replaced by `new`. Zero → `no_match`, two+ → `not_unique{matches}`
    /// (§5.2). No regex, no fuzz; matched SERVER-side.
    Match { old: String, new: String },
    /// Whole-slot write at a [`PutAt`] position.
    Put { at: PutAt, text: String },
}

/// One edit in a batch: a target ref, the edit shape, an optional per-node CAS
/// guard. There is NO span field — a client cannot supply a byte offset, so the
/// wrong-offset write is UNREPRESENTABLE (D-C1, §4.4). The model twin of
/// `wire::Edit`; the crates never share a type (no-serde law).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub target: Ref,
    pub edit: EditKind,
    /// Node-grain guard (§5.1): compared against `blake3(target's full span
    /// bytes)[:16]` re-derived from the PRE-BATCH state. `None` = unguarded (a
    /// legal wire frame — requiredness is the Go ratchet, §5.3).
    pub if_node_rev: Option<NodeRev>,
}

/// A batch splice request (contract §4.4 — `splice` is batch-only, one response
/// shape). Every target and guard resolves against the PRE-BATCH state; targets
/// must be disjoint. There is NO span field anywhere request-side (D-C1). This
/// is the reshape of the v1 single `{span, if_node_rev, text}` splice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpliceRequest {
    /// World-grain guard, checked FIRST (§5.1) — a mismatch fails the WHOLE
    /// batch (`root_mismatch` → resync). `None` = unguarded.
    pub if_root: Option<MerkleRoot>,
    pub edits: Vec<Edit>,
}

/// A receipt append riding INSIDE the sealed batch (§6.1, D-C3): the receipt
/// file's append position (its EOF) and the pre-rendered line bytes. The bytes
/// are rendered by the `receipt` crate and folded in by the caller BEFORE
/// validation, so the append commits in the SAME batch and single root advance
/// as the content edits. Model seals it; it never renders (body formatting is
/// not this crate's, per charter).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptAppend {
    pub span: ByteSpan,
    pub text: String,
}

/// One validated edit: the exact PRE-BATCH byte span to replace and the
/// replacement text — the write instruction `fs` executes. Spans index the
/// pre-batch bytes; the batch's edits are disjoint (validated), so `fs` applies
/// them in one pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedEdit {
    pub span: ByteSpan,
    pub text: String,
}

/// The validation verdict for a batch (contract §5.2 failure split + §4.4 batch
/// laws). `Validated` is the capability token `fs` demands; every other variant
/// is a typed refusal the dispatch boundary maps to its wire error frame. Only
/// the first failing check (in validation order) is returned — matching the
/// single-error response shape of the §5.2 worked frames.
#[derive(Debug, Clone, PartialEq)]
pub enum SpliceVerdict {
    /// All guards passed; the sealed batch is ready for `fs`.
    Validated(ValidatedBatch),
    /// `if_root` failed (world-grain, checked FIRST) — the whole batch is
    /// refused, recovery `resync` (§5.1).
    RootMismatch {
        expected: MerkleRoot,
        actual: MerkleRoot,
    },
    /// A target ref did not resolve → `ref_not_found` (refresh, §4.5).
    RefNotFound,
    /// A target ref was ambiguous → `ambiguous_ref` (§2.1); the candidate
    /// targets ride along.
    Ambiguous(Vec<Target>),
    /// `if_node_rev` failed → `cas_mismatch` (refresh, §5.2). The retryable
    /// one: re-`cat`, re-derive, splice again.
    CasMismatch { expected: NodeRev, actual: NodeRev },
    /// Guard PASSED and `old` was absent → `no_match` (fix, §5.2): provably a
    /// typo. `matches` is 0 (carried for wire-frame parity).
    NoMatch { matches: usize },
    /// `old` occurred 2+ times → `not_unique{matches}` (fix, §5.2). Add context
    /// bytes to `old`, exactly as with the Edit tool.
    NotUnique { matches: usize },
    /// Two edits' targets are not disjoint → `bad_request{overlap}` (§4.4). The
    /// overlapping target spans ride along.
    Overlap { spans: Vec<ByteSpan> },
    /// The post-apply reparse loses containment → `would_corrupt{lost}` (§4.4).
    /// `lost` = the hpath chains of sections destroyed OUTSIDE the edited
    /// regions (rule-derived: a section byte-disjoint from every replaced region
    /// that no longer resolves after reparse was corrupted, not rewritten).
    WouldCorrupt { lost: Vec<Vec<String>> },
    /// A replaced region would split a multi-byte UTF-8 character →
    /// `bad_request` (§1 write-side multibyte refusal; the reparse-gate
    /// guarantor). Unreachable through the public `match`/`put` API — valid-UTF-8
    /// edits at char-aligned resolved spans are self-synchronizing, so a
    /// mid-char write is unrepresentable (parallel to D-C1) — the guard is
    /// retained so any mid-char region refuses LOUD rather than corrupting bytes.
    MultibyteSplit,
}

/// A batch that passed validation against a live `Document`. Only `model` can
/// mint one (the private `_sealed` field — external crates cannot name it, so
/// cannot construct the struct even though its data fields are public), and
/// `fs` accepts only one. An unvalidated batch cannot reach disk by
/// construction; the pipeline (validate here, execute in `fs`) is enforced by
/// types, not review. The receipt append rides INSIDE (§6.1).
///
/// The seal is a compile-level fact — no external crate can mint one:
///
/// ```compile_fail
/// // `_sealed` is private, so this fails to compile outside `model`.
/// let _ = model::ValidatedBatch { edits: Vec::new(), receipt: None };
/// ```
#[expect(
    clippy::manual_non_exhaustive,
    reason = "_sealed is a capability seal (only model mints), not future-proofing"
)]
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedBatch {
    /// The content edits, sorted by span start (disjoint — validated).
    pub edits: Vec<ValidatedEdit>,
    /// The receipt append riding inside the same sealed commit, if the request
    /// named a receipt (§6.1).
    pub receipt: Option<ReceiptAppend>,
    _sealed: (),
}

/// Validate a batch splice against a live `Document` (contract §4.4/§5). The
/// order (§5.1): `if_root` FIRST (world-grain, fails the whole batch), then per
/// edit resolve → CAS → match/put, then batch-wide disjointness and one
/// simulated reparse (`would_corrupt`). `live_root` is the caller's ambient
/// corpus root for the `if_root` comparison (`None` = the caller is not guarding
/// the world root — §5.3); `receipt` is a pre-rendered append the caller folds
/// in so it rides inside the sealed batch. On success, mints the sealed
/// [`ValidatedBatch`] — the only path to `fs`.
#[must_use]
pub fn validate_batch(
    doc: &Document,
    live_root: Option<&MerkleRoot>,
    batch: &SpliceRequest,
    receipt: Option<ReceiptAppend>,
) -> SpliceVerdict {
    // 1. World guard FIRST (§5.1): compared only when the client guarded AND the
    // caller supplied the live root; a mismatch fails the whole batch.
    if let (Some(expected), Some(actual)) = (&batch.if_root, live_root)
        && expected != actual
    {
        return SpliceVerdict::RootMismatch {
            expected: expected.clone(),
            actual: actual.clone(),
        };
    }

    let raw = &doc.raw;

    // 2. Per edit, in order: resolve → CAS → compute the replaced region. The
    // first failure (in edit order) is returned — the §5.2 single-error shape.
    let mut planned: Vec<PlannedEdit> = Vec::with_capacity(batch.edits.len());
    for edit in &batch.edits {
        // Upsert on an `fm_key` target is the ONE create-or-replace shape: the
        // key may not exist yet, so it plans BEFORE the resolve gate (which
        // would refuse a missing key `RefNotFound`). CAS compares the honest
        // before-rev (§5.1) — the existing line's rev, or the empty insertion
        // point's rev for a create. Non-`fm_key` upserts fall through to the
        // normal path (the dispatch write path refuses them first).
        if let (
            EditKind::Put {
                at: PutAt::Upsert,
                text: value,
            },
            Ref::FmKey(key),
        ) = (&edit.edit, &edit.target)
        {
            let plan = plan_fm_upsert(doc, key, value);
            if let Some(expected) = &edit.if_node_rev
                && *expected != plan.before_rev
            {
                return SpliceVerdict::CasMismatch {
                    expected: expected.clone(),
                    actual: plan.before_rev,
                };
            }
            if let Err(v) = guard_char_aligned(raw, &plan.region) {
                return v;
            }
            planned.push(PlannedEdit {
                target: plan.target_span,
                region: plan.region,
                text: plan.text,
            });
            continue;
        }
        let resolved = match resolve_full(doc, &edit.target) {
            Ok(r) => r,
            Err(ResolveError::NotFound) => return SpliceVerdict::RefNotFound,
            Err(ResolveError::Ambiguous(c)) => return SpliceVerdict::Ambiguous(c),
        };
        // CAS (§5.1): the ONE derivation — blake3 of the target's full span
        // bytes, already minted as `node_rev`.
        if let Some(expected) = &edit.if_node_rev
            && *expected != resolved.node_rev
        {
            return SpliceVerdict::CasMismatch {
                expected: expected.clone(),
                actual: resolved.node_rev.clone(),
            };
        }
        let (region, text) = match &edit.edit {
            EditKind::Match { old, new } => match match_region(raw, &resolved.span, old) {
                Ok(region) => (region, new.clone()),
                Err(v) => return v,
            },
            EditKind::Put { at, text } => {
                let region = match at {
                    // Upsert on a non-`fm_key` target degrades to `all` (the
                    // dispatch write path refuses it before it reaches here).
                    PutAt::All | PutAt::Upsert => resolved.span.clone(),
                    PutAt::Content => resolved.content_span.clone(),
                    PutAt::End => resolved.span.end..resolved.span.end,
                };
                (region, text.clone())
            }
        };
        // Write-side multibyte guarantor (§1): the replaced region must fall on
        // char boundaries. Char-aligned resolved spans + valid-UTF-8 edits make
        // this hold by construction; the guard refuses loud if it ever does not.
        if let Err(v) = guard_char_aligned(raw, &region) {
            return v;
        }
        planned.push(PlannedEdit {
            target: resolved.span,
            region,
            text,
        });
    }

    // 3. Disjointness (§4.4): targets must not overlap (containment counts).
    if let Some(spans) = first_overlap(&planned) {
        return SpliceVerdict::Overlap { spans };
    }

    // 4. One simulated reparse (§4.4): a post-apply parse that loses containment
    // refuses `would_corrupt`.
    if let Some(lost) = would_corrupt(doc, &planned) {
        return SpliceVerdict::WouldCorrupt { lost };
    }

    // Mint the sealed batch — edits in pre-batch offset order.
    let mut edits: Vec<ValidatedEdit> = planned
        .into_iter()
        .map(|p| ValidatedEdit {
            span: p.region,
            text: p.text,
        })
        .collect();
    edits.sort_by_key(|e| e.span.start);
    SpliceVerdict::Validated(ValidatedBatch {
        edits,
        receipt,
        _sealed: (),
    })
}

/// A resolved-and-planned edit: the full target span (for disjointness), the
/// replaced byte region, and the replacement text.
struct PlannedEdit {
    target: ByteSpan,
    region: ByteSpan,
    text: String,
}

/// Locate the exactly-one occurrence of `old` within the target's FULL span
/// bytes (§4.4), returning the absolute replaced region. `str`-level matching is
/// char-aligned by construction (a valid-UTF-8 needle in a valid-UTF-8 haystack
/// is self-synchronizing), so a byte count == char-aligned occurrence count.
///
/// # Errors
/// [`SpliceVerdict::NoMatch`] (count 0) or [`SpliceVerdict::NotUnique`] (2+).
fn match_region(raw: &str, span: &ByteSpan, old: &str) -> Result<ByteSpan, SpliceVerdict> {
    let hay = &raw[span.clone()];
    let mut hits = hay.match_indices(old);
    let Some((first, _)) = hits.next() else {
        return Err(SpliceVerdict::NoMatch { matches: 0 });
    };
    // Count the rest without re-scanning from zero: non-overlapping, left→right
    // (Edit-tool semantics), so 1 + the remaining tail.
    let count = 1 + hay[first + old.len()..].matches(old).count();
    if count > 1 {
        return Err(SpliceVerdict::NotUnique { matches: count });
    }
    let start = span.start + first;
    Ok(start..start + old.len())
}

/// The write-side multibyte guarantor (§1): both ends of a replaced region must
/// be UTF-8 char boundaries, else the splice would split a multi-byte character.
///
/// # Errors
/// [`SpliceVerdict::MultibyteSplit`] when either boundary lands mid-character.
fn guard_char_aligned(raw: &str, region: &ByteSpan) -> Result<(), SpliceVerdict> {
    if raw.is_char_boundary(region.start) && raw.is_char_boundary(region.end) {
        Ok(())
    } else {
        Err(SpliceVerdict::MultibyteSplit)
    }
}

/// The first pair of non-disjoint TARGET spans (§4.4), or `None`. Containment
/// counts as overlap (a section and a section it contains are not disjoint).
/// Touching boundaries (`[a,b)` then `[b,c)`) are disjoint.
fn first_overlap(planned: &[PlannedEdit]) -> Option<Vec<ByteSpan>> {
    let mut idx: Vec<usize> = (0..planned.len()).collect();
    idx.sort_by_key(|&i| planned[i].target.start);
    for w in idx.windows(2) {
        let (a, b) = (&planned[w[0]].target, &planned[w[1]].target);
        // sorted by start; a nests-or-precedes b — overlap iff a extends past
        // b's start.
        if a.end > b.start {
            return Some(vec![a.clone(), b.clone()]);
        }
    }
    None
}

/// Apply the planned edits to the raw bytes and reparse; report the hpath chains
/// of any pre-batch section that was byte-DISJOINT from every replaced region
/// yet no longer resolves — containment lost (§4.4 `would_corrupt`). A section
/// inside an edited region is legitimately rewritten (not "lost"); one outside
/// every edit whose heading was destroyed by a neighbouring edit bleeding into
/// it (e.g. a separator-less `at:"end"`) is corruption. Returns `None` when
/// containment holds.
fn would_corrupt(doc: &Document, planned: &[PlannedEdit]) -> Option<Vec<Vec<String>>> {
    let new_raw = apply_regions(&doc.raw, planned);
    let new_doc = build(new_raw.clone(), syntax::parse(&new_raw));
    let mut lost: Vec<Vec<String>> = Vec::new();
    collect_lost(&doc.root, planned, &new_doc, &mut lost);
    if lost.is_empty() { None } else { Some(lost) }
}

/// Recurse the pre-batch tree; for each `Section` byte-disjoint from every
/// replaced region, require its hpath to still resolve post-reparse.
fn collect_lost(
    node: &Node,
    planned: &[PlannedEdit],
    new_doc: &Document,
    lost: &mut Vec<Vec<String>>,
) {
    if let NodeKind::Section { .. } = &node.kind {
        let disjoint = planned
            .iter()
            .all(|p| region_disjoint(&node.span, &p.region));
        if disjoint && let Some(hpath) = &node.hpath {
            let segs: Vec<HpathSeg> = hpath
                .iter()
                .map(|h| HpathSeg {
                    h: h.clone(),
                    n: None,
                })
                .collect();
            // Containment is LOST only when the section vanished entirely
            // (`NotFound`). A pre-existing OR newly-created duplicate heading
            // resolves `Ambiguous` — the section still exists, merely un-unique
            // — so an unrelated byte-disjoint edit to such a file is NOT
            // corruption (the F6 refuse-all death mode: a stray duplicate must
            // not poison every write to the file; ambiguity surfaces only when a
            // caller ADDRESSES the duplicate, refuse-ambiguous-only, U2.2).
            if matches!(
                resolve_full(new_doc, &Ref::Hpath(segs)),
                Err(ResolveError::NotFound)
            ) {
                lost.push(hpath.clone());
            }
        }
    }
    for c in &node.children {
        collect_lost(c, planned, new_doc, lost);
    }
}

/// Two byte ranges are disjoint (touching boundaries and zero-width inserts at a
/// boundary count as disjoint).
fn region_disjoint(a: &ByteSpan, b: &ByteSpan) -> bool {
    a.end <= b.start || b.end <= a.start
}

/// Rebuild the raw string with each planned region replaced by its text
/// (regions are disjoint — validated), copying the unedited gaps verbatim.
fn apply_regions(raw: &str, planned: &[PlannedEdit]) -> String {
    let mut regions: Vec<&PlannedEdit> = planned.iter().collect();
    regions.sort_by_key(|p| p.region.start);
    let mut out = String::with_capacity(raw.len());
    let mut cursor = 0;
    for p in regions {
        out.push_str(&raw[cursor..p.region.start]);
        out.push_str(&p.text);
        cursor = p.region.end;
    }
    out.push_str(&raw[cursor..]);
    out
}

// ---------------------------------------------------------------------------
// integrity (rung 3) + corpus index (rung 5 borrow surface)
// ---------------------------------------------------------------------------

/// The corpus Merkle root over the hash domain (contract §12.2), minted as the
/// prefixed `Root` token (§12.3). `files` is the domain-filtered set — each
/// entry a vault-relative path and the file's **raw bytes**. The leaf hashes
/// raw bytes, so the parse tree plays no part in the root; membership (which
/// files are in the domain) is `fs`'s call — F3-DOMAIN's `Domain::contains` —
/// and this crate hashes exactly the set it is handed.
///
/// `version` is the domain's prefix counter (`Domain::version()`, riding
/// `mdfs_config.yaml`, §12.3): `0` ⇒ `b3:`, `1` ⇒ `b3a:`, … A domain-rule
/// change bumps it so a `b3:` cursor can never silently match a `b3a:` world —
/// the token carries the prefix even when the hex repeats. Taking the version
/// as a plain integer keeps `model` blind to `fs` (no dependency inversion:
/// the caller reads `Domain::version()` and passes the number).
///
/// Encoding (§12.2, verbatim): leaf = `blake3(raw file)` full 32 B; interior =
/// `blake3` over children sorted by raw name bytes, each
/// `uleb128(len(name)) ‖ name ‖ type_byte (0x00 file / 0x01 dir) ‖ hash32`;
/// empty dirs pruned; the workspace root's own name is never hashed.
#[must_use]
pub fn merkle_root(files: &[(&str, &[u8])], version: u32) -> MerkleRoot {
    let mut tree = MerkleDir::default();
    for (path, bytes) in files {
        tree.insert(path, bytes);
    }
    let hex = blake3::Hash::from_bytes(tree.fold()).to_hex().to_string();
    MerkleRoot(format!("{}{hex}", root_prefix(version)))
}

/// A directory in the merkle tree — named entries, each a file (raw bytes,
/// leaf-hashed on fold) or a subdirectory. Built from file paths alone, so a
/// directory carries ≥1 descendant unless explicitly emptied.
#[derive(Default)]
struct MerkleDir {
    entries: BTreeMap<String, MerkleEntry>,
}

enum MerkleEntry {
    File(Vec<u8>),
    Dir(MerkleDir),
}

impl MerkleDir {
    /// Insert one file at its `/`-split path (last write wins on a duplicate
    /// path, matching the oracle's dict). Empty segments are dropped; a
    /// path that collides with an existing file prefix is ignored (a real hash
    /// domain never mixes a file and a directory at one name).
    fn insert(&mut self, path: &str, bytes: &[u8]) {
        let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let Some((file_name, dirs)) = segs.split_last() else {
            return;
        };
        let mut dir = self;
        for seg in dirs {
            let entry = dir
                .entries
                .entry((*seg).to_string())
                .or_insert_with(|| MerkleEntry::Dir(MerkleDir::default()));
            match entry {
                MerkleEntry::Dir(sub) => dir = sub,
                MerkleEntry::File(_) => return,
            }
        }
        dir.entries
            .insert((*file_name).to_string(), MerkleEntry::File(bytes.to_vec()));
    }

    /// Fold this directory to its 32-byte node hash (§12.2): children ordered by
    /// raw name bytes, encoded `uleb128(len) ‖ name ‖ type ‖ hash32`, then
    /// `blake3` of the buffer. Empty subdirs contribute nothing.
    fn fold(&self) -> [u8; 32] {
        let mut children: Vec<(&str, bool, [u8; 32])> = Vec::new();
        for (name, entry) in &self.entries {
            match entry {
                MerkleEntry::File(bytes) => {
                    children.push((name, false, *blake3::hash(bytes).as_bytes()));
                }
                MerkleEntry::Dir(dir) if !dir.entries.is_empty() => {
                    children.push((name, true, dir.fold()));
                }
                MerkleEntry::Dir(_) => {} // empty dir pruned (§12.2)
            }
        }
        children.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        let mut enc: Vec<u8> = Vec::new();
        for (name, is_dir, hash) in children {
            let name = name.as_bytes();
            write_uleb128(&mut enc, name.len());
            enc.extend_from_slice(name);
            enc.push(u8::from(is_dir));
            enc.extend_from_slice(&hash);
        }
        *blake3::hash(&enc).as_bytes()
    }
}

/// Unsigned LEB128 (the §12.2 varint): low 7 bits per byte, high bit = "more".
/// A value below 128 emits a single byte — the fixture case the contract notes.
fn write_uleb128(out: &mut Vec<u8>, mut value: usize) {
    loop {
        let byte = u8::try_from(value & 0x7f).unwrap_or(0);
        value >>= 7;
        if value == 0 {
            out.push(byte);
            break;
        }
        out.push(byte | 0x80);
    }
}

/// The §12.3 `Root`-token prefix for a domain `version`: `0` ⇒ `b3:`,
/// `1` ⇒ `b3a:`, `26` ⇒ `b3z:`, `27` ⇒ `b3aa:` — `b3` followed by a bijective
/// base-26 suffix (`a` = 1, no zero digit), advancing on each domain-rule
/// change so cross-domain roots never collide in the token space.
fn root_prefix(version: u32) -> String {
    let mut n = version;
    let mut suffix = String::new();
    while n > 0 {
        n -= 1;
        suffix.push(char::from(b'a' + u8::try_from(n % 26).unwrap_or(0)));
        n /= 26;
    }
    let suffix: String = suffix.chars().rev().collect();
    format!("b3{suffix}:")
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
        // A linkpath WITH subdirs (`a/b`) addresses a path ENDING in `a/b.md`, not
        // just any file named `b` — getFirstLinkpathDest matches the fuller path
        // (§4.5). Narrow the basename candidates to those whose path carries the
        // whole subpath suffix; only fall back to the bare-basename set when the
        // qualifier matches nothing (a stale subpath still resolves best-effort).
        // Without this, `[[a/b]]` collides with every other `b.md` in the vault.
        if key.contains('/') {
            let suffix = format!("/{key}.md");
            let narrowed: Vec<String> = candidates
                .iter()
                .filter(|p| p.to_lowercase().ends_with(&suffix))
                .cloned()
                .collect();
            if !narrowed.is_empty() {
                return pick_source_relative(&narrowed, from);
            }
        }
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

    /// A subpath linkpath (`a/b`) resolves to the path ENDING in `a/b.md`, not to
    /// an unrelated file sharing the basename `b` — getFirstLinkpathDest honors the
    /// subdirs (§4.5). Fixture: FIVE `caveman.md` basename collisions plus the real
    /// `sources/git/caveman/CAVEMAN.md`; `[[caveman/CAVEMAN]]` must pick the last.
    /// Falsified by the bare basename: without the subpath narrow it would collide
    /// with (and, source-relative, prefer) `effects/skills/caveman.md`.
    #[test]
    fn resolve_linkpath_honors_subpath_over_basename_collision() {
        let mut index = CorpusIndex::new();
        let empty = build(String::new(), syntax::parse(""));
        for p in [
            "sources/caveman.md",
            "effects/skills/caveman.md",
            "domains/agents/skills/caveman.md",
            "sources/git/caveman/CAVEMAN.md",
        ] {
            index.insert(p, &empty);
        }
        // From the effect page, the bare `caveman` would be source-relative to the
        // effect page itself; the SUBPATH `caveman/CAVEMAN` must override that.
        assert_eq!(
            index.resolve_linkpath("caveman/CAVEMAN", "effects/skills/caveman.md"),
            Some("sources/git/caveman/CAVEMAN.md".to_string()),
            "the subpath qualifier selects the path ending in caveman/CAVEMAN.md",
        );
        // A bare basename still resolves best-effort over the collision set.
        assert!(
            index
                .resolve_linkpath("caveman", "effects/skills/caveman.md")
                .is_some(),
            "a bare basename still resolves (source-relative pick over the set)",
        );
        // A subpath that matches no path suffix falls back to the basename set.
        assert!(
            index
                .resolve_linkpath("nowhere/caveman", "sources/caveman.md")
                .is_some(),
            "a stale subpath degrades to the bare-basename resolution",
        );
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
        HpathSeg {
            h: h.to_string(),
            n: None,
        }
    }

    fn seg_n(h: &str, n: u32) -> HpathSeg {
        HpathSeg {
            h: h.to_string(),
            n: Some(n),
        }
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
        // Frozen §4.4: the fm_key LEAF span is terminator-EXCLUDED — `[4,15]` =
        // `title: Plan` (§1 leaf law: leaf block nodes exclude the final line
        // terminator). NOT the fence-to-fence container `[0,20]` (§18 row 3).
        assert_eq!(title.span, 4..15);
        assert_eq!(&doc.raw[title.span.clone()], "title: Plan");
        let independent =
            blake3::hash(&doc.raw.as_bytes()[4..15]).to_hex().as_str()[..16].to_string();
        assert_eq!(title.node_rev.0, independent);
        // Independent blake3 over the frozen leaf bytes pins the frozen §4.4
        // `node_rev_before` verbatim — the value cannot drift silently.
        assert_eq!(independent, "fa77480c79a853bc");
        assert_eq!(
            resolve(&doc, &Ref::FmKey("nope".to_string())),
            Err(ResolveError::NotFound)
        );
    }

    /// GATE 2 (frozen §4.4 dry example): the armed-write transition
    /// `title: Plan` → `title: Plan v2` COMPUTED through model's own
    /// resolve → `validate_batch` → apply, never quoted. Lands `span_after`
    /// `[4,18]` and `node_rev_after` `fb49e9df2257fab8`, both terminator-excluded.
    #[test]
    fn fm_key_armed_write_lands_frozen_span_after_and_rev() {
        let doc = build_plan(); // S0
        // node_rev_before: the terminator-excluded leaf [4,15] = fa77480c… (§4.4).
        let before = resolve(&doc, &Ref::FmKey("title".to_string())).expect("title S0");
        assert_eq!(before.span, 4..15);
        assert_eq!(before.node_rev.0, "fa77480c79a853bc");
        // Arm "Plan" → "Plan v2" over the fm_key leaf through model's sealed path.
        let b = batch(vec![match_edit(
            Ref::FmKey("title".to_string()),
            "Plan",
            "Plan v2",
            None,
        )]);
        let SpliceVerdict::Validated(vb) = validate_batch(&doc, None, &b, None) else {
            panic!("fm_key armed write must validate");
        };
        let s1_raw = apply_validated(&doc.raw, &vb);
        let s1 = build(s1_raw.clone(), syntax::parse(&s1_raw));
        // span_after [4,18] = "title: Plan v2" (14 bytes), terminator still excluded.
        let after = resolve(&s1, &Ref::FmKey("title".to_string())).expect("title S1");
        assert_eq!(after.span, 4..18);
        assert_eq!(&s1.raw[after.span.clone()], "title: Plan v2");
        let independent =
            blake3::hash(&s1.raw.as_bytes()[4..18]).to_hex().as_str()[..16].to_string();
        assert_eq!(after.node_rev.0, independent);
        assert_eq!(independent, "fb49e9df2257fab8"); // frozen §4.4 node_rev_after
    }

    // -----------------------------------------------------------------------
    // U2.11 — frontmatter multi-line block-sequence write grain
    // -----------------------------------------------------------------------
    //
    // Fixture = the LIVE pinned specimen: the a475ccfc cross-review frontmatter
    // at file rev df4198ba8ba4ab02 (rev verified via ccc-statusd read before
    // transcription). `inputs:` is a multi-line block SEQUENCE (11 items),
    // `tags:` a single-line FLOW sequence, `finalized_at:` the block's trailing
    // sibling. At the pinned rev the LIVE file is CORRUPTED: `finalized_at:` sits
    // wedged BETWEEN `inputs:` and its items (see `SPECIMEN_FM_WEDGED`) — the exact
    // silent corruption this unit fixes. `SPECIMEN_FM_CLEAN` is the pre-corruption
    // shape (finalized_at as the trailing sibling), the byte-stable block the
    // grain must round-trip.

    /// The pre-corruption specimen frontmatter — `inputs:` a well-formed block
    /// sequence, `finalized_at:` its trailing sibling.
    const SPECIMEN_FM_CLEAN: &str = "---\ntype: review\nsession: 22-01-meridian-attestation-module\nowner: \"[[a475ccfc]]\"\nrole: adversary (team-3, workflow-first arm)\nstatus: final\ncreated_at: 2026-07-22T23:45-04:00\ntags: [type/review, round2, cross-review, adversary]\ninputs:\n  - \"results/round2/design-1.md@d8536666b42dc8fd\"\n  - \"results/round2/design-2.md@a895cd0c580edf7b\"\n  - \"results/round2/design-3.md@32ed1508fb3396fa\"\n  - \"decisions/2026-07-22-meridian-go-end-state.md\"\n  - \"decisions/2026-07-22-pin-vocabulary-and-gating.md\"\n  - \"results/design-law-brief.md\"\n  - \"results/round-1-report.md\"\n  - \"results/engine-analysis.md\"\n  - \"[[substrate]]\"\n  - \"[[reconciliation]]\"\n  - \"[[attestation-tournament]]\"\nfinalized_at: 2026-07-23T00:20-04:00\n---\n\n# Cross-review — a475ccfc (team-3 adversary)\n\nbody\n";

    /// The LIVE bytes at df4198ba8ba4ab02 — `finalized_at:` wedged between
    /// `inputs:` and its block items (the in-the-wild silent corruption).
    const SPECIMEN_FM_WEDGED: &str = "---\ntype: review\nsession: 22-01-meridian-attestation-module\nowner: \"[[a475ccfc]]\"\nrole: adversary (team-3, workflow-first arm)\nstatus: final\ncreated_at: 2026-07-22T23:45-04:00\ntags: [type/review, round2, cross-review, adversary]\ninputs:\nfinalized_at: 2026-07-23T00:20-04:00\n  - \"results/round2/design-1.md@d8536666b42dc8fd\"\n  - \"results/round2/design-2.md@a895cd0c580edf7b\"\n  - \"results/round2/design-3.md@32ed1508fb3396fa\"\n  - \"decisions/2026-07-22-meridian-go-end-state.md\"\n  - \"decisions/2026-07-22-pin-vocabulary-and-gating.md\"\n  - \"results/design-law-brief.md\"\n  - \"results/round-1-report.md\"\n  - \"results/engine-analysis.md\"\n  - \"[[substrate]]\"\n  - \"[[reconciliation]]\"\n  - \"[[attestation-tournament]]\"\n---\n\n# Cross-review — a475ccfc (team-3 adversary)\n\nbody\n";

    fn specimen_clean() -> Document {
        build(
            SPECIMEN_FM_CLEAN.to_string(),
            syntax::parse(SPECIMEN_FM_CLEAN),
        )
    }

    /// U2.11 grain: a multi-line block-sequence value grows the `fm_key` leaf over
    /// the WHOLE value (key line + every indented item), while a single-line flow
    /// or scalar value keeps its frozen one-line grain (§1 leaf law). The old
    /// line-oriented grain stopped at the key line, orphaning the block on replace.
    #[test]
    fn fm_key_multiline_block_sequence_grain_spans_full_value() {
        let doc = specimen_clean();
        // `inputs:` (block sequence) → the grain covers the key line AND all 11
        // indented items, ending at the last item (terminator excluded), never
        // bleeding into the next key `finalized_at:`.
        let inputs = resolve(&doc, &Ref::FmKey("inputs".to_string())).expect("inputs fm_key");
        let grain = &doc.raw[inputs.span.clone()];
        assert!(grain.starts_with("inputs:\n  - \"results/round2/design-1.md"));
        assert!(grain.ends_with("  - \"[[attestation-tournament]]\""));
        assert!(
            !grain.contains("finalized_at"),
            "grain must stop before the next top-level key"
        );
        // honest CAS: the rev covers the FULL block value, not just the key line.
        let independent = blake3::hash(grain.as_bytes()).to_hex().as_str()[..16].to_string();
        assert_eq!(inputs.node_rev.0, independent);

        // `tags:` (flow sequence, single line) → grain is exactly the key line.
        let tags = resolve(&doc, &Ref::FmKey("tags".to_string())).expect("tags fm_key");
        assert_eq!(
            &doc.raw[tags.span.clone()],
            "tags: [type/review, round2, cross-review, adversary]"
        );
        // `status:` (scalar) → grain is exactly the key line (frozen leaf law).
        let status = resolve(&doc, &Ref::FmKey("status".to_string())).expect("status fm_key");
        assert_eq!(&doc.raw[status.span.clone()], "status: final");
    }

    /// U2.11 corrupting-patch fixture: the `put` properties patch that formerly
    /// orphaned the block sequence (a rev minted over silent corruption) now
    /// ENCODES CORRECTLY — the upsert replaces the WHOLE `inputs:` value, the 11
    /// items are gone, every sibling key survives byte-identical, and the reparse
    /// is clean. No silent-corrupt path remains: the only write that validates is
    /// one whose applied bytes reparse to exactly the intended frontmatter, and
    /// the minted rev is the honest rev over the clean new value.
    #[test]
    fn fm_multiline_upsert_encodes_correctly_no_orphan() {
        let doc = specimen_clean();
        let b = batch(vec![put_edit(
            Ref::FmKey("inputs".to_string()),
            PutAt::Upsert,
            "[design-1, design-2]",
        )]);
        let SpliceVerdict::Validated(vb) = validate_batch(&doc, None, &b, None) else {
            panic!("multi-line upsert must validate (encode correctly)");
        };
        let out = apply_validated(&doc.raw, &vb);
        let new_doc = build(out.clone(), syntax::parse(&out));

        // `inputs` is now the intended scalar — the block items are GONE (no orphan).
        let inputs = resolve(&new_doc, &Ref::FmKey("inputs".to_string())).expect("inputs after");
        assert_eq!(
            &new_doc.raw[inputs.span.clone()],
            "inputs: [design-1, design-2]"
        );
        for orphan in [
            "results/round2/design-1.md@d8536666b42dc8fd",
            "results/round2/design-3.md@32ed1508fb3396fa",
            "[[attestation-tournament]]",
        ] {
            assert!(
                !out.contains(orphan),
                "orphaned block item survived: {orphan}"
            );
        }
        // every sibling key survives byte-identical (the write touched only inputs).
        for (k, v) in [
            ("type", "type: review"),
            (
                "tags",
                "tags: [type/review, round2, cross-review, adversary]",
            ),
            ("finalized_at", "finalized_at: 2026-07-23T00:20-04:00"),
        ] {
            let r = resolve(&new_doc, &Ref::FmKey(k.to_string())).expect(k);
            assert_eq!(&new_doc.raw[r.span.clone()], v, "sibling {k} corrupted");
        }
        // the minted rev is the HONEST rev over the clean new value.
        let honest = blake3::hash(b"inputs: [design-1, design-2]")
            .to_hex()
            .as_str()[..16]
            .to_string();
        assert_eq!(inputs.node_rev.0, honest);
    }

    /// U2.11 round-trip byte-stability: reading the `inputs:` block sequence and
    /// writing the SAME bytes back through the sealed splice path leaves the file
    /// byte-identical — the grain the reader sees is exactly the grain the writer
    /// replaces (`at:all`), so a no-op patch is a no-op on disk.
    #[test]
    fn fm_multiline_block_roundtrip_byte_stable() {
        let doc = specimen_clean();
        let inputs = resolve(&doc, &Ref::FmKey("inputs".to_string())).expect("inputs fm_key");
        let read_back = doc.raw[inputs.span.clone()].to_string();
        assert!(
            read_back.contains("\n  - \"[[reconciliation]]\""),
            "read the block"
        );
        let b = batch(vec![put_edit(
            Ref::FmKey("inputs".to_string()),
            PutAt::All,
            &read_back,
        )]);
        let SpliceVerdict::Validated(vb) = validate_batch(&doc, None, &b, None) else {
            panic!("round-trip write must validate");
        };
        let out = apply_validated(&doc.raw, &vb);
        assert_eq!(
            out, SPECIMEN_FM_CLEAN,
            "read -> patch -> write must be byte-stable"
        );
    }

    /// U2.11 LIVE pinned witness (df4198ba8ba4ab02): on the corrupted specimen the
    /// wedged `finalized_at:` splits `inputs:` from its items — `inputs` resolves
    /// to an EMPTY grain and the 11 items misattribute to `finalized_at`'s grain.
    /// The corrected grain attributes lines to the nearest preceding column-0 key
    /// exactly as YAML block scope does, making the corruption visible instead of
    /// silent.
    #[test]
    fn fm_wedged_specimen_at_pinned_rev_grain() {
        let doc = build(
            SPECIMEN_FM_WEDGED.to_string(),
            syntax::parse(SPECIMEN_FM_WEDGED),
        );
        // `inputs:` is immediately followed by a column-0 key → empty value grain.
        let inputs = resolve(&doc, &Ref::FmKey("inputs".to_string())).expect("inputs fm_key");
        assert_eq!(&doc.raw[inputs.span.clone()], "inputs:");
        // the block items now hang off `finalized_at:` (the wedge's visible effect).
        let fin = resolve(&doc, &Ref::FmKey("finalized_at".to_string())).expect("finalized_at");
        let grain = &doc.raw[fin.span.clone()];
        assert!(
            grain.starts_with(
                "finalized_at: 2026-07-23T00:20-04:00\n  - \"results/round2/design-1.md"
            )
        );
        assert!(grain.ends_with("  - \"[[attestation-tournament]]\""));
    }

    /// DERIVED DATA (advisor 44870138 classification): the full-terminator trim
    /// (`\n` ⇒ 1, `\r\n` ⇒ 2, terminator-less ⇒ 0) is the correct mechanical
    /// reading of the §1 leaf law, but NO frozen worked value exercises `\r\n` —
    /// so these fixtures pin DATA, not law. TRIPWIRE: if any future frozen worked
    /// value contradicts them, STOP and report (do not silently re-pin).
    #[test]
    fn trim_terminator_excludes_full_terminator_derived_data() {
        let lf = b"title: Plan\n";
        assert_eq!(trim_terminator(lf, 0, lf.len()), 11); // \n ⇒ trim 1
        let crlf = b"title: Plan\r\n";
        assert_eq!(trim_terminator(crlf, 0, crlf.len()), 11); // \r\n ⇒ trim 2, no dangling \r
        let bare = b"title: Plan";
        assert_eq!(trim_terminator(bare, 0, bare.len()), 11); // terminator-less ⇒ trim 0
        assert_eq!(trim_terminator(b"\n", 0, 1), 0); // never underflows `start`
    }

    #[test]
    fn resolve_hpath_duplicate_siblings_ambiguous_unless_occurrence_given() {
        // Two identical `## Beta` siblings under `# A`: the mint plane refuses to
        // silently pick (contract §2.1) — Ambiguous unless an occurrence `n`
        // disambiguates (1-based, document order).
        let raw = "# A\n\n## Beta\n\nfirst\n\n## Beta\n\nsecond\n".to_string();
        let doc = build(raw.clone(), syntax::parse(&raw));
        let ResolveError::Ambiguous(cands) =
            resolve(&doc, &Ref::Hpath(vec![seg("A"), seg("Beta")]))
                .expect_err("duplicate is ambiguous")
        else {
            panic!("duplicate sibling hpath must resolve Ambiguous");
        };
        assert_eq!(cands.len(), 2);
        let first = resolve(&doc, &Ref::Hpath(vec![seg("A"), seg_n("Beta", 1)])).expect("Beta#1");
        let second = resolve(&doc, &Ref::Hpath(vec![seg("A"), seg_n("Beta", 2)])).expect("Beta#2");
        assert!(
            first.span.start < second.span.start,
            "n follows document order"
        );
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
        assert!(
            cands[0].span.start < cands[1].span.start,
            "candidates in document order"
        );
        let solo = resolve(&doc, &Ref::anchor("solo").unwrap()).expect("solo anchor");
        assert!(!solo.node_rev.0.is_empty());
        assert_eq!(
            resolve(&doc, &Ref::anchor("absent").unwrap()),
            Err(ResolveError::NotFound)
        );
    }

    /// GATE 1 (ANCHOR-GRAIN mint plane): on the §6.3 worked S1 receipts fixture,
    /// `resolve_anchor` serves the anchor's HOST BLOCK-LEAF — the `^r-000042`
    /// list-item line, terminator excluded (`[26,248]`, rev `639a2dca46f6fcc8`) —
    /// NOT the 9-byte `[239,248]` inline `^id` marker span. §2.1: "an anchor
    /// becomes a write target by the same one-hop path as a section"; §4.1/§6.3
    /// pin the worked span/rev; §1: "leaf block nodes exclude the final line
    /// terminator". The marker span stays a legitimate `syntax` §1 inline fact
    /// (untouched); the model re-derives the write-target grain at `build`.
    #[test]
    fn resolve_anchor_serves_host_block_leaf_s1_receipts() {
        let raw = merkle_fixtures().receipts_v1;
        assert_eq!(raw.len(), 249, "S1 receipts fixture is 249 bytes (§0.3)");
        let doc = build(raw.clone(), syntax::parse(&raw));
        let t = resolve(&doc, &Ref::anchor("r-000042").unwrap()).expect("mint resolves ^r-000042");
        assert_eq!(
            t.span,
            26..248,
            "host block-leaf span, not the [239,248] marker"
        );
        assert_eq!(
            t.node_rev.0, "639a2dca46f6fcc8",
            "§6.3 rev over the block-leaf bytes"
        );
        // the 9-byte inline marker grain must never be the mint write-target.
        assert_ne!(t.span, 239..248, "marker-grain defect must not resurface");
        // independent rev check over exactly the block-leaf bytes (no reliance on
        // model's own hashing for the ground truth).
        let independent =
            blake3::hash(&raw.as_bytes()[26..248]).to_hex().as_str()[..16].to_string();
        assert_eq!(
            t.node_rev.0, independent,
            "rev = blake3(block-leaf bytes)[:16]"
        );
    }

    // -----------------------------------------------------------------------
    // validate_batch (rung 4) — §4.4 batch grammar + §5 CAS/failure split
    // -----------------------------------------------------------------------

    /// S2 plan (`plan_v2`): "ship by September" at Q3, "- new item" appended to
    /// Q4 — the state the §5.2 worked failures run against.
    fn plan_s2() -> Document {
        let raw = merkle_fixtures().plan_v2;
        build(raw.clone(), syntax::parse(&raw))
    }

    /// S1 plan (`plan_v1`): "ship by September", before the E4 Q4 append.
    fn plan_s1() -> Document {
        let raw = merkle_fixtures().plan_v1;
        build(raw.clone(), syntax::parse(&raw))
    }

    fn hpath(segs: &[&str]) -> Ref {
        Ref::Hpath(segs.iter().map(|h| seg(h)).collect())
    }

    fn match_edit(target: Ref, old: &str, new: &str, guard: Option<&str>) -> Edit {
        Edit {
            target,
            edit: EditKind::Match {
                old: old.to_string(),
                new: new.to_string(),
            },
            if_node_rev: guard.map(|g| NodeRev(g.to_string())),
        }
    }

    fn put_edit(target: Ref, at: PutAt, text: &str) -> Edit {
        Edit {
            target,
            edit: EditKind::Put {
                at,
                text: text.to_string(),
            },
            if_node_rev: None,
        }
    }

    fn batch(edits: Vec<Edit>) -> SpliceRequest {
        SpliceRequest {
            if_root: None,
            edits,
        }
    }

    /// Reconstruct the applied bytes from a sealed batch's public edits — the
    /// pass `fs` will make. Proves the sealed spans/texts are usable downstream.
    fn apply_validated(raw: &str, vb: &ValidatedBatch) -> String {
        let mut edits = vb.edits.clone();
        edits.sort_by_key(|e| e.span.start);
        let mut out = String::new();
        let mut cursor = 0;
        for e in &edits {
            out.push_str(&raw[cursor..e.span.start]);
            out.push_str(&e.text);
            cursor = e.span.end;
        }
        out.push_str(&raw[cursor..]);
        out
    }

    /// GATE 1a (§5.2 frame 88): the client holds S0's Q3 rev (33d5…) against
    /// S2 where Q3 is 41f6… → `cas_mismatch{expected,actual}`, refresh. Actual
    /// is RE-DERIVED with blake3 here (no reliance on model's own hashing).
    #[test]
    fn gate1_cas_mismatch_expected_actual() {
        let doc = plan_s2();
        let q3_bytes = &doc.raw.as_bytes()[49..75];
        let actual = blake3::hash(q3_bytes).to_hex().as_str()[..16].to_string();
        assert_eq!(actual, "41f643f034e5681f", "S2 Q3 rev, oracle-pinned");
        let b = batch(vec![match_edit(
            hpath(&["Goals", "Q3"]),
            "ship by September",
            "ship by October",
            Some("33d5b0e1b27cb48b"),
        )]);
        assert_eq!(
            validate_batch(&doc, None, &b, None),
            SpliceVerdict::CasMismatch {
                expected: NodeRev("33d5b0e1b27cb48b".to_string()),
                actual: NodeRev("41f643f034e5681f".to_string()),
            }
        );
    }

    /// GATE 1b (§5.2 frame 89): guard PASSES (41f6…), old-string absent →
    /// `no_match{matches:0}`, fix — provably a typo. Count computed.
    #[test]
    fn gate1_no_match_guard_passed() {
        let doc = plan_s2();
        assert_eq!(
            doc.raw[49..75].matches("ship by August").count(),
            0,
            "old-string absent at S2 (Q3 says September)"
        );
        let b = batch(vec![match_edit(
            hpath(&["Goals", "Q3"]),
            "ship by August",
            "ship by October",
            Some("41f643f034e5681f"),
        )]);
        assert_eq!(
            validate_batch(&doc, None, &b, None),
            SpliceVerdict::NoMatch { matches: 0 }
        );
    }

    /// GATE 1c (§5.2 frame 91): `item` occurs twice in Q4@S2 (`- item one`,
    /// `- new item`) → `not_unique{matches:2}`, fix. Count computed.
    #[test]
    fn gate1_not_unique_matches_two() {
        let doc = plan_s2();
        assert_eq!(
            doc.raw[75..150].matches("item").count(),
            2,
            "'item' twice in Q4@S2"
        );
        let b = batch(vec![match_edit(
            hpath(&["Goals", "Q4"]),
            "item",
            "entry",
            None,
        )]);
        assert_eq!(
            validate_batch(&doc, None, &b, None),
            SpliceVerdict::NotUnique { matches: 2 }
        );
    }

    /// GATE 2 (compile-level): no request-side type carries a byte span. The
    /// only fields are the match-based grammar; a `SpliceRequest`/`Edit` cannot
    /// name an offset (D-C1 — the wrong-offset write is unrepresentable). This
    /// test compiling at all IS the gate; the field-shape assert documents it.
    #[test]
    fn gate2_no_span_field_request_side() {
        let e = match_edit(hpath(&["Goals", "Q3"]), "a", "b", None);
        // Destructure every field — a `span` field would force a compile error.
        let Edit {
            target: _,
            edit: _,
            if_node_rev: _,
        } = &e;
        let SpliceRequest {
            if_root: _,
            edits: _,
        } = batch(vec![e]);
    }

    /// GATE 3 (seal, positive): only `model` mints a `ValidatedBatch`; a valid
    /// batch yields the capability token `fs` demands. The NEGATIVE half — that
    /// no external crate can construct one — is the `compile_fail` doctest on
    /// [`ValidatedBatch`] (the private `_sealed` field).
    #[test]
    fn gate3_seal_minted_on_success() {
        let doc = plan_s1();
        let b = batch(vec![match_edit(
            hpath(&["Goals", "Q3"]),
            "ship by September",
            "ship by October",
            Some("41f643f034e5681f"),
        )]);
        let SpliceVerdict::Validated(vb) = validate_batch(&doc, None, &b, None) else {
            panic!("clean CAS + unique match must validate");
        };
        assert_eq!(vb.edits.len(), 1);
        assert!(vb.receipt.is_none());
    }

    /// The receipt append rides INSIDE the sealed batch (§6.1): a pre-rendered
    /// append folded into validation is carried on the sealed token, to commit
    /// in the same batch. Model seals it; it never renders the bytes.
    #[test]
    fn seal_carries_receipt_append() {
        let doc = plan_s1();
        let b = batch(vec![match_edit(
            hpath(&["Goals", "Q3"]),
            "ship by September",
            "ship by October",
            None,
        )]);
        let receipt = ReceiptAppend {
            span: 26..26,
            text: "- splice notes/plan.md ^r-000099".to_string(),
        };
        let SpliceVerdict::Validated(vb) = validate_batch(&doc, None, &b, Some(receipt.clone()))
        else {
            panic!("must validate");
        };
        assert_eq!(vb.receipt, Some(receipt), "receipt rides inside the seal");
    }

    /// A clean E3 match applies as raw replacement: Q3 August→September on S0
    /// yields `plan_v1` byte-exact (validated span/text usable by `fs`).
    #[test]
    fn validated_match_applies_to_s1() {
        let doc = build_plan(); // S0
        let b = batch(vec![match_edit(
            hpath(&["Goals", "Q3"]),
            "ship by August",
            "ship by September",
            Some("33d5b0e1b27cb48b"),
        )]);
        let SpliceVerdict::Validated(vb) = validate_batch(&doc, None, &b, None) else {
            panic!("E3 must validate");
        };
        assert_eq!(
            apply_validated(&doc.raw, &vb),
            merkle_fixtures().plan_v1,
            "match replacement reproduces S1"
        );
    }

    /// GATE 5 (§4.4 at:end raw-concat): E4 appends `- new item\n` at Q4's
    /// span-end (EOF) → `plan_v2` by PURE byte concatenation, NO synthesized
    /// separator. Verified both against the frozen S2 bytes and against
    /// `plan_v1 + text` (the raw-concat law itself).
    #[test]
    fn gate5_at_end_raw_concat() {
        let doc = plan_s1();
        let f = merkle_fixtures();
        let b = batch(vec![put_edit(
            hpath(&["Goals", "Q4"]),
            PutAt::End,
            "- new item\n",
        )]);
        let SpliceVerdict::Validated(vb) = validate_batch(&doc, None, &b, None) else {
            panic!("at:end append must validate");
        };
        let applied = apply_validated(&doc.raw, &vb);
        assert_eq!(applied, f.plan_v2, "byte-exact S2");
        assert_eq!(
            applied,
            format!("{}{}", f.plan_v1, "- new item\n"),
            "pure raw concat — no synthesized separator"
        );
        // the append is a zero-width insert at span-end (EOF here).
        assert_eq!(vb.edits[0].span, 139..139, "insert at Q4 span-end");
    }

    /// GATE 4a (§4.4 disjointness): two edits whose TARGETS nest are not
    /// disjoint → `overlap` (`bad_request`). Goals ⊃ Q3, both resolve+match, then
    /// the batch-wide check refuses with the overlapping target spans.
    #[test]
    fn gate4_overlap_bad_request() {
        let doc = plan_s2();
        let b = batch(vec![
            put_edit(hpath(&["Goals"]), PutAt::End, "x\n"),
            match_edit(hpath(&["Goals", "Q3"]), "September", "October", None),
        ]);
        let SpliceVerdict::Overlap { spans } = validate_batch(&doc, None, &b, None) else {
            panic!("nested targets must refuse overlap");
        };
        assert_eq!(spans, vec![20..150, 49..75], "Goals ⊃ Q3");
    }

    /// GATE 4b (§4.4 `would_corrupt`): a separator-less `at:"end"` on a NON-final
    /// section bleeds into the next heading and destroys it — the reparse loses
    /// containment → `would_corrupt{lost}`. RULE-DERIVED DATA: a section
    /// byte-disjoint from every replaced region that no longer resolves was
    /// corrupted, not rewritten (the at:end law's own warning, §4.4).
    #[test]
    fn gate4_would_corrupt_lost() {
        let doc = plan_s2();
        // `MORE` (no leading \n) inserted at Q3's span-end (= Q4's heading start)
        // yields `…September\n\nMORE## Q4…` — `## Q4` is no longer at line start.
        let b = batch(vec![put_edit(hpath(&["Goals", "Q3"]), PutAt::End, "MORE")]);
        let SpliceVerdict::WouldCorrupt { lost } = validate_batch(&doc, None, &b, None) else {
            panic!("heading-destroying append must refuse would_corrupt");
        };
        assert_eq!(
            lost,
            vec![vec!["Goals".to_string(), "Q4".to_string()]],
            "Q4 (disjoint from the edit) was destroyed"
        );
    }

    /// A well-formed `at:"end"` on the same non-final section (leading `\n`
    /// carried by the caller) preserves containment → validates. The mirror of
    /// gate 4b: the raw-concat law puts the separator on the caller, and getting
    /// it right keeps `## Q4` at line start.
    #[test]
    fn at_end_with_separator_preserves_containment() {
        let doc = plan_s2();
        let b = batch(vec![put_edit(
            hpath(&["Goals", "Q3"]),
            PutAt::End,
            "extra\n",
        )]);
        let SpliceVerdict::Validated(vb) = validate_batch(&doc, None, &b, None) else {
            panic!("separator-carrying append must validate");
        };
        let applied = apply_validated(&doc.raw, &vb);
        assert!(
            applied.contains("\n\nextra\n## Q4\n"),
            "Q4 heading survives"
        );
    }

    /// U2.2 refuse-ambiguous-only + the F6 fix: a file with a DUPLICATE heading
    /// refuses ONLY the write that addresses the ambiguous selector — an
    /// unambiguous sibling still validates, and the duplicate is addressable by
    /// node index. Before the fix, `collect_lost` re-resolved each disjoint
    /// section's bare hpath and counted an `Ambiguous` result as `would_corrupt`,
    /// so one stray duplicate refused EVERY write to the file (the F6 refuse-all
    /// death mode that forced outside-jail repair).
    #[test]
    fn duplicate_heading_refuses_ambiguous_only_sibling_serves() {
        // two `## Objective` under `# Task`, plus an unambiguous `## Notes`.
        let raw = "# Task\n\n## Objective\n\nalpha ^a1b2c3\n\n## Objective\n\nbeta ^d4e5f6\n\n## Notes\n\ngamma\n".to_string();
        let doc = build(raw.clone(), syntax::parse(&raw));

        // (a) a write at the AMBIGUOUS bare selector refuses `Ambiguous`, naming
        // both duplicate targets — refuse-ambiguous-only.
        let amb = batch(vec![put_edit(
            hpath(&["Task", "Objective"]),
            PutAt::Content,
            "rewritten\n",
        )]);
        let SpliceVerdict::Ambiguous(cands) = validate_batch(&doc, None, &amb, None) else {
            panic!("a write at the ambiguous selector must refuse Ambiguous");
        };
        assert_eq!(cands.len(), 2, "both duplicates are named candidates");

        // (b) a write at the UNAMBIGUOUS sibling still SERVES — the F6 fix: a
        // stray duplicate elsewhere no longer poisons a byte-disjoint write.
        let sibling = batch(vec![put_edit(
            hpath(&["Task", "Notes"]),
            PutAt::Content,
            "served\n",
        )]);
        assert!(
            matches!(
                validate_batch(&doc, None, &sibling, None),
                SpliceVerdict::Validated(_)
            ),
            "an unambiguous sibling write must validate despite the file's duplicate heading"
        );

        // (c) the duplicate is addressable by node index (`n=`) — the write serves.
        let by_index = batch(vec![put_edit(
            Ref::Hpath(vec![seg("Task"), seg_n("Objective", 1)]),
            PutAt::Content,
            "first only\n",
        )]);
        assert!(
            matches!(
                validate_batch(&doc, None, &by_index, None),
                SpliceVerdict::Validated(_)
            ),
            "node-index addressing disambiguates the duplicate"
        );
    }

    /// `put{at:"content"}` replaces the content span, heading PRESERVED (§4.4);
    /// `put{at:"all"}` replaces the full span, heading included. The content
    /// span begins after the heading line's terminator.
    #[test]
    fn put_content_preserves_heading_all_replaces_it() {
        let doc = plan_s2();
        let content = batch(vec![put_edit(
            hpath(&["Goals", "Q3"]),
            PutAt::Content,
            "REPLACED\n",
        )]);
        let SpliceVerdict::Validated(vb) = validate_batch(&doc, None, &content, None) else {
            panic!("put:content must validate");
        };
        assert_eq!(vb.edits[0].span, 55..75, "content span: after `## Q3\\n`");
        assert!(apply_validated(&doc.raw, &vb).contains("## Q3\nREPLACED\n## Q4"));

        let all = batch(vec![put_edit(
            hpath(&["Goals", "Q3"]),
            PutAt::All,
            "## Q3\ndone\n",
        )]);
        let SpliceVerdict::Validated(vb) = validate_batch(&doc, None, &all, None) else {
            panic!("put:all must validate");
        };
        assert_eq!(vb.edits[0].span, 49..75, "full span, heading included");
    }

    /// `if_root` is the world guard, checked FIRST (§5.1): a mismatch fails the
    /// whole batch (`root_mismatch` → resync); an equal guard proceeds.
    #[test]
    fn if_root_world_guard_checked_first() {
        let doc = plan_s2();
        let live = MerkleRoot("b3:aaaa".to_string());
        let mut b = batch(vec![match_edit(
            hpath(&["Goals", "Q3"]),
            "September",
            "October",
            None,
        )]);
        b.if_root = Some(MerkleRoot("b3:bbbb".to_string()));
        assert_eq!(
            validate_batch(&doc, Some(&live), &b, None),
            SpliceVerdict::RootMismatch {
                expected: MerkleRoot("b3:bbbb".to_string()),
                actual: live.clone(),
            }
        );
        // equal guard → proceeds past the world check to a clean validation.
        b.if_root = Some(live.clone());
        assert!(matches!(
            validate_batch(&doc, Some(&live), &b, None),
            SpliceVerdict::Validated(_)
        ));
    }

    /// A missing target → `ref_not_found` (refresh, §4.5).
    #[test]
    fn missing_target_ref_not_found() {
        let doc = plan_s2();
        let b = batch(vec![match_edit(hpath(&["Goals", "Q9"]), "a", "b", None)]);
        assert_eq!(
            validate_batch(&doc, None, &b, None),
            SpliceVerdict::RefNotFound
        );
    }

    /// GATE 6 (§1 write-side multibyte refusal). The guarantor: a replaced
    /// region off a UTF-8 char boundary refuses `bad_request`. This is exercised
    /// WHITE-BOX because the wrong state is unrepresentable through the public
    /// `match`/`put` API — valid-UTF-8 edits at char-aligned resolved spans are
    /// self-synchronizing, so a mid-char region cannot arise from a batch
    /// (parallel to D-C1). Design, not dodge: the guard exists so any mid-char
    /// region refuses loud; the black-box half proves legitimate multibyte
    /// content splices cleanly.
    #[test]
    fn gate6_multibyte_split_guarantor() {
        // `日` = E6 97 A5 (bytes 0..3); byte 1 and 2 are mid-character.
        assert_eq!(
            guard_char_aligned("日本", &(1..3)),
            Err(SpliceVerdict::MultibyteSplit),
            "region starting mid-`日` refuses"
        );
        assert_eq!(
            guard_char_aligned("日本", &(0..3)),
            Ok(()),
            "aligned region (whole `日`) passes"
        );
        assert_eq!(guard_char_aligned("café", &(3..5)), Ok(()), "`é` is [3,5)");
    }

    /// GATE 6 black-box: a `match` over multibyte content validates and applies
    /// cleanly — the self-synchronizing property in action (the refusal above is
    /// for the unrepresentable case, not legitimate non-ASCII edits).
    #[test]
    fn gate6_multibyte_match_validates() {
        let raw = "# Café ☕\n\ncafé ☕ 日本語 tea\n".to_string();
        let doc = build(raw.clone(), syntax::parse(&raw));
        let b = batch(vec![match_edit(
            hpath(&["Café ☕"]),
            "日本語",
            "コーヒー",
            None,
        )]);
        let SpliceVerdict::Validated(vb) = validate_batch(&doc, None, &b, None) else {
            panic!("legitimate multibyte match must validate");
        };
        assert_eq!(
            apply_validated(&doc.raw, &vb),
            "# Café ☕\n\ncafé ☕ コーヒー tea\n"
        );
    }

    // -----------------------------------------------------------------------
    // merkle_root (rung 3) — §12.2 encoding + §12.3 prefix bump
    // -----------------------------------------------------------------------

    // Frozen hex ground truth (independently recomputed, 69/69).
    const R0_HEX: &str = "74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9";
    const R1_HEX: &str = "10769ae1c77f5646750f3f52df2d055156b411145a02b8361ecd32af1357a1b7";
    const R2_HEX: &str = "83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68";
    const R0_WRONG_HEX: &str = "75a61c883e372102cfe7d75e94992b9be65e33fbe95956897a4cf2ea45bb8f1b";
    const R_V0_DRAFTS_HEX: &str =
        "05f0c6192308db5937c3e1352d1f9a6fc31b89b1a57175c8af6ce7903525aa4a";

    // Non-plan fixture bytes, verbatim from the contract §0.3 fixture bytes (docs/wire-contract-v2.md).
    const RECEIPTS_V0: &str = "# Receipts \u{2014} 2026-07-18\n"; // em dash = 3-byte UTF-8
    const GH_README: &str = "# CI notes\n";
    const DRAFT_TMP: &str = "scratch\n";

    /// The six-root corpus fixtures, built the way the oracle builds them: raw
    /// plan/receipts bytes across S0→S2, each receipts file interpolating the
    /// prior root token + the Q3/Q4 section revs (every value pinned in
    /// an independent oracle). Byte-length asserts guard the
    /// transcription; the roots themselves are the ultimate check.
    struct MerkleFixtures {
        plan_v0: String,
        plan_v1: String,
        plan_v2: String,
        receipts_v0: String,
        receipts_v1: String,
        receipts_v2: String,
    }

    fn merkle_fixtures() -> MerkleFixtures {
        let plan_v0 = PLAN_S0.to_string();
        let plan_v1 = plan_v0.replace("ship by August", "ship by September");
        let plan_v2 = format!("{plan_v1}- new item\n");

        let receipts_v0 = RECEIPTS_V0.to_string();
        // Receipt lines exactly as §6.3 prints them (root_before + section revs).
        let receipt_42 = format!(
            "- splice notes/plan.md id=42 actor=agent:b0864fb2 now=2026-07-18T20:31:04Z \
             root_before=b3:{R0_HEX} edits=1 Goals>Q3 match 33d5b0e1b27cb48b->41f643f034e5681f ^r-000042\n"
        );
        let receipts_v1 = format!("{receipts_v0}{receipt_42}");
        let receipt_43 = format!(
            "- splice notes/plan.md id=57 actor=agent:b0864fb2 now=2026-07-18T20:33:41Z \
             root_before=b3:{R1_HEX} edits=1 Goals>Q4 put:end 4b8bc385a58da0e0->f43203a1f0b4c9a3 ^r-000043\n"
        );
        let receipts_v2 = format!("{receipts_v1}{receipt_43}");

        // Transcription guards (contract §0.3 sizes + §6.3 receipt-line bytes).
        assert_eq!(plan_v0.len(), 136);
        assert_eq!(plan_v1.len(), 139);
        assert_eq!(plan_v2.len(), 150);
        assert_eq!(receipts_v0.len(), 26);
        assert_eq!(receipt_42.len(), 223);
        assert_eq!(receipt_43.len(), 225);
        assert_eq!(receipts_v1.len(), 249);
        assert_eq!(receipts_v2.len(), 474);

        MerkleFixtures {
            plan_v0,
            plan_v1,
            plan_v2,
            receipts_v0,
            receipts_v1,
            receipts_v2,
        }
    }

    /// Gate root 1–3: R0/R1/R2, the corpus root at S0/S1/S2 (version 0 ⇒ `b3:`).
    #[test]
    fn merkle_root_r0_r1_r2() {
        let f = merkle_fixtures();
        let r0 = merkle_root(
            &[
                ("notes/plan.md", f.plan_v0.as_bytes()),
                ("receipts/2026-07-18.md", f.receipts_v0.as_bytes()),
            ],
            0,
        );
        assert_eq!(r0.0, format!("b3:{R0_HEX}"));
        let r1 = merkle_root(
            &[
                ("notes/plan.md", f.plan_v1.as_bytes()),
                ("receipts/2026-07-18.md", f.receipts_v1.as_bytes()),
            ],
            0,
        );
        assert_eq!(r1.0, format!("b3:{R1_HEX}"));
        let r2 = merkle_root(
            &[
                ("notes/plan.md", f.plan_v2.as_bytes()),
                ("receipts/2026-07-18.md", f.receipts_v2.as_bytes()),
            ],
            0,
        );
        assert_eq!(r2.0, format!("b3:{R2_HEX}"));
    }

    /// Gate root 4: the §12.1 wrong-ignore counterfactual. The correct domain
    /// (`.github/` dropped) computes R0; a broken ignore that lets
    /// `.github/README.md` in computes a different root — it cannot pass both.
    #[test]
    fn merkle_root_counterfactual_12_1() {
        let f = merkle_fixtures();
        assert_eq!(GH_README.len(), 11);
        let plan = ("notes/plan.md", f.plan_v0.as_bytes());
        let receipts = ("receipts/2026-07-18.md", f.receipts_v0.as_bytes());
        let correct = merkle_root(&[plan, receipts], 0);
        assert_eq!(correct.0, format!("b3:{R0_HEX}"));
        let wrong = merkle_root(
            &[plan, receipts, (".github/README.md", GH_README.as_bytes())],
            0,
        );
        assert_eq!(wrong.0, format!("b3:{R0_WRONG_HEX}"));
        assert_ne!(correct, wrong, "the ignore decision is load-bearing");
    }

    /// Gate roots 5–6: the §12.3 domain bump pair. v0 keeps `drafts/tmp.md`
    /// (version 0 ⇒ `b3:`); v1 ignores it and bumps the version (⇒ `b3a:`).
    /// v1's surviving set equals R2's, so the HEX repeats — yet the tokens never
    /// compare equal, a foreign `b3:` cursor can't silently match a `b3a:` world.
    #[test]
    fn merkle_root_domain_bump_12_3() {
        let f = merkle_fixtures();
        assert_eq!(DRAFT_TMP.len(), 8);
        let plan = ("notes/plan.md", f.plan_v2.as_bytes());
        let receipts = ("receipts/2026-07-18.md", f.receipts_v2.as_bytes());

        let bump_v0 = merkle_root(
            &[plan, receipts, ("drafts/tmp.md", DRAFT_TMP.as_bytes())],
            0,
        );
        assert_eq!(bump_v0.0, format!("b3:{R_V0_DRAFTS_HEX}"));

        let bump_v1 = merkle_root(&[plan, receipts], 1);
        assert_eq!(bump_v1.0, format!("b3a:{R2_HEX}"));

        let r2 = merkle_root(&[plan, receipts], 0);
        // bump_hex_equals_R2: identical hex …
        assert_eq!(
            bump_v1.0.strip_prefix("b3a:"),
            r2.0.strip_prefix("b3:"),
            "bump_hex_equals_R2",
        );
        // … yet the tokens never compare equal (the entire point of the bump).
        assert_ne!(bump_v1, r2, "different prefix ⇒ tokens never equal");
        assert_ne!(bump_v0, bump_v1);
    }

    /// §12.3 prefix mapping — the bijective base-26 suffix after `b3`.
    #[test]
    fn root_prefix_bijective_base26() {
        assert_eq!(root_prefix(0), "b3:");
        assert_eq!(root_prefix(1), "b3a:");
        assert_eq!(root_prefix(2), "b3b:");
        assert_eq!(root_prefix(26), "b3z:");
        assert_eq!(root_prefix(27), "b3aa:");
        assert_eq!(root_prefix(28), "b3ab:");
        assert_eq!(root_prefix(52), "b3az:");
        assert_eq!(root_prefix(53), "b3ba:");
        assert_eq!(root_prefix(702), "b3zz:");
        assert_eq!(root_prefix(703), "b3aaa:");
    }

    /// §12.2 "empty dirs pruned": a directory carrying only an empty subdir
    /// folds identically to one that never had it (white-box on the fold).
    #[test]
    fn empty_dir_is_pruned_12_2() {
        let mut base = MerkleDir::default();
        base.entries
            .insert("a.md".to_string(), MerkleEntry::File(b"x".to_vec()));
        let mut with_empty = MerkleDir::default();
        with_empty
            .entries
            .insert("a.md".to_string(), MerkleEntry::File(b"x".to_vec()));
        with_empty
            .entries
            .insert("empty".to_string(), MerkleEntry::Dir(MerkleDir::default()));
        assert_eq!(
            base.fold(),
            with_empty.fold(),
            "empty dir contributes nothing"
        );
    }

    /// The §12.2 varint is unsigned LEB128 (single byte below 128).
    #[test]
    fn write_uleb128_matches_spec() {
        let cases: &[(usize, &[u8])] = &[
            (0, &[0x00]),
            (7, &[0x07]),
            (127, &[0x7f]),
            (128, &[0x80, 0x01]),
            (300, &[0xac, 0x02]),
        ];
        for (value, want) in cases {
            let mut out = Vec::new();
            write_uleb128(&mut out, *value);
            assert_eq!(&out, want, "uleb128({value})");
        }
    }
}
