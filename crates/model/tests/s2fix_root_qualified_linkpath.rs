//! R44/fix3 — root-level path arm of `resolve_linkpath` subpath narrowing.
//!
//! Subpath `a/b` matches paths ending in `a/b.md`: nested (`/a/b.md` suffix) or
//! vault-root (`lower == qualified`, no leading separator). Nested-only fixtures
//! leave the root arm unguarded; `resolve_ref` rule 2 short-circuits before it.
//! Corpus here: both `guide/setup.md` and `setup.md`.

use model::CorpusIndex;

/// The linking page: in neither candidate's directory, so the source-relative
/// preference is silent and the pick falls to shortest-then-lexicographic — the
/// tie-break that answers `setup.md` when the qualifier is not applied.
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

/// **The guard.** `[[guide/setup]]` in a corpus holding both `guide/setup.md`
/// and `setup.md` addresses the qualified path. Revert the `lower == qualified`
/// arm and the narrowed set is empty — no path ends in `/guide/setup.md`, the
/// root-level one having no leading separator — so the resolution degrades to
/// the bare-basename set and answers `setup.md`: a different document, under one
/// address.
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

/// **The anti-vacuity CONTROL.** The guard is only evidence while the fallback
/// answers something ELSE: same index, same pick function, same `from`, the
/// qualifier removed. If this fixture ever stops being ambiguous — `setup.md`
/// dropped, the basename key changed, the tie-break reordered — the guard would
/// pass for a reason it does not test. This fails LOUD first.
#[test]
fn the_bare_basename_fallback_answers_the_other_document() {
    assert_eq!(
        index().resolve_linkpath("setup", FROM),
        Some(BARE.to_owned()),
        "the collision is real and the unqualified pick answers `setup.md` — the wrong document \
         for `[[guide/setup]]`, which is what makes the qualifier load-bearing"
    );
}
