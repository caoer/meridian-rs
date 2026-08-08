// The build script's git-reading half, in its own file so
// `tests/build_script_watch.rs` can `include!` it and hold the watch list
// against a real repository without running cargo. It cannot include
// `build.rs` itself: a build script's `fn main` and inner doc comments do not
// survive inclusion into another crate root.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The git paths whose change can mean HEAD names a different commit — the
/// list `build.rs` turns into `cargo::rerun-if-changed` lines. A value rather
/// than a side effect so the gate can read it.
fn watch_paths(manifest: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    // `--git-path` resolves inside the real git directory, so a linked worktree
    // (whose `.git` is a file pointing elsewhere) watches its OWN HEAD.
    watch(manifest, "HEAD", &mut paths);
    // A ref can live in a file or in `packed-refs`; watch both, since only one
    // of them exists for any given branch and either one moving is a new HEAD.
    watch(manifest, "packed-refs", &mut paths);
    if let Some(refname) = git(manifest, &["symbolic-ref", "-q", "HEAD"]) {
        watch(manifest, &refname, &mut paths);
    }
    paths
}

/// Collect the named git path for the watch list. A path git does not
/// resolve, or one that does not exist, is skipped: naming a missing file would re-run the
/// script on EVERY build, which is a cost with no answer behind it.
///
fn watch(manifest: &Path, git_path: &str, paths: &mut Vec<PathBuf>) {
    let Some(resolved) = git(manifest, &["rev-parse", "--git-path", git_path]) else {
        return;
    };
    let path = manifest.join(&resolved);
    if path.exists() {
        paths.push(path);
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
