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
task.fix-note.caps: md.set_field
task.fix-note.args: value
task.fix-note.env: HOME_WIKI
task.fix-drift: \"[[#^fix-1]]\"
task.fix-drift.caps: md.set_field:status, md.append_section
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

/// `--list` names every task with language, guarantee class, contract, and — where
/// capabilities apply — its caps. Exit 0. The bash row reads `unsandboxed  effects:
/// undeclared` (`docs/laws.md` § Amendment — capabilities do not apply to bash).
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
        "unsandboxed",
        "effects: undeclared",
        "md.set_field",
        "env: HOME_WIKI",
    ] {
        assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
    }
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
    assert_eq!(fix_note["caps"]["effective"][0], "md.set_field");
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
/// § Amendment), so the surface states the two honest facts instead.
#[test]
fn dry_bash_shows_block_and_refuses_exec() {
    let ws = Ws::new();
    let out = ws.run(&["tasks.md", "fix-drift", "--dry"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("NOT executed"), "{text}");
    assert!(text.contains("touch pwned-by-fix-drift"), "{text}");
    assert!(text.contains("unsandboxed"), "{text}");
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
        stderr(&out).contains("capability denied: md.set_field on 'status'"),
        "the executor's own words, both tenses: {}",
        stderr(&out)
    );
    // Still a rehearsal: nothing landed.
    let after = std::fs::read_to_string(ws.file("nocaps.md")).expect("page");
    assert_eq!(before, after);
}
