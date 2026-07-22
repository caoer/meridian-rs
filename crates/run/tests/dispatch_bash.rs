//! U6a gates — the two-phase bash dispatch: pre-exec receipt + locked-window
//! `root_after_phase1` (#19 addendum), the S2/S6/#21 failure matrix, the
//! executor choke point, and the U8 stdout record riding the exec.

use std::collections::BTreeMap;
use std::time::Duration;

use rules::Provenance;
use run::caps::CapSet;
use run::dispatch_bash::{self, BashDispatch, BashError, Phase2};
use run::exec::ExecStatus;
use run::executor::{ExecError, ReceiptAddr, WorkspaceLock};
use run::shim::ShimError;

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

/// The documented emitter: one `md.set_field status=done` record + trailer.
const EMIT_SET_FIELD: &str = r#"
p='{"op":"md.set_field","field":"status","value":"done"}'
printf '%s:%s\n' "${#p}" "$p" >&"$MD_EFFECT_FD"
printf 'end:1\n' >&3
"#;

fn dispatch_of<'a>(
    source: &'a str,
    scratch: &'a tempfile::TempDir,
    caps: &'a CapSet,
) -> BashDispatch<'a> {
    BashDispatch {
        page: "page.md",
        task: "fix-x",
        task_rev: "b3:proc-bash",
        source,
        args: vec![],
        env: BTreeMap::new(),
        invocation_id: "inv-1",
        now: Some("2026-07-22T02:00:00Z"),
        caps,
        pre_receipt: Some(ReceiptAddr {
            path: "receipts/2026-07-22.md".to_owned(),
            anchor: "p-000001".to_owned(),
        }),
        receipt: Some(ReceiptAddr {
            path: "receipts/2026-07-22.md".to_owned(),
            anchor: "r-000001".to_owned(),
        }),
        takeover: false,
        scratch: scratch.path(),
        timeout: Duration::from_secs(30),
    }
}

#[test]
fn a_clean_run_applies_the_shim_batch_two_phase() {
    let (_tmp, root) = workspace();
    let scratch = tempfile::tempdir().unwrap();
    let caps = CapSet::parse("md.set_field").unwrap();
    let root_before = fs::domain_snapshot(&root).unwrap().1;
    let mut live: Vec<u8> = Vec::new();

    let src = format!("echo running{EMIT_SET_FIELD}");
    let out = dispatch_bash::run(&root, &dispatch_of(&src, &scratch, &caps), &mut live).unwrap();

    // The page changed through the ONE phase-2 batch.
    let text = std::fs::read_to_string(root.0.join("page.md")).unwrap();
    assert!(text.contains("status: done"), "{text}");
    assert!(out.status.success());
    let Phase2::Applied { effects, applied } = &out.phase2 else {
        panic!("expected Applied, got {:?}", out.phase2);
    };
    assert_eq!(effects.len(), 1);
    let applied = applied.as_ref().expect("md.* applied");
    assert_eq!(applied.applied, 1);
    assert!(applied.event.is_some());

    // #19: the effects were pinned to the COMPUTED root_after_phase1 — the
    // phase-1 receipt commit changed the corpus, so the pin is NOT the
    // pre-run root.
    let Provenance::Run { root_at_eval, .. } = &effects[0].provenance else {
        panic!("run-plane provenance expected");
    };
    assert_ne!(
        *root_at_eval, root_before.0,
        "the pin must be the post-phase-1 root, never a stale pre-run root"
    );

    // Both receipt lines landed: the pre-exec anchor (S2) and the completion.
    assert!(out.pre_receipt_line.is_some());
    let receipts = std::fs::read_to_string(root.0.join("receipts/2026-07-22.md")).unwrap();
    assert!(receipts.contains("^p-000001"), "{receipts}");
    assert!(receipts.contains("^r-000001"), "{receipts}");
    assert!(
        receipts.contains(root_at_eval.as_str()),
        "the completion receipt records the phase-2 pin"
    );

    // The U8 record ran: live tee + sealed out-of-tree log (ruling 7).
    assert_eq!(live, b"running\n");
    let record = out.stdout.expect("stdout record sealed");
    assert_eq!(record.bytes, 8);
    let log = std::fs::read(root.0.join(".meridian/runs/inv-1.log")).unwrap();
    assert_eq!(log, b"running\n");
}

#[test]
fn a_nonzero_exit_refuses_phase2_and_phase1_stands() {
    let (_tmp, root) = workspace();
    let scratch = tempfile::tempdir().unwrap();
    let caps = CapSet::parse("md.set_field").unwrap();
    let mut live: Vec<u8> = Vec::new();

    // A VALID stream, then a failing exit — S2: exec-fail refuses phase 2.
    let src = format!("{EMIT_SET_FIELD}\nexit 7");
    let out = dispatch_bash::run(&root, &dispatch_of(&src, &scratch, &caps), &mut live).unwrap();

    assert_eq!(out.status, ExecStatus::Exited { code: 7 });
    assert!(matches!(out.phase2, Phase2::RefusedExecFailed));
    // Nothing applied; the pre-exec receipt stands committed (orphan-lint
    // anchor) and no completion line joined it.
    assert_eq!(
        std::fs::read_to_string(root.0.join("page.md")).unwrap(),
        PAGE
    );
    let receipts = std::fs::read_to_string(root.0.join("receipts/2026-07-22.md")).unwrap();
    assert!(receipts.contains("^p-000001"));
    assert!(!receipts.contains("^r-000001"));
}

#[test]
fn a_truncated_stream_fails_the_whole_batch_closed() {
    let (_tmp, root) = workspace();
    let scratch = tempfile::tempdir().unwrap();
    let caps = CapSet::parse("md.set_field").unwrap();
    let mut live: Vec<u8> = Vec::new();

    // A record with NO trailer and a clean exit — S6: fails closed.
    let src = r#"
p='{"op":"md.set_field","field":"status","value":"done"}'
printf '%s:%s\n' "${#p}" "$p" >&3
"#;
    let out = dispatch_bash::run(&root, &dispatch_of(src, &scratch, &caps), &mut live).unwrap();

    assert!(out.status.success());
    assert!(matches!(
        out.phase2,
        Phase2::RefusedShim(ShimError::MissingTrailer)
    ));
    assert_eq!(
        std::fs::read_to_string(root.0.join("page.md")).unwrap(),
        PAGE
    );
}

#[test]
fn a_timeout_is_distinct_and_refuses_phase2() {
    let (_tmp, root) = workspace();
    let scratch = tempfile::tempdir().unwrap();
    let caps = CapSet::parse("md.set_field").unwrap();
    let mut live: Vec<u8> = Vec::new();

    let mut d = dispatch_of("sleep 30", &scratch, &caps);
    d.timeout = Duration::from_millis(300);
    let out = dispatch_bash::run(&root, &d, &mut live).unwrap();

    assert!(matches!(out.status, ExecStatus::TimedOut { .. }));
    assert!(matches!(out.phase2, Phase2::RefusedTimeout));
    assert_eq!(
        std::fs::read_to_string(root.0.join("page.md")).unwrap(),
        PAGE
    );
    // The record still sealed — the log is truth even for a killed step.
    assert!(out.stdout.is_ok());
}

#[test]
fn zero_descriptors_on_a_clean_exit_is_not_a_fault() {
    let (_tmp, root) = workspace();
    let scratch = tempfile::tempdir().unwrap();
    let caps = CapSet::none();
    let mut live: Vec<u8> = Vec::new();

    let out =
        dispatch_bash::run(&root, &dispatch_of("echo ok", &scratch, &caps), &mut live).unwrap();

    assert!(out.status.success());
    assert!(matches!(
        out.phase2,
        Phase2::Applied { ref effects, applied: None } if effects.is_empty()
    ));
    assert_eq!(
        std::fs::read_to_string(root.0.join("page.md")).unwrap(),
        PAGE
    );
}

#[test]
fn the_choke_point_refuses_an_uncapped_descriptor() {
    let (_tmp, root) = workspace();
    let scratch = tempfile::tempdir().unwrap();
    let caps = CapSet::none(); // deny-by-default
    let mut live: Vec<u8> = Vec::new();

    let out = dispatch_bash::run(
        &root,
        &dispatch_of(EMIT_SET_FIELD, &scratch, &caps),
        &mut live,
    )
    .unwrap();

    assert!(matches!(
        out.phase2,
        Phase2::RefusedExec {
            error: ExecError::CapDenied { .. },
            ..
        }
    ));
    assert_eq!(
        std::fs::read_to_string(root.0.join("page.md")).unwrap(),
        PAGE
    );
}

#[test]
fn a_held_workspace_lock_is_a_fast_typed_refusal() {
    let (_tmp, root) = workspace();
    let scratch = tempfile::tempdir().unwrap();
    let caps = CapSet::none();
    let mut live: Vec<u8> = Vec::new();

    let _held = WorkspaceLock::acquire(&root.0).unwrap();
    let err =
        dispatch_bash::run(&root, &dispatch_of("echo hi", &scratch, &caps), &mut live).unwrap_err();
    assert!(matches!(err, BashError::Phase1(ExecError::WorkspaceBusy)));
}

#[test]
fn an_unsafe_invocation_id_refuses_before_anything_commits() {
    let (_tmp, root) = workspace();
    let scratch = tempfile::tempdir().unwrap();
    let caps = CapSet::none();
    let mut live: Vec<u8> = Vec::new();

    let mut d = dispatch_of("echo hi", &scratch, &caps);
    d.invocation_id = "../evil";
    let err = dispatch_bash::run(&root, &d, &mut live).unwrap_err();
    assert!(matches!(err, BashError::Record(_)));
    assert!(!root.0.join("receipts").exists(), "nothing committed");
}

#[test]
fn without_a_pre_receipt_the_run_still_pins_a_locked_window_root() {
    let (_tmp, root) = workspace();
    let scratch = tempfile::tempdir().unwrap();
    let caps = CapSet::parse("md.set_field").unwrap();
    let root_before = fs::domain_snapshot(&root).unwrap().1;
    let mut live: Vec<u8> = Vec::new();

    let mut d = dispatch_of(EMIT_SET_FIELD, &scratch, &caps);
    d.pre_receipt = None;
    let out = dispatch_bash::run(&root, &d, &mut live).unwrap();

    assert!(out.pre_receipt_line.is_none());
    let Phase2::Applied { effects, applied } = &out.phase2 else {
        panic!("expected Applied, got {:?}", out.phase2);
    };
    assert!(applied.is_some());
    // No phase-1 commit → root_after_phase1 IS the locked-window root0.
    let Provenance::Run { root_at_eval, .. } = &effects[0].provenance else {
        panic!("run-plane provenance expected");
    };
    assert_eq!(*root_at_eval, root_before.0);
}
