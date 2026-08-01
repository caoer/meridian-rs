//! The reserved-path spellings, held together across the two crates that mirror
//! them — asserted from the door, which is what breaks when they drift.
//!
//! # Why a test and not a shared constant
//! `policy` is I/O-free and knows nothing of disk; `fs` walks disk and knows nothing
//! of rules. Neither may depend on the other, so each names the reserved paths for
//! itself and the doc comments say "mirrors". A mirror maintained only by a comment
//! drifts the first time someone renames one side, and the drift is silent in the
//! worst direction: the door would go on protecting a page nobody writes any more,
//! while the page that IS the enforcement substrate quietly became an ordinary file
//! any write could delete.
//!
//! `wire-serve` is the crate that holds both — it reads the artifact through
//! `policy::armed` and resolves paths through `fs::domain` — so the agreement is
//! asserted where its violation would land.

/// The armed-set artifact: the INDEX's successor under tag registration. Its
/// spelling is load-bearing twice — `policy::binding`'s integrity floor refuses its
/// deletion at the door, and `fs::domain` keeps its parent walkable whatever the
/// ignore list says. Those are two different crates protecting one page.
#[test]
fn the_armed_rules_artifact_has_one_spelling() {
    assert_eq!(
        policy::armed::ARMED_RULES_PATH,
        fs::domain::ARMED_RULES_PATH,
        "the artifact's path is mirrored in `policy::armed` and `fs::domain`, and they \
         have drifted — the door and the walk now disagree about which page is the \
         enforcement substrate"
    );
}

#[test]
fn the_index_and_the_marker_have_one_spelling_each() {
    assert_eq!(
        policy::RESERVED_INDEX_PATH,
        fs::domain::RESERVED_INDEX_PATH,
        "the attested INDEX's path is mirrored in `policy::binding` and `fs::domain`"
    );
    assert_eq!(
        policy::ATTESTED_MARKER_PATH,
        fs::domain::ATTESTED_MARKER_PATH,
        "the once-armed marker's path is mirrored in `policy::binding` and `fs::domain`"
    );
}
