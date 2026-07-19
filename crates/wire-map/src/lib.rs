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

/// Project a parsed document onto the `toc` row list (contract §4.1): the
/// complete write kit — frontmatter row with `keys`, heading rows with
/// `level` + `hpath` + `content_span`, anchor-bearing block rows echoing
/// their HOST block kind as the open `kind` string (the worked anchor row is
/// `"list_item"`). Rows in the frozen total order (span.start asc, span.end
/// desc, §5.2 kind ordinal — anchor rows take the `anchor` ordinal).
///
/// The header fields (`path`/`file_rev`/`root`) are the caller's: `file_rev`
/// is the document root's `node_rev` (same family, whole-file bytes), the
/// ambient `root` is workspace-grain — neither is a per-row fact.
#[must_use]
pub fn project_toc(doc: &model::Document) -> Vec<wire::TocNode> {
    let raw = doc.raw.as_bytes();
    let mut rows = Vec::new();
    collect_toc(&doc.root, &doc.root, raw, &mut rows);
    rows.sort_by(|a, b| {
        a.span
            .0
            .cmp(&b.span.0)
            .then(b.span.1.cmp(&a.span.1))
            .then(toc_ordinal(a).cmp(&toc_ordinal(b)))
    });
    rows
}

/// §5.2 kind ordinal for toc rows: `frontmatter` 0, `heading` 1; anchor rows
/// (whatever host kind they echo) sort at the `anchor` ordinal (4).
fn toc_ordinal(row: &wire::TocNode) -> u8 {
    match row.kind.as_str() {
        "frontmatter" => 0,
        "heading" => 1,
        _ => 4,
    }
}

fn collect_toc(node: &model::Node, root: &model::Node, raw: &[u8], out: &mut Vec<wire::TocNode>) {
    use model::NodeKind as M;
    let span = wire::Span(node.span.start as u64, node.span.end as u64);
    let base = wire::TocNode {
        kind: String::new(),
        level: None,
        hpath: None,
        anchor: None,
        span,
        content_span: None,
        node_rev: wire::NodeRev(node.node_rev.0.clone()),
        text_prefix_16b: prefix_16b(raw, node.span.start),
        keys: None,
    };
    match &node.kind {
        M::Frontmatter { map } => out.push(wire::TocNode {
            kind: "frontmatter".into(),
            keys: Some(map.keys().map(str::to_owned).collect()),
            ..base
        }),
        M::Section { level, .. } => out.push(wire::TocNode {
            kind: "heading".into(),
            level: Some(u32::from(*level)),
            hpath: Some(
                node.hpath
                    .clone()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|h| wire::HpathSeg { h, n: None })
                    .collect(),
            ),
            content_span: Some(content_span(raw, &node.span)),
            ..base
        }),
        M::Anchor { name } => out.push(wire::TocNode {
            kind: host_kind(root, &node.span, raw),
            anchor: Some(name.clone()),
            ..base
        }),
        _ => {}
    }
    for child in &node.children {
        collect_toc(child, root, raw, out);
    }
}

/// §1 rev sub-law: the content span starts at the first byte after the heading
/// line's terminator and runs to the section span's end; an unterminated
/// heading at EOF has an empty content span at the section end.
fn content_span(raw: &[u8], span: &std::ops::Range<usize>) -> wire::Span {
    let start = raw[span.start..span.end]
        .iter()
        .position(|&b| b == b'\n')
        .map_or(span.end, |i| span.start + i + 1);
    wire::Span(start as u64, span.end as u64)
}

/// The anchor row's HOST block kind (contract §4.1: the `^id` block echoes as
/// a node keyed by its `anchor`, carrying the host block's kind as an open
/// string). A dialect block sharing the anchor's exact span names the host
/// directly; otherwise the raw bytes at span start classify the block —
/// list-marker → `list_item`, else `paragraph`.
fn host_kind(root: &model::Node, span: &std::ops::Range<usize>, raw: &[u8]) -> String {
    use model::NodeKind as M;
    if let Some(kind) = same_span_block(root, span) {
        return kind;
    }
    let rest = &raw[span.start.min(raw.len())..span.end.min(raw.len())];
    let after_marker = rest
        .strip_prefix(b"- ")
        .or_else(|| rest.strip_prefix(b"* "))
        .or_else(|| rest.strip_prefix(b"+ "))
        .or_else(|| {
            let digits = rest.iter().take_while(|b| b.is_ascii_digit()).count();
            (digits > 0 && rest.get(digits).is_some_and(|b| *b == b'.' || *b == b')'))
                .then(|| &rest[digits + 1..])
        });
    if after_marker.is_some() {
        "list_item".into()
    } else {
        return_kind(&M::Paragraph)
    }
}

/// Search the tree for a non-anchor dialect block with exactly `span`.
fn same_span_block(node: &model::Node, span: &std::ops::Range<usize>) -> Option<String> {
    use model::NodeKind as M;
    if node.span == *span {
        match &node.kind {
            M::TaskItem { .. } | M::Callout { .. } | M::CodeBlock { .. } | M::Table => {
                return Some(return_kind(&node.kind));
            }
            _ => {}
        }
    }
    node.children.iter().find_map(|c| same_span_block(c, span))
}

/// Open-string kind names for host blocks (kebab/snake per the wire's kind
/// vocabulary: `task`, `callout`, `fence`, `table`, `list_item`, `paragraph`).
fn return_kind(kind: &model::NodeKind) -> String {
    use model::NodeKind as M;
    match kind {
        M::TaskItem { .. } => "task".into(),
        M::Callout { .. } => "callout".into(),
        M::CodeBlock { .. } => "fence".into(),
        M::Table => "table".into(),
        _ => "paragraph".into(),
    }
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
mod toc_tests {
    use super::project_toc;

    /// §0.3 `notes/plan.md` at S0, exact bytes (136 B, LF, trailing newline).
    const PLAN_S0: &str = "---\ntitle: Plan\n---\n# Goals\n\nShip the contract.\n\n## Q3\n\nship by August\n\n## Q4\n\n- item one\n- see [[2026-07-18]]\n- blocked on [[roadmap]]\n";

    /// §6.3 E3 receipt line, frozen bytes (also pinned in `crates/receipt`).
    const E3_LINE: &str = "- splice notes/plan.md id=42 actor=agent:b0864fb2 now=2026-07-18T20:31:04Z root_before=b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9 edits=1 Goals>Q3 match 33d5b0e1b27cb48b->41f643f034e5681f ^r-000042";

    fn rows(raw: &str) -> Vec<wire::TocNode> {
        project_toc(&model::build(raw.to_string(), syntax::parse(raw)))
    }

    fn seg(h: &str) -> wire::HpathSeg {
        wire::HpathSeg {
            h: h.into(),
            n: None,
        }
    }

    /// Contract §4.1 worked S0 toc rows, value-for-value (`root`/`file_rev` are
    /// header fields, the caller's — this pins the ROW projection).
    #[test]
    fn worked_s0_plan_toc_rows() {
        assert_eq!(PLAN_S0.len(), 136, "frozen fixture byte count");
        let got = rows(PLAN_S0);
        let expected = vec![
            wire::TocNode {
                kind: "frontmatter".into(),
                level: None,
                hpath: None,
                anchor: None,
                span: wire::Span(0, 20),
                content_span: None,
                node_rev: wire::NodeRev("26796ebec5d0bf1a".into()),
                text_prefix_16b: "---\ntitle: Plan\n".into(),
                keys: Some(vec!["title".into()]),
            },
            wire::TocNode {
                kind: "heading".into(),
                level: Some(1),
                hpath: Some(vec![seg("Goals")]),
                anchor: None,
                span: wire::Span(20, 136),
                content_span: Some(wire::Span(28, 136)),
                node_rev: wire::NodeRev("a6665baff294bd04".into()),
                text_prefix_16b: "# Goals\n\nShip th".into(),
                keys: None,
            },
            wire::TocNode {
                kind: "heading".into(),
                level: Some(2),
                hpath: Some(vec![seg("Goals"), seg("Q3")]),
                anchor: None,
                span: wire::Span(49, 72),
                content_span: Some(wire::Span(55, 72)),
                node_rev: wire::NodeRev("33d5b0e1b27cb48b".into()),
                text_prefix_16b: "## Q3\n\nship by A".into(),
                keys: None,
            },
            wire::TocNode {
                kind: "heading".into(),
                level: Some(2),
                hpath: Some(vec![seg("Goals"), seg("Q4")]),
                anchor: None,
                span: wire::Span(72, 136),
                content_span: Some(wire::Span(78, 136)),
                node_rev: wire::NodeRev("4b8bc385a58da0e0".into()),
                text_prefix_16b: "## Q4\n\n- item on".into(),
                keys: None,
            },
        ];
        assert_eq!(got, expected);
    }

    /// Contract §4.1 worked ANCHOR toc rows (the late law): the `^r-000042`
    /// block echoes as a `list_item` row keyed by its `anchor`, block-leaf
    /// span (terminator excluded), its own `node_rev`; the lone top-level
    /// heading spans the whole file so its rev equals `file_rev`.
    #[test]
    fn worked_s1_receipts_anchor_toc_rows() {
        let raw = format!("# Receipts — 2026-07-18\n{E3_LINE}\n");
        assert_eq!(raw.len(), 249, "S1 receipts byte count (worked §0.3)");
        let got = rows(&raw);
        let expected = vec![
            wire::TocNode {
                kind: "heading".into(),
                level: Some(1),
                hpath: Some(vec![seg("Receipts — 2026-07-18")]),
                anchor: None,
                span: wire::Span(0, 249),
                content_span: Some(wire::Span(26, 249)),
                node_rev: wire::NodeRev("2731acfa39bbb92c".into()),
                text_prefix_16b: "# Receipts — 2".into(),
                keys: None,
            },
            wire::TocNode {
                kind: "list_item".into(),
                level: None,
                hpath: None,
                anchor: Some("r-000042".into()),
                span: wire::Span(26, 248),
                content_span: None,
                node_rev: wire::NodeRev("639a2dca46f6fcc8".into()),
                text_prefix_16b: "- splice notes/p".into(),
                keys: None,
            },
        ];
        assert_eq!(got, expected);
    }

    /// Host-kind classification beyond the worked `list_item`: a task checkbox
    /// block sharing the anchor's span echoes `task`; a bare-paragraph anchor
    /// echoes `paragraph`.
    #[test]
    fn anchor_host_kind_task_and_paragraph() {
        let raw = "# H\n\n- [ ] do the thing ^t-1\n\nplain block ^p-1\n";
        let got = rows(raw);
        let kinds: Vec<(&str, &str)> = got
            .iter()
            .filter_map(|r| r.anchor.as_deref().map(|a| (r.kind.as_str(), a)))
            .collect();
        assert_eq!(kinds, vec![("task", "t-1"), ("paragraph", "p-1")]);
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
