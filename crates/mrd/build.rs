//! Build identity for `mrd --version` (G10). Every crate carries the one workspace stamp, so
//! `CARGO_PKG_VERSION` names the release and cannot tell two binaries apart. The commit can — so
//! this script bakes the tree it was built from into `MRD_BUILD_SHA`, and the CLI prints it.
//!
//! The stamp is a SHA WITH AN OPTIONAL MARKER, never a bare sha (`docs/release.md` §5.1):
//! `<sha>` where the worktree matched HEAD, `<sha>-dirty` where tracked content diverged from it,
//! `unknown` where no attributable identity could be read. A bare sha therefore ASSERTS a whole
//! commit, and that assertion is the one a reader acts on.
//!
//! Two rules keep the answer honest:
//!
//! - **It is read, never invented.** Nothing here composes a sha from parts or falls back to a
//!   plausible one. An unverifiable CLEAN claim is not published — a probe that fails publishes
//!   `unknown`, because "I could not tell" and "it was clean" are opposite answers.
//! - **It is re-read on EVERY build.** The answer is a function of the working tree, and a
//!   worktree edit moves no ref, so no watch list keyed on git's files can see one. This script
//!   names a sentinel path that never exists, which is how cargo is told "always". A stamp that
//!   outlives the tree it describes is exactly the defect this closes: it lets a binary built
//!   before its own commit report a clean identity nobody can attribute it to.
//!
//! `MRD_BUILD_SHA` supplied in the environment rides verbatim with no probe. The supplier owns
//! that claim, and supplying it to make a gate agree invents the answer the pin exists to give.

include!("build_git.rs");

use std::path::PathBuf;

fn main() {
    println!("cargo::rerun-if-env-changed=MRD_BUILD_SHA");
    // Cargo re-runs a script whose watched path does not exist, and this one never will.
    // Naming it inside OUT_DIR keeps the sentinel scoped to this build's own directory.
    let out_dir = std::env::var("OUT_DIR").expect("cargo always sets OUT_DIR");
    println!("cargo::rerun-if-changed={out_dir}/always-rerun-the-identity-probe");

    let manifest = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("cargo always sets CARGO_MANIFEST_DIR"),
    );
    let sha = env_sha().unwrap_or_else(|| stamp(&manifest));
    println!("cargo::rustc-env=MRD_BUILD_SHA={sha}");
}

/// The commit the environment supplied, if it supplied a non-empty one.
fn env_sha() -> Option<String> {
    let sha = std::env::var("MRD_BUILD_SHA").ok()?.trim().to_owned();
    (!sha.is_empty()).then_some(sha)
}

/// The identity this tree can prove: the commit plus whether the build came from
/// the whole of it. Either question failing collapses the answer to `unknown`
/// rather than to a sha that would claim more than was measured.
fn stamp(manifest: &Path) -> String {
    match (head_sha(manifest), worktree_dirty(manifest)) {
        (Some(sha), Some(false)) => sha,
        (Some(sha), Some(true)) => format!("{sha}-dirty"),
        _ => "unknown".to_owned(),
    }
}
