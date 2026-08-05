//! U4b fixtures: `meridian-*` block elision on the RENDER face ONLY.
//!
//! The page carries a REAL canonical lock block (built by `lock::render` — the
//! engine-sole-writer byte form, #8 §3) plus a `meridian-journal` block and an ordinary
//! `rust` fence, mirroring the U0 `meridian-block` corpus doc. Byte pin #4: the raw
//! read/cat face carries `meridian-*` blocks VERBATIM and never routes through render;
//! elision lives behind [`ToonRenderer::with_meridian_elision`].
//!
//! **U36:** the predicate is `lock::is_engine_emitted` — derived from the registered
//! canonical writers, not from the `meridian-*` namespace. So the lock elides (the engine
//! writes it) while `meridian-journal`, which no writer claims, renders. Reservation
//! stays uniform (`lock::is_meridian_lang`, #8 §1); readership is per-language.
//!
//!

use render::{Header, RenderJob, RenderedSection, Renderer, SectionRow, ToonRenderer};
use wire_map::facts::{ReadFact, read_facts, resolve_selector};

fn build(raw: &str) -> (model::Document, Vec<ReadFact>) {
    let doc = model::build(raw.to_string(), syntax::parse(raw));
    let facts = read_facts(&wire_map::project_toc(&doc), raw.as_bytes());
    (doc, facts)
}

/// A populated lock in the canonical engine-written byte form.
fn lock_block() -> String {
    // FIXTURE REPAIR ONLY (U14): U8 replaced the `objects:` plane and the
    // `declared_ref` row with the R4 shape — `object` + `hash` + a `path`
    // ARRAY selector — so this fixture no longer compiled. Rebuilt in the
    // current shape; this test is about fence-language ELISION, so all it
    // needs from the lock is that it renders as the engine's block.
    let mut l = lock::Lock::new();
    l.upsert_pin(lock::PinEntry::new(
        "corpus/other",
        "9ae3f1deadbeef",
        lock::Selector::Path(vec!["Design".to_string()]),
        &format!("fp1.span2.b3.{}", "ab".repeat(32)),
    ));
    lock::render(&l)
}

fn fixture_page() -> String {
    format!(
        "# Config\n\n{}\n\nbody after lock block\n\n# Code\n\n```rust\nfn main() {{}}\n```\n\n```meridian-journal\n- ts: 2026-01-01\n```\n",
        lock_block()
    )
}

fn render_sections(
    doc: &model::Document,
    rows: &[SectionRow<'_>],
    r: ToonRenderer,
) -> render::Rendered {
    let header = Header {
        display_path: "$S/x.md",
        file_rev: "r",
        fingerprint: "fp",
        words_total: 0,
        decorations: &render::NO_DECORATIONS,
    };
    r.render(
        doc,
        &RenderJob::Sections {
            header,
            rows,
            notice: None,
        },
    )
    .expect("renders")
}

fn content_slice<'a>(raw: &'a str, fact: &ReadFact) -> &'a str {
    let cs = fact.content_span.as_ref().expect("content span");
    &raw[usize::try_from(cs.0).expect("start fits")..usize::try_from(cs.1).expect("end fits")]
}

/// The U4b acceptance fixture: a page with a REAL `meridian-lock` block
/// renders ELIDED on the render/readText face, VERBATIM on the raw face.
#[test]
fn real_lock_elided_on_render_face_verbatim_on_raw() {
    let block = lock_block();
    let raw = fixture_page();
    let (doc, facts) = build(&raw);
    let config =
        resolve_selector(&facts, &wire::ReadSel::parse("Config")).expect("Config resolves");
    let code = resolve_selector(&facts, &wire::ReadSel::parse("Code")).expect("Code resolves");

    // RAW face (read/cat = the content span bytes): VERBATIM — pin #4.
    assert!(
        content_slice(&raw, config).contains(&block),
        "raw face carries the canonical lock bytes verbatim"
    );
    assert!(content_slice(&raw, code).contains("```meridian-journal"));

    // RENDER face, production configuration: every meridian-* block gone.
    let rows = [
        SectionRow {
            sel: &wire::ReadSel::parse("Config"),
            fact: config,
        },
        SectionRow {
            sel: &wire::ReadSel::parse("Code"),
            fact: code,
        },
    ];
    let out = render_sections(&doc, &rows, ToonRenderer::with_meridian_elision());
    assert!(
        !out.text.contains("meridian-lock"),
        "lock block elided: {}",
        out.text
    );
    // U36 re-based this assertion, and the reason is a MEASUREMENT: nothing in
    // the engine emits a `meridian-journal` block. The language appears in this
    // fixture and in one `lock` unit test and NOWHERE in `src/` — the receipt
    // journal is a page of `^receipt` LINES (`crates/receipt/src/journal.rs`),
    // not a fenced block. So it has no canonical writer, elision is now derived
    // from having one, and a reserved language the engine does not write renders.
    //
    // It therefore stands here as the THIRD-LANGUAGE case (neither mount, nor
    // tool, nor an engine block), whose rule is: readership follows authorship.
    // The engine-emitted direction is carried by `meridian-lock` above, and by a
    // freshly registered writer in
    // `crates/testsuite/tests/u36_per_language_elision.rs`.
    assert!(
        out.text.contains("meridian-journal"),
        "a reserved language with NO canonical writer renders — reservation is \
         uniform, readership is per-language (U36): {}",
        out.text
    );
    assert!(
        !out.text.contains("fp1.span2.b3."),
        "no lock payload leaks into the rendered face"
    );
    assert!(out.text.contains("body after lock block"), "{}", out.text);
    assert!(
        out.text.contains("```rust\nfn main() {}\n```"),
        "non-engine fences render verbatim: {}",
        out.text
    );

    // The rendered word recount excludes the elided bytes; the FACT keeps
    // the raw-face count (addressing is untouched by render).
    let rendered_config: &RenderedSection = &out.sections[0];
    assert!(
        rendered_config.words < config.words,
        "render-face words ({}) recounted below the raw fact ({})",
        rendered_config.words,
        config.words
    );
}

/// The golden-pinned default stays INERT against the REAL canonical lock
/// bytes (the U0 gates construct `ToonRenderer::default()`).
#[test]
fn default_renderer_emits_real_lock_verbatim() {
    let block = lock_block();
    let raw = fixture_page();
    let (doc, facts) = build(&raw);
    let config =
        resolve_selector(&facts, &wire::ReadSel::parse("Config")).expect("Config resolves");
    let rows = [SectionRow {
        sel: &wire::ReadSel::parse("Config"),
        fact: config,
    }];
    let out = render_sections(&doc, &rows, ToonRenderer::default());
    assert!(
        out.sections[0].content.contains(&block),
        "default() is inert: canonical lock bytes ride the rendered content verbatim"
    );
}

/// Fully-elided ≠ empty (coordinator ruling): a page whose ONLY body is its
/// lock block still renders the page's non-lock STRUCTURE — the TOON head's
/// row for the section, and the section's body marker — never a bare empty
/// string. The raw face carries the block verbatim regardless (pin #4).
///
/// U15 made this claim harder to lose rather than softer: the section survives
/// as a declared ROW carrying its own `rev`, so "the read served this section
/// and it rendered to nothing" is now a fact in the head, not an inference
/// from a banner's presence.
#[test]
fn all_lock_page_renders_structure_not_empty() {
    let block = lock_block();
    let raw = format!("# Pins\n\n{block}\n");
    let (doc, facts) = build(&raw);
    let fact = resolve_selector(&facts, &wire::ReadSel::parse("Pins")).expect("resolves");

    // Raw face: verbatim.
    assert!(content_slice(&raw, fact).contains(&block));

    // Render face: block gone, structure stays.
    let rows = [SectionRow {
        sel: &wire::ReadSel::parse("Pins"),
        fact,
    }];
    let out = render_sections(&doc, &rows, ToonRenderer::with_meridian_elision());
    assert!(!out.text.is_empty(), "never a bare empty string");
    assert!(
        out.text
            .contains(&format!("\n  \"1\",Pins,{},0,", fact.sec_rev)),
        "the head still declares the section, with its rev: {}",
        out.text
    );
    assert!(
        out.text.contains("== 1 =="),
        "and the body marker is the surviving structure: {}",
        out.text
    );
    assert!(
        !out.text.contains("meridian-lock"),
        "the lock itself is elided: {}",
        out.text
    );
    assert_eq!(out.sections[0].words, 0, "no words survive elision");
}

/// An elided block takes its own line with it (fence-to-fence bytes plus
/// one trailing newline); the surrounding bytes stay untouched.
#[test]
fn elision_swallows_the_block_line() {
    let raw = format!("# H\n\nbefore\n\n{}\n\nafter\n", lock_block());
    let (doc, facts) = build(&raw);
    let fact = resolve_selector(&facts, &wire::ReadSel::parse("H")).expect("resolves");
    let rows = [SectionRow {
        sel: &wire::ReadSel::parse("H"),
        fact,
    }];
    let out = render_sections(&doc, &rows, ToonRenderer::with_meridian_elision());
    assert_eq!(
        out.sections[0].content, "\nbefore\n\n\nafter\n",
        "block + its own line removed, surrounding bytes untouched"
    );
}
