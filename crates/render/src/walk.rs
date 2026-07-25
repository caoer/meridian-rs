//! The node-grain section walker: emits a section's RENDERED content with
//! the block-elision hook applied pre-emit (decision #8 seam). With the hook
//! inert (the M1 default) the emission is byte-identical to the raw content
//! slice — the U0 goldens pin that identity; the raw read/cat face NEVER
//! routes through here.
//!
//! G1: every failure is a typed [`RenderFailed`] — no indexing without a
//! bounds check, no panics.

use crate::{ClaimLink, Decorations, ElideBy, RenderFailed};
use wire_map::facts::ReadFact;

/// One rewrite the emission applies to the raw content span: replace
/// `range` with `text`. An elided block is a removal (empty text); a
/// claim-link decoration is an insertion (empty range). Both are the same
/// shape so ONE ordered cursor pass applies them — two passes over shifting
/// coordinates is how byte-exact walkers go wrong.
struct Rewrite<'a> {
    range: (usize, usize),
    text: &'a str,
}

/// Emit one resolved section's rendered content.
///
/// Heading facts walk their content span, applying the two decoration hooks
/// the marathon decisions reserve: fenced blocks whose info-string the
/// `elide` predicate matches are dropped (plus one trailing newline, so an
/// elided block does not leave its own line behind), and claim-links named in
/// `decorations` gain their shaped `@fp` token (#6, stage-2 S10).
///
/// Anchor facts emit the raw block-leaf content (marker-stripped) — a leaf
/// hosts no fenced block, and its bytes are re-cut in a different coordinate
/// space by the marker strip, so NEITHER hook fires there. A block-leaf read
/// therefore serves the raw face, which is honest: an undecorated link claims
/// nothing.
///
/// # Errors
/// Typed [`RenderFailed`] when a span exceeds the document bytes (a
/// projection/walker disagreement — a bug surfaced loudly, never a panic).
pub fn emit_section(
    doc: &model::Document,
    fact: &ReadFact,
    elide: Option<ElideBy>,
    decorations: &Decorations,
) -> Result<String, RenderFailed> {
    let raw = doc.raw.as_bytes();
    let Some(cs) = &fact.content_span else {
        // block-leaf (anchor) content: raw face, marker-stripped
        let bytes = wire_map::facts::section_content(fact, raw);
        return Ok(String::from_utf8_lossy(&bytes).into_owned());
    };
    let (start, end) = checked_bounds(cs, raw.len(), fact)?;
    if elide.is_none() && decorations.is_empty() {
        // both hooks inert: the emission IS the raw slice (U0 byte-parity)
        return Ok(String::from_utf8_lossy(&raw[start..end]).into_owned());
    }

    let mut rewrites: Vec<Rewrite<'_>> = Vec::new();
    if let Some(elide) = elide {
        collect_elided(&doc.root, start, end, elide, raw, &mut rewrites);
    }
    if !decorations.is_empty() {
        collect_decorations(&doc.root, start, end, decorations, &doc.raw, &mut rewrites);
    }
    rewrites.sort_by_key(|r| r.range);

    let mut out = Vec::with_capacity(end - start);
    let mut cursor = start;
    for Rewrite {
        range: (r_start, r_end),
        text,
    } in rewrites
    {
        if r_start < cursor {
            continue; // nested inside an already-applied rewrite
        }
        if r_end > end {
            return Err(RenderFailed {
                node_kind: "fence".into(),
                node_ref: format!("span {r_start}..{r_end}"),
                reason: format!("fenced block exceeds the section content span {start}..{end}"),
            });
        }
        out.extend_from_slice(&raw[cursor..r_start]);
        out.extend_from_slice(text.as_bytes());
        cursor = r_end;
    }
    if cursor < end {
        out.extend_from_slice(&raw[cursor..end]);
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// Recursively collect the REMOVAL rewrites for `CodeBlock` nodes within
/// `[start, end)` whose info-string the predicate matches — the block's span
/// plus its own line's trailing newline, so an elided block leaves no blank
/// line behind.
fn collect_elided(
    node: &model::Node,
    start: usize,
    end: usize,
    elide: ElideBy,
    raw: &[u8],
    out: &mut Vec<Rewrite<'_>>,
) {
    if let model::NodeKind::CodeBlock { lang, .. } = &node.kind
        && node.span.start >= start
        && node.span.end <= end
        && elide.matches(lang)
    {
        let swallow_nl = usize::from(raw.get(node.span.end) == Some(&b'\n'));
        out.push(Rewrite {
            range: (node.span.start, node.span.end + swallow_nl),
            text: "",
        });
    }
    for child in &node.children {
        collect_elided(child, start, end, elide, raw, out);
    }
}

/// Recursively collect the INSERTION rewrites for claim-links within
/// `[start, end)`: a `Wikilink`/`Embed` whose (target, block) address the
/// caller decorated gains its shaped `@fp` token immediately after the block
/// id, inside the link's own bytes.
///
/// The insertion point is the end of the block-ref SLOT, and the slot comes
/// from `syntax::block_slot` — this walker does not own the wikilink grammar
/// and must not grow a second spelling of it. Locating the slot by searching
/// the node's bytes for `#^<block>` was that second spelling, and a label
/// quoting its own block ref (`[[guide#^goal|guide#^goal in full]]`) won the
/// search: the token landed in the label, where the put-side strip cannot see
/// it, and reached disk. Decorate and strip now read the slot from one owner,
/// so a token this crate mints is one that crate removes.
fn collect_decorations<'a>(
    node: &model::Node,
    start: usize,
    end: usize,
    decorations: &'a Decorations,
    raw: &str,
    out: &mut Vec<Rewrite<'a>>,
) {
    if let (
        model::NodeKind::Wikilink {
            target,
            block: Some(block),
            ..
        }
        | model::NodeKind::Embed {
            target,
            block: Some(block),
            ..
        },
        true,
    ) = (&node.kind, node.span.start >= start && node.span.end <= end)
        && let Some(token) = decorations.get(&ClaimLink {
            target: target.clone(),
            block: block.clone(),
        })
        && let Some(slot) = syntax::block_slot(raw, &node.span, block)
    {
        out.push(Rewrite {
            range: (slot.end, slot.end),
            text: token,
        });
    }
    for child in &node.children {
        collect_decorations(child, start, end, decorations, raw, out);
    }
}

fn checked_bounds(
    span: &wire::Span,
    len: usize,
    fact: &ReadFact,
) -> Result<(usize, usize), RenderFailed> {
    let start = usize::try_from(span.0).unwrap_or(usize::MAX);
    let end = usize::try_from(span.1).unwrap_or(usize::MAX);
    if end > len || start > end {
        return Err(RenderFailed {
            node_kind: "heading".into(),
            node_ref: fact.hpath.clone(),
            reason: format!("content span {start}..{end} exceeds the document ({len} bytes)"),
        });
    }
    Ok((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NO_DECORATIONS;

    /// G1: a corrupted span surfaces as a typed `render_failed`, never a
    /// panic — the sidecar loop is panic-free by law.
    #[test]
    fn oversized_span_is_typed_error() {
        let raw = "# H\n\nbody\n";
        let doc = model::build(raw.to_string(), syntax::parse(raw));
        let fact = ReadFact {
            n: "1".into(),
            depth: 1,
            title: "H".into(),
            hpath: "H".into(),
            words: 1,
            sec_rev: "x".into(),
            span: wire::Span(0, 10),
            content_span: Some(wire::Span(5, 9999)),
            anchor: None,
        };
        let err = emit_section(&doc, &fact, None, &NO_DECORATIONS).expect_err("typed failure");
        assert_eq!(err.node_kind, "heading");
        assert_eq!(err.node_ref, "H");
        assert!(err.reason.contains("exceeds"), "{}", err.reason);
    }

    /// The inert hook emits the raw slice byte-identically.
    #[test]
    fn inert_hook_is_identity() {
        let raw = "# H\n\none ```not-a-block``` two\n";
        let doc = model::build(raw.to_string(), syntax::parse(raw));
        let facts = wire_map::facts::read_facts(&wire_map::project_toc(&doc), raw.as_bytes());
        let fact = &facts[0];
        let cs = fact.content_span.as_ref().expect("content span");
        let expect =
            &raw.as_bytes()[usize::try_from(cs.0).unwrap()..usize::try_from(cs.1).unwrap()];
        let got = emit_section(&doc, fact, None, &NO_DECORATIONS).expect("renders");
        assert_eq!(got.as_bytes(), expect, "identity emission");
    }
}
