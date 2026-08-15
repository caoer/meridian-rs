//! Delta change-fact computation (contract §7, D3-DELTA — wire-lane-owned).
//!
//! Computes Delta facts only (no-serde charter); `wire-map` projects to
//! `wire::Delta`; the serving host owns the envelope (seq/roots/actor/now, §9).
//!
//! **Deepest-changed-node (D-C7, §7.1):** entries name the deepest
//! mint-addressable node per changed byte range (sections, anchor host-blocks,
//! fm keys). Ancestor revs change implicitly; re-readable via `toc`, never
//! duplicated into the delta.
//!
//! **Node-grain (decision 012, §7.4):** grain is exactly [`NodeDelta`] —
//! §2.1 identity, rev transition, span after. No key-grain sub-entries.
//!
//! **Regions (sub-node-grain ruling, 2026-08-14 — dogfood r1 prober F1):**
//! changed ranges are LINE-GRAIN regions (unique-line anchored diff), one
//! entry set per region — a multi-edit batch reports each touched node
//! instead of rolling up to the ancestor that contains the whole byte range,
//! and a batch spanning frontmatter and body loses neither half. Section
//! identities carry the §2.1 occurrence disambiguator wherever siblings
//! duplicate, and a `removed` entry is minted only on a PROVEN absence
//! ([`crate::ResolveError::NotFound`]) — an ambiguous identity is never
//! fabricated into a removal; honest omission degrades to the file-grain
//! fact. Classifies `created`/`modified`/`deleted`; `renamed` needs
//! cross-file correlation only callers have.

use crate::{ByteSpan, Document, Node, NodeKind, NodeRev, Ref, ResolveError, resolve};

/// File-level change class this module can derive from two states of one
/// path. `renamed` is the caller's (cross-file knowledge).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileChangeKind {
    Created,
    Modified,
    Deleted,
}

/// v2 §7.1 node change class, plus the v3 `anchored` word.
///
/// `Anchored` says the node moved SOLELY by gaining an anchor id — an
/// attestation minted onto it, not content someone rewrote. It is a byte
/// verdict, never an intent: a write that changes content and mints an anchor
/// in the same node stays [`NodeChangeKind::Edited`], because content did
/// change and a reader must not be told otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeChangeKind {
    Added,
    Edited,
    Anchored,
    Removed,
}

/// One node-grain change fact: identity in the §2.1 grammar ([`Ref`]), rev
/// transition, span after. Absences follow §7.1: no `node_rev_before` on
/// `added`, no `node_rev_after`/`span_after` on `removed`.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeDelta {
    pub target: Ref,
    pub change: NodeChangeKind,
    pub node_rev_before: Option<NodeRev>,
    pub node_rev_after: Option<NodeRev>,
    pub span_after: Option<ByteSpan>,
}

/// One file's change facts: class, file-rev transition (absent-by-tense per
/// §7.1: no `before` on created, no `after` on deleted), node entries.
#[derive(Debug, Clone, PartialEq)]
pub struct FileDelta {
    pub change: FileChangeKind,
    pub file_rev_before: Option<NodeRev>,
    pub file_rev_after: Option<NodeRev>,
    pub nodes: Vec<NodeDelta>,
}

/// Compute one path's change facts between two states. `None` when nothing
/// changed (identical bytes) or both states are absent. Created/deleted files
/// carry no node entries — the file-level fact is the change, and the node
/// inventory is re-readable via `toc` (§7.1).
#[must_use]
pub fn file_delta(before: Option<&Document>, after: Option<&Document>) -> Option<FileDelta> {
    match (before, after) {
        (None, None) => None,
        (None, Some(a)) => Some(FileDelta {
            change: FileChangeKind::Created,
            file_rev_before: None,
            file_rev_after: Some(a.root.node_rev.clone()),
            nodes: Vec::new(),
        }),
        (Some(b), None) => Some(FileDelta {
            change: FileChangeKind::Deleted,
            file_rev_before: Some(b.root.node_rev.clone()),
            file_rev_after: None,
            nodes: Vec::new(),
        }),
        (Some(b), Some(a)) => {
            if b.root.node_rev == a.root.node_rev {
                return None; // unchanged
            }
            Some(FileDelta {
                change: FileChangeKind::Modified,
                file_rev_before: Some(b.root.node_rev.clone()),
                file_rev_after: Some(a.root.node_rev.clone()),
                nodes: node_deltas(b, a),
            })
        }
    }
}

/// The node entries for one modified file (D-C7, region-grain). Per changed
/// region, two passes, one per tense:
///
/// - **After-side** (`added`/`edited`): the deepest addressable node
///   containing the region — leading blank lines (structural glue) and the
///   final line terminator trimmed first, so an appended block or section
///   matches itself, never its host. A region inside the frontmatter block
///   reports EACH changed key line.
/// - **Before-side** (`removed`): the shallowest before-nodes intersecting
///   the region whose identity PROVABLY no longer resolves (`NotFound`); a
///   removed subtree's descendants stay implicit, and an ambiguous identity
///   is never reported — a feed must not fabricate a removal.
///
/// A pure append has an empty before-region (no removed scan); a pure
/// deletion has an empty after-region (no added/edited entry). Entries
/// dedupe by identity across regions.
///
/// **Anchor mints (§7.1 `anchored`):** a region whose whole content is one
/// newly-resolving anchor marker reports its node `anchored` instead of
/// `edited` — attesting to a page must never read as rewriting it. The
/// classification is per region and `edited` wins: a node touched by any
/// other region in the same batch carries the honest content verdict.
#[must_use]
pub fn node_deltas(before: &Document, after: &Document) -> Vec<NodeDelta> {
    let mut out = Vec::new();
    for (b_range, a_range) in changed_regions(&before.raw, &after.raw) {
        if !a_range.is_empty() {
            let mint = anchor_mint(before, after, &b_range, &a_range);
            let probe = trim_final_terminator(&after.raw, a_range);
            if !probe.is_empty() {
                collect_touched(before, after, &after.root, &probe, mint, &mut out);
            }
        }
        if !b_range.is_empty() {
            let probe = trim_final_terminator(&before.raw, b_range);
            collect_removed(before, after, &before.root, &probe, &mut out);
        }
    }
    out
}

/// After-side DFS: the DEEPEST addressable nodes intersecting the region —
/// a node with an intersecting addressable descendant recurses instead of
/// reporting itself (ancestor revs stay implicit, D-C7), a frontmatter node
/// reports each key line the region intersects, and structural glue between
/// children (blank lines, a heading's trailing terminator) names nobody. A
/// region spanning several siblings therefore reports EACH of them; `added`
/// vs `edited` is the identity's own tense in `before`.
fn collect_touched(
    before: &Document,
    after: &Document,
    node: &Node,
    range: &ByteSpan,
    mint: bool,
    out: &mut Vec<NodeDelta>,
) {
    // The root's span covers the whole file, so a non-empty probe always
    // intersects it; children gate themselves here.
    let intersects = node.span.start < range.end && range.start < node.span.end;
    if !intersects {
        return;
    }
    if let NodeKind::Frontmatter { map } = &node.kind {
        for key in map.keys() {
            let target = Ref::FmKey(key.to_string());
            let Ok(t) = resolve(after, &target) else {
                continue;
            };
            if t.span.start < range.end && range.start < t.span.end {
                push_touched(before, target, &t.node_rev, &t.span, mint, out);
            }
        }
        return;
    }
    let addressable_children: Vec<&Node> = node
        .children
        .iter()
        .filter(|c| c.span.start < range.end && range.start < c.span.end && addressable_subtree(c))
        .collect();
    if addressable_children.is_empty() {
        // Deepest intersecting addressable node — report it, if it is one.
        if let Some(target) = identity_of(after, node, range) {
            push_touched(before, target, &node.node_rev, &node.span, mint, out);
        }
        return;
    }
    for child in addressable_children {
        collect_touched(before, after, child, range, mint, out);
    }
}

/// Does this subtree hold ANY addressable node (section, anchor,
/// frontmatter)? Non-addressable wrappers recurse so an anchor nested in
/// plain blocks still counts.
fn addressable_subtree(node: &Node) -> bool {
    matches!(
        node.kind,
        NodeKind::Section { .. } | NodeKind::Anchor { .. } | NodeKind::Frontmatter { .. }
    ) || node.children.iter().any(addressable_subtree)
}

/// One after-side entry, tense resolved in `before`: `edited` on a surviving
/// identity whose rev moved (`anchored` where that move was only an anchor
/// mint), `added` on a proven absence. An `Ambiguous` before-identity stays
/// silent — with occurrence-qualified section identities the arm is
/// vestigial, and a feed that cannot name the prior node truthfully says
/// nothing rather than guessing (file-grain honesty).
///
/// A repeat touch of an identity already entered keeps the entry the first
/// region minted, with one exception: a NON-mint region promotes a standing
/// `anchored` entry to `edited`. Content did change, so the row must say so —
/// the mint verdict is only true for a node nothing else touched.
fn push_touched(
    before: &Document,
    target: Ref,
    rev_after: &NodeRev,
    span_after: &ByteSpan,
    mint: bool,
    out: &mut Vec<NodeDelta>,
) {
    if let Some(seen) = out.iter_mut().find(|d| d.target == target) {
        if !mint && seen.change == NodeChangeKind::Anchored {
            seen.change = NodeChangeKind::Edited;
        }
        return;
    }
    match resolve(before, &target) {
        Ok(prior) => {
            if prior.node_rev != *rev_after {
                out.push(NodeDelta {
                    target,
                    change: if mint {
                        NodeChangeKind::Anchored
                    } else {
                        NodeChangeKind::Edited
                    },
                    node_rev_before: Some(prior.node_rev),
                    node_rev_after: Some(rev_after.clone()),
                    span_after: Some(span_after.clone()),
                });
            }
        }
        Err(ResolveError::NotFound) => out.push(NodeDelta {
            target,
            change: NodeChangeKind::Added,
            node_rev_before: None,
            node_rev_after: Some(rev_after.clone()),
            span_after: Some(span_after.clone()),
        }),
        Err(ResolveError::Ambiguous(_)) => {}
    }
}

/// Is this region exactly one anchor mint? A byte verdict with three parts,
/// all required:
///
/// 1. deleting one `^id` marker from the after-region yields the
///    before-region byte for byte — so nothing else in the region changed;
/// 2. `id` resolves as an anchor in `after`;
/// 3. `id` does NOT resolve in `before` — a marker that merely moved is not
///    a mint, and the node that lost it reports its own honest change.
///
/// Both mint spellings qualify: the marker on its own line (the pin door's
/// own form) and the tail id appended to a content line.
fn anchor_mint(before: &Document, after: &Document, b: &ByteSpan, a: &ByteSpan) -> bool {
    let a_text = &after.raw[a.clone()];
    let b_text = &before.raw[b.clone()];
    markers(a_text).into_iter().any(|(span, id)| {
        strip_marker(a_text, &span) == b_text
            && Ref::anchor(id).is_ok_and(|r| {
                resolve(after, &r).is_ok()
                    && matches!(resolve(before, &r), Err(ResolveError::NotFound))
            })
    })
}

/// Every `^id` marker in `text`: its span and its id. The id grammar is
/// [`Ref::anchor`]'s (`[A-Za-z0-9-]+`); a `^` starting no valid id is not a
/// marker.
fn markers(text: &str) -> Vec<(ByteSpan, &str)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    for (i, _) in text.match_indices('^') {
        let end = bytes[i + 1..]
            .iter()
            .position(|b| !(b.is_ascii_alphanumeric() || *b == b'-'))
            .map_or(bytes.len(), |n| i + 1 + n);
        if end > i + 1 {
            out.push((i..end, &text[i + 1..end]));
        }
    }
    out
}

/// `text` without the marker at `span`, cleaned the way the mint wrote it: a
/// marker alone on its line takes the whole line with it, a tail id takes the
/// whitespace that separated it from the content.
fn strip_marker(text: &str, span: &ByteSpan) -> String {
    let bytes = text.as_bytes();
    let line_start = text[..span.start].rfind('\n').map_or(0, |n| n + 1);
    let line_end = text[span.end..]
        .find('\n')
        .map_or(text.len(), |n| span.end + n + 1);
    let alone = bytes[line_start..span.start]
        .iter()
        .all(u8::is_ascii_whitespace)
        && bytes[span.end..line_end]
            .iter()
            .all(u8::is_ascii_whitespace);
    let (cut_start, cut_end) = if alone {
        (line_start, line_end)
    } else {
        let mut start = span.start;
        while start > line_start && bytes[start - 1].is_ascii_whitespace() {
            start -= 1;
        }
        (start, span.end)
    };
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..cut_start]);
    out.push_str(&text[cut_end..]);
    out
}

/// Block-leaf spans exclude their final line terminator (§1 span law); a
/// changed range that ends on one matches candidates up to it.
fn trim_final_terminator(raw: &str, r: ByteSpan) -> ByteSpan {
    if r.end > r.start && raw.as_bytes()[r.end - 1] == b'\n' {
        r.start..r.end - 1
    } else {
        r
    }
}

/// DFS for removed identities: a node intersecting the changed range whose
/// identity PROVABLY no longer resolves (`NotFound`) is a `removed` entry,
/// and its subtree stays implicit. A resolving ancestor recurses — a removed
/// child may hide under a surviving parent. An `Ambiguous` answer is NOT
/// absence: nothing is reported for it (a feed must never fabricate a
/// removal), and with occurrence-qualified section identities the arm is
/// vestigial anyway. A frontmatter node reports each removed key the range
/// intersects.
fn collect_removed(
    before: &Document,
    after: &Document,
    node: &Node,
    range: &ByteSpan,
    out: &mut Vec<NodeDelta>,
) {
    let intersects = node.span.start < range.end && range.start < node.span.end;
    if !intersects && !range.is_empty() {
        return;
    }
    if let NodeKind::Frontmatter { map } = &node.kind {
        for key in map.keys() {
            let target = Ref::FmKey(key.to_string());
            let Ok(t) = resolve(before, &target) else {
                continue;
            };
            if t.span.start < range.end
                && range.start < t.span.end
                && matches!(resolve(after, &target), Err(ResolveError::NotFound))
                && !out.iter().any(|d| d.target == target)
            {
                out.push(NodeDelta {
                    target,
                    change: NodeChangeKind::Removed,
                    node_rev_before: Some(t.node_rev),
                    node_rev_after: None,
                    span_after: None,
                });
            }
        }
        return;
    }
    if let Some(target) = identity_of(before, node, range)
        && matches!(resolve(after, &target), Err(ResolveError::NotFound))
    {
        if !out.iter().any(|d| d.target == target) {
            out.push(NodeDelta {
                target,
                change: NodeChangeKind::Removed,
                node_rev_before: Some(node.node_rev.clone()),
                node_rev_after: None,
                span_after: None,
            });
        }
        return; // descendants of a removed node stay implicit
    }
    for child in &node.children {
        collect_removed(before, after, child, range, out);
    }
}

/// Line-grain changed regions, in order, non-overlapping — unique-line
/// anchored (patience-style): common lines unique in both texts partition
/// them, and each unanchored stretch emits one region pair. Either side of a
/// pair may be empty (pure insertion/deletion). Empty when byte-identical.
fn changed_regions(before: &str, after: &str) -> Vec<(ByteSpan, ByteSpan)> {
    if before == after {
        return Vec::new();
    }
    let b_lines = line_spans(before);
    let a_lines = line_spans(after);
    let mut out = Vec::new();
    partition(
        before,
        after,
        &b_lines,
        &a_lines,
        0..b_lines.len(),
        0..a_lines.len(),
        &mut out,
    );
    out
}

/// Per-line byte spans, terminator included.
fn line_spans(raw: &str) -> Vec<ByteSpan> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, b) in raw.bytes().enumerate() {
        if b == b'\n' {
            out.push(start..i + 1);
            start = i + 1;
        }
    }
    if start < raw.len() {
        out.push(start..raw.len());
    }
    out
}

/// One partition step over line index ranges: trim common edge lines, anchor
/// on lines unique to both slices (longest increasing anchor chain), recurse
/// between anchors; an unanchorable stretch is one region pair.
fn partition(
    b_raw: &str,
    a_raw: &str,
    b_lines: &[ByteSpan],
    a_lines: &[ByteSpan],
    mut b: std::ops::Range<usize>,
    mut a: std::ops::Range<usize>,
    out: &mut Vec<(ByteSpan, ByteSpan)>,
) {
    // Trim common prefix/suffix lines.
    while !b.is_empty()
        && !a.is_empty()
        && line_eq(b_raw, a_raw, &b_lines[b.start], &a_lines[a.start])
    {
        b.start += 1;
        a.start += 1;
    }
    while !b.is_empty()
        && !a.is_empty()
        && line_eq(b_raw, a_raw, &b_lines[b.end - 1], &a_lines[a.end - 1])
    {
        b.end -= 1;
        a.end -= 1;
    }
    if b.is_empty() && a.is_empty() {
        return;
    }
    if b.is_empty() || a.is_empty() {
        out.push((bytes_of(b_raw, b_lines, &b), bytes_of(a_raw, a_lines, &a)));
        return;
    }
    // Patience anchors: line contents unique in BOTH slices.
    let anchors = anchor_chain(b_raw, a_raw, b_lines, a_lines, &b, &a);
    if anchors.is_empty() {
        out.push((bytes_of(b_raw, b_lines, &b), bytes_of(a_raw, a_lines, &a)));
        return;
    }
    let (mut pb, mut pa) = (b.start, a.start);
    for (bi, ai) in anchors {
        partition(b_raw, a_raw, b_lines, a_lines, pb..bi, pa..ai, out);
        (pb, pa) = (bi + 1, ai + 1);
    }
    partition(b_raw, a_raw, b_lines, a_lines, pb..b.end, pa..a.end, out);
}

fn line_eq(b_raw: &str, a_raw: &str, b: &ByteSpan, a: &ByteSpan) -> bool {
    b_raw[b.clone()] == a_raw[a.clone()]
}

/// The byte span a line index range covers (empty ranges collapse to their
/// boundary offset — position only, never content).
fn bytes_of(raw: &str, lines: &[ByteSpan], r: &std::ops::Range<usize>) -> ByteSpan {
    if r.is_empty() {
        let at = lines.get(r.start).map_or(raw.len(), |l| l.start);
        return at..at;
    }
    lines[r.start].start..lines[r.end - 1].end
}

/// Lines unique in both slices, paired by content, kept as a longest
/// strictly-increasing chain on the `after` side — the patience anchor set.
fn anchor_chain(
    b_raw: &str,
    a_raw: &str,
    b_lines: &[ByteSpan],
    a_lines: &[ByteSpan],
    b: &std::ops::Range<usize>,
    a: &std::ops::Range<usize>,
) -> Vec<(usize, usize)> {
    use std::collections::HashMap;
    let mut b_seen: HashMap<&str, Option<usize>> = HashMap::new();
    for i in b.clone() {
        b_seen
            .entry(&b_raw[b_lines[i].clone()])
            .and_modify(|e| *e = None)
            .or_insert(Some(i));
    }
    let mut a_seen: HashMap<&str, Option<usize>> = HashMap::new();
    for i in a.clone() {
        a_seen
            .entry(&a_raw[a_lines[i].clone()])
            .and_modify(|e| *e = None)
            .or_insert(Some(i));
    }
    // Pairs in b-order with both sides unique.
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    for i in b.clone() {
        let text = &b_raw[b_lines[i].clone()];
        if let (Some(Some(bi)), Some(Some(ai))) = (b_seen.get(text), a_seen.get(text))
            && *bi == i
        {
            pairs.push((i, *ai));
        }
    }
    // Longest strictly-increasing subsequence on the after index.
    let mut tails: Vec<usize> = Vec::new(); // indices into pairs
    let mut back: Vec<Option<usize>> = vec![None; pairs.len()];
    for (idx, &(_, ai)) in pairs.iter().enumerate() {
        let pos = tails.partition_point(|&t| pairs[t].1 < ai);
        back[idx] = pos.checked_sub(1).map(|p| tails[p]);
        if pos == tails.len() {
            tails.push(idx);
        } else {
            tails[pos] = idx;
        }
    }
    let mut chain = Vec::new();
    let mut cur = tails.last().copied();
    while let Some(i) = cur {
        chain.push(pairs[i]);
        cur = back[i];
    }
    chain.reverse();
    chain
}

fn contains(span: &ByteSpan, range: &ByteSpan) -> bool {
    span.start <= range.start && range.end <= span.end
}

/// The §2.1 identity of an addressable candidate node, if it is one.
/// Section identities are occurrence-qualified (sub-node-grain ruling): a
/// path segment whose heading text repeats among its siblings carries the
/// 1-based `n`, so the identity resolves to exactly this node — without it,
/// a duplicate resolves `Ambiguous` and every consumer of the entry (and the
/// removal scan above) would be reasoning about the wrong node.
fn identity_of(doc: &Document, node: &Node, range: &ByteSpan) -> Option<Ref> {
    match &node.kind {
        NodeKind::Section { .. } => section_ref_of(doc, node),
        NodeKind::Anchor { name } => Ref::anchor(name.clone()).ok(),
        NodeKind::Frontmatter { map } => {
            // refine to the changed key line
            map.keys().find_map(|key| {
                let target = Ref::FmKey(key.to_string());
                let t = resolve(doc, &target).ok()?;
                contains(&t.span, range).then_some(target)
            })
        }
        _ => None,
    }
}

/// The occurrence-qualified hpath of a section node: walk the section chain
/// root→node by span containment; at each level, `n` is the node's 1-based
/// position among same-heading siblings — set only where the heading
/// duplicates, so unique paths keep their bare §2.1 form byte-for-byte.
fn section_ref_of(doc: &Document, node: &Node) -> Option<Ref> {
    let mut segs = Vec::new();
    let mut scope = &doc.root;
    loop {
        let next = section_children(scope)
            .find(|c| contains(&c.span, &node.span) || c.span == node.span)?;
        let h = heading_of(next)?.to_owned();
        let same: Vec<&Node> = section_children(scope)
            .filter(|c| heading_of(c) == Some(h.as_str()))
            .collect();
        let n = (same.len() > 1)
            .then(|| {
                same.iter()
                    .position(|c| std::ptr::eq(*c, next))
                    .and_then(|p| u32::try_from(p + 1).ok())
            })
            .flatten();
        segs.push(crate::HpathSeg { h, n });
        if std::ptr::eq(next, node) {
            return Some(Ref::Hpath(segs));
        }
        scope = next;
    }
}

/// A node's section children, in document order.
fn section_children(node: &Node) -> impl Iterator<Item = &Node> {
    node.children
        .iter()
        .filter(|c| matches!(c.kind, NodeKind::Section { .. }))
}

/// A section node's heading text.
fn heading_of(node: &Node) -> Option<&str> {
    match &node.kind {
        NodeKind::Section { heading_text, .. } => Some(heading_text.as_str()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build;

    fn doc(raw: &str) -> Document {
        build(raw.to_string(), syntax::parse(raw))
    }

    /// D-C7: an edit inside a nested section names the deepest section, not
    /// its ancestors, and never duplicates ancestor entries.
    #[test]
    fn edited_names_deepest_section_only() {
        let b = doc("# A\n\n## B\n\nold text\n");
        let a = doc("# A\n\n## B\n\nnew text\n");
        let got = node_deltas(&b, &a);
        assert_eq!(got.len(), 1, "no ancestor duplication: {got:?}");
        let d = &got[0];
        assert_eq!(
            d.target,
            Ref::Hpath(vec![
                crate::HpathSeg {
                    h: "A".into(),
                    n: None
                },
                crate::HpathSeg {
                    h: "B".into(),
                    n: None
                }
            ])
        );
        assert_eq!(d.change, NodeChangeKind::Edited);
        assert!(d.node_rev_before.is_some() && d.node_rev_after.is_some());
        assert_eq!(d.span_after.as_ref(), Some(&(5..a.raw.len())));
    }

    /// The F3 row (dogfood-r9-user): a pin mints its anchor into the TARGET
    /// section, and the section must NOT report `edited` — attesting to a
    /// page never reads as rewriting it.
    #[test]
    fn anchor_mint_reports_anchored_not_edited() {
        let b = doc("# R\n\n## Acceptance walk\n\nA consequential section.\n");
        let a = doc("# R\n\n## Acceptance walk\n^acceptance-walk\n\nA consequential section.\n");
        let got = node_deltas(&b, &a);
        let sec = got
            .iter()
            .find(|d| matches!(&d.target, Ref::Hpath(segs) if segs.last().unwrap().h == "Acceptance walk"))
            .expect("the minted-into section reports: {got:?}");
        assert_eq!(sec.change, NodeChangeKind::Anchored);
        assert!(sec.node_rev_before.is_some() && sec.node_rev_after.is_some());
    }

    /// The tail spelling mints the same verdict.
    #[test]
    fn tail_anchor_mint_reports_anchored() {
        let b = doc("# R\n\n## Walk\n\nbody line\n");
        let a = doc("# R\n\n## Walk\n\nbody line ^walk-1\n");
        let got = node_deltas(&b, &a);
        assert_eq!(got.len(), 1, "one entry: {got:?}");
        assert_eq!(got[0].change, NodeChangeKind::Anchored);
    }

    /// Content wins: a write that rewrites the section AND mints an anchor
    /// says `edited`, because content did change.
    #[test]
    fn content_edit_beside_a_mint_stays_edited() {
        let b = doc("# R\n\n## Walk\n\nold body\n");
        let a = doc("# R\n\n## Walk\n^walk-2\n\nnew body\n");
        let got = node_deltas(&b, &a);
        let sec = got
            .iter()
            .find(|d| matches!(&d.target, Ref::Hpath(segs) if segs.last().unwrap().h == "Walk"))
            .expect("the section reports: {got:?}");
        assert_eq!(sec.change, NodeChangeKind::Edited);
    }

    /// A marker that MOVED is not a mint: the id already resolved in
    /// `before`, so the receiving node reports its own content change.
    #[test]
    fn moved_anchor_is_not_a_mint() {
        let b = doc("# R\n\n## A\n^m1\n\n## B\n\nbody\n");
        let a = doc("# R\n\n## A\n\n## B\n^m1\n\nbody\n");
        let got = node_deltas(&b, &a);
        assert!(
            got.iter().all(|d| d.change != NodeChangeKind::Anchored),
            "a moved marker mints nothing: {got:?}"
        );
    }

    /// An appended anchor-bearing block echoes as the anchor added; the host
    /// section's rev change stays implicit.
    #[test]
    fn appended_anchor_block_is_added_anchor_entry() {
        let b = doc("# R\n");
        let a = doc("# R\n- receipt line ^r-000042\n");
        let got = node_deltas(&b, &a);
        assert_eq!(got.len(), 1);
        let d = &got[0];
        assert_eq!(d.target, Ref::anchor("r-000042").unwrap());
        assert_eq!(d.change, NodeChangeKind::Added);
        assert_eq!(d.node_rev_before, None, "added: no before rev (§7.1)");
        assert_eq!(
            d.span_after.as_ref(),
            Some(&(4..28)),
            "host-block leaf span"
        );
    }

    /// A removed section is a `removed` entry: before-rev only, no after
    /// facts (§7.1 tense absences).
    #[test]
    fn removed_section_carries_before_rev_only() {
        let b = doc("# A\n\n## Gone\n\nbody\n\n## Stays\n\nx\n");
        let a = doc("# A\n\n## Stays\n\nx\n");
        let got = node_deltas(&b, &a);
        assert_eq!(got.len(), 1, "{got:?}");
        let d = &got[0];
        assert_eq!(
            d.target,
            Ref::Hpath(vec![
                crate::HpathSeg {
                    h: "A".into(),
                    n: None
                },
                crate::HpathSeg {
                    h: "Gone".into(),
                    n: None
                }
            ])
        );
        assert_eq!(d.change, NodeChangeKind::Removed);
        assert!(d.node_rev_before.is_some());
        assert_eq!(d.node_rev_after, None);
        assert_eq!(d.span_after, None);
    }

    /// A frontmatter edit refines to the changed key line (`fm_key` — the
    /// §2.1 frontmatter plane), not the whole block.
    #[test]
    fn frontmatter_edit_names_the_key_line() {
        let b = doc("---\ntitle: Plan\nowner: zt\n---\n# H\n");
        let a = doc("---\ntitle: Plan v2\nowner: zt\n---\n# H\n");
        let got = node_deltas(&b, &a);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].target, Ref::FmKey("title".into()));
        assert_eq!(got[0].change, NodeChangeKind::Edited);
    }

    // C0 — one-put gated close over a realistic task card

    /// Task card fixture: `status:` + `## Verdict` — the two halves one put must cover.
    fn task_card(status: &str, verdict: &str) -> String {
        format!(
            "---\ntype: task\nstatus: {status}\nowner: e4201e72\n---\n\n\
             # Task: widget-rollout\n\n## Objective\n\nShip the widget.\n\n\
             ## Verdict\n\n{verdict}\n"
        )
    }

    fn verdict_hpath() -> Ref {
        Ref::Hpath(vec![
            crate::HpathSeg {
                h: "Task: widget-rollout".into(),
                n: None,
            },
            crate::HpathSeg {
                h: "Verdict".into(),
                n: None,
            },
        ])
    }

    /// C0 control 1 — the `status:` line alone names `fm_key`; the control that
    /// proves the probe can see an `fm_key` entry at all.
    #[test]
    fn c0_status_alone_names_fm_key() {
        let b = doc(&task_card("in-progress", "pending"));
        let a = doc(&task_card("review", "pending"));
        let got = node_deltas(&b, &a);
        println!("C0 case 1 (status alone) population = {got:#?}");
        assert_eq!(got.len(), 1, "population: {got:?}");
        assert_eq!(got[0].target, Ref::FmKey("status".into()));
        assert_eq!(got[0].change, NodeChangeKind::Edited);
    }

    /// C0 control 2 — the `## Verdict` body alone names its `hpath`; the control
    /// that proves the probe can see an `Hpath` entry at all.
    #[test]
    fn c0_verdict_section_alone_names_hpath() {
        let b = doc(&task_card("in-progress", "pending"));
        let a = doc(&task_card("in-progress", "passed - gates green"));
        let got = node_deltas(&b, &a);
        println!("C0 case 2 (section alone) population = {got:#?}");
        assert_eq!(got.len(), 1, "population: {got:?}");
        assert_eq!(got[0].target, verdict_hpath());
        assert_eq!(got[0].change, NodeChangeKind::Edited);
    }

    /// C0 case 3, RATIFIED (sub-node-grain ruling, 2026-08-14 — supersedes the
    /// 2026-07-30 observed pin): a one-put gated close (`status:` +
    /// `## Verdict`) reports BOTH touched nodes. The joint edit is two
    /// line-grain regions, each attributed at its own deepest node — the
    /// dogfood r1 "zero node rows on a batch" misreport is the red this
    /// ratifies away.
    #[test]
    fn c0_gated_close_one_put_reports_both_touched_nodes() {
        let b = doc(&task_card("in-progress", "pending"));
        let a = doc(&task_card("review", "passed - gates green"));

        let got = node_deltas(&b, &a);
        println!("C0 case 3 (gated close, one put) population = {got:#?}");
        assert_eq!(got.len(), 2, "both halves are reported: {got:?}");
        let status = got
            .iter()
            .find(|d| d.target == Ref::FmKey("status".into()))
            .expect("the status flip is reported");
        assert_eq!(status.change, NodeChangeKind::Edited);
        let verdict = got
            .iter()
            .find(|d| d.target == verdict_hpath())
            .expect("the Verdict body edit is reported");
        assert_eq!(verdict.change, NodeChangeKind::Edited);

        let fd = file_delta(Some(&b), Some(&a)).expect("file changed");
        assert_eq!(fd.change, FileChangeKind::Modified);
        assert_eq!(fd.nodes.len(), 2, "{:?}", fd.nodes);
    }

    /// C0 case 3b, RATIFIED: appending `## Verdict` while flipping `status:`
    /// in one splice reports the fm key edited AND the section added — a
    /// truly-added section never vanishes behind a frontmatter touch, and an
    /// append alone names the appended section, not its parent.
    #[test]
    fn c0_gated_close_appending_verdict_reports_key_and_added_section() {
        let b = doc(
            "---\ntype: task\nstatus: in-progress\nowner: e4201e72\n---\n\n\
                     # Task: widget-rollout\n\n## Objective\n\nShip the widget.\n",
        );
        let a = doc("---\ntype: task\nstatus: review\nowner: e4201e72\n---\n\n\
                     # Task: widget-rollout\n\n## Objective\n\nShip the widget.\n\n\
                     ## Verdict\n\npassed - gates green\n");

        // The appended section names ITSELF (added), never a bare parent
        // roll-up. The separator blank line lives inside §Objective's span
        // (§1 span law), so Objective's rev moved and its edited entry is
        // byte-truth — a CAS holder on it must see the invalidation.
        let objective = Ref::Hpath(vec![
            crate::HpathSeg {
                h: "Task: widget-rollout".into(),
                n: None,
            },
            crate::HpathSeg {
                h: "Objective".into(),
                n: None,
            },
        ]);
        let alone = node_deltas(
            &doc(
                "---\ntype: task\nstatus: in-progress\nowner: e4201e72\n---\n\n\
                  # Task: widget-rollout\n\n## Objective\n\nShip the widget.\n",
            ),
            &doc(
                "---\ntype: task\nstatus: in-progress\nowner: e4201e72\n---\n\n\
                  # Task: widget-rollout\n\n## Objective\n\nShip the widget.\n\n\
                  ## Verdict\n\npassed - gates green\n",
            ),
        );
        println!("C0 case 3b control (append alone) population = {alone:#?}");
        assert_eq!(alone.len(), 2, "no parent roll-up, no removal: {alone:?}");
        assert!(
            alone
                .iter()
                .any(|d| d.target == verdict_hpath() && d.change == NodeChangeKind::Added),
            "{alone:?}"
        );
        assert!(
            alone
                .iter()
                .any(|d| d.target == objective && d.change == NodeChangeKind::Edited),
            "the sibling whose span gained the separator line reports: {alone:?}"
        );

        let got = node_deltas(&b, &a);
        println!("C0 case 3b (append verdict + flip status) population = {got:#?}");
        assert_eq!(got.len(), 3, "every touched node reported: {got:?}");
        assert!(
            got.iter()
                .any(|d| d.target == Ref::FmKey("status".into())
                    && d.change == NodeChangeKind::Edited),
            "{got:?}"
        );
        assert!(
            got.iter()
                .any(|d| d.target == verdict_hpath() && d.change == NodeChangeKind::Added),
            "{got:?}"
        );
        assert!(
            got.iter()
                .any(|d| d.target == objective && d.change == NodeChangeKind::Edited),
            "{got:?}"
        );
    }

    // sub-node-grain (dogfood r1 prober F1): the batch and duplicate shapes.

    /// Two appends to two sibling sections report EACH child edited — never a
    /// roll-up to the shared parent, never a lost child (F1 row 2).
    #[test]
    fn two_appends_report_each_touched_child() {
        let b = doc("# Pinner\n\n## Claims\n\nc1\n\n## Delta\n\nd1\n");
        let a = doc("# Pinner\n\n## Claims\n\nc1\nc2\n\n## Delta\n\nd1\nd2\n");
        let got = node_deltas(&b, &a);
        println!("two appends population = {got:#?}");
        let seg = |h: &str| crate::HpathSeg {
            h: h.into(),
            n: None,
        };
        assert_eq!(got.len(), 2, "one entry per touched child: {got:?}");
        for child in ["Claims", "Delta"] {
            let d = got
                .iter()
                .find(|d| d.target == Ref::Hpath(vec![seg("Pinner"), seg(child)]))
                .unwrap_or_else(|| panic!("§Pinner/{child} is reported: {got:?}"));
            assert_eq!(d.change, NodeChangeKind::Edited);
        }
    }

    /// The F1 false-removal repro: on a file with duplicate `### Dup`
    /// siblings, an append into `## Beta` plus a born `## Gamma` reports
    /// exactly those two changes — and NEVER a removal that did not happen.
    /// (Deployed pair served `removed §Pin Target/Dup` with the real changes
    /// absent; the file was verified intact.)
    #[test]
    fn duplicate_headings_never_fabricate_a_removal() {
        let b = doc(
            "# Pin Target\n\n## Alpha\n\na\n\n### Dup\n\nfirst\n\n### Dup\n\nsecond\n\n\
             ## Beta\n\nb\n",
        );
        let a = doc(
            "# Pin Target\n\n## Alpha\n\na\n\n### Dup\n\nfirst\n\n### Dup\n\nsecond\n\n\
             ## Beta\n\nb\nBeta touched for sub probe.\n\n## Gamma\n\nGamma born for sub probe.\n",
        );
        let got = node_deltas(&b, &a);
        println!("duplicate-heading population = {got:#?}");
        assert!(
            got.iter().all(|d| d.change != NodeChangeKind::Removed),
            "nothing was removed — a delta feed must never fabricate one: {got:?}"
        );
        let seg = |h: &str| crate::HpathSeg {
            h: h.into(),
            n: None,
        };
        assert!(
            got.iter().any(
                |d| d.target == Ref::Hpath(vec![seg("Pin Target"), seg("Beta")])
                    && d.change == NodeChangeKind::Edited
            ),
            "the Beta edit is reported: {got:?}"
        );
        assert!(
            got.iter().any(
                |d| d.target == Ref::Hpath(vec![seg("Pin Target"), seg("Gamma")])
                    && d.change == NodeChangeKind::Added
            ),
            "the born Gamma is reported: {got:?}"
        );
    }

    /// A REAL removal among duplicate siblings stays truthful: the removed
    /// occurrence is named with its §2.1 occurrence disambiguator, and the
    /// surviving twin is not misreported.
    #[test]
    fn a_real_removal_among_duplicates_names_the_occurrence() {
        let b = doc("# R\n\n## Dup\n\nfirst\n\n## Dup\n\nsecond\n\n## Tail\n\nt\n");
        let a = doc("# R\n\n## Dup\n\nfirst\n\n## Tail\n\nt\n");
        let got = node_deltas(&b, &a);
        println!("real removal among duplicates population = {got:#?}");
        let removed: Vec<_> = got
            .iter()
            .filter(|d| d.change == NodeChangeKind::Removed)
            .collect();
        assert_eq!(removed.len(), 1, "exactly one removal: {got:?}");
        let Ref::Hpath(segs) = &removed[0].target else {
            panic!("a section identity: {:?}", removed[0].target);
        };
        assert_eq!(segs.last().unwrap().h, "Dup");
        assert_eq!(
            segs.last().unwrap().n,
            Some(2),
            "the removed occurrence is disambiguated: {:?}",
            removed[0].target
        );
    }

    /// An edit INSIDE one of two duplicate siblings names that occurrence —
    /// the identity a consumer can resolve back through the mint plane.
    #[test]
    fn an_edit_in_a_duplicate_sibling_carries_its_occurrence() {
        let b = doc("# R\n\n## Dup\n\nfirst\n\n## Dup\n\nsecond\n");
        let a = doc("# R\n\n## Dup\n\nfirst\n\n## Dup\n\nsecond touched\n");
        let got = node_deltas(&b, &a);
        println!("edited duplicate population = {got:#?}");
        assert_eq!(got.len(), 1, "{got:?}");
        let Ref::Hpath(segs) = &got[0].target else {
            panic!("a section identity: {:?}", got[0].target);
        };
        assert_eq!(segs.last().unwrap().h, "Dup");
        assert_eq!(segs.last().unwrap().n, Some(2), "{:?}", got[0].target);
        assert_eq!(got[0].change, NodeChangeKind::Edited);
        assert!(
            resolve(&a, &got[0].target).is_ok(),
            "the served identity resolves at the mint plane"
        );
    }

    /// A multi-key frontmatter edit in one region reports EACH changed key —
    /// per-key grain survives adjacency.
    #[test]
    fn adjacent_frontmatter_keys_each_get_an_entry() {
        let b = doc("---\ntitle: Plan\nowner: zt\n---\n# H\n");
        let a = doc("---\ntitle: Plan v2\nowner: e4\n---\n# H\n");
        let got = node_deltas(&b, &a);
        println!("adjacent fm keys population = {got:#?}");
        assert_eq!(got.len(), 2, "{got:?}");
        for key in ["title", "owner"] {
            assert!(
                got.iter()
                    .any(|d| d.target == Ref::FmKey(key.into())
                        && d.change == NodeChangeKind::Edited),
                "fm key {key} is reported: {got:?}"
            );
        }
    }

    /// File-level tenses (§7.1): created carries after-rev only, deleted
    /// carries before-rev only, unchanged is None.
    #[test]
    fn file_delta_tense_absences() {
        let d = doc("# H\n");
        let created = file_delta(None, Some(&d)).unwrap();
        assert_eq!(created.change, FileChangeKind::Created);
        assert_eq!(created.file_rev_before, None);
        assert!(created.file_rev_after.is_some());
        let deleted = file_delta(Some(&d), None).unwrap();
        assert_eq!(deleted.change, FileChangeKind::Deleted);
        assert_eq!(deleted.file_rev_after, None);
        assert_eq!(file_delta(Some(&d), Some(&d)), None);
        assert_eq!(file_delta(None, None), None);
    }
}
