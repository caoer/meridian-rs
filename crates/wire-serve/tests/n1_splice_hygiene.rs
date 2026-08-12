//! N-1 splice hygiene (wire-contract § A.3, ZT-ratified 2026-08-12): after any
//! plan-door splice, every boundary the splice touches carries exactly ONE
//! blank line — block and section boundaries alike — and a list-item payload
//! appended to a trailing list joins it flush. Fixtures mirror the mrd-mcp
//! probe's measured defects (session 12-04-f2-mrd-integration,
//! results/mrd-mcp-probe.md N-1).
//!
//! Flow per case: seed file → plan splice through the production choke-point →
//! assert final bytes.

use std::path::PathBuf;

use wire::{HpathSeg, Path as WPath, PlanEdit};
use wire_serve::write::{SpliceArgs, splice};

fn ws(files: &[(&str, &str)]) -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    for (rel, body) in files {
        let p = dir.path().join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(p, body).expect("seed");
    }
    let root = fs::WorkspaceRoot(PathBuf::from(dir.path()));
    (dir, root)
}

fn plan_args(path: &str, plan_edits: Vec<PlanEdit>) -> SpliceArgs {
    SpliceArgs {
        id: None,
        origin: wire_serve::guard::Origin::InProcess,
        path: WPath(path.into()),
        actor: Some("agent:n1".into()),
        now: Some("2026-08-12T12:00:00Z".into()),
        receipt: None,
        if_root: None,
        dry: false,
        force: false,
        edits: Vec::new(),
        plan_edits,
        pin: None,
    }
}

fn seg(h: &str) -> HpathSeg {
    HpathSeg {
        h: h.into(),
        n: None,
    }
}

/// Run one plan splice and return the file's final bytes.
fn apply(seed: &str, edits: Vec<PlanEdit>) -> String {
    let (dir, root) = ws(&[("page.md", seed)]);
    splice(&root, None, &plan_args("page.md", edits), &[], None).expect("plan splice commits");
    std::fs::read_to_string(dir.path().join("page.md")).expect("read back")
}

fn append(hpath: &[&str], body: &str) -> PlanEdit {
    PlanEdit::Append {
        hpath: hpath.iter().map(|h| seg(h)).collect(),
        body: body.into(),
        rev: None,
    }
}

/// Measured defect 1 (probe N-1, put lane): an append of a list item to a
/// section ending in a list minted a paragraph break — the section's trailing
/// separator ended up INSIDE the list, splitting it. The item must join the
/// list flush, and the boundary before the next heading must stay exactly one
/// blank line.
#[test]
fn append_list_item_joins_a_trailing_list_flush() {
    let out = apply(
        "# Page\n\n## Write tests\n\n- birth\n- batch\n\n## Anchors\n\nx\n",
        vec![append(&["Page", "Write tests"], "- script flip")],
    );
    assert_eq!(
        out, "# Page\n\n## Write tests\n\n- birth\n- batch\n- script flip\n\n## Anchors\n\nx\n",
        "list item joins the list flush; one blank line before the next heading"
    );
}

/// Measured defect 2 (probe N-1, script lane): an append at a section boundary
/// landed flush against the next heading. Exactly one blank line must separate
/// the appended block from the following heading — and one blank line from the
/// prior block.
#[test]
fn append_at_a_section_boundary_keeps_one_blank_line_each_side() {
    let out = apply(
        "# Page\n\n## Write tests\n\nprose line\n\n## Anchors\n\nx\n",
        vec![append(&["Page", "Write tests"], "- marker line")],
    );
    assert_eq!(
        out, "# Page\n\n## Write tests\n\nprose line\n\n- marker line\n\n## Anchors\n\nx\n",
        "new block: one blank line after the prior block, one before the next heading"
    );
}

/// EOF append to a trailing list stays a flush join (this worked before N-1;
/// it must keep working).
#[test]
fn append_list_item_at_eof_stays_flush() {
    let out = apply(
        "# Memo\n\n- entry a\n",
        vec![append(&["Memo"], "- entry b")],
    );
    assert_eq!(out, "# Memo\n\n- entry a\n- entry b\n");
}

/// EOF append of a new block gets its blank-line boundary (bare tail today
/// lazy-continues the paragraph).
#[test]
fn append_new_block_at_eof_gets_a_blank_line() {
    let out = apply("# Memo\n\nprose\n", vec![append(&["Memo"], "more prose")]);
    assert_eq!(out, "# Memo\n\nprose\n\nmore prose\n");
}

/// Append to a section whose content is empty: one blank line under the
/// heading, one before the next heading.
#[test]
fn append_into_an_empty_section_is_canonical_both_sides() {
    let out = apply(
        "# Page\n\n## Empty\n\n## Next\n\nx\n",
        vec![append(&["Page", "Empty"], "body")],
    );
    assert_eq!(out, "# Page\n\n## Empty\n\nbody\n\n## Next\n\nx\n");
}

/// A payload's own leading/trailing blank lines collapse into the canonical
/// separators — boundaries are the engine's, interior bytes the caller's.
#[test]
fn payload_edge_blank_lines_collapse_into_canonical_separators() {
    let out = apply(
        "# Page\n\n## S\n\nprose\n\n## T\n\nx\n",
        vec![append(&["Page", "S"], "\n\nnew block\n\n\n")],
    );
    assert_eq!(out, "# Page\n\n## S\n\nprose\n\nnew block\n\n## T\n\nx\n");
}

/// Measured defect 3 (probe fixture: `## Level tests` flush against its own
/// body): `replace_section` consumed the blank line under its own heading
/// and the separator before the next heading. The composed content must
/// carry both boundaries.
#[test]
fn replace_section_keeps_the_blank_under_its_heading_and_before_the_next() {
    let seed = "# Page\n\n## Level tests\n\nold body\n\n## Next section\n\nx\n";
    let (dir, root) = ws(&[("page.md", seed)]);
    let doc = fs::load(&root, std::path::Path::new("page.md")).expect("load");
    let rev = model::resolve(
        &doc,
        &model::Ref::Hpath(vec![
            model::HpathSeg {
                h: "Page".into(),
                n: None,
            },
            model::HpathSeg {
                h: "Level tests".into(),
                n: None,
            },
        ]),
    )
    .expect("resolves")
    .node_rev
    .0;
    splice(
        &root,
        None,
        &plan_args(
            "page.md",
            vec![PlanEdit::ReplaceSection {
                hpath: vec![seg("Page"), seg("Level tests")],
                body: "new body".into(),
                rev: Some(rev),
            }],
        ),
        &[],
        None,
    )
    .expect("replace commits");
    let out = std::fs::read_to_string(dir.path().join("page.md")).expect("read");
    assert_eq!(
        out, "# Page\n\n## Level tests\n\nnew body\n\n## Next section\n\nx\n",
        "one blank line under the heading, one before the next heading"
    );
}

/// `replace_section` with an empty body mid-file: the emptied section keeps one
/// blank line between its heading and the next (words:0 is legal — spec case
/// 10).
#[test]
fn replace_section_empty_body_keeps_one_boundary_blank() {
    let seed = "# Page\n\n## Gone\n\nold\n\n## Next\n\nx\n";
    let (dir, root) = ws(&[("page.md", seed)]);
    let doc = fs::load(&root, std::path::Path::new("page.md")).expect("load");
    let rev = model::resolve(
        &doc,
        &model::Ref::Hpath(vec![
            model::HpathSeg {
                h: "Page".into(),
                n: None,
            },
            model::HpathSeg {
                h: "Gone".into(),
                n: None,
            },
        ]),
    )
    .expect("resolves")
    .node_rev
    .0;
    splice(
        &root,
        None,
        &plan_args(
            "page.md",
            vec![PlanEdit::ReplaceSection {
                hpath: vec![seg("Page"), seg("Gone")],
                body: String::new(),
                rev: Some(rev),
            }],
        ),
        &[],
        None,
    )
    .expect("replace commits");
    let out = std::fs::read_to_string(dir.path().join("page.md")).expect("read");
    assert_eq!(out, "# Page\n\n## Gone\n\n## Next\n\nx\n");
}

/// create after a section whose tail already carries the separator: exactly
/// one blank line before the born heading (today the unconditional leading
/// `\n` mints a second one) and one before the following heading.
#[test]
fn create_lands_with_one_blank_line_each_side() {
    let out = apply(
        "# Page\n\ncontent\n\n# Tail\n\nx\n",
        vec![PlanEdit::Create {
            parent_hpath: vec![seg("Page")],
            title: "Born".into(),
            body: "hello".into(),
            rev: None,
        }],
    );
    assert_eq!(
        out, "# Page\n\ncontent\n\n## Born\n\nhello\n\n# Tail\n\nx\n",
        "one blank line before the born heading, one after its body"
    );
}

/// create with an empty body: heading only, canonical boundaries, no stray
/// blank line minted for the absent body.
#[test]
fn create_with_empty_body_mints_no_stray_blank() {
    let out = apply(
        "# Page\n\ncontent\n",
        vec![PlanEdit::Create {
            parent_hpath: vec![seg("Page")],
            title: "Born".into(),
            body: String::new(),
            rev: None,
        }],
    );
    assert_eq!(out, "# Page\n\ncontent\n\n## Born\n");
}

/// Ordered-list payloads join a trailing ordered list flush too.
#[test]
fn ordered_list_item_joins_flush() {
    let out = apply(
        "# Page\n\n## Steps\n\n1. first\n2. second\n\n## Next\n\nx\n",
        vec![append(&["Page", "Steps"], "3. third")],
    );
    assert_eq!(
        out,
        "# Page\n\n## Steps\n\n1. first\n2. second\n3. third\n\n## Next\n\nx\n"
    );
}

/// A non-list block appended after a trailing list is a block boundary — one
/// blank line, never a flush lazy-continuation.
#[test]
fn paragraph_after_a_trailing_list_gets_its_blank_line() {
    let out = apply(
        "# Page\n\n## S\n\n- item\n\n## T\n\nx\n",
        vec![append(&["Page", "S"], "closing prose")],
    );
    assert_eq!(
        out,
        "# Page\n\n## S\n\n- item\n\nclosing prose\n\n## T\n\nx\n"
    );
}

/// A list item appended after a CLOSED code fence does not flush-join — the
/// fence's last line is not a list, so the boundary is a block boundary.
#[test]
fn list_item_after_a_code_fence_is_a_block_boundary() {
    let out = apply(
        "# Page\n\n## S\n\n```\n- looks listy\n```\n\n## T\n\nx\n",
        vec![append(&["Page", "S"], "- real item")],
    );
    assert_eq!(
        out, "# Page\n\n## S\n\n```\n- looks listy\n```\n\n- real item\n\n## T\n\nx\n",
        "fence interior never drives the join decision (parsed tree, not line regex)"
    );
}

/// Task-list items are list items for the join rule.
#[test]
fn task_item_joins_a_task_list_flush() {
    let out = apply(
        "# Page\n\n## Tasks\n\n- [ ] one\n\n## Next\n\nx\n",
        vec![append(&["Page", "Tasks"], "- [ ] two")],
    );
    assert_eq!(
        out,
        "# Page\n\n## Tasks\n\n- [ ] one\n- [ ] two\n\n## Next\n\nx\n"
    );
}

/// Appending to a section with child sections lands at the subtree end with
/// canonical boundaries (the child's tail list joins flush — same locus as
/// before, now hygienic).
#[test]
fn append_to_a_parent_lands_at_subtree_end_canonically() {
    let out = apply(
        "# Page\n\n## Parent\n\nintro\n\n### Child\n\n- a\n\n## After\n\nx\n",
        vec![append(&["Page", "Parent"], "- b")],
    );
    assert_eq!(
        out,
        "# Page\n\n## Parent\n\nintro\n\n### Child\n\n- a\n- b\n\n## After\n\nx\n"
    );
}
