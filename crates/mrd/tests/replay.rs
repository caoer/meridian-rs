//! Gates for `mrd rules replay`, driving the REAL binary (`CARGO_BIN_EXE_mrd`).
//!
//! Two sources are covered: the committed synthetic snapshot corpus (the CI
//! lane's fixture) and a live git history built in a tempdir. The invariants:
//! replay is deterministic (same history twice ⇒ byte-identical report), a
//! deliberately-dead rule is reported, and a live rule shows its exact fire
//! count.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

/// The committed synthetic corpus that ships with the crate.
fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/replay-corpus")
}

fn run(args: &[&str]) -> Output {
    Command::new(mrd_bin())
        .args(args)
        .output()
        .expect("spawn mrd")
}

fn stdout(out: &Output) -> String {
    String::from_utf8(out.stdout.clone()).expect("utf8 stdout")
}

/// Replay the committed snapshot corpus, returning the markdown report.
fn replay_snapshots() -> Output {
    let corpus = corpus();
    let snapshots = corpus.join("snapshots");
    let rules = corpus.join("rules");
    run(&[
        "rules",
        "replay",
        "--snapshots",
        snapshots.to_str().unwrap(),
        "--rules",
        rules.to_str().unwrap(),
    ])
}

#[test]
fn snapshot_replay_is_deterministic() {
    let a = replay_snapshots();
    let b = replay_snapshots();
    assert!(a.status.success(), "first replay failed");
    assert!(b.status.success(), "second replay failed");
    // Determinism inherited from the kernel: same history ⇒ byte-identical report.
    assert_eq!(
        stdout(&a),
        stdout(&b),
        "replay of the same history must be byte-identical"
    );
}

#[test]
fn snapshot_replay_detects_the_dead_rule_and_counts_the_live_ones() {
    let out = replay_snapshots();
    assert!(out.status.success(), "replay failed: {}", stdout(&out));
    let report = stdout(&out);

    // The deliberately-dead rule is reported under "Dead rules (never fired)".
    let dead_section = report
        .split("## Dead rules")
        .nth(1)
        .expect("a dead-rules section");
    let dead_body = dead_section
        .split("## Per-rule")
        .next()
        .expect("dead-rules body");
    assert!(
        dead_body.contains("`dead_priority`"),
        "dead_priority must be listed as never-fired; got:\n{dead_body}"
    );
    // ...and only that one is dead.
    assert!(report.contains("dead_rules: 1"), "exactly one dead rule");
    assert!(report.contains("fired_rules: 2"), "two rules fired");

    // The live rules show their exact fire counts (3 events each over the corpus).
    assert!(
        report.contains("| `live_status` | 3 | 3 | 0 |"),
        "live_status fires 3× emitting 3 effects; got:\n{report}"
    );
    assert!(
        report.contains("| `live_section` | 3 | 3 | 0 |"),
        "live_section fires 3× emitting 3 effects; got:\n{report}"
    );

    // Effect-kind distribution reflects the two live rules' descriptor kinds.
    assert!(report.contains("| `proto.notice` | 3 |"));
    assert!(report.contains("| `proto.warn` | 3 |"));
}

#[test]
fn missing_rules_dir_is_a_tool_failure() {
    let out = run(&[
        "rules",
        "replay",
        "--snapshots",
        corpus().join("snapshots").to_str().unwrap(),
    ]);
    assert!(!out.status.success(), "missing --rules must fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--rules"), "diagnostic names --rules");
}

// --- git source ------------------------------------------------------------

fn git(repo: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn commit_state(repo: &Path, file: &str, body: &str, message: &str) {
    let path = repo.join(file);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("mkdir");
    }
    std::fs::write(&path, body).expect("write doc");
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-q", "-m", message]);
}

#[test]
fn git_replay_synthesizes_events_from_real_history() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path().join("wiki");
    std::fs::create_dir_all(&repo).expect("repo dir");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "t@example.com"]);
    git(&repo, &["config", "user.name", "t"]);
    git(&repo, &["config", "commit.gpgsign", "false"]);

    // Two commits: create a doc, then move its `status`.
    commit_state(
        &repo,
        "plan.md",
        "---\ntitle: Plan\nstatus: todo\n---\n# Plan\n\nBody.\n",
        "add plan",
    );
    commit_state(
        &repo,
        "plan.md",
        "---\ntitle: Plan\nstatus: done\n---\n# Plan\n\nBody.\n",
        "advance status",
    );

    let rules = corpus().join("rules");
    let out = run(&[
        "rules",
        "replay",
        repo.to_str().unwrap(),
        "--rules",
        rules.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "git replay failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report = stdout(&out);

    // Source + range come from git; the status rule fires twice (creation + the
    // status transition), and the dead rule stays dead over real history too.
    assert!(report.contains("source: git"), "git source; got:\n{report}");
    assert!(
        report.contains("| `live_status` | 2 |"),
        "live_status fires on creation + transition; got:\n{report}"
    );
    let dead_body = report
        .split("## Dead rules")
        .nth(1)
        .and_then(|s| s.split("## Per-rule").next())
        .expect("dead-rules body");
    assert!(
        dead_body.contains("`dead_priority`"),
        "dead_priority stays dead over git history"
    );
}
