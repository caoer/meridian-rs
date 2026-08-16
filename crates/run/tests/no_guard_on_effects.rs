//! Card `effects-lane` — the no-guard-on-effects ruling
//! (`decisions/2026-08-15-no-guard-on-effects.md`; plan §4.10; run-plane.md
//! § the no-guard amendment): `run` holds NO world pin, and no refusal on
//! this door is a premise refusal.
//!
//! The F3 scenario fixture (dogfood r9 prober § F3): one run, a foreign
//! fleet write advancing the corpus mid-flight. Under the retired law the
//! plane's self-manufactured pin refused `root_mismatch` and the caller's
//! unrelated work died. Under the ruled law a foreign advance re-derives and
//! proceeds; a vanished unrelated record drops from view and never fails
//! another target. The receipt still attests the root the effects were
//! produced against — observation honesty, never a compared premise.

use std::collections::BTreeMap;

use effects::{ArgValue, Effect, EffectKind, Provenance};
use run::caps::Authority;
use run::executor::{self, ApplyRequest, ReceiptAddr};

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

fn set_status_done() -> Effect {
    Effect {
        kind: EffectKind::SetField,
        rule_id: "t".to_owned(),
        seq: 0,
        depth: 0,
        provenance: Provenance::Run {
            invocation_id: "inv-1".to_owned(),
            root_at_eval: "b3:x".to_owned(),
        },
        args: [
            ("field".to_owned(), ArgValue::Str("status".to_owned())),
            ("value".to_owned(), ArgValue::Str("done".to_owned())),
        ]
        .into_iter()
        .collect::<BTreeMap<_, _>>(),
    }
}

fn request<'a>(observed: &'a model::MerkleRoot, effects: &'a [Effect]) -> ApplyRequest<'a> {
    ApplyRequest {
        page: "page.md",
        task: "fix-x",
        task_rev: "b3:proc-test",
        invocation_id: "inv-1",
        now: Some("2026-08-15T01:00:00Z"),
        effects,
        authority: &Authority::Unsandboxed,
        observed_root: observed,
        receipt: Some(ReceiptAddr {
            path: "receipts/run.md".to_owned(),
            anchor: "r-000001".to_owned(),
        }),
        exec: None,
        actor: None,
        depth: 0,
        delta: None,
    }
}

/// F3, the foreign-advance half: the run's eval-time observation is stale —
/// an unrelated fleet write advanced the corpus after eval — and the apply
/// PROCEEDS. Under the retired self-pin this exact shape refused
/// `root_mismatch` and killed work the foreign write never touched.
#[test]
fn a_foreign_advance_after_eval_proceeds_and_commits() {
    let (tmp, root) = workspace();
    // The observation the effects were produced against, taken at eval time.
    let at_eval = fs::domain_snapshot(&root).unwrap().1;

    // The foreign advance: an unrelated record lands after eval.
    std::fs::write(tmp.path().join("unrelated.md"), "# Foreign\n").unwrap();
    let moved = fs::domain_snapshot(&root).unwrap().1;
    assert_ne!(at_eval, moved, "the corpus really advanced");

    let effects = [set_status_done()];
    let applied = executor::apply(&root, &request(&at_eval, &effects))
        .expect("a foreign advance re-derives and proceeds — never a premise refusal");
    assert_eq!(applied.applied, 1);

    let page = std::fs::read_to_string(tmp.path().join("page.md")).unwrap();
    assert!(page.contains("status: done"), "the edit landed:\n{page}");

    // Observation honesty: the receipt attests the root the effects were
    // produced against — the stale-at-commit eval root, as observed.
    let line = applied.receipt_line.expect("a receipt rode the commit");
    assert!(
        line.contains(&format!("\"root_pin\":\"{}\"", at_eval.0)),
        "the receipt attests the eval-time observation: {line}"
    );
}

/// F3, the vanished-record half: an unrelated record disappears after eval
/// and the apply still proceeds — a vanished unrelated record drops from
/// view and never fails another target.
#[test]
fn a_vanished_unrelated_record_never_fails_another_target() {
    let (tmp, root) = workspace();
    std::fs::write(tmp.path().join("doomed.md"), "# Doomed\n").unwrap();
    let at_eval = fs::domain_snapshot(&root).unwrap().1;

    std::fs::remove_file(tmp.path().join("doomed.md")).unwrap();

    let effects = [set_status_done()];
    let applied = executor::apply(&root, &request(&at_eval, &effects))
        .expect("a vanished unrelated record fails no other target");
    assert_eq!(applied.applied, 1);
    let page = std::fs::read_to_string(tmp.path().join("page.md")).unwrap();
    assert!(page.contains("status: done"), "the edit landed:\n{page}");
}

/// The retired per-target pin-and-verify (foreign-edit law, decision #26)
/// gates nothing: a target edited outside the run plane since its last
/// governed write is overwritten like any other edit — no takeover flag
/// exists, no receipt scan runs.
#[test]
fn a_foreign_edit_since_the_last_governed_write_applies_without_ceremony() {
    let (tmp, root) = workspace();
    let r0 = fs::domain_snapshot(&root).unwrap().1;

    // First governed write lands a receipt whose after-rev anchors `status`.
    let effects = [set_status_done()];
    executor::apply(&root, &request(&r0, &effects)).expect("first governed write");

    // A foreign hand edits the governed target outside the run plane.
    let page = std::fs::read_to_string(tmp.path().join("page.md")).unwrap();
    std::fs::write(
        tmp.path().join("page.md"),
        page.replace("status: done", "status: foreign"),
    )
    .unwrap();

    // The second governed write proceeds — the former `foreign_edit`
    // refusal is retired with the ruling.
    let r1 = fs::domain_snapshot(&root).unwrap().1;
    let again = [set_status_done()];
    let mut req = request(&r1, &again);
    req.receipt = Some(ReceiptAddr {
        path: "receipts/run.md".to_owned(),
        anchor: "r-000002".to_owned(),
    });
    executor::apply(&root, &req).expect("no per-target pin-and-verify remains on this door");
    let page = std::fs::read_to_string(tmp.path().join("page.md")).unwrap();
    assert!(page.contains("status: done"), "the edit landed:\n{page}");
}
