//! U9 gates: the run report renders a `RunReport` into the closed report-state
//! enum, the md-local caps partition (daemon/proto surfaced unexecuted), the
//! `narrowed[]` facts (#23 gate condition b), the generic cap-reached line, and
//! a `--json` ONE-object whose facts equal the human text's.

use std::collections::BTreeMap;

use model::MerkleRoot;
use run::caps::{Cap, CapResolution, CapSet, CapSource};
use run::dispatch_bash::{BashOutcome, Phase2};
use run::dispatch_starlark::DispatchOutcome;
use run::exec::ExecStatus;
use run::fence::GuaranteeClass;
use run::record::StdoutRecord;
use run::report::{self, ReportState};
use run::runner::{RunReport, TaskOutcome};
use run::shim::ShimStream;
use run::snapshot::Detection;
use rules::{ArgValue, Effect, EffectKind, Provenance};

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
    effect(EffectKind::SetField, &[("field", "status"), ("value", "done")])
}

fn proto() -> Effect {
    effect(EffectKind::Notice, &[("message", "hi")])
}

fn caps(effective: &str, source: CapSource, narrowed: &[&str]) -> CapResolution {
    CapResolution {
        effective: CapSet::parse(effective).unwrap(),
        source,
        narrowed: narrowed.iter().map(|c| Cap::parse(c).unwrap()).collect(),
    }
}

fn starlark(effects: Vec<Effect>, caps: CapResolution, cap_reached: bool) -> RunReport {
    let unexecuted = effects
        .iter()
        .filter(|e| e.kind.domain() != rules::Domain::Md)
        .cloned()
        .collect();
    RunReport {
        task: "fix-x".to_owned(),
        task_rev: "b3:proc-abc".to_owned(),
        guarantee: GuaranteeClass::Hermetic,
        caps,
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
        guarantee: GuaranteeClass::Detected,
        caps: caps("md.set_field", CapSource::Explicit, &[]),
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
        caps("md.set_field", CapSource::Explicit, &[]),
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
        caps("md.set_field", CapSource::Explicit, &[]),
        true,
    ));
    assert_eq!(r.state, ReportState::WithheldDepthCap);
    // Generic line — never a per-effect suppression listing.
    assert!(r.to_text().contains("depth cap reached"));
}

/// #23 gate condition (b): the report renders the caps a ceiling narrowed away.
#[test]
fn narrowed_caps_are_rendered_in_text_and_json() {
    let r = report::render(&starlark(
        vec![md()],
        caps(
            "md.set_field",
            CapSource::Convention("fix-*".to_owned()),
            &["md.append_section"],
        ),
        false,
    ));
    assert_eq!(r.caps.narrowed, vec!["md.append_section".to_owned()]);
    assert_eq!(r.caps.source, "convention:fix-*");
    assert!(r.to_text().contains("narrowed by ceiling: md.append_section"));

    // --json is ONE object carrying the same narrowed facts.
    let json = r.to_json().unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(value.is_object(), "one object: {json}");
    assert_eq!(value["caps"]["narrowed"][0], "md.append_section");
    assert_eq!(value["state"], "applied");
    assert_eq!(value["task_rev"], "b3:proc-abc");
}

#[test]
fn bash_phase1_committed_phase2_refused_is_partial() {
    let r = report::render(&bash(
        Phase2::RefusedExecFailed,
        ExecStatus::Exited { code: 7 },
        Some("- run {} ^p-000001"),
    ));
    assert_eq!(r.state, ReportState::Partial);
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
