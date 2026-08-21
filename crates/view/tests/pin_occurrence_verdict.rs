//! r8 D3 — the verdict half of occurrence handling: a lock row that stores the
//! RESOLVED occurrence (`"Dup#2"`) classifies at its one sibling, while a
//! legacy n-less row over duplicates stays grey `ambiguous` (the ledger
//! genuinely cannot say which sibling that row meant).
//!
//! The conversion under test is [`view::walk::model_selector`] — the one R4
//! structural door (U22) every verdict computer routes through.

use model::selector::{Color, GreyReason, classify_pin, selector_display};
use view::walk::model_selector;

/// Two same-named siblings with DIFFERENT bodies — a fingerprint over one can
/// never verify over the other, so a green here proves the address chose.
const DUP: &str = "# Guide\n\n## Dup\n\nfirst dup body.\n\n## Dup\n\nsecond dup body.\n";

fn doc(raw: &str) -> std::sync::Arc<model::Document> {
    std::sync::Arc::new(model::build(raw.to_string(), syntax::parse(raw)))
}

/// Live fingerprint of the n-th `Guide/Dup` occurrence — the token a correct
/// pin of that sibling holds.
fn live_token(d: &model::Document, occurrence: u32) -> String {
    let target = model::resolve(
        d,
        &model::Ref::Hpath(vec![
            model::HpathSeg {
                h: "Guide".into(),
                n: None,
            },
            model::HpathSeg {
                h: "Dup".into(),
                n: Some(occurrence),
            },
        ]),
    )
    .expect("the occurrence resolves");
    model::fingerprint::fingerprint_span(d, &target.span, &syntax::anchor_removals(&d.raw))
        .expect("the fixture section has content")
        .into_string()
}

/// An occurrence-bearing row resolves to its one sibling: green at that
/// sibling's token, measured-red at the other's — never grey.
#[test]
fn an_occurrence_bearing_row_classifies_at_its_one_sibling() {
    let d = doc(DUP);
    let sel = model_selector(
        "guide",
        &lock::Selector::Path(vec!["Guide".into(), "Dup#2".into()]),
    );

    assert_eq!(
        classify_pin(&sel, &live_token(&d, 2), Some(&d)),
        Color::Green,
        "the stored occurrence addresses the second sibling — a fresh mint \
         never walks grey in its own session"
    );
    assert!(
        matches!(
            classify_pin(&sel, &live_token(&d, 1), Some(&d)),
            Color::Red(_)
        ),
        "the wrong sibling's token is measured drift, not ambiguity"
    );
}

/// A legacy row without an occurrence stays grey `ambiguous` over duplicates —
/// the fix never reinterprets old rows.
#[test]
fn a_legacy_row_without_an_occurrence_stays_grey_ambiguous() {
    let d = doc(DUP);
    let sel = model_selector(
        "guide",
        &lock::Selector::Path(vec!["Guide".into(), "Dup".into()]),
    );
    assert_eq!(
        classify_pin(&sel, &live_token(&d, 1), Some(&d)),
        Color::Grey(GreyReason::Ambiguous)
    );
}

/// The display spelling round-trips the read face's own occurrence grammar.
#[test]
fn the_occurrence_display_matches_the_read_face_grammar() {
    let sel = model_selector(
        "guide",
        &lock::Selector::Path(vec!["Guide".into(), "Dup#2".into()]),
    );
    assert_eq!(selector_display(&sel), "Guide/Dup#2");
}

/// Only a well-formed trailing ordinal (`#` + a 1-based decimal, no leading
/// zero) is claimed by the occurrence spelling; every other `#`-bearing segment
/// stays literal heading text. Guard, green on both sides of the fix: the mint
/// refuses `#`-bearing headings, so such rows are hand-authored — and a row
/// whose text still resolves must keep resolving.
#[test]
fn a_malformed_ordinal_suffix_stays_literal_heading_text() {
    for literal in ["Dup#0", "Dup#02", "Dup#2x"] {
        let raw = format!("# G\n\n## {literal}\n\nliteral body.\n");
        let d = doc(&raw);
        let sel = model_selector(
            "g",
            &lock::Selector::Path(vec!["G".into(), literal.to_string()]),
        );
        let token = {
            let target = model::resolve(
                &d,
                &model::Ref::Hpath(vec![
                    model::HpathSeg {
                        h: "G".into(),
                        n: None,
                    },
                    model::HpathSeg {
                        h: literal.to_string(),
                        n: None,
                    },
                ]),
            )
            .expect("the literal heading resolves by its exact text");
            model::fingerprint::fingerprint_span(&d, &target.span, &syntax::anchor_removals(&d.raw))
                .expect("content")
                .into_string()
        };
        assert_eq!(
            classify_pin(&sel, &token, Some(&d)),
            Color::Green,
            "a `#`-bearing segment that is not a well-formed ordinal stays \
             literal heading text: {literal}"
        );
    }
}

/// An inner segment carries its ordinal too — occurrence handling is
/// per-segment, not leaf-only (duplicated parents each owning a child).
#[test]
fn an_inner_segment_ordinal_addresses_through_duplicated_parents() {
    let nested =
        "# Guide\n\n## Dup\n\n### Child\n\nunder first.\n\n## Dup\n\n### Child\n\nunder second.\n";
    let d = doc(nested);
    let sel = model_selector(
        "guide",
        &lock::Selector::Path(vec!["Guide".into(), "Dup#2".into(), "Child".into()]),
    );
    let token = {
        let target = model::resolve(
            &d,
            &model::Ref::Hpath(vec![
                model::HpathSeg {
                    h: "Guide".into(),
                    n: None,
                },
                model::HpathSeg {
                    h: "Dup".into(),
                    n: Some(2),
                },
                model::HpathSeg {
                    h: "Child".into(),
                    n: None,
                },
            ]),
        )
        .expect("the nested occurrence resolves");
        model::fingerprint::fingerprint_span(&d, &target.span, &syntax::anchor_removals(&d.raw))
            .expect("content")
            .into_string()
    };
    assert_eq!(classify_pin(&sel, &token, Some(&d)), Color::Green);
}
