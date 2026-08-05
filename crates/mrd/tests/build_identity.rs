//! `mrd --version` — the build identity, over the process boundary (G10,
//! dogfood 2026-08-04 run 01).
//!
//! # What this file holds the CLI to
//! Before this surface existed, `mrd --version` fell through to `unknown
//! subcommand: --version` plus 239 lines of help at exit 2, so the only way to
//! tell two `mrd` binaries apart was to hash them. Three propositions are
//! gated here:
//!
//! 1. **All three spellings answer, on stdout, at exit 0** — `--version`, the
//!    conventional `-V`, and the bare word.
//! 2. **The answer is ONE line, and it carries a commit.** An identity that
//!    scrolls, or that prints only `0.0.0` (which every crate in this
//!    publish-nothing workspace carries forever), identifies nothing.
//! 3. **The commit is READ, never invented.** In this repository the line must
//!    carry the real HEAD; `unknown` is the only other legal word, and it is
//!    legal only where a build could reach no repository.

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
    assert!(line.starts_with("mrd 0.0.0 (git "), "{line}");
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

/// The commit is the repositorys own HEAD — read, never invented. The test asks git the same
/// question the build script did. When git cannot answer HERE (a source tree with no
/// repository), the only legal identity is the literal `unknown`: this gate accepts that word
/// and no other, so a build that could not know its commit can never print one that looks real.
///
///
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
