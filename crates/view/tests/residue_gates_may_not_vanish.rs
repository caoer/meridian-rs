//! Gates in `board_residue_gates.rs` may not vanish silently.
//!
//! `board_residue` sits at permanent zero by ruling — a never-moved counter is
//! indistinguishable from a disconnected one except while tests run. Those
//! five tests ARE the instrument.
//!
//! Arm 1: Cargo.toml explicit `[[test]]` targets (file delete/rename fails build).
//! Arm 2: this file pins armed `#[test]` names by reading rustc-compiled bytes
//! (catches in-place disarm). Hand-authored name list, never derived from the
//! scanned file; keyed per name, not total.

use std::path::PathBuf;

/// Hand-authored. Never derived from the scanned file.
const RESIDUE_GATES: &[&str] = &[
    "residue_is_zero_over_a_healthy_corpus_and_the_board_is_not_empty",
    "a_verdict_less_row_vanishes_from_board_and_is_disclosed_by_residue",
    "the_synthetic_row_sits_between_classified_rows",
    "the_two_public_projections_agree_on_every_key_over_real_documents",
    "predicate_is_one_definition_shared_by_both_projections",
];

const GATES_FILE: &str = "tests/board_residue_gates.rs";

#[derive(Debug, PartialEq, Eq)]
struct TestFn {
    name: String,
    ignored: bool,
}

/// Every `#[test]` in `text` (attributes accumulate until an item consumes
/// them). Attribute lines only — doc-comment mentions of `#[ignore]` are prose.
fn test_fns(text: &str) -> Vec<TestFn> {
    let mut found = Vec::new();
    let mut is_test = false;
    let mut ignored = false;

    for line in text.lines() {
        let line = line.trim_start();
        if line.starts_with("#[") {
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

/// Each pinned gate present and armed (catches in-place disarm).
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

/// Exact pin set — no unpinned gate may arrive unprotected.
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

/// Control for `test_fns` itself (shared mechanism cannot check itself).
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

    // Prose about #[ignore] is not a disarm.
    assert_eq!(
        test_fns("/// mentions #[ignore] in prose\n#[test]\nfn kept() {}\n"),
        vec![TestFn {
            name: "kept".into(),
            ignored: false
        }],
        "a doc comment naming the attribute is documentation, not a disarm"
    );

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
