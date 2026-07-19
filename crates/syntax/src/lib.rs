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

use pulldown_cmark::{
    CodeBlockKind, Event, HeadingLevel, LinkType, Options, Parser, Tag, TagEnd,
};
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
    Frontmatter {
        keys: Vec<String>,
    },
    Heading {
        level: u8,
        text: String,
    },
    Fence {
        info_string: String,
        unterminated: bool,
    },
    InlineCode,
    Anchor {
        id: String,
    },
    Wikilink {
        target: String,
        heading: Option<String>,
        block: Option<String>,
        alias: Option<String>,
    },
    Embed {
        target: String,
        heading: Option<String>,
        block: Option<String>,
        alias: Option<String>,
    },
    Callout {
        r#type: String,
        fold: String,
    },
    Task {
        checked: bool,
        depth: u32,
    },
    Table,
    Comment,
}

/// Markdown bytes → dialect nodes with byte-exact spans. Pure; the whole crate
/// surface. Input is `&str` because the wire refuses non-UTF-8 files upstream
/// (`invalid_utf8`) — disk bytes and string bytes are the same bytes here.
///
/// The fork's `into_offset_iter()` is the span source of truth (every event
/// carries its own byte range). Two post-passes the fork deliberately excludes
/// run here over the raw source: `%%comment%%` (cross-block, masked) and anchor
/// block ids. Nodes come out in the §1 total order.
// One cohesive single-pass event dispatcher: the shared cursor state
// (masks, open items, wikilink accumulator) makes splitting the match arms into
// helpers cost more coupling than it saves in length.
#[allow(clippy::too_many_lines)]
#[must_use]
pub fn parse(input: &str) -> Vec<DialectNode> {
    let mut nodes: Vec<DialectNode> = Vec::new();

    // Exclusion masks, appended in event order (each already sorted ascending
    // and non-overlapping): the raw-source post-passes binary-search them so
    // mask-dense files stay linear, never quadratic.
    let mut fenced_mask: Vec<Range<usize>> = Vec::new();
    let mut inline_mask: Vec<Range<usize>> = Vec::new();
    let mut fm_mask: Vec<Range<usize>> = Vec::new();

    let mut cur_heading: Option<Range<usize>> = None;
    // (is_image, range, dest, alias buffer, has_pothole)
    let mut cur_wikilink: Option<(bool, Range<usize>, String, String, bool)> = None;
    let mut list_depth: u32 = 0;
    let mut bq_depth: u32 = 0;
    // (item range, list depth at Start(Item)); a task node is minted only if a
    // TaskListMarker follows — plain items are not a dialect fact.
    let mut open_items: Vec<(Range<usize>, u32)> = Vec::new();

    for (ev, range) in Parser::new_ext(input, parser_options()).into_offset_iter() {
        match ev {
            Event::Start(Tag::MetadataBlock(_)) => {
                // frontmatter only when the file opens with exactly "---\n"
                if range.start != 0 || !input.starts_with("---\n") {
                    continue;
                }
                fm_mask.push(range.clone());
                let keys = frontmatter_keys(&input[range.clone()]);
                nodes.push(DialectNode {
                    kind: DialectKind::Frontmatter { keys },
                    span: trim_block(input, &range),
                });
            }
            Event::Start(Tag::Heading { .. }) => cur_heading = Some(range),
            Event::End(TagEnd::Heading(level)) => {
                if let Some(hrange) = cur_heading.take() {
                    let span = trim_block(input, &hrange);
                    let slice = &input[span.clone()];
                    // ATX only (≤3 indent, leading '#'); setext is not extracted
                    let dedented = slice.trim_start_matches(' ');
                    if slice.len() - dedented.len() > 3 || !dedented.starts_with('#') {
                        continue;
                    }
                    let text = dedented.trim_start_matches('#').trim().to_string();
                    nodes.push(DialectNode {
                        kind: DialectKind::Heading {
                            level: heading_num(level),
                            text,
                        },
                        span,
                    });
                }
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) => {
                fenced_mask.push(range.clone());
                let span = trim_block(input, &range);
                let unterminated = fence_unterminated(&input[span.clone()]);
                nodes.push(DialectNode {
                    kind: DialectKind::Fence {
                        info_string: info.to_string(),
                        unterminated,
                    },
                    span,
                });
            }
            // 4+ space indent is never a fence; pulldown keeps indented code
            // atomic, so no inline nodes leak from it.
            Event::Code(_) => {
                if let Some((_, _, _, alias, _)) = cur_wikilink.as_mut() {
                    alias.push_str(&input[range.clone()]);
                }
                // inline code is single-line only, never inside a fence/frontmatter
                if !input[range.clone()].contains('\n') {
                    inline_mask.push(range.clone());
                    nodes.push(DialectNode {
                        kind: DialectKind::InlineCode,
                        span: range,
                    });
                }
            }
            Event::Start(Tag::BlockQuote { .. }) => {
                bq_depth += 1;
                if bq_depth == 1 {
                    // callout = top-level quote whose first line is `> [!type]±`
                    let mut probe_end = range.end.min(range.start + 256);
                    while probe_end > range.start && !input.is_char_boundary(probe_end) {
                        probe_end -= 1;
                    }
                    if let Some((ctype, fold)) = callout_head(&input[range.start..probe_end]) {
                        nodes.push(DialectNode {
                            kind: DialectKind::Callout { r#type: ctype, fold },
                            span: trim_block(input, &range),
                        });
                    }
                }
            }
            Event::End(TagEnd::BlockQuote(_)) => bq_depth = bq_depth.saturating_sub(1),
            Event::Start(Tag::List(_)) => list_depth += 1,
            Event::End(TagEnd::List(_)) => list_depth = list_depth.saturating_sub(1),
            Event::Start(Tag::Item) => open_items.push((range, list_depth)),
            Event::End(TagEnd::Item) => {
                open_items.pop();
            }
            Event::TaskListMarker(checked) => {
                if let Some((item, depth)) = open_items.last() {
                    // task span = marker byte to the end of the item's first line
                    let start = item.start;
                    let end = input[start..item.end]
                        .find('\n')
                        .map_or(item.end, |p| start + p);
                    nodes.push(DialectNode {
                        kind: DialectKind::Task {
                            checked,
                            depth: *depth,
                        },
                        span: trim_block(input, &(start..end)),
                    });
                }
            }
            Event::Start(Tag::Table(_)) => nodes.push(DialectNode {
                kind: DialectKind::Table,
                span: trim_block(input, &range),
            }),
            Event::Start(Tag::Link {
                link_type: LinkType::WikiLink { has_pothole, .. },
                dest_url,
                ..
            }) => {
                cur_wikilink = Some((false, range, dest_url.to_string(), String::new(), has_pothole));
            }
            Event::Start(Tag::Image {
                link_type: LinkType::WikiLink { has_pothole, .. },
                dest_url,
                ..
            }) => {
                cur_wikilink = Some((true, range, dest_url.to_string(), String::new(), has_pothole));
            }
            Event::End(TagEnd::Link | TagEnd::Image) => {
                if let Some((is_image, mut wrange, dest, alias, has_pothole)) = cur_wikilink.take() {
                    // no newline inside a wikilink
                    if input[wrange.clone()].contains('\n') {
                        continue;
                    }
                    // `![[x]]` can reach us as a Link preceded by a literal `!` —
                    // fold the `!` into an embed span.
                    let mut embed = is_image;
                    if !embed && wrange.start > 0 && input.as_bytes()[wrange.start - 1] == b'!' {
                        embed = true;
                        wrange.start -= 1;
                    }
                    let (target, heading, block) = split_wikilink_target(&dest);
                    let alias = has_pothole.then_some(alias);
                    let kind = if embed {
                        DialectKind::Embed {
                            target,
                            heading,
                            block,
                            alias,
                        }
                    } else {
                        DialectKind::Wikilink {
                            target,
                            heading,
                            block,
                            alias,
                        }
                    };
                    nodes.push(DialectNode { kind, span: wrange });
                }
            }
            Event::Text(t) => {
                if let Some((_, _, _, alias, _)) = cur_wikilink.as_mut() {
                    alias.push_str(&t);
                }
            }
            _ => {}
        }
    }

    fenced_mask.sort_by_key(|r| r.start);
    scan_anchors(input, &fenced_mask, &inline_mask, &mut nodes);
    scan_comments(input, &fenced_mask, &inline_mask, &fm_mask, &mut nodes);

    // §1 total order: span.start asc, span.end desc (container before contained),
    // then kind ordinal asc — fully deterministic.
    nodes.sort_by(|a, b| {
        a.span
            .start
            .cmp(&b.span.start)
            .then(b.span.end.cmp(&a.span.end))
            .then(kind_ordinal(&a.kind).cmp(&kind_ordinal(&b.kind)))
    });
    nodes
}

/// The fork options the dialect grammar needs (tables, task lists, YAML
/// metadata blocks, wikilinks — strikethrough/footnotes keep pulldown's token
/// stream faithful to the app).
fn parser_options() -> Options {
    let mut o = Options::empty();
    o.insert(Options::ENABLE_TABLES);
    o.insert(Options::ENABLE_TASKLISTS);
    o.insert(Options::ENABLE_STRIKETHROUGH);
    o.insert(Options::ENABLE_FOOTNOTES);
    o.insert(Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);
    o.insert(Options::ENABLE_WIKILINKS);
    o
}

/// Block span: trim trailing line terminators — leaf block nodes exclude the
/// final line terminator (contract §1).
fn trim_block(src: &str, r: &Range<usize>) -> Range<usize> {
    let b = src.as_bytes();
    let mut end = r.end.min(src.len());
    while end > r.start && (b[end - 1] == b'\n' || b[end - 1] == b'\r') {
        end -= 1;
    }
    r.start..end
}

fn heading_num(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// Is this fenced-code slice unterminated (no valid closer line)? A closer must
/// be same-char, run ≥ opener, ≤3 indent, nothing after the run.
fn fence_unterminated(slice: &str) -> bool {
    let mut lines = slice.lines();
    let Some(first) = lines.next() else {
        return true;
    };
    let ft = first.trim_start_matches(' ');
    // not a fence we recognize → don't flag
    let Some(fchar @ ('`' | '~')) = ft.chars().next() else {
        return false;
    };
    let orun = ft.chars().take_while(|&c| c == fchar).count();
    let last = match slice.lines().last() {
        Some(l) if slice.lines().count() > 1 => l,
        _ => return true,
    };
    let indent = last.len() - last.trim_start_matches(' ').len();
    let lt = last.trim_start_matches(' ');
    let crun = lt.chars().take_while(|&c| c == fchar).count();
    !(indent <= 3 && crun >= orun && lt[crun..].trim().is_empty())
}

/// `> [!type]±` callout head (arbitrary Obsidian types; no-space `>[!x]` counts).
fn callout_head(head: &str) -> Option<(String, String)> {
    let b = head.as_bytes();
    let mut i = 0;
    // [ \t]{0,3}
    let mut ws = 0;
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        ws += 1;
        if ws > 3 {
            return None;
        }
        i += 1;
    }
    if b.get(i) != Some(&b'>') {
        return None;
    }
    i += 1;
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    if b.get(i) != Some(&b'[') || b.get(i + 1) != Some(&b'!') {
        return None;
    }
    i += 2;
    let type_start = i;
    while i < b.len() && is_callout_type_char(b[i]) {
        i += 1;
    }
    if i == type_start {
        return None;
    }
    let ctype = head[type_start..i].to_string();
    if b.get(i) != Some(&b']') {
        return None;
    }
    i += 1;
    let fold = match b.get(i) {
        Some(&c) if c == b'+' || c == b'-' => char::from(c).to_string(),
        _ => String::new(),
    };
    Some((ctype, fold))
}

/// Top-level frontmatter keys in document order. Full YAML parsing is `model`'s
/// job (the `YamlMap` build); `syntax` stays dependency-minimal, so this reads
/// only column-0 `key:` lines — the flat frontmatter the corpus uses.
fn frontmatter_keys(block: &str) -> Vec<String> {
    let mut keys = Vec::new();
    for line in block.lines() {
        if line.trim() == "---" || line.trim().is_empty() {
            continue;
        }
        // top-level keys sit at column 0; indented lines are values/nesting
        if line.starts_with([' ', '\t']) || line.trim_start().starts_with('#') {
            continue;
        }
        if let Some(colon) = line.find(':') {
            let key = line[..colon].trim().trim_matches(['"', '\'']);
            if !key.is_empty() {
                keys.push(key.to_string());
            }
        }
    }
    keys
}

/// Split a wikilink dest into `(target, heading, block)` — `#frag` is a heading
/// fragment, `#^frag` a block ref.
fn split_wikilink_target(dest: &str) -> (String, Option<String>, Option<String>) {
    match dest.split_once('#') {
        None => (dest.to_string(), None, None),
        Some((target, frag)) => frag.strip_prefix('^').map_or_else(
            || (target.to_string(), Some(frag.to_string()), None),
            |block| (target.to_string(), None, Some(block.to_string())),
        ),
    }
}

/// The one block-id charset (decision 011 / contract §2.4): `[A-Za-z0-9-]` —
/// Obsidian app-exact (Latin letters, digits, dash), on BOTH the mint and walk
/// planes. THE single normative definition in code; `model::Ref` anchor
/// validation and every downstream mint position route through it. No `_` — the
/// dead two-plane superset is gone.
#[must_use]
pub fn is_block_id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-'
}

/// Is `s` a well-formed block id — non-empty and every char in the one charset
/// ([`is_block_id_char`])? A legacy `_`-bearing id is NOT (ruling 011): outside
/// the strict-plane grammar, refused `bad_request` at the mint boundary.
#[must_use]
pub fn is_block_id(s: &str) -> bool {
    !s.is_empty() && s.chars().all(is_block_id_char)
}

/// Callout-type identifier bytes (`> [!note]`): ASCII alphanumerics plus dash
/// and the legacy underscore. A callout type is not an address — distinct from
/// the block-id charset and untouched by ruling 011.
fn is_callout_type_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

/// Is `pos` inside any range of a sorted, non-overlapping mask?
fn in_mask(mask: &[Range<usize>], pos: usize) -> bool {
    let i = mask.partition_point(|r| r.end <= pos);
    mask.get(i).is_some_and(|r| r.start <= pos && pos < r.end)
}

/// Obsidian block-id anchors: `^id` at line tail only, `^` delimiter included
/// (contract §1 inline node), not inside fenced or inline code.
fn scan_anchors(
    src: &str,
    fenced: &[Range<usize>],
    inline: &[Range<usize>],
    nodes: &mut Vec<DialectNode>,
) {
    let bytes = src.as_bytes();
    let mut line_start = 0;
    loop {
        let nl = src[line_start..].find('\n').map(|p| line_start + p);
        let line_end = nl.unwrap_or(src.len());
        // trailing spaces/tabs and a CRLF `\r` do not count against the tail
        let mut end = line_end;
        while end > line_start && matches!(bytes[end - 1], b' ' | b'\t' | b'\r') {
            end -= 1;
        }
        // collect the block-id run ending at the (trimmed) line tail — app-exact
        // charset (ruling 011); a `_` stops the run, so a `_`-bearing id fails
        // the `^`-delimiter check below and is silently dropped, as the app does.
        let mut i = end;
        while i > line_start && is_block_id_char(char::from(bytes[i - 1])) {
            i -= 1;
        }
        // need ≥1 id char, a `^` delimiter, then start-of-line or space/tab
        if i < end
            && i > line_start
            && bytes[i - 1] == b'^'
            && (i - 1 == line_start || matches!(bytes[i - 2], b' ' | b'\t'))
        {
            let caret = i - 1;
            if !in_mask(fenced, caret) && !in_mask(inline, caret) {
                nodes.push(DialectNode {
                    kind: DialectKind::Anchor {
                        id: src[i..end].to_string(),
                    },
                    span: caret..end,
                });
            }
        }
        match nl {
            Some(p) => line_start = p + 1,
            None => break,
        }
    }
}

/// `%%...%%` comments — paired, non-greedy, may span blocks; the fork
/// deliberately excludes these (the post-pass owns them). Not inside fenced or
/// inline code or frontmatter.
fn scan_comments(
    src: &str,
    fenced: &[Range<usize>],
    inline: &[Range<usize>],
    fm: &[Range<usize>],
    nodes: &mut Vec<DialectNode>,
) {
    let mut starts: Vec<usize> = Vec::new();
    let mut pos = 0;
    while let Some(off) = src[pos..].find("%%") {
        let at = pos + off;
        if !in_mask(fenced, at) && !in_mask(inline, at) && !in_mask(fm, at) {
            starts.push(at);
        }
        pos = at + 2;
    }
    let mut i = 0;
    while i + 1 < starts.len() {
        nodes.push(DialectNode {
            kind: DialectKind::Comment,
            span: starts[i]..starts[i + 1] + 2,
        });
        i += 2;
    }
}

/// Kind ordinal for the §1 total-order tiebreak — the `DialectKind` declaration
/// order (frontmatter = 0 … comment = 10).
fn kind_ordinal(kind: &DialectKind) -> u8 {
    match kind {
        DialectKind::Frontmatter { .. } => 0,
        DialectKind::Heading { .. } => 1,
        DialectKind::Fence { .. } => 2,
        DialectKind::InlineCode => 3,
        DialectKind::Anchor { .. } => 4,
        DialectKind::Wikilink { .. } => 5,
        DialectKind::Embed { .. } => 6,
        DialectKind::Callout { .. } => 7,
        DialectKind::Task { .. } => 8,
        DialectKind::Table => 9,
        DialectKind::Comment => 10,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bytes each node's span covers, paired with its kind ordinal name —
    /// the reader's view of what `parse` extracted.
    fn slices(src: &str) -> Vec<(u8, &str)> {
        parse(src)
            .into_iter()
            .map(|n| (kind_ordinal(&n.kind), &src[n.span]))
            .collect::<Vec<_>>()
    }

    fn first_span(src: &str, want: u8) -> &str {
        let n = parse(src);
        let node = n
            .iter()
            .find(|n| kind_ordinal(&n.kind) == want)
            .unwrap_or_else(|| panic!("no node of ordinal {want} in {src:?}"));
        &src[node.span.clone()]
    }

    /// Every emitted span is in-bounds and on UTF-8 char boundaries — the
    /// read-side guarantor that a span never splits a multi-byte character.
    fn span_faults(src: &str, nodes: &[DialectNode]) -> Vec<String> {
        let mut faults = Vec::new();
        for n in nodes {
            let Range { start, end } = n.span;
            if start > end || end > src.len() {
                faults.push(format!("{:?} span [{start},{end}) out of bounds", n.kind));
            } else if !src.is_char_boundary(start) || !src.is_char_boundary(end) {
                faults.push(format!("{:?} span [{start},{end}) splits a char", n.kind));
            }
        }
        faults
    }

    // ---- Gate 2: span-law fixtures (contract §1) ----

    #[test]
    fn span_law_multibyte_split_refusal() {
        // multi-byte content pressed against every construct boundary; no span
        // may land inside a UTF-8 sequence (§1 multibyte-split refusal, read side).
        let src = "---\ntitle: Café ☕\n---\n\
                   # 日本語 Héading\n\n\
                   Para with `côdé` and [[Nóte#Ünï|älias]] then ![[imäge.png]] end. ^blk-café\n\n\
                   > [!tip] 标题 çà\n> 内容 body ^inquote\n\n\
                   - [x] tâsk 完了 ^ontask\n\n\
                   | ä | 本 |\n|---|---|\n| 1 | 2 |\n\n\
                   ```rûst\nfn 日() {}\n```\n\n\
                   %% cömment 注释 %%\n";
        let nodes = parse(src);
        let faults = span_faults(src, &nodes);
        assert!(faults.is_empty(), "char-splitting spans: {faults:?}");
        // and the doc genuinely exercised multi-byte-bearing constructs
        assert!(nodes.iter().any(|n| matches!(n.kind, DialectKind::Heading { .. })));
        assert!(nodes.iter().any(|n| matches!(n.kind, DialectKind::Callout { .. })));
        assert!(nodes.iter().any(|n| matches!(n.kind, DialectKind::Comment)));
    }

    #[test]
    fn span_law_block_excludes_terminator() {
        // every block kind: span ends at the last byte of its final line, the
        // trailing terminator excluded (§1 leaf-block law).
        assert_eq!(
            first_span("---\ntitle: hi\ntags: [a]\n---\n\n# H\n", 0),
            "---\ntitle: hi\ntags: [a]\n---"
        );
        assert_eq!(first_span("# Heading here\n", 1), "# Heading here");
        assert_eq!(first_span("```rust\nfn x() {}\n```\n", 2), "```rust\nfn x() {}\n```");
        assert_eq!(first_span("> [!note] Title\n> body\n\n", 7), "> [!note] Title\n> body");
        assert_eq!(first_span("- [ ] do the thing\n", 8), "- [ ] do the thing");
        assert_eq!(
            first_span("| a | b |\n|---|---|\n| 1 | 2 |\n", 9),
            "| a | b |\n|---|---|\n| 1 | 2 |"
        );
        // none of them carry a trailing newline
        for kind in [0u8, 1, 2, 7, 8, 9] {
            for src in [
                "---\nk: v\n---\n",
                "# H\n",
                "```\nx\n```\n",
                "> [!n] t\n> b\n",
                "- [ ] t\n",
                "| a |\n|---|\n| 1 |\n",
            ] {
                for n in parse(src) {
                    if kind_ordinal(&n.kind) == kind {
                        assert!(!src[n.span].ends_with('\n'));
                    }
                }
            }
        }
    }

    #[test]
    fn span_law_inline_includes_delimiters() {
        // inline nodes carry their own delimiters (§1): backticks, `[[ ]]`, the
        // `!` of an embed, the `^` of an anchor.
        assert_eq!(first_span("text `code here` more\n", 3), "`code here`");
        assert_eq!(first_span("see [[Page One]] now\n", 5), "[[Page One]]");
        assert_eq!(first_span("here ![[img.png]] end\n", 6), "![[img.png]]");
        assert_eq!(first_span("paragraph tail ^my-anchor\n", 4), "^my-anchor");
    }

    // ---- Relocation behaviour (adapted from the parser-bench donor tests) ----

    #[test]
    fn frontmatter_span_and_keys() {
        let src = "---\ntitle: hi\ntags: [a]\n---\n\n# H\n";
        let fm = parse(src)
            .into_iter()
            .find(|n| matches!(n.kind, DialectKind::Frontmatter { .. }))
            .unwrap();
        assert_eq!(&src[fm.span], "---\ntitle: hi\ntags: [a]\n---");
        let DialectKind::Frontmatter { keys } = fm.kind else {
            unreachable!()
        };
        assert_eq!(keys, vec!["title".to_string(), "tags".to_string()]);
    }

    #[test]
    fn bom_prefixed_dashes_not_frontmatter() {
        let src = "\u{feff}---\ntitle: hi\n---\nbody\n";
        assert!(parse(src)
            .iter()
            .all(|n| !matches!(n.kind, DialectKind::Frontmatter { .. })));
    }

    #[test]
    fn headings_atx_only_level_and_text() {
        // level + written text (backticks kept); setext not extracted; no hpath.
        let src = "# A\n## B `c`\ntext\nSetext\n======\n### D\n";
        let heads: Vec<(u8, String)> = parse(src)
            .into_iter()
            .filter_map(|n| match n.kind {
                DialectKind::Heading { level, text } => Some((level, text)),
                _ => None,
            })
            .collect();
        assert_eq!(
            heads,
            vec![
                (1, "A".to_string()),
                (2, "B `c`".to_string()),
                (3, "D".to_string()),
            ]
        );
    }

    #[test]
    fn wikilinks_embeds_and_fragments() {
        let src = "See [[Page One|alias]] and [[Other#Head]] and [[X#^blk]] and ![[img.png]].\n";
        let nodes = parse(src);
        // one embed, three wikilinks, delimiters included
        assert_eq!(
            slices(src)
                .into_iter()
                .filter(|(k, _)| *k == 5 || *k == 6)
                .map(|(_, s)| s)
                .collect::<Vec<_>>(),
            vec!["[[Page One|alias]]", "[[Other#Head]]", "[[X#^blk]]", "![[img.png]]"]
        );
        let wl: Vec<_> = nodes
            .iter()
            .filter_map(|n| match &n.kind {
                DialectKind::Wikilink { target, heading, block, alias } => {
                    Some((target.clone(), heading.clone(), block.clone(), alias.clone()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(wl[0], ("Page One".into(), None, None, Some("alias".into())));
        assert_eq!(wl[1], ("Other".into(), Some("Head".into()), None, None));
        assert_eq!(wl[2], ("X".into(), None, Some("blk".into()), None));
        assert!(nodes.iter().any(|n| matches!(&n.kind, DialectKind::Embed { target, .. } if target == "img.png")));
    }

    #[test]
    fn callouts_top_level_only_with_fold() {
        let src = "> [!note] Title\n> body\n\n> plain quote\n\n>[!custom-type]- folded\n> x\n\n> outer\n> > [!nested] inner\n";
        let callouts: Vec<(String, String)> = parse(src)
            .into_iter()
            .filter_map(|n| match n.kind {
                DialectKind::Callout { r#type, fold } => Some((r#type, fold)),
                _ => None,
            })
            .collect();
        assert_eq!(
            callouts,
            vec![
                ("note".to_string(), String::new()),
                ("custom-type".to_string(), "-".to_string()),
            ]
        );
    }

    #[test]
    fn anchors_masked_in_code_not_unicode() {
        let src = "para ^anchor1\n\n```\ncode ^notanchor\n%% not comment %%\n```\n\nline ^a2\nmid ^not anchor\nuni ^ünïcode\n";
        assert_eq!(
            slices(src)
                .into_iter()
                .filter(|(k, _)| *k == 4)
                .map(|(_, s)| s)
                .collect::<Vec<_>>(),
            vec!["^anchor1", "^a2"]
        );
    }

    #[test]
    fn anchor_underscore_rejected_app_exact() {
        // CHARSET-GUARD (ruling 011 / contract §2.4): block ids are app-exact
        // `[A-Za-z0-9-]`. A `_`-bearing id is outside the charset, so the lexer
        // — matching the app's silent drop — does not scan it as an anchor; a
        // clean dash-bearing id still scans.
        let src = "dropped ^b_1\nalso dropped ^mixed-id_2\nkept ^mixed-id-2\n";
        assert_eq!(
            slices(src)
                .into_iter()
                .filter(|(k, _)| *k == 4)
                .map(|(_, s)| s)
                .collect::<Vec<_>>(),
            vec!["^mixed-id-2"]
        );
    }

    #[test]
    fn is_block_id_enforces_one_charset() {
        // The single normative predicate: app-exact, both planes (ruling 011).
        assert!(is_block_id("r-000042"));
        assert!(is_block_id("clean-1"));
        assert!(is_block_id("Beta2"));
        assert!(!is_block_id("under-probe_x")); // `_` outside the charset
        assert!(!is_block_id("a_04"));
        assert!(!is_block_id("")); // empty is not an id
        assert!(!is_block_id("has space"));
    }

    #[test]
    fn crlf_anchor_tail() {
        let src = "line one ^crlf-anchor\r\nplain\r\n";
        assert_eq!(first_span(src, 4), "^crlf-anchor");
    }

    #[test]
    fn comments_paired_non_greedy() {
        let src = "%% one %%\ntext\n%% unclosed\n";
        assert_eq!(first_span(src, 10), "%% one %%");
        assert_eq!(
            parse(src).iter().filter(|n| matches!(n.kind, DialectKind::Comment)).count(),
            1
        );
    }

    #[test]
    fn task_first_line_and_table_and_no_plain_items() {
        let src = "- [ ] todo line\n  continuation\n- [x] done\n- plain\n\n| a | b |\n|---|---|\n| 1 | 2 |\n";
        let tasks: Vec<(bool, &str)> = parse(src)
            .into_iter()
            .filter_map(|n| match n.kind {
                DialectKind::Task { checked, .. } => Some((checked, "")),
                _ => None,
            })
            .zip(["- [ ] todo line", "- [x] done"])
            .map(|((checked, _), slice)| (checked, slice))
            .collect();
        assert_eq!(tasks, vec![(false, "- [ ] todo line"), (true, "- [x] done")]);
        assert_eq!(first_span(src, 8), "- [ ] todo line");
        assert_eq!(first_span(src, 9), "| a | b |\n|---|---|\n| 1 | 2 |");
    }

    #[test]
    fn unterminated_fence_flagged() {
        let open = parse("# H\n```rust\nfn x() {}\n");
        assert!(open.iter().any(|n| matches!(
            &n.kind,
            DialectKind::Fence { unterminated: true, info_string } if info_string == "rust"
        )));
        let closed = parse("```\nok\n```\n");
        assert!(closed.iter().any(|n| matches!(
            n.kind,
            DialectKind::Fence { unterminated: false, .. }
        )));
    }

    #[test]
    fn cjk_blockquote_probe_no_panic() {
        // the 256-byte callout-head probe must clamp to a char boundary
        let body = "什么这是一个很长的中文引用块".repeat(20);
        let plain = parse(&format!("> {body}\n"));
        assert!(plain.iter().all(|n| !matches!(n.kind, DialectKind::Callout { .. })));
        let callout = parse(&format!("> [!note] 标题\n> {body}\n"));
        assert_eq!(
            callout.iter().filter(|n| matches!(n.kind, DialectKind::Callout { .. })).count(),
            1
        );
    }

    #[test]
    fn total_order_is_deterministic() {
        // overlapping heading + anchor: sorted by start asc, end desc, kind asc
        let src = "## Target ^h-anchor\n";
        let ords: Vec<u8> = parse(src).iter().map(|n| kind_ordinal(&n.kind)).collect();
        // heading [0,19] starts before anchor [10,19]
        assert_eq!(ords, vec![1, 4]);
    }
}
