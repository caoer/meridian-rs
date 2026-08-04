//! U4b final validation: the elision law against a lock the ENGINE actually
//! wrote — B's U11 `lock_write` (engine-sole-writer, EOF placement, CAS +
//! flock) — not a static fixture. The engine-written `meridian-lock` block
//! renders ELIDED on the render/readText face
//! ([`TextRenderer::with_meridian_elision`], predicate =
//! `lock::is_meridian_lang`) and rides the raw read/cat face VERBATIM
//! (byte pin #4: the raw face never routes through render).

use render::{Header, RenderJob, Renderer, SectionRow, TextRenderer};
use wire::{NodeRev, Path as WPath};
use wire_map::facts::{read_facts, resolve_selector};
use wire_serve::write::{LockWriteArgs, lock_write};

const PAGE: &str = "---\ntitle: Pinning\n---\n# Claims\n\nthe claim body ^c1\n";

#[test]
fn engine_written_lock_elided_on_render_face_verbatim_on_raw() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("page.md"), PAGE).expect("fixture");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());

    // Pin the claim with a REAL minted fingerprint and let the ENGINE write
    // the lock through the guarded U11 path.
    let doc0 = fs::load(&root, std::path::Path::new("page.md")).expect("load");
    let mut l = lock::Lock::new();
    l.upsert_pin(lock::PinEntry::new(
        "page",
        "9ae3f1deadbeef",
        // R4's anchor pin: a `^id` as the SOLE path element — block grain, never
        // widened to the host section.
        lock::Selector::Path(vec!["^c1".to_string()]),
        &model::fingerprint::fingerprint(&doc0, &doc0.root)
            .expect("fixture target has content")
            .into_string(),
    ));
    let out = lock_write(
        &root,
        None,
        &LockWriteArgs {
            id: None,
            path: WPath("page.md".into()),
            lock: l.clone(),
            actor: Some("engine:test".into()),
            now: Some("2026-07-24T12:00:00Z".into()),
            if_root: None,
            if_file_rev: NodeRev(doc0.root.node_rev.0.clone()),
            dry: false,
        },
    )
    .expect("the engine writes the lock");
    assert!(out.created, "birth at EOF");

    // Re-load the engine-written page from disk.
    let doc = fs::load(&root, std::path::Path::new("page.md")).expect("re-load");
    let block = lock::render(&l);
    assert!(
        doc.raw.contains(&block),
        "the engine wrote the canonical block bytes"
    );

    let facts = read_facts(&wire_map::project_toc(&doc), doc.raw.as_bytes());
    let fact = resolve_selector(&facts, &wire::ReadSel::parse("Claims")).expect("Claims resolves");

    // RAW read/cat face (the content-span bytes): VERBATIM — pin #4. The
    // EOF-placed lock sits inside the last section's subtree-inclusive span.
    let cs = fact.content_span.as_ref().expect("content span");
    let raw_face = &doc.raw
        [usize::try_from(cs.0).expect("start fits")..usize::try_from(cs.1).expect("end fits")];
    assert!(
        raw_face.contains(&block),
        "raw face carries the engine-written lock verbatim: {raw_face}"
    );

    // RENDER face, production configuration: the lock is gone, the claim
    // body stays, and no pin payload leaks.
    let rows = [SectionRow {
        sel: &wire::ReadSel::parse("Claims"),
        fact,
    }];
    let header = Header {
        display_path: "$S/page.md",
        file_rev: &doc.root.node_rev.0,
        words_total: 0,
        decorations: &render::NO_DECORATIONS,
    };
    let rendered = TextRenderer::with_meridian_elision()
        .render(
            &doc,
            &RenderJob::Sections {
                header,
                rows: &rows,
                notice: None,
            },
        )
        .expect("renders");
    assert!(
        !rendered.text.contains("meridian-lock"),
        "engine-written lock elided from the render face: {}",
        rendered.text
    );
    assert!(
        !rendered.text.contains("fp1."),
        "no pin payload leaks into the render face: {}",
        rendered.text
    );
    assert!(
        rendered.text.contains("the claim body ^c1"),
        "claim body renders: {}",
        rendered.text
    );
}
