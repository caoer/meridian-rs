//! E2e gates for the lazy root-at-eval fold (`docs/run-plane.md` § The run
//! plane), driving the REAL binary over its process boundary.
//!
//! `MRD_TIMING` is the instrument (`docs/status.md` § The timing mode): the
//! `snapshot*` phases are emitted inside `fs::domain_snapshot_with_leaves`, so
//! their ABSENCE from the log is the proof that the corpus was never walked —
//! a wall-clock number would only be evidence about this machine on this day.
//!
//! What the in-process gate (`run/tests/lazy_snapshot.rs`) cannot show is that
//! the whole binary honours it: the CLI door, the runner and the dispatcher
//! each used to fold, and only a process can prove none of them does.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

/// The fixture: one effect-free task, one that emits a `notice` (no md.\*
/// batch, no receipt — the token's only consumer is the reported provenance),
/// and one that edits (the batch's `observed_root`, attested as `root_pin`).
const PAGE: &str = "\
---
task.quiet: \"[[#^quiet-1]]\"
task.noticer: \"[[#^notice-1]]\"
task.editor: \"[[#^edit-1]]\"
task.editor.caps: md.edit
---

# Tasks

```starlark
def run(ctx):
    pass
```
^quiet-1

```starlark
def run(ctx):
    notice(message = \"advisory only\")
```
^notice-1

```starlark
def run(ctx):
    set_field(field = \"status\", value = \"done\")
```
^edit-1
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

    /// `mrd run <args>` with the workspace pinned via the tier-1 override;
    /// `MRD_TIMING` set when asked for, removed otherwise.
    fn run(&self, timing: bool, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_mrd"));
        command
            .arg("run")
            .args(args)
            .env("MERIDIAN_WORKSPACE", self.path())
            .current_dir(self.path());
        if timing {
            command.env("MRD_TIMING", "1");
        } else {
            command.env_remove("MRD_TIMING");
        }
        command.output().expect("spawn mrd")
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// The `phase=` names on a stream, in completion order.
fn phases(raw: &str) -> Vec<String> {
    raw.lines()
        .filter(|line| line.starts_with("mrd-timing "))
        .filter_map(|line| {
            line.split(' ')
                .find_map(|f| f.strip_prefix("phase="))
                .map(ToOwned::to_owned)
        })
        .collect()
}

/// Every `root_at_eval` in a `--dry --json` report, in emission order.
fn observed(out: &Output) -> Vec<String> {
    let v: Value = serde_json::from_str(&stdout(out)).expect("json");
    v["effects"]
        .as_array()
        .expect("effects")
        .iter()
        .map(|e| {
            e["root_at_eval"]
                .as_str()
                .unwrap_or_else(|| panic!("run-plane effect carries root_at_eval: {e}"))
                .to_owned()
        })
        .collect()
}

/// **No effect ⇒ no fold**, dry and live, through the real binary. The `eval`
/// phase is asserted present in the same breath: a run that emitted no phases
/// at all would pass a bare "no snapshot line" check for the wrong reason.
#[test]
fn an_effect_free_task_never_walks_the_corpus() {
    let ws = Ws::new();
    assert_no_fold(&ws, &["tasks.md", "quiet", "--dry", "--json"]);
    assert_no_fold(&ws, &["tasks.md", "quiet", "--json"]);
}

/// **The live gate is md-only.** A live `notice` has no reader for the token —
/// the live report is `kind` + `domain` (`run::report::EffectLine`), there is
/// no batch, no `observed_root` and no receipt — so the corpus is not walked.
/// This is the hook shape (notice / remind / send, live, once per event): the
/// case the whole gate exists for, through the real binary.
///
/// The SAME page under `--dry` folds, and the test beside this one asserts it:
/// the dry report serializes provenance, so there the token has a reader.
#[test]
fn a_live_notice_only_task_never_walks_the_corpus() {
    let ws = Ws::new();
    assert_no_fold(&ws, &["tasks.md", "noticer", "--json"]);
}

/// One case of "this run's gate did not fire": the run succeeds, the block
/// demonstrably ran (`eval` present — a run that emitted no phase at all would
/// pass a bare "no snapshot line" check for the wrong reason), and no
/// `snapshot*` phase exists. Shared by the effect-free cases and by the live
/// `notice`-only one, so the message names the gate rather than the fixture.
fn assert_no_fold(ws: &Ws, args: &[&str]) {
    let out = ws.run(true, args);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let names = phases(&stderr(&out));
    assert!(
        names.iter().any(|p| p == "eval"),
        "the block did not run: {names:?} ({args:?})"
    );
    assert!(
        !names.iter().any(|p| p.starts_with("snapshot")),
        "the corpus was folded for a run whose gate should not have fired: {names:?} ({args:?})"
    );
}

/// **In the REHEARSAL tense, any effect ⇒ the fold happens**, and the emitted
/// effect carries a real token. `--dry` serializes whole effects, provenance
/// included, so even a `notice` — no batch, no receipt — puts the token in
/// front of a reader and is owed it.
#[test]
fn an_emitting_task_folds_and_stamps_the_effect_under_dry() {
    let ws = Ws::new();
    let out = ws.run(true, &["tasks.md", "noticer", "--dry", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));

    let names = phases(&stderr(&out));
    assert!(
        names.iter().any(|p| p == "snapshot"),
        "an emitted effect must be observed: {names:?}"
    );
    // The fold FOLLOWS the eval that decided it was needed (run-plane.md
    // § Timing phases, the `dispatch` row). Lines print in completion order.
    let eval = names.iter().position(|p| p == "eval").expect("eval phase");
    let snapshot = names
        .iter()
        .position(|p| p == "snapshot")
        .expect("snapshot phase");
    assert!(eval < snapshot, "the fold ran before the eval: {names:?}");

    let tokens = observed(&out);
    assert_eq!(tokens.len(), 1);
    assert!(
        !tokens[0].is_empty(),
        "the unobserved placeholder reached the report"
    );
}

/// The stamp is the LIVE fold, not a constant: it is stable across two runs of
/// an unchanged tree and moves when a domain member appears.
#[test]
fn the_stamp_is_the_live_root() {
    let ws = Ws::new();
    let args = ["tasks.md", "noticer", "--dry", "--json"];

    let first = observed(&ws.run(false, &args));
    let again = observed(&ws.run(false, &args));
    assert_eq!(first, again, "--dry changed the corpus");

    std::fs::write(ws.file("second.md"), "# Second\n").expect("member");
    let after = observed(&ws.run(false, &args));
    assert_ne!(first, after, "a new domain member must move the token");
}

/// Dry and live agree: the token `--dry` reports on an unchanged tree is the
/// one the live run attests in the receipt as `root_pin`. This is the identity
/// the eager fold used to give for free — it is the reason the change is
/// law-neutral, so it is gated rather than argued.
#[test]
fn the_dry_token_is_the_root_pin_the_live_run_attests() {
    let ws = Ws::new();

    let dry = observed(&ws.run(false, &["tasks.md", "editor", "--dry", "--json"]));
    assert_eq!(dry.len(), 1);

    let live = ws.run(false, &["tasks.md", "editor", "--json"]);
    assert_eq!(live.status.code(), Some(0), "{}", stderr(&live));

    // Matched as the FIELD: the row is compact JSON, and a bare
    // `contains(token)` would also pass on a token that landed elsewhere.
    let receipt = std::fs::read_to_string(ws.file("receipts/run.md")).expect("receipt written");
    assert!(
        receipt.contains(&format!("\"root_pin\":\"{}\"", dry[0])),
        "the receipt attests a root the dry run never showed\ndry: {}\n{receipt}",
        dry[0]
    );
    assert!(
        std::fs::read_to_string(ws.file("tasks.md"))
            .expect("page")
            .contains("status: done"),
        "the live arm applied nothing"
    );
}
