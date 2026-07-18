//! **Charter:** the §5.2 node object — the shape `toc` and `extract` answer
//! with, shared verbatim between both ops.
//!
//! Node ordering (frozen, a total order): `span.start` ascending; ties by
//! `span.end` *descending* (container before contained); full-span ties by
//! kind-enum ordinal ascending — the declaration order of [`NodeKind`] below.
//! At most one node of a given kind per exact span.

use serde::{Deserialize, Serialize};

use crate::vocab::Span;

/// Kind enum, v1 (contract §5.2). Declaration order IS the ordinal tiebreak
/// order (`frontmatter` = 0 … `comment` = 10) — do not reorder. New kinds
/// arrive by amendment; clients ignore unknown kinds (§7 rule 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NodeKind {
    Frontmatter,
    Heading,
    Fence,
    InlineCode,
    Anchor,
    Wikilink,
    Embed,
    Callout,
    Task,
    Table,
    Comment,
}

/// One node on the wire (contract §5.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub kind: NodeKind,
    /// Per §2 span laws.
    pub span: Span,
    /// Prefix window per the frozen prefix law: `raw[start .. start+16]`
    /// clamped at EOF, longest-valid-UTF-8 prefix decoded, each remaining
    /// byte escaped `\xhh`. Both sides compute it; bit-for-bit agreement.
    pub text_prefix_16b: String,
    /// Headings only. One array element per heading, root→self, text as
    /// written (ATX markers stripped, whitespace trimmed, inline markup
    /// kept). Array form is delimiter-free: `## a/b` needs no escaping.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hpath: Option<Vec<String>>,
    /// Present (and `true`) only when a fence runs to EOF unclosed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unterminated: Option<bool>,
    /// Kind-specific payload; absent when the kind defines none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub info: Option<NodeInfo>,
}

/// Per-kind `info` payloads (contract §5.2 table). Untagged: the kind field
/// on [`Node`] decides which shape applies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum NodeInfo {
    /// `frontmatter` — top-level YAML mapping keys, document order. Exists
    /// only when the file begins with the exact 4-byte prefix `---\n`.
    Frontmatter { keys: Vec<String> },
    /// `fence` — info string, possibly `""`.
    Fence { info_string: String },
    /// `wikilink` and `embed` (same shape; embed's span includes the `!`).
    /// `heading` and `block` are mutually exclusive; block IDs share the
    /// anchor charset `[A-Za-z0-9_-]+` — one charset everywhere.
    Link {
        target: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        heading: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        block: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        alias: Option<String>,
    },
    /// `callout` — `type` per `> [!type]` (charset `[A-Za-z0-9_-]+`), `fold`
    /// is `"+"`, `"-"`, or `""`.
    Callout {
        #[serde(rename = "type")]
        callout_type: String,
        fold: String,
    },
    /// `task` — checklist items only; `depth` is 1-based list nesting.
    Task { checked: bool, depth: u32 },
}
