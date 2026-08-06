//! Root-level path arm of `resolve_linkpath` subpath narrowing.
//!
//! Subpath `a/b` matches paths ending in `a/b.md`: nested (`/a/b.md` suffix) or
//! vault-root (`lower == qualified`, no leading separator). Nested-only fixtures
//! leave the root arm unguarded; `resolve_ref` rule 2 short-circuits before it.
//! Corpus here: both `guide/setup.md` and `setup.md`.

use model::CorpusIndex;

/// In neither candidate's directory: the source-relative preference is silent,
/// so the unqualified pick falls to the tie-break and answers `setup.md`.
const FROM: &str = "notes/plan.md";

/// The root-level qualified path and its bare-basename collision.
const QUALIFIED: &str = "guide/setup.md";
const BARE: &str = "setup.md";

fn index() -> CorpusIndex {
    let mut index = CorpusIndex::new();
    let empty = model::build(String::new(), syntax::parse(""));
    for p in [QUALIFIED, BARE] {
        index.insert(p, &empty);
    }
    index
}

/// `[[guide/setup]]` must address the qualified path. Without the
/// `lower == qualified` arm the narrowed set is empty and resolution degrades
/// to the bare-basename pick, answering `setup.md`.
#[test]
fn a_root_level_subpath_beats_its_bare_basename_collision() {
    let index = index();
    let qualified = index.resolve_linkpath("guide/setup", FROM);
    assert_eq!(
        qualified,
        Some(QUALIFIED.to_owned()),
        "the subpath qualifier selects the path that IS `guide/setup.md`, at the vault root"
    );
    assert_ne!(
        qualified,
        index.resolve_linkpath("setup", FROM),
        "and it beats the fallback — if these agreed, the assertion above would prove nothing"
    );
}

/// Anti-vacuity control: the guard above is only evidence while the
/// unqualified fallback answers the other document. Fails first if the
/// fixture stops being ambiguous.
#[test]
fn the_bare_basename_fallback_answers_the_other_document() {
    assert_eq!(
        index().resolve_linkpath("setup", FROM),
        Some(BARE.to_owned()),
        "the collision is real and the unqualified pick answers `setup.md` — the wrong document \
         for `[[guide/setup]]`, which is what makes the qualifier load-bearing"
    );
}
