//! The in-memory world model: governed node tree (kind/span/node_rev/hpath),
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

/// Placeholder for parsed frontmatter YAML (policy-schema §2: the engine parses
/// frontmatter — dialect-smart includes frontmatter). YAML library choice is an
/// implementation decision deferred to the rung that lands it; the placeholder
/// keeps the skeleton dependency-honest.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct YamlMap(pub BTreeMap<String, String>);

/// The governed tree node. Every node carries kind + span + node_rev + hpath
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
    Document { path: String, line_count: u32 },
    Frontmatter { map: YamlMap },
    Section { heading_text: String, level: u8 },
    Heading { text: String, level: u8 },
    Paragraph,
    List,
    ListItem,
    TaskItem { checked: bool },
    CodeBlock { lang: String },
    Callout { r#type: String, fold: String },
    Table,
    Wikilink { target: String, fragment: Option<String> },
    Link { target: String },
    Embed { target: String },
    Anchor { name: String },
    Tag { name: String },
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
pub fn build(raw: String, nodes: Vec<syntax::DialectNode>) -> Document {
    let _ = (&raw, &nodes);
    todo!("rung 1: governed-tree assembly (sections govern children, hpath chains)")
}

// ---------------------------------------------------------------------------
// resolve (rung 2)
// ---------------------------------------------------------------------------

/// A ref to resolve: `#hpath` (human `/`-joined form) or `#^anchor-id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ref {
    Hpath(String),
    Anchor(String),
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

/// `file#hpath` / `file#^anchor` → span + node_rev. Section span = heading line
/// through end of subtree (what makes the vision's splice example coherent).
pub fn resolve(doc: &Document, r#ref: &Ref) -> Result<Target, ResolveError> {
    let _ = (doc, r#ref);
    todo!("rung 2: hpath/anchor resolution over the governed tree")
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
    CasMismatch { expected: NodeRev, actual: NodeRev },
}

/// A splice that passed CAS validation against a live `Document`. Only `model`
/// can mint one (private field), and `fs::apply_splice` only accepts one.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedSplice {
    pub span: ByteSpan,
    pub text: String,
    _sealed: (),
}

/// Validate a splice against the current tree: span integrity + rev ladder
/// (meridian I1/I2 anchor resolution + fresh/stale/omitted semantics relocate
/// here as the validation core).
pub fn validate_splice(doc: &Document, req: &SpliceRequest) -> SpliceVerdict {
    let _ = (doc, req);
    todo!("rung 2: CAS validation + rev ladder")
}

// ---------------------------------------------------------------------------
// integrity (rung 3) + corpus index (rung 5 borrow surface)
// ---------------------------------------------------------------------------

/// Current root over one document (rung 3); corpus root composes over these.
pub fn merkle_root(doc: &Document) -> MerkleRoot {
    let _ = doc;
    todo!("rung 3: per-node hash fold")
}

/// The resident corpus name index (rung 4+ daemon state). Model state —
/// derived, disposable (law 2). `query` and `policy` borrow it as a capability
/// parameter (`Option<&CorpusIndex>`); neither owns it, and there is no
/// policy→query dependency — both are siblings over model.
#[derive(Debug, Default)]
pub struct CorpusIndex {
    _names: BTreeMap<String, Vec<String>>,
}
