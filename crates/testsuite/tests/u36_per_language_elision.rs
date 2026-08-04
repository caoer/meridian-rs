//! **U36 — elision is PER-LANGUAGE, derived from a property, never from a list.**
//!
//! A namespace answers *"is this ours?"*. Elision answers *"should a reader see
//! it?"*. `lock::is_meridian_lang` served both, so `mrd read MERIDIAN.md`
//! printed the prose and dropped every mount block the user had authored —
//! byte-identically to a config that declares nothing (U6, pinned in
//! `meridian_md.rs`).
//!
//! The property that separates them is **"does the ENGINE EMIT this block's
//! bytes?"** — `lock::is_engine_emitted`, derived from the registered canonical
//! writers (`lock::EngineEmitted`).
//!
//! # The gate is BOTH directions, and one alone is insufficient (S3-R17)
//!
//! 1. a NEW engine-emitted block, added **without touching the elision
//!    predicate**, IS elided;
//! 2. a language the engine only **PARSES** is **NOT** elided — no matter which
//!    crate names it.
//!
//! (2) is what makes this a law rather than a namespace test in disguise, and it
//! is the half that reddens: on the pre-U36 tree `meridian-mount` is elided, and
//! under a *mention*-keyed derivation it is elided again, because `crates/config`
//! names `meridian-mount` in a `pub const`. Both reddens are recorded in the U36
//! card.

use render::{Header, RenderJob, Renderer, SectionRow, TextRenderer};
use wire_map::facts::{ReadFact, read_facts};

// ── Direction 1's subject: a NEW engine-emitted block ──────────────────────
//
// This type exists ONLY here, and it is what makes direction 1 a real test
// rather than a restatement of the shipped set. It is engine-emitted for
// exactly one reason: it owns a canonical writer and declares it beside itself.
// Registering it touches NEITHER `lock::is_engine_emitted` NOR
// `render::ElideBy::matches` — that untouched-ness IS the gate.

/// A brand-new engine block, invented by this test.
struct TestJournal {
    ts: &'static str,
}

impl lock::EngineEmitted for TestJournal {
    const LANG: &'static str = "meridian-u36testjournal";

    fn emit_canonical(&self) -> String {
        format!("```{}\n- ts: {}\n```", Self::LANG, self.ts)
    }
}

lock::engine_emits!(TestJournal);

/// A `meridian-*` language with **no** writer anywhere — the third-language
/// case (neither mount, nor tool, nor an engine block). Named only as a string,
/// exactly as an unclaimed reserved language reaches the engine.
const UNCLAIMED_LANG: &str = "meridian-u36unclaimed";

fn build(raw: &str) -> (model::Document, Vec<ReadFact>) {
    let doc = model::build(raw.to_string(), syntax::parse(raw));
    let facts = read_facts(&wire_map::project_toc(&doc), raw.as_bytes());
    (doc, facts)
}

/// Render every section of `raw` through the production render face and return
/// the concatenated section content.
fn rendered_content(raw: &str) -> String {
    let (doc, facts) = build(raw);
    // U14: a row echoes the selector STRUCTURALLY, so the selectors are built
    // alongside the rows and borrowed by them.
    let sels: Vec<wire::ReadSel> = facts
        .iter()
        .map(|fact| wire::ReadSel::Hpath {
            hpath: fact.hpath.clone(),
        })
        .collect();
    let rows: Vec<SectionRow<'_>> = facts
        .iter()
        .zip(&sels)
        .map(|(fact, sel)| SectionRow { sel, fact })
        .collect();
    let rendered = TextRenderer::with_meridian_elision()
        .render(
            &doc,
            &RenderJob::Sections {
                header: Header {
                    display_path: "~/MERIDIAN.md",
                    file_rev: &doc.root.node_rev.0,
                    words_total: 0,
                    decorations: &render::NO_DECORATIONS,
                },
                rows: &rows,
                notice: None,
            },
        )
        .expect("renders");
    rendered
        .sections
        .iter()
        .map(|s| s.content.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// One page carrying, side by side: the engine's shipped block, a NEW
/// engine-emitted block, the two languages the engine only parses, an unclaimed
/// reserved language, and an ordinary fence.
fn mixed_page() -> String {
    // FIXTURE REPAIR ONLY (U14): U8 deleted the `objects:` plane — R4 carries
    // the blob hash per pin row — so `Lock::set_object` no longer exists and
    // this test binary did not compile at `u8-rekey`. Rebuilt as an R4 pin row;
    // this test is about fence LANGUAGES, so the lock's contents are only
    // required to render as an engine-emitted block.
    let mut l = lock::Lock::new();
    l.upsert_pin(lock::PinEntry::new(
        "corpus/x",
        "9ae3f1deadbeef",
        lock::Selector::Path(vec!["Roots".into()]),
        "fp1.green.0000000000000000",
    ));
    let lock_block = lock::render(&l);
    let new_engine_block = lock::EngineEmitted::emit_canonical(&TestJournal { ts: "2026-07-26" });
    format!(
        "# Roots\n\nprose the reader keeps\n\n{lock_block}\n\n{new_engine_block}\n\n\
         ```{mount}\nname: field-notes\npath: /Users/Shared/projects/field-notes\nkind: vault\nvault: field-notes\n```\n\n\
         ```{tool}\nname: t\nkind: mcp\n```\n\n\
         ```{unclaimed}\nwhatever: 1\n```\n\n\
         ```rust\nfn main() {{}}\n```\n",
        mount = config::MOUNT_LANG,
        tool = config::TOOL_LANG,
        unclaimed = UNCLAIMED_LANG,
    )
}

/// **Direction 1.** A new engine-emitted block is elided, and the predicate was
/// never touched to make it so.
///
/// What this kills: a derivation that only knows the languages that existed when
/// it was written — the enumerated list's actual failure mode (it silently omits
/// new members, #8 §1).
#[test]
fn a_new_engine_emitted_block_is_elided_without_touching_the_predicate() {
    let raw = mixed_page();
    let out = rendered_content(&raw);

    assert!(
        lock::is_engine_emitted(<TestJournal as lock::EngineEmitted>::LANG),
        "registering a canonical writer is what makes a language engine-emitted"
    );
    assert!(
        !out.contains("u36testjournal"),
        "the new engine block's fence survived the render face: {out}"
    );
    assert!(
        !out.contains("2026-07-26"),
        "the new engine block's BODY survived the render face: {out}"
    );
    // The shipped engine block still elides — direction 1 did not cost the
    // behavior the rule was ratified for.
    assert!(
        !out.contains(lock::LANG),
        "the lock block survived the render face: {out}"
    );
}

/// **Direction 2 — the half that makes this a law.** A language the engine only
/// PARSES is NOT elided, no matter which crate names it.
///
/// What this kills: (a) the pre-U36 namespace predicate, which elides these; and
/// (b) a derivation keyed on *mention* — `crates/config` names both languages in
/// `pub const`s (`config::MOUNT_LANG`, `config::TOOL_LANG`) and merely PARSES
/// them, so a mention-keyed predicate elides them and reproduces the defect
/// wearing derivation clothes.
#[test]
fn a_parsed_only_language_is_not_elided_however_it_is_named() {
    let raw = mixed_page();
    let out = rendered_content(&raw);

    for lang in [config::MOUNT_LANG, config::TOOL_LANG] {
        assert!(
            !lock::is_engine_emitted(lang),
            "`{lang}` has no canonical writer — the engine only parses it"
        );
        assert!(
            lock::is_meridian_lang(lang),
            "`{lang}` IS in the reserved namespace — reservation stays uniform, \
             and that is exactly why readership had to stop riding on it"
        );
        assert!(
            out.contains(lang),
            "the render face dropped `{lang}`, which the USER authored: {out}"
        );
    }
    assert!(
        out.contains("field-notes"),
        "the declared root's own bytes reach the reader: {out}"
    );
    assert!(
        out.contains("kind: vault"),
        "the whole authored block reaches the reader, not just its fence: {out}"
    );
}

/// **Gate 3 — the third-language case gets its RULE and its FIXTURE in the same
/// motion** (S3-R14 item 3(a)).
///
/// A `meridian-*` block that is neither mount, nor tool, nor an engine block.
///
/// **The rule: readership follows AUTHORSHIP.** The engine did not write those
/// bytes — no canonical writer claims the language — so a human did, and a human's
/// bytes are shown. An unclaimed reserved language renders.
///
/// That is what makes `config`'s SKIP of the same block safe: `config` cannot
/// parse a language it has no grammar for, and before U36 the block was skipped
/// AND elided, so it vanished twice over. Now the skip drops it from the mount
/// table while the reader still sees it — nothing disappears silently.
#[test]
fn an_unclaimed_reserved_language_renders_and_config_skips_it() {
    let raw = mixed_page();
    let out = rendered_content(&raw);

    assert!(
        lock::is_meridian_lang(UNCLAIMED_LANG),
        "the reservation covers it — the namespace is a prefix law (#8 §1)"
    );
    assert!(
        !lock::is_engine_emitted(UNCLAIMED_LANG),
        "no canonical writer claims it, so the engine does not emit it"
    );
    assert!(
        out.contains(UNCLAIMED_LANG),
        "an unclaimed reserved language renders — the reader sees what the \
         engine did not write: {out}"
    );

    // The `config` half of the same rule: skipped, never refused, and the skip
    // is now visible on the render face rather than compounding into a silence.
    let page = format!(
        "---\ntype: {ty}\nversion: 1\n---\n\n\
         ```{mount}\nname: a\npath: /x\nkind: git-folder\n```\n\n\
         ```{unclaimed}\nwhatever: 1\n```\n",
        ty = config::CONFIG_TYPE,
        mount = config::MOUNT_LANG,
        unclaimed = UNCLAIMED_LANG,
    );
    let cfg = config::parse(&page, std::path::Path::new("~/MERIDIAN.md"))
        .expect("an unclaimed reserved language does not refuse the config");
    assert_eq!(
        cfg.mounts()
            .iter()
            .map(|m| m.name.as_str())
            .collect::<Vec<_>>(),
        ["a"],
        "the unclaimed block contributes no mount, and does not disturb the one that parsed"
    );
}

/// The raw face is untouched: elision is render-face only (byte pin #4), and M1's
/// *"no invented placeholder bytes"* still stands — U36 changed WHICH languages
/// are elided, never whether elision invents bytes.
#[test]
fn the_raw_face_still_carries_every_reserved_block_verbatim() {
    let raw = mixed_page();
    for lang in [
        lock::LANG,
        <TestJournal as lock::EngineEmitted>::LANG,
        config::MOUNT_LANG,
        config::TOOL_LANG,
        UNCLAIMED_LANG,
    ] {
        assert!(
            raw.contains(&format!("```{lang}")),
            "the raw face carries `{lang}` verbatim"
        );
    }
    let out = rendered_content(&raw);
    assert!(
        !out.contains("NOTICE"),
        "no marker bytes were invented for the elided blocks: {out}"
    );
}

/// The namespace reservation did NOT move (U36 bound): `is_meridian_lang` still
/// answers the prefix question for every language, including the ones that now
/// render.
///
/// This is the discrimination the unit exists to establish, asserted directly:
/// **every** engine-emitted language is reserved, and **not** every reserved
/// language is engine-emitted. The second half is the new law.
#[test]
fn reservation_is_uniform_and_readership_is_not() {
    let reserved_and_emitted = [lock::LANG, <TestJournal as lock::EngineEmitted>::LANG];
    let reserved_not_emitted = [config::MOUNT_LANG, config::TOOL_LANG, UNCLAIMED_LANG];

    for lang in reserved_and_emitted {
        assert!(lock::is_meridian_lang(lang), "`{lang}` is reserved");
        assert!(lock::is_engine_emitted(lang), "`{lang}` is engine-emitted");
    }
    for lang in reserved_not_emitted {
        assert!(lock::is_meridian_lang(lang), "`{lang}` is reserved");
        assert!(
            !lock::is_engine_emitted(lang),
            "`{lang}` is reserved but NOT engine-emitted — one owner, two purposes, \
             now two predicates"
        );
    }

    // A language outside the namespace is neither, and the two predicates agree
    // there — the acceptance half, without which "not elided" is satisfied by a
    // predicate that returns false for everything.
    for lang in ["rust", "yaml", "meridian", ""] {
        assert!(!lock::is_meridian_lang(lang), "`{lang}` is not reserved");
        assert!(
            !lock::is_engine_emitted(lang),
            "`{lang}` is not engine-emitted"
        );
    }
}
