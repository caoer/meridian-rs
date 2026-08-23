//! The tree gate lives in TWO places: `build_git.rs` (the engine's probe) and
//! three `.woodpecker` files that re-implement it by hand in shell. Nothing else
//! keeps them in agreement, and they have drifted twice — the tag lanes
//! published `d3accfbe4e28…-dirty`, then ci.yaml published TEN `-dirty` main
//! heads (fixed in PR 199). This test is the guard the drift asked for: it
//! derives the expected probe from `build_git.rs`'s own argument list and
//! asserts every YAML copy carries it — flags, tri-state shape, the fourth-state
//! (HEAD vs stamp) check, and `--locked` on the one cargo call that can rewrite
//! a tracked file.
//!
//! Why the SHAPE and not only the flags: PR 199 measured that
//! `if [ -n "$(git …)" ]` FAILS OPEN — a git exiting non-zero with empty stdout
//! reads as "clean" and the lane publishes a release stamped with a sha it
//! never measured. Matching the flags without matching the question is how a
//! gate fails open, so the guard binds the whole `if ! porcelain="$( … )"` line.
//!
//! Card: 22-18-hook-support-design/tasks/ci-tree-gate-blind-spots.

use std::fs;
use std::path::PathBuf;

const PIPELINE_FILES: [&str; 3] = [
    ".woodpecker/ci.yaml",
    ".woodpecker/tag-linux-amd64.yaml",
    ".woodpecker/tag-darwin-arm64.yaml",
];

fn repo_root() -> PathBuf {
    // crates/mrd -> repo root. The test is meaningless outside the repo, so a
    // missing .woodpecker is a FAILURE (the guard's subject is gone), never a skip.
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// The YAML files QUOTE the fail-open anti-pattern inside comments (that is how
/// they teach it), so every absence assertion must see only executable lines.
fn without_comments(s: &str) -> String {
    s.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Extract the argument list `worktree_dirty` passes to git, from source text.
/// Anchored to the function so another `git_allowing_empty` call cannot leak in.
fn engine_probe_args() -> Vec<String> {
    let src = read("crates/mrd/build_git.rs");
    let start = src
        .find("fn worktree_dirty")
        .expect("build_git.rs no longer defines worktree_dirty — the probe this guard mirrors");
    let body = &src[start..];
    let end = body
        .find(")?;")
        .expect("worktree_dirty's git call shape changed — re-derive this guard");
    let call = &body[..end];
    let args: Vec<String> = call
        .split('"')
        .skip(1)
        .step_by(2) // every second fragment is inside quotes
        .map(str::to_owned)
        .collect();
    assert!(
        !args.is_empty(),
        "extracted no string literals from worktree_dirty — parser out of date"
    );
    args
}

/// Every YAML copy of the probe must be the engine's, flag for flag, in the
/// tri-state (non-fail-open) shape.
#[test]
fn yaml_probes_match_the_engine_flag_for_flag() {
    let args = engine_probe_args();
    // ["--no-optional-locks", "status", "--porcelain", "--untracked-files=no"]
    let expected_opener = format!("if ! porcelain=\"$(git {})\"; then", args.join(" "));
    for rel in PIPELINE_FILES {
        let body = without_comments(&read(rel));
        assert!(
            body.contains(&expected_opener),
            "{rel}: probe drifted from build_git.rs — expected the tri-state opener\n  {expected_opener}\n\
             (either the engine's flags changed and the YAML must follow, or the YAML \
             was edited away from the engine's question)"
        );
    }
}

/// The fail-open form must not come back anywhere executable: a git exiting
/// non-zero with empty stdout must never read as "clean".
#[test]
fn fail_open_probe_shape_is_absent() {
    for rel in PIPELINE_FILES {
        let body = without_comments(&read(rel));
        assert!(
            !body.contains("[ -n \"$(git"),
            "{rel}: found the fail-open probe shape `[ -n \"$(git …)\" ]` in executable \
             text — it publishes on a git that could not answer (PR 199, reproduced \
             with a stub git exiting 128)"
        );
    }
}

/// The fourth state (F7, binding review of PR 199): the probe answers for HEAD,
/// the stamp names `CI_COMMIT_SHA` / `$rev`, and only this guard compares the two.
/// It must exist, be tri-state itself, and run BEFORE the stamp.
#[test]
fn head_vs_stamp_guard_present_and_before_the_stamp() {
    for rel in PIPELINE_FILES {
        let body = without_comments(&read(rel));
        let guard = "if ! head=\"$(git rev-parse HEAD)\"; then";
        let guard_at = body.find(guard).unwrap_or_else(|| {
            panic!(
                "{rel}: the fourth-state guard (tri-state `head=\"$(git rev-parse HEAD)\"`) \
                 is gone — the probe would again answer for a tree the stamp does not name"
            )
        });
        let stamp_at = body.find("MRD_BUILD_SHA=").unwrap_or_else(|| {
            panic!("{rel}: no stamp line found — lane shape changed, re-derive this guard")
        });
        assert!(
            guard_at < stamp_at,
            "{rel}: the HEAD-vs-stamp guard sits AFTER the stamp — it must refuse before \
             MRD_BUILD_SHA is written"
        );
    }
}

/// `cargo deny` shells out to `cargo metadata`, the one cargo call in the
/// pipeline that can rewrite the tracked Cargo.lock on a stale resolve
/// (deny.toml sets all-features=true, widening it). Every deny invocation must
/// carry --locked, like every other cargo call in the lane.
#[test]
fn cargo_deny_is_locked() {
    let body = without_comments(&read(".woodpecker/ci.yaml"));
    let deny_lines: Vec<&str> = body.lines().filter(|l| l.contains("cargo deny")).collect();
    assert!(
        !deny_lines.is_empty(),
        ".woodpecker/ci.yaml: no cargo deny invocation found — lane removed? re-derive this guard"
    );
    for line in deny_lines {
        assert!(
            line.contains("--locked"),
            ".woodpecker/ci.yaml: cargo deny without --locked can rewrite tracked \
             Cargo.lock mid-pipeline and red main from the publish step: {line}"
        );
    }
}
