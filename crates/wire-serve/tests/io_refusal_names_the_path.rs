//! Card `engine-io-error-names-no-path`, the WIRE half: the `io_error` a
//! caller actually receives names the directory the walk could not list.
//!
//! The wire seam is faithful and always was — `io_refusal(e.to_string())`
//! prints everything it is given, and `std::fs` errors carry no path, so the
//! fact was gone before it arrived. The mint moved into `crates/fs`; this
//! gate pins that the wire frame INHERITS it, so the fix cannot be proved on
//! one face and quietly regress on the other.
//!
//! What the caller saw during the 2026-08-24 incident, verbatim:
//!     io_error: Permission denied (os error 13)
//! — for a mode-000 directory that could have been anywhere under the root.

use std::path::Path;

use wire::ErrorCode;

fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, contents).unwrap();
}

fn lock(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o000)).unwrap();
    assert!(
        std::fs::read_dir(dir).is_err(),
        "PRECONDITION FAILED: {} is still listable at mode 000 — this gate \
         cannot run as a privileged user, and passing here would test nothing",
        dir.display()
    );
}

fn unlock(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A door that observes the corpus refuses `io_error` with the offending
/// directory in the `cause` the caller reads.
#[test]
fn the_wire_io_error_cause_names_the_unlistable_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    write(&ws, "a.md", "# A\n");
    write(&ws, "notes/locked/c.md", "# C\n");
    let root = fs::WorkspaceRoot(std::fs::canonicalize(&ws).unwrap());
    let locked = root.0.join("notes/locked");
    lock(&locked);

    let err = wire_serve::write::scope_token(&root, None, None)
        .expect_err("a corpus it cannot walk refuses");

    assert_eq!(err.code, ErrorCode::IoError);
    let cause = err
        .cause
        .as_deref()
        .expect("the io refusal carries its cause");
    assert!(
        cause.contains("notes/locked"),
        "the wire frame must name the directory — a bare errno sends the \
         caller hunting the whole tree from outside: {cause:?}"
    );

    unlock(&locked);
}

/// Negative control: the same door on a healthy corpus mints no refusal, so
/// the assertion above discriminates rather than matching whatever every run
/// produces.
#[test]
fn a_healthy_corpus_mints_no_io_refusal_at_the_same_door() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    write(&ws, "a.md", "# A\n");
    write(&ws, "notes/locked/c.md", "# C\n");
    let root = fs::WorkspaceRoot(std::fs::canonicalize(&ws).unwrap());

    wire_serve::write::scope_token(&root, None, None)
        .expect("the same door serves a readable corpus");
}
