//! In-memory world model: governed tree (`kind/span/node_rev/hpath`), resolve,
//! CAS-splice validation, Merkle roots — deliberately non-serializable.
//!
//! # Charter
//! **Owns:** derived world model from `syntax`'s dialect stream — governed tree
//! (policy-schema §2), `node_rev` / Merkle roots, `#hpath`/`#^anchor` resolve,
//! CAS splice validation, node-level diff. Corpus name index (derived,
//! disposable; `query`/`policy` borrow). Frozen Go-text heading predicate
//! ([`gotext`]) shared by `wire-map` and `policy` so they cannot drift.
//!
//! **Never:** file I/O (`fs`), persistence (law 2: cold rebuild), protocol types
//! (law 3), body formatting.
//!
//! **No serde on any public type** — model facts reach the wire only via
//! the serving host's projection seam (law 3).

use std::collections::{BTreeMap, btree_map};
use std::ops::Range;

use addr::{Addr, AddrError, MountName, MountSet};

pub mod delta;
pub mod fingerprint;
pub mod gotext;
pub mod scalar;
pub mod selector;
pub mod walk;

/// Half-open byte range into a file's raw bytes. Distinct from the wire's
/// serializable span on purpose — converting between them is the host's job.
pub type ByteSpan = Range<usize>;

/// Engine-minted `hash-algo` label — `blake3-256(span bytes)[:16]` (contract v2
/// §1; node-rev-merkle-spec §1). Non-native labels stay out of node-rev compare
/// ([`is_native_algo`]) and can never be green. Sole owner so `pin`/`view` agree.
pub const NODE_REV_ALGO: &str = "node-rev";

/// Effect-page `hash-algo` for the v1→v2 supersede (design-2 §6.3: merkle-v1
/// composition with sha256→blake3, norm-v1→raw bytes). Whole-page `v2` pins
/// verify through the same node-rev compare as native `node-rev`.
pub const SUPERSEDE_ALGO_V2: &str = "v2";

/// Whether `algo` is engine-native (`node-rev` | `v2`) for green/red node-rev
/// compare. Other labels (v1, statusd-file-rev, …) are not recomputable here.
/// Fence only — not a renderer.
#[must_use]
pub fn is_native_algo(algo: &str) -> bool {
    algo == NODE_REV_ALGO || algo == SUPERSEDE_ALGO_V2
}

/// Node content hash — CAS token model-side form. Opaque.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NodeRev(pub String);

/// Merkle root over a file or the corpus. 32-byte guard/freshness cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MerkleRoot(pub String);

/// Parsed frontmatter, document order preserved — wire `keys` must echo
/// document order (B1 predicate 4). Flat `(key, value)` pairs, first occurrence
/// wins; no YAML library (no-serde crate law).
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
/// is the host's.
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
    /// Wire-observable leaf — every dialect construct is representable.
    InlineCode,
    /// Wire-observable leaf.
    Comment,
}

/// One parsed file: the tree plus the raw bytes it was derived from — spans
/// index into `raw`.
#[derive(Debug, Clone)]
pub struct Document {
    pub raw: String,
    pub root: Node,
}

/// Build the governed tree from `syntax`'s dialect stream — the syntax→model
/// seam.
///
/// Structure: `Document` root (span = whole file) → an optional `Frontmatter`
/// node then heading-nested `Section` nodes; leaf dialect constructs attach to
/// the deepest section that spans them. Every node carries
/// `node_rev = blake3-256(span bytes)[:16]` (contract §1), so the root's
/// `node_rev` equals `file_rev` by construction.
#[must_use]
pub fn build(raw: String, nodes: Vec<syntax::DialectNode>) -> Document {
    use syntax::DialectKind as D;

    // Host-candidate blocks for anchor attachment (F-R4 Obsidian parity):
    // heading lines plus the leaf dialect blocks, collected before the node
    // walk so every anchor sees the whole block inventory.
    let hosts: Vec<ByteSpan> = nodes
        .iter()
        .filter_map(|n| match n.kind {
            D::Heading { .. } | D::Fence { .. } | D::Callout { .. } | D::Table | D::Task { .. } => {
                Some(n.span.clone())
            }
            _ => None,
        })
        .collect();
    let fm_span: Option<ByteSpan> = nodes
        .iter()
        .find_map(|n| matches!(n.kind, D::Frontmatter { .. }).then(|| n.span.clone()));

    let mut frontmatter: Option<Node> = None;
    let mut headings: Vec<(usize, u8, String)> = Vec::new();
    let mut leaves: Vec<Node> = Vec::new();

    for node in nodes {
        let syntax::DialectNode { kind, span } = node;
        match kind {
            D::Frontmatter { .. } => {
                // Fence-to-fence, terminator-inclusive (§18 row 3): the model
                // frontmatter node is span-lawed with the section
                // (newline-inclusive) family, so extend over the terminator
                // `syntax` trimmed off the closing fence.
                let span = span.start..extend_terminator(raw.as_bytes(), span.end);
                let map = parse_frontmatter(&raw, &span);
                frontmatter = Some(leaf_node(&raw, NodeKind::Frontmatter { map }, span));
            }
            D::Heading { level, text } => headings.push((span.start, level, text)),
            other => {
                if let Some(kind) = leaf_kind(other) {
                    // An anchor's model node carries its host block-leaf span,
                    // not the inline `^id` marker span `syntax` emits (§2.1 /
                    // §4.1 / §6.3). Every other leaf keeps its span.
                    let span = match kind {
                        NodeKind::Anchor { .. } => {
                            anchor_host_span(raw.as_bytes(), &span, &hosts, fm_span.as_ref())
                        }
                        _ => span,
                    };
                    leaves.push(leaf_node(&raw, kind, span));
                }
            }
        }
    }

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
/// `marker` — standard Obsidian block-reference attachment (F-R4 ruling,
/// 2026-08-13; ground truth: Obsidian 1.13.6's own block cache, pinned in
/// `tests/obsidian_block_parity.rs`). Spans are terminator-excluded
/// (contract §1 leaf-block law; the write-target is this host block).
///
/// - **Tail id** (content precedes the marker on its line): the enclosing
///   leaf block — a `hosts` block containing the line (whole callout/table;
///   a heading or task line), a list ITEM line (item grain, never the list),
///   else the whole contiguous paragraph run.
/// - **Own-line id**: attaches to the nearest preceding block, skipping
///   blank lines and other marker-only lines, marker line excluded — except
///   directly below a paragraph or list-item line (no blank), where it JOINS
///   that block, marker line included (lazy continuation). A blank-separated
///   id below a list hosts the whole contiguous list.
/// - **No preceding block** (document start / only frontmatter above) or a
///   caret inside the frontmatter (literal YAML): the marker's own line —
///   the anchor stays resolvable at line grain; the face planes exclude the
///   frontmatter case.
fn anchor_host_span(
    bytes: &[u8],
    marker: &ByteSpan,
    hosts: &[ByteSpan],
    fm: Option<&ByteSpan>,
) -> ByteSpan {
    let line = line_around(bytes, marker.start);
    let in_fm = |l: &ByteSpan| fm.is_some_and(|f| f.start <= l.start && l.end <= f.end);
    if in_fm(&line) {
        return line;
    }
    let own_line = bytes[line.start..marker.start]
        .iter()
        .all(u8::is_ascii_whitespace);
    if !own_line {
        // Tail id: the enclosing leaf block.
        if let Some(h) = smallest_host(hosts, line.start) {
            return h;
        }
        if is_list_line(bytes, &line) {
            return line;
        }
        return paragraph_run(bytes, hosts, fm, &line, &line);
    }
    // Own-line id: scan upward to the nearest content line; blank lines and
    // other marker-only lines are transparent to attachment.
    let mut gap = false;
    let mut probe = line.clone();
    loop {
        let Some(prev) = prev_line(bytes, &probe) else {
            return line; // document start: the marker keeps its own line
        };
        if in_fm(&prev) {
            return line; // only frontmatter above: the marker keeps its line
        }
        if is_blank(bytes, &prev) {
            gap = true;
            probe = prev;
            continue;
        }
        if is_marker_only_line(bytes, &prev) {
            probe = prev;
            continue;
        }
        if let Some(h) = smallest_host(hosts, prev.start) {
            return h;
        }
        if is_list_line(bytes, &prev) {
            // Direct-below joins the ITEM (item + marker line); a blank gap
            // attaches to the whole contiguous list.
            return if gap {
                list_run(bytes, &prev)
            } else {
                prev.start..line.end
            };
        }
        // Paragraph target: a direct-below marker joins the run (lazy
        // continuation, marker line included); a gap keeps it outside.
        let last = if gap { prev.clone() } else { line };
        return paragraph_run(bytes, hosts, fm, &prev, &last);
    }
}

/// The line containing byte `pos`: start-of-line → line end, terminator
/// excluded (`\n` and a preceding `\r`).
fn line_around(bytes: &[u8], pos: usize) -> ByteSpan {
    let start = bytes[..pos]
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(0, |p| p + 1);
    let mut end = bytes[pos..]
        .iter()
        .position(|&b| b == b'\n')
        .map_or(bytes.len(), |p| pos + p);
    if end > start && bytes[end - 1] == b'\r' {
        end -= 1;
    }
    start..end
}

/// The line before the one starting at `cur.start`, terminator-excluded.
fn prev_line(bytes: &[u8], cur: &ByteSpan) -> Option<ByteSpan> {
    if cur.start == 0 {
        return None;
    }
    Some(line_around(bytes, cur.start - 1))
}

fn is_blank(bytes: &[u8], line: &ByteSpan) -> bool {
    bytes[line.clone()].iter().all(u8::is_ascii_whitespace)
}

/// A line that is exactly one block-id marker (the scanner's own-line shape):
/// optional whitespace, `^` + `[A-Za-z0-9-]+`, optional trailing whitespace.
fn is_marker_only_line(bytes: &[u8], line: &ByteSpan) -> bool {
    let s = &bytes[line.clone()];
    let s = trim_ascii(s);
    let Some(rest) = s.strip_prefix(b"^") else {
        return false;
    };
    !rest.is_empty() && rest.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'-')
}

fn trim_ascii(mut s: &[u8]) -> &[u8] {
    while let [b, rest @ ..] = s
        && b.is_ascii_whitespace()
    {
        s = rest;
    }
    while let [rest @ .., b] = s
        && b.is_ascii_whitespace()
    {
        s = rest;
    }
    s
}

/// A list-item line: optional indent, then a `-`/`*`/`+` bullet or an
/// ordered `1.`/`1)` marker followed by a space or line end.
fn is_list_line(bytes: &[u8], line: &ByteSpan) -> bool {
    let s = trim_ascii(&bytes[line.clone()]);
    if s.starts_with(b"- ") || s.starts_with(b"* ") || s.starts_with(b"+ ") {
        return true;
    }
    if s == b"-" || s == b"*" || s == b"+" {
        return true;
    }
    let digits = s.iter().take_while(|b| b.is_ascii_digit()).count();
    digits > 0
        && s.get(digits).is_some_and(|b| *b == b'.' || *b == b')')
        && s.get(digits + 1).is_none_or(|b| *b == b' ')
}

/// The innermost host block containing byte `pos` (largest start; shortest
/// span on a tie).
fn smallest_host(hosts: &[ByteSpan], pos: usize) -> Option<ByteSpan> {
    hosts
        .iter()
        .filter(|h| h.start <= pos && pos < h.end)
        .max_by(|a, b| {
            a.start
                .cmp(&b.start)
                .then((b.end - b.start).cmp(&(a.end - a.start)))
        })
        .cloned()
}

/// The maximal contiguous paragraph run through `first`..`last`: extended
/// over adjacent lines that are non-blank, outside every host block and the
/// frontmatter, and not list-item lines. Returns `run_start..run_end`
/// terminator-excluded.
fn paragraph_run(
    bytes: &[u8],
    hosts: &[ByteSpan],
    fm: Option<&ByteSpan>,
    first: &ByteSpan,
    last: &ByteSpan,
) -> ByteSpan {
    let in_run = |l: &ByteSpan| {
        !is_blank(bytes, l)
            && smallest_host(hosts, l.start).is_none()
            && !fm.is_some_and(|f| f.start <= l.start && l.end <= f.end)
            && !is_list_line(bytes, l)
    };
    let mut start = first.clone();
    while let Some(prev) = prev_line(bytes, &start) {
        if !in_run(&prev) {
            break;
        }
        start = prev;
    }
    let mut end = last.clone();
    while let Some(next) = next_line(bytes, &end) {
        if !in_run(&next) {
            break;
        }
        end = next;
    }
    start.start..end.end
}

/// The line after the one ending at `cur.end`, terminator-excluded; `None`
/// at end of input.
fn next_line(bytes: &[u8], cur: &ByteSpan) -> Option<ByteSpan> {
    let mut pos = cur.end;
    if pos < bytes.len() && bytes[pos] == b'\r' {
        pos += 1;
    }
    if pos < bytes.len() && bytes[pos] == b'\n' {
        pos += 1;
    } else {
        return None; // unterminated last line
    }
    if pos >= bytes.len() {
        return None;
    }
    Some(line_around(bytes, pos))
}

/// The whole contiguous list around item line `item`: adjacent non-blank
/// lines that are list-item lines or indented continuations.
fn list_run(bytes: &[u8], item: &ByteSpan) -> ByteSpan {
    let in_list = |l: &ByteSpan| {
        !is_blank(bytes, l)
            && (is_list_line(bytes, l)
                || bytes[l.clone()]
                    .first()
                    .is_some_and(|b| *b == b' ' || *b == b'\t'))
    };
    let mut start = item.clone();
    while let Some(prev) = prev_line(bytes, &start) {
        if !in_list(&prev) {
            break;
        }
        start = prev;
    }
    let mut end = item.clone();
    while let Some(next) = next_line(bytes, &end) {
        if !in_list(&next) {
            break;
        }
        end = next;
    }
    start.start..end.end
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
/// the §1 leaf-block law, and the exact inverse of [`extend_terminator`]. Full
/// terminator, never a naive `-1`: `\n` ⇒ 1, `\r\n` ⇒ 2, a terminator-less
/// line ⇒ 0.
fn trim_terminator(bytes: &[u8], start: usize, mut end: usize) -> usize {
    if end > start && bytes[end - 1] == b'\n' {
        end -= 1;
        if end > start && bytes[end - 1] == b'\r' {
            end -= 1;
        }
    }
    end
}

/// Model's own flat-frontmatter parse (top-level `key: value`, column 0) — the
/// keys authority, not `syntax`'s best-effort list. No serde/YAML crate
/// (no-serde crate law). Key order is document order, first occurrence wins —
/// the wire `keys` surface echoes it verbatim (B1 predicate 4).
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
/// model kind yet and are dropped.
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

// resolve

/// One hpath segment — heading text + optional 1-based occurrence among
/// identical siblings (contract §2.1). Model twin of `wire::HpathSeg` (no shared
/// type; the host converts).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HpathSeg {
    /// Heading text, matched **byte-exactly** against the containment tree
    /// (the mint plane never case-folds — that is the walk plane's job).
    pub h: String,
    /// 1-based occurrence among identical sibling heading texts. `None` demands
    /// uniqueness: a duplicate with no disambiguator resolves `Ambiguous`
    /// (contract §2.1) — the mint plane never silently picks.
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
/// contract §2.4). `model` is wire-blind, so this is the model-side refusal; the
/// dispatch boundary maps it to the wire `bad_request`. The walk plane does not
/// use it — there the same id is silently dropped (decision 013).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadAnchorId {
    pub id: String,
}

impl Ref {
    /// Build a mint-plane anchor ref, enforcing the one block-id charset. Every
    /// mint position that builds an anchor ref from untrusted input routes
    /// through here rather than `Ref::Anchor` directly.
    ///
    /// # Errors
    /// [`BadAnchorId`] when `id` is empty or bears a char outside `[A-Za-z0-9-]`.
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
    /// Duplicate hpaths — carries the candidate list.
    Ambiguous(Vec<Target>),
}

/// The strict mint-plane lookup (contract §2.1) backing `cat`/`toc`/splice
/// targets: an `hpath` walked byte-exactly down the containment tree, an exact
/// block `anchor`, or a top-level `fm_key`. Section span = heading line through
/// end of subtree. Never case-folds, never silently picks — that is the [`walk`]
/// plane's job.
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
/// token. The content span backs `put{at:"content"}`; the full span backs
/// `put{at:"all"}`, `put{at:"end"}` and `match` (§4.4). Internal to validation —
/// the public [`resolve`] projects it to [`Target`].
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
/// content span is its full span.
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

/// A top-level frontmatter key → its full value grain (span + rev): the key line
/// plus any indented continuation lines, never the whole fence-to-fence block.
///
/// A scalar or flow value (`title: Plan`, `tags: [a, b]`) has no continuation,
/// so the grain is the single key line, terminator-excluded (§1 leaf law; frozen
/// §4.4 `[4,15]`). A block value extends the grain over every item, so an
/// upsert/replace addresses the whole value and can never orphan the tail.
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
        // top-level keys sit at column 0; match byte-exactly up to the colon.
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

/// The end byte (terminator included) of the frontmatter line starting at
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

/// The full grain span of a top-level frontmatter key whose line starts at
/// `key_line_start`: the key line, extended over every indented continuation
/// line of a block value. A blank line is carried only when a later indented
/// line follows — trailing blanks belong to the inter-key gap. The scan stops at
/// the next column-0 non-blank line or the block end; the returned end excludes
/// the last content line's terminator (§1 leaf law).
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
            grain_end = trim_terminator(bytes, cursor, line_end);
        } else {
            // column-0 non-blank: the next key or the closing fence.
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

/// Plan a frontmatter-key upsert against the pre-batch document — the
/// create-or-replace site for `{key}: {value}` (`PutAt::Upsert`). The insertion
/// offset is server-derived (never a client byte offset, D-C1):
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
    // A create has no prior node: the honest before-rev is the born-from-nothing
    // token, minted once for every plane's birth.
    let empty_rev = born_before_rev();
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

/// The pre-batch armed before-fact for an `fm_key` upsert target: the existing
/// key line's span + rev when present, else the empty insertion point's span +
/// the empty-span rev (a create has no prior node). `value` does not affect it.
#[must_use]
pub fn fm_upsert_before(doc: &Document, key: &str) -> Target {
    let plan = plan_fm_upsert(doc, key, "");
    Target {
        span: plan.target_span,
        node_rev: plan.before_rev,
    }
}

/// The born-from-nothing before-token — blake3("")[:16]
/// (`af1349b9f5f9a1a6`). Every birth's armed `node_rev_before`
/// (wire-contract A.6.3a′ teaching row and the create-door law): not a claim
/// that an empty node existed — the op says birth, the token says
/// born-from-nothing.
#[must_use]
pub fn born_before_rev() -> NodeRev {
    node_rev(&[], &(0..0))
}

// CAS-splice validation — here; execution in `fs`

/// Where a `put` writes within its resolved target (contract §4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PutAt {
    /// Replace the full span, heading included.
    All,
    /// Replace the content span, heading preserved (§1 content-span law; ==
    /// full span for a headingless target — see [`content_span`]).
    Content,
    /// Insert `text` at the span-end byte — raw byte concatenation, no
    /// synthesized separator (§4.4 `at:"end"` law). `text` that must begin a new
    /// line carries its own leading `\n`; a result that loses containment
    /// refuses [`SpliceVerdict::WouldCorrupt`].
    End,
    /// Set a frontmatter key (create-or-replace) — valid only on a
    /// [`Ref::FmKey`] target. `text` is the value; the server composes
    /// `{key}: {value}` and replaces, inserts, or synthesizes the `---` block
    /// (see [`plan_fm_upsert`]). The insertion offset is derived from the
    /// document — never client-supplied (D-C1).
    Upsert,
}

/// The two edit shapes (contract §4.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditKind {
    /// Edit-exact: `old` must occur exactly once in the target's full span
    /// bytes; replaced by `new`. Zero → `no_match`, two+ → `not_unique{matches}`
    /// (§5.2). No regex, no fuzz; matched server-side.
    Match { old: String, new: String },
    /// Whole-slot write at a [`PutAt`] position.
    Put { at: PutAt, text: String },
}

/// One edit in a batch: a target ref, the edit shape, an optional per-node CAS
/// guard. There is no span field — a client cannot supply a byte offset, so the
/// wrong-offset write is unrepresentable (D-C1, §4.4). The model twin of
/// `wire::Edit`; the crates never share a type (no-serde law).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub target: Ref,
    pub edit: EditKind,
    /// Node-grain guard (§5.1): compared against `blake3(target's full span
    /// bytes)[:16]` re-derived from the pre-batch state. `None` = unguarded (a
    /// legal wire frame — requiredness is the Go ratchet, §5.3).
    pub if_node_rev: Option<NodeRev>,
}

/// A batch splice request (contract §4.4 — `splice` is batch-only, one response
/// shape). Every target and guard resolves against the pre-batch state; targets
/// must be disjoint. There is no span field anywhere request-side (D-C1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpliceRequest {
    /// World-grain guard, checked first (§5.1) — a mismatch fails the whole
    /// batch (`root_mismatch` → resync). `None` = unguarded.
    pub if_root: Option<MerkleRoot>,
    pub edits: Vec<Edit>,
    /// The one engine-minted span edit riding inside the same batch.
    /// `None` for every caller-shaped batch.
    pub engine: Option<EngineEdit>,
}

/// Engine-minted span edit inside the batch — not resolved from a caller
/// [`Ref`]. Sole inhabitant: `meridian-lock`, whose fenced block the §2.1
/// grammar cannot address. No wire shape produces this, so clients cannot mint
/// one (D-C1). Validated with the planned edits (char-align, disjointness,
/// reparse).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineEdit {
    /// The pre-batch byte span these bytes replace (empty span = an insert).
    pub span: ByteSpan,
    /// The replacement bytes, already canonical (the minter renders them).
    pub text: String,
}

/// A receipt append riding inside the sealed batch (§6.1, D-C3): the receipt
/// file's append position (its EOF) and the pre-rendered line bytes. Folded in
/// by the caller before validation, so the append commits in the same batch and
/// single root advance as the content edits. Model seals it, never renders it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptAppend {
    pub span: ByteSpan,
    pub text: String,
}

/// One validated edit: the exact pre-batch byte span to replace and the
/// replacement text — the write instruction `fs` executes. Spans index the
/// pre-batch bytes and the batch's edits are disjoint, so `fs` applies them in
/// one pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedEdit {
    pub span: ByteSpan,
    pub text: String,
    /// Batch-order origin: which request edit this sealed edit came from
    /// (§4.4 armed↔request alignment — the armed-fact builder reads it back
    /// to locate a birth's landed bytes). The engine-minted pin edit, when
    /// present, indexes one past the caller's edits.
    pub index: usize,
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
    /// `if_root` failed (world-grain, checked first) — the whole batch is
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
    /// Guard passed and `old` was absent → `no_match` (fix, §5.2). `matches` is
    /// 0 (carried for wire-frame parity).
    NoMatch { matches: usize },
    /// `old` occurred 2+ times → `not_unique{matches}` (fix, §5.2).
    NotUnique { matches: usize },
    /// Two edits' replaced regions are not disjoint → `bad_request{overlap}`
    /// (§4.4 region grain). `edits` are the offending pair's batch-order
    /// indices (the engine-minted edit, when present, indexes one past the
    /// caller's edits); `spans` are their replaced regions.
    Overlap {
        edits: Vec<usize>,
        spans: Vec<ByteSpan>,
    },
    /// The post-apply reparse loses containment →
    /// `would_corrupt{containment_lost}` (§4.4). `lost` = the hpath chains of
    /// sections destroyed outside the edited regions (byte-disjoint from every
    /// replaced region, yet gone after reparse: corrupted, not rewritten).
    /// `cause` is what the reparse MEASURED, and is `None` when the lost
    /// sections do not share one — the remedy a face teaches computes from it,
    /// so an unmeasured cause must teach nothing rather than guess.
    WouldCorrupt {
        lost: Vec<Vec<String>>,
        cause: Option<CorruptCause>,
    },
    /// An edit wrote past the span it named →
    /// `would_corrupt{transition_unrepresentable}` (§4.4, `decisions/0018`).
    /// §4.4 makes the target's span the region an edit rewrites and §1 excludes
    /// a leaf's line terminator from that span, so a `text` ending in a
    /// separator places bytes the node never covers. Those bytes cannot move
    /// `node_rev` (a function of the node's span bytes, node-rev-merkle-spec
    /// §2), so the armed transition would be a lie and `if_node_rev` would
    /// compare a constant. `target` is the offending edit's own ref.
    TransitionUnrepresentable { target: Ref },
    /// A replaced region would split a multi-byte UTF-8 character →
    /// `bad_request` (§1 write-side multibyte refusal). Unreachable through the
    /// public `match`/`put` API — valid-UTF-8 edits at char-aligned resolved
    /// spans are self-synchronizing — but retained so any mid-char region
    /// refuses loudly rather than corrupting bytes.
    MultibyteSplit,
}

/// WHY a byte-disjoint section stopped resolving, measured on the post-apply
/// reparse — never inferred from the edit text. The two causes want unlike
/// remedies, and one refusal that teaches the other's is a taught-recovery
/// loop: following it repairs nothing and the caller resends the same batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorruptCause {
    /// The heading line no longer parses as a heading at all — its text is not
    /// a heading of its own level anywhere in the reparse. The commonest source
    /// is `at:"end"` glue (raw byte concatenation, §4.4).
    HeadingDestroyed,
    /// The heading still parses at its own level, but its ancestry moved, so
    /// the hpath no longer resolves — a heading in the written text adopted the
    /// sections that follow it.
    Reparented,
}

/// Batch that passed validation. Only `model` mints (private `_sealed`); `fs`
/// accepts only this type — unvalidated batches cannot reach disk. Receipt
/// rides inside (§6.1).
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

/// Write-plane candidate — bytes about to land. Inner `Document` is private;
/// the only mint path is [`candidate_of_body`] / [`candidate_of_batch`], and
/// `fs` byte-landing primitives accept nothing else. Seal is compile-level:
///
/// ```compile_fail
/// // The field is private, so this fails to compile outside `model`.
/// let doc = model::build(String::new(), Vec::new());
/// let _ = model::CandidateDocument(doc);
/// ```
///
/// Access via [`CandidateDocument::document`] / [`raw`](CandidateDocument::raw) /
/// [`into_document`](CandidateDocument::into_document).
#[derive(Debug, Clone)]
pub struct CandidateDocument(Document);

impl CandidateDocument {
    /// The candidate document, borrowed.
    #[must_use]
    pub fn document(&self) -> &Document {
        &self.0
    }

    /// The exact bytes this write is about to land.
    #[must_use]
    pub fn raw(&self) -> &str {
        &self.0.raw
    }

    /// The candidate document, taken.
    #[must_use]
    pub fn into_document(self) -> Document {
        self.0
    }
}

/// Mint the candidate for a whole-file write: parse `raw` — the exact bytes that
/// will reach disk — and stamp the document's own path. `build` is I/O-free and
/// leaves the path empty, but the armed gate scopes its rules by that value, so
/// an unstamped candidate is invisible to every path-scoped convention.
#[must_use]
pub fn candidate_of_body(path: &str, raw: String) -> CandidateDocument {
    let nodes = syntax::parse(&raw);
    let mut doc = build(raw, nodes);
    if let NodeKind::Document { path: p, .. } = &mut doc.root.kind {
        p.clear();
        p.push_str(path);
    }
    CandidateDocument(doc)
}

/// Mint the candidate a sealed batch produces: dry-apply the validated span
/// edits to `pre_image` (the bytes the spans index) and parse the result once
/// (§4.4's one-reparse law). The single owner of apply-then-reparse.
#[must_use]
pub fn candidate_of_batch(
    path: &str,
    pre_image: &str,
    sealed: &ValidatedBatch,
) -> CandidateDocument {
    let mut raw = pre_image.to_owned();
    // The sealed edits are disjoint and sorted by span start, so splicing
    // back-to-front keeps every remaining span in pre-image coordinates.
    for edit in sealed.edits.iter().rev() {
        raw.replace_range(edit.span.clone(), &edit.text);
    }
    candidate_of_body(path, raw)
}

/// Validate a batch splice against a live `Document` (contract §4.4/§5). The
/// order (§5.1): `if_root` first (world-grain, fails the whole batch), then per
/// edit resolve → CAS → match/put, then batch-wide disjointness and one
/// simulated reparse (`would_corrupt`). `live_root` is the caller's ambient
/// corpus root for the `if_root` comparison (`None` = not guarding the world
/// root, §5.3); `receipt` is a pre-rendered append that rides inside the sealed
/// batch. On success, mints the sealed [`ValidatedBatch`] — the only path to
/// `fs`.
#[must_use]
// SUPPRESSION, NOT A FIX (9eca88f7, 2026-08-10, authorized by fix/board 5d8bce96).
// This is 102 lines against a threshold of 100. Splitting corruption-detection
// code is the wrong change to make while `cargo test --workspace` is dark — it
// had not executed in CI since 2026-08-09T10:09:33Z, and this `-D warnings` lint
// was the last step standing between the repo and its test lane.
// REMOVE THIS ATTRIBUTE when whoever owns `crates/model` splits the §5.1 ordering
// into named steps with a green suite in front of them. The lint is stylistic;
// nothing here is suppressed for correctness.
#[allow(clippy::too_many_lines)]
pub fn validate_batch(
    doc: &Document,
    live_root: Option<&MerkleRoot>,
    batch: &SpliceRequest,
    receipt: Option<ReceiptAppend>,
) -> SpliceVerdict {
    // 1. World guard (§5.1): compared only when the client guarded and the
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
    // The rev-on-change guard's inputs, index-aligned with the caller's edits:
    // each edit's own target and the pre-batch rev it resolved to. `None` for
    // the `fm_key` upsert door, whose before-rev is legitimately the empty
    // insertion point on a create — a move from nothing is a real transition.
    let mut guarded: Vec<Option<(Ref, NodeRev)>> = Vec::with_capacity(batch.edits.len());
    for edit in &batch.edits {
        // Upsert on an `fm_key` target plans before the resolve gate: the key
        // may not exist yet, and resolve would refuse it `RefNotFound`. CAS
        // compares the honest before-rev (§5.1) — the existing line's rev, or
        // the empty insertion point's rev for a create.
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
                region: plan.region,
                text: plan.text,
            });
            guarded.push(None);
            continue;
        }
        let resolved = match resolve_full(doc, &edit.target) {
            Ok(r) => r,
            Err(ResolveError::NotFound) => return SpliceVerdict::RefNotFound,
            Err(ResolveError::Ambiguous(c)) => return SpliceVerdict::Ambiguous(c),
        };
        // CAS (§5.1): blake3 of the target's full span bytes, already minted as
        // `node_rev`.
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
        // char boundaries.
        if let Err(v) = guard_char_aligned(raw, &region) {
            return v;
        }
        planned.push(PlannedEdit { region, text });
        guarded.push(Some((edit.target.clone(), resolved.node_rev.clone())));
    }

    // 2b. The engine-minted span edit joins the planned set after the caller's
    // edits, before every batch-wide rung.
    if let Some(engine) = &batch.engine {
        match plan_engine_edit(raw, engine) {
            Ok(planned_engine) => planned.push(planned_engine),
            Err(v) => return v,
        }
    }

    // 3. Disjointness (§4.4, region grain): the replaced regions must not
    // overlap. Targets may nest — an append to a child and an append under its
    // parent rewrite different bytes and compose legally.
    if let Some((edits, spans)) = first_overlap(&planned) {
        return SpliceVerdict::Overlap { edits, spans };
    }

    // 4. One simulated reparse (§4.4), shared by both post-reparse guards — the
    // one-reparse law is why they take the built document rather than each
    // building their own.
    let new_raw = apply_regions(raw, &planned);
    let new_doc = build(new_raw.clone(), syntax::parse(&new_raw));

    // 4a. A post-apply parse that loses containment refuses `would_corrupt`.
    if let Some((lost, cause)) = would_corrupt(doc, &planned, &new_doc) {
        return SpliceVerdict::WouldCorrupt { lost, cause };
    }

    // 4b. An edit must write INSIDE the span it named (decisions/0018; §4.4
    // with node-rev-merkle-spec §2). A leaf's span excludes its line
    // terminator (§1, and §4.4 for `fm_key`), so its extent ends there and a
    // `text` carrying a separator writes a byte the node never covers. Those
    // bytes cannot change the node's bytes, so `node_rev` cannot move and
    // `if_node_rev` would guard a constant — two callers holding the same rev
    // both write, both succeed, and neither is told.
    if let Some(target) = first_write_past_its_span(raw, &planned, &new_doc, &guarded) {
        return SpliceVerdict::TransitionUnrepresentable { target };
    }

    // Mint the sealed batch — edits in pre-batch offset order, each stamped
    // with its batch-order origin (the sort is stable, so same-point inserts
    // keep request order and the stamp survives it).
    let mut edits: Vec<ValidatedEdit> = planned
        .into_iter()
        .enumerate()
        .map(|(index, p)| ValidatedEdit {
            span: p.region,
            text: p.text,
            index,
        })
        .collect();
    edits.sort_by_key(|e| e.span.start);
    SpliceVerdict::Validated(ValidatedBatch {
        edits,
        receipt,
        _sealed: (),
    })
}

/// A resolved-and-planned edit: the replaced byte region (the §4.4
/// disjointness grain) and the replacement text.
struct PlannedEdit {
    region: ByteSpan,
    text: String,
}

/// Locate the exactly-one occurrence of `old` within the target's full span
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
    // Non-overlapping, left→right: 1 + the count over the remaining tail.
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

/// Plan the one [`EngineEdit`]: its span is its address, so nothing resolves.
/// An out-of-range span is still refused by the char-alignment guarantor
/// (`is_char_boundary` is false past the end), so a mis-minted span can never
/// splice into invented bytes.
fn plan_engine_edit(raw: &str, engine: &EngineEdit) -> Result<PlannedEdit, SpliceVerdict> {
    guard_char_aligned(raw, &engine.span)?;
    Ok(PlannedEdit {
        region: engine.span.clone(),
        text: engine.text.clone(),
    })
}

/// The first pair of non-disjoint replaced regions (§4.4 region grain), or
/// `None`, as (batch-order indices, regions). Containment of a non-empty
/// region counts as overlap. Touching boundaries (`[a,b)` then `[b,c)`) and
/// zero-width regions at a shared byte are disjoint — same-point inserts
/// apply in request order ([`ValidatedBatch`]'s stable sort).
fn first_overlap(planned: &[PlannedEdit]) -> Option<(Vec<usize>, Vec<ByteSpan>)> {
    let mut idx: Vec<usize> = (0..planned.len()).collect();
    // (start, end) key: at a shared start byte the zero-width insert sorts
    // before the non-empty region, so a boundary insert reads disjoint
    // regardless of batch order — the verdict must be order-independent.
    idx.sort_by_key(|&i| (planned[i].region.start, planned[i].region.end));
    for w in idx.windows(2) {
        let (a, b) = (&planned[w[0]].region, &planned[w[1]].region);
        // sorted by start: overlap iff a extends past b's start. Zero-width
        // regions at one byte satisfy `a.end <= b.start` and read disjoint.
        if a.end > b.start {
            return Some((vec![w[0], w[1]], vec![a.clone(), b.clone()]));
        }
    }
    None
}

/// Apply the planned edits to the raw bytes and reparse; report the hpath chains
/// of any pre-batch section that was byte-disjoint from every replaced region
/// yet no longer resolves — containment lost (§4.4 `would_corrupt`). A section
/// inside an edited region is legitimately rewritten; one outside every edit
/// whose heading a neighbouring edit destroyed (e.g. a separator-less
/// `at:"end"`) is corruption. `None` when containment holds.
/// The cause rides along: it is measured on the same reparse, per lost section,
/// and reported only when every lost section agrees on it.
fn would_corrupt(
    doc: &Document,
    planned: &[PlannedEdit],
    new_doc: &Document,
) -> Option<(Vec<Vec<String>>, Option<CorruptCause>)> {
    let mut lost: Vec<(Vec<String>, CorruptCause)> = Vec::new();
    collect_lost(&doc.root, planned, new_doc, &mut lost);
    if lost.is_empty() {
        return None;
    }
    let first = lost[0].1;
    let cause = lost.iter().all(|(_, c)| *c == first).then_some(first);
    Some((lost.into_iter().map(|(h, _)| h).collect(), cause))
}

/// The first caller edit that changed bytes yet left its own target's
/// `node_rev` standing still, as that edit's ref — the write-past-the-named-span
/// guard (`decisions/0018`; node-rev-merkle-spec §2 with §4.4). `None` when
/// every byte-changing edit moved the rev of the node it named.
///
/// **The test is the UNMOVED REV, and containment is its explanation rather
/// than its predicate — the two are NOT equivalent.** A write may place some
/// bytes outside its target and still move that target's rev (an `at:"end"`
/// section append whose text opens a sibling heading shrinks the section and
/// grows it by the separator): the transition is then truthful, the guard is
/// live, and nothing is owed. What cannot stand is a changed file over a rev
/// that did not move — then the node never received the bytes at all,
/// `if_node_rev` compares a constant, and two callers holding that rev both
/// write, both succeed, and neither is told.
///
/// **Keyed on the mechanism, never on the `at:` scope**, and that bound is
/// measured: the escape is reachable through `at:"end"`, `at:"all"`,
/// `at:"content"` AND `match` — and `match` is not an `at:` scope at all, so a
/// guard enumerating scopes misses it by construction. A leaf's span excludes
/// its line terminator (§1), so its extent ENDS there and a `text` carrying a
/// separator writes a byte the node never covers.
///
/// An edit whose text equals the bytes it replaces is skipped: it is a lawful
/// no-op and its rev standing still is the truth. A target that stopped
/// resolving is NOT this guard's finding — that is the `target_identity` death,
/// measured at the door that arms the facts.
fn first_write_past_its_span(
    raw: &str,
    planned: &[PlannedEdit],
    new_doc: &Document,
    guarded: &[Option<(Ref, NodeRev)>],
) -> Option<Ref> {
    for (i, slot) in guarded.iter().enumerate() {
        let Some((target, before_rev)) = slot else {
            continue;
        };
        let plan = &planned[i];
        // A put whose text equals the bytes it replaces changes nothing, so its
        // rev standing still is the TRUTH rather than a misreport.
        if raw[plan.region.clone()] == *plan.text {
            continue;
        }
        let Ok(after) = resolve_full(new_doc, target) else {
            continue;
        };
        if after.node_rev == *before_rev {
            return Some(target.clone());
        }
    }
    None
}

/// Recurse the pre-batch tree; for each `Section` byte-disjoint from every
/// replaced region, require its hpath to still resolve post-reparse.
fn collect_lost(
    node: &Node,
    planned: &[PlannedEdit],
    new_doc: &Document,
    lost: &mut Vec<(Vec<String>, CorruptCause)>,
) {
    if let NodeKind::Section {
        heading_text,
        level,
    } = &node.kind
    {
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
            // Containment is lost only when the section vanished entirely
            // (`NotFound`). A duplicate heading resolves `Ambiguous` — the
            // section still exists — so a stray duplicate must not poison every
            // byte-disjoint write to the file (refuse-ambiguous-only).
            if matches!(
                resolve_full(new_doc, &Ref::Hpath(segs)),
                Err(ResolveError::NotFound)
            ) {
                // The cause is READ OFF the reparse, never inferred from the
                // edit text: the heading either still parses at its own level
                // somewhere (its ancestry moved) or it stopped being a heading.
                let cause = if heading_survives(&new_doc.root, heading_text, *level) {
                    CorruptCause::Reparented
                } else {
                    CorruptCause::HeadingDestroyed
                };
                lost.push((hpath.clone(), cause));
            }
        }
    }
    for c in &node.children {
        collect_lost(c, planned, new_doc, lost);
    }
}

/// Does a section with this heading text and level exist anywhere in the
/// post-reparse tree? Identity-at-its-own-level, ancestry ignored — that is
/// exactly the difference between a reparented section and a destroyed one.
fn heading_survives(node: &Node, text: &str, level: u8) -> bool {
    if let NodeKind::Section {
        heading_text,
        level: l,
    } = &node.kind
        && heading_text == text
        && *l == level
    {
        return true;
    }
    node.children
        .iter()
        .any(|c| heading_survives(c, text, level))
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

// integrity + corpus index

/// Corpus Merkle root over the hash domain (contract §12.2), as prefixed `Root`
/// token (§12.3). `files` = domain-filtered vault paths + **raw bytes** (leaf =
/// raw hash; parse tree plays no part). Membership is `fs`'s call.
///
/// **Names are raw bytes** (merkle-spec §4/§9 name truthfulness): the path is
/// anything byte-viewable — `&str` (its UTF-8 bytes, identity), `&[u8]` (the
/// exact on-disk `OsStr` bytes). `/` is the segment separator; every other
/// byte, valid UTF-8 or not, is a name byte. Distinct names are distinct
/// leaves by construction — no decode, so no decode can collapse them.
///
/// `version` = domain prefix counter (`0`⇒`b3:`, `1`⇒`b3a:`, …) so domain-rule
/// bumps cannot silently match. Plain integer keeps `model` fs-blind.
///
/// Encoding (§12.2): leaf `blake3(raw)` 32 B; interior `blake3` over children
/// sorted by name bytes, each `uleb128(len)‖name‖type(0x00/0x01)‖hash32`; empty
/// dirs pruned; workspace root name not hashed.
#[must_use]
pub fn merkle_root<N: AsRef<[u8]>>(files: &[(N, &[u8])], version: u32) -> MerkleRoot {
    let leaves: Vec<(&[u8], [u8; 32])> = files
        .iter()
        .map(|(path, bytes)| (path.as_ref(), leaf_digest(bytes)))
        .collect();
    merkle_root_of_leaves(&leaves, version)
}

/// The §12.2 leaf: `blake3(raw)`. The ONE definition, so a caller that has
/// already hashed a file's bytes can hand the digest to
/// [`merkle_root_of_leaves`] instead of the bytes and get the same root by
/// construction rather than by a second implementation agreeing.
#[must_use]
pub fn leaf_digest(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}

/// [`merkle_root`] over leaves that are ALREADY hashed — the same tree, the
/// same encoding, the same root.
///
/// This exists because the daemon re-derives the corpus root far more often
/// than the corpus changes: a currency check that reuses the leaf digest of
/// every unmoved file reads no bytes and copies none, while a file whose stat
/// identity moved is re-read and re-hashed through [`leaf_digest`]. The fold
/// cannot drift from [`merkle_root`]'s, because that function is now this
/// function with a hashing step in front of it.
///
/// `version` and the encoding are exactly [`merkle_root`]'s.
#[must_use]
pub fn merkle_root_of_leaves<N: AsRef<[u8]>>(leaves: &[(N, [u8; 32])], version: u32) -> MerkleRoot {
    let mut tree = MerkleDir::default();
    for (path, digest) in leaves {
        tree.insert(path.as_ref(), *digest);
    }
    let hex = blake3::Hash::from_bytes(tree.fold()).to_hex().to_string();
    MerkleRoot(format!("{}{hex}", root_prefix(version)))
}

/// A directory in the merkle tree — named entries, each a file (its §12.2 leaf
/// digest) or a subdirectory. Names are **raw bytes** (spec §4/§9): the tree
/// never decodes them, so a name that is not valid UTF-8 keeps its identity.
///
/// The file arm holds the 32-byte digest rather than the raw bytes: the fold
/// only ever needed `blake3(raw)`, and keeping the bytes meant every root
/// derivation copied the whole corpus into the tree before hashing it.
#[derive(Default)]
struct MerkleDir {
    entries: BTreeMap<Vec<u8>, MerkleEntry>,
}

enum MerkleEntry {
    File([u8; 32]),
    Dir(MerkleDir),
}

impl MerkleDir {
    /// Insert one file at its `/`-split path (last write wins on a duplicate
    /// path). Empty segments are dropped; a collision between a file and a
    /// directory at one name is ignored in BOTH directions — a path under an
    /// existing file prefix, and a file at a name holding an existing
    /// directory — because a hash domain never mixes a file and a directory
    /// at one name, and neither side may silently destroy the other.
    /// Splitting on the byte `0x2F` is exact: UTF-8 continuation bytes are
    /// ≥ `0x80`, so no multi-byte sequence can hide a `/`.
    fn insert(&mut self, path: &[u8], digest: [u8; 32]) {
        let segs: Vec<&[u8]> = path
            .split(|b| *b == b'/')
            .filter(|s| !s.is_empty())
            .collect();
        let Some((file_name, dirs)) = segs.split_last() else {
            return;
        };
        let mut dir = self;
        for seg in dirs {
            let entry = dir
                .entries
                .entry(seg.to_vec())
                .or_insert_with(|| MerkleEntry::Dir(MerkleDir::default()));
            match entry {
                MerkleEntry::Dir(sub) => dir = sub,
                MerkleEntry::File(_) => return,
            }
        }
        match dir.entries.entry(file_name.to_vec()) {
            btree_map::Entry::Occupied(mut slot) => {
                if let MerkleEntry::File(existing) = slot.get_mut() {
                    *existing = digest;
                }
            }
            btree_map::Entry::Vacant(slot) => {
                slot.insert(MerkleEntry::File(digest));
            }
        }
    }

    /// Fold this directory to its 32-byte node hash (§12.2): children ordered by
    /// raw name bytes (the map's key order — `Vec<u8>` sorts bytewise), encoded
    /// `uleb128(len) ‖ name ‖ type ‖ hash32`, then `blake3` of the buffer.
    /// Empty subdirs contribute nothing.
    fn fold(&self) -> [u8; 32] {
        let mut enc: Vec<u8> = Vec::new();
        for (name, entry) in &self.entries {
            let (is_dir, hash) = match entry {
                MerkleEntry::File(digest) => (false, *digest),
                MerkleEntry::Dir(dir) if !dir.entries.is_empty() => (true, dir.fold()),
                MerkleEntry::Dir(_) => continue, // empty dir pruned (§12.2)
            };
            write_uleb128(&mut enc, name.len());
            enc.extend_from_slice(name);
            enc.push(u8::from(is_dir));
            enc.extend_from_slice(&hash);
        }
        *blake3::hash(&enc).as_bytes()
    }
}

/// Unsigned LEB128 (the §12.2 varint): low 7 bits per byte, high bit = "more".
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

/// The resident corpus name index — derived, disposable model state (law 2).
/// `query` and `policy` borrow it as a capability parameter; neither owns it.
///
/// Houses the vault name/alias index the walk plane's stage 1 needs
/// (`getFirstLinkpathDest` parity, contract §4.5): file basename → paths and
/// frontmatter alias → paths, both lowercased (stage 1 is case-insensitive).
#[derive(Debug, Default, PartialEq, Eq)]
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
    /// The multi-candidate tie-break is a §13.4 pack-pinned unknown: this is a
    /// deterministic source-relative-then-shortest pick, not an oracle answer.
    #[must_use]
    pub fn resolve_linkpath(&self, linkpath: &str, from: &str) -> Option<String> {
        // C-3 (address-grammar § 5.1): a `linkpath` whose head carries a `:`
        // must be root-peeled by the caller first — otherwise the basename
        // fallback turns `sessions:24-01-retro/notes.md` into `notes` and
        // matches the ambient root's `notes.md`. Refusing here makes the
        // fallback intra-root by construction (C-1). Scoped to the head-colon
        // rule only, not the wider `addr::confined`: a `..` spelling is a vault
        // address this seam answers correctly.
        if addr::head_carries_root_separator(linkpath) {
            return None;
        }
        let key = linkpath.trim().trim_end_matches(".md").to_lowercase();
        let base = key.rsplit('/').next().unwrap_or(key.as_str()).to_string();
        let candidates = self
            .by_basename
            .get(&base)
            .or_else(|| self.by_alias.get(&key))?;
        // Subdir linkpath (`a/b`) ⇒ path ending in `a/b.md` (§4.5), not every
        // `b.md`. Match vault-root (`a/b.md`) or nested (`c/a/b.md`); fall back
        // to bare basename only if no qualifier hit (stale subpath best-effort).
        if key.contains('/') {
            let qualified = format!("{key}.md");
            let suffix = format!("/{qualified}");
            let narrowed: Vec<String> = candidates
                .iter()
                .filter(|p| {
                    let lower = p.to_lowercase();
                    lower == qualified || lower.ends_with(&suffix)
                })
                .cloned()
                .collect();
            if !narrowed.is_empty() {
                return pick_source_relative(&narrowed, from);
            }
        }
        pick_source_relative(candidates, from)
    }

    /// Sole address owner: resolve a ref spelling (`meridian-lock` ref,
    /// wikilink, etc.) to a corpus path. Every plane that turns a spelling into
    /// a document calls this — two owners hash two documents.
    ///
    /// Resolution is a mount lookup: parse address, peel root, then run the
    /// precedence against the corpus that root selects —
    /// 1. spelling is a corpus key (full vault path with `.md`);
    /// 2. spelling + `.md` is a corpus key;
    /// 3. [`CorpusIndex::resolve_linkpath`] (basename/alias).
    ///
    /// `mounts` = names this machine binds; `corpus` = roots loaded. Bound but
    /// unreadable ⇒ grey (§ 8 M6), not parse failure / `file_not_found`.
    #[must_use]
    pub fn resolve_ref(
        &self,
        spelling: &str,
        from: &str,
        corpus: &RootedCorpus<'_>,
        mounts: &MountSet,
    ) -> RefResolution {
        let addr = match Addr::parse(spelling) {
            Ok(addr) => addr,
            Err(err) => return RefResolution::Malformed(err),
        };

        let Some(root) = addr.root().cloned() else {
            // Ambient root — majority path.
            return match self.three_rules(spelling, from, corpus.ambient_docs()) {
                Some(path) => RefResolution::Ambient(path),
                None => RefResolution::NotFound {
                    root: None,
                    path: addr.path().to_owned(),
                    selector: addr.has_selector().then(|| addr.selector().to_owned()),
                },
            };
        };

        // S3-R43: unknown name → grey unmounted (declare it); declared-but-
        // unreadable → name the path instead of prescribing a declaration.
        if !mounts.is_bound(&root) {
            return match mounts.unreachable(&root) {
                Some(u) => RefResolution::PathUnseeable {
                    root,
                    path: u.path.clone(),
                    detail: u.detail.clone(),
                },
                None => RefResolution::Unmounted(root),
            };
        }

        // Bound in table but no corpus loaded here — unreachable, never undeclared.
        let Some(mounted) = corpus.root(&root) else {
            return RefResolution::PathUnseeable {
                root,
                path: String::new(),
                detail: "the mount table binds this root, but no corpus for it \
                         was loaded in this process"
                    .to_owned(),
            };
        };

        // Re-confine path to the resolved mount (multi-root has no ambient join).
        if !addr::confined(addr.path()) {
            return RefResolution::Malformed(AddrError::AmbiguousColon {
                found: addr.path().to_owned(),
            });
        }

        // Three rules on the target root only (C-2: no ambient fallback). Miss ⇒
        // file_not_found scoped to that root, not grey.
        match mounted.index.three_rules(addr.path(), from, mounted.docs) {
            Some(path) => RefResolution::Rooted { root, path },
            None => RefResolution::NotFound {
                root: Some(root),
                path: addr.path().to_owned(),
                selector: addr.has_selector().then(|| addr.selector().to_owned()),
            },
        }
    }

    /// Three rules over one root's corpus (key / key+`.md` / linkpath), shared
    /// by the ambient and mounted arms of [`resolve_ref`].
    fn three_rules(
        &self,
        spelling: &str,
        from: &str,
        docs: &BTreeMap<String, Document>,
    ) -> Option<String> {
        if docs.contains_key(spelling) {
            return Some(spelling.to_owned());
        }
        let with_md = format!("{spelling}.md");
        if docs.contains_key(&with_md) {
            return Some(with_md);
        }
        self.resolve_linkpath(spelling, from)
    }
}

/// One mounted root's corpus, as the resolver sees it: its documents and its
/// own name index. Every mounted root is the same shape — the kind taxonomy
/// left the schema (ZT 2026-08-13), and a mounted tree's documents are parsed
/// wherever they came from.
#[derive(Debug)]
pub struct MountedRoot<'a> {
    index: CorpusIndex,
    docs: &'a BTreeMap<String, Document>,
}

impl MountedRoot<'_> {
    /// This root's documents.
    #[must_use]
    pub fn docs(&self) -> &BTreeMap<String, Document> {
        self.docs
    }
}

/// The root-keyed corpus (`docs/address-grammar.md` § 7.2): the ambient root's
/// documents, plus one entry per mounted root, keyed by canonical mount name.
///
/// `config`/`fs` load each root's documents into it before resolution runs, so
/// the resolver needs no paths and `model` needs no filesystem. Borrowed, never
/// owning — a corpus is large and already lives in the caller's hands.
#[derive(Debug)]
pub struct RootedCorpus<'a> {
    ambient: &'a BTreeMap<String, Document>,
    mounted: BTreeMap<MountName, MountedRoot<'a>>,
    /// The filter that decided which ambient paths are in this map, when the
    /// builder supplied it. `None` = this corpus cannot say, never "everything
    /// is in the domain" — see [`RootedCorpus::in_hash_domain`].
    ambient_domain: Option<&'a dyn HashDomain>,
    /// The disk behind the ambient root, when the builder supplied it. `None` =
    /// this corpus cannot say whether a path it does not hold is absent or
    /// merely unhashed — see [`RootedCorpus::on_ambient_disk`].
    ambient_disk: Option<&'a dyn AmbientDisk>,
}

/// The §12 hash-domain question, asked by whoever holds a corpus and answered
/// by whoever built it (`fs::Domain`).
///
/// `model` owns no filesystem, so it owns the QUESTION and never the filter.
/// The question is load-bearing on the colour plane: a corpus is the hash
/// domain, so a target absent from it is *outside sight* rather than *missing*
/// whenever the domain excludes it, and only the domain can tell the two apart
/// (`wire-contract.md` §12.1, verdict-plane clause).
pub trait HashDomain: std::fmt::Debug {
    /// Do this root-relative path's bytes enter the merkle root?
    ///
    /// `false` means "not hashed", never "not addressable": the excluded file
    /// is still served by every door the caller names a path at.
    fn contains(&self, rel: &str) -> bool;
}

/// The existence question — "is there a file at this ambient-root-relative
/// path?" — asked by whoever holds a corpus and answered by whoever built it
/// (`fs::AmbientRootDisk`).
///
/// Same division as [`HashDomain`]: `model` owns no filesystem, so it owns the
/// QUESTION. The question is load-bearing on the colour plane because
/// **absence outranks domain membership** (`wire-contract.md` §12.1,
/// verdict-plane clause; session decision 0049): the corpus map cannot tell an
/// out-of-domain file that is PRESENT from one that is DELETED — both are
/// missing from it for the same reason — and only a disk read separates them. A
/// grey "not in the hash domain" over a file that is not there is a false
/// sentence, and it fails in the certifying direction.
pub trait AmbientDisk: std::fmt::Debug {
    /// Does a file exist at this ambient-root-relative path?
    ///
    /// The path is resolved against the ambient root the corpus was built from,
    /// never against the caller's working directory (session decision 0045: a
    /// named target is assessed by READING it, at the root it resolves under).
    ///
    /// `None` = **this path is not askable here** — it does not spell a location
    /// strictly inside the root, so no read happened. The implementor never
    /// answers `false` for a path it did not read: `false` is a MEASURED
    /// absence and the colour plane turns it into a red.
    fn exists(&self, rel: &str) -> Option<bool>;
}

impl<'a> RootedCorpus<'a> {
    /// The single-root world: an ambient corpus and no mounts.
    #[must_use]
    pub fn ambient(docs: &'a BTreeMap<String, Document>) -> Self {
        RootedCorpus {
            ambient: docs,
            mounted: BTreeMap::new(),
            ambient_domain: None,
            ambient_disk: None,
        }
    }

    /// Record the hash domain the ambient map was built under. Chainable.
    ///
    /// A face that colours pins supplies this; without it the corpus answers
    /// [`in_hash_domain`](Self::in_hash_domain) with `None` and the colour
    /// plane keeps its pre-0034 behaviour rather than guessing.
    #[must_use]
    pub fn with_hash_domain(mut self, domain: &'a dyn HashDomain) -> Self {
        self.ambient_domain = Some(domain);
        self
    }

    /// Record the disk the ambient map was built from. Chainable.
    ///
    /// A face that colours pins supplies this beside
    /// [`with_hash_domain`](Self::with_hash_domain); without it the corpus
    /// answers [`on_ambient_disk`](Self::on_ambient_disk) with `None` and the
    /// colour plane cannot separate an out-of-domain PRESENT target from a
    /// DELETED one, so it says the weaker of the two things rather than guessing.
    #[must_use]
    pub fn with_ambient_disk(mut self, disk: &'a dyn AmbientDisk) -> Self {
        self.ambient_disk = Some(disk);
        self
    }

    /// The ambient hash domain this corpus was built under, when the builder
    /// supplied one — for a face that must carry it further, never to re-answer
    /// [`in_hash_domain`](Self::in_hash_domain) by hand.
    #[must_use]
    pub fn hash_domain(&self) -> Option<&'a dyn HashDomain> {
        self.ambient_domain
    }

    /// Is `path` inside the hash domain of the root it belongs to?
    ///
    /// `None` = **cannot say** — no domain was supplied for that root. Callers
    /// must not read `None` as `true`: the whole point of the question is that
    /// absence from the corpus map means two different things.
    ///
    /// Mounted roots answer `None` today: each is built by its own workspace's
    /// domain and no face carries those filters here yet.
    #[must_use]
    pub fn in_hash_domain(&self, root: Option<&MountName>, path: &str) -> Option<bool> {
        match root {
            None => self.ambient_domain.map(|d| d.contains(path)),
            Some(_) => None,
        }
    }

    /// Is there a file at `path` on the ambient root's disk?
    ///
    /// `None` = **cannot say** — no disk was supplied, exactly as
    /// [`in_hash_domain`](Self::in_hash_domain) answers when no domain was.
    /// Callers must not read `None` as `true`.
    ///
    /// Mounted roots answer `None`: a miss inside a mounted root is already
    /// measured by resolution (`RefResolution::NotFound { root: Some(_), .. }`),
    /// so nothing here needs to re-ask it. The ambient root is the one whose
    /// absence the corpus map cannot express, because the hash domain removes
    /// present files from that map for a reason that has nothing to do with the
    /// disk.
    #[must_use]
    pub fn on_ambient_disk(&self, root: Option<&MountName>, path: &str) -> Option<bool> {
        match root {
            None => self.ambient_disk.and_then(|d| d.exists(path)),
            Some(_) => None,
        }
    }

    /// Bind one mounted root's corpus under its canonical name, building that
    /// root's own name index. Chainable.
    #[must_use]
    pub fn with_root(mut self, name: MountName, docs: &'a BTreeMap<String, Document>) -> Self {
        let mut index = CorpusIndex::new();
        for (path, doc) in docs {
            index.insert(path, doc);
        }
        self.mounted.insert(name, MountedRoot { index, docs });
        self
    }

    /// The ambient root's documents.
    #[must_use]
    pub fn ambient_docs(&self) -> &BTreeMap<String, Document> {
        self.ambient
    }

    /// One mounted root, by canonical name.
    #[must_use]
    pub fn root(&self, name: &MountName) -> Option<&MountedRoot<'a>> {
        self.mounted.get(name)
    }

    /// Every root whose corpus is loaded here.
    pub fn loaded_names(&self) -> impl Iterator<Item = &MountName> {
        self.mounted.keys()
    }
}

/// Root-aware resolution outcome.
///
/// Keep [`Unmounted`] distinct from [`NotFound`]: unmounted = outside sight
/// (grey); missing file in a mounted root = measured absence (`file_not_found`).
///
/// [`Unmounted`]: RefResolution::Unmounted
/// [`NotFound`]: RefResolution::NotFound
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefResolution {
    /// Ambient root → corpus path.
    Ambient(String),
    /// Mounted root → path inside that root (never ambient same-basename).
    Rooted {
        /// Canonical root the address named.
        root: MountName,
        /// Corpus path inside that root.
        path: String,
    },
    /// Root undeclared. **Grey**, never red / `file_not_found`.
    Unmounted(MountName),
    /// Root declared but unreadable (S3-R43). Grey; refusal names the path.
    PathUnseeable {
        root: MountName,
        /// Bound path; empty only when the caller loaded no corpus for a bound root.
        path: String,
        detail: String,
    },
    /// Well-formed, root bound+readable, path missing in that root's corpus.
    /// Carries root/path/selector so the refusal can scope `file_not_found` to
    /// the miss (address-grammar § 5.2 F4). Parts come from the parsed address;
    /// callers must not re-split joined spellings (decision 14).
    NotFound {
        /// Root of the miss — `None` = ambient.
        root: Option<MountName>,
        path: String,
        /// Selector as carried (`None` = page grain).
        selector: Option<String>,
    },
    /// Malformed address, or opaque root with a selector.
    Malformed(AddrError),
}

impl RefResolution {
    /// Corpus path if resolved. Drops root — callers that need root read the variant.
    #[must_use]
    pub fn path(&self) -> Option<&str> {
        match self {
            RefResolution::Ambient(path) | RefResolution::Rooted { path, .. } => Some(path),
            _ => None,
        }
    }

    /// Undeclared root, if any. Not true for [`PathUnseeable`] (declared; S3-R43).
    #[must_use]
    pub fn unmounted(&self) -> Option<&MountName> {
        match self {
            RefResolution::Unmounted(root) => Some(root),
            _ => None,
        }
    }

    /// Did this address name a root that is declared but unreadable here?
    #[must_use]
    pub fn path_unseeable(&self) -> Option<(&MountName, &str, &str)> {
        match self {
            RefResolution::PathUnseeable { root, path, detail } => {
                Some((root, path.as_str(), detail.as_str()))
            }
            _ => None,
        }
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
/// frontmatter parse keeps the `aliases` value as one string, so the inline list
/// `[a, b]` (or a bare single value) is parsed here.
///
/// Public because the sql cache's append delta (`view::store`) must speak the
/// SAME alias keys the resolver indexes — a second parser would drift.
#[must_use]
pub fn doc_aliases(doc: &Document) -> Vec<String> {
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
///
/// Public for the same consumer as [`doc_aliases`]: the sql cache's append
/// delta parses OLD alias values back out of its own frontmatter rows.
#[must_use]
pub fn parse_alias_list(value: &str) -> Vec<String> {
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
/// lexicographically first.
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

    /// A subpath linkpath (`a/b`) resolves to the path ending in `a/b.md`, not to
    /// an unrelated file sharing the basename `b` — getFirstLinkpathDest honors
    /// the subdirs (§4.5).
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
        assert_eq!(
            index.resolve_linkpath("caveman/CAVEMAN", "effects/skills/caveman.md"),
            Some("sources/git/caveman/CAVEMAN.md".to_string()),
            "the subpath qualifier selects the path ending in caveman/CAVEMAN.md",
        );
        assert!(
            index
                .resolve_linkpath("caveman", "effects/skills/caveman.md")
                .is_some(),
            "a bare basename still resolves (source-relative pick over the set)",
        );
        assert!(
            index
                .resolve_linkpath("nowhere/caveman", "sources/caveman.md")
                .is_some(),
            "a stale subpath degrades to the bare-basename resolution",
        );
    }

    #[test]
    fn frontmatter_node_is_fence_to_fence_terminator_inclusive() {
        // §18 row 3: [0,20], not syntax's trimmed [0,19).
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
        // file_rev is defined over whole-file bytes, independent of tree shape.
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
        // The refusal carries the offending lexeme for the wire `bad_request`.
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
        // byte-exact: the mint plane never case-folds.
        assert_eq!(
            resolve(&doc, &Ref::Hpath(vec![seg("goals")])),
            Err(ResolveError::NotFound)
        );
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
        // Frozen §4.4: the fm_key leaf span is terminator-excluded — `[4,15]`,
        // not the fence-to-fence container `[0,20]` (§18 row 3).
        assert_eq!(title.span, 4..15);
        assert_eq!(&doc.raw[title.span.clone()], "title: Plan");
        let independent =
            blake3::hash(&doc.raw.as_bytes()[4..15]).to_hex().as_str()[..16].to_string();
        assert_eq!(title.node_rev.0, independent);
        // the frozen §4.4 `node_rev_before`.
        assert_eq!(independent, "fa77480c79a853bc");
        assert_eq!(
            resolve(&doc, &Ref::FmKey("nope".to_string())),
            Err(ResolveError::NotFound)
        );
    }

    /// GATE 2 (frozen §4.4 dry example): the armed-write transition
    /// `title: Plan` → `title: Plan v2` through resolve → `validate_batch` →
    /// apply lands `span_after` `[4,18]` and `node_rev_after`
    /// `fb49e9df2257fab8`, both terminator-excluded.
    #[test]
    fn fm_key_armed_write_lands_frozen_span_after_and_rev() {
        let doc = build_plan(); // S0
        let before = resolve(&doc, &Ref::FmKey("title".to_string())).expect("title S0");
        assert_eq!(before.span, 4..15);
        assert_eq!(before.node_rev.0, "fa77480c79a853bc");
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
        let after = resolve(&s1, &Ref::FmKey("title".to_string())).expect("title S1");
        assert_eq!(after.span, 4..18);
        assert_eq!(&s1.raw[after.span.clone()], "title: Plan v2");
        let independent =
            blake3::hash(&s1.raw.as_bytes()[4..18]).to_hex().as_str()[..16].to_string();
        assert_eq!(after.node_rev.0, independent);
        assert_eq!(independent, "fb49e9df2257fab8"); // frozen §4.4 node_rev_after
    }

    // U2.11 — frontmatter multi-line block-sequence write grain

    /// Pre-corruption specimen — well-formed `inputs:` block sequence.
    const SPECIMEN_FM_CLEAN: &str = "---\ntype: review\nsession: 22-01-meridian-attestation-module\nowner: \"[[a475ccfc]]\"\nrole: adversary (team-3, workflow-first arm)\nstatus: final\ncreated_at: 2026-07-22T23:45-04:00\ntags: [type/review, round2, cross-review, adversary]\ninputs:\n  - \"results/round2/design-1.md@d8536666b42dc8fd\"\n  - \"results/round2/design-2.md@a895cd0c580edf7b\"\n  - \"results/round2/design-3.md@32ed1508fb3396fa\"\n  - \"decisions/2026-07-22-meridian-go-end-state.md\"\n  - \"decisions/2026-07-22-pin-vocabulary-and-gating.md\"\n  - \"results/design-law-brief.md\"\n  - \"results/round-1-report.md\"\n  - \"results/engine-analysis.md\"\n  - \"[[substrate]]\"\n  - \"[[reconciliation]]\"\n  - \"[[attestation-tournament]]\"\nfinalized_at: 2026-07-23T00:20-04:00\n---\n\n# Cross-review — a475ccfc (team-3 adversary)\n\nbody\n";

    /// Corruption specimen — `finalized_at:` wedged under `inputs:`.
    const SPECIMEN_FM_WEDGED: &str = "---\ntype: review\nsession: 22-01-meridian-attestation-module\nowner: \"[[a475ccfc]]\"\nrole: adversary (team-3, workflow-first arm)\nstatus: final\ncreated_at: 2026-07-22T23:45-04:00\ntags: [type/review, round2, cross-review, adversary]\ninputs:\nfinalized_at: 2026-07-23T00:20-04:00\n  - \"results/round2/design-1.md@d8536666b42dc8fd\"\n  - \"results/round2/design-2.md@a895cd0c580edf7b\"\n  - \"results/round2/design-3.md@32ed1508fb3396fa\"\n  - \"decisions/2026-07-22-meridian-go-end-state.md\"\n  - \"decisions/2026-07-22-pin-vocabulary-and-gating.md\"\n  - \"results/design-law-brief.md\"\n  - \"results/round-1-report.md\"\n  - \"results/engine-analysis.md\"\n  - \"[[substrate]]\"\n  - \"[[reconciliation]]\"\n  - \"[[attestation-tournament]]\"\n---\n\n# Cross-review — a475ccfc (team-3 adversary)\n\nbody\n";

    fn specimen_clean() -> Document {
        build(
            SPECIMEN_FM_CLEAN.to_string(),
            syntax::parse(SPECIMEN_FM_CLEAN),
        )
    }

    /// U2.11: block-sequence `fm_key` grain spans full value (key + indented
    /// items); flow/scalar keep one-line grain (§1 leaf). Line-only grain orphaned blocks.
    #[test]
    fn fm_key_multiline_block_sequence_grain_spans_full_value() {
        let doc = specimen_clean();
        // block sequence → grain covers the key line and all 11 indented items.
        let inputs = resolve(&doc, &Ref::FmKey("inputs".to_string())).expect("inputs fm_key");
        let grain = &doc.raw[inputs.span.clone()];
        assert!(grain.starts_with("inputs:\n  - \"results/round2/design-1.md"));
        assert!(grain.ends_with("  - \"[[attestation-tournament]]\""));
        assert!(
            !grain.contains("finalized_at"),
            "grain must stop before the next top-level key"
        );
        // the rev covers the full block value, not just the key line.
        let independent = blake3::hash(grain.as_bytes()).to_hex().as_str()[..16].to_string();
        assert_eq!(inputs.node_rev.0, independent);

        // `tags:` (flow sequence, single line) → grain is exactly the key line.
        let tags = resolve(&doc, &Ref::FmKey("tags".to_string())).expect("tags fm_key");
        assert_eq!(
            &doc.raw[tags.span.clone()],
            "tags: [type/review, round2, cross-review, adversary]"
        );
        // `status:` (scalar) → grain is exactly the key line.
        let status = resolve(&doc, &Ref::FmKey("status".to_string())).expect("status fm_key");
        assert_eq!(&doc.raw[status.span.clone()], "status: final");
    }

    /// U2.11: the upsert replaces the whole `inputs:` value — the 11 items are
    /// gone, every sibling key survives byte-identical, and the minted rev is
    /// the rev over the clean new value.
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

        // `inputs` is now the intended scalar — no orphaned block items.
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
        // every sibling key survives byte-identical.
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
        let honest = blake3::hash(b"inputs: [design-1, design-2]")
            .to_hex()
            .as_str()[..16]
            .to_string();
        assert_eq!(inputs.node_rev.0, honest);
    }

    /// U2.11 round-trip byte-stability: the grain the reader sees is exactly the
    /// grain the writer replaces (`at:all`), so a no-op patch is a no-op on disk.
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

    /// U2.11: wedged `finalized_at:` empties `inputs` grain; items hang off
    /// `finalized_at`. Corrected grain follows YAML block scope (nearest column-0 key).
    #[test]
    fn fm_wedged_specimen_at_pinned_rev_grain() {
        let doc = build(
            SPECIMEN_FM_WEDGED.to_string(),
            syntax::parse(SPECIMEN_FM_WEDGED),
        );
        // `inputs:` is immediately followed by a column-0 key → empty value grain.
        let inputs = resolve(&doc, &Ref::FmKey("inputs".to_string())).expect("inputs fm_key");
        assert_eq!(&doc.raw[inputs.span.clone()], "inputs:");
        // the block items now hang off `finalized_at:`.
        let fin = resolve(&doc, &Ref::FmKey("finalized_at".to_string())).expect("finalized_at");
        let grain = &doc.raw[fin.span.clone()];
        assert!(
            grain.starts_with(
                "finalized_at: 2026-07-23T00:20-04:00\n  - \"results/round2/design-1.md"
            )
        );
        assert!(grain.ends_with("  - \"[[attestation-tournament]]\""));
    }

    /// Full-terminator trim (`\n`⇒1, `\r\n`⇒2, none⇒0) — §1 leaf law mechanics.
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

    /// On the §6.3 worked S1 receipts fixture, `resolve_anchor` serves the
    /// anchor's host block-leaf — the `^r-000042` list-item line, terminator
    /// excluded (`[26,286]`, rev `60bbee70d4a63a48`) — not the 9-byte
    /// `[277,286]` inline `^id` marker span (§1 / §2.1 / §4.1 / §6.3).
    #[test]
    fn resolve_anchor_serves_host_block_leaf_s1_receipts() {
        let raw = merkle_fixtures().receipts_v1;
        assert_eq!(raw.len(), 287, "S1 receipts fixture is 287 bytes (§0.3)");
        let doc = build(raw.clone(), syntax::parse(&raw));
        let t = resolve(&doc, &Ref::anchor("r-000042").unwrap()).expect("mint resolves ^r-000042");
        assert_eq!(
            t.span,
            26..286,
            "host block-leaf span, not the [277,286] marker"
        );
        assert_eq!(
            t.node_rev.0, "60bbee70d4a63a48",
            "§6.3 rev over the block-leaf bytes"
        );
        assert_ne!(t.span, 277..286, "marker-grain defect must not resurface");
        // independent rev check over exactly the block-leaf bytes.
        let independent =
            blake3::hash(&raw.as_bytes()[26..286]).to_hex().as_str()[..16].to_string();
        assert_eq!(
            t.node_rev.0, independent,
            "rev = blake3(block-leaf bytes)[:16]"
        );
    }

    // validate_batch — §4.4 batch grammar + §5 CAS/failure split

    /// S2 plan (`plan_v2`) — state for §5.2 worked failures.
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
            engine: None,
        }
    }

    /// Reconstruct the applied bytes from a sealed batch's public edits — the
    /// pass `fs` will make.
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
    /// S2 where Q3 is 41f6… → `cas_mismatch{expected,actual}`, refresh.
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

    /// GATE 1b (§5.2 frame 89): guard passes (41f6…), old-string absent →
    /// `no_match{matches:0}`.
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
    /// `- new item`) → `not_unique{matches:2}`.
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

    /// GATE 2 (compile-level): no request-side type carries a byte span — a
    /// `SpliceRequest`/`Edit` cannot name an offset (D-C1). This test compiling
    /// at all is the gate.
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
            engine: _,
        } = batch(vec![e]);
    }

    /// The engine-minted span edit is sealed like any other edit: it lands in
    /// the sealed batch, is checked for disjointness against the caller's edits,
    /// and is char-alignment guarded.
    #[test]
    fn an_engine_edit_rides_the_sealed_batch_and_obeys_every_batch_rung() {
        let raw = "# A\n\nbody\n";
        let doc = build(raw.to_string(), syntax::parse(raw));

        // Alone: an EOF insert seals as one validated edit at that span.
        let mut req = batch(Vec::new());
        req.engine = Some(EngineEdit {
            span: raw.len()..raw.len(),
            text: "```meridian-lock\nversion: 1\n```\n".to_string(),
        });
        let SpliceVerdict::Validated(sealed) = validate_batch(&doc, None, &req, None) else {
            panic!("the engine edit validates");
        };
        assert_eq!(sealed.edits.len(), 1);
        assert_eq!(sealed.edits[0].span, raw.len()..raw.len());

        // Beside a caller edit on a disjoint target: both seal, in offset order.
        let mut both = batch(vec![match_edit(hpath(&["A"]), "body", "BODY", None)]);
        both.engine = req.engine.clone();
        let SpliceVerdict::Validated(sealed) = validate_batch(&doc, None, &both, None) else {
            panic!("caller edit + engine edit validate together");
        };
        assert_eq!(sealed.edits.len(), 2);
        assert!(sealed.edits[0].span.start < sealed.edits[1].span.start);

        // Overlapping the caller's target span: the disjointness rung refuses.
        let mut clash = batch(vec![put_edit(hpath(&["A"]), PutAt::All, "# A\n")]);
        clash.engine = Some(EngineEdit {
            span: 2..4,
            text: "x".to_string(),
        });
        assert!(
            matches!(
                validate_batch(&doc, None, &clash, None),
                SpliceVerdict::Overlap { .. }
            ),
            "an engine edit is not exempt from disjointness"
        );

        // Out of range / mid-character: refused, never spliced into thin air.
        let mut bad = batch(Vec::new());
        bad.engine = Some(EngineEdit {
            span: raw.len()..raw.len() + 5,
            text: "x".to_string(),
        });
        assert_eq!(
            validate_batch(&doc, None, &bad, None),
            SpliceVerdict::MultibyteSplit,
            "a span past the end fails the char-alignment guarantor"
        );

        let uni = "# Ünïcode\n";
        let udoc = build(uni.to_string(), syntax::parse(uni));
        let mut split = batch(Vec::new());
        split.engine = Some(EngineEdit {
            span: 3..4, // inside the two-byte `Ü`
            text: "x".to_string(),
        });
        assert_eq!(
            validate_batch(&udoc, None, &split, None),
            SpliceVerdict::MultibyteSplit
        );
    }

    /// GATE 3 (seal, positive): a valid batch yields the capability token `fs`
    /// demands. The negative half is the `compile_fail` doctest on
    /// [`ValidatedBatch`].
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

    /// The receipt append rides inside the sealed batch (§6.1) so it commits in
    /// the same batch.
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
    /// yields `plan_v1` byte-exact.
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
    /// span-end (EOF) → `plan_v2` by pure byte concatenation, no synthesized
    /// separator.
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
        assert_eq!(vb.edits[0].span, 139..139, "insert at Q4 span-end");
    }

    /// The rev-on-change guard (node-rev-merkle-spec §2), on the host kinds
    /// that carry a terminator-excluding LINE-grain leaf span. `at:"end"`
    /// puts its insertion point ON the terminator, so a `text` opening with
    /// a newline lands in a fresh line the node never covers: the file
    /// changes, the node does not, and the armed transition would be a lie.
    ///
    /// Since the F-R4 host widening, host kind discriminates by GRAIN:
    /// list-item and heading hosts stay line-grain and refuse; a paragraph
    /// host is the whole run, so the same `"\nX"` EXTENDS the paragraph —
    /// the node's bytes change, the rev moves, and the write is accepted
    /// with a true transition (see the paragraph case below).
    #[test]
    fn end_append_escaping_its_node_refuses_on_every_anchor_host() {
        for (name, raw, id) in [
            ("list item", "- item one ^li-1\n- item two\n", "li-1"),
            ("heading line", "# Top ^h-1\n\nbody text\n", "h-1"),
        ] {
            let doc = build(raw.to_string(), syntax::parse(raw));
            let b = batch(vec![put_edit(Ref::anchor(id).unwrap(), PutAt::End, "\nX")]);
            let SpliceVerdict::TransitionUnrepresentable { target } =
                validate_batch(&doc, None, &b, None)
            else {
                panic!("{name} host: an end-append that escapes its node must refuse");
            };
            assert_eq!(
                target,
                Ref::anchor(id).unwrap(),
                "{name}: names the offender"
            );
        }
    }

    /// The F-R4 counterpart: a paragraph host is run-grain, so the end-append
    /// that used to escape the marker's line now lands INSIDE the block — the
    /// appended line becomes paragraph continuation, the node's bytes change,
    /// and the guard sees a true rev transition instead of a constant.
    #[test]
    fn end_append_on_a_paragraph_host_extends_the_run() {
        let raw = "# Top\n\ncompletely different paragraph text here ^zzz-9\n\n## Alpha\n\nbody\n";
        let doc = build(raw.to_string(), syntax::parse(raw));
        let b = batch(vec![put_edit(
            Ref::anchor("zzz-9").unwrap(),
            PutAt::End,
            "\nX",
        )]);
        let SpliceVerdict::Validated(vb) = validate_batch(&doc, None, &b, None) else {
            panic!("a run-grain host accepts the append that extends it");
        };
        let applied = apply_validated(&doc.raw, &vb);
        let new_doc = build(applied.clone(), syntax::parse(&applied));
        let after = resolve(&new_doc, &Ref::anchor("zzz-9").unwrap())
            .expect("anchor still resolves post-append");
        assert_eq!(
            &new_doc.raw[after.span.clone()],
            "completely different paragraph text here ^zzz-9\nX",
            "the appended line joined the host run"
        );
    }

    /// EVERY scope that can reach the escape, measured at v1.0.0 and held here
    /// so no future narrowing can quietly drop one. `at:"end"`, `at:"all"`,
    /// `at:"content"` and `match` all place a byte past a leaf's span when
    /// their text ends in a separator — and `match` is NOT an `at:` scope, so
    /// a guard enumerating scopes would miss it by construction
    /// (`decisions/0018`).
    #[test]
    fn every_scope_that_writes_past_a_leaf_span_refuses() {
        let raw = "- item one ^li-1\n- item two\n";
        let li = || Ref::anchor("li-1").unwrap();
        let cases: Vec<(&str, Edit)> = vec![
            ("at:end", put_edit(li(), PutAt::End, "\nX")),
            (
                "at:all + separator",
                put_edit(li(), PutAt::All, "- item one ^li-1\n"),
            ),
            (
                "at:content + separator",
                put_edit(li(), PutAt::Content, "- item one ^li-1\n"),
            ),
            (
                "match, new ends in a separator",
                Edit {
                    target: li(),
                    edit: EditKind::Match {
                        old: "^li-1".into(),
                        new: "^li-1\n".into(),
                    },
                    if_node_rev: None,
                },
            ),
        ];
        for (name, edit) in cases {
            let doc = build(raw.to_string(), syntax::parse(raw));
            let SpliceVerdict::TransitionUnrepresentable { target } =
                validate_batch(&doc, None, &batch(vec![edit]), None)
            else {
                panic!("{name}: a write past the named span must refuse");
            };
            assert_eq!(target, li(), "{name}: names the offender");
        }
    }

    /// The same escape on an `fm_key` leaf, whose span excludes its terminator
    /// by the §4.4 leaf law — the family is defined over the SPAN LAW, not over
    /// the anchor door.
    #[test]
    fn fm_key_leaf_writes_past_its_span_refuse() {
        let raw = "---\ntitle: Plan\n---\n\n# Top\n\nbody\n";
        let key = || Ref::FmKey("title".into());
        for (name, edit) in [
            ("at:end", put_edit(key(), PutAt::End, "\nowner: zt")),
            (
                "at:all + separator",
                put_edit(key(), PutAt::All, "title: Plan\n"),
            ),
        ] {
            let doc = build(raw.to_string(), syntax::parse(raw));
            let SpliceVerdict::TransitionUnrepresentable { target } =
                validate_batch(&doc, None, &batch(vec![edit]), None)
            else {
                panic!("fm_key {name}: a write past the named span must refuse");
            };
            assert_eq!(target, key());
        }
    }

    /// The NO-fire half, and it is what keeps the guard from being a blanket
    /// ban: a write whose bytes land INSIDE its target commits. Without these
    /// the refusals above would pass just as well on an engine that refused
    /// everything.
    #[test]
    fn writes_that_land_inside_their_node_still_validate() {
        // A section span is newline-inclusive and runs to the next heading, so
        // every scope's bytes land inside it — including a trailing separator.
        let doc = plan_s1();
        let b = batch(vec![put_edit(
            hpath(&["Goals", "Q4"]),
            PutAt::End,
            "- new item\n",
        )]);
        assert!(
            matches!(
                validate_batch(&doc, None, &b, None),
                SpliceVerdict::Validated(_)
            ),
            "at:end on a section writes inside its own span"
        );

        // The same scopes on the leaf, WITHOUT a trailing separator: the bytes
        // are contained, so they commit. This is the line the guard walks.
        let raw = "- item one ^li-1\n- item two\n";
        let li = || Ref::anchor("li-1").unwrap();
        for (name, edit) in [
            ("at:all", put_edit(li(), PutAt::All, "- item ONE ^li-1")),
            (
                "match",
                Edit {
                    target: li(),
                    edit: EditKind::Match {
                        old: "one".into(),
                        new: "ONE".into(),
                    },
                    if_node_rev: None,
                },
            ),
        ] {
            let doc = build(raw.to_string(), syntax::parse(raw));
            assert!(
                matches!(
                    validate_batch(&doc, None, &batch(vec![edit]), None),
                    SpliceVerdict::Validated(_)
                ),
                "{name} without a trailing separator lands inside the node"
            );
        }
    }

    /// GATE 4a (§4.4 disjointness, region grain): two edits whose replaced
    /// regions overlap refuse `overlap` (`bad_request`), carrying the
    /// offending batch indices + regions.
    #[test]
    fn gate4_overlap_bad_request() {
        let doc = plan_s2();
        let b = batch(vec![
            put_edit(hpath(&["Goals"]), PutAt::All, "# Goals\n\nrewritten\n"),
            match_edit(hpath(&["Goals", "Q3"]), "September", "October", None),
        ]);
        let SpliceVerdict::Overlap { edits, spans } = validate_batch(&doc, None, &b, None) else {
            panic!("a whole-tree rewrite plus an edit inside it must refuse overlap");
        };
        assert_eq!(edits, vec![0, 1], "the offending pair, batch order");
        assert_eq!(
            spans,
            vec![20..150, 64..73],
            "Goals rewrite ⊃ Q3 match bytes"
        );
    }

    /// The region-grain mirror of gate 4a: NESTED TARGETS whose replaced
    /// regions touch different bytes compose in one batch (§4.4 amended
    /// 2026-08-06) — an append at the parent's span-end plus a `match` inside
    /// a child is the mixed-batch shape the target-grain law wrongly refused.
    #[test]
    fn gate4_nested_targets_disjoint_regions_validate() {
        let doc = plan_s2();
        let b = batch(vec![
            put_edit(hpath(&["Goals"]), PutAt::End, "x\n"),
            match_edit(hpath(&["Goals", "Q3"]), "September", "October", None),
        ]);
        let SpliceVerdict::Validated(vb) = validate_batch(&doc, None, &b, None) else {
            panic!("nested targets with disjoint regions must validate");
        };
        let applied = apply_validated(&doc.raw, &vb);
        assert_eq!(
            applied,
            format!("{}x\n", doc.raw.replace("September", "October")),
            "match landed in Q3, append landed at Goals span-end"
        );
    }

    /// The F2 mixed-batch shape at model grain: append to a section plus a
    /// sibling-section birth lowered as `put at:"end"` on the PARENT (the
    /// engine's `create` lowering) — one batch, both zero-width inserts land.
    #[test]
    fn gate4_append_plus_sibling_birth_one_batch() {
        let doc = plan_s2();
        let b = batch(vec![
            put_edit(hpath(&["Goals", "Q3"]), PutAt::End, "- probe\n"),
            put_edit(hpath(&["Goals"]), PutAt::End, "\n## Q5\n\nborn\n"),
        ]);
        let SpliceVerdict::Validated(vb) = validate_batch(&doc, None, &b, None) else {
            panic!("append + sibling birth must validate as one batch");
        };
        let applied = apply_validated(&doc.raw, &vb);
        let mut expected = doc.raw.clone();
        expected.insert_str(75, "- probe\n"); // Q3 span-end
        expected.push_str("\n## Q5\n\nborn\n"); // Goals span-end (EOF)
        assert_eq!(applied, expected);

        // Order-independence: the reversed batch validates identically.
        let rev = batch(vec![
            put_edit(hpath(&["Goals"]), PutAt::End, "\n## Q5\n\nborn\n"),
            put_edit(hpath(&["Goals", "Q3"]), PutAt::End, "- probe\n"),
        ]);
        let SpliceVerdict::Validated(vb) = validate_batch(&doc, None, &rev, None) else {
            panic!("the reversed mixed batch must validate too");
        };
        assert_eq!(apply_validated(&doc.raw, &vb), expected);
    }

    /// Two zero-width inserts at ONE byte (append to the last child + birth
    /// under the parent, both at EOF) are disjoint and apply in request order.
    #[test]
    fn gate4_same_point_inserts_apply_in_request_order() {
        let doc = plan_s2();
        let b = batch(vec![
            put_edit(hpath(&["Goals", "Q4"]), PutAt::End, "- tail\n"),
            put_edit(hpath(&["Goals"]), PutAt::End, "\n## Q5\n\nborn\n"),
        ]);
        let SpliceVerdict::Validated(vb) = validate_batch(&doc, None, &b, None) else {
            panic!("same-point inserts must validate");
        };
        assert_eq!(
            apply_validated(&doc.raw, &vb),
            format!("{}- tail\n\n## Q5\n\nborn\n", doc.raw),
            "request order at the shared byte: Q4's tail before Q5's birth"
        );
    }

    /// GATE 4b (§4.4 `would_corrupt`): a separator-less `at:"end"` on a non-final
    /// section bleeds into the next heading and destroys it — the reparse loses
    /// containment → `would_corrupt{lost}`.
    #[test]
    fn gate4_would_corrupt_lost() {
        let doc = plan_s2();
        // `MORE` (no leading \n) inserted at Q3's span-end (= Q4's heading start)
        // yields `…September\n\nMORE## Q4…` — `## Q4` is no longer at line start.
        let b = batch(vec![put_edit(hpath(&["Goals", "Q3"]), PutAt::End, "MORE")]);
        let SpliceVerdict::WouldCorrupt { lost, cause } = validate_batch(&doc, None, &b, None)
        else {
            panic!("heading-destroying append must refuse would_corrupt");
        };
        assert_eq!(
            lost,
            vec![vec!["Goals".to_string(), "Q4".to_string()]],
            "Q4 (disjoint from the edit) was destroyed"
        );
        assert_eq!(
            cause,
            Some(CorruptCause::HeadingDestroyed),
            "`## Q4` stopped parsing as a heading — the newline law's own cause"
        );
    }

    /// GATE 4c (§4.4 `would_corrupt{containment_lost}`, cause `reparented`): a
    /// SHALLOWER heading written into a section's content adopts the sections
    /// that follow it. Their headings still parse — the hpaths do not resolve —
    /// so the measured cause must NOT be the newline-glue one, whose remedy
    /// (carry your own `\n`) cannot repair this batch.
    #[test]
    fn gate4_would_corrupt_reparented() {
        let doc = plan_s2();
        let b = batch(vec![put_edit(
            hpath(&["Goals", "Q3"]),
            PutAt::Content,
            "pre\n\n# Zombie\n\npost\n",
        )]);
        let SpliceVerdict::WouldCorrupt { lost, cause } = validate_batch(&doc, None, &b, None)
        else {
            panic!("a level-1 heading injected into a level-2 body reparents Q4");
        };
        assert_eq!(
            lost,
            vec![vec!["Goals".to_string(), "Q4".to_string()]],
            "Q4 now hangs under Zombie, so Goals/Q4 no longer resolves"
        );
        assert_eq!(
            cause,
            Some(CorruptCause::Reparented),
            "`## Q4` still parses at level 2 — its ancestry moved, nothing was destroyed"
        );
    }

    /// A well-formed `at:"end"` on the same non-final section (leading `\n`
    /// carried by the caller) preserves containment → validates. The mirror of
    /// gate 4b.
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

    /// U2.2 refuse-ambiguous-only: a file with a duplicate heading refuses only
    /// the write that addresses the ambiguous selector — an unambiguous sibling
    /// still validates, and the duplicate is addressable by node index.
    #[test]
    fn duplicate_heading_refuses_ambiguous_only_sibling_serves() {
        // two `## Objective` under `# Task`, plus an unambiguous `## Notes`.
        let raw = "# Task\n\n## Objective\n\nalpha ^a1b2c3\n\n## Objective\n\nbeta ^d4e5f6\n\n## Notes\n\ngamma\n".to_string();
        let doc = build(raw.clone(), syntax::parse(&raw));

        // (a) a write at the ambiguous bare selector refuses, naming both targets.
        let amb = batch(vec![put_edit(
            hpath(&["Task", "Objective"]),
            PutAt::Content,
            "rewritten\n",
        )]);
        let SpliceVerdict::Ambiguous(cands) = validate_batch(&doc, None, &amb, None) else {
            panic!("a write at the ambiguous selector must refuse Ambiguous");
        };
        assert_eq!(cands.len(), 2, "both duplicates are named candidates");

        // (b) a write at the unambiguous sibling still serves — a stray
        // duplicate elsewhere does not poison a byte-disjoint write.
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

        // (c) the duplicate is addressable by node index (`n=`).
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

    /// GATE 6 (§1 write-side multibyte refusal): a replaced region off a UTF-8
    /// char boundary refuses `bad_request`. White-box, because the state is
    /// unrepresentable through the public `match`/`put` API.
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
    /// cleanly.
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

    // merkle_root — §12.2 encoding + §12.3 prefix bump

    // Frozen hex ground truth (independently recomputed).
    const R0_HEX: &str = "74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9";
    const R1_HEX: &str = "7f3b44376c719be236279e168c22fa2f4d346cd6e5da5bcf0784adb72e7c1f12";
    const R2_HEX: &str = "6e866e13b5e65ef9961c050f8a621cf1980b00ee293be650deef5f4dbc6823f0";
    const R0_WRONG_HEX: &str = "75a61c883e372102cfe7d75e94992b9be65e33fbe95956897a4cf2ea45bb8f1b";
    const R_V0_DRAFTS_HEX: &str =
        "f8b4c62ce36f5873eb46db5cdf41db2436a3cba67ec5c47bebadbeaa8fe71ea3";

    // Non-plan fixture bytes, verbatim from the contract §0.3 fixture bytes (docs/wire-contract.md).
    const RECEIPTS_V0: &str = "# Receipts \u{2014} 2026-07-18\n"; // em dash = 3-byte UTF-8
    const GH_README: &str = "# CI notes\n";
    const DRAFT_TMP: &str = "scratch\n";

    /// The six-root corpus fixtures: raw plan/receipts bytes across S0→S2, each
    /// receipts file interpolating the prior root token + the Q3/Q4 section
    /// revs. Byte-length asserts guard the transcription.
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
        // Receipt lines exactly as §6.3 prints them (fingerprint_before + §2.1
        // JSON target form, rebaselined 2026-08-09).
        let receipt_42 = format!(
            "- splice notes/plan.md id=42 actor=agent:b0864fb2 now=2026-07-18T20:31:04Z \
             fingerprint_before=b3:{R0_HEX} edits=1 target.hpath=[{{\"h\":\"Goals\"}},{{\"h\":\"Q3\"}}] match 33d5b0e1b27cb48b->41f643f034e5681f ^r-000042\n"
        );
        let receipts_v1 = format!("{receipts_v0}{receipt_42}");
        let receipt_43 = format!(
            "- splice notes/plan.md id=57 actor=agent:b0864fb2 now=2026-07-18T20:33:41Z \
             fingerprint_before=b3:{R1_HEX} edits=1 target.hpath=[{{\"h\":\"Goals\"}},{{\"h\":\"Q4\"}}] put:end 4b8bc385a58da0e0->f43203a1f0b4c9a3 ^r-000043\n"
        );
        let receipts_v2 = format!("{receipts_v1}{receipt_43}");

        // Transcription guards (contract §0.3 sizes + §6.3 receipt-line bytes).
        assert_eq!(plan_v0.len(), 136);
        assert_eq!(plan_v1.len(), 139);
        assert_eq!(plan_v2.len(), 150);
        assert_eq!(receipts_v0.len(), 26);
        assert_eq!(receipt_42.len(), 261);
        assert_eq!(receipt_43.len(), 263);
        assert_eq!(receipts_v1.len(), 287);
        assert_eq!(receipts_v2.len(), 550);

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
    /// v1's surviving set equals R2's, so the hex repeats — yet the tokens never
    /// compare equal.
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
        // identical hex …
        assert_eq!(
            bump_v1.0.strip_prefix("b3a:"),
            r2.0.strip_prefix("b3:"),
            "bump_hex_equals_R2",
        );
        // … yet the tokens never compare equal.
        assert_ne!(bump_v1, r2, "different prefix ⇒ tokens never equal");
        assert_ne!(bump_v0, bump_v1);
    }

    /// §9 name truthfulness, the red gate (6b ruling, 2026-08-08): two
    /// DISTINCT non-UTF-8 names produce DISTINCT leaves, therefore distinct
    /// roots. At the pre-fix pin this was inexpressible — the fold took `&str`,
    /// so the fs layer lossy-decoded names first, and both names below decode
    /// to ONE replacement string (`a\u{FFFD}.md`): one leaf, last write wins,
    /// a content change could leave the fingerprint unmoved.
    #[test]
    fn distinct_non_utf8_names_distinct_leaves_9() {
        let first_name: &[u8] = b"a\xFF.md";
        let second_name: &[u8] = b"a\xFE.md";
        // The collapse premise the lossy decode created (the pin's flaw):
        assert_eq!(
            String::from_utf8_lossy(first_name),
            String::from_utf8_lossy(second_name),
            "lossy decode maps both names to one string — why it is banned from the hash path"
        );
        let content: &[u8] = b"# one\n";
        let one = merkle_root(&[(first_name, content)], 0);
        let two = merkle_root(&[(second_name, content)], 0);
        assert_ne!(
            one, two,
            "distinct name bytes ⇒ distinct leaves ⇒ distinct roots"
        );
        let both = merkle_root(
            &[(first_name, content), (second_name, b"# two\n" as &[u8])],
            0,
        );
        assert_ne!(both, one, "both members are in the tree — no collapse");
        assert_ne!(both, two, "both members are in the tree — no collapse");
    }

    /// §9 name truthfulness: a `&str` name hashes as its UTF-8 bytes —
    /// identity, not conversion — so every valid-UTF-8 corpus keeps its pinned
    /// fingerprint. R0 is the frozen §12.1 ground truth; the byte-spelled call
    /// must fold the same root.
    #[test]
    fn str_and_byte_names_fold_identically_9() {
        let f = merkle_fixtures();
        let via_str = merkle_root(
            &[
                ("notes/plan.md", f.plan_v0.as_bytes()),
                ("receipts/2026-07-18.md", f.receipts_v0.as_bytes()),
            ],
            0,
        );
        let via_bytes = merkle_root(
            &[
                (b"notes/plan.md" as &[u8], f.plan_v0.as_bytes()),
                (b"receipts/2026-07-18.md" as &[u8], f.receipts_v0.as_bytes()),
            ],
            0,
        );
        assert_eq!(
            via_str, via_bytes,
            "str names are their UTF-8 bytes — identity"
        );
        assert_eq!(via_str.0, format!("b3:{R0_HEX}"));
    }

    /// §4: a backslash inside a name is a NAME byte, never a separator — the
    /// single file `a\b.md` and the nested path `a/b.md` fold different roots.
    #[test]
    fn backslash_is_a_name_byte_4() {
        let flat = merkle_root(&[(r"a\b.md", b"x" as &[u8])], 0);
        let nested = merkle_root(&[("a/b.md", b"x" as &[u8])], 0);
        assert_ne!(flat, nested, "a separator rewrite would collapse these");
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
            .insert(b"a.md".to_vec(), MerkleEntry::File(leaf_digest(b"x")));
        let mut with_empty = MerkleDir::default();
        with_empty
            .entries
            .insert(b"a.md".to_vec(), MerkleEntry::File(leaf_digest(b"x")));
        with_empty
            .entries
            .insert(b"empty".to_vec(), MerkleEntry::Dir(MerkleDir::default()));
        assert_eq!(
            base.fold(),
            with_empty.fold(),
            "empty dir contributes nothing"
        );
    }

    /// `MerkleDir::insert` collision symmetry (review finding, three teams:
    /// grok G-P3-1 + sonnet P3 + fable F9): a hash domain never mixes a file
    /// and a directory at one name, and the guard must hold in BOTH
    /// directions. The file-prefix arm already ignores `a/b.md` after file
    /// `a`; this gate pins the mirror — a file landing on an existing
    /// DIRECTORY name is ignored, never allowed to silently drop the subtree.
    /// Unreachable from the fs walk (a real tree cannot hold both); pub-API
    /// hygiene.
    #[test]
    fn file_on_dir_collision_preserves_subtree() {
        let subtree_only = merkle_root(&[("a/b.md", b"x" as &[u8])], 0);
        let file_on_dir = merkle_root(&[("a/b.md", b"x" as &[u8]), ("a", b"y" as &[u8])], 0);
        assert_eq!(
            file_on_dir, subtree_only,
            "a file colliding with an existing directory is ignored — the subtree survives"
        );

        // The already-guarded mirror stays: a path under an existing file
        // prefix is ignored.
        let file_only = merkle_root(&[("a", b"y" as &[u8])], 0);
        let dir_under_file = merkle_root(&[("a", b"y" as &[u8]), ("a/b.md", b"x" as &[u8])], 0);
        assert_eq!(
            dir_under_file, file_only,
            "a path under an existing file prefix is ignored"
        );

        // Last write wins on a duplicate PATH — untouched by the guard.
        let second = merkle_root(&[("a", b"y2" as &[u8])], 0);
        let dup = merkle_root(&[("a", b"y1" as &[u8]), ("a", b"y2" as &[u8])], 0);
        assert_eq!(dup, second, "duplicate path: last write still wins");
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
