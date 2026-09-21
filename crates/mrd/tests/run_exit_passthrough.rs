//! `--exit-passthrough` — the step's own exit code on the rc
//! (`docs/run-plane.md` § The CLI surface (locked)).
//!
//! The gap this closes: `mrd run` collapsed every nonzero step exit onto the
//! triad's findings leg, so a caller branching on `$?` could not tell a step
//! that ran and found something (exit 1, by a widespread convention) from one
//! that could not run at all (exit 2) — and a broken instrument reading as a
//! finding is the dangerous direction, because it looks like signal.
//!
//! The flag moves exactly one leg. These gates hold both halves: the code
//! arrives when asked for, and every leg the flag does NOT govern keeps its
//! own — above all the plane's own detection finding, which is mrd's claim
//! about a write the step made and never the step's exit to report.

use std::path::Path;
use std::process::{Command, Output};

/// One page of bash steps, one per exit this suite branches on, plus the
/// stray-write step (a detected delta beside a nonzero exit) and a starlark
/// step (no exit code to pass through at all).
const PAGE: &str = r#"---
task.clean: "[[#^clean-1]]"
task.finding: "[[#^finding-1]]"
task.fault: "[[#^fault-1]]"
task.high: "[[#^high-1]]"
task.stray: "[[#^stray-1]]"
task.stray.env: WS
task.hermetic: "[[#^hermetic-1]]"
---

# Tasks

```bash
echo clean
```
^clean-1

```bash
echo "the deploy has drifted"
exit 1
```
^finding-1

```bash
echo "the walk never happened"
exit 2
```
^fault-1

```bash
exit 42
```
^high-1

```bash
printf 'stray\n' > "$WS/stray.md"
exit 2
```
^stray-1

```starlark
def run(ctx):
    pass
```
^hermetic-1
"#;

struct Ws {
    tmp: tempfile::TempDir,
}

impl Ws {
    fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join("tasks.md"), PAGE).expect("page");
        Self { tmp }
    }

    fn path(&self) -> &Path {
        self.tmp.path()
    }

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

/// The default reading is untouched: both steps leave through the findings
/// leg, which is exactly the shape the issue measured.
#[test]
fn without_the_flag_a_finding_and_a_fault_share_one_code() {
    let ws = Ws::new();
    for task in ["finding", "fault", "high"] {
        let out = ws.run(&["tasks.md", task]);
        assert_eq!(code(&out), 1, "{task}: {}", stderr(&out));
    }
}

/// The code is on the report whether or not the flag is passed — the surface
/// a caller reads instead of parsing `receipts/run.md`.
///
/// Where on stdout is part of the claim: a bash step's own stdout streams
/// live, so the report is the LAST line of the stream and never the whole of
/// it. A caller piping `--json` straight into a JSON reader gets the step's
/// output first, which is the shape the doc states.
#[test]
fn the_report_carries_the_sealed_code_either_way() {
    let ws = Ws::new();
    let out = ws.run(&["tasks.md", "fault", "--json"]);
    assert_eq!(code(&out), 1, "{}", stderr(&out));
    let stream = stdout(&out);
    assert!(
        stream.starts_with("the walk never happened"),
        "the step's own stdout leads the stream: {stream}"
    );
    let report: serde_json::Value =
        serde_json::from_str(stream.lines().last().expect("a report line")).expect("report json");
    assert_eq!(report["exec"]["exit_code"], 2, "{report}");

    let text = ws.run(&["tasks.md", "fault"]);
    assert!(
        stdout(&text).contains("exec: exited 2"),
        "{}",
        stdout(&text)
    );
}

/// The ask itself: with the flag, the step's own code IS the rc, so a finding
/// and a broken instrument are two different values to a shell gate.
#[test]
fn passthrough_hands_the_step_its_own_code() {
    let ws = Ws::new();
    for (task, expected) in [("finding", 1), ("fault", 2), ("high", 42)] {
        let out = ws.run(&["tasks.md", task, "--exit-passthrough"]);
        assert_eq!(code(&out), expected, "{task}: {}", stderr(&out));
        // The rc is the step's, and the report still says so in band.
        assert!(
            stdout(&out).contains(&format!("exec: exited {expected}")),
            "{task}: {}",
            stdout(&out)
        );
    }
}

/// A clean step is clean under the flag too — passthrough is not a failure
/// mode, and 0 keeps meaning what it meant.
#[test]
fn a_clean_step_still_exits_zero_under_the_flag() {
    let ws = Ws::new();
    let out = ws.run(&["tasks.md", "clean", "--exit-passthrough"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
}

/// The reserved legs, by the one that matters most: the step wrote an md file
/// into the tree AND exited 2. The bracket's detection refusal is the plane's
/// own finding — reporting the step's 2 instead would hand a caller "the
/// instrument broke" for what is really "your step wrote outside the
/// governed path", and the named delta would read as the step's business.
#[test]
fn the_planes_own_finding_outranks_the_steps_code() {
    let ws = Ws::new();
    let ws_env = format!("WS={}", ws.path().display());
    let out = ws.run(&["tasks.md", "stray", "--env", &ws_env, "--exit-passthrough"]);
    assert_eq!(code(&out), 1, "{}", stderr(&out));
    let report = format!("{}{}", stdout(&out), stderr(&out));
    assert!(report.contains("stray.md"), "{report}");
    assert!(report.contains("exec window"), "{report}");
}

/// A starlark task has no exit code to pass through: the fence a task
/// declares is the page's fact, not argv's, so the flag cannot refuse there
/// and stands idle instead.
#[test]
fn passthrough_stands_idle_on_a_starlark_task() {
    let ws = Ws::new();
    let out = ws.run(&["tasks.md", "hermetic", "--exit-passthrough"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
}

/// Where argv itself says no step will exec, the flag refuses BY NAME rather
/// than being ignored — a silently dropped passthrough is the
/// guard-you-believe-is-armed trap, since the caller would read the plane's
/// own leg as the step's.
#[test]
fn passthrough_refuses_where_nothing_execs() {
    let ws = Ws::new();
    for tail in [
        vec!["tasks.md", "--list", "--exit-passthrough"],
        vec!["tasks.md", "fault", "--dry", "--exit-passthrough"],
        vec!["--load", "tasks.md", "--exit-passthrough"],
        vec!["tasks.md#^fault-1", "--exit-passthrough"],
    ] {
        let out = ws.run(&tail);
        assert_eq!(code(&out), 2, "{tail:?}: {}", stderr(&out));
        assert!(
            stderr(&out).contains("--exit-passthrough"),
            "{tail:?} refused without naming the flag: {}",
            stderr(&out)
        );
    }
}

/// An accepted flag no help page names is a flag a caller can only find by
/// reading the source.
#[test]
fn the_help_page_explains_the_flag() {
    let page = String::from_utf8_lossy(
        &Command::new(env!("CARGO_BIN_EXE_mrd"))
            .args(["run", "--help"])
            .output()
            .expect("spawn mrd")
            .stdout,
    )
    .into_owned();
    let options = page.split("options:").nth(1).unwrap_or_default();
    assert!(options.contains("--exit-passthrough"), "{page}");
}
