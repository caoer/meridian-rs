//! U6a gates — the two-phase bash dispatch: pre-exec receipt + locked-window
//! `root_after_phase1` (#19 addendum), the S2/S6/#21 failure matrix, the
//! executor choke point, and the U8 stdout record riding the exec — plus the
//! U6b detection gates (the wired `ExecBracket`: #14 cheat detection, the
//! phase-2 gate, ruling-2 never-roll-back).

use std::collections::BTreeMap;
use std::time::Duration;

use effects::Provenance;
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

fn dispatch_of<'a>(source: &'a str, scratch: &'a tempfile::TempDir) -> BashDispatch<'a> {
    BashDispatch {
        page: "page.md",
        task: "fix-x",
        task_rev: "b3:proc-bash",
        source,
        args: vec![],
        env: BTreeMap::new(),
        invocation_id: "inv-1",
        now: Some("2026-07-22T02:00:00Z"),
        pre_receipt: Some(ReceiptAddr {
            path: "receipts/2026-07-22.md".to_owned(),
            anchor: "p-000001".to_owned(),
        }),
        receipt: Some(ReceiptAddr {
            path: "receipts/2026-07-22.md".to_owned(),
            anchor: "r-000001".to_owned(),
        }),
        scratch: scratch.path(),
        timeout: Duration::from_secs(30),
        actor: None,
        step_cwd: None,
        delta: None,
        observations: dispatch_bash::ObservationSource::Drawer,
    }
}

/// G3 gate 1: a run the guard would refuse writes NOTHING to the attested
/// domain — no pre-exec receipt, no journal row, every domain file
/// byte-identical.
#[cfg(unix)]
#[test]
fn a_preflight_refusal_writes_nothing_to_the_attested_domain() {
    let (tmp, root) = workspace();
    let scratch = tempfile::tempdir().unwrap();

    let secret = tmp.path().join("secret.md"); // OUT of tree, beside the ws
    std::fs::write(&secret, "out-of-tree\n").unwrap();
    std::os::unix::fs::symlink(&secret, root.0.join("linked.md")).unwrap();

    let before = domain_digest(&root);
    let err = dispatch_bash::run(
        &root,
        &dispatch_of("echo hi\nprintf 'end:1\\n' >&3\n", &scratch),
        &mut Vec::new(),
    )
    .expect_err("the guarded walk must refuse this workspace");
    assert!(matches!(err, BashError::Detection(_)), "got {err:?}");

    assert_eq!(
        before,
        domain_digest(&root),
        "a refused run must leave every attested file byte-identical"
    );
    assert!(
        !root.0.join("receipts/2026-07-22.md").exists(),
        "no pre-exec receipt may be committed for a run that never started"
    );
}

/// G3 gate 2: the refusal is not silenced, it is relocated. It lands in the
/// run log under `.meridian/` — dot-path, outside the hash domain — carrying
/// the reason, the refused path, and the invocation that never started.
#[cfg(unix)]
#[test]
fn a_preflight_refusal_is_findable_in_the_run_log() {
    let (tmp, root) = workspace();
    let scratch = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("secret.md"), "out-of-tree\n").unwrap();
    std::os::unix::fs::symlink(tmp.path().join("secret.md"), root.0.join("linked.md")).unwrap();

    let _ = dispatch_bash::run(&root, &dispatch_of("echo hi\n", &scratch), &mut Vec::new());

    let runs = root.0.join(".meridian/runs");
    let logged: String = std::fs::read_dir(&runs)
        .expect("the run log directory exists")
        .filter_map(|e| std::fs::read_to_string(e.unwrap().path()).ok())
        .collect();
    assert!(logged.contains("pre-flight refused"), "got: {logged}");
    assert!(logged.contains("linked.md"), "the refused path is named");
    assert!(logged.contains("inv-1"), "the would-be invocation is named");
    assert!(
        logged.contains("never started"),
        "the outcome is unambiguous"
    );
}

/// Every domain file's bytes, folded to one digest — the "did anything
/// attested move" question, asked without depending on any engine output.
fn domain_digest(root: &fs::WorkspaceRoot) -> Vec<(String, Vec<u8>)> {
    let domain = fs::domain::Domain::load(root).unwrap();
    fs::hash_domain(root, &domain)
        .unwrap()
        .into_iter()
        .map(|rel| {
            let bytes = std::fs::read(root.0.join(&rel)).unwrap_or_default();
            (rel.to_string_lossy().into_owned(), bytes)
        })
        .collect()
}

#[test]
fn a_clean_run_applies_the_shim_batch_two_phase() {
    let (_tmp, root) = workspace();
    let scratch = tempfile::tempdir().unwrap();
    let root_before = fs::domain_snapshot(&root).unwrap().1;
    let mut live: Vec<u8> = Vec::new();

    let src = format!("echo running{EMIT_SET_FIELD}");
    let out = dispatch_bash::run(&root, &dispatch_of(&src, &scratch), &mut live).unwrap();

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

/// G3b, the pair's first half — COMPLETION IS NOT SUCCESS. A block that ran to
/// a nonzero exit gets its effects refused AND its completion recorded: a
/// check page exiting nonzero on a finding must not leave the same bytes as a
/// crash.
#[test]
fn a_nonzero_exit_refuses_phase2_and_phase1_stands() {
    let (_tmp, root) = workspace();
    let scratch = tempfile::tempdir().unwrap();
    let mut live: Vec<u8> = Vec::new();

    // A VALID stream, then a failing exit — S2: exec-fail refuses phase 2.
    let src = format!("{EMIT_SET_FIELD}\nexit 7");
    let out = dispatch_bash::run(&root, &dispatch_of(&src, &scratch), &mut live).unwrap();

    assert_eq!(out.status, ExecStatus::Exited { code: 7 });
    let Phase2::RefusedExecFailed { applied } = &out.phase2 else {
        panic!("expected a recorded exec failure, got {:?}", out.phase2);
    };
    // The refusal half, unchanged and still the law: the block emitted a
    // set_field descriptor and NOTHING of it applied.
    assert_eq!(applied.applied, 0);
    assert_eq!(
        std::fs::read_to_string(root.0.join("page.md")).unwrap(),
        PAGE
    );
    // The record half, new: the pre-exec receipt is JOINED by a completion
    // receipt carrying the exit code, so this run is no longer an orphan.
    let receipts = std::fs::read_to_string(root.0.join("receipts/2026-07-22.md")).unwrap();
    assert!(receipts.contains("^p-000001"));
    assert!(
        receipts.contains("^r-000001"),
        "completion receipt: {receipts}"
    );
    assert!(
        receipts.contains("\"exit_code\":7"),
        "the completion receipt carries the exit code: {receipts}"
    );
    assert!(
        !receipts.contains("set_field"),
        "the refused effects are NOT recorded as applied: {receipts}"
    );
}

/// G3b, the pair's second half — a true crash STILL leaves an orphan. A
/// signaled step did not run to an exit, so no completion can honestly be
/// written and the lone pre-exec receipt is the correct record. This is the
/// assertion that keeps the first half from becoming "every run gets a
/// receipt", which would delete the orphan lint's whole population.
#[test]
fn a_signaled_step_records_no_completion_and_stays_an_orphan() {
    let (_tmp, root) = workspace();
    let scratch = tempfile::tempdir().unwrap();
    let mut live: Vec<u8> = Vec::new();

    // A valid stream, then the step kills ITSELF — the supervisor's own
    // step-end SIGKILL would be indistinguishable from a timeout, so the
    // signal has to come from inside the block.
    let src = format!("{EMIT_SET_FIELD}\nkill -TERM $$\nsleep 5");
    let out = dispatch_bash::run(&root, &dispatch_of(&src, &scratch), &mut live).unwrap();

    assert!(
        matches!(out.status, ExecStatus::Signaled { .. }),
        "expected a signaled step, got {:?}",
        out.status
    );
    assert!(matches!(out.phase2, Phase2::RefusedSignaled));
    assert_eq!(
        std::fs::read_to_string(root.0.join("page.md")).unwrap(),
        PAGE
    );
    let receipts = std::fs::read_to_string(root.0.join("receipts/2026-07-22.md")).unwrap();
    assert!(receipts.contains("^p-000001"));
    assert!(
        !receipts.contains("^r-000001"),
        "a crash leaves the orphan anchor alone: {receipts}"
    );
}

#[test]
fn a_truncated_stream_fails_the_whole_batch_closed() {
    let (_tmp, root) = workspace();
    let scratch = tempfile::tempdir().unwrap();
    let mut live: Vec<u8> = Vec::new();

    // A record with NO trailer and a clean exit — S6: fails closed.
    let src = r#"
p='{"op":"md.set_field","field":"status","value":"done"}'
printf '%s:%s\n' "${#p}" "$p" >&3
"#;
    let out = dispatch_bash::run(&root, &dispatch_of(src, &scratch), &mut live).unwrap();

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
    let mut live: Vec<u8> = Vec::new();

    let mut d = dispatch_of("sleep 30", &scratch);
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

/// Zero descriptors on a clean exit is not a fault — and it is not silence
/// either: the run happened, so it gets a completion receipt. Without one,
/// "authorised and never started" and "ran and changed nothing" would be the
/// same bytes. The page stays untouched.
#[test]
fn zero_descriptors_on_a_clean_exit_is_not_a_fault() {
    let (_tmp, root) = workspace();
    let scratch = tempfile::tempdir().unwrap();
    let mut live: Vec<u8> = Vec::new();

    let out = dispatch_bash::run(&root, &dispatch_of("echo ok", &scratch), &mut live).unwrap();

    assert!(out.status.success());
    // Not a fault: the empty batch applied, and it applied NOTHING.
    let Phase2::Applied { effects, applied } = &out.phase2 else {
        panic!(
            "an empty batch on a clean exit is not a fault, got {:?}",
            out.phase2
        );
    };
    assert!(effects.is_empty(), "no descriptors were emitted");
    assert!(
        applied.is_some(),
        "the run happened, so it must leave a completion receipt — without one \
         it is indistinguishable from a run that never started"
    );
    assert_eq!(
        applied.as_ref().unwrap().applied,
        0,
        "a completion receipt for an empty batch applies zero effects"
    );
    // Unchanged intent: the page the task did not touch stays byte-identical.
    assert_eq!(
        std::fs::read_to_string(root.0.join("page.md")).unwrap(),
        PAGE
    );
}

/// The join the orphan lint needs: a completed run leaves BOTH receipts — the
/// phase-1 pre-exec anchor and a phase-2 completion — at distinct anchors, even
/// when it emitted no effects. A pre-exec receipt with no completion is then a
/// readable state rather than an indistinguishable one.
#[test]
fn a_completed_zero_effect_run_leaves_both_receipts() {
    let (_tmp, root) = workspace();
    let scratch = tempfile::tempdir().unwrap();

    dispatch_bash::run(&root, &dispatch_of("echo ok", &scratch), &mut Vec::new()).unwrap();

    let receipts = std::fs::read_to_string(root.0.join("receipts/2026-07-22.md")).unwrap();
    assert!(
        receipts.contains("^p-000001"),
        "the phase-1 pre-exec anchor is present: {receipts}"
    );
    assert!(
        receipts.contains("^r-000001"),
        "the phase-2 completion anchor is present: {receipts}"
    );
}

/// Capabilities do not apply to bash (`docs/laws.md` § Amendment): the
/// dispatcher passes `Authority::Unsandboxed` and the descriptor APPLIES.
/// Denying the shim would only push the same write to `sed -i`, off the
/// attested path; `law_no_caps_on_bash.rs` holds both halves.
#[test]
fn an_undeclared_bash_descriptor_applies_ungoverned() {
    let (_tmp, root) = workspace();
    let scratch = tempfile::tempdir().unwrap();
    let mut live: Vec<u8> = Vec::new();

    let out = dispatch_bash::run(&root, &dispatch_of(EMIT_SET_FIELD, &scratch), &mut live).unwrap();

    assert!(
        matches!(out.phase2, Phase2::Applied { .. }),
        "{:?}",
        out.phase2
    );
    assert!(
        std::fs::read_to_string(root.0.join("page.md"))
            .unwrap()
            .contains("status: done"),
        "the descriptor did not apply"
    );
}

#[test]
fn a_held_workspace_lock_is_a_fast_typed_refusal() {
    let (_tmp, root) = workspace();
    let scratch = tempfile::tempdir().unwrap();
    let mut live: Vec<u8> = Vec::new();

    let _held = WorkspaceLock::acquire(&root.0).unwrap();
    let err = dispatch_bash::run(&root, &dispatch_of("echo hi", &scratch), &mut live).unwrap_err();
    assert!(matches!(err, BashError::Phase1(ExecError::WorkspaceBusy)));
}

#[test]
fn an_unsafe_invocation_id_refuses_before_anything_commits() {
    let (_tmp, root) = workspace();
    let scratch = tempfile::tempdir().unwrap();
    let mut live: Vec<u8> = Vec::new();

    let mut d = dispatch_of("echo hi", &scratch);
    d.invocation_id = "../evil";
    let err = dispatch_bash::run(&root, &d, &mut live).unwrap_err();
    assert!(matches!(err, BashError::Record(_)));
    assert!(!root.0.join("receipts").exists(), "nothing committed");
}

/// U6b #14, the zero-descriptor cheat end-to-end: bash writes an md file
/// into the TREE, emits nothing on the shim fd, exits 0. Phase 2 refuses, the
/// delta names the file (S4 wording), and the write is NEVER rolled back
/// (ruling 2).
#[test]
fn an_ungoverned_tree_write_refuses_phase2_with_the_delta_named() {
    let (_tmp, root) = workspace();
    let scratch = tempfile::tempdir().unwrap();
    let mut live: Vec<u8> = Vec::new();

    let src = format!("echo sneaky > '{}/rogue.md'", root.0.display());
    let out = dispatch_bash::run(&root, &dispatch_of(&src, &scratch), &mut live).unwrap();

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
    let mut live: Vec<u8> = Vec::new();

    let src = format!(
        "echo sneaky > '{}/rogue.md'\n{EMIT_SET_FIELD}",
        root.0.display()
    );
    let out = dispatch_bash::run(&root, &dispatch_of(&src, &scratch), &mut live).unwrap();

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

/// U16 honest semantics: the step now runs in the invocation cwd and is handed
/// `$MERIDIAN_PROJECT_ROOT`, so reaching the tree is EASY. Reaching it is not
/// tolerating it — a stray write through that very variable is detected as
/// `OutOfBand` and REFUSES convergence: nothing applies, no completion receipt,
/// and (ruling 2) the write is never rolled back.
#[test]
fn a_project_root_relative_stray_write_refuses_convergence() {
    let (_tmp, root) = workspace();
    let scratch = tempfile::tempdir().unwrap();
    let mut live: Vec<u8> = Vec::new();

    let src = format!("echo stray > \"$MERIDIAN_PROJECT_ROOT/stray.md\"\n{EMIT_SET_FIELD}");
    let out = dispatch_bash::run(&root, &dispatch_of(&src, &scratch), &mut live).unwrap();

    assert!(out.status.success(), "the step itself exited 0");
    assert!(matches!(out.phase2, Phase2::RefusedDetection));
    let Detection::OutOfBand(delta) = &out.detection else {
        panic!("expected OutOfBand, got {:?}", out.detection);
    };
    assert_eq!(delta.unexpected, vec!["stray.md".to_string()]);
    // Refused, not merely reported: the honest descriptor did not apply.
    assert_eq!(
        std::fs::read_to_string(root.0.join("page.md")).unwrap(),
        PAGE
    );
    let receipts = std::fs::read_to_string(root.0.join("receipts/2026-07-22.md")).unwrap();
    assert!(!receipts.contains("^r-000001"), "no completion receipt");
    // Ruling 2: never rolled back.
    assert_eq!(
        std::fs::read_to_string(root.0.join("stray.md")).unwrap(),
        "stray\n"
    );
}

/// U6b #20, the config-widening attack end-to-end: bash rewrites
/// `mdfs_config.yaml` to ignore its rogue path. The config bracket refuses
/// before the residual could be filtered by the widened domain.
#[test]
fn a_config_rewrite_in_the_window_refuses_phase2() {
    let (_tmp, root) = workspace();
    let scratch = tempfile::tempdir().unwrap();
    let mut live: Vec<u8> = Vec::new();

    let src = format!(
        "printf 'ignore:\\n  - \"rogue.md\"\\n' > '{ws}/mdfs_config.yaml'\necho sneaky > '{ws}/rogue.md'",
        ws = root.0.display()
    );
    let out = dispatch_bash::run(&root, &dispatch_of(&src, &scratch), &mut live).unwrap();

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
    let mut live: Vec<u8> = Vec::new();

    let src = format!("echo running{EMIT_SET_FIELD}");
    let mut d = dispatch_of(&src, &scratch);
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
    let mut live: Vec<u8> = Vec::new();

    let out = dispatch_bash::run(&root, &dispatch_of(EMIT_SET_FIELD, &scratch), &mut live).unwrap();

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
    let mut live: Vec<u8> = Vec::new();

    let mut d = dispatch_of("sleep 30", &scratch);
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
    let root_before = fs::domain_snapshot(&root).unwrap().1;
    let mut live: Vec<u8> = Vec::new();

    let mut d = dispatch_of(EMIT_SET_FIELD, &scratch);
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

// ---- run-walk-real-roots (dogfood r2 D-USER F2): the run plane on a ----
// ---- sessions-shaped root.                                           ----

/// The sessions-root fixture: stranger symlinks in the live root's own shape —
/// venv links under real `scratch*` dirs (a non-ASCII name among them),
/// symlinked `bin` DIRECTORIES under results experiment dirs, and a dot-path
/// snapshot link. Under the declared domain shape the whole plane comes back:
/// the walk skips every stranger and the bash task RUNS.
#[cfg(unix)]
#[test]
fn bash_runs_on_a_sessions_shaped_root_under_the_declared_domain_shape() {
    let (tmp, root) = workspace();
    let scratch = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.0.join("meridian")).unwrap();
    std::fs::write(
        root.0.join("meridian/domain.md"),
        "---\nversion: 1\nignore:\n  - \"scratch*/\"\n  - \"bin/\"\n---\n\n# Domain\n\nScratch is ungoverned by definition; venv bin dirs are strangers' tooling.\n",
    )
    .unwrap();

    let out_of_tree = tmp.path().join("stranger-target");
    std::fs::create_dir_all(&out_of_tree).unwrap();
    std::fs::write(out_of_tree.join("t.md"), "elsewhere\n").unwrap();

    // 97-shape: venv file links (one with a non-ASCII name) under real scratch dirs.
    let venv = root
        .0
        .join("year=2026/month=07/s1/results/team-g/scratch-r2-verify/venv/bin");
    std::fs::create_dir_all(&venv).unwrap();
    std::os::unix::fs::symlink(out_of_tree.join("t.md"), venv.join("\u{1D70B}thon")).unwrap();
    let venv2 = root.0.join("year=2026/month=08/s2/scratch/venv/bin");
    std::fs::create_dir_all(&venv2).unwrap();
    std::os::unix::fs::symlink(out_of_tree.join("t.md"), venv2.join("python")).unwrap();
    // 24-shape: the run dir's `bin` IS the symlink (a directory link).
    std::fs::create_dir_all(root.0.join("results/u16-experiment/run-e1-h1")).unwrap();
    std::os::unix::fs::symlink(
        &out_of_tree,
        root.0.join("results/u16-experiment/run-e1-h1/bin"),
    )
    .unwrap();
    // dot-shape: a snapshot subtree in the dot gap.
    std::fs::create_dir_all(root.0.join(".scratch-snap/run")).unwrap();
    std::os::unix::fs::symlink(&out_of_tree, root.0.join(".scratch-snap/run/bin")).unwrap();

    let outcome = dispatch_bash::run(
        &root,
        &dispatch_of("echo alive\nprintf 'end:0\\n' >&3\n", &scratch),
        &mut Vec::new(),
    )
    .expect("under the declared domain shape the walk must not refuse");
    assert!(
        outcome.detection.is_clean(),
        "the window verifies clean: {:?}",
        outcome.detection
    );
    assert!(
        matches!(outcome.status, ExecStatus::Exited { code: 0 }),
        "the block really ran: {:?}",
        outcome.status
    );
    assert!(
        matches!(outcome.phase2, Phase2::Applied { .. }),
        "phase 2 commits on the clean window: {:?}",
        outcome.phase2
    );
}

/// The same root WITHOUT the declared shape refuses — and the refusal is a
/// count plus the first offender, not one mine per attempt: a caller can size
/// the cleanup (or the missing domain shape) from one answer.
#[cfg(unix)]
#[test]
fn the_dispatch_refusal_names_the_count_and_the_first_offender() {
    let (tmp, root) = workspace();
    let scratch = tempfile::tempdir().unwrap();

    let out_of_tree = tmp.path().join("stranger-target");
    std::fs::create_dir_all(&out_of_tree).unwrap();
    std::fs::write(out_of_tree.join("t.md"), "elsewhere\n").unwrap();
    let venv = root
        .0
        .join("year=2026/month=07/s1/results/team-g/scratch-r2-verify/venv/bin");
    std::fs::create_dir_all(&venv).unwrap();
    std::os::unix::fs::symlink(out_of_tree.join("t.md"), venv.join("\u{1D70B}thon")).unwrap();
    let venv2 = root.0.join("year=2026/month=08/s2/scratch/venv/bin");
    std::fs::create_dir_all(&venv2).unwrap();
    std::os::unix::fs::symlink(out_of_tree.join("t.md"), venv2.join("python")).unwrap();
    std::fs::create_dir_all(root.0.join("results/u16-experiment/run-e1-h1")).unwrap();
    std::os::unix::fs::symlink(
        &out_of_tree,
        root.0.join("results/u16-experiment/run-e1-h1/bin"),
    )
    .unwrap();

    let err = dispatch_bash::run(
        &root,
        &dispatch_of("echo alive\nprintf 'end:0\\n' >&3\n", &scratch),
        &mut Vec::new(),
    )
    .expect_err("without the domain shape the strangers refuse the walk");
    let text = err.to_string();
    assert!(
        text.contains("3 symlinked paths refused in exec-window snapshot"),
        "the count is the claim; got: {text}"
    );
    assert!(
        text.contains("first: results/u16-experiment/run-e1-h1/bin"),
        "the first offender (sorted) is named; got: {text}"
    );
}

/// Card run-preexec-severity: a run whose observed pre-exec tree folds to
/// the computed `root_after_phase1` carries NO pre-exec fact — absence has
/// one meaning (clean), on the outcome as in the report.
#[cfg(unix)]
#[test]
fn a_quiet_pre_exec_gap_reports_no_divergence() {
    let (_tmp, root) = workspace();
    let scratch = tempfile::tempdir().unwrap();
    let out = dispatch_bash::run(
        &root,
        &dispatch_of("printf 'end:0\\n' >&3\n", &scratch),
        &mut Vec::new(),
    )
    .unwrap();
    assert!(out.pre_exec.is_none(), "got {:?}", out.pre_exec);
    assert!(matches!(out.phase2, Phase2::Applied { .. }));
}

// ---------------------------------------------------------------------------
// Observation-source lane equivalence (card run-observation-unification):
// the daemon lane — bracket observations served from a resident
// `fs::DomainCache` — must produce the SAME dispatch outcome as the CLI
// drawer lane on an identical tree. The observation values are pinned
// equivalent at the fs layer (`crates/fs/tests/cached_observation.rs`);
// these gates pin the dispatch seam itself.

/// A clean two-phase run is byte-identical across lanes: same verdict, same
/// pin, same attested tree afterwards (two identical trees fold to identical
/// roots — the merkle is path-relative).
#[test]
fn the_resident_lane_runs_the_same_clean_dispatch_as_the_drawer_lane() {
    let (_tmp_a, drawer_root) = workspace();
    let (_tmp_b, resident_root) = workspace();
    let scratch = tempfile::tempdir().unwrap();
    let src = format!("echo running{EMIT_SET_FIELD}");

    let drawer_out =
        dispatch_bash::run(&drawer_root, &dispatch_of(&src, &scratch), &mut Vec::new()).unwrap();

    let cache = std::sync::Mutex::new(fs::DomainCache::new());
    let mut resident = dispatch_of(&src, &scratch);
    resident.observations = dispatch_bash::ObservationSource::Resident(&cache);
    let resident_out = dispatch_bash::run(&resident_root, &resident, &mut Vec::new()).unwrap();

    for (out, lane) in [(&drawer_out, "drawer"), (&resident_out, "resident")] {
        assert!(out.status.success(), "{lane} lane exec failed");
        assert!(
            out.pre_exec.is_none(),
            "{lane} lane saw a pre-exec divergence"
        );
        assert!(
            out.detection.is_clean(),
            "{lane} lane window not clean: {:?}",
            out.detection
        );
        assert!(
            matches!(out.phase2, Phase2::Applied { .. }),
            "{lane} lane phase2: {:?}",
            out.phase2
        );
    }

    // The strongest form: every attested byte equal, tree to tree.
    assert_eq!(
        domain_digest(&drawer_root),
        domain_digest(&resident_root),
        "the two lanes must leave byte-identical attested trees"
    );
}

/// An in-window rogue write is named identically from the resident lane —
/// same refusal, same delta, same wording, and (ruling 2) the write stands.
#[test]
fn the_resident_lane_names_the_same_out_of_band_delta() {
    let (_tmp_a, drawer_root) = workspace();
    let (_tmp_b, resident_root) = workspace();
    let scratch = tempfile::tempdir().unwrap();

    let run_rogue = |root: &fs::WorkspaceRoot,
                     observations: dispatch_bash::ObservationSource<'_>| {
        let src = format!("echo sneaky > '{}/rogue.md'", root.0.display());
        let mut d = dispatch_of(&src, &scratch);
        d.observations = observations;
        // The source string must outlive the dispatch borrow — run inline.
        let out =
            dispatch_bash::run(root, &BashDispatch { source: &src, ..d }, &mut Vec::new()).unwrap();
        assert!(matches!(out.phase2, Phase2::RefusedDetection));
        match out.detection {
            Detection::OutOfBand(delta) => delta,
            other => panic!("expected OutOfBand, got {other:?}"),
        }
    };

    let drawer_delta = run_rogue(&drawer_root, dispatch_bash::ObservationSource::Drawer);
    let cache = std::sync::Mutex::new(fs::DomainCache::new());
    let resident_delta = run_rogue(
        &resident_root,
        dispatch_bash::ObservationSource::Resident(&cache),
    );
    assert_eq!(drawer_delta, resident_delta, "residual deltas diverge");
    assert_eq!(drawer_delta.to_string(), resident_delta.to_string());
}

/// The point of the unification: a resident-lane dispatch leaves the shared
/// memo WARM — the next currency pass over the unchanged tree enumerates
/// nothing and reads nothing, exactly what the host's other ops buy from the
/// same instrument.
#[test]
fn a_resident_lane_dispatch_leaves_the_shared_cache_warm() {
    let (_tmp, root) = workspace();
    let scratch = tempfile::tempdir().unwrap();
    let src = format!("echo running{EMIT_SET_FIELD}");

    let cache = std::sync::Mutex::new(fs::DomainCache::new());
    let mut d = dispatch_of(&src, &scratch);
    d.observations = dispatch_bash::ObservationSource::Resident(&cache);
    let out = dispatch_bash::run(&root, &d, &mut Vec::new()).unwrap();
    assert!(out.detection.is_clean(), "{:?}", out.detection);

    let mut memo = cache.lock().unwrap();
    // Phase 2 committed AFTER the bracket-close observation (the page splice
    // and the completion receipt), so the first pass pays exactly that delta —
    // never the corpus — and the pass after it pays nothing at all.
    let reads = memo.leaves_read();
    let after_run = memo.root(&root).unwrap();
    assert!(
        memo.leaves_read() - reads <= 2,
        "the first pass re-reads at most the two phase-2 movers, got {}",
        memo.leaves_read() - reads
    );
    let warm = (memo.listings(), memo.leaves_read());
    let again = memo.root(&root).unwrap();
    assert_eq!(
        (memo.listings(), memo.leaves_read()),
        warm,
        "the second pass over the unchanged tree must be stat-only"
    );
    assert_eq!(after_run, again);
    assert_eq!(
        after_run,
        fs::domain_snapshot(&root).unwrap().1,
        "and the fold still agrees with the byte-derived root"
    );
}
