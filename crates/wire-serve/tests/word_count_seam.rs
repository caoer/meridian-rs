//! One word count per rev, every face — the counting seam (session
//! `12-04-f2-mrd-integration`, card `two-faces-word-count`: F-S4 two faces
//! disagreeing at one rev, D-USER r2 F3 the toc banner double-count).
//!
//! The law under test: a `words` number is always counted over the RAW bytes
//! of the range it names, and never assembled by summing other rows. The
//! banner names the FILE, so it counts the file; a toc row names a section
//! subtree, so it counts that subtree. Summing subtree rows counts every
//! descendant once per ancestor level — the ~2x lie a reader budgets against.

use wire::{Path as WPath, ResponseBody};
use wire_serve::read::{NO_DECORATIONS, ReadParams, composed_read};

/// Three nested levels: `C`'s words live inside `B`'s subtree span, which
/// lives inside `A`'s. A sum over the rows counts `C` three times.
const NESTED: &str = "---\ntype: note\n---\n\n# A\n\none two three\n\n## B\n\nfour five six\n\n### C\n\nseven eight nine\n";

fn read_toc(raw: &str) -> ResponseBody {
    let doc = model::build(raw.to_string(), syntax::parse(raw));
    composed_read(
        &doc,
        &WPath("deep.md".into()),
        &wire::Root("r".into()),
        &ReadParams {
            display_path: Some("deep.md".into()),
            ..ReadParams::default()
        },
        &NO_DECORATIONS,
    )
    .expect("the toc serves")
}

/// `wc -w`: the number a reader cross-checks the banner against.
fn disk_words(raw: &str) -> u64 {
    raw.split_whitespace().count() as u64
}

/// The banner names the file, so it counts the file ONCE — no descendant
/// lands in it twice. Pre-fix this read `24` on a 15-word file.
#[test]
fn the_banner_counts_the_file_once() {
    let ResponseBody::Read { words_total, .. } = read_toc(NESTED) else {
        panic!("composed read answers a Read body");
    };
    assert_eq!(
        words_total,
        disk_words(NESTED),
        "the banner is the file's own word count, never a sum of subtree rows"
    );
}

/// The rows keep their subtree meaning — the fix is the banner's derivation,
/// not the row's. Stated as its own law so a later "make the rows sum to the
/// banner" change has to argue with this test.
#[test]
fn a_toc_row_counts_its_own_subtree() {
    let ResponseBody::Read { toc, .. } = read_toc(NESTED) else {
        panic!("composed read answers a Read body");
    };
    let rows = toc.expect("toc mode carries rows");
    let words: Vec<u64> = rows.iter().map(|r| r.words).collect();
    assert_eq!(
        words,
        vec![13, 8, 3],
        "each row counts its heading-excluded, subtree-inclusive content span"
    );
}

/// The same section, asked for as a body, reports the same number the map
/// reported — one fact, two faces.
#[test]
fn the_section_face_and_the_toc_face_report_one_number() {
    let ResponseBody::Read { toc, .. } = read_toc(NESTED) else {
        panic!("composed read answers a Read body");
    };
    let map_words = toc.expect("toc rows")[1].words;
    let doc = model::build(NESTED.to_string(), syntax::parse(NESTED));
    let body = composed_read(
        &doc,
        &WPath("deep.md".into()),
        &wire::Root("r".into()),
        &ReadParams {
            sections: Some(vec![wire::ReadSel::parse("A/B")]),
            display_path: Some("deep.md".into()),
            ..ReadParams::default()
        },
        &NO_DECORATIONS,
    )
    .expect("the section serves");
    let ResponseBody::Read { sections, .. } = body else {
        panic!("composed read answers a Read body");
    };
    let served = sections.expect("sections mode carries rows");
    assert_eq!(
        served[0].words, map_words,
        "§A/B has one word count, whichever face is asked"
    );
}
