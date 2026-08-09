//! The load gate: `validate` parse-checks rules once, up front (no event), so a
//! caller separates authoring faults from per-event faults. Mirrors policy's
//! compile-time predicate check.

mod support;

use effects::{EvalError, EvalLimits, Rule, validate};
use support::all_rules;

#[test]
fn all_committed_fixtures_validate() {
    validate(&all_rules(), EvalLimits::default()).expect("shipped rule fixtures validate");
}

#[test]
fn valid_rule_passes_without_an_event() {
    let rules = [Rule::new(
        "ok",
        "def on_change(event):\n    notice(message = \"hi\")\n",
    )];
    assert!(validate(&rules, EvalLimits::default()).is_ok());
}

#[test]
fn malformed_rule_fails_validation() {
    let rules = [Rule::new("bad", "def on_change(event:\n")];
    assert!(matches!(
        validate(&rules, EvalLimits::default()),
        Err(EvalError::Parse { rule_id, .. }) if rule_id == "bad"
    ));
}

#[test]
fn oversized_rule_fails_validation() {
    let big = format!("def on_change(event):\n    pass\n# {}", "x".repeat(1000));
    let rules = [Rule::new("big", big)];
    let limits = EvalLimits {
        max_source_bytes: 100,
        ..EvalLimits::default()
    };
    assert!(matches!(
        validate(&rules, limits),
        Err(EvalError::SourceTooLarge { rule_id, .. }) if rule_id == "big"
    ));
}

#[test]
fn deeply_nested_rule_fails_validation() {
    let src = format!("def on_change(event):\n    x = {}1\n", "-".repeat(2000));
    let rules = [Rule::new("deep", src)];
    assert!(matches!(
        validate(&rules, EvalLimits::default()),
        Err(EvalError::Parse { .. })
    ));
}

#[test]
fn first_bad_rule_in_a_batch_is_named() {
    let rules = [
        Rule::new("good", "def on_change(event):\n    pass\n"),
        Rule::new("bad", "def on_change(event:\n"),
    ];
    assert!(matches!(
        validate(&rules, EvalLimits::default()),
        Err(EvalError::Parse { rule_id, .. }) if rule_id == "bad"
    ));
}
