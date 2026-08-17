//! `read` refusal contract: both-selector-planes contradiction.
//!
//! Pins properties, not bytes: message must name the file, disclose partial
//! state ("Nothing was read" / "no rev was minted"), carry `Fix:`, and keep
//! the two-plane distinction plus the ruled "pass one" verdict (F-R3).

use wire::{ErrorCode, Path as WPath};
use wire_serve::read::{NO_DECORATIONS, ReadParams, composed_read};

const RAW: &str = "# Scratch notes\n\nTop matter.\n\n## Findings\n\nBody text.\n";

fn doc() -> model::Document {
    model::build(RAW.to_string(), syntax::parse(RAW))
}

/// Both `toc` and `sections[]` is undefined (planes disagree); refusal
/// must meet the exemplar bar (file + partial state + Fix + distinction).
#[test]
fn both_selector_planes_refuses_at_the_exemplar_bar() {
    let d = doc();
    let err = composed_read(
        &d,
        &WPath("notes.md".into()),
        &wire::Root("r".into()),
        &ReadParams {
            toc: Some(wire::ReadSel::parse("Scratch notes")),
            sections: Some(vec![wire::ReadSel::parse("Scratch notes/Findings")]),
            display_path: Some("notes.md".into()),
        },
        &NO_DECORATIONS,
    )
    .expect_err("passing both selector planes refuses");

    assert_eq!(err.code, ErrorCode::BadRequest);
    let m = err
        .message
        .as_deref()
        .expect("the refusal is a sentence, not a bare code");

    // 1. File named.
    assert!(m.contains("notes.md"), "names the file: {m}");

    // 2. Partial state: nothing read, no rev minted.
    assert!(
        m.contains("Nothing was read") && m.contains("no rev was minted"),
        "discloses the partial state: {m}"
    );

    // 3. Fix clause present.
    assert!(m.contains("Fix:"), "carries a fix clause: {m}");

    // 4. The ruled verdict and the two-plane distinction (F-R3): "pass one",
    // map vs content.
    assert!(m.contains("pass one"), "carries the ruled verdict: {m}");
    assert!(
        m.contains("MAP") && m.contains("CONTENT"),
        "keeps the two-plane distinction: {m}"
    );
}
