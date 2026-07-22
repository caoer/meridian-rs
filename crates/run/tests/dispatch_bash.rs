//! U6a gates — the two-phase bash dispatch: pre-exec receipt + locked-window
//! `root_after_phase1` (#19 addendum), the S2/S6/#21 failure matrix, the
//! executor choke point, and the U8 stdout record riding the exec — plus the
//! U6b detection gates (the wired `ExecBracket`: #14 cheat detection, the
//! phase-2 gate, ruling-2 never-roll-back).

use std::collections::BTreeMap;
use std::time::Duration;

use rules::Provenance;
use run::caps::CapSet;
use run::dispatch_bash::{self, BashDispatch, BashError, Phase2};
use run::exec::ExecStatus;
use run::executor::{ExecError, ReceiptAddr, WorkspaceLock};
use run::shim::ShimError;
use run::snapshot::Detection;

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

/// U6b #14, the zero-descriptor cheat end-to-end: bash writes an md file
/// into the TREE, emits nothing on the shim fd, exits 0. Without the bracket
/// this was `Applied { applied: None }`; now phase 2 refuses, the delta
/// names the file with the S4 wording, and the write is NEVER rolled back
/// (ruling 2).
#[test]
fn an_ungoverned_tree_write_refuses_phase2_with_the_delta_named() {
    let (_tmp, root) = workspace();
    let scratch = tempfile::tempdir().unwrap();
    let caps = CapSet::none();
    let mut live: Vec<u8> = Vec::new();

    let src = format!("echo sneaky > '{}/rogue.md'", root.0.display());
    let out = dispatch_bash::run(&root, &dispatch_of(&src, &scratch, &caps), &mut live).unwrap();

    assert!(out.status.success());
    assert!(matches!(out.phase2, Phase2::RefusedDetection));
    let Detection::OutOfBand(delta) = &out.detection else {
        panic!("expected OutOfBand, got {:?}", out.detection);
    };
    assert_eq!(delta.unexpected, vec!["rogue.md".to_string()]);
    let msg = out.detection.to_string();
    assert!(
        msg.contains("out-of-band change during exec window"),
        "S4 wording: {msg}"
    );
    // Ruling 2: the ungoverned write persists — never rolled back.
    assert_eq!(
        std::fs::read_to_string(root.0.join("rogue.md")).unwrap(),
        "sneaky\n"
    );
}

/// U6b #19, the cheat a naive root-compare passes: one HONEST descriptor AND
/// a rogue tree write. The residual names exactly the rogue path, and the
/// WHOLE phase 2 refuses — even the honest effect does not apply.
#[test]
fn an_honest_descriptor_plus_rogue_write_refuses_everything() {
    let (_tmp, root) = workspace();
    let scratch = tempfile::tempdir().unwrap();
    let caps = CapSet::parse("md.set_field").unwrap();
    let mut live: Vec<u8> = Vec::new();

    let src = format!(
        "echo sneaky > '{}/rogue.md'\n{EMIT_SET_FIELD}",
        root.0.display()
    );
    let out = dispatch_bash::run(&root, &dispatch_of(&src, &scratch, &caps), &mut live).unwrap();

    assert!(matches!(out.phase2, Phase2::RefusedDetection));
    let Detection::OutOfBand(delta) = &out.detection else {
        panic!("expected OutOfBand, got {:?}", out.detection);
    };
    assert_eq!(delta.unexpected, vec!["rogue.md".to_string()]);
    // The honest descriptor did NOT apply — the page is untouched.
    assert_eq!(
        std::fs::read_to_string(root.0.join("page.md")).unwrap(),
        PAGE
    );
    // No completion receipt joined the pre-exec anchor.
    let receipts = std::fs::read_to_string(root.0.join("receipts/2026-07-22.md")).unwrap();
    assert!(receipts.contains("^p-000001"));
    assert!(!receipts.contains("^r-000001"));
}

/// U6b #20, the config-widening attack end-to-end: bash rewrites
/// `mdfs_config.yaml` to ignore its rogue path. The config bracket refuses
/// before the residual could be filtered by the widened domain.
#[test]
fn a_config_rewrite_in_the_window_refuses_phase2() {
    let (_tmp, root) = workspace();
    let scratch = tempfile::tempdir().unwrap();
    let caps = CapSet::none();
    let mut live: Vec<u8> = Vec::new();

    let src = format!(
        "printf 'ignore:\\n  - \"rogue.md\"\\n' > '{ws}/mdfs_config.yaml'\necho sneaky > '{ws}/rogue.md'",
        ws = root.0.display()
    );
    let out = dispatch_bash::run(&root, &dispatch_of(&src, &scratch, &caps), &mut live).unwrap();

    assert!(matches!(out.detection, Detection::ConfigChanged));
    assert!(matches!(out.phase2, Phase2::RefusedDetection));
    assert_eq!(
        std::fs::read_to_string(root.0.join("page.md")).unwrap(),
        PAGE
    );
}

/// U13: the sealed exec facts ride the COMMITTED completion receipt — same
/// invocation, same sha256 as the sealed record — while the PRE-exec line
/// stays bare (no child had run). Env enters keys-only (S7): the value never
/// reaches the receipt file.
#[test]
fn the_completion_receipt_carries_the_sealed_exec_facts() {
    let (_tmp, root) = workspace();
    let scratch = tempfile::tempdir().unwrap();
    let caps = CapSet::parse("md.set_field").unwrap();
    let mut live: Vec<u8> = Vec::new();

    let src = format!("echo running{EMIT_SET_FIELD}");
    let mut d = dispatch_of(&src, &scratch, &caps);
    d.env = BTreeMap::from([("HOME_WIKI".to_string(), "secret-value".to_string())]);
    let out = dispatch_bash::run(&root, &d, &mut live).unwrap();

    assert!(matches!(out.phase2, Phase2::Applied { .. }));
    let record = out.stdout.as_ref().expect("record sealed");
    let receipts = std::fs::read_to_string(root.0.join("receipts/2026-07-22.md")).unwrap();

    // The completion line (^r-000001) carries the exec facts of THE record.
    let completion = receipts
        .lines()
        .find(|l| l.contains("^r-000001"))
        .expect("completion line committed");
    assert!(completion.contains("\"exec\""), "{completion}");
    assert!(
        completion.contains(&record.sha256),
        "receipt attests the sealed stdout hash: {completion}"
    );
    assert!(completion.contains("\"exit_code\":0"), "{completion}");
    assert!(completion.contains("HOME_WIKI"), "env key recorded");
    // S7: the env VALUE never reaches the receipt file.
    assert!(!receipts.contains("secret-value"), "{receipts}");

    // The PRE-exec line (^p-000001) has no exec facts — no child had run.
    let pre = receipts
        .lines()
        .find(|l| l.contains("^p-000001"))
        .expect("pre-exec line committed");
    assert!(!pre.contains("\"exec\""), "{pre}");
}

/// U6b happy path: a clean window's verdict is `Clean`, phase 2 proceeds,
/// and the verified root IS the phase-2 pin (`root_after_phase1`) — the
/// bracket and the #19 computed-root discipline agree end-to-end.
#[test]
fn a_clean_window_verdict_rides_the_outcome() {
    let (_tmp, root) = workspace();
    let scratch = tempfile::tempdir().unwrap();
    let caps = CapSet::parse("md.set_field").unwrap();
    let mut live: Vec<u8> = Vec::new();

    let out = dispatch_bash::run(
        &root,
        &dispatch_of(EMIT_SET_FIELD, &scratch, &caps),
        &mut live,
    )
    .unwrap();

    assert!(out.detection.is_clean());
    let Phase2::Applied { effects, applied } = &out.phase2 else {
        panic!("expected Applied, got {:?}", out.phase2);
    };
    assert!(applied.is_some());
    let Detection::Clean { root: verified } = &out.detection else {
        unreachable!();
    };
    let Provenance::Run { root_at_eval, .. } = &effects[0].provenance else {
        panic!("run-plane provenance expected");
    };
    assert_eq!(
        verified.0, *root_at_eval,
        "the bracket-verified root is the phase-2 pin"
    );
}

/// U6b: the verdict is rendered on EVERY exit path — a timed-out step still
/// closes the bracket (clean here: the tree was untouched).
#[test]
fn a_timeout_still_renders_the_detection_verdict() {
    let (_tmp, root) = workspace();
    let scratch = tempfile::tempdir().unwrap();
    let caps = CapSet::none();
    let mut live: Vec<u8> = Vec::new();

    let mut d = dispatch_of("sleep 30", &scratch, &caps);
    d.timeout = Duration::from_millis(300);
    let out = dispatch_bash::run(&root, &d, &mut live).unwrap();

    assert!(matches!(out.status, ExecStatus::TimedOut { .. }));
    assert!(matches!(out.phase2, Phase2::RefusedTimeout));
    assert!(out.detection.is_clean(), "untouched tree ⇒ clean verdict");
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
