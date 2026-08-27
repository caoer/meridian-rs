//! An unreadable domain config must name WHICH file it could not read.
//!
//! `read_optional` used to return the raw `std::fs` error, which carries no
//! path. `Domain::load` / `Scope::load` read both `meridian/domain.md` and
//! `mdfs_config.yaml` through it, so a face holding the error could not tell
//! which file refused. The mint is `fs::CorpusMemberError`; `kind()` stays
//! the OS kind so the `NotFound` arm still means "absent, not an error".

use std::io;
use std::path::Path;

use fs::{domain, WorkspaceRoot};

fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&p, contents).unwrap();
}

fn workspace() -> (tempfile::TempDir, WorkspaceRoot) {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    write(&ws, "a.md", "# A\n");
    let root = WorkspaceRoot(std::fs::canonicalize(&ws).unwrap());
    (tmp, root)
}

/// Mode-000 file restored however the test leaves — returned value, failed
/// assertion, or panic. A leaked mode-000 file inside TempDir cannot be
/// cleaned up, so Drop always restores.
struct LockedFile<'a>(&'a Path);

impl<'a> LockedFile<'a> {
    fn new(path: &'a Path) -> LockedFile<'a> {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o000)).unwrap();
        assert!(
            std::fs::read_to_string(path).is_err(),
            "PRECONDITION FAILED: {} is still readable at mode 000 — this suite \
             cannot run as a privileged user, and passing here would test nothing",
            path.display()
        );
        LockedFile(path)
    }
}

impl Drop for LockedFile<'_> {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(self.0, std::fs::Permissions::from_mode(0o644));
    }
}

#[test]
fn unreadable_domain_md_names_that_file() {
    let (_tmp, root) = workspace();
    write(
        &root.0,
        domain::DOMAIN_CONFIG_PATH,
        "---\nignore: []\n---\n",
    );
    let locked = root.0.join(domain::DOMAIN_CONFIG_PATH);
    let locked = LockedFile::new(&locked);

    let err = domain::Domain::load(&root).expect_err("mode-000 domain.md must refuse");
    let said = err.to_string();
    assert!(
        said.contains(domain::DOMAIN_CONFIG_PATH),
        "refusal must name {path}, not a bare errno: {said:?}",
        path = domain::DOMAIN_CONFIG_PATH
    );
    assert!(
        !said.contains(domain::CONFIG_FILE_NAME),
        "must not collapse both configs into one name: {said:?}"
    );
    assert_eq!(
        err.kind(),
        io::ErrorKind::PermissionDenied,
        "kind() must stay the OS kind so the NotFound arm still steers: {said:?}"
    );
    let member = fs::corpus_member_error(&err).expect("typed locus");
    assert_eq!(member.member, domain::DOMAIN_CONFIG_PATH);

    drop(locked);
}

#[test]
fn unreadable_mdfs_config_yaml_names_that_file() {
    let (_tmp, root) = workspace();
    write(&root.0, domain::CONFIG_FILE_NAME, "ignore: []\n");
    let locked = root.0.join(domain::CONFIG_FILE_NAME);
    let locked = LockedFile::new(&locked);

    let err = domain::Domain::load(&root).expect_err("mode-000 mdfs_config.yaml must refuse");
    let said = err.to_string();
    assert!(
        said.contains(domain::CONFIG_FILE_NAME),
        "refusal must name {path}, not a bare errno: {said:?}",
        path = domain::CONFIG_FILE_NAME
    );
    assert!(
        !said.contains(domain::DOMAIN_CONFIG_PATH),
        "must not collapse both configs into one name: {said:?}"
    );
    assert_eq!(err.kind(), io::ErrorKind::PermissionDenied, "{said:?}");
    let member = fs::corpus_member_error(&err).expect("typed locus");
    assert_eq!(member.member, domain::CONFIG_FILE_NAME);

    drop(locked);
}

#[test]
fn a_readable_domain_config_loads() {
    let (_tmp, root) = workspace();
    write(
        &root.0,
        domain::DOMAIN_CONFIG_PATH,
        "---\nignore: []\n---\n",
    );
    domain::Domain::load(&root).expect("readable domain.md is a legal config");
}

#[test]
fn an_absent_domain_config_is_not_an_error() {
    let (_tmp, root) = workspace();
    domain::Domain::load(&root).expect("absent config is the default domain, not a refusal");
}
