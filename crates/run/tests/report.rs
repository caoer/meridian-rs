//! U9 gates: the run report renders a `RunReport` into the closed report-state
//! enum, the md-local caps partition (daemon/proto surfaced unexecuted), the
//! `narrowed[]` facts (#23 gate condition b), the generic cap-reached line, and
//! a `--json` ONE-object whose facts equal the human text's.

use std::collections::BTreeMap;

use effects::{ArgValue, Effect, EffectKind, Provenance};
use model::MerkleRoot;
use run::caps::{Authority, Cap, CapResolution, CapSet, CapSource};
use run::dispatch_bash::{BashOutcome, Phase2};
use run::dispatch_starlark::DispatchOutcome;
use run::exec::ExecStatus;
use run::fence::GuaranteeClass;
use run::record::StdoutRecord;
use run::report::{self, ReportState};
use run::runner::{RunReport, TaskOutcome};
use run::shim::ShimStream;
use run::snapshot::Detection;

fn effect(kind: EffectKind, args: &[(&str, &str)]) -> Effect {
    Effect {
        kind,
        rule_id: "t".to_owned(),
        seq: 0,
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

fn md() -> Effect {
    effect(
        EffectKind::SetField,
        &[("field", "status"), ("value", "done")],
    )
}

fn proto() -> Effect {
    effect(EffectKind::Notice, &[("message", "hi")])
}

fn caps(effective: &str, source: CapSource, narrowed: &[&str]) -> CapResolution {
    CapResolution {
        effective: CapSet::parse(effective).unwrap(),
        source,
        narrowed: narrowed.iter().map(|c| Cap::parse(c).unwrap()).collect(),
        ceilings: Vec::new(),
    }
}

fn starlark(effects: Vec<Effect>, caps: CapResolution, cap_reached: bool) -> RunReport {
    let unexecuted = effects
        .iter()
        .filter(|e| e.kind.domain() != effects::Domain::Md)
        .cloned()
        .collect();
    RunReport {
        task: "fix-x".to_owned(),
        task_rev: "b3:proc-abc".to_owned(),
        guarantee: GuaranteeClass::Hermetic,
        authority: Authority::Capabilities(caps),
        outcome: TaskOutcome::Starlark(Box::new(DispatchOutcome {
            guarantee: GuaranteeClass::Hermetic,
            effects,
            applied: None,
            unexecuted,
        })),
        cascade: Vec::new(),
        cap_reached,
    }
}

fn bash(phase2: Phase2, status: ExecStatus, pre_receipt: Option<&str>) -> RunReport {
    RunReport {
        task: "fix-x".to_owned(),
        task_rev: "b3:proc-abc".to_owned(),
        guarantee: GuaranteeClass::Unsandboxed,
        // Bash claims no capability at all (docs/laws.md § Amendment).
        authority: Authority::Unsandboxed,
        outcome: TaskOutcome::Bash(Box::new(BashOutcome {
            pre_receipt_line: pre_receipt.map(str::to_owned),
            status,
            stdout: Ok(StdoutRecord {
                invocation: "inv-1".to_owned(),
                log: ".meridian/runs/inv-1.log".to_owned(),
                sha256: "0".repeat(64),
                bytes: 0,
            }),
            stderr: Vec::new(),
            shim: ShimStream {
                bytes: Vec::new(),
                overflowed: false,
            },
            pre_exec: None,
            // A clean window: these failures are exec-fail/timeout, not
            // detection — the delta line reads "none".
            detection: Detection::Clean {
                root: MerkleRoot("b3:clean".to_owned()),
            },
            phase2,
        })),
        cascade: Vec::new(),
        cap_reached: false,
    }
}

#[test]
fn applied_md_and_surfaced_proto_are_partitioned() {
    let r = report::render(&starlark(
        vec![md(), proto()],
        caps("md.edit", CapSource::Explicit, &[]),
        false,
    ));
    assert_eq!(r.state, ReportState::Applied);
    assert_eq!(r.applied.len(), 1);
    assert_eq!(r.applied[0].kind, "md.set_field");
    assert_eq!(r.applied[0].domain, "md");
    // daemon.*/proto.* surfaced, never dropped or faked.
    assert_eq!(r.unexecuted.len(), 1);
    assert_eq!(r.unexecuted[0].domain, "proto");
    // guarantee + procedure-hash carried through.
    assert_eq!(r.guarantee, "hermetic");
    assert_eq!(r.task_rev, "b3:proc-abc");
    assert!(r.out_of_band_delta.contains("hermetic"));
}

#[test]
fn a_run_with_only_non_md_effects_is_unexecuted_no_capability() {
    let r = report::render(&starlark(
        vec![proto()],
        caps("", CapSource::DenyDefault, &[]),
        false,
    ));
    assert_eq!(r.state, ReportState::UnexecutedNoCapability);
    assert!(r.applied.is_empty());
    assert_eq!(r.unexecuted.len(), 1);
}

#[test]
fn the_depth_cap_is_a_generic_withheld_state() {
    let r = report::render(&starlark(
        vec![md()],
        caps("md.edit", CapSource::Explicit, &[]),
        true,
    ));
    assert_eq!(r.state, ReportState::WithheldDepthCap);
    // Generic line — never a per-effect suppression listing, and never
    // mechanism narration ("by the kernel"): the fact is cap + not-applied
    // (report-voice audit, 2026-08-15).
    assert!(
        r.to_text()
            .contains("cascade: depth cap reached — capped md.* not applied")
    );
}

/// ZT ruling, 2026-08-15: there is no sandbox, so no human line says
/// `unsandboxed` — the `task:` line renders bare for bash and `effects:
/// undeclared` carries the fact. A POSITIVE guarantee (`hermetic`) still
/// renders, and `--json` keeps the `guarantee` class on both.
#[test]
fn a_guarantee_word_renders_only_where_positive() {
    let b = report::render(&bash(
        Phase2::Applied {
            effects: vec![],
            applied: None,
        },
        ExecStatus::Exited { code: 0 },
        Some("^p-1"),
    ));
    let text = b.to_text();
    assert!(text.contains("task: fix-x\n"), "{text}");
    assert!(text.contains("effects: undeclared\n"), "{text}");
    assert!(!text.contains("unsandboxed"), "{text}");
    assert_eq!(b.guarantee, "unsandboxed", "--json keeps the class");

    let s = report::render(&starlark(
        vec![md()],
        caps("md.edit", CapSource::Explicit, &[]),
        false,
    ));
    assert!(s.to_text().contains("task: fix-x (hermetic)"), "positive");
}

/// #23 gate condition (b): the report renders the caps a ceiling narrowed away.
#[test]
fn narrowed_caps_are_rendered_in_text_and_json() {
    let r = report::render(&starlark(
        vec![md()],
        caps(
            "md.edit",
            CapSource::Convention("fix-*".to_owned()),
            &["md.create"],
        ),
        false,
    ));
    let caps = r.caps.as_ref().expect("starlark renders its caps");
    assert_eq!(caps.narrowed, vec!["md.create".to_owned()]);
    assert_eq!(caps.source, "convention:fix-*");
    assert!(r.to_text().contains("narrowed by ceiling: md.create"));

    // --json is ONE object carrying the same narrowed facts.
    let json = r.to_json().unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(value.is_object(), "one object: {json}");
    assert_eq!(value["caps"]["narrowed"][0], "md.create");
    assert_eq!(value["state"], "applied");
    assert_eq!(value["task_rev"], "b3:proc-abc");
}

#[test]
fn bash_phase1_committed_phase2_refused_is_partial() {
    let r = report::render(&bash(
        Phase2::RefusedExecFailed {
            applied: completion_applied(),
        },
        ExecStatus::Exited { code: 7 },
        Some("- run {} ^p-000001"),
    ));
    assert_eq!(r.state, ReportState::Partial);
}

/// G3b: a signaled step did NOT complete, so it is interrupted — never the
/// `partial` a recorded nonzero exit renders.
#[test]
fn a_signaled_step_is_interrupted_not_partial() {
    let r = report::render(&bash(
        Phase2::RefusedSignaled,
        ExecStatus::Signaled { signal: 9 },
        Some("- run {} ^p-000001"),
    ));
    assert_eq!(r.state, ReportState::Interrupted);
}

/// The completion receipt an exited-nonzero run commits: an EMPTY batch under a
/// real receipt line.
fn completion_applied() -> run::executor::Applied {
    run::executor::Applied {
        applied: 0,
        event: None,
        receipt_line: Some("- run {} ^r-000001".to_owned()),
        file_rev_after: "b3:after".to_owned(),
    }
}

#[test]
fn a_bash_timeout_is_interrupted_not_partial() {
    let r = report::render(&bash(
        Phase2::RefusedTimeout,
        ExecStatus::Exited { code: 0 },
        Some("- run {} ^p-000001"),
    ));
    assert_eq!(r.state, ReportState::Interrupted);
}

/// Card run-preexec-severity: the pre-exec fact rides the report on both
/// faces. Bash clean = an explicit "none" (absence keeps one meaning);
/// divergent = the register-law line; starlark = no line at all (no exec
/// window, no pre-exec gap).
#[test]
fn pre_exec_delta_rides_both_report_faces() {
    // clean bash → explicit none
    let clean = report::render(&bash(
        Phase2::Applied {
            effects: vec![],
            applied: None,
        },
        ExecStatus::Exited { code: 0 },
        Some("^p-1"),
    ));
    assert!(clean.to_text().contains("pre-exec delta: none"));
    assert!(
        clean
            .to_json()
            .unwrap()
            .contains("\"pre_exec_delta\":\"none\"")
    );

    // divergent bash → the reason-first line, run still Applied
    let mut divergent_report = bash(
        Phase2::Applied {
            effects: vec![],
            applied: None,
        },
        ExecStatus::Exited { code: 0 },
        Some("^p-1"),
    );
    if let TaskOutcome::Bash(o) = &mut divergent_report.outcome {
        o.pre_exec = Some(run::snapshot::PreExecDivergence {
            expected: MerkleRoot("b3:aaaa".to_owned()),
            observed: MerkleRoot("b3:bbbb".to_owned()),
        });
    }
    let divergent = report::render(&divergent_report);
    assert_eq!(divergent.state, ReportState::Applied, "report, do not gate");
    let text = divergent.to_text();
    assert!(
        text.contains("pre-exec delta: out-of-band change before exec window"),
        "{text}"
    );
    assert!(
        text.contains("b3:aaaa") && text.contains("b3:bbbb"),
        "{text}"
    );
    // the window verdict stays its own line, unpolluted
    assert!(text.contains("out-of-band delta: none"), "{text}");

    // starlark → no pre-exec line on either face
    let s = report::render(&starlark(
        vec![],
        caps("md.edit", CapSource::Explicit, &[]),
        false,
    ));
    assert!(!s.to_text().contains("pre-exec delta"));
    assert!(!s.to_json().unwrap().contains("pre_exec_delta"));
}
