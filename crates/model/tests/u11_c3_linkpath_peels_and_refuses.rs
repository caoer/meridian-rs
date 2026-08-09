//! `resolve_linkpath` peels and refuses (address-grammar § 5.1 F5): a raw
//! `&str` with a `:` head must return `None`, never the ambient same-basename
//! file.

use model::CorpusIndex;

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

/// No-slash control: both spellings must converge on one answer (C-4).
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
