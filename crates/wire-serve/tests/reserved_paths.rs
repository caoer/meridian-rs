//! Reserved-path spellings mirrored across `policy` and `fs`, asserted here.
//!
//! No shared constant: `policy` is I/O-free, `fs` knows nothing of rules, and
//! neither may depend on the other — each spells the paths for itself.
//! `wire-serve` uses both crates, so drift is asserted where it would land.

/// The armed-set artifact's spelling is load-bearing twice: `policy::binding`
/// refuses its deletion, `fs::domain` keeps its parent walkable.
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

/// The once-armed marker: the pivot both the write door and the reaction
/// feeder turn on. Drift would let the two hosts disagree on armed state.
#[test]
fn the_once_armed_marker_has_one_spelling() {
    assert_eq!(
        policy::ATTESTED_MARKER_PATH,
        fs::domain::ATTESTED_MARKER_PATH,
        "the once-armed marker's path is mirrored in `policy::binding` and `fs::domain`"
    );
}

/// The artifact and the marker are the whole protected substrate; the retired
/// INDEX must not survive as a reserved path under any name.
#[test]
fn the_retired_index_is_not_still_reserved() {
    let reserved: Vec<&str> = fs::domain::RESERVED_PATHS.to_vec();
    assert!(
        !reserved.iter().any(|p| p.contains("INDEX")),
        "`conventions/INDEX.md` is no longer engine substrate, but a reserved path \
         still names it: {reserved:?}"
    );
}
