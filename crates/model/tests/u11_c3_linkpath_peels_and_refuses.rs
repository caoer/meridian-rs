//! U11 / C-3 — `resolve_linkpath` peels and refuses (address-grammar § 5.1 F5):
//! a raw `&str` with a `:` head reaching this function must return `None`, never
//! the ambient same-basename file. Permanent guard inside the owner.

use model::CorpusIndex;

/// FINDING 03's input, verbatim: a body wikilink carrying a `root:` prefix
/// basenames onto the AMBIENT root's same-basename file — a wrong SUCCESS.
#[test]
fn finding_03_a_rooted_linkpath_must_not_answer_the_ambient_file() {
    let mut index = CorpusIndex::new();
    let empty = model::build(String::new(), syntax::parse(""));
    index.insert("notes.md", &empty);

    assert_eq!(
        index.resolve_linkpath("sessions:24-01-retro/notes.md", "claim.md"),
        None,
        "a `:`-bearing head is a programming error at this seam (address-grammar C-3); \
         it must never resolve to the ambient root's same-basename file",
    );
}

/// The no-slash CONTROL — FINDING 03 needs the slash, and C-4 requires the two
/// spellings to converge on ONE answer.
#[test]
fn finding_03_control_the_no_slash_spelling_answers_the_same() {
    let mut index = CorpusIndex::new();
    let empty = model::build(String::new(), syntax::parse(""));
    index.insert("notes.md", &empty);

    assert_eq!(
        index.resolve_linkpath("sessions:notes.md", "claim.md"),
        None,
        "one address, one answer (C-4): the slashed and unslashed spellings converge",
    );
}
