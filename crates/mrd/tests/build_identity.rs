//! `mrd --version` — the build identity, over the process boundary.
//!
//! Gated here: all three spellings (`--version`, `-V`, bare `version`) answer on
//! stdout at exit 0; the answer is one line carrying a commit; the commit and
//! the state of the tree around it are read, never invented — the real HEAD
//! with a `-dirty` marker where tracked content diverged from it, or the literal
//! `unknown` where neither question could be answered.

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

/// The identity is the repository's own HEAD plus whether the build came from the WHOLE of it —
/// read, never invented (`docs/release.md` §5.1). `<sha>` asserts a clean tree, `<sha>-dirty`
/// says tracked content diverged, and `unknown` is the only legal answer where git could not be
/// asked at all.
///
/// This asserts the exact token rather than the sha alone, because the marker is the half a
/// reader acts on and an assertion that ignores it would pass on a binary lying about its tree.
/// It can only be exact because the build script re-runs on every build: cargo therefore
/// restamps before this test runs, and the one window left — another process editing this tree
/// between the build and this line — cannot arise in the isolated tree a candidate is built in.
#[test]
fn the_identity_names_the_commit_and_the_state_of_its_tree() {
    let line = stdout(&mrd(&["--version"]));
    let stamp = line
        .trim()
        .rsplit_once("(git ")
        .and_then(|(_, tail)| tail.strip_suffix(')'))
        .expect("the identity line carries a (git …) field")
        .to_owned();

    let git = |args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(env!("CARGO_MANIFEST_DIR"))
            .args(args)
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
    };

    let head = git(&["rev-parse", "HEAD"]).filter(|sha| !sha.is_empty());
    let dirty = git(&[
        "--no-optional-locks",
        "status",
        "--porcelain",
        "--untracked-files=no",
    ])
    .map(|status| !status.is_empty());

    let expected = match (head, dirty) {
        (Some(head), Some(false)) => head,
        (Some(head), Some(true)) => format!("{head}-dirty"),
        // Either question unanswerable: the build published no attributable identity, and
        // neither may this expectation invent one.
        _ => "unknown".to_owned(),
    };

    assert_eq!(
        stamp, expected,
        "the identity states the commit git names and whether the tree matched it"
    );
}

/// The flag is documented where a caller looks for it, so the surface and the
/// binary cannot disagree about whether it exists.
#[test]
fn the_listing_documents_the_flag() {
    let help = stdout(&mrd(&["--help"]));
    assert!(help.contains("-V, --version"), "{help}");
}
