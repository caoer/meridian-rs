//! A bash step's stderr is the caller's stderr (`docs/run-plane.md` § The CLI
//! surface (locked)).
//!
//! The gap this closes: the bracket captured a step's stderr and nothing read
//! it — not the terminal, not the run log, not the report — so an instrument
//! that reported its findings on stderr reported to no one (home-wiki's
//! `SKILLS.md check`: rc 1, 0 bytes of stdout, its `ERROR:` lines lost).

use std::path::Path;
use std::process::{Command, Output};

const PAGE: &str = r#"---
task.finding: "[[#^finding-1]]"
task.quiet: "[[#^quiet-1]]"
---

# Tasks

```bash
echo "on stdout"
echo "ERROR: the finding, on stderr" >&2
exit 1
```
^finding-1

```bash
echo "note: a warning on a clean run" >&2
```
^quiet-1
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

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// The step's stderr reaches mrd's stderr, ahead of mrd's own verdict line,
/// and stays out of stdout: stdout is the step's output followed by the report.
#[test]
fn a_failing_steps_stderr_reaches_the_caller_before_the_verdict() {
    let ws = Ws::new();
    let out = ws.run(&["tasks.md", "finding"]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stderr));
    let err = text(&out.stderr);
    let at = err
        .find("ERROR: the finding, on stderr")
        .unwrap_or_else(|| panic!("the step's stderr is on mrd's stderr: {err}"));
    let verdict = err.find("mrd: ").expect("mrd's own verdict line");
    assert!(
        at < verdict,
        "the step's lines precede mrd's verdict: {err}"
    );
    let stdout = text(&out.stdout);
    assert!(stdout.starts_with("on stdout"), "{stdout}");
    assert!(
        !stdout.contains("on stderr"),
        "stderr never lands on stdout: {stdout}"
    );
}

/// A clean step's stderr is forwarded too, and `--json` keeps stdout's last
/// line a parseable report.
#[test]
fn a_clean_steps_stderr_is_forwarded_and_json_stdout_stays_clean() {
    let ws = Ws::new();
    let out = ws.run(&["tasks.md", "quiet", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(
        text(&out.stderr).contains("note: a warning on a clean run"),
        "{}",
        text(&out.stderr)
    );
    let stdout = text(&out.stdout);
    let report: serde_json::Value =
        serde_json::from_str(stdout.lines().last().expect("a report line")).expect("report json");
    assert_eq!(report["exec"]["exit_code"], 0, "{report}");
}
