//! Executor gates (U4): choke-point cap validation, one atomic `if_root`
//! batch, self-guards + flock, receipts, apply→event synthesis, and the
//! decision-#26 foreign-edit law.

use std::collections::BTreeMap;

use model::MerkleRoot;
use run::caps::CapSet;
use run::executor::{self, ApplyRequest, ExecError, ReceiptAddr};
use rules::{ArgValue, Effect, EffectKind, Provenance};

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
    effect(EffectKind::SetField, &[("field", field), ("value", value)], seq)
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
            invocation_id: "inv-1",
            now: Some("2026-07-22T01:00:00Z"),
            effects: r.effects,
            caps: &r.caps,
            pin_root: &r.pin,
            live_root: &r.live,
            receipt: r.receipt.clone(),
            takeover: r.takeover,
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
    assert!(page_text(&root).contains("status: human-truth"), "preserved");

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
