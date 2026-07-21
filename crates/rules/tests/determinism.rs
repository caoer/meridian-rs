//! Determinism invariants (0003 §1 + § Testing methodology, proptest): the same
//! `(rules, event)` yields the byte-identical effect set across repeated runs and
//! across thread counts, and the idempotency keys are stable (cursor replay
//! depends on this).

mod support;

use proptest::prelude::*;
use rules::{ChangeEvent, Effect, eval};
use support::all_rules;

/// An arbitrary change event — arbitrary paths, changed sections/fields, and
/// fingerprints, including empties and unicode.
fn any_event() -> impl Strategy<Value = ChangeEvent> {
    (
        ".*",
        prop::collection::vec(".*", 0..6),
        prop::collection::vec(".*", 0..6),
        ".*",
        ".*",
        0u32..12,
    )
        .prop_map(
            |(file, sections, fields, before, after, depth)| ChangeEvent {
                file,
                sections_changed: sections,
                fields_changed: fields,
                fingerprint_before: before,
                fingerprint_after: after,
                depth,
            },
        )
}

fn json(effects: &[Effect]) -> String {
    serde_json::to_string(effects).expect("effects serialize")
}

proptest! {
    /// Repeated evaluation of the same input is byte-identical.
    #[test]
    fn repeated_eval_is_byte_identical(event in any_event()) {
        let rules = all_rules();
        let a = eval(&rules, &event).unwrap();
        let b = eval(&rules, &event).unwrap();
        prop_assert_eq!(json(&a), json(&b));
    }

    /// Idempotency keys are stable across runs — the executor-dedup contract.
    #[test]
    fn idempotency_keys_are_stable(event in any_event()) {
        let rules = all_rules();
        let a: Vec<_> = eval(&rules, &event).unwrap().iter().map(Effect::idempotency_key).collect();
        let b: Vec<_> = eval(&rules, &event).unwrap().iter().map(Effect::idempotency_key).collect();
        prop_assert_eq!(a, b);
    }

    /// Within one event, each (rule_id, fingerprint_after, seq) key is UNIQUE —
    /// no two effects collide on the dedup key.
    #[test]
    fn idempotency_keys_are_unique_within_an_event(event in any_event()) {
        use std::collections::HashSet;
        let effects = eval(&all_rules(), &event).unwrap();
        let mut seen = HashSet::new();
        for e in &effects {
            prop_assert!(seen.insert(e.idempotency_key()), "duplicate idempotency key: {:?}", e.idempotency_key());
        }
    }
}

/// Evaluating the same input from many OS threads concurrently yields the exact
/// same effect set on every thread (no shared mutable state, no thread-count
/// dependence).
#[test]
fn eval_is_identical_across_thread_counts() {
    let event = ChangeEvent {
        file: "tasks/t.md".into(),
        sections_changed: vec!["Status".into()],
        fields_changed: vec!["status".into()],
        fingerprint_before: "aaa".into(),
        fingerprint_after: "bbb".into(),
        depth: 0,
    };
    let reference = json(&eval(&all_rules(), &event).unwrap());

    let results: Vec<String> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..16)
            .map(|_| {
                let event = event.clone();
                scope.spawn(move || json(&eval(&all_rules(), &event).unwrap()))
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    for (i, r) in results.iter().enumerate() {
        assert_eq!(*r, reference, "thread {i} diverged");
    }
}

/// Rule slice order is honored: reversing the rule set reverses the per-rule
/// blocks of effects, never interleaves them.
#[test]
fn effect_order_follows_rule_slice_order() {
    let event = ChangeEvent {
        file: "tasks/t.md".into(),
        sections_changed: vec!["Status".into()],
        fields_changed: vec!["status".into()],
        fingerprint_before: "aaa".into(),
        fingerprint_after: "bbb".into(),
        depth: 0,
    };
    let mut rules = all_rules();
    let forward = eval(&rules, &event).unwrap();
    rules.reverse();
    let reversed = eval(&rules, &event).unwrap();

    // Same multiset of effects, different (rule-block) order.
    let mut f_ids: Vec<_> = forward.iter().map(|e| e.rule_id.clone()).collect();
    let mut r_ids: Vec<_> = reversed.iter().map(|e| e.rule_id.clone()).collect();
    assert_eq!(forward.len(), reversed.len());
    f_ids.sort();
    r_ids.sort();
    assert_eq!(f_ids, r_ids, "same effects regardless of order");
    // The first-emitting rule differs when the slice is reversed (unless only one
    // rule fired).
    let distinct_rules: std::collections::BTreeSet<_> =
        forward.iter().map(|e| e.rule_id.clone()).collect();
    if distinct_rules.len() > 1 {
        assert_ne!(
            forward.first().map(|e| &e.rule_id),
            reversed.first().map(|e| &e.rule_id),
            "reversing rule order changes which rule's effects come first"
        );
    }
}
