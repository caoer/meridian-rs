//! M2-BUILD oracle gate: `model::build` mints the §0.3 fixture's `node_rev`s and
//! `file_rev`s byte-exact against the independent `compute.py` oracle, and the
//! frontmatter node span obeys the frozen §18 row 3 fence-to-fence law.
//!
//! Expected values are the `compute.py` output verbatim (S0 `toc`, S2 `toc_s2`,
//! `file_revs`). `file_rev` is recomputed here independently with blake3 (a test-only
//! dev-dependency) so the assertion does not lean on `model`'s own hashing, and the
//! whole-file-node identity (`node_rev(root) == file_rev`) is proven, not assumed.

use model::{Node, NodeKind};
use std::fs;

/// Independent `file_rev` — blake3-256 of the whole file bytes, first 16 hex.
fn file_rev(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().as_str()[..16].to_string()
}

fn build(rel: &str) -> (String, Node) {
    let raw = fs::read_to_string(testsuite::wsfix_dir().join(rel))
        .unwrap_or_else(|e| panic!("read fixture {rel}: {e}"));
    let doc = model::build(raw.clone(), syntax::parse(&raw));
    (raw, doc.root)
}

/// Depth-first search for the first node satisfying `pred`.
fn find<'a>(node: &'a Node, pred: &dyn Fn(&Node) -> bool) -> Option<&'a Node> {
    if pred(node) {
        return Some(node);
    }
    node.children.iter().find_map(|c| find(c, pred))
}

fn frontmatter(root: &Node) -> &Node {
    find(root, &|n| matches!(n.kind, NodeKind::Frontmatter { .. })).expect("frontmatter node")
}

fn section<'a>(root: &'a Node, heading: &str) -> &'a Node {
    find(
        root,
        &|n| matches!(&n.kind, NodeKind::Section { heading_text, .. } if heading_text == heading),
    )
    .unwrap_or_else(|| panic!("section {heading:?}"))
}

/// Assert a node's span and `node_rev` against the oracle in one shot.
fn assert_node(node: &Node, span: std::ops::Range<usize>, rev: &str) {
    assert_eq!(node.span, span, "span for {:?}", node.kind);
    assert_eq!(node.node_rev.0, rev, "node_rev for {:?}", node.kind);
}

#[test]
fn wsfix_s0_plan_node_and_file_revs_match_oracle() {
    let (raw, root) = build("s0/notes/plan.md");
    assert_eq!(raw.len(), 136);

    // file_rev, computed independently of the tree, equals the oracle AND the
    // whole-file root node's node_rev (the identity, tested not assumed).
    let fr = file_rev(raw.as_bytes());
    assert_eq!(fr, "e3c4acaceb75b907", "oracle file_rev(plan_v0)");
    assert_node(&root, 0..136, &fr);

    // frontmatter: fence-to-fence, terminator-inclusive (§18 row 3 / §4.1 toc).
    let fm = frontmatter(&root);
    assert_node(fm, 0..20, "26796ebec5d0bf1a");
    assert_eq!(&raw[fm.span.clone()], "---\ntitle: Plan\n---\n");
    let NodeKind::Frontmatter { map } = &fm.kind else {
        unreachable!()
    };
    assert_eq!(
        map.0
            .iter()
            .find_map(|(k, v)| (k == "title").then_some(v.as_str())),
        Some("Plan")
    );

    // sections (toc_s0): Goals(L1) governs Q3/Q4(L2).
    assert_node(section(&root, "Goals"), 20..136, "a6665baff294bd04");
    assert_node(section(&root, "Q3"), 49..72, "33d5b0e1b27cb48b");
    assert_node(section(&root, "Q4"), 72..136, "4b8bc385a58da0e0");
}

#[test]
fn wsfix_s0_receipts_whole_file_heading_rev_equals_file_rev() {
    let (raw, root) = build("s0/receipts/2026-07-18.md");
    assert_eq!(raw.len(), 26);

    let fr = file_rev(raw.as_bytes());
    assert_eq!(fr, "920a40c4ee23d37c", "oracle file_rev(receipts_v0)");
    assert_node(&root, 0..26, &fr);

    // the lone top-level heading spans the whole file, so its node_rev == file_rev
    // (§4.1). No frontmatter, no anchors at S0.
    let heading = section(&root, "Receipts — 2026-07-18");
    assert_node(heading, 0..26, &fr);
    assert!(
        find(&root, &|n| matches!(n.kind, NodeKind::Frontmatter { .. })).is_none(),
        "receipts has no frontmatter"
    );
}

#[test]
fn wsfix_s2_plan_node_and_file_revs_match_oracle() {
    // S2 shifts section spans (Q3 grows, Q4 appended) — proves revs track the
    // live bytes, not a first-state snapshot. Values are compute.py `toc_s2`.
    let (raw, root) = build("s2/notes/plan.md");
    assert_eq!(raw.len(), 150);

    let fr = file_rev(raw.as_bytes());
    assert_eq!(fr, "5f27a2814b517680", "oracle file_rev(plan_v2)");
    assert_node(&root, 0..150, &fr);

    assert_node(frontmatter(&root), 0..20, "26796ebec5d0bf1a");
    assert_node(section(&root, "Goals"), 20..150, "5a8faa717fbcdb04");
    assert_node(section(&root, "Q3"), 49..75, "41f643f034e5681f");
    assert_node(section(&root, "Q4"), 75..150, "f43203a1f0b4c9a3");
}
