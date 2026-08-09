//! Build identity for `mrd --version` (G10). Every crate carries the one workspace
//! stamp, so `CARGO_PKG_VERSION` names the release and cannot tell two binaries apart. The
//! commit can — so this script bakes the commit HEAD named when it last ran into
//! `MRD_BUILD_SHA`, and the CLI prints it. Before this existed, the only way to ask a `mrd`
//! binary what it was, was to hash it ( Two rules keep the answer honest: - **It is read, never
//! invented.
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!

include!("build_watch.rs");

fn main() {
    println!("cargo::rerun-if-env-changed=MRD_BUILD_SHA");
    let manifest = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("cargo always sets CARGO_MANIFEST_DIR"),
    );
    let sha = env_sha()
        .or_else(|| git_sha(&manifest))
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo::rustc-env=MRD_BUILD_SHA={sha}");
}

/// The commit the environment supplied, if it supplied a non-empty one.
fn env_sha() -> Option<String> {
    let sha = std::env::var("MRD_BUILD_SHA").ok()?.trim().to_owned();
    (!sha.is_empty()).then_some(sha)
}

/// The commit HEAD names, plus the watch list that makes the next move of HEAD
/// re-run this script.
fn git_sha(manifest: &Path) -> Option<String> {
    let sha = git(manifest, &["rev-parse", "HEAD"])?;
    for path in watch_paths(manifest) {
        println!("cargo::rerun-if-changed={}", path.display());
    }
    Some(sha)
}
