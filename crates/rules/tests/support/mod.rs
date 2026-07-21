//! Shared test support: committed `.star` rule fixtures + event builders.

// Shared across several integration-test crates; each uses a subset, so some
// items look unused / non-reexported from any single crate's vantage.
#![allow(dead_code, unreachable_pub)]

use rules::{ChangeEvent, Rule};

/// Load a committed rule fixture by file stem (its id = the stem).
pub fn rule(stem: &str) -> Rule {
    let source = match stem {
        "status_binding" => include_str!("../fixtures/rules/status_binding.star"),
        "broadcast_outbox" => include_str!("../fixtures/rules/broadcast_outbox.star"),
        "task_conventions" => include_str!("../fixtures/rules/task_conventions.star"),
        "join_arming" => include_str!("../fixtures/rules/join_arming.star"),
        "status_log" => include_str!("../fixtures/rules/status_log.star"),
        other => panic!("unknown rule fixture: {other}"),
    };
    Rule::new(stem, source)
}

/// Every committed rule fixture, in a stable order.
pub fn all_rules() -> Vec<Rule> {
    [
        "status_binding",
        "broadcast_outbox",
        "task_conventions",
        "join_arming",
        "status_log",
    ]
    .into_iter()
    .map(rule)
    .collect()
}

/// A change event with explicit changed sections/fields.
pub fn event(
    file: &str,
    sections: &[&str],
    fields: &[&str],
    before: &str,
    after: &str,
) -> ChangeEvent {
    ChangeEvent {
        file: file.to_owned(),
        sections_changed: sections.iter().map(|s| (*s).to_owned()).collect(),
        fields_changed: fields.iter().map(|s| (*s).to_owned()).collect(),
        fingerprint_before: before.to_owned(),
        fingerprint_after: after.to_owned(),
        depth: 0,
    }
}
