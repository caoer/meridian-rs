//! **Charter:** one pure function — markdown bytes → dialect event list with
//! byte-exact spans (fork events + the `%%comment%%`/mask post-passes).
//! **Never does:** I/O, state, hashing.
//!
//! Rung 1. The parser is the pulldown-cmark fork (`ENABLE_OBSIDIAN_*`
//! capabilities: anchors, callout types + fold markers, embed `!`-folding,
//! wikilink fragment semantics). `%%comments%%` stay a post-pass by ruling:
//! they can span blocks, and cross-block spans fight CommonMark's
//! block-then-inline architecture — 12 lines out here vs touching
//! `firstpass.rs`.
//!
//! Seam (syntax → model): syntax emits flat dialect events with spans; the
//! [`crate::model`] builds the governed tree, parses frontmatter YAML, and
//! computes hpath and revs. Nothing here knows about trees, hashes, or the
//! wire.

/// Byte range `[start, end)` into the input. Same values as the wire's span
/// noun; this module deliberately does not depend on the `wire` crate (the
/// sidecar maps at the boundary).
pub type ByteSpan = core::ops::Range<usize>;

/// What the dialect grammar recognized. Fork event variants surface here at
/// the same class level as upstream's — pattern-matched,
/// exhaustiveness-checked, never string kinds born in regexes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyntaxEventKind {
    Frontmatter,
    Heading { level: u8 },
    Fence { info_string: String, unterminated: bool },
    InlineCode,
    Anchor { id: String },
    Wikilink { target: String, heading: Option<String>, block: Option<String>, alias: Option<String> },
    Embed { target: String, heading: Option<String>, block: Option<String>, alias: Option<String> },
    Callout { callout_type: String, fold: String },
    Task { checked: bool, depth: u32 },
    Table,
    /// `%%…%%` — produced by the post-pass, not the fork.
    Comment,
    /// Structural events the model needs for tree-building (paragraphs,
    /// lists, quotes…) — inventory finalized when the fork lands (step 0).
    Block,
}

/// One recognized construct with its byte-exact span. Spans are a library
/// contract inherited from pulldown (`(event, byte_range)`), never
/// reconstructed — parser version bumps fail at compile time, loudly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxEvent {
    pub kind: SyntaxEventKind,
    pub span: ByteSpan,
}

/// The one entry point: dialect parse of a full document. Pure — same bytes,
/// same events, no environment.
///
/// Span laws (carried from the lane, enforced by `testsuite`'s GT pack):
/// block spans exclude the final line terminator; inline spans include their
/// delimiters; CRLF round-trips byte-identically (`\r` stripped for matching,
/// kept in spans).
pub fn parse(src: &str) -> Vec<SyntaxEvent> {
    // Anchors the fork dependency now so the [patch.crates-io] pin is
    // exercised by `cargo check` from the first commit.
    let _fork = pulldown_cmark::Parser::new(src);
    todo!("fork event loop + %%comment%%/mask post-passes (step 0)")
}
