//! One counter for a section's words — the render face may not mint a second
//! one (session `12-04-f2-mrd-integration`, card `two-faces-word-count`,
//! F-S4: two faces, one rev, two counts).
//!
//! `words` is a fact about the section's CONTENT, counted over its raw bytes.
//! The rendered projection is allowed to show fewer bytes than the content
//! has — elision drops engine-emitted blocks (U4b), decoration adds tokens —
//! so counting the projection would publish a number that changes with how
//! the section is displayed while its rev stands still. That is the drift
//! this test forbids: the served `words` is the same number the structured
//! read plane serves, whatever the renderer did to the text.

use render::{Header, RenderJob, Renderer, SectionRow, ToonRenderer};
use wire_map::facts::{ReadFact, read_facts};

/// An engine-emitted block, invented here (the u36 idiom): its bytes are
/// elided from rendered content, so a projection-derived count sees fewer
/// words than the section holds.
struct TestLedger;

impl lock::EngineEmitted for TestLedger {
    const LANG: &'static str = "meridian-wordcountledger";

    fn emit_canonical(&self) -> String {
        format!("```{}\n- seven eight nine ten\n```", Self::LANG)
    }
}

lock::engine_emits!(TestLedger);

const RAW: &str = "# A\n\none two three\n\n```meridian-wordcountledger\n- seven eight nine ten\n```\n\nfour five six\n";

fn build(raw: &str) -> (model::Document, Vec<ReadFact>) {
    let doc = model::build(raw.to_string(), syntax::parse(raw));
    let facts = read_facts(&wire_map::project_toc(&doc), raw.as_bytes());
    (doc, facts)
}

/// The rendered sections face publishes the content's count, not the
/// projection's. Pre-fix the elided block's words vanished from the number
/// while the raw plane kept them — the same section, two counts, one rev.
#[test]
fn the_rendered_section_publishes_the_content_word_count() {
    let (doc, facts) = build(RAW);
    let sel = wire::ReadSel::Hpath {
        hpath: facts[0].hpath.clone(),
    };
    let rows = vec![SectionRow {
        sel: &sel,
        fact: &facts[0],
    }];
    let rendered = ToonRenderer::with_meridian_elision()
        .render(
            &doc,
            &RenderJob::Sections {
                header: Header {
                    display_path: "a.md",
                    file_rev: &doc.root.node_rev.0,
                    fingerprint: "fp",
                    words_total: 0,
                    decorations: &render::NO_DECORATIONS,
                },
                rows: &rows,
                notice: None,
            },
        )
        .expect("the section renders");
    let raw_words = wire_map::facts::section_words(&facts[0], doc.raw.as_bytes());
    assert!(
        !rendered.sections[0].content.contains("wordcountledger"),
        "the engine block is elided from the projection — the premise of this test"
    );
    assert_eq!(
        rendered.sections[0].words, raw_words,
        "the section's word count is a fact about its content, not about how \
         much of it the renderer chose to show"
    );
}
