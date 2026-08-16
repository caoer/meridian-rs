//! Wire decode of the `scoped-guards` family (fp-grain PR-4).
//!
//! The in-process fold lives in `scoped_guards.rs`. This file pins the
//! DOOR: field walls, pair law, un-negotiated teaching, sugar desugar.
//! Bare `if_fingerprint` stays the v2 root premise — those tests are
//! untouched (`u10_guard.rs`, `write_e2e`).

use serde_json::{Map, Value, json};
use wire::{ErrorCode, Op};
use wire_serve::decode::decode;
use wire_serve::guard::lower_premises;
use wire_serve::rev::Rev;

fn obj(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        other => panic!("fixture must be an object, got {other}"),
    }
}

fn splice_edit() -> Value {
    json!([{
        "target": {"hpath": [{"h": "Target"}]},
        "edit": {"match": {"old": "x", "new": "y"}}
    }])
}

#[test]
fn v2_scope_on_splice_is_unnegotiated_not_unknown_field() {
    let err = decode(
        &obj(json!({
            "op": "splice",
            "path": "a.md",
            "if_root": "b3:aaaa",
            "scope": "a.md",
            "edits": splice_edit(),
        })),
        Rev::V2,
    )
    .expect_err("v2 never negotiated the family");
    assert_eq!(err.code, ErrorCode::BadRequest);
    let message = err.message.expect("teaching");
    assert!(
        message.contains("did not negotiate scoped-guards"),
        "the §8.2 un-negotiated text speaks: {message}"
    );
    assert!(
        message.contains("scope"),
        "the teaching names the field: {message}"
    );
    assert!(
        !message.contains("unknown request field"),
        "this is not a typo wall: {message}"
    );
}

#[test]
fn v2_bare_fingerprint_op_stays_parameterless() {
    let op = decode(&obj(json!({"op": "root"})), Rev::V2).expect("bare mint");
    assert_eq!(
        op,
        Op::Root {
            scope: None,
            scope_bytes: None
        }
    );
}

#[test]
fn v2_scope_on_fingerprint_is_unnegotiated() {
    let err = decode(&obj(json!({"op": "root", "scope": "a.md"})), Rev::V2)
        .expect_err("v2 mint is world-only");
    assert_eq!(err.code, ErrorCode::BadRequest);
    let message = err.message.expect("teaching");
    assert!(
        message.contains("did not negotiate scoped-guards"),
        "{message}"
    );
}

#[test]
fn v3_sugar_pair_without_token_refuses() {
    let err = decode(
        &obj(json!({
            "op": "splice",
            "path": "a.md",
            "scope": "a.md",
            "edits": splice_edit(),
        })),
        Rev::V3,
    )
    .expect_err("half a premise");
    assert_eq!(err.code, ErrorCode::BadRequest);
    let message = err.message.expect("teaching");
    assert!(
        message.contains("scope without if_fingerprint"),
        "{message}"
    );
}

#[test]
fn v3_guards_both_spellings_refuse() {
    let err = decode(
        &obj(json!({
            "op": "splice",
            "path": "a.md",
            "edits": splice_edit(),
            "guards": [{
                "fingerprint": "b3:aaaa",
                "scope": "a.md",
                "scope_bytes": "YS5tZA"
            }]
        })),
        Rev::V3,
    )
    .expect_err("one node, one spelling");
    assert_eq!(err.code, ErrorCode::BadRequest);
    let message = err.message.expect("teaching");
    assert!(message.contains("both scope and scope_bytes"), "{message}");
}

#[test]
fn v3_mint_both_spellings_refuse_the_mint_pair_text() {
    let err = decode(
        &obj(json!({
            "op": "root",
            "scope": "a.md",
            "scope_bytes": "YS5tZA"
        })),
        Rev::V3,
    )
    .expect_err("a mint names ONE node");
    assert_eq!(err.code, ErrorCode::BadRequest);
    let message = err.message.expect("teaching");
    assert!(
        message.contains("names its node twice"),
        "the mint-pair text, not the premise-pair text: {message}"
    );
}

#[test]
fn v3_sugar_desugars_to_one_scoped_premise_and_consumes_the_world_slot() {
    let op = decode(
        &obj(json!({
            "op": "splice",
            "path": "a/target.md",
            "if_root": "b3:dead",
            "scope": "a/target.md",
            "edits": splice_edit(),
        })),
        Rev::V3,
    )
    .expect("sugar is legal");
    let Op::Splice {
        if_root,
        scope,
        guards,
        ..
    } = op
    else {
        panic!("splice: {op:?}");
    };
    let (world, premises) = lower_premises(if_root, scope, &guards).expect("lower");
    assert!(
        world.is_none(),
        "scoped sugar is not also a world guard: {world:?}"
    );
    assert_eq!(premises.len(), 1);
    assert_eq!(
        premises[0].scope.as_deref(),
        Some(std::path::Path::new("a/target.md"))
    );
}

#[test]
fn v3_bare_if_root_stays_the_world_guard() {
    let op = decode(
        &obj(json!({
            "op": "splice",
            "path": "a.md",
            "if_root": "b3:dead",
            "edits": splice_edit(),
        })),
        Rev::V3,
    )
    .expect("bare sugar");
    let Op::Splice {
        if_root,
        scope,
        guards,
        ..
    } = op
    else {
        panic!("splice");
    };
    let (world, premises) = lower_premises(if_root, scope, &guards).expect("lower");
    assert!(world.is_some(), "bare if_fingerprint is the v2 world slot");
    assert!(premises.is_empty(), "no list entries");
}

#[test]
fn v3_mint_arm_decodes_scope() {
    let op =
        decode(&obj(json!({"op": "root", "scope": "a/target.md"})), Rev::V3).expect("scoped mint");
    assert_eq!(
        op,
        Op::Root {
            scope: Some(wire::Path("a/target.md".into())),
            scope_bytes: None
        }
    );
}

#[test]
fn top_level_scope_bytes_on_splice_is_unknown() {
    // §5.4 matrix: scope_bytes is a top-level field on NO write door.
    let err = decode(
        &obj(json!({
            "op": "splice",
            "path": "a.md",
            "if_root": "b3:dead",
            "scope_bytes": "YS5tZA",
            "edits": splice_edit(),
        })),
        Rev::V3,
    )
    .expect_err("not a top-level splice field");
    assert_eq!(err.code, ErrorCode::BadRequest);
    let message = err.message.expect("teaching");
    assert!(
        message.contains("unknown request field `scope_bytes`"),
        "{message}"
    );
}
