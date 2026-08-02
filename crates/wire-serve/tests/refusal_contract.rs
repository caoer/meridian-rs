//! The `read` refusal contract — site 8 of the issue-05 family.
//!
//! **This refusal had no test at all**, which is how it survived three sweeps.
//! It was found neither by grepping the defect STRING ("mode toc") nor the
//! defect CLASS ("no section addressed"), because it is phrased like neither:
//! it explains a distinction between two selector planes and never names a
//! file, a partial state, or a fix. Leader `2702bc87` surfaced it by enumerating
//! the message corpus for literals carrying the sweep's own bar — `Fix:` and a
//! partial-state clause — and looking at the refusal-shaped literals in NEITHER
//! set.
//!
//! Its sibling 305 lines away in the same file (`mode sections needs selectors`)
//! got the full treatment in this sweep. One exemplary, one untouched, same
//! plane — which is exactly the half-leaked vocabulary the card exists to close.
//!
//! Asserted as PROPERTIES, not bytes, for the reason ruling `d9419c03` gives
//! about the u4a2 goldens: a byte pin forbids all rewording, including
//! improvements.

use wire::{ErrorCode, Path as WPath};
use wire_serve::read::{NO_DECORATIONS, ReadParams, composed_read};

const RAW: &str = "# Scratch notes\n\nTop matter.\n\n## Findings\n\nBody text.\n";

fn doc() -> model::Document {
    model::build(RAW.to_string(), syntax::parse(RAW))
}

/// Passing BOTH a `#fragment` and `sections[]` is a caller contradiction: the
/// fragment scopes the whole call, `sections[]` selects document-absolute
/// paths, so honouring both is undefined. The refusal for it must still meet
/// the bar every other refusal in this sweep now meets.
#[test]
fn both_selector_planes_refuses_at_the_exemplar_bar() {
    let d = doc();
    let err = composed_read(
        &d,
        &WPath("notes.md".into()),
        &wire::Root("r".into()),
        &ReadParams {
            frag: Some("Scratch-notes".into()),
            sections: Some(vec!["Scratch-notes/Findings".into()]),
            display_path: Some("notes.md".into()),
            ..ReadParams::default()
        },
        None,
        &NO_DECORATIONS,
    )
    .expect_err("passing both selector planes refuses");

    assert_eq!(err.code, ErrorCode::BadRequest);
    let m = err
        .message
        .as_deref()
        .expect("the refusal is a sentence, not a bare code");

    // 1. THE FILE. `display` is already in scope one line above the refusal, so
    //    omitting it was never a plumbing limit — the refusal simply never said
    //    which page it was talking about.
    assert!(m.contains("notes.md"), "names the file: {m}");

    // 2. THE PARTIAL STATE. A read that refuses serves nothing and mints no
    //    receipt; saying so is what stops a caller wondering whether a rev they
    //    now hold came from this call.
    assert!(
        m.contains("Nothing was read") && m.contains("no rev was minted"),
        "discloses the partial state: {m}"
    );

    // 3. THE FIX, naming an act rather than restating the problem.
    assert!(m.contains("Fix:"), "carries a fix clause: {m}");

    // 4. The one thing the ORIGINAL got right, kept: it taught the distinction
    //    between the two planes. The defect was never that it explained too
    //    much, only that it stopped before saying what to do.
    assert!(
        m.contains("scopes the whole call"),
        "keeps the distinction the original taught: {m}"
    );
}
