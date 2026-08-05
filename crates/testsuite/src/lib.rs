//! Consolidated integration-test member carrying the frozen GT pack as data — no library
//! code.
//!
//! # Charter
//! **Owns:** the workspace's integration tests, as ONE test binary (`tests/main.rs`,
//! modules per concern — the matklad consolidation doctrine), and the frozen GT pack
//! under `data/gt/`. Rung 1's parse-truth gate (every lane node reproduced byte-for-byte)
//! runs here against that pack.
//!
//! **Never does:** ship code (dependents: none, ever), own fixtures that aren't frozen
//! (scratch fixtures belong to the test module that uses them), edit the GT pack (frozen
//! means frozen — disagreements are findings for lane 0).
//!

/// Path to the frozen GT pack, for test modules.
#[must_use]
pub fn gt_pack_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data/gt")
}

/// Path to the §0.3 wsfix worked-fixture pack (the build oracle).
#[must_use]
pub fn wsfix_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data/wsfix")
}

/// The adversarial-harness probe packs + walkvault fixture vault.
#[must_use]
pub fn harness_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data/harness")
}

/// The U0 read/put parity pack (corpus + captured goldens) — a byte-exact
/// copy of ccc-statusd's `testdata/parity/`; see `data/parity/README.md`
/// for provenance. The authoritative cutover gate is the Go-side harness.
#[must_use]
pub fn parity_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data/parity")
}

/// Path to the CHARSET-GUARD discrimination pack (the single block-id charset,
/// §2.4) — per-mint-position `_`-refusal inputs plus the pack-pinned walk
/// answer, for downstream units to ride.
#[must_use]
pub fn charset_guard_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data/charset-guard")
}

/// Path to the `MERIDIAN.md` in-file schema pack (stage 3, U2): `cases.json`
/// pairs every case with its required outcome, `corpus/` holds the
/// acceptances and `refusals/` the malformed classes.
#[must_use]
pub fn meridian_md_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data/meridian-md")
}
