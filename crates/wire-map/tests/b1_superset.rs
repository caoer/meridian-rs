//! B1-SUPERSET: four wire-observable superset-by-embedding predicates
//! (contract §15 B1) at `wire_map::project`. Real parsed documents only.
//!
//!
//!
//!
//!
//!
//!

use wire::NodeKind as K;

fn project(raw: &str) -> Vec<wire::Node> {
    let doc = model::build(raw.to_owned(), syntax::parse(raw));
    wire_map::project(&doc)
}

/// Predicate 1: every dialect construct is wire-representable (11-kind enum).
///
///
///
#[test]
fn every_dialect_construct_is_wire_representable() {
    let src = "---\ntitle: Superset\n---\n\
               # Alpha\n\n\
               Para with `inline` and [[notes/x#H|alias]] then ![[img/y.png#^blk-1]] end. ^blk-a\n\n\
               > [!note]- folded\n> body\n\n\
               - [x] a task\n\n\
               | a | b |\n|---|---|\n| 1 | 2 |\n\n\
               ```rust\nfn f() {}\n```\n\n\
               %% a comment %%\n";
    let nodes = project(src);
    for kind in [
        K::Frontmatter,
        K::Heading,
        K::Fence,
        K::InlineCode,
        K::Anchor,
        K::Wikilink,
        K::Embed,
        K::Callout,
        K::Task,
        K::Table,
        K::Comment,
    ] {
        assert!(
            nodes.iter().any(|n| n.kind == kind),
            "dialect construct not wire-representable: {kind:?} missing from projection"
        );
    }
}

/// Predicate 2: wikilink info is carried WHOLE — target, heading, block,
/// alias all survive projection (contract §5.2 info table); the embed's
/// block ref survives beside it.
#[test]
fn wikilink_info_carried_whole() {
    let src = "# A\n\nSee [[notes/x#H|alias]] and ![[img/y.png#^blk-1]] end.\n";
    let nodes = project(src);
    let link = nodes
        .iter()
        .find(|n| n.kind == K::Wikilink)
        .expect("wikilink node projected");
    assert_eq!(
        link.info,
        Some(wire::Info::Wikilink {
            target: "notes/x".into(),
            heading: Some("H".into()),
            block: None,
            alias: Some("alias".into()),
        }),
        "wikilink info must be carried whole"
    );
    let embed = nodes
        .iter()
        .find(|n| n.kind == K::Embed)
        .expect("embed node projected");
    assert_eq!(
        embed.info,
        Some(wire::Info::Wikilink {
            target: "img/y.png".into(),
            heading: None,
            block: Some("blk-1".into()),
            alias: None,
        }),
        "embed block ref must be carried whole"
    );
}

/// Predicate 3: fence `unterminated` rides the wire (contract §5.2 — present
/// only when true; the honest EOF story is wire-observable).
#[test]
fn fence_unterminated_rides_the_wire() {
    let terminated = project("# A\n\n```rust\nfn f() {}\n```\n");
    let fence = terminated
        .iter()
        .find(|n| n.kind == K::Fence)
        .expect("terminated fence projected");
    assert_eq!(fence.unterminated, None, "terminated fence omits the flag");

    let unterminated = project("# A\n\n```rust\nfn f() {}\n");
    let fence = unterminated
        .iter()
        .find(|n| n.kind == K::Fence)
        .expect("unterminated fence projected");
    assert_eq!(
        fence.unterminated,
        Some(true),
        "unterminated fence must say so on the wire"
    );
}

/// Predicate 4: frontmatter `keys` preserve document order, never sorted.
///
///
///
#[test]
fn frontmatter_key_order_preserved_in_keys() {
    // deliberately non-alphabetical: a sort would betray itself
    let src = "---\ntitle: T\nzeta: z\nalpha: a\nmiddle: m\n---\n\n# A\n";
    let nodes = project(src);
    let fm = nodes
        .iter()
        .find(|n| n.kind == K::Frontmatter)
        .expect("frontmatter node projected");
    assert_eq!(
        fm.info,
        Some(wire::Info::Frontmatter {
            keys: vec![
                "title".into(),
                "zeta".into(),
                "alpha".into(),
                "middle".into()
            ],
        }),
        "keys must preserve document order, never sorted order"
    );
}

/// M2-PROJECT gate: the frozen total order holds over the projected list —
/// span.start asc, span.end desc (container before contained), kind ordinal
/// as tiebreak (contract §5.2) — property-asserted pairwise over a document
/// exercising every construct, nested sections included.
#[test]
fn projection_emits_frozen_total_order() {
    let src = "---\ntitle: T\n---\n\
               # A\n\npara `x` [[l]] ^a1\n\n\
               ## B\n\n- [ ] t\n\n### C\n\n| a |\n|---|\n\n\
               ## B\n\n%% c %%\n\n```\nf\n";
    let nodes = project(src);
    assert!(nodes.len() > 5, "corpus projects a non-trivial list");
    for pair in nodes.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        let ordered = a.span.0 < b.span.0
            || (a.span.0 == b.span.0
                && (a.span.1 > b.span.1 || (a.span.1 == b.span.1 && a.kind <= b.kind)));
        assert!(
            ordered,
            "frozen order violated: {:?}@{:?} before {:?}@{:?}",
            a.kind, a.span, b.kind, b.span
        );
    }
}
