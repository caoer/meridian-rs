//! U5 gates: the hermetic dispatch path end-to-end — starlark `run(ctx)` →
//! executor splice batch; guarantee class labeled; non-md effects surface
//! unexecuted; the pure `evaluate` seam applies nothing.

use std::collections::BTreeMap;

use effects::{Domain, EvalError, EvalLimits};
use model::MerkleRoot;
use run::caps::{Authority, CapSet};
use run::dispatch_starlark::{self, DispatchError, StarlarkDispatch};
use run::executor::{ExecError, ReceiptAddr};

/// Empty run-birth fields for these fixtures.
static TEST_EMPTY_FIELDS: BTreeMap<String, String> = BTreeMap::new();
use run::fence::GuaranteeClass;

const PAGE: &str = "\
---
status: todo
---

# Tasks
";

fn workspace() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("page.md"), PAGE).unwrap();
    let root = fs::WorkspaceRoot(tmp.path().to_owned());
    (tmp, root)
}

fn dispatch_of<'a>(
    source: &'a str,
    root_at_eval: &'a MerkleRoot,
    authority: &'a Authority,
) -> StarlarkDispatch<'a> {
    StarlarkDispatch {
        page: "page.md",
        task: "fix-x",
        task_rev: "b3:proc-star",
        source,
        args: vec!["done".to_owned()],
        env: BTreeMap::new(),
        invocation_id: "inv-1",
        now: Some("2026-07-22T02:00:00Z"),
        root_at_eval,
        authority,
        receipt: Some(ReceiptAddr {
            path: "receipts/2026-07-22.md".to_owned(),
            anchor: "r-000001".to_owned(),
        }),
        limits: EvalLimits::default(),
        actor: None,
        delta: None,
        fields: &TEST_EMPTY_FIELDS,
        birth_seq: None,
        ambient: None,
    }
}

#[test]
fn starlark_set_field_applies_via_the_splice_batch() {
    let (_tmp, root) = workspace();
    let now = fs::domain_snapshot(&root).unwrap().1;
    let caps = Authority::granted(CapSet::parse("md.edit").unwrap());
    let src = "def run(ctx):\n    set_field(field = \"status\", value = ctx.args[0])\n    notice(message = \"applied\")\n";

    let out = dispatch_starlark::dispatch(&root, &dispatch_of(src, &now, &caps)).unwrap();

    // The page changed through the ONE batch; the event is real.
    let text = std::fs::read_to_string(root.0.join("page.md")).unwrap();
    assert!(text.contains("status: done"), "{text}");
    let applied = out.applied.expect("md.* applied");
    assert_eq!(applied.applied, 1);
    assert!(applied.event.is_some());
    assert!(applied.receipt_line.is_some());

    // Guarantee class labeled hermetic; the notice surfaces unexecuted.
    assert_eq!(out.guarantee, GuaranteeClass::Hermetic);
    assert_eq!(out.effects.len(), 2, "full truth, never filtered");
    assert_eq!(out.unexecuted.len(), 1);
    assert_eq!(out.unexecuted[0].kind.domain(), Domain::Proto);
}

#[test]
fn no_md_effects_means_nothing_applied_nothing_written() {
    let (_tmp, root) = workspace();
    let now = fs::domain_snapshot(&root).unwrap().1;
    let caps = Authority::granted(CapSet::none());
    let src = "def run(ctx):\n    notice(message = \"advisory only\")\n";
    let out = dispatch_starlark::dispatch(&root, &dispatch_of(src, &now, &caps)).unwrap();
    assert!(out.applied.is_none());
    assert_eq!(out.unexecuted.len(), 1);
    assert_eq!(
        std::fs::read_to_string(root.0.join("page.md")).unwrap(),
        PAGE
    );
    assert!(!root.0.join("receipts").exists());
}

#[test]
fn wrong_plane_source_propagates_the_typed_eval_error() {
    let (_tmp, root) = workspace();
    let now = fs::domain_snapshot(&root).unwrap().1;
    let caps = Authority::granted(CapSet::none());
    let src = "def on_change(event):\n    pass\n";
    let err = dispatch_starlark::dispatch(&root, &dispatch_of(src, &now, &caps)).unwrap_err();
    assert!(matches!(
        err,
        DispatchError::Eval(EvalError::MissingEntry {
            expected: "run",
            wrong_plane: Some("on_change"),
            ..
        })
    ));
}

#[test]
fn cap_denied_at_the_choke_propagates_and_applies_nothing() {
    let (_tmp, root) = workspace();
    let now = fs::domain_snapshot(&root).unwrap().1;
    let caps = Authority::granted(CapSet::none());
    let src = "def run(ctx):\n    set_field(field = \"status\", value = \"x\")\n";
    let err = dispatch_starlark::dispatch(&root, &dispatch_of(src, &now, &caps)).unwrap_err();
    assert!(matches!(
        err,
        DispatchError::Exec(ExecError::CapDenied { .. })
    ));
    assert_eq!(
        std::fs::read_to_string(root.0.join("page.md")).unwrap(),
        PAGE
    );
}

#[test]
fn evaluate_alone_is_the_dry_seam_full_truth_nothing_applied() {
    let (_tmp, root) = workspace();
    let now = fs::domain_snapshot(&root).unwrap().1;
    let caps = Authority::granted(CapSet::parse("md.edit").unwrap());
    let src = "def run(ctx):\n    set_field(field = \"status\", value = \"x\")\n";
    let effects = dispatch_starlark::evaluate(&dispatch_of(src, &now, &caps)).unwrap();
    assert_eq!(effects.len(), 1, "all descriptors reported");
    assert_eq!(
        std::fs::read_to_string(root.0.join("page.md")).unwrap(),
        PAGE
    );
}

/// The machinery floor reaches the STARLARK lane end to end (card
/// create-door-machinery-containment). The grant here is the narrow
/// `md.create:tasks/*.md` that ADMITS the declared path — the capability
/// passes and the door still refuses, which is the whole point of judging the
/// landing on an axis the caps do not read.
#[test]
fn a_starlark_birth_into_a_machinery_dir_refuses() {
    for dir in [".git", ".meridian", "meridian", "receipts"] {
        let (_tmp, root) = workspace();
        let now = fs::domain_snapshot(&root).unwrap().1;
        let caps = Authority::granted(CapSet::parse("md.create:tasks/*.md").unwrap());
        let src = format!(
            "def run(ctx):\n    create(path = \"tasks/card.md\", base = \"{dir}\", body = \"# Escaped\\n\")\n"
        );
        let err = dispatch_starlark::dispatch(&root, &dispatch_of(&src, &now, &caps))
            .expect_err("a machinery landing refuses at the door");
        let DispatchError::Exec(ExecError::BirthRefused { detail, .. }) = &err else {
            panic!("expected BirthRefused for base `{dir}`, got {err:?}");
        };
        assert!(
            detail.contains("bad_path"),
            "the starlark lane hits the machinery floor for `{dir}`: {detail}"
        );
        assert!(
            detail.contains(dir),
            "the refusal names the offending segment `{dir}`: {detail}"
        );
        assert!(
            !root.0.join(dir).join("tasks/card.md").exists(),
            "nothing may be born under `{dir}`"
        );
    }
}
