//! One pure function: markdown bytes → dialect node list with byte-exact spans —
//! the only crate that touches the pulldown-cmark fork.
//!
//! # Charter
//! **Owns:** the dialect grammar truth. Fork events (wikilinks, anchors, callout
//! types + fold markers, embed `!`-folding) plus the two post-passes the fork
//! deliberately excludes (`%%comment%%` — cross-block, 12 lines outside the
//! parser beats touching `firstpass.rs`; and masking). Byte-exact spans are the
//! load-bearing contract: pulldown emits `(event, byte_range)` natively, so a
//! parser version bump fails at compile time, never silently at runtime.
//!
//! **Never does:** I/O, state, hashing, world-model assembly (that's `model`'s),
//! body formatting (permanently out of scope — ccc-mdformat owns it).
//!
//! # Law enforcement (candidate thesis, this crate's part)
//! This is the ONLY crate allowed to depend on the fork. The fork's churn and
//! ours must not couple (constraint 5): every other crate sees `DialectNode`,
//! never a pulldown type, so a fork API change is a one-crate event.
//!
//! # Rungs
//! Rung 1 lands `parse` complete (the parser-bench `rust-pulldown` lane's
//! extraction core relocates here); later rungs add dialect events, never
//! callers.

use std::ops::Range;

/// A dialect event with its byte-exact span. Kinds mirror the fork's event
/// vocabulary plus post-pass constructs; `model` builds the governed tree from
/// this flat stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialectNode {
    pub kind: DialectKind,
    /// Half-open byte range into the input. Span laws (block spans exclude the
    /// final line terminator; inline spans include their delimiters) are the
    /// wire contract's §2 — enforced here at emission, tested against the GT pack.
    pub span: Range<usize>,
}

/// Dialect constructs rung 1 recognizes. First-class fork events where the fork
/// owns the grammar (`Wikilink`, `Anchor`, …), post-pass products where it
/// deliberately doesn't (`Comment`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialectKind {
    Frontmatter { keys: Vec<String> },
    Heading { level: u8, text: String },
    Fence { info_string: String, unterminated: bool },
    InlineCode,
    Anchor { id: String },
    Wikilink { target: String, heading: Option<String>, block: Option<String>, alias: Option<String> },
    Embed { target: String, heading: Option<String>, block: Option<String>, alias: Option<String> },
    Callout { r#type: String, fold: String },
    Task { checked: bool, depth: u32 },
    Table,
    Comment,
}

/// Markdown bytes → dialect nodes with byte-exact spans. Pure; the whole crate
/// surface. Input is `&str` because the wire refuses non-UTF-8 files upstream
/// (`invalid_utf8`) — disk bytes and string bytes are the same bytes here.
pub fn parse(input: &str) -> Vec<DialectNode> {
    // The fork's offset iterator is the span source of truth; the lane's
    // extraction core (parser-bench lanes/rust-pulldown) relocates into this body.
    let _events = pulldown_cmark::Parser::new(input).into_offset_iter();
    todo!("rung 1: map fork events + %%comment%%/mask post-passes to DialectNode")
}
