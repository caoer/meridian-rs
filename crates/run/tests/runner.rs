//! U7 gates — runner composition + cascade loop control: the pre-eval chain
//! composes into dispatch, `&[Rule]` EMPTY is the vacuous gen-0 stop,
//! injected synthetic rules exercise the depth-cap + md.* suppression
//! through the REAL executor path, and the #23-SHARPENED label gate refuses
//! a bash block before anything commits.

use std::collections::BTreeMap;
use std::time::Duration;

use effects::{EvalLimits, Rule};
use run::dispatch_bash::Phase2;
use run::executor::ReceiptAddr;
use run::fence::GuaranteeClass;
use run::runner::{self, RunSpec, RunnerError, TaskOutcome};

/// Empty run-birth fields for these fixtures.
static TEST_EMPTY_FIELDS: BTreeMap<String, String> = BTreeMap::new();

/// A one-task starlark page: `fix-x` sets `status` from its arg and emits a
/// notice (the non-md surface).
const STARLARK_PAGE: &str = "\
---
status: todo
task.fix-x: \"[[#^fix-1]]\"
task.fix-x.caps: md.edit
task.fix-x.args: value
---

# Tasks

```starlark
def run(ctx):
    set_field(field = \"status\", value = ctx.args[0])
    notice(message = \"ran\")
```
^fix-1
";

/// A one-task bash page: the block streams a line of stdout (bash has no
/// effect channel).
const BASH_PAGE: &str = "\
---
status: todo
task.fix-sh: \"[[#^sh-1]]\"
task.fix-sh.caps: md.edit
---

# Tasks

```bash
echo running
```
^sh-1
";

fn workspace(page: &str) -> (tempfile::TempDir, fs::WorkspaceRoot) {
    // Under target/, not $TMPDIR — the bash e2e here failed
    // `Detection(Guard(Io(NotADirectory)))` on mac at CLEAN MAIN for the
    // /var→/private/var reason documented in crates/run/tests/birth_cap.rs.
    let tmp = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    std::fs::write(tmp.path().join("page.md"), page).unwrap();
    let root = fs::WorkspaceRoot(tmp.path().to_owned());
    (tmp, root)
}

fn spec_for(scratch: &tempfile::TempDir, args: Vec<String>) -> RunSpec<'_> {
    RunSpec {
        page: "page.md",
        task: None,
        args,
        env: BTreeMap::new(),
        invocation_id: "inv-1",
        now: Some("2026-07-22T03:00:00Z"),
        receipt: Some(ReceiptAddr {
            path: "receipts/2026-07-22.md".to_owned(),
            anchor: "r-000001".to_owned(),
        }),
        pre_receipt: None,
        scratch: scratch.path(),
        timeout: Duration::from_secs(30),
        // These fixtures exercise the runner's dispatch, not the convention
        // plane: no declaring root means no ceiling, so a task's explicit caps
        // reach the executor exactly as written.
        declaring_root: None,
        limits: EvalLimits::default(),
        actor: None,
        step_cwd: None,
        delta: None,
        observations: run::dispatch_bash::ObservationSource::Drawer,
        fields: &TEST_EMPTY_FIELDS,
        birth_seq: None,
        ambient: None,
    }
}

#[test]
fn starlark_composes_end_to_end_and_labels_hermetic() {
    let (_tmp, root) = workspace(STARLARK_PAGE);
    let scratch = tempfile::tempdir().unwrap();
    let mut live: Vec<u8> = Vec::new();

    let report = runner::run(
        &root,
        &spec_for(&scratch, vec!["done".to_owned()]),
        &[],
        &mut live,
    )
    .unwrap();

    let text = std::fs::read_to_string(root.0.join("page.md")).unwrap();
    assert!(text.contains("status: done"), "{text}");
    assert_eq!(report.task, "fix-x");
    assert!(!report.task_rev.is_empty());
    assert_eq!(report.guarantee, GuaranteeClass::Hermetic);
    let TaskOutcome::Starlark(out) = &report.outcome else {
        panic!("starlark outcome expected");
    };
    assert!(out.applied.is_some());
    assert_eq!(out.unexecuted.len(), 1, "the notice surfaces unexecuted");
}

#[test]
fn empty_rules_is_the_vacuous_gen_zero_stop() {
    // Decision #5: gen 0 applied and synthesized a REAL event, but with no
    // rules there is nothing to evaluate — zero generations, no cap.
    let (_tmp, root) = workspace(STARLARK_PAGE);
    let scratch = tempfile::tempdir().unwrap();
    let mut live: Vec<u8> = Vec::new();

    let report = runner::run(
        &root,
        &spec_for(&scratch, vec!["done".to_owned()]),
        &[],
        &mut live,
    )
    .unwrap();

    let TaskOutcome::Starlark(out) = &report.outcome else {
        panic!("starlark outcome expected");
    };
    let applied = out.applied.as_ref().unwrap();
    assert!(applied.event.is_some(), "gen 0 synthesized a real event");
    assert!(report.cascade.is_empty(), "vacuous stop");
    assert!(!report.cap_reached);
}

#[test]
fn a_synthetic_rule_cascades_one_generation_through_the_real_executor() {
    let (_tmp, root) = workspace(STARLARK_PAGE);
    let scratch = tempfile::tempdir().unwrap();
    let mut live: Vec<u8> = Vec::new();

    // Fires only on the gen-0 event (status changed); its own write changes
    // `cascaded`, so gen 2 evaluates to nothing and the loop goes quiet.
    let rule = Rule::new(
        "cascade-once",
        "def on_change(event):\n    if \"status\" in event.fields_changed:\n        set_field(field = \"cascaded\", value = \"yes\")\n",
    );

    let report = runner::run(
        &root,
        &spec_for(&scratch, vec!["done".to_owned()]),
        &[rule],
        &mut live,
    )
    .unwrap();

    let text = std::fs::read_to_string(root.0.join("page.md")).unwrap();
    assert!(text.contains("status: done"), "{text}");
    assert!(text.contains("cascaded: yes"), "{text}");
    assert_eq!(report.cascade.len(), 1);
    let generation = &report.cascade[0];
    assert_eq!(generation.depth, 1);
    assert_eq!(generation.effects.len(), 1);
    let applied = generation.applied.as_ref().expect("cascade md applied");
    assert_eq!(applied.applied, 1);
    assert!(!report.cap_reached);
}

#[test]
fn the_depth_cap_suppresses_md_and_names_the_stop() {
    let (_tmp, root) = workspace(STARLARK_PAGE);
    let scratch = tempfile::tempdir().unwrap();
    let mut live: Vec<u8> = Vec::new();

    // Would cascade forever (each write changes `last` to the fresh
    // fingerprint); the kernel suppresses md.* at the cap and the terminal
    // notice survives as the unexecuted remainder.
    let rule = Rule::new(
        "cascade-forever",
        "def on_change(event):\n    set_field(field = \"last\", value = event.fingerprint_after)\n    notice(message = \"gen\")\n",
    );
    let mut spec = spec_for(&scratch, vec!["done".to_owned()]);
    spec.limits = EvalLimits {
        max_depth: 2,
        ..EvalLimits::default()
    };

    let report = runner::run(&root, &spec, &[rule], &mut live).unwrap();

    assert!(report.cap_reached, "the cap must be a named fact");
    // Gen 1 (depth 1 < 2): md applied through the REAL executor. Gen 2
    // (depth 2 >= 2): md suppressed, only the notice survives.
    assert_eq!(report.cascade.len(), 2);
    let gen1 = &report.cascade[0];
    assert_eq!(gen1.depth, 1);
    assert!(gen1.applied.is_some());
    let text = std::fs::read_to_string(root.0.join("page.md")).unwrap();
    assert!(text.contains("last:"), "{text}");
    let gen2 = &report.cascade[1];
    assert_eq!(gen2.depth, 2);
    assert!(gen2.applied.is_none(), "md suppressed at the cap");
    assert_eq!(gen2.effects.len(), 1, "the terminal notice survives");
    assert_eq!(gen2.unexecuted.len(), 1);
}

/// Under `docs/laws.md` § Amendment a bash task has no guarantee to derive —
/// the class is `Unsandboxed` (a structural fact; `--json` carries it, the
/// human face never prints the word — ZT ruling, 2026-08-15), and the bracket
/// verdict is asserted as what it is: an observation about the window, still
/// rendered, never a class.
#[test]
fn a_bash_block_runs_end_to_end_and_labels_unsandboxed() {
    let (_tmp, root) = workspace(BASH_PAGE);
    let scratch = tempfile::tempdir().unwrap();
    let mut live: Vec<u8> = Vec::new();

    let report = runner::run(&root, &spec_for(&scratch, vec![]), &[], &mut live).unwrap();

    assert_eq!(report.guarantee, GuaranteeClass::Unsandboxed);
    // The law is scoped, not a deletion: the bash path names no capability.
    assert_eq!(report.authority, run::caps::Authority::Unsandboxed);
    let TaskOutcome::Bash(out) = &report.outcome else {
        panic!("bash outcome expected");
    };
    assert!(out.status.success());
    assert!(
        out.detection.is_clean(),
        "the bracket still renders its verdict: {:?}",
        out.detection
    );
    let Phase2::Applied { applied } = &out.phase2 else {
        panic!("expected Applied, got {:?}", out.phase2);
    };
    assert_eq!(
        applied.applied, 0,
        "bash has no effect channel — the completion commit is an empty batch"
    );
    assert_eq!(
        std::fs::read_to_string(root.0.join("page.md")).unwrap(),
        BASH_PAGE,
        "the page stays byte-identical"
    );
    assert_eq!(live, b"running\n", "stdout streamed live");
    assert!(report.cascade.is_empty());
}

#[test]
fn a_contract_violation_refuses_pre_eval() {
    let (_tmp, root) = workspace(STARLARK_PAGE);
    let scratch = tempfile::tempdir().unwrap();
    let mut live: Vec<u8> = Vec::new();

    // The task declares one arg; zero supplied.
    let err = runner::run(&root, &spec_for(&scratch, vec![]), &[], &mut live).unwrap_err();
    assert!(matches!(err, RunnerError::Violation(_)));
    assert_eq!(
        std::fs::read_to_string(root.0.join("page.md")).unwrap(),
        STARLARK_PAGE
    );
}

#[test]
fn a_missing_task_propagates_the_typed_address_error() {
    let (_tmp, root) = workspace(STARLARK_PAGE);
    let scratch = tempfile::tempdir().unwrap();
    let mut live: Vec<u8> = Vec::new();

    let mut spec = spec_for(&scratch, vec!["done".to_owned()]);
    spec.task = Some("nope");
    let err = runner::run(&root, &spec, &[], &mut live).unwrap_err();
    assert!(matches!(err, RunnerError::Address(_)));
}

#[test]
fn the_caps_resolution_reaches_the_report() {
    let (_tmp, root) = workspace(STARLARK_PAGE);
    let scratch = tempfile::tempdir().unwrap();
    let mut live: Vec<u8> = Vec::new();

    let report = runner::run(
        &root,
        &spec_for(&scratch, vec!["done".to_owned()]),
        &[],
        &mut live,
    )
    .unwrap();

    let caps = report
        .authority
        .capabilities()
        .expect("starlark resolves a real capability grant");
    assert!(caps.effective.admits("md.edit", Some("page.md")));
    assert!(caps.narrowed.is_empty());
}
