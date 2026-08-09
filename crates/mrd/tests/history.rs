//! Gates for `mrd test --history` (U1.6), driving the REAL binary
//! (`CARGO_BIN_EXE_mrd`) over a REAL git repo built in a tempdir, with the
//! `reviewer-not-owner` fixture rule PAGE as the law.
//!
//! The seeded history exercises all three fidelity classes and both golden-list
//! outcomes:
//! - **C0** create by `agent:bob` — before absent (a create) ⇒ class **A
//!   structural**; reviewer ≠ owner ⇒ passes.
//! - **C1** splice by `agent:alice` (the owner) — both sides recovered ⇒
//!   class **B full-bytes**; actor == owner ⇒ **would-refuse**.
//! - **C2** splice by `agent:bob` — both sides recovered ⇒ class **B**;
//!   reviewer ≠ owner ⇒ passes.
//! - **C3** a write to `tasks/ghost.md` whose bytes are not UTF-8 ⇒ class
//!   **C grey**; counted, never run.
//!
//! The invariants:
//! - an UNDECLARED would-refuse item (C1 absent from the spec page's
//!   `golden` fence) FAILS the run (exit 1);
//! - once declared there the run passes (exit 0) with the reason rendered;
//! - the class-C grey count is asserted;
//! - `mrd rules replay` no longer parses (the retired verb, decision #8).

use std::path::Path;
use std::process::{Command, Output};

/// The workspace-relative CHECK rule page the history gates calibrate.
const CHECK_RULE_PAGE: &str = "rules/reviewer-not-owner.md";

/// Its golden list — a SPEC page in the corpus tier's D2 shape, naming the rule it excepts
/// through a `rule:` reference. It is passed with `--spec`, never derived from the rule's path:
/// the relationship is declared, not positional.
const CHECK_GOLDEN_SPEC: &str = "specs/reviewer-not-owner.md";

/// What that spec's `rule:` reference spells, resolved from the spec's own
/// directory (`specs/`) back to the rule page.
const CHECK_SPEC_RULE_REF: &str = "../rules/reviewer-not-owner.md";

/// The workspace-relative HOOK rule page: a reaction refuses zero writes, so the
/// history tier over it must report zero would-refuse items.
const HOOK_RULE_PAGE: &str = "rules/task-status-notify.md";

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

/// Run the mrd binary with `args`, returning the raw output.
fn mrd(args: &[&str]) -> Output {
    Command::new(mrd_bin())
        .args(args)
        .output()
        .expect("spawn mrd")
}

fn stdout(out: &Output) -> String {
    String::from_utf8(out.stdout.clone()).expect("utf8 stdout")
}

fn stderr(out: &Output) -> String {
    String::from_utf8(out.stderr.clone()).expect("utf8 stderr")
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("exit code")
}

/// Run a git command in `dir`, asserting success.
fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Write `body` to `dir/rel`, creating parent directories.
fn write(dir: &Path, rel: &str, body: &str) {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, body).unwrap();
}

/// Commit everything in the working tree AS `author` — the acting writer, which
/// is the commit author now that history is git.
fn commit_as(dir: &Path, author: &str, message: &str) {
    git(dir, &["add", "-A"]);
    git(
        dir,
        &[
            "-c",
            &format!("user.name={author}"),
            "commit",
            "-q",
            "-m",
            message,
        ],
    );
}

/// The full commit id of `HEAD` — half of an item id (`<commit>:<path>`).
fn head(dir: &Path) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("spawn git");
    assert!(out.status.success(), "git rev-parse HEAD failed");
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

const FIX_OPEN: &str = "\
---
owner: agent:alice
status: open
---

# Fix parser

The dialect parser drops trailing anchors on the last line.
";

const FIX_CLOSED: &str = "\
---
owner: agent:alice
status: closed
---

# Fix parser

The dialect parser drops trailing anchors on the last line.
";

const FIX_CLOSED_NOTE: &str = "\
---
owner: agent:alice
status: closed
---

# Fix parser

The dialect parser drops trailing anchors on the last line.

Reviewed by agent:bob.
";

/// The four recorded writes this tier is calibrated over, by item id.
struct Seeded {
    dir: tempfile::TempDir,
    /// C1 — alice closing her own task: the would-refuse item.
    c1: String,
    /// C3 — a write whose bytes are not UTF-8: fidelity C grey.
    c3: String,
}

impl Seeded {
    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn path_str(&self) -> &str {
        self.dir.path().to_str().expect("utf-8 tmpdir")
    }
}

/// Build the seeded git workspace: four authored commits over one task page, plus
/// the fixture rule page. The actor is the commit author — history is git.
fn seeded_workspace() -> Seeded {
    let dir = tempfile::tempdir().expect("tmpdir");
    let ws = dir.path();

    git(ws, &["init", "-q", "-b", "main"]);
    git(ws, &["config", "user.email", "test@meridian.local"]);
    git(ws, &["config", "user.name", "mrd-test"]);

    // The fixture rule PAGE on disk (its exact committed bytes).
    write(
        ws,
        CHECK_RULE_PAGE,
        include_str!("corpus/rules/reviewer-not-owner.md"),
    );

    // C0 — bob creates fix-parser (owner alice). A create has no before side, so
    // it reconstructs at fidelity A structural.
    write(ws, "tasks/fix-parser.md", FIX_OPEN);
    commit_as(ws, "agent:bob", "C0 create fix-parser by bob");

    // C1 — alice (the OWNER) closes her own task. The would-refuse.
    write(ws, "tasks/fix-parser.md", FIX_CLOSED);
    commit_as(ws, "agent:alice", "C1 alice closes her own task");
    let c1 = format!("{}:tasks/fix-parser.md", head(ws));

    // C2 — bob (a reviewer) edits fix-parser. Passes: reviewer != owner.
    write(ws, "tasks/fix-parser.md", FIX_CLOSED_NOTE);
    commit_as(ws, "agent:bob", "C2 bob edits fix-parser");

    // C3 — a write whose bytes are NOT UTF-8: fidelity C grey. Neither side recovers as a
    // document, so the row is counted and rendered, never run.
    std::fs::write(ws.join("tasks/ghost.md"), [0xff_u8, 0xfe, 0x00, 0x01]).expect("write ghost");
    commit_as(ws, "agent:alice", "C3 a write whose bytes are not UTF-8");
    let c3 = format!("{}:tasks/ghost.md", head(ws));

    Seeded { dir, c1, c3 }
}

/// The undeclared would-refuse item (C1, alice's self-close) fails the run; the report shows it
/// failing, names the history span, counts every fidelity class, and the grey count is
/// asserted.
#[test]
fn undeclared_would_refuse_fails_the_run() {
    let seeded = seeded_workspace();
    let ws = seeded.path_str();

    let out = mrd(&["test", "--history", ws, "--rule", CHECK_RULE_PAGE]);
    let so = stdout(&out);

    assert_eq!(
        code(&out),
        1,
        "an undeclared would-refuse item is a finding (exit 1):\n{so}\n{}",
        stderr(&out)
    );

    // The would-refuse item is shown failing, cited by ITEM ID, and teaches the
    // rule.
    assert!(
        so.contains(&seeded.c1),
        "the firing item is named by <commit>:<path>:\n{so}"
    );
    assert!(
        so.contains("UNDECLARED would-refuse"),
        "the item is shown failing:\n{so}"
    );
    assert!(
        so.contains("reviewer must not be the owner"),
        "the refusal message is rendered:\n{so}"
    );

    // The report names its history span, first item id .. last.
    assert!(
        so.contains("history span:") && so.contains(&seeded.c3),
        "the report names its history span, ending at the last recorded write:\n{so}"
    );

    // Every fidelity class is counted; the class-C grey count is asserted.
    assert!(
        so.contains("B full-bytes=2"),
        "two rows reconstruct full-bytes:\n{so}"
    );
    assert!(
        so.contains("A structural=1"),
        "one row reconstructs structurally (the create):\n{so}"
    );
    assert!(
        so.contains("C grey=1"),
        "one row is grey — counted, never guessed:\n{so}"
    );
}

/// Declaring the item in the golden list (through the ordinary write door) turns
/// the finding into a pass, with the declared reason rendered.
#[test]
fn declared_item_passes_with_reason_rendered() {
    let seeded = seeded_workspace();
    let ws = seeded.path();
    let wss = seeded.path_str();

    // Triage: add the exception row to the spec's golden fence (an ordinary
    // in-tree edit through the write door).
    let item = &seeded.c1;
    let golden = format!(
        "\
---
rule: {CHECK_SPEC_RULE_REF}
---

# Golden list — reviewer-not-owner

```golden
- item={item} reason=\"legacy self-close predates the reviewer-not-owner rule\"
```
"
    );
    write(ws, CHECK_GOLDEN_SPEC, &golden);

    let out = mrd(&[
        "test",
        "--history",
        wss,
        "--rule",
        CHECK_RULE_PAGE,
        "--spec",
        CHECK_GOLDEN_SPEC,
    ]);
    let so = stdout(&out);

    assert_eq!(
        code(&out),
        0,
        "a declared would-refuse item passes (exit 0):\n{so}\n{}",
        stderr(&out)
    );
    assert!(
        so.contains("would-refuse — declared"),
        "the item is rendered declared:\n{so}"
    );
    assert!(
        so.contains("legacy self-close predates the reviewer-not-owner rule"),
        "the declared reason is rendered:\n{so}"
    );
    assert!(
        !so.contains("would-refuse — UNDECLARED"),
        "no undeclared finding remains:\n{so}"
    );
    assert!(
        so.contains("0 UNDECLARED would-refuse"),
        "the summary shows zero undeclared findings:\n{so}"
    );
    // Grey is still counted even on a clean run.
    assert!(
        so.contains("C grey=1"),
        "the grey count survives a clean run:\n{so}"
    );
}

/// The JSON surface carries the fidelity tally and per-row verdicts (§4 preamble:
/// `--json` alongside the human table).
#[test]
fn json_surface_carries_fidelity_and_verdicts() {
    let seeded = seeded_workspace();
    let ws = seeded.path_str();

    let out = mrd(&["test", "--history", ws, "--rule", CHECK_RULE_PAGE, "--json"]);
    let so = stdout(&out);
    let v: serde_json::Value = serde_json::from_str(&so).expect("json parses");

    assert_eq!(v["fidelity"]["full_bytes"], 2);
    assert_eq!(v["fidelity"]["structural"], 1);
    assert_eq!(v["fidelity"]["grey"], 1);
    assert_eq!(v["summary"]["undeclared"], 1);
    assert_eq!(v["history_span"]["last"], serde_json::json!(seeded.c3));
}

/// A golden exception row with no declared reason is a malformed golden list — a
/// declared exception must state why (exit 2, loud).
#[test]
fn hook_history_reports_zero_undeclared_over_an_exact_span() {
    let seeded = seeded_workspace();
    let ws = seeded.path();
    write(
        ws,
        HOOK_RULE_PAGE,
        include_str!("hook-tier/rules/task-status-notify.md"),
    );

    let out = mrd(&[
        "test",
        "--history",
        ws.to_str().unwrap(),
        "--rule",
        HOOK_RULE_PAGE,
        "--json",
    ]);
    let report: serde_json::Value =
        serde_json::from_str(&stdout(&out)).expect("HOOK history JSON parses");
    assert_eq!(
        code(&out),
        0,
        "a HOOK never invents a write refusal: {report}\n{}",
        stderr(&out)
    );
    assert_eq!(report["summary"]["undeclared"], 0);
    assert_eq!(report["history_span"]["last"], serde_json::json!(seeded.c3));
    assert_eq!(report["rule"], "task-status-notify");
    assert_eq!(report["rule_page"], HOOK_RULE_PAGE);

    let human = mrd(&[
        "test",
        "--history",
        ws.to_str().unwrap(),
        "--rule",
        HOOK_RULE_PAGE,
    ]);
    let human_stdout = stdout(&human);
    assert_eq!(code(&human), 0, "HOOK history human report passes");
    assert!(
        human_stdout.contains("history span:") && human_stdout.contains(&seeded.c3),
        "the report names the exact span: {human_stdout}"
    );
    assert!(
        human_stdout.contains("0 UNDECLARED would-refuse"),
        "the report names the zero-UNDECLARED verdict: {human_stdout}"
    );
}

#[test]
fn golden_exception_without_a_reason_is_refused() {
    let seeded = seeded_workspace();
    let ws = seeded.path();
    let wss = ws.to_str().unwrap();
    write(
        ws,
        CHECK_GOLDEN_SPEC,
        &format!(
            "---\nrule: {CHECK_SPEC_RULE_REF}\n---\n\n```golden\n- item=r-000002 no reason here\n```\n"
        ),
    );

    let out = mrd(&[
        "test",
        "--history",
        wss,
        "--rule",
        CHECK_RULE_PAGE,
        "--spec",
        CHECK_GOLDEN_SPEC,
    ]);
    assert_eq!(code(&out), 2, "a reasonless exception is a tool failure");
    assert!(
        stderr(&out).contains("reason"),
        "the refusal names the missing reason: {}",
        stderr(&out)
    );
}

/// Decision 8: `mrd rules replay` is retired — no alias, no shim. The verb no longer parses;
/// the CLI refuses it loudly (exit 2). The `rules` namespace has since been reassigned to the
/// effective-rules print verb (`mrd rules [PATH]`).
#[test]
fn mrd_rules_replay_no_longer_parses() {
    for retired in [
        vec!["rules", "replay", "--rules", "x", "--snapshots", "y"],
        vec!["rules", "replay"],
    ] {
        let out = mrd(&retired);
        assert_eq!(
            code(&out),
            2,
            "the retired verb is a tool failure: {retired:?}"
        );
        assert!(!stderr(&out).is_empty(), "the refusal is loud: {retired:?}");
        assert!(
            String::from_utf8_lossy(&out.stdout).is_empty(),
            "a retired form prints no rule set: {retired:?} gave {}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
}

/// D2a's load-bearing check: the spec declares which rule it excepts, and the runner verifies
/// that declaration instead of trusting the caller to pair them. A sibling bound by position
/// could never be wrong — and could never be checked either; naming the spec makes a
/// mispairing possible, so the mispairing must be refused.
#[test]
fn a_spec_naming_another_rule_is_refused() {
    let seeded = seeded_workspace();
    let ws = seeded.path();
    let wss = ws.to_str().unwrap();

    // A well-formed spec, with a real exception row — but it names the HOOK page
    // while the run calibrates the CHECK page.
    write(
        ws,
        CHECK_GOLDEN_SPEC,
        "---\nrule: ../rules/task-status-notify.md\n---\n\n\
         ```golden\n- item=r-000002 reason=\"declared against the wrong law\"\n```\n",
    );

    let out = mrd(&[
        "test",
        "--history",
        wss,
        "--rule",
        CHECK_RULE_PAGE,
        "--spec",
        CHECK_GOLDEN_SPEC,
    ]);
    let se = stderr(&out);
    assert_eq!(
        code(&out),
        2,
        "a spec that names another rule is a tool failure, not a silent empty list:\n{se}"
    );
    assert!(
        se.contains("rules/task-status-notify.md"),
        "the refusal names what the spec declared: {se}"
    );
    assert!(
        se.contains(CHECK_RULE_PAGE),
        "the refusal names what is actually being calibrated: {se}"
    );
}

/// A spec whose frontmatter declares no `rule:` is malformed, not empty. An
/// unattributed golden list would excuse findings for a law it never named.
#[test]
fn a_spec_with_no_rule_reference_is_refused() {
    let seeded = seeded_workspace();
    let ws = seeded.path();
    let wss = ws.to_str().unwrap();

    write(
        ws,
        CHECK_GOLDEN_SPEC,
        "# Golden list\n\n```golden\n- item=r-000002 reason=\"unattributed\"\n```\n",
    );

    let out = mrd(&[
        "test",
        "--history",
        wss,
        "--rule",
        CHECK_RULE_PAGE,
        "--spec",
        CHECK_GOLDEN_SPEC,
    ]);
    let se = stderr(&out);
    assert_eq!(
        code(&out),
        2,
        "an unattributed golden list is refused:\n{se}"
    );
    assert!(
        se.contains("rule:"),
        "the refusal names the missing reference: {se}"
    );
}

/// No `--spec` is the empty list — nothing declared yet, so the would-refuse item stands as a
/// finding. This is the pre-triage state, and it must stay reachable: the tier is usable before
/// anyone writes a spec page.
#[test]
fn without_a_spec_nothing_is_declared() {
    let seeded = seeded_workspace();
    let ws = seeded.path();
    let wss = ws.to_str().unwrap();

    let out = mrd(&[
        "test",
        "--history",
        wss,
        "--rule",
        CHECK_RULE_PAGE,
        "--json",
    ]);
    let report: serde_json::Value =
        serde_json::from_str(&stdout(&out)).expect("history JSON parses");
    assert_eq!(
        code(&out),
        1,
        "the undeclared would-refuse item is a finding"
    );
    assert_eq!(report["summary"]["undeclared"], 1);
    assert_eq!(report["summary"]["declared"], 0);
    assert!(
        report["golden_spec"].is_null(),
        "no spec is null, never an empty path: {report}"
    );
}
