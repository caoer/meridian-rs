//! Law A-2 (`docs/address-grammar.md` § 4.4): a fragment is selector bytes to
//! its end — `@` included. The name lane never splits an `@fp` out of a
//! fragment, so an `@`-bearing heading is addressable by its own spelling.
//!
//! The constructible pair from the design: two REAL headings, one the other's
//! prefix up to an `@`. Under the superseded split the spelling of the
//! `@`-bearing heading resolved to the WRONG real section — the silent-wrong
//! class, not a miss.

use model::{HpathSeg, Ref};

fn doc(raw: &str) -> model::Document {
    model::build(raw.to_string(), syntax::parse(raw))
}

/// Resolve one `Addr` fragment the machine-canonical way: the selector as a
/// single hpath segment, byte-exact.
fn resolve_selector_bytes(
    doc: &model::Document,
    spelling: &str,
) -> Result<model::Target, model::ResolveError> {
    let addr = addr::Addr::parse(spelling).expect("the spelling parses");
    model::resolve(
        doc,
        &Ref::Hpath(vec![HpathSeg {
            h: addr.selector().to_string(),
            n: None,
        }]),
    )
}

/// The silent-wrong pair: headings `Deploy` and `Deploy@prod` both REAL. The
/// spelling `#Deploy@prod` must resolve byte-exactly to `Deploy@prod` — never
/// to its `@`-truncated prefix, which names the other real section.
#[test]
fn an_at_bearing_heading_resolves_to_itself_never_its_truncated_prefix() {
    let raw = "# Deploy\n\nthe wrong section\n\n# Deploy@prod\n\nthe right section\n";
    let doc = doc(raw);

    let deploy = model::resolve(
        &doc,
        &Ref::Hpath(vec![HpathSeg {
            h: "Deploy".into(),
            n: None,
        }]),
    )
    .expect("the plain heading resolves");
    let deploy_prod = model::resolve(
        &doc,
        &Ref::Hpath(vec![HpathSeg {
            h: "Deploy@prod".into(),
            n: None,
        }]),
    )
    .expect("the @-bearing heading resolves");

    let resolved =
        resolve_selector_bytes(&doc, "page.md#Deploy@prod").expect("the spelling resolves");
    assert_eq!(
        resolved.span, deploy_prod.span,
        "the spelling must resolve to the @-bearing heading's own section",
    );
    assert_ne!(
        resolved.span, deploy.span,
        "and never to the truncated prefix's section — the silent-wrong class",
    );
}

/// The design's spaced pair: `Deploy` / `Deploy @ prod`. Byte-exact means the
/// spaces are selector bytes too.
#[test]
fn the_design_pair_deploy_at_prod_is_addressable_by_its_own_spelling() {
    let raw = "# Deploy\n\nplain\n\n# Deploy @ prod\n\ndecorated name, real heading\n";
    let doc = doc(raw);

    let expected = model::resolve(
        &doc,
        &Ref::Hpath(vec![HpathSeg {
            h: "Deploy @ prod".into(),
            n: None,
        }]),
    )
    .expect("the heading is real");

    let resolved =
        resolve_selector_bytes(&doc, "page.md#Deploy @ prod").expect("the spelling resolves");
    assert_eq!(resolved.span, expected.span);
}

/// The type follows the law: the fragment is recorded verbatim as the
/// selector, and `Display` round-trips it without an `@` re-join.
#[test]
fn the_fragment_is_selector_bytes_to_its_end_and_round_trips() {
    for (spelling, selector) in [
        ("page.md#Deploy@prod", "Deploy@prod"),
        ("page.md#Deploy @ prod", "Deploy @ prod"),
        ("page.md#Sec@green.b3af12cd", "Sec@green.b3af12cd"),
        ("sessions:a/b.md#^claim@fp1.span2.b3.dead", "^claim@fp1.span2.b3.dead"),
    ] {
        let addr = addr::Addr::parse(spelling).expect("parses");
        assert_eq!(addr.selector(), selector, "{spelling}: every fragment byte is selector bytes");
        assert_eq!(addr.to_string(), spelling, "{spelling}: lossless round-trip");
    }
}
