//! E2e gates for `mrd run` (U10 half A — the U7-independent surface), driving the REAL binary
//! over its process boundary against a fixture workspace (`MERIDIAN_WORKSPACE` tier-1
//! override). Covers the LOCKED argv surface, the exit triad legs reachable pre-U7, `--list`,
//! and both `--dry` paths — starlark end-to-end effect truth, bash show-and-refuse.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

/// The multi-task fixture page: two starlark tasks, a bash task with a full
/// contract, and a bash task under the read-only `check-*` convention.
const PAGE: &str = "\
---
task.check-links: \"[[#^chk-1]]\"
task.fix-note: \"[[#^note-1]]\"
task.fix-note.caps: md.edit
task.fix-note.args: value
task.fix-note.env: HOME_WIKI
task.fix-drift: \"[[#^fix-1]]\"
task.fix-drift.caps: md.edit:tasks/*.md, md.create:tasks/*.md
task.check-sh: \"[[#^sh-1]]\"
---

# Tasks

```starlark
def run(ctx):
    pass
```
^chk-1

```starlark
def run(ctx):
    set_field(field = \"status\", value = ctx.args[0])
    notice(message = \"advisory\")
```
^note-1

```bash
touch pwned-by-fix-drift
```
^fix-1

```bash
echo nope
```
^sh-1
";

/// A page declaring exactly ONE task — the TASK-omitted single leg.
const SOLO_PAGE: &str = "\
---
task.solo: \"[[#^solo-1]]\"
---

# Tasks

```starlark
def run(ctx):
    pass
```
^solo-1
";

struct Ws {
    tmp: tempfile::TempDir,
}

impl Ws {
    fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join("tasks.md"), PAGE).expect("page");
        std::fs::write(tmp.path().join("solo.md"), SOLO_PAGE).expect("solo page");
        Self { tmp }
    }

    fn path(&self) -> &Path {
        self.tmp.path()
    }

    fn file(&self, rel: &str) -> PathBuf {
        self.tmp.path().join(rel)
    }

    /// Run `mrd run <args>` with the workspace pinned via the tier-1
    /// override, cwd inside the workspace.
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_mrd"))
            .arg("run")
            .args(args)
            .env("MERIDIAN_WORKSPACE", self.path())
            .current_dir(self.path())
            .output()
            .expect("spawn mrd")
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("exit code")
}

/// `--list` names every task with language, contract, and — where capabilities
/// apply — its caps. Exit 0. The bash row reads `effects: undeclared`, with NO
/// guarantee word: a class renders only where positive (`hermetic`) — there is
/// no sandbox, so `unsandboxed` names nothing (ZT ruling, 2026-08-15).
#[test]
fn list_shows_every_task_with_class_and_caps() {
    let ws = Ws::new();
    let out = ws.run(&["tasks.md", "--list"]);
    let text = stdout(&out);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    for needle in [
        "check-links",
        "fix-note",
        "fix-drift",
        "check-sh",
        "starlark",
        "hermetic",
        "effects: undeclared",
        "md.edit",
        "env: HOME_WIKI",
    ] {
        assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
    }
    assert!(!text.contains("unsandboxed"), "{text}");
    // check-sh is bash under check-*: the LIST shows its typed refusal
    // instead of hiding the row.
    assert!(text.contains("read-only convention"), "{text}");
}

/// `--list --json` is machine-readable with the same facts.
#[test]
fn list_json_shape() {
    let ws = Ws::new();
    let out = ws.run(&["tasks.md", "--list", "--json"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let v: Value = serde_json::from_str(&stdout(&out)).expect("json");
    assert_eq!(v["page"], "tasks.md");
    let tasks = v["tasks"].as_array().expect("tasks array");
    assert_eq!(tasks.len(), 4);
    let fix_note = tasks
        .iter()
        .find(|t| t["task"] == "fix-note")
        .expect("fix-note row");
    assert_eq!(fix_note["lang"], "starlark");
    assert_eq!(fix_note["guarantee"], "hermetic");
    assert_eq!(fix_note["caps"]["effective"][0], "md.edit");
    assert_eq!(fix_note["caps"]["source"], "explicit");
    assert_eq!(fix_note["args"][0], "value");
    assert_eq!(fix_note["env"][0], "HOME_WIKI");
}

/// TASK omitted on a many-task page: the tasks list prints and the exit is 2
/// — the CLI never guesses.
#[test]
fn task_omitted_many_lists_and_exits_2() {
    let ws = Ws::new();
    let out = ws.run(&["tasks.md"]);
    assert_eq!(code(&out), 2);
    assert!(stdout(&out).contains("fix-drift"), "{}", stdout(&out));
    assert!(stderr(&out).contains("name one"), "{}", stderr(&out));
}

/// TASK omitted on a single-task page resolves and RUNS that task — the
/// one-binding leg of the omission rule (§2.1: one runs, many refuse).
#[test]
fn task_omitted_single_runs_it() {
    let ws = Ws::new();
    let out = ws.run(&["solo.md"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("solo"), "{text}");
    assert!(text.contains("hermetic"), "{text}");
}

/// Unknown TASK → exit 2 with the declared names.
#[test]
fn unknown_task_exits_2() {
    let ws = Ws::new();
    let out = ws.run(&["tasks.md", "no-such"]);
    assert_eq!(code(&out), 2);
    assert!(
        stderr(&out).contains("no task 'no-such'"),
        "{}",
        stderr(&out)
    );
}

/// A contract violation exits 2 WITH the declared contract shown.
#[test]
fn contract_violation_shows_the_contract() {
    let ws = Ws::new();
    // fix-note declares args: [value], env: [HOME_WIKI]; supply neither.
    let out = ws.run(&["tasks.md", "fix-note", "--dry"]);
    assert_eq!(code(&out), 2);
    let err = stderr(&out);
    assert!(err.contains("takes 1 arg(s)"), "{err}");
    assert!(err.contains("declared contract"), "{err}");
    assert!(err.contains("HOME_WIKI"), "{err}");
}

/// Undeclared `--env` refuses (deny-by-default covers inputs) — exit 2.
#[test]
fn undeclared_env_exits_2() {
    let ws = Ws::new();
    let out = ws.run(&[
        "tasks.md",
        "fix-note",
        "--env",
        "HOME_WIKI=/w",
        "--env",
        "SNEAKY=x",
        "--dry",
        "--",
        "done",
    ]);
    assert_eq!(code(&out), 2);
    assert!(
        stderr(&out).contains("does not declare env 'SNEAKY'"),
        "{}",
        stderr(&out)
    );
}

/// `--dry` starlark: END-TO-END descriptor truth — the kernel evaluates, the
/// full effect set prints (md + proto), and NOTHING applies.
#[test]
fn dry_starlark_prints_effect_truth_applies_nothing() {
    let ws = Ws::new();
    let before = std::fs::read_to_string(ws.file("tasks.md")).expect("page");
    let out = ws.run(&[
        "tasks.md",
        "fix-note",
        "--env",
        "HOME_WIKI=/w",
        "--dry",
        "--json",
        "--",
        "done",
    ]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let v: Value = serde_json::from_str(&stdout(&out)).expect("json");
    assert_eq!(v["dry"], true);
    assert_eq!(v["applied"], false);
    assert_eq!(v["guarantee"], "hermetic");
    let effects = v["effects"].as_array().expect("effects");
    assert_eq!(effects.len(), 2, "full truth, never filtered");
    assert_eq!(effects[0]["kind"], "md.set_field");
    // The page is byte-identical: --dry applied nothing.
    let after = std::fs::read_to_string(ws.file("tasks.md")).expect("page");
    assert_eq!(before, after);
}

/// `--dry` bash: the block shows, exec is refused (exit 0, the refusal is the content) — and
/// the block demonstrably did not run. A bash task declares no capability (`docs/laws.md`
/// § Amendment), so the surface states `effects: undeclared` and nothing more.
#[test]
fn dry_bash_shows_block_and_refuses_exec() {
    let ws = Ws::new();
    let out = ws.run(&["tasks.md", "fix-drift", "--dry"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("NOT executed"), "{text}");
    assert!(text.contains("touch pwned-by-fix-drift"), "{text}");
    // No guarantee word for bash (ZT ruling, 2026-08-15): `effects:
    // undeclared` is the fact; a negation naming no real sandbox is not.
    assert!(!text.contains("unsandboxed"), "{text}");
    assert!(text.contains("effects: undeclared"), "{text}");
    // No descriptor fiction and no exec: the touch never happened.
    assert!(!ws.file("pwned-by-fix-drift").exists());
}

/// Bash under a `check-*` name refuses on the RUN leg (exit 1) — the
/// invocation is well-formed, the plane says no.
#[test]
fn bash_under_check_convention_exits_1() {
    let ws = Ws::new();
    let out = ws.run(&["tasks.md", "check-sh", "--dry"]);
    assert_eq!(code(&out), 1, "{}", stderr(&out));
    assert!(
        stderr(&out).contains("read-only convention 'check-*'"),
        "{}",
        stderr(&out)
    );
}

/// The locked surface refuses argv it does not define — exit 2.
#[test]
fn unknown_flag_exits_2() {
    let ws = Ws::new();
    let out = ws.run(&["tasks.md", "fix-note", "--force"]);
    assert_eq!(code(&out), 2);
    assert!(stderr(&out).contains("unknown flag"), "{}", stderr(&out));
}

/// A tail slot (`task.<name>.args: title, rows...`) takes any number of
/// trailing args, zero included, and every one of them reaches `ctx.args`.
#[test]
fn a_variadic_tail_reaches_ctx_args_at_any_count() {
    let ws = Ws::new();
    std::fs::write(
        ws.file("fmt.md"),
        "---\ntask.fmt: \"[[#^f-1]]\"\ntask.fmt.caps: md.edit\ntask.fmt.args: title, rows...\n---\n\n# Tasks\n\n```starlark\ndef run(ctx):\n    set_field(field = \"status\", value = \"|\".join(ctx.args))\n```\n^f-1\n",
    )
    .expect("fmt page");
    for (supplied, expected) in [
        (vec!["t"], "t"),
        (vec!["t", "r1"], "t|r1"),
        (vec!["t", "r1", "r2"], "t|r1|r2"),
    ] {
        let mut argv = vec!["fmt.md", "fmt", "--dry", "--json", "--"];
        argv.extend(supplied);
        let out = ws.run(&argv);
        assert_eq!(code(&out), 0, "{}", stderr(&out));
        let v: Value = serde_json::from_str(&stdout(&out)).expect("json");
        assert_eq!(v["effects"][0]["args"]["value"], expected);
    }
}

/// The tail is a floor, not a licence: the fixed prefix still refuses, and the
/// refusal echoes the declaration as written.
#[test]
fn a_variadic_tail_still_refuses_a_missing_fixed_arg() {
    let ws = Ws::new();
    std::fs::write(
        ws.file("fmt.md"),
        "---\ntask.fmt: \"[[#^f-1]]\"\ntask.fmt.args: title, rows...\n---\n\n# Tasks\n\n```starlark\ndef run(ctx):\n    pass\n```\n^f-1\n",
    )
    .expect("fmt page");
    let out = ws.run(&["fmt.md", "fmt", "--dry"]);
    assert_eq!(code(&out), 2);
    let err = stderr(&out);
    assert!(
        err.contains("takes at least 1 arg(s) (title, rows...)"),
        "{err}"
    );
    assert!(
        err.contains("declared contract: args: [title, rows...]"),
        "{err}"
    );
}

/// `--list` echoes a tail in the spelling it was declared with, on both faces.
#[test]
fn list_echoes_the_tail_suffix() {
    let ws = Ws::new();
    std::fs::write(
        ws.file("fmt.md"),
        "---\ntask.fmt: \"[[#^f-1]]\"\ntask.fmt.args: title, rows...\n---\n\n# Tasks\n\n```starlark\ndef run(ctx):\n    pass\n```\n^f-1\n",
    )
    .expect("fmt page");
    let text = stdout(&ws.run(&["fmt.md", "--list"]));
    assert!(text.contains("args: title, rows..."), "{text}");
    let v: Value =
        serde_json::from_str(&stdout(&ws.run(&["fmt.md", "--list", "--json"]))).expect("json");
    assert_eq!(v["tasks"][0]["args"][1], "rows...");
}

/// `--dry` rehearses the capability gate (dogfood r2 F2): a task that emits
/// an effect its authority denies refuses on the RUN leg exactly as the live
/// call would — a rehearsal that passes what live refuses predicts nothing.
#[test]
fn dry_rehearses_the_capability_gate() {
    let ws = Ws::new();
    std::fs::write(
        ws.file("nocaps.md"),
        "---\ntask.nocaps: \"[[#^n-1]]\"\n---\n\n# Tasks\n\n```starlark\ndef run(ctx):\n    set_field(field = \"status\", value = \"x\")\n```\n^n-1\n",
    )
    .expect("nocaps page");
    let before = std::fs::read_to_string(ws.file("nocaps.md")).expect("page");
    let out = ws.run(&["nocaps.md", "--dry"]);
    assert_eq!(code(&out), 1, "the choke point refuses the rehearsal too");
    assert!(
        stderr(&out).contains("capability denied: md.edit on 'nocaps.md'"),
        "the executor's own words, both tenses: {}",
        stderr(&out)
    );
    // Still a rehearsal: nothing landed.
    let after = std::fs::read_to_string(ws.file("nocaps.md")).expect("page");
    assert_eq!(before, after);
}

/// A page miss under an env-override workspace names the root AND its source
/// (dogfood F6): the exact command that answered minutes earlier misses after a
/// sticky `MERIDIAN_WORKSPACE` from another setup redirects the root — the
/// refusal must point at the root, not the ref. Repro shape: cwd inside a git
/// workspace that HAS the page, override naming a demo root that does not.
#[test]
fn page_miss_names_override_workspace_and_source() {
    let board = tempfile::tempdir().expect("board");
    std::fs::create_dir(board.path().join(".git")).expect("git entry");
    std::fs::write(board.path().join("queries.md"), SOLO_PAGE).expect("page");
    let demo = tempfile::tempdir().expect("demo");
    let out = Command::new(env!("CARGO_BIN_EXE_mrd"))
        .args(["run", "queries.md"])
        .env("MERIDIAN_WORKSPACE", demo.path())
        .current_dir(board.path())
        .output()
        .expect("spawn mrd");
    assert_eq!(code(&out), 2);
    let demo_canon = demo.path().canonicalize().expect("canon");
    let want = format!(
        "page not found: queries.md (workspace {}, source: env-override)",
        demo_canon.display()
    );
    assert!(stderr(&out).contains(&want), "{}", stderr(&out));
}

/// The disclosure is general: a git-root-anchored miss names that root and
/// `source: git-root` in the same form.
#[test]
fn page_miss_names_git_root_workspace_and_source() {
    let ws = tempfile::tempdir().expect("ws");
    std::fs::create_dir(ws.path().join(".git")).expect("git entry");
    let out = Command::new(env!("CARGO_BIN_EXE_mrd"))
        .args(["run", "missing.md"])
        .env_remove("MERIDIAN_WORKSPACE")
        .current_dir(ws.path())
        .output()
        .expect("spawn mrd");
    assert_eq!(code(&out), 2);
    let canon = ws.path().canonicalize().expect("canon");
    let want = format!(
        "page not found: missing.md (workspace {}, source: git-root)",
        canon.display()
    );
    assert!(stderr(&out).contains(&want), "{}", stderr(&out));
}

/// A committing one-task page for the receipt-key gate: `status` exists and
/// `mark` sets it, so every run commits one batch and mints one receipt line.
const CANON_PAGE: &str = "\
---
status: todo
task.mark: \"[[#^mark-1]]\"
task.mark.caps: md.edit
task.mark.args: value
---

# Tasks

```starlark
def run(ctx):
    set_field(field = \"status\", value = ctx.args[0])
```
^mark-1
";

/// §2.1's echo law at the argv boundary (run-plane.md § Record ↔ receipt
/// linkage), composed with the §1 door-family admission (wire-contract §12.1
/// line 878): one page owns ONE receipt key, and the two laws deliver it in
/// two halves. The workspace-relative spelling — the one grammar — runs and
/// receipts under the page's own key. The `./`-prefixed and machine-absolute
/// spellings violate §1, so the door refuses them BEFORE the rebind and they
/// mint NOTHING — a second key is not merely rewritten away, it is
/// unreachable from this face. (The rebind stays live as the wire door's
/// resolution seam; the CLI admission runs first, per the two cards' agreed
/// ordering.) Before the boundary, each spelling ran AND owned its own key,
/// and the histories could not see each other.
#[test]
fn receipt_page_key_is_canonical_across_argv_spellings() {
    let ws = Ws::new();
    std::fs::write(ws.file("canon.md"), CANON_PAGE).expect("canon page");

    // The one grammar: two committing runs under the workspace-relative key.
    for value in ["v0", "v1"] {
        let out = ws.run(&["canon.md", "mark", "--", value]);
        assert_eq!(code(&out), 0, "canon.md: {}", stderr(&out));
    }
    let receipts = std::fs::read_to_string(ws.file("receipts/run.md")).expect("receipt file");
    let count_after_admitted = receipts.lines().filter(|l| l.starts_with("- run ")).count();
    assert!(
        count_after_admitted >= 2,
        "two committed runs mint at least two receipt lines:\n{receipts}"
    );

    // The §1 refusals: neither spelling reaches execution, neither mints.
    let abs = ws.file("canon.md");
    for spelling in ["./canon.md", abs.to_str().expect("utf-8 path")] {
        let out = ws.run(&[spelling, "mark", "--", "vX"]);
        assert_eq!(code(&out), 2, "spelling {spelling}: {}", stderr(&out));
        assert!(
            stderr(&out).contains("the run door admits only workspace-relative"),
            "spelling {spelling} refuses in the family voice: {}",
            stderr(&out)
        );
    }
    let receipts = std::fs::read_to_string(ws.file("receipts/run.md")).expect("receipt file");
    let bodies: Vec<&str> = receipts
        .lines()
        .filter_map(|l| l.strip_prefix("- run "))
        .collect();
    assert_eq!(
        bodies.len(),
        count_after_admitted,
        "a refused spelling mints NO receipt line:\n{receipts}"
    );

    let pages: std::collections::BTreeSet<String> = bodies
        .iter()
        .map(|body| {
            let json = body.rsplit_once(" ^").map_or(*body, |(j, _)| j);
            serde_json::from_str::<Value>(json).expect("receipt json")["page"]
                .as_str()
                .expect("page fact")
                .to_owned()
        })
        .collect();
    assert_eq!(
        pages.into_iter().collect::<Vec<_>>(),
        vec!["canon.md".to_owned()],
        "one page owns ONE receipt key across argv spellings:\n{receipts}"
    );
}

/// A `cwd-default` miss stays bare: `Answer::root` is `None` there — a
/// defaulted cwd is not a workspace, and the refusal does not promote it to
/// one.
#[test]
fn page_miss_on_cwd_default_stays_bare() {
    let dir = tempfile::tempdir().expect("dir");
    let out = Command::new(env!("CARGO_BIN_EXE_mrd"))
        .args(["run", "missing.md"])
        .env_remove("MERIDIAN_WORKSPACE")
        .current_dir(dir.path())
        .output()
        .expect("spawn mrd");
    assert_eq!(code(&out), 2);
    let err = stderr(&out);
    assert!(err.contains("page not found: missing.md"), "{err}");
    assert!(!err.contains("(workspace"), "{err}");
}
