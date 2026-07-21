//! Capability routing (0003 §2): the engine emits ALL effects deterministically;
//! a `CapabilitySet` is the downstream filter. Routing partitions by declared
//! kind and never changes which effects were produced.

mod support;

use rules::{CapabilitySet, Domain, EffectKind, eval};
use support::{all_rules, event};

fn mixed_effects() -> Vec<rules::Effect> {
    // A status-field change on a task file fires task_conventions (notice +
    // refresh_view) and status_log (append_section) — proto, daemon, and md kinds.
    eval(
        &all_rules(),
        &event("tasks/t.md", &[], &["status"], "aaa", "bbb"),
    )
    .unwrap()
}

#[test]
fn full_capability_admits_everything() {
    let effects = mixed_effects();
    let (admitted, rejected) = CapabilitySet::all().route(effects.clone());
    assert_eq!(admitted.len(), effects.len());
    assert!(rejected.is_empty());
}

#[test]
fn empty_capability_rejects_everything() {
    let effects = mixed_effects();
    let (admitted, rejected) = CapabilitySet::none().route(effects.clone());
    assert!(admitted.is_empty());
    assert_eq!(rejected.len(), effects.len());
}

#[test]
fn domain_capability_routes_by_domain() {
    let effects = mixed_effects();
    let n_md = effects
        .iter()
        .filter(|e| e.kind.domain() == Domain::Md)
        .count();
    assert!(n_md > 0, "fixture should include an md.* effect");

    // A wire-only consumer (proto.*) admits proto, rejects md + daemon.
    let proto_only = CapabilitySet::none().with_domain(Domain::Proto);
    let (admitted, rejected) = proto_only.route(effects.clone());
    assert!(admitted.iter().all(|e| e.kind.domain() == Domain::Proto));
    assert!(rejected.iter().all(|e| e.kind.domain() != Domain::Proto));
    assert_eq!(admitted.len() + rejected.len(), effects.len());
}

#[test]
fn single_kind_capability_admits_only_that_kind() {
    let effects = mixed_effects();
    let caps = CapabilitySet::none().with(EffectKind::RefreshView);
    let (admitted, _) = caps.route(effects);
    assert!(admitted.iter().all(|e| e.kind == EffectKind::RefreshView));
    assert_eq!(admitted.len(), 1, "one refresh_view in the fixture");
}

#[test]
fn routing_preserves_order_within_partitions() {
    let effects = mixed_effects();
    let seqs_before: Vec<(String, u32)> = effects
        .iter()
        .filter(|e| e.kind.domain() == Domain::Proto)
        .map(|e| (e.rule_id.clone(), e.seq))
        .collect();
    let (admitted, _) = CapabilitySet::none()
        .with_domain(Domain::Proto)
        .route(effects);
    let seqs_after: Vec<(String, u32)> = admitted
        .iter()
        .map(|e| (e.rule_id.clone(), e.seq))
        .collect();
    assert_eq!(seqs_before, seqs_after, "order preserved through routing");
}

#[test]
fn from_iter_builds_a_set() {
    let caps: CapabilitySet = [EffectKind::Notice, EffectKind::RefreshView]
        .into_iter()
        .collect();
    let effects = mixed_effects();
    let (admitted, rejected) = caps.route(effects);
    // The fixture has exactly one notice + one refresh_view → both admitted.
    assert_eq!(admitted.len(), 2, "collected kinds must actually admit");
    assert!(!rejected.is_empty(), "the append_section is rejected");
    assert!(
        admitted
            .iter()
            .all(|e| matches!(e.kind, EffectKind::Notice | EffectKind::RefreshView))
    );
}
