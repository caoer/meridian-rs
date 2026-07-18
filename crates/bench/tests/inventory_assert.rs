//! The ≥-planted assertion: for every generated file, a correct parser must
//! find at least the constructs the generator planted (the generator's
//! recognizable-context invariant makes the inventory a sound lower bound).
//! This extends correctness checking to arbitrary generated scale; the frozen
//! GT pack in `testsuite` remains the only byte-exact truth.
//!
//! Dormant: `#[ignore]` until rung 1 gives `syntax::parse` a body — remove the
//! attribute when the rung lands (build-out obligation, see bench's lib doc).

use bench::inventory::Inventory;
use bench::profile::{ConstructRate, KNOWN_CONSTRUCTS, Profile, Recipe, SizeDist};

fn kind_id(kind: &syntax::DialectKind) -> &'static str {
    match kind {
        syntax::DialectKind::Frontmatter { .. } => "frontmatter",
        syntax::DialectKind::Heading { .. } => "heading",
        syntax::DialectKind::Fence { .. } => "fence",
        syntax::DialectKind::InlineCode => "inline_code",
        syntax::DialectKind::Anchor { .. } => "anchor",
        syntax::DialectKind::Wikilink { .. } => "wikilink",
        syntax::DialectKind::Embed { .. } => "embed",
        syntax::DialectKind::Callout { .. } => "callout",
        syntax::DialectKind::Task { .. } => "task",
        syntax::DialectKind::Table => "table",
        syntax::DialectKind::Comment => "comment",
    }
}

#[test]
#[ignore = "dormant until rung 1: syntax::parse is todo!() — remove this attribute when the rung lands"]
fn parser_finds_at_least_planted() {
    let constructs = KNOWN_CONSTRUCTS
        .iter()
        .map(|id| {
            (
                (*id).to_owned(),
                ConstructRate {
                    file_rate: 1.0,
                    per_file: 4.0,
                },
            )
        })
        .collect();
    let profile = Profile {
        name: "assert-dense".into(),
        provenance: "≥-planted assertion".into(),
        files: 25,
        size: SizeDist {
            median_bytes: 3000,
            sigma: 0.8,
            max_bytes: 65_536,
        },
        constructs,
        pathology: bench::profile::Pathology {
            unterminated_fence_rate: 0.2,
        },
        unicode: bench::profile::Unicode { cjk_rate: 0.2 },
    };
    let recipe = Recipe::new(profile, 20_260_718, None);
    for index in 0..recipe.files {
        let gf = bench::generator::generate_file(&recipe, index);
        let mut parsed = Inventory::new();
        for node in syntax::parse(&gf.text) {
            parsed.bump(kind_id(&node.kind));
        }
        for (id, planted) in &gf.inventory.0 {
            if id == "fence_unterminated" {
                continue; // subset of fence; kind-level check covers it
            }
            assert!(
                parsed.count(id) >= *planted,
                "{}: parser found {} `{id}`, generator planted {planted}",
                gf.rel_path,
                parsed.count(id),
            );
        }
    }
}
