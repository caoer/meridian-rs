//! **R31 — the empty-span fingerprint class.**
//!
//! The invariant, verbatim and non-negotiable:
//!
//! > A fingerprint must not be able to match content it does not cover.
//!
//! `blake3` of empty input is a *universal match*: every empty-normalizing span
//! in every document mints the identical token, so such a pin can never drift.
//! It is the purest false green there is — a permanent green that no edit
//! anywhere can turn red, in the module whose entire product is that green
//! means green.
//!
//! # What this file proves, and in which order
//!
//! 1. **The enumeration is derived from a TYPE, not written as prose** — the
//!    ref-form door set IS [`Selector`]'s variants, and the `match` in
//!    [`disposition`] is exhaustive, so a new selector class cannot ship
//!    without dispositioning itself here (R35(c): *wherever a door set can be
//!    derived from a type, derive it and assert it; the matrix is the fallback,
//!    not the goal*). A prose table in a card falls out of date silently; this
//!    cannot.
//! 2. **Every form's disposition carries a WITNESS** — a can-normalize-empty
//!    form shows a document where it does; a structurally-non-empty form shows
//!    the bytes that survive, so "it cannot happen" is measured, never asserted.
//! 3. **The assert is the REFUSAL, not the colour** (R31's test shape). A fix
//!    that renders an empty-span pin grey and still accepts it would pass a
//!    colour assertion and ship a pin that cannot drift. So the mint asserts
//!    `Err(EmptySpan)` and the verdict asserts a never-green *verdict arm*.

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
    /// Some instance of this form normalizes to nothing. This is the class R31
    /// closes, and every such form needs a live witness below.
    CanNormalizeEmpty,
    /// No instance can, and the reason is structural — not "we could not think
    /// of one". Needs a witness showing which bytes survive.
    StructurallyNonEmpty,
    /// Never reaches the fingerprint plane at all — grey before resolution.
    NeverResolved,
}

/// **THE ENUMERATION.** Exhaustive over [`Selector`] by construction: adding a
/// fifth ref form breaks this `match` at compile time, so the class cannot be
/// re-opened by a new address class without someone dispositioning it.
///
/// - [`Selector::Page`] — resolves to the document ROOT. **This is the form the
///   ruling did not predict.** R31 enumerated fragment forms (bare anchor,
///   heading-only section, anchor on a blank line); the fragment-LESS whole-page
///   ref reaches the same hasher, and an empty file (or one whose whole content
///   is own-line anchors) normalizes to nothing.
/// - [`Selector::Heading`] — the section span starts at the heading LINE, whose
///   `#` bytes no removal rule can reach (`anchor_removals` removes only anchor
///   markers plus at most one preceding space, or a whole own-line anchor line;
///   R2b reaches back over at most a `\r\n`). So a heading section always keeps
///   its heading. The ruling's predicted "heading-only section with no body"
///   form therefore does NOT normalize empty — witnessed below.
/// - [`Selector::Block`] — resolves to the anchor's HOST LINE
///   (`model::anchor_host_span`, terminator-excluded). An OWN-LINE anchor's R2
///   removal covers that whole line plus its terminator, so the intersection is
///   the entire span: empty. An inline/tail anchor keeps its host text.
/// - [`Selector::ImmutableRoot`] — grey `immutable-root` before any resolution.
fn disposition(sel: &Selector) -> Disposition {
    match sel {
        Selector::Page | Selector::Block(_) => Disposition::CanNormalizeEmpty,
        Selector::Heading(_) => Disposition::StructurallyNonEmpty,
        Selector::ImmutableRoot { .. } => Disposition::NeverResolved,
    }
}

/// One enumerated row: a ref form instance and what its canonical bytes are.
struct Row {
    /// The row name as it appears in the card's enumeration table.
    name: &'static str,
    raw: &'static str,
    sel: Selector,
    /// `true` when THIS instance normalizes to nothing.
    empties: bool,
}

fn rows() -> Vec<Row> {
    let block = |id: &str| Selector::Block(id.to_string());
    let heading =
        |segs: &[&str]| Selector::Heading(segs.iter().map(|s| (*s).to_string()).collect());
    vec![
        // ── Block: the own-line anchor, in all three shapes norm-v2 can remove
        Row {
            name: "bare #^anchor, own line mid-file (R2)",
            raw: "# H\n\n^guideline\n\nbody\n",
            sel: block("guideline"),
            empties: true,
        },
        Row {
            name: "bare #^anchor, own line at EOF (R2b)",
            raw: "# H\n\n^guideline",
            sel: block("guideline"),
            empties: true,
        },
        Row {
            name: "bare #^anchor, own line indented (R2)",
            raw: "# H\n\n  ^guideline\n",
            sel: block("guideline"),
            empties: true,
        },
        // ── Block: the inline forms, which keep their host text
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
        // ── Heading: every shape the ruling suspected, all structurally safe
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
        // ── Page: THE UNPREDICTED FORM
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

/// Row 1 of the table: every enumerated instance's canonical bytes, measured —
/// and each one agreeing with its FORM's disposition. A `CanNormalizeEmpty`
/// form must actually have a witness that empties (otherwise the row is a
/// worry, not a finding); a `StructurallyNonEmpty` form must have none.
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
    // Arithmetic that closes (R32): both CanNormalizeEmpty forms carry a
    // witness. An enumeration whose own count does not close is the shape of
    // every finding in this loop.
    forms_seen_empty.sort_unstable();
    assert_eq!(
        forms_seen_empty,
        vec!["Block", "Page"],
        "expected witnesses for exactly the two CanNormalizeEmpty forms"
    );
}

/// **THE MINT REFUSAL — the assert is the refusal, not the colour.** Every
/// enumerated instance that normalizes to nothing is refused by the owner with
/// the TYPED error; every instance that does not is minted. No third outcome.
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

/// The defect stated as the property it violates: without the guard, every
/// empty-normalizing span in every document mints the SAME token. This is the
/// non-vacuity control the fixture-trap rule demands — it proves the two
/// documents are genuinely different, so the shared digest was the bug and not
/// an artifact of identical fixtures.
#[test]
fn two_unrelated_documents_share_one_empty_digest() {
    let a = (
        "# Alpha\n\n^one\n\nalpha body\n",
        Selector::Block("one".into()),
    );
    let b = (
        "# Beta\n\n^two\n\nbeta body — utterly different\n",
        Selector::Block("two".into()),
    );

    // The documents are genuinely different: their PAGE fingerprints differ.
    let (fa, fb) = (
        mint(a.0, &Selector::Page).expect("alpha page has content"),
        mint(b.0, &Selector::Page).expect("beta page has content"),
    );
    assert_ne!(fa, fb, "the two fixture documents must differ");

    // Yet the two anchors' canonical bytes are BOTH empty — one digest for two
    // unrelated pages. That is the universal match, and it is why the owner
    // refuses instead of hashing.
    assert!(canonical(a.0, &a.1).is_empty());
    assert!(canonical(b.0, &b.1).is_empty());
    assert_eq!(mint(a.0, &a.1), Err(EmptySpan));
    assert_eq!(mint(b.0, &b.1), Err(EmptySpan));
}

/// **THE VERDICT SIDE — the load-bearing guard (reviewer's reachability
/// addendum).** The class is unreachable through `mrd pin`, so it arrives in
/// HAND- or TOOL-AUTHORED lock blocks: a stored pin whose digest is
/// `blake3("")`. Such a pin must never read green — against the document it was
/// forged for, against an unrelated document, or after the target is edited.
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

        // And the colour the render sees: red, never green, never grey.
        let color = classify_pin(&row.sel, &forged, Some(&d));
        assert_eq!(
            color,
            Color::Red(RedReason::Drifted),
            "{}: a forged empty-span pin must render red",
            row.name
        );
    }
}

/// The closure stated as the thing that was broken: the pin that COULD NEVER
/// DRIFT now reddens, and it reddens *before* any edit — the content it claims
/// to cover was never there. Editing the target cannot make it green again
/// either, which is the "no edit anywhere can turn it red" defect inverted.
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

    // Non-vacuity: the same anchor id on an INLINE host is a genuine pin — it
    // mints, it is green against its own bytes, and it reddens when they move.
    // So the red above is the empty span, not a blanket verdict on `#^id`.
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
