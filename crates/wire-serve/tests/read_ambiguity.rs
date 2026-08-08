//! Composed-`read` selector resolution under duplicate headings (wire-contract
//! A.3, dogfood F4/F6): a bare-duplicate selector refuses `ambiguous_ref`
//! naming each candidate's machine address; an `n`-pinned segment selector
//! resolves the one occurrence it names; the all-fail refusal names EVERY
//! failed selector; the partial-read notice names failures the same way.

use wire::{ErrorCode, HpathSeg, Path as WPath, ReadSel, ResponseBody};
use wire_serve::read::{NO_DECORATIONS, ReadParams, composed_read};

/// Two `Dup` siblings under one parent — the dogfood F4 shape.
const DOC: &str = "# Top\n\n## Dup\n\nfirst copy\n\n## Dup\n\nsecond copy\n\n## Solo\n\nalone\n";

/// Two list items carrying ONE block id — the dogfood-p1-read-ambiguous-ref
/// shape (wire-contract A.3, door symmetry over duplicate block ids).
const DUP_ANCHOR_DOC: &str =
    "# Tasks\n\n- first ^same-id\n\n- second ^same-id\n\n# Notes\n\nalone\n";

fn doc_of(body: &str) -> model::Document {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("dup.md"), body).expect("write");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    fs::load(&root, std::path::Path::new("dup.md")).expect("load")
}

fn read_of(body: &str, sections: Vec<ReadSel>) -> Result<ResponseBody, Box<wire::ErrorBody>> {
    composed_read(
        &doc_of(body),
        &WPath("dup.md".into()),
        &wire::Root("r".into()),
        &ReadParams {
            sections: Some(sections),
            display_path: Some("dup.md".into()),
            ..ReadParams::default()
        },
        None,
        &NO_DECORATIONS,
    )
}

fn read(sections: Vec<ReadSel>) -> Result<ResponseBody, Box<wire::ErrorBody>> {
    read_of(DOC, sections)
}

fn seg(h: &str, n: Option<u32>) -> HpathSeg {
    HpathSeg { h: h.into(), n }
}

/// F4: the bare-duplicate string selector answers the honest ambiguity —
/// `ambiguous_ref`, 2 matches, both candidates named by their machine address
/// — never `no section addressed`.
#[test]
fn bare_duplicate_selector_refuses_ambiguous_naming_candidates() {
    let err = *read(vec![ReadSel::parse("Top/Dup")]).expect_err("ambiguity refuses");
    assert_eq!(err.code, ErrorCode::AmbiguousRef);
    let msg = err.message.as_deref().expect("message");
    assert!(
        !msg.contains("no section addressed"),
        "the section exists twice; 'no section addressed' is the dishonest answer: {msg}"
    );
    assert!(msg.contains("\"Top/Dup\" is ambiguous (2 matches"), "{msg}");
    assert!(
        msg.contains(r#"[{"h":"Top"},{"h":"Dup","n":1}]"#)
            && msg.contains(r#"[{"h":"Top"},{"h":"Dup","n":2}]"#),
        "both candidates named by machine address: {msg}"
    );
}

/// F4: the machine address (segment array with `n`) resolves the exact
/// occurrence — the toc row's published address reads back its own row.
#[test]
fn n_pinned_segment_selector_resolves_the_named_occurrence() {
    for (n, body) in [(1, "first copy"), (2, "second copy")] {
        let out = read(vec![ReadSel::Hpath {
            hpath: vec![seg("Top", None), seg("Dup", Some(n))],
        }])
        .expect("the pinned occurrence resolves");
        let ResponseBody::Read { sections, .. } = out else {
            panic!("read body expected");
        };
        let sections = sections.expect("sections mode");
        assert_eq!(sections.len(), 1);
        assert!(
            sections[0].content.contains(body),
            "n={n} must land on its own copy, got {:?}",
            sections[0].content
        );
    }
}

/// F6: when ALL selectors fail, the refusal names every one with its own
/// reason — symmetric with the partial-read notice, not first-only.
#[test]
fn all_fail_refusal_names_every_failed_selector() {
    let err = *read(vec![
        ReadSel::parse("Ghost"),
        ReadSel::parse("Top/Dup"),
        ReadSel::parse("Phantom"),
    ])
    .expect_err("all selectors fail");
    // Mixed miss + ambiguity: the miss keeps the refresh-class code.
    assert_eq!(err.code, ErrorCode::RefNotFound);
    let msg = err.message.as_deref().expect("message");
    assert!(msg.contains("no section addressed by \"Ghost\""), "{msg}");
    assert!(msg.contains("no section addressed by \"Phantom\""), "{msg}");
    assert!(msg.contains("\"Top/Dup\" is ambiguous (2 matches"), "{msg}");
    assert!(
        msg.contains("Nothing was read and no rev was minted."),
        "{msg}"
    );
}

/// The single-miss refusal keeps its standing spelling byte-for-byte, and its
/// remedy speaks both surfaces' dialect (F5) — never a bare `mrd` alone.
#[test]
fn single_miss_keeps_spelling_and_remedy_names_both_dialects() {
    let err = *read(vec![ReadSel::parse("Ghost")]).expect_err("miss refuses");
    assert_eq!(err.code, ErrorCode::RefNotFound);
    let msg = err.message.as_deref().expect("message");
    assert!(
        msg.starts_with("read: no section addressed by \"Ghost\" in dup.md."),
        "{msg}"
    );
    assert!(msg.contains("mode:\"toc\""), "MCP dialect named: {msg}");
    assert!(msg.contains("no --section"), "CLI dialect named: {msg}");
}

/// P1 (dogfood-p1-read-ambiguous-ref): a `^id` selector whose id two blocks
/// carry refuses `ambiguous_ref` — never a silent first match. The old door
/// served the first occurrence with exit 0 and handed out a `sec_rev` the
/// write door then refused, so read-then-write on a duplicated anchor was
/// unserviceable. No machine address exists per candidate (duplicate ids
/// share one spelling; the anchor grammar has no `n`), so the refusal counts
/// the carriers and teaches the anchor remedy.
#[test]
fn duplicate_anchor_selector_refuses_ambiguous_never_serves_first() {
    let err = *read_of(DUP_ANCHOR_DOC, vec![ReadSel::parse("^same-id")])
        .expect_err("a duplicated block id refuses");
    assert_eq!(err.code, ErrorCode::AmbiguousRef);
    let msg = err.message.as_deref().expect("message");
    assert!(
        !msg.contains("no section addressed"),
        "two blocks matched; a miss-shaped message is the dishonest answer: {msg}"
    );
    assert!(
        msg.contains("\"^same-id\" is ambiguous (2 blocks carry this id"),
        "{msg}"
    );
    assert!(
        msg.contains("give each a distinct id"),
        "the remedy speaks the anchor grammar: {msg}"
    );
    assert!(
        !msg.contains("rename one heading"),
        "never the heading-duplicate remedy on an anchor refusal: {msg}"
    );
}

/// Partial failure over a duplicated anchor: the resolved selector still
/// serves, and the notice names the duplicate with the anchor-plane reason —
/// no rev is minted for it.
#[test]
fn partial_failure_notice_names_the_duplicated_anchor() {
    let out = read_of(
        DUP_ANCHOR_DOC,
        vec![ReadSel::parse("Notes"), ReadSel::parse("^same-id")],
    )
    .expect("the resolved selector still serves");
    let ResponseBody::Read {
        sections, notice, ..
    } = out
    else {
        panic!("read body expected");
    };
    assert_eq!(sections.expect("sections mode").len(), 1);
    let notice = notice.expect("partial read carries a notice");
    assert!(
        notice.contains("^same-id (ambiguous, 2 blocks carry this id)"),
        "{notice}"
    );
}

/// Partial failure still serves the resolved content; the notice names the
/// ambiguous selector with its candidates, and mints no rev for it.
#[test]
fn partial_failure_notice_names_ambiguity_with_candidates() {
    let out = read(vec![ReadSel::parse("Top/Solo"), ReadSel::parse("Top/Dup")])
        .expect("the resolved selector still serves");
    let ResponseBody::Read {
        sections, notice, ..
    } = out
    else {
        panic!("read body expected");
    };
    assert_eq!(sections.expect("sections mode").len(), 1);
    let notice = notice.expect("partial read carries a notice");
    assert!(
        notice.starts_with("unresolved selectors (no rev minted): "),
        "{notice}"
    );
    assert!(
        notice.contains("Top/Dup (ambiguous, 2 matches:")
            && notice.contains(r#"[{"h":"Top"},{"h":"Dup","n":1}]"#),
        "{notice}"
    );
}
