// The build script's git-reading half, in its own file so
// `tests/build_script_git.rs` can `include!` it and hold the probes against a
// real repository without running cargo. It cannot include `build.rs` itself: a
// build script's `fn main` and inner doc comments do not survive inclusion into
// another crate root.

use std::path::Path;
use std::process::Command;

/// The commit HEAD names, or `None` where this build can reach no repository.
fn head_sha(manifest: &Path) -> Option<String> {
    git(manifest, &["rev-parse", "HEAD"])
}

/// Whether tracked content diverges from HEAD — the difference between "built
/// from commit X" and "built from a worktree that was once X". `None` means the
/// question could not be answered, which is never the same as `false`.
///
/// Untracked files are excluded on purpose: the question is whether this build
/// is attributable to HEAD, and an untracked file only reaches the compiler
/// through a tracked `mod` line, which is itself a divergence. `--no-optional-
/// locks` keeps the probe from writing the index, so a build can never block a
/// concurrent git in the same tree.
fn worktree_dirty(manifest: &Path) -> Option<bool> {
    let status = git_allowing_empty(
        manifest,
        &[
            "--no-optional-locks",
            "status",
            "--porcelain",
            "--untracked-files=no",
        ],
    )?;
    Some(!status.is_empty())
}

/// Run git in the crate directory and return its trimmed stdout, or `None` for any
/// failure at all — no git on PATH, no repository, a non-zero exit. Every one of
/// those means the same thing here: this build cannot read a commit. Empty output
/// is a failure too, because no question asked through this helper has the empty
/// string for an answer.
fn git(manifest: &Path, args: &[&str]) -> Option<String> {
    let text = git_allowing_empty(manifest, args)?;
    (!text.is_empty()).then_some(text)
}

/// The same call for the one question whose answer CAN be empty: a clean
/// worktree prints nothing. Folding that into [`git`] would read "clean" as
/// "could not tell", which is the one confusion this whole stamp exists to end.
fn git_allowing_empty(manifest: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(manifest)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8(out.stdout).ok()?.trim().to_owned())
}
