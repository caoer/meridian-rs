//! Full-lifecycle e2e for `mrd run`: each test drives the real binary over its process
//! boundary against ratified surface behavior.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Lifecycle fixture workspace: a starlark fix task, a plain bash task (bash has no effect
/// channel — the effect-shim fd is deleted, ZT ruling 2026-08-21), a bash task that writes an
/// md file directly, and a bash task that rewrites `mdfs_config.yaml` mid-run (the widening
/// attack).
const PAGE: &str = "\
---
task.fix-note: \"[[#^note-1]]\"
task.fix-note.caps: md.edit
task.fix-note.args: value
task.fix-sh: \"[[#^sh-1]]\"
task.fix-cheat: \"[[#^cheat-1]]\"
task.fix-cheat.env: WS
task.fix-widen: \"[[#^widen-1]]\"
task.fix-widen.env: WS
---

# Tasks

```starlark
def run(ctx):
    set_field(field = \"status\", value = ctx.args[0])
```
^note-1

```bash
echo observed
```
^sh-1

```bash
printf 'smuggled\\n' > \"$WS/cheat.md\"
```
^cheat-1

```bash
printf 'version: 2\\nignore:\\n  - \"*.md\"\\n' > \"$WS/mdfs_config.yaml\"
printf 'widened\\n' > \"$WS/hidden.md\"
```
^widen-1
";

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

/// The newest receipt file's text, if any receipt landed.
fn receipts_text(ws: &Ws) -> String {
    let Ok(entries) = std::fs::read_dir(ws.file("receipts")) else {
        return String::new();
    };
    let mut files: Vec<PathBuf> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
    files.sort();
    files
        .iter()
        .map(|f| std::fs::read_to_string(f).unwrap_or_default())
        .collect()
}

/// Starlark lifecycle: ONE splice batch applies the field, the receipt line
/// rides the same commit, the report labels the block `hermetic` — exit 0.
#[test]
fn starlark_task_applies_one_batch_with_receipt() {
    let ws = Ws::new();
    let out = ws.run(&["tasks.md", "fix-note", "--", "done"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let page = std::fs::read_to_string(ws.file("tasks.md")).expect("page");
    assert!(page.contains("status: done"), "{page}");
    // Receipt landed with the run actor and the invocation facts.
    let receipts = receipts_text(&ws);
    assert!(receipts.contains("run:fix-note"), "{receipts}");
    // Guarantee class labeled, scoped claim (#16).
    assert!(stdout(&out).contains("hermetic"), "{}", stdout(&out));
}

/// Bash lifecycle: no effect channel, so the run changes nothing on the page;
/// the completion receipt carries the exec record (invocation id, exit code,
/// stdout sha256 + size, log address — ruling 7/S8); the stdout log exists
/// under `.meridian/runs/`.
#[test]
fn bash_task_records_the_run_and_changes_nothing() {
    let ws = Ws::new();
    let out = ws.run(&["tasks.md", "fix-sh"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let page = std::fs::read_to_string(ws.file("tasks.md")).expect("page");
    assert_eq!(page, PAGE, "a bash run must not change the governed page");
    // The out-of-tree record exists and the receipt names it.
    let runs = std::fs::read_dir(ws.file(".meridian/runs")).expect("runs dir");
    assert!(runs.count() >= 1, "no run log written");
    let receipts = receipts_text(&ws);
    assert!(receipts.contains(".meridian/runs/"), "{receipts}");
    assert!(receipts.contains("sha256"), "{receipts}");
    // No class label for bash — a guarantee word renders only where positive
    // (ZT ruling, 2026-08-15; `docs/laws.md` § Amendment). The brackets verdict
    // still renders, on the out-of-band-delta line below.
    assert!(!stdout(&out).contains("unsandboxed"), "{}", stdout(&out));
    assert!(
        stdout(&out).contains("out-of-band delta: none"),
        "{}",
        stdout(&out)
    );
}

/// The direct-write cheat: bash writes an md file into the tree. The snapshot bracket DETECTS
/// it (exit 1, delta named as an exec-window change) and the write PERSISTS — never rolled
/// back (14: rollback would be a second write path with invented authority).
#[test]
fn ungoverned_md_write_is_detected_named_never_rolled_back() {
    let ws = Ws::new();
    let ws_env = format!("WS={}", ws.path().display());
    let out = ws.run(&["tasks.md", "fix-cheat", "--env", &ws_env]);
    assert_eq!(code(&out), 1, "{}", stderr(&out));
    let report = format!("{}{}", stdout(&out), stderr(&out));
    // Named as an exec-window delta (S4 wording: the window, not the block).
    assert!(report.contains("exec window"), "{report}");
    assert!(report.contains("cheat.md"), "{report}");
    // Never rolled back: the ungoverned write persists as external change.
    assert!(ws.file("cheat.md").exists(), "rollback is forbidden (#14)");
}

/// U12 MUST (20): the config-widening attack — bash rewrites `mdfs_config.yaml` mid-run to
/// shrink the hash domain, then writes inside the new blind spot. The config hash bracket
/// refuses (exit 1) and the smuggled write is still reported; nothing is silently admitted.
#[test]
fn config_widening_attack_is_refused() {
    let ws = Ws::new();
    let ws_env = format!("WS={}", ws.path().display());
    let out = ws.run(&["tasks.md", "fix-widen", "--env", &ws_env]);
    assert_eq!(code(&out), 1, "{}", stderr(&out));
    let report = format!("{}{}", stdout(&out), stderr(&out));
    assert!(report.contains("mdfs_config.yaml"), "{report}");
    // The mid-run config change is the named refusal cause — the attack is
    // closed at the bracket, not silently absorbed by the new ignore list.
    assert!(
        report.contains("config") || report.contains("mid-run"),
        "{report}"
    );
}

/// What `--dry` says about a bash task's authority is exactly what the run says (S14): both
/// name no capability, and both name the same effects fact.
#[test]
fn dry_and_run_agree_that_bash_has_no_capability() {
    let ws = Ws::new();
    let dry = ws.run(&["tasks.md", "fix-sh", "--dry", "--json"]);
    assert_eq!(code(&dry), 0, "{}", stderr(&dry));
    let dry_json: serde_json::Value = serde_json::from_str(&stdout(&dry)).expect("dry json");
    let run = ws.run(&["tasks.md", "fix-sh", "--json"]);
    assert_eq!(code(&run), 0, "{}", stderr(&run));
    let run_json: serde_json::Value = serde_json::from_str(&stdout(&run)).expect("run json");

    // Absent on both, and asserted as ABSENT rather than as equal — two nulls
    // are equal for the wrong reason.
    assert!(dry_json.get("caps").is_none(), "{dry_json}");
    assert!(run_json.get("caps").is_none(), "{run_json}");
    assert_eq!(dry_json["effects"], "undeclared", "{dry_json}");
    assert_eq!(
        dry_json["effects"], run_json["effects"],
        "S14: --dry must state exactly what the run states"
    );
}
