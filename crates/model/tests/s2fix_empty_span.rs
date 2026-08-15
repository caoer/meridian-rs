//! Empty-span fingerprint class: a fingerprint must not be able to match
//! content it does not cover.
//!
//! `blake3("")` is a universal match (identical token for every empty span) —
//! a permanent false green. This file:
//!
//! 1. Enumerates ref forms from [`Selector`] (exhaustive `match` in
//!    [`disposition`]).
//! 2. Witnesses each disposition (can-empty shows a doc; non-empty shows
//!    surviving bytes).
//! 3. Asserts the refusal, not a colour — mint `Err(EmptySpan)`, verdict
//!    never-green arm (a grey-and-accept fix would still ship an undriftable pin).

use model::fingerprint::{ContentVerdict, EmptySpan, fingerprint_span};
use model::selector::{Color, RedReason, Selector, classify_pin, resolve_selector};

fn doc(raw: &str) -> model::Document {
    model::build(raw.to_string(), syntax::parse(raw))
}

/// The canonical (norm-v2) bytes a selector's resolved span covers — the exact
/// input the one hasher would see.
fn canonical(raw: &str, sel: &Selector) -> Vec<u8> {
    let d = doc(raw);
    let (target_doc, resolved) =
        resolve_selector(sel, Some(&d)).unwrap_or_else(|c| panic!("{sel:?} must resolve: {c:?}"));
    syntax::norm_v2_slice(
        &target_doc.raw,
        &resolved.span,
        &syntax::anchor_removals(&target_doc.raw),
    )
}

/// Mint over a selector's resolved span, through the production owner.
fn mint(raw: &str, sel: &Selector) -> Result<model::fingerprint::Fingerprint, EmptySpan> {
    let d = doc(raw);
    let (target_doc, resolved) =
        resolve_selector(sel, Some(&d)).unwrap_or_else(|c| panic!("{sel:?} must resolve: {c:?}"));
    fingerprint_span(
        target_doc,
        &resolved.span,
        &syntax::anchor_removals(&target_doc.raw),
    )
}

/// How a ref form stands to the empty-normalized-span class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Disposition {
    /// Some instance of this form normalizes to nothing; every such form needs
    /// a live witness below.
    CanNormalizeEmpty,
    /// Structurally cannot normalize empty. Needs a witness of surviving bytes.
    StructurallyNonEmpty,
    /// Never reaches the fingerprint plane at all — grey before resolution.
    NeverResolved,
}

/// Exhaustive over [`Selector`] — a new class must disposition here at compile time.
///
/// - [`Selector::Page`] — document root; empty/own-line-anchor-only files normalize empty.
/// - [`Selector::Heading`] — section includes heading line (`#` unreachable by
///   removals); never empty.
/// - [`Selector::Block`] — host block; an own-line anchor with NO preceding
///   block (document start — F-R4 attachment reaches everything else) keeps
///   its marker line as host, and the R2 removal empties it; every attached
///   or inline form keeps host bytes.
/// - [`Selector::ImmutableRoot`] — grey before resolution.
fn disposition(sel: &Selector) -> Disposition {
    match sel {
        Selector::Page | Selector::Block(_) => Disposition::CanNormalizeEmpty,
        Selector::Heading(_) => Disposition::StructurallyNonEmpty,
        Selector::ImmutableRoot { .. } => Disposition::NeverResolved,
    }
}

/// One enumerated row: a ref form instance and what its canonical bytes are.
struct Row {
    name: &'static str,
    raw: &'static str,
    sel: Selector,
    /// `true` when this instance normalizes to nothing.
    empties: bool,
}

fn rows() -> Vec<Row> {
    let block = |id: &str| Selector::Block(id.to_string());
    let heading = |segs: &[&str]| {
        Selector::Heading(
            segs.iter()
                .map(|s| model::HpathSeg {
                    h: (*s).to_string(),
                    n: None,
                })
                .collect(),
        )
    };
    vec![
        // Block: own-line anchor with no preceding block — the only shape
        // whose host still empties under the R2/R2b removal (an attached
        // own-line anchor hosts the block it attaches to; F-R4)
        Row {
            name: "bare #^anchor, own line at document start (R2)",
            raw: "^guideline\n\nbody\n",
            sel: block("guideline"),
            empties: true,
        },
        Row {
            name: "bare #^anchor, own line at EOF (R2b)",
            raw: "^guideline",
            sel: block("guideline"),
            empties: true,
        },
        Row {
            name: "bare #^anchor, own line indented (R2)",
            raw: "  ^guideline\n\nbody\n",
            sel: block("guideline"),
            empties: true,
        },
        // Block: own-line anchor mid-file — attached since F-R4, keeps the
        // host block's bytes (this row used to empty when the host was the
        // marker's own line)
        Row {
            name: "bare #^anchor, own line mid-file (attached)",
            raw: "# H\n\n^guideline\n\nbody\n",
            sel: block("guideline"),
            empties: false,
        },
        // Block: inline forms keep host text
        Row {
            name: "#^anchor hosted by a list item (R1)",
            raw: "# H\n\n- item text ^listanchor\n",
            sel: block("listanchor"),
            empties: false,
        },
        Row {
            name: "#^anchor trailing a paragraph (R1)",
            raw: "# H\n\nsome prose ^tail\n",
            sel: block("tail"),
            empties: false,
        },
        // Heading: never empty (heading line survives)
        Row {
            name: "heading-only section, no body",
            raw: "# H\n\n## Empty\n\n## Next\nx\n",
            sel: heading(&["H", "Empty"]),
            empties: false,
        },
        Row {
            name: "heading-only section at EOF",
            raw: "# H\n\n## Empty\n",
            sel: heading(&["H", "Empty"]),
            empties: false,
        },
        Row {
            name: "section whose whole body is an own-line anchor",
            raw: "# H\n\n## S\n^a\n\n## Next\nx\n",
            sel: heading(&["H", "S"]),
            empties: false,
        },
        Row {
            name: "heading line that is only the marker",
            raw: "#\n",
            sel: heading(&[""]),
            empties: false,
        },
        // Page grain can empty
        Row {
            name: "whole-page ref over an empty file",
            raw: "",
            sel: Selector::Page,
            empties: true,
        },
        Row {
            name: "whole-page ref over an anchors-only file",
            raw: "^a\n",
            sel: Selector::Page,
            empties: true,
        },
        Row {
            name: "whole-page ref over an anchors-only file at EOF",
            raw: "^a",
            sel: Selector::Page,
            empties: true,
        },
        Row {
            name: "whole-page ref over an ordinary file",
            raw: "# H\nbody\n",
            sel: Selector::Page,
            empties: false,
        },
    ]
}

/// Every enumerated instance's canonical bytes, measured — each agreeing with
/// its form's disposition. A `CanNormalizeEmpty` form must have a witness that
/// empties; a `StructurallyNonEmpty` form must have none.
#[test]
fn the_enumeration_measures_every_ref_form() {
    let mut forms_seen_empty: Vec<&'static str> = Vec::new();
    for row in rows() {
        let canon = canonical(row.raw, &row.sel);
        assert_eq!(
            canon.is_empty(),
            row.empties,
            "{}: canonical bytes were {:?}",
            row.name,
            String::from_utf8_lossy(&canon)
        );
        let d = disposition(&row.sel);
        if row.empties {
            assert_eq!(
                d,
                Disposition::CanNormalizeEmpty,
                "{}: a form with an empty witness must be dispositioned CanNormalizeEmpty",
                row.name
            );
            let form = match row.sel {
                Selector::Block(_) => "Block",
                Selector::Page => "Page",
                Selector::Heading(_) => "Heading",
                Selector::ImmutableRoot { .. } => "ImmutableRoot",
            };
            if !forms_seen_empty.contains(&form) {
                forms_seen_empty.push(form);
            }
        }
        if d == Disposition::StructurallyNonEmpty {
            assert!(
                !canon.is_empty(),
                "{}: dispositioned StructurallyNonEmpty but normalized to nothing",
                row.name
            );
        }
    }
    // Both CanNormalizeEmpty forms carry a witness.
    forms_seen_empty.sort_unstable();
    assert_eq!(
        forms_seen_empty,
        vec!["Block", "Page"],
        "expected witnesses for exactly the two CanNormalizeEmpty forms"
    );
}

/// Every instance that normalizes to nothing is refused by the owner with the
/// typed error; every other instance is minted. No third outcome.
#[test]
fn the_owner_refuses_every_empty_form_and_mints_every_other() {
    for row in rows() {
        match (mint(row.raw, &row.sel), row.empties) {
            (Err(EmptySpan), true) | (Ok(_), false) => {}
            (Ok(fp), true) => panic!(
                "{}: minted {} over an EMPTY normalized span — a token that matches \
                 every document",
                row.name, fp
            ),
            (Err(EmptySpan), false) => {
                panic!("{}: refused a span that has content", row.name)
            }
        }
    }
}

/// Without the guard, every empty-normalizing span in every document mints the
/// same token. Non-vacuity control: the two documents are genuinely different,
/// so a shared digest cannot be an artifact of identical fixtures.
#[test]
fn two_unrelated_documents_share_one_empty_digest() {
    // Document-start own-line anchors: the one shape whose host is still the
    // marker's own line (nothing precedes to attach to — F-R4), so the R2
    // removal empties the canonical span.
    let a = ("^one\n\nalpha body\n", Selector::Block("one".into()));
    let b = (
        "^two\n\nbeta body — utterly different\n",
        Selector::Block("two".into()),
    );

    let (fa, fb) = (
        mint(a.0, &Selector::Page).expect("alpha page has content"),
        mint(b.0, &Selector::Page).expect("beta page has content"),
    );
    assert_ne!(fa, fb, "the two fixture documents must differ");

    assert!(canonical(a.0, &a.1).is_empty());
    assert!(canonical(b.0, &b.1).is_empty());
    assert_eq!(mint(a.0, &a.1), Err(EmptySpan));
    assert_eq!(mint(b.0, &b.1), Err(EmptySpan));
}

/// Verdict side: the class is unreachable through `mrd pin`, so it arrives in
/// hand- or tool-authored lock blocks — a stored pin whose digest is
/// `blake3("")`. Such a pin must never read green.
#[test]
fn a_stored_empty_span_pin_can_never_read_green() {
    // The forged token: what a hand-authored lock carries today.
    let forged = format!("fp1.span2.b3.{}", blake3::hash(b"").to_hex());

    for row in rows().into_iter().filter(|r| r.empties) {
        let d = doc(row.raw);
        let verdict = model::fingerprint::verify_content_span(
            &d,
            &resolve_selector(&row.sel, Some(&d))
                .expect("resolves")
                .1
                .span,
            &forged,
        );
        assert_eq!(
            verdict,
            ContentVerdict::EmptySpan,
            "{}: an empty span must not reach the compare at all",
            row.name
        );
        assert_ne!(verdict, ContentVerdict::Green, "{}", row.name);

        let color = classify_pin(&row.sel, &forged, Some(&d));
        assert_eq!(
            color,
            Color::Red(RedReason::Drifted),
            "{}: a forged empty-span pin must render red",
            row.name
        );
    }
}

/// A forged empty-span pin reddens before any edit — the content it claims to
/// cover was never there — and no edit can make it green.
#[test]
fn the_pin_that_could_never_drift_now_reddens_and_stays_red() {
    let forged = format!("fp1.span2.b3.{}", blake3::hash(b"").to_hex());
    let sel = Selector::Block("guideline".into());

    let before = "# H\n\n^guideline\n\noriginal body\n";
    let after = "# H\n\n^guideline\n\nTOTALLY DIFFERENT BODY\n";

    for (label, raw) in [("before the edit", before), ("after the edit", after)] {
        let d = doc(raw);
        assert_eq!(
            classify_pin(&sel, &forged, Some(&d)),
            Color::Red(RedReason::Drifted),
            "{label}: the forged pin must be red"
        );
    }

    // Non-vacuity: the same anchor id on an inline host is a genuine pin —
    // green against its own bytes, red when they move. The red above is the
    // empty span, not a blanket verdict on `#^id`.
    let inline_v1 = "# H\n\n- real content ^guideline\n";
    let inline_v2 = "# H\n\n- edited content ^guideline\n";
    let honest = mint(inline_v1, &sel)
        .expect("an inline anchor has content")
        .into_string();
    assert_eq!(
        classify_pin(&sel, &honest, Some(&doc(inline_v1))),
        Color::Green
    );
    assert_eq!(
        classify_pin(&sel, &honest, Some(&doc(inline_v2))),
        Color::Red(RedReason::Drifted)
    );
}
