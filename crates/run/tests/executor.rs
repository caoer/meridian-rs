//! Executor gates (U4): choke-point cap validation, one atomic unguarded
//! batch (no world pin — no-guard-on-effects ruling, 2026-08-15), self-guards
//! + flock, receipts, apply→event synthesis.

use std::collections::BTreeMap;

use effects::{ArgValue, Effect, EffectKind, Provenance};
use model::MerkleRoot;
use run::caps::{Authority, CapSet};

/// Empty run-birth fields for these fixtures.
static TEST_EMPTY_FIELDS: BTreeMap<String, String> = BTreeMap::new();
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
    observed: MerkleRoot,
    receipt: Option<ReceiptAddr>,
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
            observed_root: &r.observed,
            receipt: r.receipt.clone(),
            exec: None,
            actor: None,
            depth: 0,
            delta: None,
            fields: &TEST_EMPTY_FIELDS,
            birth_seq: None,
            ambient: None,
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
            observed: now,
            receipt: Some(receipt_addr(1)),
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
            observed: now,
            receipt: Some(receipt_addr(1)),
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
fn joint_field_and_section_batch_synthesizes_event_with_both_names() {
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
            observed: now,
            receipt: Some(receipt_addr(1)),
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
    // RATIFIED (sub-node-grain ruling, 2026-08-14): region-grain node deltas
    // give the joint batch BOTH its identities.
    assert_eq!(event.fields_changed, vec!["status".to_owned()], "{event:?}");
    assert_eq!(
        event.sections_changed,
        vec!["Tasks#Log".to_owned()],
        "{event:?}"
    );
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
            observed: now,
            receipt: Some(receipt_addr(1)),
        },
    )
    .unwrap_err();
    assert_eq!(
        err,
        ExecError::CapDenied {
            kind: "md.set_field".to_owned(),
            target: "status".to_owned(),
            // No ceiling narrowed here: the grant simply never held the cap —
            // the refusal names the measured grants (none) and the
            // `task.<name>.caps` declaration (r3 gap 6b).
            ceiling: None,
            declared: Vec::new(),
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
            observed: now,
            receipt: None,
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
            observed: now,
            receipt: None,
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
            observed: now,
            receipt: None,
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
            observed: now,
            receipt: None,
        },
    )
    .unwrap_err();
    assert!(matches!(err, ExecError::NonMdEffect { .. }));
}

/// The no-guard law at this seam (2026-08-15 ruling): a stale observation —
/// the corpus advanced since the effects were produced — commits anyway.
/// The former `root_mismatch` premise refusal is RETIRED; the full F3
/// fixture lives in `tests/no_guard_on_effects.rs`.
#[test]
fn a_stale_observation_commits_nothing_refuses() {
    let (_tmp, root) = workspace();
    apply(
        &root,
        &Req {
            effects: &[set_field("status", "done", 0)],
            caps: write_caps(),
            observed: MerkleRoot("b3:stale".to_owned()),
            receipt: Some(receipt_addr(1)),
        },
    )
    .expect("no world pin exists on this door");
    assert!(page_text(&root).contains("status: done"));
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
            observed: now.clone(),
            receipt: None,
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
            observed: now,
            receipt: None,
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

/// Two consecutive governed writes on the same target stay clean — the
/// receipt history is attested record, compared by nothing (the former
/// decision-#26 foreign-edit scan is retired; its positive case lives in
/// `tests/no_guard_on_effects.rs`).
#[test]
fn a_re_run_over_its_own_receipt_history_is_clean() {
    let (_tmp, root) = workspace();
    let now = current_root(&root);
    apply(
        &root,
        &Req {
            effects: &[set_field("status", "done", 0)],
            caps: write_caps(),
            observed: now,
            receipt: Some(receipt_addr(1)),
        },
    )
    .unwrap();
    let now = current_root(&root);
    apply(
        &root,
        &Req {
            effects: &[set_field("status", "review", 0)],
            caps: write_caps(),
            observed: now,
            receipt: Some(receipt_addr(2)),
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
                            observed: now,
                            receipt: Some(receipt_addr(u64::from(i) + 1)),
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
            observed: now,
            receipt: Some(receipt_addr(1)),
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
            observed: now,
            receipt: Some(receipt_addr(1)),
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
            observed_root: &now,
            receipt: Some(receipt_addr(9)),
            exec: Some(&exec),
            actor: None,
            depth: 0,
            delta: None,
            fields: &TEST_EMPTY_FIELDS,
            birth_seq: None,
            ambient: None,
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

/// **R25 — the fifth door to the lock ARTIFACT.** The run plane lands
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
                "```meridian-lock\nversion: 2\npins:\n  - object: \"[[t]]\"\n    \
                 hash: \"9ae3f1deadbeef\"\n    path: [\"T\", \"S\"]\n    \
                 fingerprint: \"fp1.span2.b3.a8222f5a\"\n```",
                0,
            )],
            caps: write_caps(),
            observed: now,
            receipt: None,
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
            "{PAGE}\n```meridian-lock\nversion: 2\npins:\n  - object: \"[[t]]\"\n    \
             hash: \"9ae3f1deadbeef\"\n    path: [\"T\", \"S\"]\n    \
             fingerprint: \"fp1.span2.b3.a8222f5a\"\n```\n"
        ),
    )
    .unwrap();
    let now = current_root(&root);
    apply(
        &root,
        &Req {
            effects: &[append("Log", "- a run-plane line", 0)],
            caps: write_caps(),
            observed: now,
            receipt: None,
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
            observed_root: &now,
            receipt: None,
            exec: None,
            actor: None,
            depth: 0,
            delta: None,
            fields: &TEST_EMPTY_FIELDS,
            birth_seq: None,
            ambient: None,
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

/// r3 gap 6b (card `refusal-teaching-gaps-r3`): a deny-by-default refusal
/// names WHY (only declared caps are granted), the declared grants the
/// resolution measured, and `task.<name>.caps` as the declaration that
/// grants — never a bare denial the caller must source-dive to repair.
#[test]
fn cap_denial_without_a_ceiling_teaches_the_caps_declaration() {
    let (_tmp, root) = workspace();
    let now = current_root(&root);
    let err = apply(
        &root,
        &Req {
            effects: &[append("Log", "- x", 0)],
            caps: CapSet::parse("md.set_field:status").unwrap(),
            observed: now,
            receipt: None,
        },
    )
    .unwrap_err();
    let m = err.to_string();
    assert!(
        m.contains("only declared capabilities are granted"),
        "names the deny-by-default reason: {m}"
    );
    assert!(
        m.contains("md.set_field:status"),
        "lists the grants the resolution measured: {m}"
    );
    assert!(
        m.contains("task.<name>.caps"),
        "names the declaration that grants: {m}"
    );
}

/// Gap 6b control — a task declaring NO caps says so, instead of listing an
/// empty set.
#[test]
fn cap_denial_on_an_empty_grant_says_the_task_declares_none() {
    let (_tmp, root) = workspace();
    let now = current_root(&root);
    let err = apply(
        &root,
        &Req {
            effects: &[append("Log", "- x", 0)],
            caps: CapSet::none(),
            observed: now,
            receipt: None,
        },
    )
    .unwrap_err();
    let m = err.to_string();
    assert!(
        m.contains("declares no md.* capabilities"),
        "states the empty grant plainly: {m}"
    );
    assert!(
        m.contains("task.<name>.caps"),
        "names the declaration that grants: {m}"
    );
}

/// ⑤-F2 regression (2026-08-17). A value-identical `md.set_field` keeps the
/// STORED spelling — quotes included — so the composed line's bytes, and with
/// them the field hash / `prop_rev` / corpus root, do not move.
///
/// The defect this pins closed: the run plane re-composed `{key}: {value}`
/// from the RAW caller string, so a stored `manifest: "a: b"` re-written with
/// its own parsed value landed as `manifest: a: b` — the parsed value
/// round-tripped while every value-identical write moved the line's bytes
/// (quote drop), defeating idempotence-on-hash and littering git with no-op
/// diffs. The fix routes the value through the ONE § A.6.3c encoder
/// (`yaml_preserve_or_encode`) the wire's upsert door already uses.
#[test]
fn value_identical_set_field_keeps_stored_quoting_and_moves_no_hash() {
    let (_tmp, root) = workspace();
    let quoted_page = "\
---
manifest: \"worker — cas client: lock rewrite\"
---

# Tasks

body
";
    std::fs::write(root.0.join("page.md"), quoted_page).unwrap();

    // The caller writes the PARSED value — exactly what a read served it.
    let before_root = current_root(&root);
    let applied = apply(
        &root,
        &Req {
            effects: &[set_field(
                "manifest",
                "worker — cas client: lock rewrite",
                0,
            )],
            caps: write_caps(),
            observed: before_root.clone(),
            receipt: None,
        },
    )
    .unwrap();

    assert_eq!(
        page_text(&root),
        quoted_page,
        "a value-identical set_field moved the page's bytes — the stored quoting was not preserved"
    );
    assert_eq!(
        current_root(&root),
        before_root,
        "identical bytes must leave the corpus root unmoved (Merkle law)"
    );
    if let Some(event) = &applied.event {
        assert_eq!(
            event.fingerprint_before, event.fingerprint_after,
            "a byte-identical write synthesized a moved fingerprint"
        );
    }
}

/// The verbatim half of ⑤-F2's boundary: a FRESH value still lands AS SENT —
/// this plane is raw-grain by contract (`s2fix_run_plane_fp` pins the raw
/// edges), so the fix is preservation-only and fresh values are untouched.
/// The follow-up value-identical write of the same string moves nothing:
/// either the preservation arm fires, or the verbatim arm reproduces the
/// same bytes — idempotence-on-bytes holds on both paths.
#[test]
fn fresh_set_field_lands_verbatim_and_is_then_idempotent() {
    let (_tmp, root) = workspace();

    let value = "worker — auditing outbox: effect migration";
    apply(
        &root,
        &Req {
            effects: &[set_field("manifest", value, 0)],
            caps: write_caps(),
            observed: current_root(&root),
            receipt: None,
        },
    )
    .unwrap();
    let first = page_text(&root);
    assert!(
        first.contains(&format!("manifest: {value}")),
        "a fresh run-plane value lands AS SENT (raw-grain contract):\n{first}"
    );

    let root_after_first = current_root(&root);
    apply(
        &root,
        &Req {
            effects: &[set_field("manifest", value, 1)],
            caps: write_caps(),
            observed: root_after_first.clone(),
            receipt: None,
        },
    )
    .unwrap();
    assert_eq!(
        page_text(&root),
        first,
        "the value-identical re-write moved the line's bytes"
    );
    assert_eq!(current_root(&root), root_after_first);
}
