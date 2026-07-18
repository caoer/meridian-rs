//! Consolidated integration-test member carrying the frozen GT pack as data —
//! no library code.
//!
//! # Charter
//! **Owns:** the workspace's integration tests, as ONE test binary
//! (`tests/main.rs`, modules per concern — the matklad consolidation doctrine),
//! and the frozen GT pack under `data/gt/` (provenance: `data/gt/PROVENANCE.md`).
//! Rung 1's parse-truth gate (every lane node reproduced byte-for-byte) runs
//! here against that pack.
//!
//! **Never does:** ship code (dependents: none, ever), own fixtures that aren't
//! frozen (scratch fixtures belong to the test module that uses them), edit the
//! GT pack (frozen means frozen — disagreements are findings for lane 0).

/// Path to the frozen GT pack, for test modules.
#[must_use]
pub fn gt_pack_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data/gt")
}
