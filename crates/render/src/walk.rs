//! The node-grain section walker: emits a section's RENDERED content with
//! the block-elision hook applied pre-emit (decision #8 seam). With the hook
//! inert (the M1 default) the emission is byte-identical to the raw content
//! slice — the U0 goldens pin that identity; the raw read/cat face NEVER
//! routes through here.
//!
//! G1: every failure is a typed [`RenderFailed`] — no indexing without a
//! bounds check, no panics.

use crate::{ElideBy, RenderFailed};
use wire_map::facts::ReadFact;

/// Emit one resolved section's rendered content.
///
/// Heading facts walk their content span, dropping any fenced block whose
/// info-string the `elide` predicate matches (plus one trailing newline, so
/// an elided block does not leave its own line behind). Anchor facts emit
/// the raw block-leaf content (marker-stripped) — a leaf hosts no fenced
/// block, so the hook has nothing to test.
///
/// # Errors
/// Typed [`RenderFailed`] when a span exceeds the document bytes (a
/// projection/walker disagreement — a bug surfaced loudly, never a panic).
pub fn emit_section(
    doc: &model::Document,
    fact: &ReadFact,
    elide: Option<ElideBy>,
) -> Result<String, RenderFailed> {
    let raw = doc.raw.as_bytes();
    let Some(cs) = &fact.content_span else {
        // block-leaf (anchor) content: raw face, marker-stripped
        let bytes = wire_map::facts::section_content(fact, raw);
        return Ok(String::from_utf8_lossy(&bytes).into_owned());
    };
    let (start, end) = checked_bounds(cs, raw.len(), fact)?;
    let Some(elide) = elide else {
        // hook inert: the emission IS the raw slice (U0 byte-parity)
        return Ok(String::from_utf8_lossy(&raw[start..end]).into_owned());
    };

    // collect the elided fenced blocks inside the content span, in order
    let mut blocks: Vec<(usize, usize)> = Vec::new();
    collect_elided(&doc.root, start, end, elide, &mut blocks);
    blocks.sort_unstable();

    let mut out = Vec::with_capacity(end - start);
    let mut cursor = start;
    for (b_start, b_end) in blocks {
        if b_start < cursor {
            continue; // nested inside an already-elided block
        }
        if b_end > end {
            return Err(RenderFailed {
                node_kind: "fence".into(),
                node_ref: format!("span {b_start}..{b_end}"),
                reason: format!("fenced block exceeds the section content span {start}..{end}"),
            });
        }
        out.extend_from_slice(&raw[cursor..b_start]);
        // swallow the block's own line: one trailing newline when present
        cursor = if raw.get(b_end) == Some(&b'\n') {
            b_end + 1
        } else {
            b_end
        };
    }
    if cursor < end {
        out.extend_from_slice(&raw[cursor..end]);
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// Recursively collect `CodeBlock` node spans within `[start, end)` whose
/// info-string the predicate matches.
fn collect_elided(
    node: &model::Node,
    start: usize,
    end: usize,
    elide: ElideBy,
    out: &mut Vec<(usize, usize)>,
) {
    if let model::NodeKind::CodeBlock { lang, .. } = &node.kind
        && node.span.start >= start
        && node.span.end <= end
        && elide.matches(lang)
    {
        out.push((node.span.start, node.span.end));
    }
    for child in &node.children {
        collect_elided(child, start, end, elide, out);
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
        let err = emit_section(&doc, &fact, None).expect_err("typed failure");
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
        let got = emit_section(&doc, fact, None).expect("renders");
        assert_eq!(got.as_bytes(), expect, "identity emission");
    }
}
