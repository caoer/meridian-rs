//! W-5/W-4 — `replace_section` payload containment (wire-contract §A.3, the
//! containment law; ZT-ratified spec `replace-section-containment`, session
//! 12-04-f2-mrd-integration).
//!
//! The invariant under test: after `replace_section(target)`, every byte
//! outside the target's subtree is identical, and the target's subtree is
//! exactly the payload. Escapes refuse whole with the
//! `payload_escapes_section` grammar; the first-line address echo is the one
//! silent normalization; a refusal folds in the stale-rev fact when the
//! passed rev is also stale (one refusal, both facts — W-4).
//!
//! Test names carry the spec's case numbers (`case_NN`) so the 12-row matrix
//! is auditable against the ratified spec 1:1.

use std::path::PathBuf;

use wire::{ErrorCode, HpathSeg, PlanEdit, ResponseBody};
use wire_serve::write::{SpliceArgs, SpliceOutcome, splice};

/// The spec's fixture document, byte-exact.
const PAGE: &str = "# Page\n\n## Level tests\nold body\n\n## Next section\n";

fn ws() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("page.md"), PAGE).expect("seed");
    let root = fs::WorkspaceRoot(PathBuf::from(dir.path()));
    (dir, root)
}

fn seg(h: &str) -> HpathSeg {
    HpathSeg {
        h: h.into(),
        n: None,
    }
}

/// The target's live node rev, read back from disk.
fn target_rev(root: &fs::WorkspaceRoot) -> String {
    let doc = fs::load(root, std::path::Path::new("page.md")).expect("load");
    let target = model::Ref::Hpath(vec![
        model::HpathSeg {
            h: "Page".into(),
            n: None,
        },
        model::HpathSeg {
            h: "Level tests".into(),
            n: None,
        },
    ]);
    model::resolve(&doc, &target).expect("resolves").node_rev.0
}

/// One `replace_section` on `§Page/Level tests` with `body` and `rev`.
fn replace(
    root: &fs::WorkspaceRoot,
    body: &str,
    rev: &str,
) -> Result<SpliceOutcome, Box<wire::ErrorBody>> {
    splice(
        root,
        None,
        &SpliceArgs {
            id: None,
            origin: wire_serve::guard::Origin::InProcess,
            path: wire::Path("page.md".into()),
            actor: Some("agent:w5".into()),
            now: Some("2026-08-12T12:00:00Z".into()),
            receipt: None,
            if_root: None,
            dry: false,
            force: false,
            edits: Vec::new(),
            plan_edits: vec![PlanEdit::ReplaceSection {
                hpath: vec![seg("Page"), seg("Level tests")],
                body: body.into(),
                rev: Some(rev.into()),
            }],
            pin: None,
        },
        &[],
        None,
    )
}

fn page_bytes(dir: &tempfile::TempDir) -> String {
    std::fs::read_to_string(dir.path().join("page.md")).expect("read")
}

/// A refusal must leave the file byte-identical and speak the
/// `payload_escapes_section` grammar naming the offending line and BOTH
/// levels (spec § Refusal grammar).
#[track_caller]
fn assert_escape_refusal(
    err: &wire::ErrorBody,
    dir: &tempfile::TempDir,
    line: u32,
    payload_level: u32,
    heading_text: &str,
) {
    assert_eq!(err.code, ErrorCode::BadRequest, "fix-class refusal");
    let msg = err.message.as_deref().expect("teaching message");
    assert!(
        msg.starts_with("payload_escapes_section — "),
        "grammar token leads: {msg}"
    );
    assert!(
        msg.contains(&format!(
            "body line {line} is a level-{payload_level} heading"
        )),
        "names the offending line + its level: {msg}"
    );
    assert!(
        msg.contains(&format!("\"{heading_text}\"")),
        "names the offending heading text: {msg}"
    );
    assert!(
        msg.contains(
            "target \"Page/Level tests\" is level 2, so payload headings must be level 3+"
        ),
        "names the target's level and the floor: {msg}"
    );
    assert!(
        msg.contains(
            "replace_section replaces the target's subtree only. To create a sibling, \
             target the parent section or use create_section."
        ),
        "teaches the honest alternative: {msg}"
    );
    assert_eq!(page_bytes(dir), PAGE, "refused whole — file unchanged");
}

/// The armed fact must name the addressed target with a real rev transition —
/// receipt honesty at the armed grain (§6.4 renders receipts from these).
#[track_caller]
fn assert_armed_names_target(out: &SpliceOutcome) {
    let ResponseBody::Splice { armed, .. } = &out.body else {
        panic!("splice body");
    };
    assert_eq!(armed.edits.len(), 1);
    let e = &armed.edits[0];
    let wire::SecRef::Hpath { hpath } = &e.target else {
        panic!("hpath target");
    };
    assert_eq!(
        hpath.iter().map(|s| s.h.as_str()).collect::<Vec<_>>(),
        vec!["Page", "Level tests"],
        "the armed fact names the section the caller addressed"
    );
    assert_ne!(
        e.node_rev_before, e.node_rev_after,
        "a landed replace arms a real transition"
    );
}

/// Case 1 — plain body, no headings: lands in the section, siblings untouched.
#[test]
fn case_01_plain_body_lands_in_section() {
    let (dir, root) = ws();
    let out = replace(&root, "plain new body\n", &target_rev(&root)).expect("commits");
    assert_armed_names_target(&out);
    assert_eq!(
        page_bytes(&dir),
        "# Page\n\n## Level tests\nplain new body\n## Next section\n"
    );
}

/// Case 2 — `###` under the h2 target: nests inside the target.
#[test]
fn case_02_deeper_h3_nests_inside_target() {
    let (dir, root) = ws();
    replace(&root, "intro\n\n### Sub\n\nnested\n", &target_rev(&root)).expect("commits");
    assert_eq!(
        page_bytes(&dir),
        "# Page\n\n## Level tests\nintro\n\n### Sub\n\nnested\n## Next section\n"
    );
    // The h3 resolves INSIDE the target's chain.
    let doc = fs::load(&root, std::path::Path::new("page.md")).expect("load");
    let sub = model::Ref::Hpath(
        ["Page", "Level tests", "Sub"]
            .into_iter()
            .map(|h| model::HpathSeg {
                h: h.into(),
                n: None,
            })
            .collect(),
    );
    assert!(model::resolve(&doc, &sub).is_ok(), "h3 nests inside target");
}

/// Case 3 — `####` skip-level under the h2 target: still nests inside.
#[test]
fn case_03_skip_level_h4_nests_inside_target() {
    let (dir, root) = ws();
    replace(&root, "#### Deep\n\nskip-level\n", &target_rev(&root)).expect("commits");
    assert_eq!(
        page_bytes(&dir),
        "# Page\n\n## Level tests\n#### Deep\n\nskip-level\n## Next section\n"
    );
}

/// Case 4 — the payload's FIRST line echoes the target's own heading (same
/// level, same title): the echo is the caller repeating the address — it is
/// stripped silently and the remainder splices. The armed fact still names
/// the target with a true transition.
#[test]
fn case_04_echo_first_line_normalizes_away() {
    let (dir, root) = ws();
    let out = replace(
        &root,
        "## Level tests\n\nnew body written including the address echo\n",
        &target_rev(&root),
    )
    .expect("the echo normalizes — no refusal");
    assert_armed_names_target(&out);
    assert_eq!(
        page_bytes(&dir),
        "# Page\n\n## Level tests\n\nnew body written including the address echo\n## Next section\n",
        "heading stripped, remainder spliced, Next section untouched"
    );
}

/// Case 5 — the target's own heading at a LATER position: refused, and the
/// refusal names the duplicate-sibling consequence plus the first-line-only
/// echo law.
#[test]
fn case_05_same_title_later_position_refuses() {
    let (dir, root) = ws();
    let err = replace(
        &root,
        "intro\n\n## Level tests\n\nmore\n",
        &target_rev(&root),
    )
    .expect_err("refuses");
    assert_escape_refusal(&err, &dir, 3, 2, "Level tests");
    let msg = err.message.as_deref().expect("message");
    assert!(
        msg.contains("the target's own name"),
        "teaches the duplicate-sibling name: {msg}"
    );
    assert!(
        msg.contains("normalized away only as the payload's FIRST line"),
        "teaches the first-line echo law: {msg}"
    );
}

/// Case 6 — a same-level heading with a DIFFERENT title: the sibling escape.
/// Today this landed, emptied the target, and receipted `wrote §target` —
/// the highest-blast half of W-5.
#[test]
fn case_06_same_level_different_title_refuses() {
    let (dir, root) = ws();
    let err = replace(
        &root,
        "## A sibling with a different title\n\nescapes today\n",
        &target_rev(&root),
    )
    .expect_err("refuses");
    assert_escape_refusal(&err, &dir, 1, 2, "A sibling with a different title");
}

/// Case 7 — an h1 payload under an h2 target: the second-document-root
/// escape. Same grammar as case 6.
#[test]
fn case_07_h1_payload_refuses() {
    let (dir, root) = ws();
    let err = replace(
        &root,
        "# h1 inside an h2 body\n\nescapes today\n",
        &target_rev(&root),
    )
    .expect_err("refuses");
    assert_escape_refusal(&err, &dir, 1, 1, "h1 inside an h2 body");
}

/// Case 8 — `##`/`#` lines inside a fenced code block are code, never
/// headings: containment judges the PARSED payload, not lines.
#[test]
fn case_08_fenced_hash_lines_are_contained() {
    let (dir, root) = ws();
    replace(
        &root,
        "```\n## not a heading\n# nor this\n```\n",
        &target_rev(&root),
    )
    .expect("fence content is code — commits");
    assert_eq!(
        page_bytes(&dir),
        "# Page\n\n## Level tests\n```\n## not a heading\n# nor this\n```\n## Next section\n"
    );
}

/// Case 9 — setext heading in the payload. This test PINS current ATX-only
/// dialect behavior pending a slated engine-side define (leader ruling
/// 2026-08-12): the engine's dialect parse does not mint setext headings, so
/// the payload splices CONTAINED as body text and no new section exists in
/// the engine's tree. The engine/Obsidian divergence (`CommonMark` renders an
/// h2) stays recorded in the ratified spec, case 9 — a future define should
/// flip exactly this test.
#[test]
fn case_09_setext_pins_atx_only_dialect_pending_engine_define() {
    let (dir, root) = ws();
    replace(
        &root,
        "Setext title\n------------\n\nbody under it\n",
        &target_rev(&root),
    )
    .expect("setext is not a dialect heading — splices contained");
    assert_eq!(
        page_bytes(&dir),
        "# Page\n\n## Level tests\nSetext title\n------------\n\nbody under it\n## Next section\n"
    );
    // The engine tree holds NO section named by the setext line.
    let doc = fs::load(&root, std::path::Path::new("page.md")).expect("load");
    let ghost = model::Ref::Hpath(vec![model::HpathSeg {
        h: "Setext title".into(),
        n: None,
    }]);
    assert!(
        model::resolve(&doc, &ghost).is_err(),
        "no engine-side section is minted by a setext underline"
    );
}

/// Case 10 — empty body: the section empties (words:0 is legal).
#[test]
fn case_10_empty_body_empties_the_section() {
    let (dir, root) = ws();
    replace(&root, "", &target_rev(&root)).expect("commits");
    assert_eq!(
        page_bytes(&dir),
        "# Page\n\n## Level tests\n## Next section\n"
    );
}

/// Case 11 — echo heading only, nothing else: normalizes to an empty
/// section, with and without a trailing terminator.
#[test]
fn case_11_echo_only_normalizes_to_empty_section() {
    for echo_only in ["## Level tests\n", "## Level tests"] {
        let (dir, root) = ws();
        replace(&root, echo_only, &target_rev(&root)).expect("commits");
        assert_eq!(
            page_bytes(&dir),
            "# Page\n\n## Level tests\n## Next section\n",
            "echo-only payload {echo_only:?} empties the section"
        );
    }
}

/// Case 12 — same title one level DEEPER: ordinary h3 content, never
/// normalized (the echo law requires the target's own level).
#[test]
fn case_12_same_title_deeper_nests_without_normalization() {
    let (dir, root) = ws();
    replace(
        &root,
        "### Level tests\n\ndeeper same title\n",
        &target_rev(&root),
    )
    .expect("commits");
    assert_eq!(
        page_bytes(&dir),
        "# Page\n\n## Level tests\n### Level tests\n\ndeeper same title\n## Next section\n"
    );
    // Resolves as the target's own child.
    let doc = fs::load(&root, std::path::Path::new("page.md")).expect("load");
    let child = model::Ref::Hpath(
        ["Page", "Level tests", "Level tests"]
            .into_iter()
            .map(|h| model::HpathSeg {
                h: h.into(),
                n: None,
            })
            .collect(),
    );
    assert!(model::resolve(&doc, &child).is_ok(), "h3 child resolves");
}

/// W-4 — an escaping payload sent with a STALE rev draws ONE refusal that
/// carries both facts: the containment teaching AND the stale-rev fact with
/// the current rev inline. No CONFLICT-then-would_corrupt two-step.
#[test]
fn w4_stale_rev_and_escaping_payload_one_refusal_both_facts() {
    let (dir, root) = ws();
    let live = target_rev(&root);
    let err = replace(
        &root,
        "# h1 inside an h2 body\n\nescapes today\n",
        "deadbeefdeadbeef",
    )
    .expect_err("refuses");
    assert_escape_refusal(&err, &dir, 1, 1, "h1 inside an h2 body");
    let msg = err.message.as_deref().expect("message");
    assert!(
        msg.contains("the rev you passed is stale"),
        "carries the CAS fact: {msg}"
    );
    assert!(
        msg.contains("\"deadbeefdeadbeef\""),
        "names the rev the caller sent: {msg}"
    );
    assert!(
        msg.contains(&live),
        "carries the CURRENT rev as the resend token: {msg}"
    );
}

/// W-4 companion — a stale rev with a CLEAN payload stays the ordinary
/// `cas_mismatch` lane (the masking fix must not eat the pure-CAS path).
#[test]
fn w4_stale_rev_with_clean_payload_stays_cas_mismatch() {
    let (dir, root) = ws();
    let err = replace(&root, "clean body\n", "deadbeefdeadbeef").expect_err("refuses");
    assert_eq!(err.code, ErrorCode::CasMismatch);
    assert_eq!(page_bytes(&dir), PAGE, "refused whole — file unchanged");
}
