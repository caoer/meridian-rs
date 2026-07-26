//! **U11 / C-3 — `resolve_linkpath` PEELS AND REFUSES** (address-grammar § 5.1,
//! row F5: *"a raw `&str` with a `:` head reaching `resolve_linkpath` directly →
//! `None`, asserted by a test"*).
//!
//! **The redden, quoted.** On the base commit `3aad9de1` this file failed
//! exactly as FINDING 03 predicts:
//!
//! ```text
//! ---- finding_03_a_rooted_linkpath_must_not_answer_the_ambient_file ----
//!   left: Some("notes.md")
//!  right: None
//! ```
//!
//! …while the no-slash CONTROL passed on the same run — *"the bug needs a slash
//! after the root prefix, which every real cross-root ref has."* The shortest
//! test case is the one that behaves correctly, which is how this survived.
//!
//! It stays as a permanent guard rather than being deleted with the fix: the
//! defect is INSIDE the owner, not at a door, so U10's retype reached this
//! function without fixing it and nothing but this assertion stops a future
//! caller passing a raw `&str` from reproducing it silently.

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
