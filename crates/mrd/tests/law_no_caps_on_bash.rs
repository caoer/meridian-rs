//! The executable gate for `docs/laws.md` § "Amendment — capabilities do not apply to bash".
//! The law is prose; this file is what makes it hold: a test that fails if a bash task ever
//! resolves a capability. That section names this file and this file names that section, so a
//! reader who proposes "just a small cap check on bash" meets both.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

/// The fixture: a bash block that declares nothing, a bash block whose frontmatter names a
/// capability anyway, a starlark block that declares nothing, and a bash block under the
/// `check-*` name convention (whose refusal is a NAME law, not a capability, and must
/// survive).
const PAGE: &str = "\
---
task.undeclared-bash: \"[[#^ub-1]]\"
task.claiming-bash: \"[[#^cb-1]]\"
task.claiming-bash.caps: md.set_field:nothing-like-this
task.undeclared-star: \"[[#^us-1]]\"
task.check-sh: \"[[#^chk-1]]\"
---

# Tasks

```bash
true
```
^ub-1

```bash
true
```
^cb-1

```starlark
def run(ctx):
    set_field(field = \"status\", value = \"starlark-wrote-this\")
```
^us-1

```bash
echo nope
```
^chk-1
";

/// Every spelling by which the engine could claim something about a bash
/// block's authority. None may appear on a bash surface.
const FORBIDDEN: [&str; 4] = ["caps", "deny-default", "read-only", "convention"];

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

    fn file(&self, rel: &str) -> PathBuf {
        self.tmp.path().join(rel)
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

/// The one `--list` line naming `task` (rows are one per line).
fn row(text: &str, task: &str) -> String {
    text.lines()
        .find(|l| l.trim_start().starts_with(task))
        .unwrap_or_else(|| panic!("no row for {task} in:\n{text}"))
        .to_owned()
}

/// Assert no capability claim appears in `surface`.
fn claims_nothing(surface: &str, what: &str) {
    for needle in FORBIDDEN {
        assert!(
            !surface.contains(needle),
            "{what} claims capability authority over bash — found {needle:?} in:\n{surface}"
        );
    }
}

// ── Half 1: no bash surface names a capability ──────────────────────────────

/// `--list` describes a bash task as a shell with undeclared effects, and says
/// nothing about capabilities — and no guarantee word: `unsandboxed` names a
/// sandbox that does not exist (ZT ruling, 2026-08-15). The starlark row on the
/// same page still carries its caps — the law is scoped to bash, not a deletion.
#[test]
fn list_row_for_bash_claims_no_capability() {
    let ws = Ws::new();
    let out = ws.run(&["tasks.md", "--list"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);

    for task in ["undeclared-bash", "claiming-bash"] {
        let line = row(&text, task);
        claims_nothing(&line, &format!("--list row for {task}"));
        assert!(!line.contains("unsandboxed"), "{line}");
        assert!(line.contains("effects: undeclared"), "{line}");
    }

    // Scoped, not deleted: starlark keeps its real contract on the same page.
    let star = row(&text, "undeclared-star");
    assert!(star.contains("hermetic"), "{star}");
    assert!(star.contains("caps"), "{star}");
}

/// `--list --json`: a bash row carries NO `caps` object. An absent key is the
/// only honest encoding — `null` or `[]` would still be an answer.
#[test]
fn list_json_for_bash_carries_no_caps_key() {
    let ws = Ws::new();
    let out = ws.run(&["tasks.md", "--list", "--json"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let v: Value = serde_json::from_str(&stdout(&out)).expect("json");
    let tasks = v["tasks"].as_array().expect("tasks array");
    let find = |name: &str| {
        tasks
            .iter()
            .find(|t| t["task"] == name)
            .unwrap_or_else(|| panic!("no {name} row"))
    };
    for task in ["undeclared-bash", "claiming-bash"] {
        let row = find(task);
        assert!(row.get("caps").is_none(), "bash row carries caps: {row}");
        assert_eq!(row["guarantee"], "unsandboxed", "{row}");
        assert_eq!(row["effects"], "undeclared", "{row}");
    }
    assert!(
        find("undeclared-star").get("caps").is_some(),
        "starlark lost its caps"
    );
}

/// The full run report for a bash task claims nothing about its authority.
#[test]
fn run_report_for_bash_claims_no_capability() {
    let ws = Ws::new();
    let out = ws.run(&["tasks.md", "undeclared-bash"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    claims_nothing(&text, "run report");
    // No `unsandboxed` token on the human face (ZT ruling, 2026-08-15).
    assert!(!text.contains("unsandboxed"), "{text}");
    assert!(text.contains("effects: undeclared"), "{text}");
}

/// `--json` of the same run: no `caps` object anywhere in the report.
#[test]
fn run_report_json_for_bash_carries_no_caps_key() {
    let ws = Ws::new();
    let out = ws.run(&["tasks.md", "undeclared-bash", "--json"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let v: Value = serde_json::from_str(&stdout(&out)).expect("json");
    assert!(v.get("caps").is_none(), "bash report carries caps: {v}");
    assert_eq!(v["effects"], "undeclared", "{v}");
}

/// `--dry` on bash shows the block and refuses to exec — and claims nothing
/// about what the block may do.
#[test]
fn dry_bash_claims_no_capability() {
    let ws = Ws::new();
    let out = ws.run(&["tasks.md", "undeclared-bash", "--dry"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let text = stdout(&out);
    claims_nothing(&text, "--dry report");
    // No `unsandboxed` token on the human face (ZT ruling, 2026-08-15).
    assert!(!text.contains("unsandboxed"), "{text}");
    assert!(text.contains("effects: undeclared"), "{text}");
}

// ── Half 2: no resolution runs underneath the silence ───────────────────────

/// A bash block has no effect channel at all (the effect-shim fd is deleted —
/// ZT ruling 2026-08-21): the run completes, and the governed tree is
/// byte-identical. There is nothing for a capability to govern.
#[test]
fn a_bash_block_has_no_effect_channel() {
    let ws = Ws::new();
    let out = ws.run(&["tasks.md", "undeclared-bash"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let page = std::fs::read_to_string(ws.file("tasks.md")).expect("page");
    assert_eq!(page, PAGE, "a bash run must not change the governed page");
}

/// A bash block whose frontmatter names a cap still runs exactly like any
/// other bash block. The declaration governs nothing and grants nothing — it
/// is not a narrower grant, it is not a grant at all.
#[test]
fn bash_frontmatter_cap_declaration_governs_nothing() {
    let ws = Ws::new();
    let out = ws.run(&["tasks.md", "claiming-bash"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let page = std::fs::read_to_string(ws.file("tasks.md")).expect("page");
    assert_eq!(page, PAGE, "a bash cap declaration must change nothing");
}

// ── The contract that stays real ────────────────────────────────────────────

/// Starlark is UNCHANGED: an undeclared blocks descriptor still refuses at the choke point,
/// exit 1, and nothing applies. The law narrows capabilities to starlark; it does not weaken
/// them there.
#[test]
fn starlark_capability_enforcement_is_untouched() {
    let ws = Ws::new();
    let before = std::fs::read_to_string(ws.file("tasks.md")).expect("page");
    let out = ws.run(&["tasks.md", "undeclared-star"]);
    assert_eq!(code(&out), 1, "{}", stderr(&out));
    assert!(
        stderr(&out).contains("capability denied"),
        "{}",
        stderr(&out)
    );
    let after = std::fs::read_to_string(ws.file("tasks.md")).expect("page");
    assert_eq!(before, after, "a refused starlark batch applied something");
}

/// The `check-*` / `verify-*` bash refusal is a NAME law, not a capability —
/// it survives the removal of cap resolution from the bash path.
#[test]
fn check_star_still_refuses_a_bash_fence() {
    let ws = Ws::new();
    let out = ws.run(&["tasks.md", "check-sh"]);
    assert_ne!(code(&out), 0, "{}", stdout(&out));
    assert!(
        stderr(&out).contains("read-only convention"),
        "{}",
        stderr(&out)
    );
}
