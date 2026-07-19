//! The named model→wire projection seam (review C1 ruling, 4/4 lenses):
//! tree-flatten + `text_prefix_16b` + node ordering as a **tested library
//! function, never bin code**.
//!
//! # Charter
//! **Owns:** the projection from `model`'s governed tree (non-serializable, by
//! law) to `wire`'s flat node list (contract §5.2): flattening, kind mapping,
//! the frozen prefix law, and the frozen total node order.
//!
//! **Never does:** I/O, framing, parsing, business logic, body formatting.
//!
//! # Law 3, as amended (review C1)
//! Original wording: "only the bin sees wire and model together." Amended:
//! **"only the named wire-map seam and the bin see both."** This crate IS that
//! seam — the one place projection behavior lives and is tested; `sidecar`
//! stays wiring-only. Growing projection pressure lands here, never in a bin.
//!
//! # Rungs
//! Rung 1: `project` (toc/extract node lists). Rung 2+: projection of resolve
//! targets and splice verdicts joins additively.

/// Project a parsed document onto the wire node list: flatten the governed
/// tree to the contract's flat kinds, compute `text_prefix_16b` from the raw
/// bytes, attach `node_rev`s, and emit in the frozen total order
/// (span.start asc, span.end desc, kind-ordinal asc — contract §5.2).
///
/// Model-only structure (document root, paragraphs, lists, plain links, tags)
/// is not wire-observable and is skipped — the four B1 superset predicates
/// pin that nothing wire-observable is lost.
#[must_use]
pub fn project(doc: &model::Document) -> Vec<wire::Node> {
    let mut out = Vec::new();
    flatten(&doc.root, doc.raw.as_bytes(), &mut out);
    // the frozen total order: span.start asc, span.end desc, kind ordinal
    // (wire::NodeKind derives Ord over the frozen declaration order)
    out.sort_by(|a, b| {
        a.span
            .0
            .cmp(&b.span.0)
            .then(b.span.1.cmp(&a.span.1))
            .then(a.kind.cmp(&b.kind))
    });
    out
}

fn flatten(node: &model::Node, raw: &[u8], out: &mut Vec<wire::Node>) {
    if let Some((kind, info, unterminated)) = wire_view(&node.kind) {
        let start = node.span.start;
        out.push(wire::Node {
            kind,
            span: wire::Span(node.span.start as u64, node.span.end as u64),
            text_prefix_16b: prefix_16b(raw, start),
            hpath: (kind == wire::NodeKind::Heading).then(|| {
                node.hpath
                    .clone()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|h| wire::HpathSeg { h, n: None })
                    .collect()
            }),
            unterminated: unterminated.then_some(true),
            info,
            node_rev: Some(wire::NodeRev(node.node_rev.0.clone())),
        });
    }
    for child in &node.children {
        flatten(child, raw, out);
    }
}

/// The model→wire kind map: `(wire kind, per-kind info, unterminated)` for
/// wire-observable nodes, `None` for model-only structure.
fn wire_view(kind: &model::NodeKind) -> Option<(wire::NodeKind, Option<wire::Info>, bool)> {
    use model::NodeKind as M;
    use wire::{Info, NodeKind as W};
    Some(match kind {
        M::Frontmatter { map } => (
            W::Frontmatter,
            Some(Info::Frontmatter {
                keys: map.keys().map(str::to_owned).collect(),
            }),
            false,
        ),
        // a wire "heading" is the model SECTION: heading-inclusive span to the
        // next boundary (§1 span sub-laws), hpath chain attached
        M::Section { .. } => (W::Heading, None, false),
        M::CodeBlock { lang, unterminated } => (
            W::Fence,
            Some(Info::Fence {
                info_string: lang.clone(),
            }),
            *unterminated,
        ),
        M::InlineCode => (W::InlineCode, None, false),
        M::Comment => (W::Comment, None, false),
        M::Anchor { .. } => (W::Anchor, None, false),
        M::Wikilink {
            target,
            heading,
            block,
            alias,
        } => (
            W::Wikilink,
            Some(Info::Wikilink {
                target: target.clone(),
                heading: heading.clone(),
                block: block.clone(),
                alias: alias.clone(),
            }),
            false,
        ),
        M::Embed {
            target,
            heading,
            block,
            alias,
        } => (
            W::Embed,
            Some(Info::Wikilink {
                target: target.clone(),
                heading: heading.clone(),
                block: block.clone(),
                alias: alias.clone(),
            }),
            false,
        ),
        M::Callout { r#type, fold } => (
            W::Callout,
            Some(Info::Callout {
                r#type: r#type.clone(),
                fold: fold.clone(),
            }),
            false,
        ),
        M::TaskItem { checked, depth } => (
            W::Task,
            Some(Info::Task {
                checked: *checked,
                depth: *depth,
            }),
            false,
        ),
        M::Table => (W::Table, None, false),
        // model-only structure: not wire-observable
        M::Document { .. }
        | M::Heading { .. }
        | M::Paragraph
        | M::List
        | M::ListItem
        | M::Link { .. }
        | M::Tag { .. } => return None,
    })
}

/// The frozen prefix law (contract §5.2), implemented — the seam proof, real
/// code with the contract's worked examples as tests.
///
/// Window: `raw[start : min(start+16, len)]` — clamped at EOF, never padded.
/// Decode: longest valid-UTF-8 head emitted as-is; each remaining byte as the
/// four-char ASCII sequence `\xhh` (lowercase hex). Only a single multibyte
/// character truncated by the window end ever gets escaped.
#[must_use]
#[expect(
    clippy::missing_panics_doc,
    reason = "the unwrap re-reads only bytes from_utf8 already validated"
)]
pub fn prefix_16b(raw: &[u8], start: usize) -> String {
    use std::fmt::Write;

    let end = start.saturating_add(16).min(raw.len());
    let window = &raw[start.min(raw.len())..end];
    match std::str::from_utf8(window) {
        Ok(s) => s.to_owned(),
        Err(e) => {
            let k = e.valid_up_to();
            let mut out = String::with_capacity(k + (window.len() - k) * 4);
            out.push_str(std::str::from_utf8(&window[..k]).unwrap());
            for b in &window[k..] {
                let _ = write!(out, "\\x{b:02x}");
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::prefix_16b;

    /// Contract §5.2 worked example: 2-byte split. `é` = 0xc3 0xa9 at bytes
    /// 15–17; the window cuts after its first byte.
    #[test]
    fn split_multibyte_2byte() {
        let raw = "# 1234567890123é tail".as_bytes();
        assert_eq!(prefix_16b(raw, 0), "# 1234567890123\\xc3");
    }

    /// Contract §5.2 worked example: 4-byte split. `𝄞` = 0xf0 0x9d 0x84 0x9e
    /// at bytes 14–18; the window cuts after its second byte.
    #[test]
    fn split_multibyte_4byte() {
        let raw = "# abcdefghijkl𝄞 x".as_bytes();
        assert_eq!(prefix_16b(raw, 0), "# abcdefghijkl\\xf0\\x9d");
    }

    /// Contract §5.3 example: a node starting fewer than 16 bytes before EOF
    /// has a short prefix — clamped, never padded ("## Beta\n", 8 bytes).
    #[test]
    fn clamped_at_eof() {
        let raw = "---\ntitle: demo\n---\n\n# Alpha\n\n## Beta\n".as_bytes();
        assert_eq!(prefix_16b(raw, 30), "## Beta\n");
    }
}
