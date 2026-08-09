//! `mrd --version` — the build identity, over the process boundary.
//!
//! Gated here: all three spellings (`--version`, `-V`, bare `version`) answer on
//! stdout at exit 0; the answer is one line carrying a commit; the commit is
//! read, never invented — the real HEAD, or the literal `unknown` where a build
//! could reach no repository.

use std::process::{Command, Output};

fn mrd(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mrd"))
        .args(args)
        .output()
        .expect("run mrd")
}

fn code(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// The identity line itself: exit 0, on stdout, one line, naming the package
/// and a commit.
#[test]
fn the_version_line_names_the_package_and_a_commit() {
    let out = mrd(&["--version"]);
    assert_eq!(code(&out), 0, "asking a binary what it is is not a failure");
    let line = stdout(&out);
    assert_eq!(
        line.lines().count(),
        1,
        "the identity is one line, not a page:\n{line}"
    );
    let line = line.trim();
    // Derived, never a literal: the assertion tracks the workspace stamp, so a
    // release bump cannot red this gate on a version string it was never
    // testing (v1 stamp, `docs/release.md` §5.1).
    let expected = concat!("mrd ", env!("CARGO_PKG_VERSION"), " (git ");
    assert!(line.starts_with(expected), "{line}");
    assert!(line.ends_with(')'), "{line}");
}

/// The three spellings are one answer. `-V` is the convention a caller reaches
/// for first, and the bare word is what `mrd help`'s neighbour looks like.
#[test]
fn every_spelling_prints_the_same_identity() {
    let long = stdout(&mrd(&["--version"]));
    for spelling in ["-V", "version"] {
        let out = mrd(&[spelling]);
        assert_eq!(code(&out), 0, "`mrd {spelling}` answers");
        assert_eq!(stdout(&out), long, "`mrd {spelling}` is the same answer");
    }
}

/// The commit is the repositorys own HEAD — read, never invented. When git cannot answer here
/// (a source tree with no repository), the only legal identity is the literal `unknown`.
#[test]
fn the_commit_is_the_one_git_names() {
    let line = stdout(&mrd(&["--version"]));
    let sha = line
        .trim()
        .rsplit_once("(git ")
        .and_then(|(_, tail)| tail.strip_suffix(')'))
        .expect("the identity line carries a (git …) field")
        .to_owned();

    let head = Command::new("git")
        .arg("-C")
        .arg(env!("CARGO_MANIFEST_DIR"))
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned());

    match head {
        Some(head) => assert_eq!(sha, head, "the identity names the commit git names"),
        None => assert_eq!(
            sha, "unknown",
            "no repository to read: the only honest answer is `unknown`"
        ),
    }
}

/// The flag is documented where a caller looks for it, so the surface and the
/// binary cannot disagree about whether it exists.
#[test]
fn the_listing_documents_the_flag() {
    let help = stdout(&mrd(&["--help"]));
    assert!(help.contains("-V, --version"), "{help}");
}
