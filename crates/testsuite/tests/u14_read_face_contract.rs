//! The read face's own contract, over the U0 corpus — the gate that replaces
//! the retired Go-parity golden replays.
//!
//! The retired goldens fed captured requests in a selector grammar the engine
//! removed, and the captures cannot be re-taken against a redesigned contract.
//! The byte pins are converted into contracts over the engine's own stated
//! laws, per case, and no shared assertion is loosened. The corpus itself is
//! not a capture — it is 11 hand-built input documents — so it stays, and this
//! file replays every one of them: the engine agrees with itself, which is the
//! property a consumer actually depends on. The TOON goldens re-pin the
//! rendered face.

use wire_map::facts::{ReadFact, anchor_rows, read_facts, resolve_selector, section_content};

/// Every corpus document, as (doc-dir name, relative markdown path).
fn corpus_docs() -> Vec<(String, String)> {
    let dir = testsuite::parity_dir().join("corpus");
    let mut out = Vec::new();
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("corpus dir {}: {e}", dir.display()))
        .map(|e| {
            e.expect("dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    for name in names {
        let doc_dir = dir.join(&name);
        if !doc_dir.is_dir() {
            continue;
        }
        collect_md(&doc_dir, &doc_dir, &mut out, &name);
    }
    out
}

fn collect_md(
    base: &std::path::Path,
    at: &std::path::Path,
    out: &mut Vec<(String, String)>,
    doc: &str,
) {
    let Ok(entries) = std::fs::read_dir(at) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_md(base, &p, out, doc);
        } else if p.extension().is_some_and(|x| x == "md") {
            let rel = p
                .strip_prefix(base)
                .expect("under base")
                .to_string_lossy()
                .into_owned();
            out.push((doc.to_owned(), rel));
        }
    }
}

fn facts_of(doc: &str, rel: &str) -> (String, Vec<ReadFact>) {
    let path = testsuite::parity_dir().join("corpus").join(doc).join(rel);
    let raw = String::from_utf8(
        std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())),
    )
    .expect("corpus is UTF-8");
    let document = model::build(raw.clone(), syntax::parse(&raw));
    let facts = read_facts(&wire_map::project_toc(&document), raw.as_bytes());
    (raw, facts)
}

/// Every address the read face publishes resolves back to the row that
/// published it. The negative half is what makes it bite: resolving each
/// published address must land on its own row, not merely on some row.
#[test]
fn every_published_address_resolves_back_to_its_own_row() {
    let mut rows_checked = 0_u32;
    for (doc, rel) in corpus_docs() {
        let (_, facts) = facts_of(&doc, &rel);
        for (i, f) in facts.iter().enumerate() {
            let ctx = format!("{doc}/{rel} row {i}");
            let sel = if let Some(id) = &f.anchor {
                wire::ReadSel::Anchor { anchor: id.clone() }
            } else {
                wire::ReadSel::Hpath {
                    hpath: f.hpath.clone(),
                }
            };
            let hit = resolve_selector(&facts, &sel)
                .unwrap_or_else(|| panic!("{ctx}: its own published address resolves"));
            assert_eq!(
                (hit.n.as_str(), hit.span),
                (f.n.as_str(), f.span),
                "{ctx}: the address published by a row must resolve to THAT row"
            );
            rows_checked += 1;
        }
    }
    assert!(
        rows_checked >= 40,
        "the corpus must actually exercise this: {rows_checked} rows"
    );
}

/// The dewey plane is positional, and not even unique: `# One` → `1`, then
/// `### Jump Three` (a level skip) and `## Back Two` both land on `1.1`.
/// `DeweyCounter` numbers by the level sequence it walks, so a document that
/// skips a level and later pops back reuses an ordinal; `resolve_selector`
/// hands back the first. This is why `write::canonical_selector` canonicalizes
/// a dewey to the row's structural address before a pin records it — an
/// ordinal is not a stable name for a section.
#[test]
fn a_dewey_ordinal_resolves_first_match_and_is_not_a_unique_address() {
    // Half 1: resolving an ordinal always lands on the FIRST row carrying it.
    for (doc, rel) in corpus_docs() {
        let (_, facts) = facts_of(&doc, &rel);
        for f in facts.iter().filter(|f| f.depth > 0) {
            let hit = resolve_selector(&facts, &wire::ReadSel::Dewey { n: f.n.clone() })
                .unwrap_or_else(|| panic!("{doc}/{rel}: dewey {} resolves", f.n));
            let first = facts
                .iter()
                .find(|c| c.n == f.n)
                .expect("at least the row we came from");
            assert_eq!(
                hit.span, first.span,
                "{doc}/{rel}: dewey {} resolves to the FIRST row carrying it",
                f.n
            );
        }
    }

    // Half 2: the collision is real, so half 1 is not vacuous.
    let (_, facts) = facts_of("level-jumps", "corpus/level-jumps.md");
    let mut seen: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();
    for f in facts.iter().filter(|f| f.depth > 0) {
        *seen.entry(f.n.as_str()).or_default() += 1;
    }
    let collided = seen.values().filter(|&&c| c > 1).count();
    assert!(
        collided > 0,
        "level-jumps must still exercise the ordinal collision that makes a dewey \
         unfit as a pinned address; if this document changed, the finding needs \
         re-measuring rather than deleting: {seen:?}"
    );
}

/// Sections-mode content is the RAW face and self-verifying: the bytes served
/// are exactly the span bytes, and the word count published beside them is
/// recomputed over those same bytes (the op-owner ruling the read arm carries).
/// Corpus-wide, so no document is exempt.
#[test]
fn section_content_is_the_raw_span_and_its_words_are_counted_over_it() {
    let mut checked = 0_u32;
    for (doc, rel) in corpus_docs() {
        let (raw, facts) = facts_of(&doc, &rel);
        for f in &facts {
            let content = section_content(f, raw.as_bytes());
            let text = String::from_utf8_lossy(&content);
            if let Some(cs) = f.content_span {
                assert_eq!(
                    content,
                    raw.as_bytes()[usize::try_from(cs.0).expect("span fits")
                        ..usize::try_from(cs.1).expect("span fits")]
                        .to_vec(),
                    "{doc}/{rel}: a heading section serves its content span VERBATIM"
                );
                // The toc row's `words` and the sections row's recomputed
                // `words` describe the SAME bytes, so they must agree — the
                // invariant that keeps a caller from seeing one count in the
                // map and another in the content.
                assert_eq!(
                    wire_map::gotext::fields_count(&text) as u64,
                    f.words,
                    "{doc}/{rel}: the row's word count is the count over the bytes it serves"
                );
            }
            checked += 1;
        }
    }
    assert!(checked >= 40, "sections exercised: {checked}");
}

/// The two planes stay disjoint corpus-wide: no anchor fact reaches a `toc`
/// consumer, and no heading fact reaches an `anchors` consumer.
#[test]
fn the_heading_and_anchor_planes_stay_disjoint_over_the_whole_corpus() {
    for (doc, rel) in corpus_docs() {
        let (_, facts) = facts_of(&doc, &rel);
        assert!(
            wire_map::facts::toc_rows(&facts, &[])
                .iter()
                .all(|f| f.anchor.is_none() && f.depth > 0),
            "{doc}/{rel}: the heading plane carries headings and NOTHING else"
        );
        assert!(
            anchor_rows(&facts, &[])
                .iter()
                .all(|f| f.anchor.is_some() && f.depth == 0 && f.hpath.is_empty()),
            "{doc}/{rel}: the anchor plane carries anchors, and they publish no \
             heading address (U14)"
        );
    }
}

/// Assert the absence where a law died: `wire::Node` carried `hpath_text` — a
/// sanitized joined address on a machine surface — and it was removed ("no
/// string address forms in machine surfaces"). The enriched v3 `extract` face
/// must never grow it back. A tripwire: a restoration would pass every other
/// gate silently, because adding a field breaks nothing.
#[test]
fn the_extract_face_never_republishes_a_joined_string_address() {
    let (raw, _) = facts_of("basic", "corpus/basic.md");
    let doc = model::build(raw.clone(), syntax::parse(&raw));
    let body = wire_serve::read::extract(&doc, &wire::Path("corpus/basic.md".into()), None, true);
    let json = serde_json::to_string(&body).expect("serializes");
    assert!(
        json.contains("\"hpath\""),
        "the enriched extract face publishes the SEGMENT address"
    );
    assert!(
        !json.contains("hpath_text"),
        "ZT decision 14 / U14: `hpath_text` — the sanitized joined address — is \
         retired from the machine surface. The node already carries `hpath` as \
         segments, which is the address that round-trips; a joined string beside \
         it is a second, lossy spelling of one address. Restoring it needs a \
         ruling, not a field."
    );
}
