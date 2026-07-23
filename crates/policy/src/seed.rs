//! The throwaway SEED convention (U1.3) — `reviewer-not-owner`.
//!
//! # Why it exists
//! The harness (`mrd test`, U1.2) and the door (`gate()`, U4.2) need a REAL
//! `check_change` to pre-test against BEFORE the U4.4 floor conventions land (plan
//! adversarial-C2 fix: "U1.3 owns a seed convention; the sequencing gift is
//! restated"). This module ships one: `reviewer-not-owner`, embedded so later units
//! reach it with [`load_seed_convention`] and no path resolution.
//!
//! # It is throwaway
//! The seed is scaffolding, never arm it — the real floor conventions are U4.4's.
//! When they land, this seed can be deleted (delete-don't-migrate).

use crate::check_eval::CheckLimits;
use crate::convention::{Convention, ConventionFiles, LoadError, load_convention};

/// The seed convention's subject slug.
pub const SEED_CONVENTION_SLUG: &str = "reviewer-not-owner";

const CHECK_MD: &str = include_str!("../seed/reviewer-not-owner/CHECK.md");
const BASE_FIX_PARSER: &str = include_str!("../seed/reviewer-not-owner/base/tasks/fix-parser.md");
const SCENARIO_OWNER_SELF_CLOSE: &str =
    include_str!("../seed/reviewer-not-owner/scenarios/owner-self-close.md");
const SCENARIO_REVIEWER_CLOSE: &str =
    include_str!("../seed/reviewer-not-owner/scenarios/reviewer-close.md");

/// The seed convention's folder as embedded files — a [`ConventionFiles`] backed by
/// `include_str!`, so the seed is compiled in and needs no disk access. The full
/// folder (CHECK + base + scenarios) travels with the crate for U1.2/U4.2.
pub struct SeedFiles;

impl SeedFiles {
    /// The `(rel_path, contents)` pairs of the embedded seed folder.
    const FILES: [(&'static str, &'static str); 4] = [
        ("CHECK.md", CHECK_MD),
        ("base/tasks/fix-parser.md", BASE_FIX_PARSER),
        ("scenarios/owner-self-close.md", SCENARIO_OWNER_SELF_CLOSE),
        ("scenarios/reviewer-close.md", SCENARIO_REVIEWER_CLOSE),
    ];
}

impl ConventionFiles for SeedFiles {
    fn read(&self, rel_path: &str) -> std::io::Result<String> {
        Self::FILES
            .iter()
            .find(|(rel, _)| *rel == rel_path)
            .map(|(_, body)| (*body).to_string())
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("seed convention has no {rel_path}"),
                )
            })
    }

    fn exists(&self, rel_path: &str) -> bool {
        Self::FILES.iter().any(|(rel, _)| *rel == rel_path)
    }
}

/// The embedded seed folder accessor (for U1.2's mount and U4.2's arming).
#[must_use]
pub fn seed_convention_files() -> SeedFiles {
    SeedFiles
}

/// Load the seed convention through the loader under the given limits — a real
/// [`Convention`] with a real `check_change`, for pre-testing before U4.4.
///
/// # Errors
/// [`LoadError`] if the embedded seed ever stops satisfying the loader (a
/// regression guard: the seed IS a loader fixture).
pub fn load_seed_convention(limits: CheckLimits) -> Result<Convention, LoadError> {
    load_convention(SEED_CONVENTION_SLUG, &SeedFiles, limits)
}
