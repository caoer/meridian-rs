//! The remove door's decode wall (§ A.3): no `force` field exists on the op —
//! a frame carrying one refuses at the strict field wall, loud — and the
//! guard field decodes as schema-optional (§ A.1), the demand being semantic.

use serde_json::{Map, Value, json};
use wire::{ErrorCode, Op};
use wire_serve::decode::decode;
use wire_serve::rev::Rev;

fn frame(v: Value) -> Map<String, Value> {
    v.as_object().expect("test frame is an object").clone()
}

/// The full request decodes to `Op::Remove`, every field carried.
#[test]
fn the_remove_request_decodes_whole() {
    let op = decode(
        &frame(json!({
            "id": 9,
            "op": "remove",
            "path": "notes/old.md",
            "if_file_rev": "e3c4acaceb75b907",
            "actor": "agent:b0864fb2",
            "now": "2026-08-15T12:00:00Z",
            "if_root": "b3:aaa",
            "dry": true
        })),
        Rev::V3,
    )
    .expect("a well-formed remove decodes");
    let Op::Remove {
        path,
        if_file_rev,
        actor,
        now,
        if_root,
        dry,
    } = op
    else {
        panic!("decoded to another op");
    };
    assert_eq!(path.0, "notes/old.md");
    assert_eq!(
        if_file_rev.map(|r| r.0).as_deref(),
        Some("e3c4acaceb75b907")
    );
    assert_eq!(actor.as_deref(), Some("agent:b0864fb2"));
    assert_eq!(now.as_deref(), Some("2026-08-15T12:00:00Z"));
    assert_eq!(if_root.map(|r| r.0).as_deref(), Some("b3:aaa"));
    assert_eq!(dry, Some(true));
}

/// § A.1: a rev-less frame still DECODES — the `guard_required` demand is the
/// engine's, after decode, never a frame rejection.
#[test]
fn a_rev_less_remove_frame_still_decodes() {
    let op = decode(
        &frame(json!({"id": 1, "op": "remove", "path": "notes/old.md"})),
        Rev::V3,
    )
    .expect("schema-optional guard: the frame decodes");
    assert!(matches!(
        op,
        Op::Remove {
            if_file_rev: None,
            ..
        }
    ));
}

/// § A.3: the op declares no `force` — the strict field wall refuses the key,
/// so the no-escape-hatch ruling is structural, not a runtime branch.
#[test]
fn a_force_bearing_remove_hits_the_strict_field_wall() {
    let err = decode(
        &frame(json!({
            "id": 2,
            "op": "remove",
            "path": "notes/old.md",
            "if_file_rev": "e3c4acaceb75b907",
            "force": true
        })),
        Rev::V3,
    )
    .expect_err("force does not exist on the remove op");
    assert_eq!(err.code, ErrorCode::BadRequest);
    assert!(
        err.message.as_deref().is_some_and(|m| m.contains("force")),
        "the refusal names the unknown field: {:?}",
        err.message
    );
}

/// §9: a malformed `now` refuses at decode — validated, never generated.
#[test]
fn a_malformed_now_refuses_at_decode() {
    let err = decode(
        &frame(json!({
            "id": 3,
            "op": "remove",
            "path": "notes/old.md",
            "if_file_rev": "e3c4acaceb75b907",
            "now": "yesterday-ish"
        })),
        Rev::V3,
    )
    .expect_err("a malformed now refuses");
    assert_eq!(err.code, ErrorCode::BadRequest);
}
