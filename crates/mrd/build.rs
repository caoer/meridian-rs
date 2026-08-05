//! Build identity for `mrd --version` (G10). The workspace publishes nothing, so
//! `CARGO_PKG_VERSION` is `0.0.0` for every crate and cannot tell two binaries apart. The
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

use std::path::{Path, PathBuf};
use std::process::Command;

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
    // `--git-path` resolves inside the real git directory, so a linked worktree
    // (whose `.git` is a file pointing elsewhere) watches its OWN HEAD.
    watch(manifest, "HEAD");
    // A ref can live in a file or in `packed-refs`; watch both, since only one
    // of them exists for any given branch and either one moving is a new HEAD.
    watch(manifest, "packed-refs");
    if let Some(refname) = git(manifest, &["symbolic-ref", "-q", "HEAD"]) {
        watch(manifest, &refname);
    }
    Some(sha)
}

/// Ask cargo to re-run this script when the named git path changes. A path git does not
/// resolve, or one that does not exist, is skipped: naming a missing file would re-run the
/// script on EVERY build, which is a cost with no answer behind it.
///
fn watch(manifest: &Path, git_path: &str) {
    let Some(resolved) = git(manifest, &["rev-parse", "--git-path", git_path]) else {
        return;
    };
    let path = manifest.join(&resolved);
    if path.exists() {
        println!("cargo::rerun-if-changed={}", path.display());
    }
}

/// Run git in the crates directory and return its trimmed stdout, or `None` for any failure at
/// all — no git on PATH, no repository, a non-zero exit. Every one of those means the same
/// thing here: this build cannot read a commit.
///
fn git(manifest: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(manifest)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (!text.is_empty()).then_some(text)
}
