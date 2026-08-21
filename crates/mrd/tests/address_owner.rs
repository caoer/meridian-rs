//! Finding 18 — ONE address owner: the pin plane and the decoration plane must turn a ref
//! spelling into the SAME document. This test crate is where the two planes meet: `view::walk`
//! computes the color `mrd walk` / `mrd status` render, and
//! `wire_serve::read::page_decorations` mints the `@fp` tone word a read carries.

use std::collections::BTreeMap;

/// The corpus: `a/b.md` carries the pinned section; `b.md` is a decoy with the same basename
/// and a same-titled section of different bytes, so resolving to it is measurably wrong. The
/// ref is a heading (`a/bSection`), which is what a pin fingerprint covers.
fn corpus() -> (model::Docs, String) {
    let target = "# Page\n\n## Section\n\nthe pinned body\n";
    let decoy = "# Page\n\n## Section\n\nDIFFERENT bytes entirely\n";

    let mut docs: model::Docs = BTreeMap::new();
    docs.insert("a/b.md".to_string(), doc(target));
    docs.insert("b.md".to_string(), doc(decoy));

    // Fingerprint minted the way the engine mints it, so the pin is genuinely green on `a/b.md`.
    let fingerprint = live_fingerprint(&docs, "a/b.md", "Page/Section");
    let src = format!(
        "# Src\n\ndraws from [[a/b#^section]]\n\n{}\n",
        lock_block("a/b#Page/Section", &fingerprint)
    );
    docs.insert("src.md".to_string(), doc(&src));
    (docs, fingerprint)
}

/// The two documents must be measurably different at the pinned selector, or the gate above
/// proves nothing: a fixture whose planes hash the same bytes agrees no matter which document
/// each one picked.
#[test]
fn the_fixture_can_tell_the_two_documents_apart() {
    let (docs, pinned) = corpus();
    let decoy_fp = live_fingerprint(&docs, "b.md", "Page/Section");
    assert_ne!(
        pinned, decoy_fp,
        "`a/b.md#Section` and `b.md#Section` must fingerprint differently"
    );
}

fn doc(raw: &str) -> std::sync::Arc<model::Document> {
    std::sync::Arc::new(model::build(raw.to_string(), syntax::parse(raw)))
}

/// One R4 (`version: 2`) pin, hand-written in the exact bytes `lock::render` emits, so this
/// gate depends on the readers own parser and not on the writer that produced the row.
fn lock_block(declared_ref: &str, fingerprint: &str) -> String {
    let (target, fragment) = match declared_ref.split_once('#') {
        Some((t, f)) => (t, f),
        None => (declared_ref, ""),
    };
    let object = target.strip_suffix(".md").unwrap_or(target);
    let path = if fragment.is_empty() {
        String::new()
    } else {
        fragment
            .split('/')
            .map(|seg| format!("\"{seg}\""))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "```meridian-lock\nversion: 2\npins:\n  - object: \"[[{object}]]\"\n    \
         hash: \"9ae3f1deadbeef\"\n    path: [{path}]\n    \
         fingerprint: \"{fingerprint}\"\n```"
    )
}

/// The live fingerprint of one selector in one corpus page — the same resolve + span + removals
/// the pin minter runs (`wire-serve/src/write.rs`), so the fixture cannot pin a token the
/// readers would not recompute.
fn live_fingerprint(docs: &model::Docs, path: &str, selector: &str) -> String {
    let sel = model::selector::Selector::parse(&format!("{path}#{selector}"));
    let (doc, target) = model::selector::resolve_selector(&sel, docs.get(path).map(|d| &**d))
        .expect("the selector resolves");
    let removals = syntax::anchor_removals(&doc.raw);
    model::fingerprint::fingerprint_span(doc, &target.span, &removals)
        .expect("the fixture target has content")
        .into_string()
}

/// F18 gate — the walk color and the `@fp` tone word are computed over the same document for
/// one ref. `a/b` names `a/b.md` and the pin is green there; a plane that resolves it to
/// `b.md` instead measures the decoy bytes and mints `@red`.
#[test]
fn the_pin_plane_and_the_decoration_plane_resolve_one_ref_to_one_document() {
    let (docs, _) = corpus();
    let index = view::read_face::corpus_index(&docs);

    // Plane 1 — the address the walk/status color rests on.
    let items = view::read_face::page_lock_items_in_corpus(
        "src.md",
        docs.get("src.md").expect("src"),
        &index,
        &docs,
    );
    assert_eq!(items.len(), 1, "one declared pin, one row");
    assert_eq!(
        items[0].to_path, "a/b.md",
        "the pin plane resolves `a/b` to the path it names"
    );

    let colors = view::walk::lock_pin_colors(&docs);
    assert_eq!(colors.len(), 1, "one lock row in the corpus");
    let walk_tone = view::walk::color_tone(&colors[0].color);
    assert_eq!(
        walk_tone,
        "green",
        "the pin is live against `a/b.md`: {}",
        view::walk::color_label(&colors[0].color)
    );

    // Plane 2 — the address the `@fp` decoration mints its tone over.
    let decorations = wire_serve::read::page_decorations(&index, &docs, "src.md");
    assert_eq!(
        decorations.len(),
        1,
        "the body link addressing the pinned block is decorated"
    );
    let token = decorations.values().next().expect("the minted token");

    assert!(
        token.contains(&format!("@{walk_tone}.")),
        "the tone word carries the SAME verdict the walk measured ({walk_tone}), \
         so both planes hashed one document: token {token}"
    );
}

/// The precedence itself, stated once at the owner: every plane that turns a spelling into a
/// document calls `CorpusIndex::resolve_ref`, and the three rules are exact key, key + `.md`,
/// then linkpath parity.
#[test]
fn the_address_owner_answers_the_subpath_spelling_with_the_subpath_document() {
    let (docs, _) = corpus();
    let index = view::read_face::corpus_index(&docs);

    let corpus = model::RootedCorpus::ambient(&docs);
    let no_mounts = addr::MountSet::default();

    assert_eq!(
        index
            .resolve_ref("a/b", "src.md", &corpus, &no_mounts)
            .path(),
        Some("a/b.md"),
        "a full-path ref without its extension names the path it spells"
    );
    assert_eq!(
        index
            .resolve_ref("a/b.md", "src.md", &corpus, &no_mounts)
            .path(),
        Some("a/b.md"),
        "the same address, spelled with the extension"
    );
    assert_eq!(
        index.resolve_ref("b", "src.md", &corpus, &no_mounts).path(),
        Some("b.md"),
        "the bare basename still names the root file"
    );
    assert_eq!(
        index
            .resolve_ref("nowhere/at/all", "src.md", &corpus, &no_mounts)
            .path(),
        None,
        "an address the corpus does not hold is None — unresolved stays first-class"
    );

    // The wrapper the read face hands the walk plane adds exactly one thing: the
    // unresolvable spelling comes back verbatim, to be rendered red.
    assert_eq!(
        view::read_face::resolve_to_path("a/b", "src.md", &index, &docs),
        "a/b.md"
    );
    assert_eq!(
        view::read_face::resolve_to_path("nowhere/at/all", "src.md", &index, &docs),
        "nowhere/at/all"
    );
}
