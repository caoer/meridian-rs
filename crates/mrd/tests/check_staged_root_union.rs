//! **The staged interval's roots are in the set `mrd check` builds** (W5).
//!
//! W5 narrowed `check`'s mount corpora to the roots its own lock addresses name.
//! `check --staged` assesses TWO corpora against ONE table (F1, the second
//! interval), so the needed set has to be the UNION over both — and the failure
//! mode of getting that wrong is silent-looking rather than loud: the staged
//! corpus's cross-root pin would resolve into a root whose corpus this process
//! never built.
//!
//! # Why an absent target is the sharp instrument here, and no fingerprint is minted
//! `model::CorpusIndex::resolve_ref` distinguishes the two states this gate is
//! about *before* it ever compares bytes:
//!
//! - root bound **and its corpus loaded** → the miss is a measured absence
//!   inside that root (`NotFound { root: Some(..) }`);
//! - root bound and **its corpus NOT loaded** → `PathUnseeable`, carrying the
//!   named refusal *"the mount table binds this root, but no corpus for it was
//!   loaded in this process"*.
//!
//! So a cross-root pin at a page that does not exist separates "the union
//! included this root" from "the union missed it" exactly, and it does so
//! without minting a fingerprint over transplanted bytes (the heavier recipe
//! `f6_check_sees_the_mount_table.rs` needs, because it measures COLOUR). This
//! gate measures REACHABILITY, which is the thing the union can break.
//!
//! That refusal string is also the whole safety argument for narrowing at all:
//! an under-collecting caller gets a named refusal, never a false green. This
//! file is the gate that keeps that promise honest for the one caller whose
//! needed set spans two corpora.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The exact refusal `resolve_ref` raises when a root is bound but this process
/// holds no corpus for it — the symptom of an under-collecting union, and the
/// string this gate exists to prove absent.
const NO_CORPUS_REFUSAL: &str = "no corpus for it";

fn mrd_bin() -> PathBuf {
    std::env::var_os("MRD_BIN")
        .map_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_mrd")), PathBuf::from)
}

fn run(home: &Path, cache: &Path, config: &Path, cwd: &Path, args: &[&str]) -> Output {
    Command::new(mrd_bin())
        .args(args)
        .current_dir(cwd)
        .env("HOME", home)
        .env("XDG_CACHE_HOME", cache)
        .env("MERIDIAN_CONFIG", config)
        .env_remove("MERIDIAN_WORKSPACE")
        .output()
        .expect("spawn mrd")
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("LC_ALL", "C")
        .env("GIT_AUTHOR_NAME", "w5")
        .env("GIT_AUTHOR_EMAIL", "w5@example.invalid")
        .env("GIT_COMMITTER_NAME", "w5")
        .env("GIT_COMMITTER_EMAIL", "w5@example.invalid")
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn staged_covers_a_root_the_worktree_does_not() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let cache = tmp.path().join("xdg-cache");
    let other = tmp.path().join("other");
    let ws = tmp.path().join("ws");
    for dir in [&home, &other, &ws] {
        std::fs::create_dir_all(dir).expect("mkdir");
    }

    // The mounted root declares its own canonical name (INV-5) — without this
    // the bind renders undeclared and every reading below is vacuous.
    std::fs::write(
        other.join("MERIDIAN.md"),
        "---\ntype: meridian-root\nversion: 1\nname: other\n---\n\n# Other root\n",
    )
    .expect("root declaration");
    std::fs::write(other.join("present.md"), "# Present\n\nreal page.\n").expect("page");

    let config = home.join("MERIDIAN.md");
    std::fs::write(
        &config,
        format!(
            "---\ntype: meridian-config\nversion: 1\n---\n\n# Test roots\n\n\
             ```meridian-mount\nname: other\npath: {}\nkind: vault\nvault: othervault\n```\n",
            other.display()
        ),
    )
    .expect("mount table");

    let init = run(&home, &cache, &config, &ws, &["init"]);
    assert!(
        init.status.success(),
        "init: {}",
        String::from_utf8_lossy(&init.stderr)
    );

    // The WORKTREE names no root at all. This is the asymmetry the gate needs:
    // a needed-set computed from the worktree alone collects nothing, so if the
    // union is wrong the staged pin below has no corpus to resolve into.
    git(&ws, &["init"]);
    std::fs::write(
        ws.join("plain.md"),
        "# Plain\n\n## Body\n\nnames no root.\n",
    )
    .expect("plain");
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-m", "plain"]);

    // The STAGED interval, and only it, carries the cross-root address. The
    // target does not exist inside `other` — see the header for why that is the
    // sharp instrument rather than a weaker fixture.
    std::fs::write(
        ws.join("claim.md"),
        "# Claim\n\n## Body\n\ndrawn from another root.\n\n\
         ```meridian-lock\nversion: 2\npins:\n  - object: \"[[other:absent]]\"\n    \
         hash: \"0000000000000000000000000000000000000000\"\n    path: []\n    \
         fingerprint: \"sha256:0000000000000000000000000000000000000000000000000000000000000000\"\n```\n",
    )
    .expect("claim page");
    git(&ws, &["add", "claim.md"]);
    // Restore the worktree so the two intervals genuinely DIVERGE: the index
    // carries the cross-root pin and the worktree does not.
    std::fs::remove_file(ws.join("claim.md")).expect("remove from the worktree");

    let out = run(
        &home,
        &cache,
        &config,
        &ws,
        &["check", "--staged", "--json"],
    );
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    assert!(
        !text.contains(NO_CORPUS_REFUSAL),
        "the staged interval names root `other`, so `other` must be in the set \
         `mrd check` builds corpora for. It answered with the under-collect \
         refusal instead — the needed set was computed from the worktree corpus \
         alone and missed the staged one (W5). It read:\n{text}"
    );
}
