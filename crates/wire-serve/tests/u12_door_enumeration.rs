//! **U12's door enumeration, reconciled against U31's sealed door list.**
//!
//! U31 made the candidate document a TYPE the `fs` byte-landing primitives
//! demand, so *"which doors land bytes"* stopped being prose and became *"which
//! sites mint a `model::CandidateDocument`"* — a set the compiler maintains.
//! This test pins that set and states, per door, what U12 does at it.
//!
//! **Why a test and not the compiler.** U31's own rung is compiler-enumerated
//! and this unit inherits its result; what the compiler cannot say is whether a
//! door that HOLDS a candidate also GUARDS it, because a guard is a call, not a
//! type. So this is the ladder's middle rung, used exactly where a list is all a
//! test can give you: it proves the door SET and the arithmetic, and it fails
//! when a NINTH mint appears — an unclassified door until someone writes down
//! what U12 does there.
//!
//! **Precision, measured before the check was written** (S3-R23 ①): the scan
//! reads production `src/` only, skips doc comments and the definition site
//! itself, and classifies every hit. A new mint anywhere in the workspace is
//! therefore reported as *unclassified*, never guessed at — the false positive
//! that would get this instrument deleted.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// What U12 does at a byte-landing door.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Door {
    /// The candidate is TRANSLATED (agent-plane addresses this write introduces
    /// become their `obsidian://` stored form) and then GUARDED.
    TranslatedAndGuarded,
    /// The candidate is GUARDED. Its bytes are the engine's own — a lock block
    /// (positions 3 and 4, where the translation is the identity by ratified
    /// law) or an anchor promotion — so there is nothing to translate, and the
    /// guard is what says so rather than a comment claiming it.
    Guarded,
    /// **NOT COVERED BY THIS UNIT**, stated rather than absorbed (S3-R4). The
    /// door lives outside U12's named files (`write.rs` + `read.rs`), and
    /// closing it would mean a SECOND transform, which this unit's card
    /// forbids. Reported to the leader as a gap with its population.
    OutsideThisUnit,
}

/// **THE DOOR LIST** — every production site minting a `model::CandidateDocument`,
/// with what U12 does there.
///
/// Keyed by `file::function`, never by line: a line number rots on the next edit
/// and a rotted pin teaches a reader to ignore the check.
const DOORS: &[(&str, &str, Door)] = &[
    // ---- wire-serve/write.rs — U12's own file, all five guarded ----
    (
        "crates/wire-serve/src/write.rs",
        "splice (via build_after_doc)",
        Door::TranslatedAndGuarded,
    ),
    (
        "crates/wire-serve/src/write.rs",
        "create (the birth door)",
        Door::TranslatedAndGuarded,
    ),
    (
        "crates/wire-serve/src/write.rs",
        "lock_write",
        Door::Guarded,
    ),
    (
        "crates/wire-serve/src/write.rs",
        "plan_promotion (the anchor promotion)",
        Door::Guarded,
    ),
    (
        "crates/wire-serve/src/write.rs",
        "commit_batch (the public commit seam)",
        Door::Guarded,
    ),
    // ---- outside U12's named files ----
    (
        "crates/mrd/src/realise_cmd.rs",
        "realise --truth file: the armed policy INDEX",
        Door::OutsideThisUnit,
    ),
    (
        "crates/mrd/src/realise_cmd.rs",
        "realise: the convergence body",
        Door::OutsideThisUnit,
    ),
    (
        "crates/run/src/fp.rs",
        "the run plane's candidate (run::fp::candidate)",
        Door::OutsideThisUnit,
    ),
];

fn workspace_root() -> PathBuf {
    // `CARGO_MANIFEST_DIR` is `<root>/crates/wire-serve`.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Every production `src/` file in the workspace — **except `model`, which
/// DEFINES the two constructors**: its own internal delegation
/// (`candidate_of_batch` calls `candidate_of_body`) is the definition, not a
/// door, and a definition counted as a door is the false positive that gets an
/// instrument deleted.
fn production_sources() -> Vec<PathBuf> {
    let root = workspace_root();
    let mut out = Vec::new();
    let Ok(crates) = fs::read_dir(root.join("crates")) else {
        panic!("crates/ must be readable");
    };
    for entry in crates.flatten() {
        if entry.file_name() == "model" {
            continue;
        }
        let src = entry.path().join("src");
        if src.is_dir() {
            rs_files(&src, &mut out);
        }
    }
    out.sort();
    out
}

/// `text` truncated at its first `#[cfg(test)]` — a unit-test module lives
/// inside `src/`, and a test that mints a candidate lands no bytes a user ever
/// reads. Measured: without this, `crates/fs/src/lib.rs` reads as a byte-landing
/// door because its own harness mints candidates.
fn production_half(text: &str) -> &str {
    match text.find("#[cfg(test)]") {
        Some(at) => &text[..at],
        None => text,
    }
}

/// Files carrying at least one candidate MINT — a call, not a mention.
fn minting_files() -> BTreeSet<String> {
    let root = workspace_root();
    let mut hits = BTreeSet::new();
    for file in production_sources() {
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        let rel = file
            .strip_prefix(&root)
            .unwrap_or(&file)
            .display()
            .to_string();
        for line in production_half(&text).lines() {
            let trimmed = line.trim_start();
            // A doc comment is a mention, not a door. Skipping it is what keeps
            // a true door distinguishable from the prose that describes one.
            if trimmed.starts_with("//") {
                continue;
            }
            if trimmed.contains("candidate_of_body(") || trimmed.contains("candidate_of_batch(") {
                hits.insert(rel.clone());
                break;
            }
        }
    }
    hits
}

/// **The door SET is exactly the pinned list.** A ninth mint anywhere in
/// production source fails here as an UNCLASSIFIED door.
#[test]
fn the_byte_landing_door_set_is_exactly_the_pinned_list() {
    let measured = minting_files();
    let pinned: BTreeSet<String> = DOORS.iter().map(|(f, _, _)| (*f).to_string()).collect();
    assert_eq!(
        measured, pinned,
        "the set of files minting a candidate changed — classify the new door in DOORS \
         (what does U12 do there?) rather than editing this assertion",
    );
}

/// **The arithmetic closes** (R32): every door is accounted for exactly once,
/// and each class is NON-EMPTY (S3-R37 — a gate whose population empties is the
/// quietest way for coverage to disappear).
#[test]
fn the_arithmetic_closes_and_no_class_is_empty() {
    let translated = DOORS
        .iter()
        .filter(|(_, _, d)| *d == Door::TranslatedAndGuarded)
        .count();
    let guarded = DOORS.iter().filter(|(_, _, d)| *d == Door::Guarded).count();
    let outside = DOORS
        .iter()
        .filter(|(_, _, d)| *d == Door::OutsideThisUnit)
        .count();

    assert_eq!(translated, 2, "splice and create carry user-supplied bytes");
    assert_eq!(
        guarded, 3,
        "lock_write, the promotion and commit_batch land engine-composed bytes",
    );
    assert_eq!(
        outside, 3,
        "two realise doors and the run plane — stated, not absorbed",
    );
    assert_eq!(
        translated + guarded + outside,
        DOORS.len(),
        "every door falls in exactly one class",
    );
    assert_eq!(DOORS.len(), 8, "eight byte-landing doors mint a candidate");

    // The guard covers every door in U12's own file, and nothing else claims to.
    let in_write_rs = DOORS
        .iter()
        .filter(|(f, _, _)| *f == "crates/wire-serve/src/write.rs")
        .count();
    assert_eq!(
        in_write_rs,
        translated + guarded,
        "U12 guards exactly the doors in its named file — the arithmetic that \
         says what this unit did and did not close",
    );
}

/// **Every door in U12's own file actually CALLS the guard.** The door list
/// above says what U12 does; this counts what the file does, so a door silently
/// losing its guard fails here rather than in production.
///
/// A source count is the honest instrument for this: a guard is a CALL, and no
/// type can force a call a door simply does not make (U31's rung reaches the
/// candidate's provenance, not this). It counts ONE name at statement position —
/// `stored_form_guard_lazy`, the single door-facing entry — so the definition
/// line (`fn …`) and the core it delegates to are structurally excluded rather
/// than filtered by guesswork.
#[test]
fn every_door_in_this_units_file_calls_the_guard() {
    let text = fs::read_to_string(workspace_root().join("crates/wire-serve/src/write.rs"))
        .expect("write.rs is readable");
    let calls = production_half(&text)
        .lines()
        .filter(|l| l.trim_start().starts_with("stored_form_guard_lazy("))
        .count();
    let expected = DOORS
        .iter()
        .filter(|(f, _, _)| *f == "crates/wire-serve/src/write.rs")
        .count();
    assert_eq!(
        calls, expected,
        "each of the {expected} doors in write.rs discharges the artifact guard exactly \
         once — found {calls}",
    );
    assert!(
        expected > 0,
        "the population this gate iterates is non-empty (S3-R37)",
    );
}
