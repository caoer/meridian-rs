//! Behavioral quality gates for the workspace discovery ladder.
//!
//! Every test builds an on-disk fixture under a fresh `tempfile::TempDir`
//! and drives the public API only. Fixtures never touch process-global
//! state (no `env::set_var`), so the suite is parallel-safe.

use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use tempfile::TempDir;
use workspace::{
    Answer, DenyReason, ResolveError, Tier, canonicalize, deny_reason, resolve,
    resolve_with_override,
};

/// Canonical form of a fixture path (the API returns canonical paths).
fn canon(path: &Path) -> PathBuf {
    fs::canonicalize(path).expect("fixture path canonicalizes")
}

/// Resolve with no override.
fn resolve_bare(cwd: &Path) -> Answer {
    resolve_with_override(cwd, None).expect("resolution succeeds")
}

// ── The retired markers anchor NOTHING ──────────────────────────────────────

#[test]
fn marker_file_below_a_git_root_resolves_to_the_git_root() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    let inner = repo.join("inner");
    fs::create_dir_all(&inner).unwrap();
    fs::create_dir(repo.join(".git")).unwrap();
    // The retired marker, deliberately nearer than `.git`.
    fs::write(inner.join(".meridian.toml"), b"").unwrap();

    let answer = resolve_bare(&inner);
    assert_eq!(answer.tier(), Tier::GitRoot);
    assert_eq!(answer.root(), Some(canon(&repo).as_path()));
}

#[test]
fn marker_file_alone_does_not_anchor_a_workspace() {
    let tmp = TempDir::new().unwrap();
    let proj = tmp.path().join("proj");
    let cwd = proj.join("sub");
    fs::create_dir_all(&cwd).unwrap();
    fs::write(proj.join(".meridian.toml"), b"").unwrap();

    let answer = resolve_bare(&cwd);
    assert_eq!(answer.tier(), Tier::CwdDefault);
    assert_eq!(answer.root_or_cwd(), canon(&cwd));
}

#[test]
fn legacy_yaml_marker_does_not_anchor_a_workspace() {
    let tmp = TempDir::new().unwrap();
    let proj = tmp.path().join("proj");
    let cwd = proj.join("sub");
    fs::create_dir_all(&cwd).unwrap();
    fs::write(proj.join(".meridian.yaml"), b"legacy\n").unwrap();

    let answer = resolve_bare(&cwd);
    assert_eq!(answer.tier(), Tier::CwdDefault);
    assert_eq!(answer.root_or_cwd(), canon(&cwd));
}

// ── Never silently: the type refuses to hand over a defaulted root ──────────

#[test]
fn cwd_default_yields_no_root_only_an_acknowledged_cwd() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path().join("plain/leaf");
    fs::create_dir_all(&cwd).unwrap();

    let answer = resolve_bare(&cwd);
    // The whole point: a defaulted answer has NO root to take by accident.
    assert_eq!(answer.root(), None);
    // Reaching the path takes the verbose, greppable accessor.
    assert_eq!(answer.root_or_cwd(), canon(&cwd));
}

#[test]
fn an_answered_tier_yields_a_root() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    let cwd = repo.join("src");
    fs::create_dir_all(&cwd).unwrap();
    fs::create_dir(repo.join(".git")).unwrap();

    let answer = resolve_bare(&cwd);
    assert_eq!(answer.root(), Some(canon(&repo).as_path()));
    assert_eq!(answer.root_or_cwd(), canon(&repo));
}

#[test]
fn every_answer_states_its_tier_and_its_root() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::create_dir(repo.join(".git")).unwrap();
    let plain = tmp.path().join("plain");
    fs::create_dir_all(&plain).unwrap();

    // The provenance sentence belongs to the type, not to each caller.
    let git = resolve_bare(&repo.join("src")).to_string();
    assert!(git.starts_with("git-root "), "{git}");
    assert!(git.contains(canon(&repo).to_str().unwrap()), "{git}");

    let defaulted = resolve_bare(&plain).to_string();
    assert!(defaulted.starts_with("cwd-default "), "{defaulted}");
    assert!(
        defaulted.contains(canon(&plain).to_str().unwrap()),
        "{defaulted}"
    );
}

#[test]
fn tier_words_are_stable_labels() {
    assert_eq!(Tier::EnvOverride.word(), "env-override");
    assert_eq!(Tier::GitRoot.word(), "git-root");
    assert_eq!(Tier::CwdDefault.word(), "cwd-default");
}

// ── The surviving rungs ─────────────────────────────────────────────────────

#[test]
fn subfolder_of_git_resolves_to_git_top() {
    let tmp = TempDir::new().unwrap();
    let top = tmp.path().join("repo");
    let cwd = top.join("src/deep");
    fs::create_dir_all(&cwd).unwrap();
    fs::create_dir(top.join(".git")).unwrap();

    let answer = resolve_bare(&cwd);
    assert_eq!(answer.tier(), Tier::GitRoot);
    assert_eq!(answer.root(), Some(canon(&top).as_path()));
}

#[test]
fn nearest_git_wins_over_a_higher_git() {
    let tmp = TempDir::new().unwrap();
    let outer = tmp.path().join("outer");
    let inner = outer.join("inner");
    let cwd = inner.join("sub");
    fs::create_dir_all(&cwd).unwrap();
    fs::create_dir(outer.join(".git")).unwrap();
    fs::create_dir(inner.join(".git")).unwrap();

    let answer = resolve_bare(&cwd);
    assert_eq!(answer.root(), Some(canon(&inner).as_path()));
}

#[test]
fn linked_worktree_git_file_resolves_to_worktree_top() {
    let tmp = TempDir::new().unwrap();
    let main = tmp.path().join("main");
    let linked = tmp.path().join("linked");
    let cwd = linked.join("sub");
    fs::create_dir_all(&cwd).unwrap();
    fs::create_dir_all(&main).unwrap();
    // main checkout: .git as a directory
    fs::create_dir(main.join(".git")).unwrap();
    // linked worktree: .git as a FILE with a gitdir pointer we must NOT follow
    fs::write(linked.join(".git"), b"gitdir: /some/where/else\n").unwrap();

    let answer = resolve_bare(&cwd);
    assert_eq!(answer.tier(), Tier::GitRoot);
    // identity is the worktree top, NOT the pointer target, NOT the main checkout
    assert_eq!(answer.root(), Some(canon(&linked).as_path()));
    assert_ne!(answer.root(), Some(canon(&main).as_path()));
}

#[test]
fn env_override_beats_the_git_root() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir(repo.join(".git")).unwrap();
    let target = tmp.path().join("elsewhere");
    fs::create_dir_all(&target).unwrap();

    let answer = resolve_with_override(&repo, Some(target.as_os_str())).unwrap();
    assert_eq!(answer.tier(), Tier::EnvOverride);
    assert_eq!(answer.root(), Some(canon(&target).as_path()));
}

#[test]
fn env_override_nonexistent_is_loud_error() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path();
    let missing = tmp.path().join("does/not/exist");

    let err = resolve_with_override(cwd, Some(missing.as_os_str())).unwrap_err();
    assert!(matches!(err, ResolveError::EnvWorkspaceNotFound { .. }));
}

// ── Identity is canonicalize ────────────────────────────────────────────────

#[test]
fn symlinked_cwd_has_same_identity_as_real_path() {
    let tmp = TempDir::new().unwrap();
    let real = tmp.path().join("real");
    fs::create_dir_all(real.join("sub")).unwrap();
    let link = tmp.path().join("link");
    symlink(&real, &link).unwrap();

    let via_real = resolve_bare(&real.join("sub"));
    let via_link = resolve_bare(&link.join("sub"));
    assert_eq!(via_real.root_or_cwd(), via_link.root_or_cwd());
    // and canonicalize collapses the symlink directly
    assert_eq!(canonicalize(&link).unwrap(), canonicalize(&real).unwrap());
}

#[test]
fn case_variant_spelling_is_one_identity() {
    let tmp = TempDir::new().unwrap();
    // Probe: is this filesystem case-insensitive?
    let probe = tmp.path().join("CaseProbe");
    fs::create_dir(&probe).unwrap();
    let variant_probe = tmp.path().join("caseprobe");
    let case_insensitive = fs::canonicalize(&variant_probe).is_ok_and(|c| c == canon(&probe));
    if !case_insensitive {
        eprintln!("note: filesystem is case-sensitive; case-variant collapse test skipped");
        return;
    }

    let real = tmp.path().join("MixedCase");
    fs::create_dir(&real).unwrap();
    let real_spelling = tmp.path().join("MixedCase");
    let variant_spelling = tmp.path().join("mixedcase");

    let via_real = resolve_bare(&real_spelling);
    let via_variant = resolve_bare(&variant_spelling);
    assert_eq!(via_real.root_or_cwd(), via_variant.root_or_cwd());
    // on-disk case wins: the identity carries the real directory-entry name
    assert_eq!(
        via_variant.root_or_cwd().file_name().unwrap(),
        OsStr::new("MixedCase")
    );
}

// ── The deny ceiling (shared with the mount plane, never re-implemented) ────

#[test]
fn deny_ceiling_refuses_root_home_and_tmp() {
    assert_eq!(
        deny_reason(Path::new("/")),
        Some(DenyReason::FilesystemRoot)
    );
    assert_eq!(deny_reason(Path::new("/tmp")), Some(DenyReason::TempDir));

    if let Some(home) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
        assert_eq!(deny_reason(Path::new(&home)), Some(DenyReason::HomeDir));
    }
}

#[test]
fn deny_ceiling_allows_a_normal_workspace() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path().join("project/pkg");
    fs::create_dir_all(&cwd).unwrap();
    assert_eq!(deny_reason(&canon(&cwd)), None);
}

#[test]
fn cache_root_names_meridian_dir() {
    if std::env::var_os("HOME").is_none() && std::env::var_os("XDG_CACHE_HOME").is_none() {
        return;
    }
    let root = workspace::cache_root().expect("cache root resolves when HOME/XDG set");
    assert_eq!(root.file_name().unwrap(), OsStr::new("meridian"));
}

#[test]
fn resolve_reads_process_env_without_panicking() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path().join("leaf");
    fs::create_dir_all(&cwd).unwrap();
    // Only assert env-independent invariant: resolve succeeds and returns a
    // canonical path. Tier depends on the ambient MERIDIAN_WORKSPACE.
    let answer = resolve(&cwd).unwrap();
    assert!(answer.root_or_cwd().is_absolute());
}
