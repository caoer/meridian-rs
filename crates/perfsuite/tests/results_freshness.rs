//! The committed report must cover the whole registry.
//!
//! `claims.toml` is the population; `results/RESULTS.md` is the computed report over it.
//! Nothing used to fail when the two drifted: the registry grew by nine claims between
//! the last run and 2026-07-26 while the report kept rendering `3 PASS · 1 MEASURED · 11
//! UNTESTED` = 15. A tally over a SUBSET reads exactly like a tally over all of it unless
//! you count the registry yourself.
//!
//! These tests are the arm that carries the property (S3-R28(c)): a tally that merely
//! matches today proves nothing — the check that survives is the one that fails when the
//! registry MOVES. Add a claim and `covers_every_registered_claim` reddens until someone
//! recomputes the report.

use std::fs;
use std::path::{Path, PathBuf};

fn results_md() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("results")
        .join("RESULTS.md")
}

fn read_report() -> String {
    let path = results_md();
    fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read the committed report {}: {e}", path.display()))
}

/// Every registered claim id appears in the committed report. The report is
/// rendered from the registry, so an absent id means one thing only: the file
/// predates the row and has not been recomputed.
#[test]
fn committed_report_covers_every_registered_claim() {
    let claims = perfsuite::claims::load_bundled().expect("claims.toml is valid");
    let report = read_report();
    let missing: Vec<&str> = claims
        .iter()
        .map(|c| c.id.as_str())
        .filter(|id| !report.contains(&format!("`{id}`")))
        .collect();
    assert!(
        missing.is_empty(),
        "{} of {} registered claims are absent from {} — the report predates the registry \
         and its tally silently covers a subset. Recompute it: \
         `cargo bench -p perfsuite && cargo run -p perfsuite --bin bench-report`. Missing: {}",
        missing.len(),
        claims.len(),
        results_md().display(),
        missing.join(", "),
    );
}

/// The report's derived tally counts the whole registry. This is the half a
/// human reads: a tally that does not name its population reads the same
/// whether it covered every claim or ten fewer.
#[test]
fn committed_report_tally_covers_the_whole_registry() {
    let claims = perfsuite::claims::load_bundled().expect("claims.toml is valid");
    let report = read_report();
    let tally = report
        .lines()
        .find(|l| l.starts_with("**Tally:**"))
        .unwrap_or_else(|| panic!("no derived tally line in {}", results_md().display()));
    let expected = format!("over **{} claims registered**", claims.len());
    assert!(
        tally.contains(&expected),
        "the tally must state the live registry population ({} claims), got: {tally}",
        claims.len(),
    );
}

/// The report carries the run stamp its own renderer emits (date + commit).
/// A computed artifact with no recompute stamp is a hand-typed one that took
/// longer to write — and an unchecked stamp is decoration.
#[test]
fn committed_report_carries_a_recompute_stamp() {
    let report = read_report();
    let stamp = report
        .lines()
        .find(|l| l.starts_with("Run "))
        .unwrap_or_else(|| panic!("no `Run …` stamp line in {}", results_md().display()));
    assert!(
        stamp.contains(" UTC ") && stamp.contains("git `"),
        "the run stamp must carry both a UTC date and the commit it was computed at, got: {stamp}"
    );
}
