//! The build script's git half, held against real repositories — because the
//! identity it bakes (`MRD_BUILD_SHA`) is a claim about a TREE, not only about a
//! commit, and a claim nobody has watched fail is an assumption.
//!
//! Gated here: a clean worktree probes clean and a touched tracked file probes
//! dirty, measured as a TRANSITION in one repository rather than as two terminal
//! readings — a probe wired to a typo answers "clean" forever and reads as a
//! pass. Beside it: staged-but-uncommitted is divergence, an untracked file is
//! not, and a directory with no repository answers "I could not tell" rather
//! than "clean".
//!
//! What is NOT gated here, and where it is instead: that cargo re-runs this
//! script on every build. That property lives in cargo's freshness logic, so a
//! unit test cannot reach it without nesting a build inside a build. Its gate is
//! the dogfood kit's `s0-20`, which refuses a `-dirty` binary declared as a
//! clean pin — an end-to-end property gated end to end.
//!
//! The script's git-reading half is included, not shelled: `head_sha` and
//! `worktree_dirty` are the units under test, and these tests spawn `git` only —
//! no cargo build rides them.

include!("../build_git.rs");

/// Run git in the scratch repository, isolated from the machine's own config (a
/// global `commit.gpgsign` or default-branch setting must not steer a gate).
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

/// A repository holding one committed tracked file, in a fresh tempdir.
fn scratch_repo() -> tempfile::TempDir {
    let repo = tempfile::tempdir().expect("tempdir");
    run_git(repo.path(), &["init", "-q"]);
    std::fs::write(repo.path().join("tracked.txt"), "one\n").expect("write");
    run_git(repo.path(), &["add", "tracked.txt"]);
    run_git(repo.path(), &["commit", "-q", "-m", "first"]);
    repo
}

/// The transition, which is the whole test: the same probe on the same
/// repository must read clean before the edit and dirty after it. A terminal
/// reading of either value proves nothing about the probe — only the change
/// does.
#[test]
fn the_probe_reads_clean_then_dirty_across_one_edit() {
    let repo = scratch_repo();
    assert_eq!(
        worktree_dirty(repo.path()),
        Some(false),
        "a freshly committed worktree is clean"
    );

    std::fs::write(repo.path().join("tracked.txt"), "two\n").expect("write");

    assert_eq!(
        worktree_dirty(repo.path()),
        Some(true),
        "an edited tracked file is divergence from HEAD"
    );
}

/// Staged is still uncommitted, and a build made from a staged tree is not a
/// build of the commit it names.
#[test]
fn staged_but_uncommitted_content_is_divergence() {
    let repo = scratch_repo();
    std::fs::write(repo.path().join("tracked.txt"), "two\n").expect("write");
    run_git(repo.path(), &["add", "tracked.txt"]);

    assert_eq!(worktree_dirty(repo.path()), Some(true));
}

/// An untracked file is not divergence: it reaches the compiler only through a
/// tracked `mod` line, and that line is itself an edit the probe sees. Counting
/// untracked files would mark every worktree holding a scratch note as dirty,
/// which retires the marker by making it always true.
#[test]
fn an_untracked_file_is_not_divergence() {
    let repo = scratch_repo();
    std::fs::write(repo.path().join("scratch.txt"), "notes\n").expect("write");

    assert_eq!(worktree_dirty(repo.path()), Some(false));
}

/// No repository: both probes answer `None`. The build script turns that pair
/// into `unknown`, never into a clean sha — "I could not tell" and "it was
/// clean" are opposite answers, and only one of them is safe to publish.
#[test]
fn a_directory_with_no_repository_answers_neither_question() {
    let plain = tempfile::tempdir().expect("tempdir");

    assert_eq!(head_sha(plain.path()), None);
    assert_eq!(worktree_dirty(plain.path()), None);
}

/// The commit is read, never invented: whatever `head_sha` returns is what
/// `git rev-parse HEAD` says in that same tree.
#[test]
fn head_sha_names_the_commit_git_names() {
    let repo = scratch_repo();
    let out = Command::new("git")
        .arg("-C")
        .arg(repo.path())
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("spawn git");
    let expected = String::from_utf8(out.stdout)
        .expect("utf8")
        .trim()
        .to_owned();

    assert_eq!(head_sha(repo.path()), Some(expected));
}
