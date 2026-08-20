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

/// The machinery floor must COVER the engine's own reserved paths (card
/// create-door-machinery-containment). Both reserved pages live under
/// `meridian/`, so the create door's deny list already refuses a birth onto
/// them — this pins that agreement rather than leaving it a coincidence. If
/// someone rehomes the armed artifact or the marker out of `meridian/`, this
/// fails and the floor gets revisited in the same change.
#[test]
fn the_machinery_floor_covers_every_reserved_path() {
    for reserved in fs::domain::RESERVED_PATHS {
        let head = reserved.split('/').next().unwrap();
        assert!(
            wire_serve::write::MACHINERY_DIRS
                .iter()
                .any(|dir| head.eq_ignore_ascii_case(dir)),
            "the reserved path `{reserved}` does not lie under any machinery directory \
             {:?} — the create door would admit a birth onto engine substrate",
            wire_serve::write::MACHINERY_DIRS
        );
    }
}

/// The floor's members are the four the contract names, and nothing has been
/// quietly added: each entry widens what every birth lane refuses, so growth
/// belongs in `docs/run-plane.md` § the machinery floor first.
#[test]
fn the_machinery_floor_has_exactly_the_contracted_members() {
    assert_eq!(
        wire_serve::write::MACHINERY_DIRS,
        [".git", ".meridian", "meridian", "receipts"].as_slice(),
        "the create door's deny list drifted from `docs/run-plane.md` § the machinery \
         floor and `docs/wire-contract.md` § A.3"
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
