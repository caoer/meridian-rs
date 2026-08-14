//! The `toc` scope's ruled semantics (F-R3, ZT 2026-08-13; dogfood-fresh
//! findings § ZT rulings item 5): the read op's subtree scope is ONE tagged
//! selector with every position meaning one thing — a heading path or a dewey
//! ordinal resolves to one section and scopes the shape table to that
//! subtree; the anchor arm refuses (a block has no subtree); a bare duplicate
//! refuses with each candidate's machine address (never-silently-picks, the
//! sections plane's own §2.1 law); `toc` and `sections` together refuse with
//! "pass one".

use wire::{ErrorCode, Path as WPath, ReadSel, ResponseBody};
use wire_serve::read::{NO_DECORATIONS, ReadParams, composed_read};

/// Two same-titled `## Dup` siblings with distinct subtrees and one anchor
/// each, plus a distinct sibling subtree that must never leak into a scope.
const DOC: &str = "# Top\n\nintro\n\n## Dup\n\nfirst body ^d1\n\n### FirstChild\n\nc1\n\n## Dup\n\nsecond body ^d2\n\n### SecondChild\n\nc2\n\n## Beta\n\nbeta body ^b1\n";

fn doc() -> model::Document {
    model::build(DOC.to_string(), syntax::parse(DOC))
}

fn read_toc(sel: ReadSel) -> Result<ResponseBody, Box<wire::ErrorBody>> {
    composed_read(
        &doc(),
        &WPath("card.md".into()),
        &wire::Root("r".into()),
        &ReadParams {
            toc: Some(sel),
            display_path: Some("card.md".into()),
            ..ReadParams::default()
        },
        None,
        &NO_DECORATIONS,
    )
}

fn row_titles(body: &ResponseBody) -> Vec<String> {
    let ResponseBody::Read { toc, .. } = body else {
        panic!("composed read answers a Read body");
    };
    toc.as_ref()
        .expect("toc mode carries rows")
        .iter()
        .map(|r| r.title.clone())
        .collect()
}

fn anchor_ids(body: &ResponseBody) -> Vec<String> {
    let ResponseBody::Read { anchors, .. } = body else {
        panic!("composed read answers a Read body");
    };
    anchors.iter().map(|a| a.anchor.clone()).collect()
}

/// A dewey toc scope serves that ROW's subtree map — the ordinal names a
/// heading row, a heading has a subtree, and the scope is the map of it.
/// (Under the retired grammar a dewey fragment could only mean content; the
/// ruled grammar gives the position one meaning: which subtree map.)
#[test]
fn a_dewey_toc_scope_serves_that_subtree_map() {
    let body = read_toc(ReadSel::parse("1.2")).expect("row 1.2 is the second Dup");
    assert_eq!(
        row_titles(&body),
        vec!["Dup", "SecondChild"],
        "the ordinal's own row plus its descendants, nothing else"
    );
    assert_eq!(
        anchor_ids(&body),
        vec!["d2"],
        "the anchor plane is bounded by the same subtree"
    );
    let ResponseBody::Read { sections, .. } = &body else {
        unreachable!()
    };
    assert!(
        sections.is_none(),
        "a toc scope serves shape, never content"
    );
}

/// The anchor arm refuses: a block has no subtree, so no scope exists for it
/// to name. The refusal teaches the lane that serves the spelling.
#[test]
fn an_anchor_toc_scope_refuses_no_subtree() {
    let err = read_toc(ReadSel::parse("^d1")).expect_err("a block cannot scope a toc");
    assert_eq!(err.code, ErrorCode::BadRequest);
    let m = err.message.as_deref().expect("a sentence, not a bare code");
    assert!(
        m.contains("a block has no subtree"),
        "the reason opens the teaching: {m}"
    );
    assert!(
        m.contains("Nothing was read") && m.contains("no rev was minted"),
        "discloses the partial state: {m}"
    );
    assert!(
        m.contains("Fix:") && m.contains("sections"),
        "the fix names the lane that serves a block's content: {m}"
    );
}

/// A bare duplicate refuses with each candidate's machine address — the
/// never-silently-picks law the sections plane holds; silently merging the
/// siblings' subtrees (the retired prefix scope's behavior) is the defect
/// this gate pins closed.
#[test]
fn a_bare_duplicate_toc_scope_refuses_with_candidates() {
    let err = read_toc(ReadSel::parse("Top/Dup")).expect_err("two rows match");
    assert_eq!(err.code, ErrorCode::AmbiguousRef);
    let m = err.message.as_deref().expect("a sentence, not a bare code");
    assert!(
        m.contains("is ambiguous (2 matches:"),
        "counts and names the candidates: {m}"
    );
    assert!(
        m.contains(r#""n":1"#) && m.contains(r#""n":2"#),
        "each candidate's machine address carries its occurrence pin: {m}"
    );
    assert!(
        m.contains(model::selector::AMBIGUITY_FIX),
        "the published ambiguity remedy, byte-shared with the write door: {m}"
    );
}

/// An occurrence-pinned duplicate resolves: `n` names one sibling, and the
/// scope is that sibling's subtree alone.
#[test]
fn a_pinned_duplicate_toc_scope_resolves_one_sibling() {
    let body = read_toc(ReadSel::Hpath {
        hpath: vec![
            wire::HpathSeg {
                h: "Top".into(),
                n: None,
            },
            wire::HpathSeg {
                h: "Dup".into(),
                n: Some(2),
            },
        ],
    })
    .expect("the pin resolves the second sibling");
    assert_eq!(row_titles(&body), vec!["Dup", "SecondChild"]);
    assert_eq!(anchor_ids(&body), vec!["d2"]);
}

/// A heading toc scope is the subtree map — parity with the scope the
/// retired `frag` served on the unique-path case.
#[test]
fn a_heading_toc_scope_serves_the_subtree_map() {
    let body = read_toc(ReadSel::parse("Top/Beta")).expect("Beta is unique");
    assert_eq!(row_titles(&body), vec!["Beta"]);
    assert_eq!(anchor_ids(&body), vec!["b1"]);
}

/// `toc` and `sections` together refuse before either plane is consulted —
/// "pass one" (the same philosophy the engine states for selector/files).
#[test]
fn toc_beside_sections_refuses_pass_one() {
    let err = composed_read(
        &doc(),
        &WPath("card.md".into()),
        &wire::Root("r".into()),
        &ReadParams {
            toc: Some(ReadSel::parse("Top/Beta")),
            sections: Some(vec![ReadSel::parse("Top/Beta")]),
            display_path: Some("card.md".into()),
            ..ReadParams::default()
        },
        None,
        &NO_DECORATIONS,
    )
    .expect_err("two planes, one call");
    assert_eq!(err.code, ErrorCode::BadRequest);
    let m = err.message.as_deref().expect("a sentence, not a bare code");
    assert!(m.contains("pass one"), "the ruled verdict: {m}");
}
