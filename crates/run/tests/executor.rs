//! Executor gates (U4): choke-point cap validation, one atomic `if_root`
//! batch, self-guards + flock, receipts, apply→event synthesis, and the
//! decision-#26 foreign-edit law.

use std::collections::BTreeMap;

use effects::{ArgValue, Effect, EffectKind, Provenance};
use model::MerkleRoot;
use run::caps::{Authority, CapSet};
use run::executor::{self, ApplyRequest, ExecError, ReceiptAddr};

const PAGE: &str = "\
---
status: todo
---

# Tasks

## Log

- existing line
";

fn workspace() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("page.md"), PAGE).unwrap();
    let root = fs::WorkspaceRoot(tmp.path().to_owned());
    (tmp, root)
}

fn current_root(root: &fs::WorkspaceRoot) -> MerkleRoot {
    fs::domain_snapshot(root).unwrap().1
}

fn effect(kind: EffectKind, args: &[(&str, &str)], seq: u32) -> Effect {
    Effect {
        kind,
        rule_id: "t".to_owned(),
        seq,
        depth: 0,
        provenance: Provenance::Run {
            invocation_id: "inv-1".to_owned(),
            root_at_eval: "b3:x".to_owned(),
        },
        args: args
            .iter()
            .map(|(k, v)| ((*k).to_owned(), ArgValue::Str((*v).to_owned())))
            .collect::<BTreeMap<_, _>>(),
    }
}

fn set_field(field: &str, value: &str, seq: u32) -> Effect {
    effect(
        EffectKind::SetField,
        &[("field", field), ("value", value)],
        seq,
    )
}

fn append(section: &str, content: &str, seq: u32) -> Effect {
    effect(
        EffectKind::AppendSection,
        &[("section", section), ("content", content)],
        seq,
    )
}

fn receipt_addr(n: u64) -> ReceiptAddr {
    ReceiptAddr {
        path: "receipts/2026-07-22.md".to_owned(),
        anchor: format!("r-{n:06}"),
    }
}

struct Req<'a> {
    effects: &'a [Effect],
    caps: CapSet,
    pin: MerkleRoot,
    live: MerkleRoot,
    receipt: Option<ReceiptAddr>,
    takeover: bool,
}

fn apply(root: &fs::WorkspaceRoot, r: &Req<'_>) -> Result<executor::Applied, ExecError> {
    executor::apply(
        root,
        &ApplyRequest {
            page: "page.md",
            task: "fix-x",
            task_rev: "b3:proc-test",
            invocation_id: "inv-1",
            now: Some("2026-07-22T01:00:00Z"),
            effects: r.effects,
            authority: &Authority::granted(r.caps.clone()),
            pin_root: &r.pin,
            live_root: &r.live,
            receipt: r.receipt.clone(),
            takeover: r.takeover,
            exec: None,
            depth: 0,
        },
    )
}

fn write_caps() -> CapSet {
    CapSet::parse("md.set_field md.append_section").unwrap()
}

fn page_text(root: &fs::WorkspaceRoot) -> String {
    std::fs::read_to_string(root.0.join("page.md")).unwrap()
}

#[test]
fn set_field_applies_receipts_and_synthesizes_the_event() {
    let (_tmp, root) = workspace();
    let now = current_root(&root);
    let applied = apply(
        &root,
        &Req {
            effects: &[set_field("status", "done", 0)],
            caps: write_caps(),
            pin: now.clone(),
            live: now,
            receipt: Some(receipt_addr(1)),
            takeover: false,
        },
    )
    .unwrap();

    assert!(page_text(&root).contains("status: done"));
    assert_eq!(applied.applied, 1);

    // Receipt rode the same commit, machine-re-readable.
    let receipt = std::fs::read_to_string(root.0.join("receipts/2026-07-22.md")).unwrap();
    assert!(receipt.starts_with("- run {"), "{receipt}");
    assert!(receipt.contains("\"actor\":\"run:fix-x\""));
    assert!(receipt.trim_end().ends_with("^r-000001"));
    assert_eq!(applied.receipt_line.as_deref(), Some(receipt.trim_end()));

    // Apply→event synthesis: REAL fingerprints, the changed field named,
    // depth = applied generation + 1.
    let event = applied.event.expect("a real change synthesizes an event");
    assert_eq!(event.file, "page.md");
    assert!(event.fields_changed.contains(&"status".to_owned()));
    assert_ne!(event.fingerprint_before, event.fingerprint_after);
    assert_eq!(event.fingerprint_after, applied.file_rev_after);
    assert_eq!(event.depth, 1);
}

#[test]
fn append_section_lands_inside_the_section() {
    let (_tmp, root) = workspace();
    let now = current_root(&root);
    let applied = apply(
        &root,
        &Req {
            effects: &[append("Log", "- appended", 0)],
            caps: write_caps(),
            pin: now.clone(),
            live: now,
            receipt: Some(receipt_addr(1)),
            takeover: false,
        },
    )
    .unwrap();
    let text = page_text(&root);
    assert!(text.contains("- existing line\n- appended\n"), "{text}");
    let event = applied.event.unwrap();
    assert!(
        event.sections_changed.iter().any(|s| s.contains("Log")),
        "{:?}",
        event.sections_changed
    );
}

#[test]
fn joint_field_and_section_batch_synthesizes_event_without_node_names() {
    let (_tmp, root) = workspace();
    let now = current_root(&root);
    let applied = apply(
        &root,
        &Req {
            effects: &[
                set_field("status", "done", 0),
                append("Log", "- appended", 1),
            ],
            caps: write_caps(),
            pin: now.clone(),
            live: now,
            receipt: Some(receipt_addr(1)),
            takeover: false,
        },
    )
    .unwrap();

    let text = page_text(&root);
    assert!(text.contains("status: done"), "{text}");
    assert!(text.contains("- existing line\n- appended\n"), "{text}");
    assert_eq!(applied.applied, 2);

    let event = applied.event.expect("a real change synthesizes an event");
    eprintln!(
        "POPULATION synthesized fields={} sections={}",
        event.fields_changed.len(),
        event.sections_changed.len()
    );
    assert!(event.fields_changed.is_empty(), "{event:?}");
    assert!(event.sections_changed.is_empty(), "{event:?}");
    assert_ne!(event.fingerprint_before, event.fingerprint_after);
    assert_eq!(event.depth, 1);
}

#[test]
fn choke_point_denies_undeclared_kind_before_any_io() {
    let (_tmp, root) = workspace();
    let now = current_root(&root);
    let err = apply(
        &root,
        &Req {
            effects: &[set_field("status", "done", 0)],
            caps: CapSet::none(),
            pin: now.clone(),
            live: now,
            receipt: Some(receipt_addr(1)),
            takeover: false,
        },
    )
    .unwrap_err();
    assert_eq!(
        err,
        ExecError::CapDenied {
            kind: "md.set_field".to_owned(),
            target: "status".to_owned(),
        }
    );
    assert_eq!(page_text(&root), PAGE, "nothing applied");
    assert!(!root.0.join("receipts").exists(), "no receipt on refusal");
}

#[test]
fn target_scoped_cap_binds_at_the_choke() {
    let (_tmp, root) = workspace();
    let now = current_root(&root);
    let caps = CapSet::parse("md.set_field:status").unwrap();
    // status admitted…
    apply(
        &root,
        &Req {
            effects: &[set_field("status", "done", 0)],
            caps: caps.clone(),
            pin: now.clone(),
            live: now,
            receipt: None,
            takeover: false,
        },
    )
    .unwrap();
    // …title denied.
    let now = current_root(&root);
    let err = apply(
        &root,
        &Req {
            effects: &[set_field("title", "X", 0)],
            caps,
            pin: now.clone(),
            live: now,
            receipt: None,
            takeover: false,
        },
    )
    .unwrap_err();
    assert!(matches!(err, ExecError::CapDenied { target, .. } if target == "title"));
}

#[test]
fn one_denied_effect_refuses_the_whole_batch() {
    let (_tmp, root) = workspace();
    let now = current_root(&root);
    let caps = CapSet::parse("md.set_field").unwrap();
    let err = apply(
        &root,
        &Req {
            effects: &[set_field("status", "done", 0), append("Log", "- x", 1)],
            caps,
            pin: now.clone(),
            live: now,
            receipt: None,
            takeover: false,
        },
    )
    .unwrap_err();
    assert!(matches!(err, ExecError::CapDenied { .. }));
    assert_eq!(page_text(&root), PAGE, "atomic: nothing applied");
}

#[test]
fn non_md_effect_is_a_dispatch_bug_refused_loud() {
    let (_tmp, root) = workspace();
    let now = current_root(&root);
    let err = apply(
        &root,
        &Req {
            effects: &[effect(EffectKind::Notice, &[("message", "hi")], 0)],
            caps: write_caps(),
            pin: now.clone(),
            live: now,
            receipt: None,
            takeover: false,
        },
    )
    .unwrap_err();
    assert!(matches!(err, ExecError::NonMdEffect { .. }));
}

#[test]
fn stale_pin_is_root_mismatch_nothing_applied() {
    let (_tmp, root) = workspace();
    let live = current_root(&root);
    let err = apply(
        &root,
        &Req {
            effects: &[set_field("status", "done", 0)],
            caps: write_caps(),
            pin: MerkleRoot("b3:stale".to_owned()),
            live,
            receipt: Some(receipt_addr(1)),
            takeover: false,
        },
    )
    .unwrap_err();
    assert!(matches!(err, ExecError::RootMismatch { .. }));
    assert_eq!(page_text(&root), PAGE);
}

#[test]
fn missing_and_ambiguous_sections_are_typed() {
    let (_tmp, root) = workspace();
    let now = current_root(&root);
    let err = apply(
        &root,
        &Req {
            effects: &[append("Nope", "- x", 0)],
            caps: write_caps(),
            pin: now.clone(),
            live: now.clone(),
            receipt: None,
            takeover: false,
        },
    )
    .unwrap_err();
    assert_eq!(
        err,
        ExecError::SectionNotFound {
            section: "Nope".to_owned()
        }
    );

    std::fs::write(
        root.0.join("page.md"),
        "# A\n\n## Log\n\na\n\n# B\n\n## Log\n\nb\n",
    )
    .unwrap();
    let now = current_root(&root);
    let err = apply(
        &root,
        &Req {
            effects: &[append("Log", "- x", 0)],
            caps: write_caps(),
            pin: now.clone(),
            live: now,
            receipt: None,
            takeover: false,
        },
    )
    .unwrap_err();
    assert_eq!(
        err,
        ExecError::SectionAmbiguous {
            section: "Log".to_owned(),
            count: 2
        }
    );
}

#[test]
fn foreign_edit_is_refused_with_the_three_way_frame() {
    let (_tmp, root) = workspace();
    let now = current_root(&root);
    // Run 1: governed write, receipt anchors fm:status.
    apply(
        &root,
        &Req {
            effects: &[set_field("status", "done", 0)],
            caps: write_caps(),
            pin: now.clone(),
            live: now,
            receipt: Some(receipt_addr(1)),
            takeover: false,
        },
    )
    .unwrap();

    // A HUMAN edits the field out-of-band. A re-run loads this state, so its
    // CAS token matches — only the receipt anchor can see the divergence.
    let edited = page_text(&root).replace("status: done", "status: human-truth");
    std::fs::write(root.0.join("page.md"), edited).unwrap();

    let now = current_root(&root);
    let err = apply(
        &root,
        &Req {
            effects: &[set_field("status", "auto", 0)],
            caps: write_caps(),
            pin: now.clone(),
            live: now.clone(),
            receipt: Some(receipt_addr(2)),
            takeover: false,
        },
    )
    .unwrap_err();
    let ExecError::ForeignEdit {
        target,
        last_governed,
        current,
    } = err
    else {
        panic!("expected ForeignEdit, got {err:?}");
    };
    assert_eq!(target, "fm:status");
    assert_ne!(last_governed, current, "the three-way frame is real");
    assert!(
        page_text(&root).contains("status: human-truth"),
        "preserved"
    );

    // Explicit takeover overrides (decision #26).
    apply(
        &root,
        &Req {
            effects: &[set_field("status", "auto", 0)],
            caps: write_caps(),
            pin: now.clone(),
            live: now,
            receipt: Some(receipt_addr(2)),
            takeover: true,
        },
    )
    .unwrap();
    assert!(page_text(&root).contains("status: auto"));
}

#[test]
fn re_run_without_foreign_edit_is_clean() {
    let (_tmp, root) = workspace();
    let now = current_root(&root);
    apply(
        &root,
        &Req {
            effects: &[set_field("status", "done", 0)],
            caps: write_caps(),
            pin: now.clone(),
            live: now,
            receipt: Some(receipt_addr(1)),
            takeover: false,
        },
    )
    .unwrap();
    // No human touched the target: current rev == last governed after-rev.
    let now = current_root(&root);
    apply(
        &root,
        &Req {
            effects: &[set_field("status", "review", 0)],
            caps: write_caps(),
            pin: now.clone(),
            live: now,
            receipt: Some(receipt_addr(2)),
            takeover: false,
        },
    )
    .unwrap();
    assert!(page_text(&root).contains("status: review"));
}

#[test]
fn flock_serializes_two_concurrent_appliers() {
    // LOCK_NB (decision #9): a loser never waits inside the executor — it gets
    // the typed WorkspaceBusy refusal and retries HERE, in the caller. Both
    // appends still land, never interleaved.
    let (_tmp, root) = workspace();
    std::thread::scope(|scope| {
        for i in 0..2u32 {
            let root = root.clone();
            scope.spawn(move || {
                loop {
                    let now = current_root(&root);
                    match apply(
                        &root,
                        &Req {
                            effects: &[append("Log", &format!("- from thread {i}"), 0)],
                            caps: write_caps(),
                            pin: now.clone(),
                            live: now,
                            receipt: Some(receipt_addr(u64::from(i) + 1)),
                            takeover: false,
                        },
                    ) {
                        Ok(_) => break,
                        Err(ExecError::WorkspaceBusy) => {
                            std::thread::sleep(std::time::Duration::from_millis(2));
                        }
                        Err(e) => panic!("unexpected refusal: {e:?}"),
                    }
                }
            });
        }
    });
    let text = page_text(&root);
    assert!(text.contains("- from thread 0"), "{text}");
    assert!(text.contains("- from thread 1"), "{text}");
}

#[test]
fn held_lock_is_a_fast_typed_refusal_never_a_wait() {
    // Decision #9 / review C4: a hung holder must never make a caller hang —
    // the second run refuses WorkspaceBusy immediately.
    let (_tmp, root) = workspace();
    let _held = executor::WorkspaceLock::acquire(&root.0).unwrap();
    let now = current_root(&root);
    let start = std::time::Instant::now();
    let err = apply(
        &root,
        &Req {
            effects: &[set_field("status", "x", 0)],
            caps: write_caps(),
            pin: now.clone(),
            live: now,
            receipt: Some(receipt_addr(1)),
            takeover: false,
        },
    )
    .unwrap_err();
    assert_eq!(err, ExecError::WorkspaceBusy);
    assert!(
        start.elapsed() < std::time::Duration::from_secs(1),
        "refusal must be immediate, not a blocked wait"
    );
    assert!(!page_text(&root).contains("status: x"), "nothing applied");
}

#[test]
fn receipt_stamps_the_procedure_hash_and_adopts_the_exec_seam() {
    use run::executor::ReceiptFacts;
    use run::record::{ExecRecord, ExecRecordSink, RunLog};

    let (_tmp, root) = workspace();
    let now = current_root(&root);
    let applied = apply(
        &root,
        &Req {
            effects: &[set_field("status", "done", 0)],
            caps: write_caps(),
            pin: now.clone(),
            live: now,
            receipt: Some(receipt_addr(1)),
            takeover: false,
        },
    )
    .unwrap();

    // The procedure-hash (task_rev) is stamped into the receipt: the line
    // attests WHICH code ran, not just the mutable task name.
    let line = applied.receipt_line.expect("a receipt rode the commit");
    assert!(line.contains("\"task_rev\":\"b3:proc-test\""), "{line}");

    // Round-trip the ReceiptFacts back: the hermetic path carries no exec facts.
    let json = line
        .strip_prefix("- run ")
        .and_then(|b| b.rsplit_once(" ^"))
        .map_or(line.as_str(), |(j, _)| j);
    let mut facts: ReceiptFacts = serde_json::from_str(json).expect("receipt round-trips");
    assert_eq!(facts.task_rev, "b3:proc-test");
    assert!(facts.exec.is_none(), "hermetic run has no child exec facts");

    // The U8 S10 seam binds on the REAL receipt type: a bash owner adopts a
    // sealed record through `fill_exec`, and it serializes back into the line.
    let stdout = RunLog::create(&root.0, "inv-seam")
        .expect("create")
        .seal()
        .expect("seal");
    facts.fill_exec(ExecRecord::new(0, stdout, [] as [(&str, &str); 0]));
    let round = serde_json::to_string(&facts).expect("re-serialize");
    assert!(
        round.contains("\"exec\""),
        "adopted exec serializes: {round}"
    );
    assert!(round.contains("inv-seam"), "{round}");
}

/// U13: exec facts threaded through `ApplyRequest.exec` land in the
/// COMMITTED receipt line — `render_receipt` commits internally, so this is
/// the only path that can put them there (post-hoc adoption cannot). S7
/// rides in by construction: env keys only, the value never touches disk.
#[test]
fn receipt_commits_the_threaded_exec_facts() {
    use run::executor::ReceiptFacts;
    use run::record::{ExecRecord, RunLog};

    let (_tmp, root) = workspace();
    let now = current_root(&root);
    let stdout = RunLog::create(&root.0, "inv-exec")
        .expect("create")
        .seal()
        .expect("seal");
    let exec = ExecRecord::new(0, stdout, [("HOME_WIKI", "secret-value")]);

    let applied = executor::apply(
        &root,
        &ApplyRequest {
            page: "page.md",
            task: "fix-x",
            task_rev: "b3:proc-test",
            invocation_id: "inv-exec",
            now: Some("2026-07-22T03:00:00Z"),
            effects: &[set_field("status", "done", 0)],
            authority: &Authority::granted(write_caps()),
            pin_root: &now,
            live_root: &now,
            receipt: Some(receipt_addr(9)),
            takeover: false,
            exec: Some(&exec),
            depth: 0,
        },
    )
    .unwrap();

    // The COMMITTED file carries the line, and the line carries the facts.
    let line = applied.receipt_line.expect("a receipt rode the commit");
    let receipts = std::fs::read_to_string(root.0.join(&receipt_addr(9).path)).unwrap();
    assert!(
        receipts.contains(&line),
        "line committed to the receipt file"
    );

    let json = line
        .strip_prefix("- run ")
        .and_then(|b| b.rsplit_once(" ^"))
        .map_or(line.as_str(), |(j, _)| j);
    let facts: ReceiptFacts = serde_json::from_str(json).expect("receipt round-trips");
    let exec_back = facts.exec.expect("threaded exec facts committed");
    assert_eq!(exec_back.exit_code, 0);
    assert_eq!(exec_back.stdout.invocation, "inv-exec");
    assert_eq!(exec_back.env_keys, vec!["HOME_WIKI".to_string()]);
    // S7: the env VALUE never reaches the committed receipt.
    assert!(!receipts.contains("secret-value"), "{receipts}");
}

/// **Advisor R25 — the fifth door to the lock ARTIFACT.** The run plane lands
/// bytes through `fs::apply_batch`, bypassing the wire choke-point entirely, and
/// it mints no pin: `md.append_section` composing a `meridian-lock` block is a
/// pin nobody computed. The assertion is on the WRITE (`is_err`), never on the
/// colour the forged claim would render (R26).
#[test]
fn the_run_plane_cannot_write_a_meridian_lock_block() {
    let (_tmp, root) = workspace();
    let now = current_root(&root);
    let before = page_text(&root);
    let forged = apply(
        &root,
        &Req {
            effects: &[append(
                "Log",
                "```meridian-lock\nversion: 1\npins:\n  - ref: \"t.md#T/S\"\n    fingerprint: \"fp1.b3.deadbeef\"\n```",
                0,
            )],
            caps: write_caps(),
            pin: now.clone(),
            live: now,
            receipt: None,
            takeover: false,
        },
    );
    assert!(
        matches!(forged, Err(ExecError::LockArtifact { .. })),
        "the run plane must refuse to write the attestation artifact: {forged:?}"
    );
    assert_eq!(page_text(&root), before, "nothing was applied");
}

/// The control: ordinary run-plane content beside an untouched lock still lands.
/// A guard that refused every apply on a pinned page would be worthless.
#[test]
fn the_run_plane_still_writes_beside_an_untouched_lock() {
    let (tmp, root) = workspace();
    std::fs::write(
        tmp.path().join("page.md"),
        format!(
            "{PAGE}\n```meridian-lock\nversion: 1\npins:\n  - ref: \"t.md#T/S\"\n    fingerprint: \"fp1.b3.deadbeef\"\n```\n"
        ),
    )
    .unwrap();
    let now = current_root(&root);
    apply(
        &root,
        &Req {
            effects: &[append("Log", "- a run-plane line", 0)],
            caps: write_caps(),
            pin: now.clone(),
            live: now,
            receipt: None,
            takeover: false,
        },
    )
    .expect("an ordinary apply on a pinned page still commits");
    let text = page_text(&root);
    assert!(text.contains("- a run-plane line"), "{text}");
    assert!(text.contains("```meridian-lock"), "{text}");
}

// ── the canonical intent → executor adapter (R13 ruling) ─────────────────────

fn intent(action: &str, target: Option<&str>, payload: Option<&str>) -> policy::Intent {
    policy::Intent {
        rule_id: "adapter-control".to_owned(),
        seq: 0,
        action: action.to_owned(),
        target: target.map(str::to_owned),
        severity: None,
        payload: payload.map(str::to_owned),
        receipt: "page.md#^r-000001".to_owned(),
    }
}

fn change_provenance() -> Provenance {
    Provenance::Change {
        fingerprint_before: "before".to_owned(),
        fingerprint_after: "after".to_owned(),
    }
}

fn adapt(intents: &[policy::Intent]) -> Result<executor::IntentApply, executor::AdapterError> {
    executor::IntentApply::from_intents(intents, receipt_addr(9), &change_provenance(), 0)
}

/// The mapping is mechanical: `action` selects the kind, `target` addresses the
/// node, `payload` is what lands there — and the adapted batch is the SAME
/// `ApplyRequest` production executes, receipt included.
#[test]
fn the_adapter_maps_canonical_intents_onto_the_production_batch() {
    let (_tmp, root) = workspace();
    let now = current_root(&root);
    let adapted = adapt(&[
        intent("md.set_field", Some("status"), Some("done")),
        intent("md.append_section", Some("Log"), Some("- adapted line")),
    ])
    .expect("both canonical intents adapt");

    // `target` becomes the addressed node key, `payload` the content key — exactly
    // the descriptor args the planner already reads.
    let shapes: Vec<(EffectKind, BTreeMap<String, ArgValue>)> = adapted
        .effects()
        .iter()
        .map(|effect| (effect.kind, effect.args.clone()))
        .collect();
    assert_eq!(
        shapes,
        vec![
            (EffectKind::SetField, set_field("status", "done", 0).args),
            (
                EffectKind::AppendSection,
                append("Log", "- adapted line", 0).args
            ),
        ],
        "the adapted descriptors are the ones the planner already reads"
    );
    // The emitting rule and the emitting event's plane facts are carried THROUGH:
    // the adapter invents no provenance a `policy::Intent` never recorded.
    for effect in adapted.effects() {
        assert_eq!(effect.rule_id, "adapter-control");
        assert_eq!(effect.provenance, change_provenance());
    }

    let applied = executor::apply(
        &root,
        &adapted.request(&ApplyRequest {
            page: "page.md",
            task: "adapter",
            task_rev: "b3:proc-test",
            invocation_id: "inv-1",
            now: None,
            effects: &[],
            authority: &Authority::granted(write_caps()),
            pin_root: &now,
            live_root: &now,
            receipt: None,
            takeover: false,
            exec: None,
            depth: 0,
        }),
    )
    .expect("the production executor applies the adapted batch");

    assert_eq!(applied.applied, 2, "one generation, one atomic batch");
    assert!(
        applied.receipt_line.is_some(),
        "the receipt address rode the same request"
    );
    let page = page_text(&root);
    assert!(page.contains("status: done"), "{page}");
    assert!(page.contains("- adapted line"), "{page}");
}

/// `proto.*` never passes through the Markdown adapter, so the production
/// `[proto.send]` slice-1 allowlist is untouched by this seam.
#[test]
fn the_adapter_refuses_an_action_outside_the_md_surface() {
    for action in ["notify", "proto.send", "daemon.refresh_view"] {
        let error = adapt(&[intent(action, Some("r"), Some("p"))])
            .expect_err("a non-md action has no Markdown descriptor");
        assert_eq!(
            error,
            executor::AdapterError::NonMdAction {
                rule_id: "adapter-control".to_owned(),
                action: action.to_owned(),
            },
            "{action}"
        );
        assert!(error.to_string().contains(action), "the refusal names it");
    }
}

/// A payload missing a required key, or carrying a key its action does not use, is a
/// LOUD refusal — never a defaulted or partial write. One bad intent refuses the
/// whole generation, because one generation is one batch.
#[test]
fn the_adapter_refuses_an_incomplete_or_over_specified_intent() {
    let missing_target = adapt(&[intent("md.set_field", None, Some("done"))])
        .expect_err("no target means no addressed node");
    assert!(
        matches!(
            missing_target,
            executor::AdapterError::MissingKey { key: "target", .. }
        ),
        "{missing_target:?}"
    );

    let missing_payload = adapt(&[intent("md.append_section", Some("Log"), None)])
        .expect_err("no payload means nothing to land");
    assert!(
        matches!(
            missing_payload,
            executor::AdapterError::MissingKey { key: "payload", .. }
        ),
        "{missing_payload:?}"
    );

    let mut over = intent("md.set_field", Some("status"), Some("done"));
    over.severity = Some("info".to_owned());
    let unused = adapt(&[over]).expect_err("severity is a proto-plane classification");
    assert!(
        matches!(
            unused,
            executor::AdapterError::UnusedKey {
                key: "severity",
                ..
            }
        ),
        "{unused:?}"
    );

    // The control: with the single offending key corrected, the same generation
    // adapts — the refusals above are that one key's doing.
    assert!(
        adapt(&[
            intent("md.set_field", Some("status"), Some("done")),
            intent("md.append_section", Some("Log"), Some("line")),
        ])
        .is_ok(),
        "the corrected baseline adapts"
    );
}
