//! The build script's watch list, held against a real repository — because the
//! identity it bakes (`MRD_BUILD_SHA`) is only honest while cargo re-runs the
//! script when HEAD moves. Gated here: after a `git pack-refs --all --prune`
//! transition, the next commit still touches at least one watched path. That is
//! the 2026-08-08 stale-bake hole: at script run time the loose branch ref does
//! not exist (skipped), the next commit recreates it unwatched, and HEAD and
//! `packed-refs` both stay bytewise still — so the baked commit misreports until
//! something else re-runs the script. The plain loose-ref repository keeps its
//! coverage, and the list never names a missing file (that would re-run the
//! script on every build).
//!
//! The script's git-reading half is included, not shelled: `watch_paths` is the
//! unit under test, and these tests spawn `git` only — no cargo build rides
//! them.

include!("../build_watch.rs");

use std::collections::BTreeMap;

/// Run git in the scratch repository, isolated from the machine's own config
/// (a global `commit.gpgsign` or default-branch setting must not steer a gate).
fn run_git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .args(args)
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A repository with one commit, in a fresh tempdir.
fn scratch_repo() -> tempfile::TempDir {
    let repo = tempfile::tempdir().expect("tempdir");
    run_git(repo.path(), &["init", "-q"]);
    run_git(
        repo.path(),
        &["commit", "--allow-empty", "-q", "-m", "first"],
    );
    repo
}

/// The bytes behind every watched path — `None` where the file is gone. Cargo's
/// freshness check is content-shaped, so "did a watched path change" is exactly
/// "did this map change".
fn snapshot(paths: &[PathBuf]) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    paths
        .iter()
        .map(|p| (p.clone(), std::fs::read(p).ok()))
        .collect()
}

/// The hole itself: pack the refs, take the watch list a first build would take,
/// then commit. Some watched path must differ, or the next build keeps the old
/// baked commit.
#[test]
fn a_commit_after_pack_refs_still_reaches_a_watched_path() {
    let repo = scratch_repo();
    run_git(repo.path(), &["pack-refs", "--all", "--prune"]);

    let watched = watch_paths(repo.path());
    assert!(!watched.is_empty(), "a live repository yields a watch list");
    let before = snapshot(&watched);

    run_git(
        repo.path(),
        &["commit", "--allow-empty", "-q", "-m", "second"],
    );

    assert_ne!(
        before,
        snapshot(&watched),
        "a commit after pack-refs moved no watched path — the baked identity is stale"
    );
}

/// The ordinary loose-ref repository: the branch ref itself is watched, a commit
/// reaches it, and the list never names a missing file.
#[test]
fn a_loose_ref_repository_keeps_its_coverage() {
    let repo = scratch_repo();

    let watched = watch_paths(repo.path());
    assert!(
        watched.iter().all(|p| p.exists()),
        "the list never names a missing file: {watched:?}"
    );

    let refname = Command::new("git")
        .arg("-C")
        .arg(repo.path())
        .args(["symbolic-ref", "-q", "HEAD"])
        .output()
        .expect("spawn git");
    let refname = String::from_utf8(refname.stdout)
        .expect("utf8")
        .trim()
        .to_owned();
    assert!(
        watched.iter().any(|p| p.ends_with(&refname)),
        "the loose branch ref {refname} is on the list: {watched:?}"
    );

    let before = snapshot(&watched);
    run_git(
        repo.path(),
        &["commit", "--allow-empty", "-q", "-m", "second"],
    );
    assert_ne!(
        before,
        snapshot(&watched),
        "a loose-ref commit moved no watched path"
    );
}
