//! Golden fixtures (0003 § Testing methodology — insta snapshots): a committed
//! rule + a change event → the EXACT effect descriptor set. The `.snap` files
//! are the frozen contract; any change to the emitted descriptors is a visible
//! diff a reviewer must accept.

mod support;

use effects::{EffectKind, eval};
use support::{all_rules, event, rule};

// status_binding fires when the Status section moved but the frontmatter status
// field did not — one advisory warn.
#[test]
fn golden_status_binding_drift_warns() {
    let effects = eval(
        &[rule("status_binding")],
        &event("tasks/t.md", &["Status"], &[], "aaa", "bbb"),
    )
    .unwrap();
    insta::assert_json_snapshot!("status_binding_drift", effects);
}

// status_binding stays silent when the field moved too (no drift).
#[test]
fn golden_status_binding_in_sync_is_silent() {
    let effects = eval(
        &[rule("status_binding")],
        &event("tasks/t.md", &["Status"], &["status"], "aaa", "bbb"),
    )
    .unwrap();
    assert!(
        effects.is_empty(),
        "in-sync change teaches nothing: {effects:?}"
    );
}

// broadcast_outbox sends a fleet re-read anchored to the post-change fingerprint.
#[test]
fn golden_broadcast_outbox_sends() {
    let effects = eval(
        &[rule("broadcast_outbox")],
        &event("BROADCAST.md", &["body"], &[], "v1", "v2"),
    )
    .unwrap();
    insta::assert_json_snapshot!("broadcast_outbox_send", effects);
}

// task_conventions emits notice then refresh_view — two effects, per-rule seq
// 0 then 1, two domains (proto then daemon).
#[test]
fn golden_task_conventions_two_effects_seq() {
    let effects = eval(
        &[rule("task_conventions")],
        &event("tasks/t.md", &[], &["status"], "aaa", "bbb"),
    )
    .unwrap();
    assert_eq!(effects.len(), 2);
    assert_eq!(effects[0].seq, 0);
    assert_eq!(effects[1].seq, 1);
    assert_eq!(effects[0].kind, EffectKind::Notice);
    assert_eq!(effects[1].kind, EffectKind::RefreshView);
    insta::assert_json_snapshot!("task_conventions_notice_refresh", effects);
}

// join_arming fires only on a creation (empty prior fingerprint) under agents/.
#[test]
fn golden_join_arming_on_create() {
    let effects = eval(
        &[rule("join_arming")],
        &event("agents/abcd1234.md", &[], &[], "", "born"),
    )
    .unwrap();
    insta::assert_json_snapshot!("join_arming_create", effects);
}

// join_arming does NOT fire on an update to an existing agent file.
#[test]
fn golden_join_arming_skips_update() {
    let effects = eval(
        &[rule("join_arming")],
        &event("agents/abcd1234.md", &["Notes"], &[], "old", "new"),
    )
    .unwrap();
    assert!(
        effects.is_empty(),
        "existing-file update does not re-arm: {effects:?}"
    );
}

// status_log writes a durable log line INTO the tree (the cursor-replay escape).
#[test]
fn golden_status_log_appends() {
    let effects = eval(
        &[rule("status_log")],
        &event("tasks/t.md", &[], &["status"], "aaa", "bbb"),
    )
    .unwrap();
    assert_eq!(effects[0].kind, EffectKind::AppendSection);
    insta::assert_json_snapshot!("status_log_append", effects);
}

// All rules over one event: a status-field change on a task file. Deterministic
// union across the whole rule set, in slice order.
#[test]
fn golden_all_rules_on_status_change() {
    let effects = eval(
        &all_rules(),
        &event("tasks/t.md", &["Status"], &["status"], "aaa", "bbb"),
    )
    .unwrap();
    insta::assert_json_snapshot!("all_rules_status_change", effects);
}
