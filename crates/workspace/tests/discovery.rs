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
    DenyReason, Resolution, ResolveError, Tier, canonicalize, deny_reason, resolve,
    resolve_with_override,
};

/// Canonical form of a fixture path (the API returns canonical paths).
fn canon(path: &Path) -> PathBuf {
    fs::canonicalize(path).expect("fixture path canonicalizes")
}

/// Resolve with no tier-1 override.
fn resolve_bare(cwd: &Path) -> Resolution {
    resolve_with_override(cwd, None).expect("resolution succeeds")
}

#[test]
fn subfolder_of_marker_resolves_to_marker_top() {
    let tmp = TempDir::new().unwrap();
    let top = tmp.path().join("proj");
    let cwd = top.join("a/b/c");
    fs::create_dir_all(&cwd).unwrap();
    fs::write(top.join(".meridian.toml"), b"").unwrap();

    let res = resolve_bare(&cwd);
    assert_eq!(res.tier, Tier::Marker);
    assert_eq!(res.workspace, canon(&top));
}

#[test]
fn subfolder_of_git_resolves_to_git_top() {
    let tmp = TempDir::new().unwrap();
    let top = tmp.path().join("repo");
    let cwd = top.join("src/deep");
    fs::create_dir_all(&cwd).unwrap();
    fs::create_dir(top.join(".git")).unwrap();

    let res = resolve_bare(&cwd);
    assert_eq!(res.tier, Tier::GitRoot);
    assert_eq!(res.workspace, canon(&top));
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

    let res = resolve_bare(&cwd);
    assert_eq!(res.tier, Tier::GitRoot);
    // identity is the worktree top, NOT the pointer target, NOT the main checkout
    assert_eq!(res.workspace, canon(&linked));
    assert_ne!(res.workspace, canon(&main));
}

#[test]
fn symlinked_cwd_has_same_identity_as_real_path() {
    let tmp = TempDir::new().unwrap();
    let real = tmp.path().join("real");
    fs::create_dir_all(real.join("sub")).unwrap();
    let link = tmp.path().join("link");
    symlink(&real, &link).unwrap();

    let via_real = resolve_bare(&real.join("sub"));
    let via_link = resolve_bare(&link.join("sub"));
    assert_eq!(via_real.workspace, via_link.workspace);
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
    assert_eq!(via_real.workspace, via_variant.workspace);
    // on-disk case wins: the identity carries the real directory-entry name
    assert_eq!(
        via_variant.workspace.file_name().unwrap(),
        OsStr::new("MixedCase")
    );
}

#[test]
fn env_override_beats_marker() {
    let tmp = TempDir::new().unwrap();
    let proj = tmp.path().join("proj");
    fs::create_dir_all(&proj).unwrap();
    fs::write(proj.join(".meridian.toml"), b"").unwrap();
    let target = tmp.path().join("elsewhere");
    fs::create_dir_all(&target).unwrap();

    let res = resolve_with_override(&proj, Some(target.as_os_str())).unwrap();
    assert_eq!(res.tier, Tier::EnvOverride);
    assert_eq!(res.workspace, canon(&target));
}

#[test]
fn marker_beats_nearer_git() {
    let tmp = TempDir::new().unwrap();
    let outer = tmp.path().join("outer");
    let inner = outer.join("inner");
    let cwd = inner.join("sub");
    fs::create_dir_all(&cwd).unwrap();
    // marker is HIGHER (outer); git is NEARER (inner) — marker must still win
    fs::write(outer.join(".meridian.toml"), b"").unwrap();
    fs::create_dir(inner.join(".git")).unwrap();

    let res = resolve_bare(&cwd);
    assert_eq!(res.tier, Tier::Marker);
    assert_eq!(res.workspace, canon(&outer));
}

#[test]
fn legacy_yaml_marker_recognized() {
    let tmp = TempDir::new().unwrap();
    let proj = tmp.path().join("proj");
    let cwd = proj.join("sub");
    fs::create_dir_all(&cwd).unwrap();
    fs::write(
        proj.join(".meridian.yaml"),
        b"legacy: contents never parsed\n",
    )
    .unwrap();

    let res = resolve_bare(&cwd);
    assert_eq!(res.tier, Tier::Marker);
    assert_eq!(res.workspace, canon(&proj));
}

#[test]
fn bare_when_no_marker_git_or_override() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path().join("plain/leaf");
    fs::create_dir_all(&cwd).unwrap();

    let res = resolve_bare(&cwd);
    assert_eq!(res.tier, Tier::Bare);
    assert_eq!(res.workspace, canon(&cwd));
}

#[test]
fn env_override_nonexistent_is_loud_error() {
    let tmp = TempDir::new().unwrap();
    let cwd = tmp.path();
    let missing = tmp.path().join("does/not/exist");

    let err = resolve_with_override(cwd, Some(missing.as_os_str())).unwrap_err();
    assert!(matches!(err, ResolveError::EnvWorkspaceNotFound { .. }));
}

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
    let res = resolve(&cwd).unwrap();
    assert!(res.workspace.is_absolute());
}
