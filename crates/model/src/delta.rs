//! Delta change-fact computation (contract §7, D3-DELTA — wire-lane-owned).
//!
//! Non-serde, by the model charter: this module computes the FACTS a Delta
//! carries; `wire-map` projects them onto `wire::Delta` and the sidecar
//! assembles the envelope (seq/roots/actor/now — the engine records nothing
//! it wasn't given, §9).
//!
//! **The deepest-changed-node law (D-C7, §7.1):** node entries name the
//! deepest mint-addressable node containing each changed byte range —
//! sections, anchor-bearing blocks (host-block grain), and frontmatter key
//! lines. Ancestor section revs change implicitly (rev = span hash) and are
//! re-readable via `toc`, never duplicated into the delta.
//!
//! **Node-grain at birth (decision 012, §7.4):** the grain is exactly
//! [`NodeDelta`] — identity in the §2.1 grammar, rev transition, span after.
//! No key-grain sub-entries exist here; that amendment slot is contract
//! prose, future-only.
//!
//! Known bound (documented, not hidden): changed ranges are derived by
//! common-prefix/common-suffix over the raw bytes — one contiguous range per
//! file revision. The rung-4 write path knows its exact edit spans and may
//! refine this; `renamed` detection needs cross-file correlation only the
//! watcher/apply layers have, so this module classifies `created`/
//! `modified`/`deleted` and leaves `renamed` to callers that know.

use crate::{ByteSpan, Document, Node, NodeKind, NodeRev, Ref, resolve};

/// File-level change class this module can derive from two states of one
/// path. `renamed` is the caller's (cross-file knowledge).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileChangeKind {
    Created,
    Modified,
    Deleted,
}

/// v2 §7.1 node change class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeChangeKind {
    Added,
    Edited,
    Removed,
}

/// One node-grain change fact: identity in THE §2.1 grammar ([`Ref`]), rev
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
/// changed (identical bytes) or both states are absent. Created/deleted
/// files carry no node entries — the file-level fact is the change; the node
/// inventory is re-readable via `toc` (never duplicated, §7.1 posture).
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

/// The node entries for one modified file (D-C7). Two passes, one per tense:
///
/// - **After-side** (`added`/`edited`): the deepest addressable node
///   containing the after-range. A range ending in a line terminator is
///   matched with that final terminator trimmed — block-leaf spans exclude
///   their terminator by the §1 span law, so an appended anchor block (the
///   worked E3 receipts pattern) matches the BLOCK, not its host section.
/// - **Before-side** (`removed`): the shallowest before-nodes intersecting
///   the before-range whose identity no longer resolves — a removed
///   subtree's descendants stay implicit (never duplicated), mirroring the
///   ancestor-implicitness law.
///
/// A pure append has an empty before-range (no removed scan); a pure
/// deletion has an empty after-range (no added/edited entry).
#[must_use]
pub fn node_deltas(before: &Document, after: &Document) -> Vec<NodeDelta> {
    let Some((b_range, a_range)) = changed_ranges(&before.raw, &after.raw) else {
        return Vec::new();
    };
    let mut out = Vec::new();

    if !a_range.is_empty() {
        let probe = trim_final_terminator(&after.raw, a_range);
        if let Some((target, node)) = deepest_addressable(after, &probe) {
            match resolve(before, &target) {
                Ok(prior) => out.push(NodeDelta {
                    target,
                    change: NodeChangeKind::Edited,
                    node_rev_before: Some(prior.node_rev),
                    node_rev_after: Some(node.node_rev.clone()),
                    span_after: Some(node.span.clone()),
                }),
                Err(_) => out.push(NodeDelta {
                    target,
                    change: NodeChangeKind::Added,
                    node_rev_before: None,
                    node_rev_after: Some(node.node_rev.clone()),
                    span_after: Some(node.span.clone()),
                }),
            }
        }
    }

    if !b_range.is_empty() {
        let probe = trim_final_terminator(&before.raw, b_range);
        collect_removed(before, after, &before.root, &probe, &mut out);
    }

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
/// identity fails to resolve in `after` is a `removed` entry; its subtree
/// stays implicit (no recursion below a removed node). A resolving ancestor
/// recurses — a removed child may hide under a surviving parent.
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
    if let Some(target) = identity_of(before, node, range)
        && resolve(after, &target).is_err()
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

/// Common-prefix/common-suffix changed ranges; `None` when byte-identical.
/// Suffix is clamped so the ranges never overlap the prefix.
fn changed_ranges(before: &str, after: &str) -> Option<(ByteSpan, ByteSpan)> {
    let (b, a) = (before.as_bytes(), after.as_bytes());
    if b == a {
        return None;
    }
    let prefix = b.iter().zip(a).take_while(|(x, y)| x == y).count();
    let max_suffix = b.len().min(a.len()) - prefix;
    let suffix = b
        .iter()
        .rev()
        .zip(a.iter().rev())
        .take_while(|(x, y)| x == y)
        .count()
        .min(max_suffix);
    Some((prefix..b.len() - suffix, prefix..a.len() - suffix))
}

/// The deepest mint-addressable node containing `range`, with its §2.1
/// identity: anchor-bearing blocks and sections (smallest containing span
/// wins — an anchor block inside its section is deeper); a range inside the
/// frontmatter block refines to the changed key line (`fm_key`).
fn deepest_addressable<'d>(doc: &'d Document, range: &ByteSpan) -> Option<(Ref, &'d Node)> {
    let mut best: Option<(Ref, &Node)> = None;
    walk_candidates(doc, &doc.root, range, &mut best);
    best
}

fn contains(span: &ByteSpan, range: &ByteSpan) -> bool {
    span.start <= range.start && range.end <= span.end
}

fn walk_candidates<'d>(
    doc: &'d Document,
    node: &'d Node,
    range: &ByteSpan,
    best: &mut Option<(Ref, &'d Node)>,
) {
    if contains(&node.span, range)
        && let Some(target) = identity_of(doc, node, range)
    {
        let better = best
            .as_ref()
            .is_none_or(|(_, b)| node.span.len() <= b.span.len());
        // resolve the identity to ITS node — for fm_key the entry node is
        // the key line, not the frontmatter block
        if better && let Ok(t) = resolve(doc, &target) {
            let entry = find_by_span(&doc.root, &t.span).unwrap_or(node);
            *best = Some((target, entry));
        }
    }
    for child in &node.children {
        walk_candidates(doc, child, range, best);
    }
}

/// The §2.1 identity of an addressable candidate node, if it is one.
fn identity_of(doc: &Document, node: &Node, range: &ByteSpan) -> Option<Ref> {
    match &node.kind {
        NodeKind::Section { .. } => Some(Ref::Hpath(
            node.hpath
                .clone()?
                .into_iter()
                .map(|h| crate::HpathSeg { h, n: None })
                .collect(),
        )),
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

/// The tree node whose span equals `span` (the resolved entry node).
fn find_by_span<'d>(node: &'d Node, span: &ByteSpan) -> Option<&'d Node> {
    if node.span == *span {
        return Some(node);
    }
    node.children.iter().find_map(|c| find_by_span(c, span))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build;

    fn doc(raw: &str) -> Document {
        build(raw.to_string(), syntax::parse(raw))
    }

    /// D-C7: an edit inside a nested section names the DEEPEST section, not
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

    /// An appended anchor-bearing block echoes as the ANCHOR added (the
    /// worked E3 receipts pattern) — the host section's rev change stays
    /// implicit.
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

    /// A frontmatter edit refines to the changed key LINE (`fm_key` — the
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
