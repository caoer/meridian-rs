//! U12 full-lifecycle e2e scaffolding for `mrd run` — the suite that
//! ACTIVATES WHEN U7 LANDS (the runner compose / execute leg). Every test is
//! `#[ignore]` with that reason; activation is deleting the attribute, not
//! writing the suite under review pressure. Assertions bind only to ratified
//! surface behavior (plan §3 decisions + verdict rulings), never to
//! in-flight internals: each test drives the REAL binary over its process
//! boundary.
//!
//! Coverage map (plan §4 U12): starlark + bash apply via ONE splice batch;
//! deny-by-default caps at the choke; the snapshot bracket catching an
//! ungoverned md write (named, never rolled back — #14); the
//! config-widening attack REFUSED (#20, a U12 MUST); guarantee-class labels
//! (#16/#23); receipt + run-record linkage (ruling 7, S8).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Lifecycle fixture workspace: a starlark fix task, a bash fix task that
/// emits one governed descriptor over the effect-shim fd, a bash task with
/// NO caps that still tries to emit, a bash task that writes an md file
/// directly (zero descriptors), and a bash task that rewrites
/// `mdfs_config.yaml` mid-run (the widening attack).
const PAGE: &str = "\
---
task.fix-note: \"[[#^note-1]]\"
task.fix-note.caps: md.set_field
task.fix-note.args: value
task.fix-shim: \"[[#^shim-1]]\"
task.fix-shim.caps: md.set_field:status
task.fix-uncapped: \"[[#^uncap-1]]\"
task.fix-cheat: \"[[#^cheat-1]]\"
task.fix-widen: \"[[#^widen-1]]\"
---

# Tasks

```starlark
def run(ctx):
    set_field(field = \"status\", value = ctx.args[0])
```
^note-1

```bash
p='{\"kind\":\"md.set_field\",\"args\":{\"field\":\"status\",\"value\":\"shimmed\"}}'
printf '%s:%s\\n' \"${#p}\" \"$p\" >&\"$MD_EFFECT_FD\"
printf 'end:1\\n' >&\"$MD_EFFECT_FD\"
```
^shim-1

```bash
p='{\"kind\":\"md.set_field\",\"args\":{\"field\":\"status\",\"value\":\"denied\"}}'
printf '%s:%s\\n' \"${#p}\" \"$p\" >&\"$MD_EFFECT_FD\"
printf 'end:1\\n' >&\"$MD_EFFECT_FD\"
```
^uncap-1

```bash
printf 'smuggled\\n' > \"$MRD_WORKSPACE_ROOT/cheat.md\"
```
^cheat-1

```bash
printf 'version: 2\\nignore:\\n  - \"*.md\"\\n' > \"$MRD_WORKSPACE_ROOT/mdfs_config.yaml\"
printf 'widened\\n' > \"$MRD_WORKSPACE_ROOT/hidden.md\"
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
#[ignore = "U7 execute leg pending — activate when the runner lands"]
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

/// Bash lifecycle: the shim descriptor applies through the SAME choke point
/// and splice batch; the receipt carries the exec record (invocation id,
/// exit code, stdout sha256 + size, log address — ruling 7/S8); the stdout
/// log exists under `.meridian/runs/`.
#[test]
#[ignore = "U7 execute leg pending — activate when the runner lands"]
fn bash_task_applies_via_shim_with_run_record() {
    let ws = Ws::new();
    let out = ws.run(&["tasks.md", "fix-shim"]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    let page = std::fs::read_to_string(ws.file("tasks.md")).expect("page");
    assert!(page.contains("status: shimmed"), "{page}");
    // The out-of-tree record exists and the receipt names it.
    let runs = std::fs::read_dir(ws.file(".meridian/runs")).expect("runs dir");
    assert!(runs.count() >= 1, "no run log written");
    let receipts = receipts_text(&ws);
    assert!(receipts.contains(".meridian/runs/"), "{receipts}");
    assert!(receipts.contains("sha256"), "{receipts}");
    // Detected class labeled — and only because U6b's detection landed (#23).
    assert!(stdout(&out).contains("detected"), "{}", stdout(&out));
}

/// Deny-by-default at the choke: an undeclared block's descriptor refuses
/// (exit 1), and NOTHING applies — the batch is atomic.
#[test]
#[ignore = "U7 execute leg pending — activate when the runner lands"]
fn deny_by_default_refuses_undeclared_descriptor() {
    let ws = Ws::new();
    let before = std::fs::read_to_string(ws.file("tasks.md")).expect("page");
    let out = ws.run(&["tasks.md", "fix-uncapped"]);
    assert_eq!(code(&out), 1, "{}", stderr(&out));
    assert!(stderr(&out).contains("capability denied"), "{}", stderr(&out));
    let after = std::fs::read_to_string(ws.file("tasks.md")).expect("page");
    assert_eq!(before, after, "refusal must apply nothing");
}

/// The zero-descriptor cheat: bash writes an md file directly. The snapshot
/// bracket DETECTS it (exit 1, delta named as an exec-window change) and the
/// write PERSISTS — never rolled back (#14: rollback would be a second write
/// path with invented authority).
#[test]
#[ignore = "U7 execute leg pending — activate when the runner lands"]
fn ungoverned_md_write_is_detected_named_never_rolled_back() {
    let ws = Ws::new();
    let out = ws.run(&["tasks.md", "fix-cheat"]);
    assert_eq!(code(&out), 1, "{}", stderr(&out));
    let report = format!("{}{}", stdout(&out), stderr(&out));
    // Named as an exec-window delta (S4 wording: the window, not the block).
    assert!(report.contains("exec window"), "{report}");
    assert!(report.contains("cheat.md"), "{report}");
    // Never rolled back: the ungoverned write persists as external change.
    assert!(ws.file("cheat.md").exists(), "rollback is forbidden (#14)");
}

/// U12 MUST (#20): the config-widening attack — bash rewrites
/// `mdfs_config.yaml` mid-run to shrink the hash domain, then writes inside
/// the new blind spot. The config hash bracket refuses (exit 1) and the
/// smuggled write is still reported; nothing is silently admitted.
#[test]
#[ignore = "U7 execute leg pending — activate when the runner lands"]
fn config_widening_attack_is_refused() {
    let ws = Ws::new();
    let out = ws.run(&["tasks.md", "fix-widen"]);
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

/// S14: the caps `--dry` displays are byte-identical to the caps the real
/// run enforces at the choke point — compare the two renderings.
#[test]
#[ignore = "U7 execute leg pending — activate when the runner lands"]
fn dry_caps_are_byte_identical_to_choke_caps() {
    let ws = Ws::new();
    let dry = ws.run(&["tasks.md", "fix-shim", "--dry", "--json"]);
    assert_eq!(code(&dry), 0, "{}", stderr(&dry));
    let dry_json: serde_json::Value = serde_json::from_str(&stdout(&dry)).expect("dry json");
    let run = ws.run(&["tasks.md", "fix-shim", "--json"]);
    assert_eq!(code(&run), 0, "{}", stderr(&run));
    let run_json: serde_json::Value = serde_json::from_str(&stdout(&run)).expect("run json");
    assert_eq!(
        dry_json["caps"]["effective"], run_json["caps"]["effective"],
        "S14: dry caps must equal choke caps byte-for-byte"
    );
}
