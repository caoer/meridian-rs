//! **Charter:** the in-memory world model — governed node tree
//! (kind/span/node_rev/hpath), Merkle roots, `resolve`, CAS-`splice`
//! validation, node-level diff. **Never does:** file I/O, persistence,
//! protocol types.
//!
//! Rungs 1–3 live here (`toc`/`extract` read the tree; `resolve`/`splice`
//! validate against it; `root`/`guard` answer from it). Rungs 5–6 *borrow*
//! this module's tree and corpus index as siblings — policy takes the index
//! as a capability parameter (`Option<&CorpusIndex>` shape), never owns it.
//!
//! The tree (not a flat node list): sections govern their children, hpath
//! chains root→node — per policy-schema-design §2, which is this module's
//! public node surface, not just policy's input. Model types are
//! deliberately serde-free: what crosses the process boundary is the `wire`
//! crate's business, and the sidecar maps at that seam.

use crate::syntax::SyntaxEvent;

/// Byte range `[start, end)`. Same values as the wire's span noun; no `wire`
/// dependency by charter.
pub type Span = core::ops::Range<usize>;

/// Short content hash of a node's bytes — the CAS token. Successor of
/// meridian's `sec_rev`/`file_rev` (computed-never-committed); the hash
/// algorithm goes through the wire-contract amendment, and the migration
/// must state the mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeRev(pub String);

/// Node kinds of the governed tree (policy-schema-design §2). Richer than
/// the rung-1 wire kind enum: the tree carries structure the wire doesn't
/// ship until later rungs.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind {
    Document { path: String, line_count: u32 },
    Frontmatter { map: FrontmatterMap },
    Section { heading_text: String, level: u8 },
    Heading { text: String, level: u8 },
    Paragraph,
    List,
    ListItem,
    TaskItem { checked: bool },
    CodeBlock { lang: Option<String> },
    Callout { callout_type: String, fold: String },
    Table,
    Wikilink { target: String, fragment: Option<String> },
    Link { target: String },
    Embed { target: String, fragment: Option<String> },
    Anchor { name: String },
    Tag { name: String },
}

/// Parsed frontmatter surface. Rung 1 needs the top-level key list (wire
/// `info.keys`); the full parsed-YAML map (policy-schema §2) lands with the
/// YAML dependency when rung 6's selector engine needs values, not just
/// keys. Field, not redesign, when it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontmatterMap {
    /// Top-level mapping keys, document order.
    pub keys: Vec<String>,
}

/// One node of the governed tree. Every node carries kind + span + node_rev
/// + hpath (policy-schema §2); hpath is `None` for document and frontmatter.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub kind: NodeKind,
    pub span: Span,
    pub node_rev: NodeRev,
    /// Chain of heading texts root→governing heading, one element per
    /// segment (delimiter-free; rendering as `#A/B/C` is a display concern).
    pub hpath: Option<Vec<String>>,
    pub children: Vec<Node>,
}

/// A file's world model: the governed tree plus its integrity roots.
/// Derived, disposable, re-derived from disk — never persisted (law 2).
#[derive(Debug, Clone, PartialEq)]
pub struct Tree {
    pub root_node: Node,
    /// Merkle root over the tree (rung 3): the 32-byte cursor Go carries.
    pub root: String,
}

/// Build the governed tree from syntax events: structure governance,
/// frontmatter parse, hpath chains, node_rev hashing, Merkle root.
pub fn build(src: &str, events: Vec<SyntaxEvent>) -> Tree {
    let _ = (src, events);
    todo!("tree governance + hpath + rev hashing (rungs 1–3)")
}

/// A ref to resolve (rung 2): `#hpath` or `#^anchor` — the address grammar
/// of `file#hpath` / `file#^anchor`, file part already split off by the
/// caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ref(pub String);

/// A resolved address: the splice-able target.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    pub span: Span,
    pub node_rev: NodeRev,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// No node matches the ref.
    NotFound,
    /// Duplicate hpath — candidates listed, caller disambiguates (carried
    /// over from meridian's duplicate-hpath candidate-list error).
    Ambiguous { candidates: Vec<String> },
}

/// `file#hpath` / `file#^anchor` → span + rev (rung 2). For a heading ref
/// the target is the **section** (heading line through end of subtree).
pub fn resolve(tree: &Tree, r: &Ref) -> Result<Target, ResolveError> {
    let _ = (tree, r);
    todo!("hpath/anchor resolution over the governed tree (rung 2)")
}

/// The CAS guard of a `splice` (rung 2): replace exactly `span`, iff the
/// node's current rev equals `if_node_rev`.
///
/// Axis E position (b): validation here is *integrity only* — span/rev
/// freshness, I1/I2 anchor rules. Write **authorization** (I3: who may write
/// this section) is Go-side; this model trusts pre-authorized callers. The
/// oracle sees states, not actors (policy-schema T2 precedent).
#[derive(Debug, Clone, PartialEq)]
pub struct SpliceGuard {
    pub span: Span,
    pub if_node_rev: NodeRev,
}

/// CAS failure: the world moved since the caller looked. The one error whose
/// purpose is to be retried — re-resolve, re-derive, splice again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CasMismatch {
    pub expected: NodeRev,
    pub actual: NodeRev,
}

/// Validate a splice against the current tree. Pure validation — the atomic
/// tmp+fsync+rename execution belongs to [`crate::fs`], the only writer.
pub fn validate_splice(tree: &Tree, guard: &SpliceGuard) -> Result<(), CasMismatch> {
    let _ = (tree, guard);
    todo!("rev ladder: fresh/stale/omitted/append-dedupe/destructive-requires-fresh (rung 2)")
}
