//! Depth cap / cascade (0003 §5): `md.*` effects are the cascading ones (the
//! engine applies them to the tree, producing a new change). Past the cascade
//! ceiling they are suppressed so a rule chain cannot loop forever; terminal
//! `daemon.*` / `proto.*` effects still emit. Every effect is stamped with its
//! event's depth.

mod support;

use effects::{ChangeEvent, Domain, EvalLimits, EventFacts, eval_with_limits};
use support::rule;

fn limits(max_depth: u32) -> EvalLimits {
    EvalLimits {
        max_depth,
        ..EvalLimits::default()
    }
}

fn status_event(depth: u32) -> ChangeEvent {
    ChangeEvent {
        file: "tasks/t.md".into(),
        sections_changed: vec![],
        fields_changed: vec!["status".into()],
        changes: vec![],
        facts: EventFacts::default(),
        fingerprint_before: "aaa".into(),
        fingerprint_after: "bbb".into(),
        depth,
    }
}

#[test]
fn below_cap_md_effects_emit() {
    // task_conventions (notice+refresh_view) + status_log (append_section = md.*).
    let rules = [rule("task_conventions"), rule("status_log")];
    let effects = eval_with_limits(&rules, &status_event(0), limits(8)).unwrap();
    assert!(
        effects.iter().any(|e| e.kind.domain() == Domain::Md),
        "below the cap, md.* effects are present"
    );
}

#[test]
fn at_cap_md_effects_are_suppressed_others_survive() {
    let rules = [rule("task_conventions"), rule("status_log")];
    // depth == max_depth → at the ceiling.
    let effects = eval_with_limits(&rules, &status_event(8), limits(8)).unwrap();
    assert!(
        effects.iter().all(|e| e.kind.domain() != Domain::Md),
        "at the cap, no md.* effect survives: {effects:?}"
    );
    // The terminal proto.notice + daemon.refresh_view still emit.
    assert!(
        effects.iter().any(|e| e.kind.domain() == Domain::Proto),
        "proto.* effects still emit at the cap"
    );
    assert!(
        effects.iter().any(|e| e.kind.domain() == Domain::Daemon),
        "daemon.* effects still emit at the cap"
    );
}

#[test]
fn above_cap_md_still_suppressed() {
    let rules = [rule("status_log")];
    let effects = eval_with_limits(&rules, &status_event(50), limits(8)).unwrap();
    assert!(
        effects.iter().all(|e| e.kind.domain() != Domain::Md),
        "beyond the cap, md.* stays suppressed"
    );
}

#[test]
fn effects_are_stamped_with_event_depth() {
    let rules = [rule("task_conventions")];
    let effects = eval_with_limits(&rules, &status_event(3), limits(8)).unwrap();
    assert!(!effects.is_empty());
    assert!(
        effects.iter().all(|e| e.depth == 3),
        "every effect carries the event depth"
    );
}

#[test]
fn max_depth_zero_suppresses_md_even_at_depth_zero() {
    // A degenerate cap of 0 means depth 0 already >= cap → md suppressed from the
    // first generation. (Deterministic edge, documents the boundary.)
    let rules = [rule("status_log"), rule("task_conventions")];
    let effects = eval_with_limits(&rules, &status_event(0), limits(0)).unwrap();
    assert!(effects.iter().all(|e| e.kind.domain() != Domain::Md));
    assert!(!effects.is_empty(), "non-md effects still emit");
}
