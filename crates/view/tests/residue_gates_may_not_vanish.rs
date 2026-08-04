//! **The gates in `board_residue_gates.rs` may not vanish silently.**
//!
//! `board_residue` is a disclosed counter whose production value is permanently
//! ZERO by ruling — both routes to a NULL verdict are unreachable, and the
//! ruling is what keeps them that way. That is exactly what makes it fragile:
//!
//! > A never-moved disclosed counter is indistinguishable from a DISCONNECTED
//! > one except while its tests run.
//!
//! So those five tests are not evidence ABOUT the instrument — **they ARE the
//! instrument.** Delete them, rename them, or `#[ignore]` them and the counter
//! silently becomes a decoration while every gate stays green, **because a
//! deleted test and a passing test emit the same bytes to a gate.**
//!
//! # Two arms, and what they share
//!
//! **ARM 1 is not in this file and is not a test.** `crates/view/Cargo.toml`
//! declares `board_residue_gates` (and this file) as EXPLICIT `[[test]]`
//! targets. Deleting or renaming either file is then a cargo target-resolution
//! failure — the build stops before a test runner exists, so nothing about it
//! can be ignored, filtered, or skipped. That is the arm that survives this file
//! being disarmed.
//!
//! **ARM 2 is this file**, and it covers what a manifest cannot see: the tests
//! being emptied out IN PLACE while the file keeps its name. It reads the real
//! bytes rustc compiles and pins which functions actually carry `#[test]`.
//!
//! **What the arms share, named per all-hands 26/44:** both rest on
//! `crates/view/Cargo.toml` naming the files. A maintainer who deletes the
//! `[[test]]` entries defeats arm 1 and can then delete the files. That is the
//! residual, and it is deliberate — the objective is that the tests cannot
//! vanish SILENTLY, not that they cannot be removed. Removing them now costs an
//! edit to a manifest and an edit to a file whose entire subject is that they
//! must not go, both of which a reader meets.
//!
//! # Why a text scan is a READ and not a RECOMPUTE (all-hands 38)
//!
//! #38's defect is a test that rebuilds the value production computes and then
//! compares its reconstruction against itself. This scan rebuilds nothing: the
//! bytes it reads are the bytes rustc reads, and the pinned list below is
//! hand-authored, never derived from the file it checks. A guard that recovered
//! its expected set by scanning the same file would agree with it by
//! construction and pass through any change at all.
//!
//! It keys by NAME and asserts per name, never by total — u12's door census
//! learned that the hard way, where a bag count let one door be deleted and
//! another duplicated with the gate green.

use std::path::PathBuf;

/// The gates that make `board_residue` a detector rather than a decoration.
///
/// **Hand-authored. Never derived from the file.** Each name is one claim about
/// the counter; the doc comment on each test in `board_residue_gates.rs` says
/// which. If you are here because this list disagreed with the file, the
/// question is not "how do I make it pass" — it is "which of these claims about
/// the residue counter am I dropping, and does the ruling still hold without
/// it".
const RESIDUE_GATES: &[&str] = &[
    "residue_is_zero_over_a_healthy_corpus_and_the_board_is_not_empty",
    "a_verdict_less_row_vanishes_from_board_and_is_disclosed_by_residue",
    "the_synthetic_row_sits_between_classified_rows",
    "the_two_public_projections_agree_on_every_key_over_real_documents",
    "predicate_is_one_definition_shared_by_both_projections",
];

const GATES_FILE: &str = "tests/board_residue_gates.rs";

/// One function that carries `#[test]`, and whether anything disarmed it.
#[derive(Debug, PartialEq, Eq)]
struct TestFn {
    name: String,
    ignored: bool,
}

/// Every `#[test]` function in `text`, read the way the compiler reads it:
/// attributes accumulate until an item consumes them.
///
/// Attribute lines only — a doc comment that MENTIONS `#[ignore]` is prose about
/// the mechanism, not the mechanism. Scanning prose would make the guard fire on
/// a file that explains itself, which is a guard that punishes documentation.
fn test_fns(text: &str) -> Vec<TestFn> {
    let mut found = Vec::new();
    let mut is_test = false;
    let mut ignored = false;

    for line in text.lines() {
        let line = line.trim_start();
        if line.starts_with("#[") {
            // `#[ignore]` and `#[ignore = "reason"]` are the same disarm.
            if line.starts_with("#[test]") {
                is_test = true;
            } else if line.starts_with("#[ignore") {
                ignored = true;
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("fn ") {
            if is_test {
                let name = rest
                    .split(['(', '<'])
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                found.push(TestFn { name, ignored });
            }
            is_test = false;
            ignored = false;
        }
    }
    found
}

fn gates_source() -> String {
    let file = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(GATES_FILE);
    std::fs::read_to_string(&file).unwrap_or_else(|e| {
        panic!(
            "{}: {e}\n\nTHE RESIDUE GATES ARE GONE. `board_residue` is a counter \
             whose production value is permanently zero; without these tests \
             nothing distinguishes it from a counter that is no longer wired to \
             anything. If this file moved, move this pin with it.",
            file.display()
        )
    })
}

/// Each pinned gate is present AND armed. Keyed by name, asserted per name.
///
/// This is the arm that catches disarming IN PLACE: a `#[test]` attribute
/// removed, a function renamed, a body left behind. All three keep the file at
/// its usual length and all three are silent to a gate.
#[test]
fn every_residue_gate_is_present_and_armed() {
    let armed = test_fns(&gates_source());

    for pinned in RESIDUE_GATES {
        let found = armed.iter().find(|t| t.name == *pinned);
        assert!(
            found.is_some(),
            "residue gate `{pinned}` is not an armed #[test] in {GATES_FILE} — \
             renamed, deleted, or its #[test] attribute was removed. A gate that \
             is not run is not a gate, and `board_residue` sits at zero either \
             way: that is the whole reason this pin exists."
        );
        assert!(
            !found.expect("checked above").ignored,
            "residue gate `{pinned}` is #[ignore]d. An ignored test and a passing \
             test emit the same bytes to a gate, so this is the one change that \
             turns the residue counter back into a decoration WITHOUT ANY \
             OUTPUT SAYING SO."
        );
    }
}

/// The set is EXACTLY the pinned set — no gate arrived unpinned.
///
/// Separate from the per-name arm above on purpose. That one answers "is every
/// claim still made"; this one answers "is every claim still pinned". A new gate
/// added to that file without a line here would be protected by nothing, and the
/// next person to delete it would meet no red at all.
#[test]
fn no_residue_gate_is_unpinned() {
    let armed = test_fns(&gates_source());

    for t in &armed {
        assert!(
            RESIDUE_GATES.contains(&t.name.as_str()),
            "`{}` is a #[test] in {GATES_FILE} that this pin does not know about. \
             Add it to RESIDUE_GATES — deliberately, with the claim it makes \
             written into its doc comment — or it is unprotected.",
            t.name
        );
    }
    assert_eq!(
        armed.len(),
        RESIDUE_GATES.len(),
        "the gates file carries {} armed tests and {} are pinned",
        armed.len(),
        RESIDUE_GATES.len()
    );
}

/// **The guard's own control.** `test_fns` is the machinery both arms above
/// share, so a defect in it disarms them together while they keep reporting
/// green — all-hands 44: a set of checks that share a mechanism cannot check
/// that mechanism.
///
/// This runs it over a hand-authored input whose answer is known by
/// construction, never over the file it guards. It is the reason the two
/// assertions above are evidence about `board_residue_gates.rs` rather than
/// evidence that a scanner returned something.
#[test]
fn the_scanner_itself_detects_each_disarm() {
    let armed = "#[test]\nfn kept() {}\n";
    assert_eq!(
        test_fns(armed),
        vec![TestFn {
            name: "kept".into(),
            ignored: false
        }],
        "baseline: an armed test is found"
    );

    assert_eq!(
        test_fns("#[test]\n#[ignore]\nfn kept() {}\n"),
        vec![TestFn {
            name: "kept".into(),
            ignored: true
        }],
        "#[ignore] is seen"
    );
    assert_eq!(
        test_fns("#[test]\n#[ignore = \"flaky\"]\nfn kept() {}\n"),
        vec![TestFn {
            name: "kept".into(),
            ignored: true
        }],
        "#[ignore = \"reason\"] is the same disarm and must not slip past a \
         literal `#[ignore]` match"
    );

    assert!(
        test_fns("fn kept() {}\n").is_empty(),
        "a bare fn is not a test — removing #[test] must read as a MISSING gate, \
         which is what makes the per-name arm catch a silent disarm"
    );
    assert_eq!(
        test_fns("#[test]\nfn renamed() {}\n")[0].name,
        "renamed",
        "the scanner keys on the name, so a rename cannot pass as the original"
    );

    // The one that matters for THIS file: prose about the mechanism is not the
    // mechanism. `board_residue_gates.rs` and this file both discuss `#[ignore]`
    // in their doc comments, and a scanner that read prose would fire on them.
    assert_eq!(
        test_fns("/// mentions #[ignore] in prose\n#[test]\nfn kept() {}\n"),
        vec![TestFn {
            name: "kept".into(),
            ignored: false
        }],
        "a doc comment naming the attribute is documentation, not a disarm"
    );

    // Attributes are consumed by the item that follows them: a disarm on one
    // test must not leak onto the next.
    let two = test_fns("#[test]\n#[ignore]\nfn first() {}\n\n#[test]\nfn second() {}\n");
    assert_eq!(
        two,
        vec![
            TestFn {
                name: "first".into(),
                ignored: true
            },
            TestFn {
                name: "second".into(),
                ignored: false
            }
        ],
        "an #[ignore] belongs to its own test and does not smear onto the next"
    );
}
