//! `^id` block-anchor ingress as a positive requirement (gate a as amended,
//! 2026-08-07): the engine serves the anchor lane fully — toc publication,
//! stored-link resolution, miss teaching — first-class, never primary. One
//! focused test per capability, plus the negative halves of every teaching
//! clause (a refusal must never imply absence where the honest fact is a
//! coverage limit, and never claim a limit where the fact is absence).

use wire::{ErrorCode, Path as WPath, ReadSel, ResponseBody, SecRef};
use wire_serve::read::{NO_DECORATIONS, ReadParams, cat, composed_read};

/// Two list-item anchors (`^goal`, `^gate`) and one task-hosted anchor
/// (`^t1`) — the face addresses the first two; the task host is outside the
/// face's coverage (Go parity) while the parse tree still carries it.
const DOC: &str = "# Tasks\n\n- ship the gate ^goal\n- prove the gate ^gate\n- [ ] boxed ^t1\n\n# Notes\n\nbody\n";

fn doc() -> model::Document {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("card.md"), DOC).expect("write");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    fs::load(&root, std::path::Path::new("card.md")).expect("load")
}

fn read(sections: Option<Vec<ReadSel>>) -> Result<ResponseBody, Box<wire::ErrorBody>> {
    composed_read(
        &doc(),
        &WPath("card.md".into()),
        &wire::Root("r".into()),
        &ReadParams {
            sections,
            display_path: Some("card.md".into()),
            ..ReadParams::default()
        },
        None,
        &NO_DECORATIONS,
    )
}

// Capability 1 — toc publication.

/// The composed read publishes the anchor plane in its own array, in document
/// order, and the heading plane stays anchor-free — the plane split a `toc`
/// consumer relies on. The face's coverage boundary rides along: the
/// task-hosted `^t1` is absent (list-item hosts only, Go parity).
#[test]
fn toc_mode_publishes_the_anchor_plane_in_its_own_array() {
    let ResponseBody::Read { toc, anchors, .. } = read(None).expect("toc mode serves") else {
        panic!("composed read answers a Read body");
    };
    let ids: Vec<&str> = anchors.iter().map(|a| a.anchor.as_str()).collect();
    assert_eq!(ids, vec!["goal", "gate"], "list-item anchors, document order");
    for a in &anchors {
        let (start, end) = (a.span.0 as usize, a.span.1 as usize);
        assert!(
            DOC[start..end].contains(&format!("^{}", a.anchor)),
            "the published span carries its own marker: {:?}",
            &DOC[start..end]
        );
    }
    assert!(
        toc.expect("toc rows").iter().all(|r| r.depth >= 1),
        "no anchor row leaks into the heading plane"
    );
}

// Capability 2 — stored-link resolution.

/// The strict plane lands a `^id` on its host block leaf: `cat` serves the
/// block's own bytes, and host-kind coverage does not limit it (the parse
/// tree carries every anchor, so the task-hosted `^t1` resolves here too).
#[test]
fn cat_resolves_an_anchor_to_its_host_block() {
    let d = doc();
    let ResponseBody::Cat { content, .. } = cat(
        &d,
        Some(SecRef::Anchor {
            anchor: "gate".into(),
        }),
    )
    .expect("^gate resolves") else {
        panic!("cat answers a Cat body");
    };
    assert_eq!(
        content, "- prove the gate ^gate",
        "the block leaf's own bytes, not the marker and not the section"
    );
    assert!(
        cat(
            &d,
            Some(SecRef::Anchor {
                anchor: "t1".into()
            })
        )
        .is_ok(),
        "the strict plane resolves a task-hosted anchor the read face does not list"
    );
}

// Capability 3 — miss teaching (Law A-3: never fail bare).

/// A `cat` anchor miss teaches: the standing miss spelling, the read-voiced
/// partial state, the nearest live ids, and the where-to-find clause. Before
/// this, `cat` refused with a bare code and no message.
#[test]
fn cat_anchor_miss_teaches_nearest_live_anchors() {
    let err = *cat(
        &doc(),
        Some(SecRef::Anchor {
            anchor: "gale".into(),
        }),
    )
    .expect_err("^gale is absent");
    assert_eq!(err.code, ErrorCode::RefNotFound);
    let msg = err.message.as_deref().expect("a miss carries a sentence, never a bare code");
    assert!(msg.contains("no node addressed by \"^gale\""), "{msg}");
    assert!(
        msg.contains("Nothing was read and no rev was minted."),
        "read voice, not the write plane's batch clause: {msg}"
    );
    assert!(
        !msg.contains("No edit was applied"),
        "a read refusal must not speak about edits: {msg}"
    );
    assert!(
        msg.contains("Nearest live block anchors: ^goal, ^gate, ^t1."),
        "every live id is offered, bigram-ranked, ties in document order — \
         the task-hosted ^t1 included, because the strict plane CAN address it: {msg}"
    );
}

/// The negative half of the nearest hint: a page with no anchors says so
/// plainly instead of offering an empty candidate list to hunt through.
#[test]
fn cat_anchor_miss_on_an_anchorless_page_says_so() {
    let raw = "# Only\n\nprose, no anchors\n";
    let d = model::build(raw.to_string(), syntax::parse(raw));
    let err = *cat(
        &d,
        Some(SecRef::Anchor {
            anchor: "gone".into(),
        }),
    )
    .expect_err("nothing to resolve");
    let msg = err.message.as_deref().expect("message");
    assert!(
        msg.contains("This page carries no block anchors."),
        "{msg}"
    );
    assert!(
        !msg.contains("Nearest live block anchors"),
        "no fabricated candidates on an anchorless page: {msg}"
    );
}

/// A `cat` heading miss now rides the same teaching renderer — the fix names
/// the toc read, and the sentence still opens with the standing miss spelling.
#[test]
fn cat_heading_miss_teaches_instead_of_failing_bare() {
    let err = *cat(
        &doc(),
        Some(SecRef::Hpath {
            hpath: vec![wire::HpathSeg {
                h: "Ghost".into(),
                n: None,
            }],
        }),
    )
    .expect_err("Ghost is absent");
    let msg = err.message.as_deref().expect("a miss carries a sentence, never a bare code");
    assert!(msg.contains("no node addressed by \"Ghost\""), "{msg}");
    assert!(msg.contains("toc read"), "the fix names the section map: {msg}");
}

/// A composed-read anchor miss names the nearest face-addressable ids inside
/// the standing miss spelling — appended, never reshaped, so every consumer
/// of the standing prefix still matches.
#[test]
fn composed_read_anchor_miss_teaches_nearest() {
    let err = *read(Some(vec![ReadSel::parse("^gaol")])).expect_err("^gaol is absent");
    assert_eq!(err.code, ErrorCode::RefNotFound);
    let msg = err.message.as_deref().expect("message");
    assert!(
        msg.contains("no section addressed by \"^gaol\" (nearest live block anchors: ^gate, ^goal)"),
        "the hint names the face-addressable ids, bigram-ranked, inside the \
         standing miss spelling: {msg}"
    );
}

/// The coverage-limit clause: a task-hosted anchor EXISTS on the page, so the
/// refusal names the host kind and the limit — and must not offer nearest
/// candidates, which would imply the id is absent (the md-only-limit
/// pattern: name the limit, never imply absence).
#[test]
fn composed_read_task_hosted_anchor_names_the_coverage_limit() {
    let err = *read(Some(vec![ReadSel::parse("^t1")])).expect_err("outside face coverage");
    let msg = err.message.as_deref().expect("message");
    assert!(
        msg.contains("the anchor exists on this page"),
        "absence is the wrong claim — the anchor is real: {msg}"
    );
    assert!(
        msg.contains("host block is a task"),
        "the limit names the host kind: {msg}"
    );
    assert!(
        !msg.contains("nearest live block anchors"),
        "a limit refusal must not imply absence with candidates: {msg}"
    );
}

/// A composed read over a raw fixture — the custom-doc sibling of [`read`],
/// for probes whose host shapes the standing `DOC` does not carry.
fn read_raw(raw: &str, sel: &str) -> Result<ResponseBody, Box<wire::ErrorBody>> {
    let d = model::build(raw.to_string(), syntax::parse(raw));
    composed_read(
        &d,
        &WPath("card.md".into()),
        &wire::Root("r".into()),
        &ReadParams {
            sections: Some(vec![ReadSel::parse(sel)]),
            display_path: Some("card.md".into()),
            ..ReadParams::default()
        },
        None,
        &NO_DECORATIONS,
    )
}

/// The heading-host probe (dogfood P2-c): the coverage-limit refusal names
/// the REAL host kind — a heading, never "a paragraph" — and its Fix teaches
/// a servable path (find the enclosing section in the section map). It must
/// never point at `anchors[]`: that array carries list-item hosts only, so
/// the very id the refusal reports is absent there by construction.
#[test]
fn heading_hosted_anchor_refusal_names_heading_and_serves_a_path() {
    let err = *read_raw("## Has anchor ^anch-head\n\nbody\n", "^anch-head")
        .expect_err("outside face coverage");
    assert_eq!(err.code, ErrorCode::RefNotFound, "refusal code unchanged");
    let msg = err.message.as_deref().expect("message");
    assert!(
        msg.contains("the anchor exists on this page"),
        "absence is the wrong claim — the anchor is real: {msg}"
    );
    assert!(
        msg.contains("host block is a heading"),
        "the limit names the TRUE host kind: {msg}"
    );
    assert!(!msg.contains("paragraph"), "the paragraph lie is retired: {msg}");
    assert!(
        !msg.contains("anchors[]"),
        "the Fix must not point at an array that cannot list this id: {msg}"
    );
    assert!(
        msg.contains("toc read"),
        "the Fix teaches the servable path — the section map: {msg}"
    );
}

/// The frontmatter caret-tail probe (dogfood P2-c): the host is the
/// frontmatter — outside any section, so "read the enclosing section" would
/// be a second unservable teaching. The truthful clause points at the `props`
/// plane every composed read already carries.
#[test]
fn fm_hosted_anchor_refusal_names_frontmatter_and_serves_props() {
    let err = *read_raw("---\ntitle: x ^fm-anchor\n---\n# H\n\nbody\n", "^fm-anchor")
        .expect_err("outside face coverage");
    assert_eq!(err.code, ErrorCode::RefNotFound, "refusal code unchanged");
    let msg = err.message.as_deref().expect("message");
    assert!(
        msg.contains("the anchor exists on this page"),
        "absence is the wrong claim — the caret tail is on the page: {msg}"
    );
    assert!(
        msg.contains("frontmatter"),
        "the limit names the TRUE host — the frontmatter: {msg}"
    );
    assert!(!msg.contains("paragraph"), "the paragraph lie is retired: {msg}");
    assert!(
        !msg.contains("anchors[]"),
        "the Fix must not point at an array that cannot list this id: {msg}"
    );
    assert!(
        msg.contains("props"),
        "the Fix teaches the servable path — the frontmatter plane: {msg}"
    );
}

/// The partial-read notice carries the same per-selector teaching as the
/// all-fail refusal — one vocabulary on both paths.
#[test]
fn partial_read_notice_carries_the_anchor_teaching() {
    let body = read(Some(vec![
        ReadSel::parse("Tasks"),
        ReadSel::parse("^gaol"),
    ]))
    .expect("one selector serves");
    let ResponseBody::Read { notice, .. } = body else {
        panic!("composed read answers a Read body");
    };
    let notice = notice.expect("a partial read carries its notice");
    assert!(
        notice.contains("^gaol (nearest live block anchors: ^gate, ^goal)"),
        "{notice}"
    );
}
