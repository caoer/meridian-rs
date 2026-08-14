//! U20b — v2 key-set pin for notification frames (exact keys, never values).
//!
//! Class sibling of `u11_mismatch_ladder` key-set pin: v3-only field on v2 is
//! value-invisible. Notifications are not replies — host emits, subscriber gets.

use std::collections::BTreeSet;

use wire::{Delta, DeltaFrame, EffectEnvelope, Root};

fn keys(value: &serde_json::Value) -> BTreeSet<String> {
    value
        .as_object()
        .expect("a frame is a JSON object")
        .keys()
        .cloned()
        .collect()
}

fn frame_of(effects: Vec<EffectEnvelope>) -> DeltaFrame {
    DeltaFrame {
        delta: Delta {
            seq: 1,
            root_before: Root("b3:a".into()),
            root_after: Root("b3:b".into()),
            actor: None,
            now: None,
            files: vec![],
        },
        effects,
        rescope: None,
        overflow: None,
    }
}

fn serialize_v2(frame: &DeltaFrame) -> serde_json::Value {
    let mut out = Vec::new();
    wire_serve::ring::write_frame(&mut out, frame, false).expect("frame serializes");
    serde_json::from_slice(&out).expect("frame is JSON")
}

/// Frozen v2: top key set `{delta}` only (§3.1 Notification; no `id`).
#[test]
fn a_v2_notification_frame_has_exactly_the_frozen_key_set() {
    let value = serialize_v2(&frame_of(vec![]));
    assert_eq!(
        keys(&value),
        ["delta"]
            .iter()
            .map(ToString::to_string)
            .collect::<BTreeSet<_>>(),
        "a v2 notification carries the delta and nothing else"
    );
    assert_eq!(
        keys(&value["delta"]),
        ["seq", "root_before", "root_after", "files"]
            .iter()
            .map(ToString::to_string)
            .collect::<BTreeSet<_>>(),
        "the v2 delta key set is frozen (§7.1); `actor`/`now` are absent, never null"
    );
}

/// § A.9's frame with everything on: the summaries plus an `unattested` row.
fn rescoped_frame() -> DeltaFrame {
    let mut frame = frame_of(vec![]);
    frame.delta.files = vec![wire::DeltaFile {
        path: wire::Path("kept.md".into()),
        change: wire::FileChange::Unattested,
        from_path: None,
        file_rev_before: Some(wire::NodeRev("e3c4acaceb75b907".into())),
        file_rev_after: None,
        nodes: vec![],
    }];
    frame.rescope = Some(wire::Rescope {
        cause: wire::Path("meridian/domain.md".into()),
        unattested: 1,
        attested: 0,
    });
    frame.overflow = Some(wire::Overflow { dropped: 3 });
    frame
}

/// § A.9 v2 demotion: the summaries are post-v2 fields (stripped typed), and
/// `unattested` demotes to `deleted` — v2 keeps its birth vocabulary.
#[test]
fn a_v2_frame_demotes_unattested_to_deleted_and_strips_the_summaries() {
    let value = serialize_v2(&rescoped_frame());
    assert_eq!(
        keys(&value),
        ["delta"]
            .iter()
            .map(ToString::to_string)
            .collect::<BTreeSet<_>>(),
        "rescope/overflow postdate frozen v2 and never reach a v2 session"
    );
    assert_eq!(
        value["delta"]["files"][0]["change"],
        serde_json::json!("deleted"),
        "v2's closed change vocabulary: the un-attestation split is v3's"
    );
}

/// § A.9 v3: the summaries ride the frame root beside `delta`, and the
/// fingerprint rekey leaves them untouched.
#[test]
fn a_v3_frame_carries_the_rescope_summaries_at_the_root() {
    let mut out = Vec::new();
    wire_serve::ring::write_frame(&mut out, &rescoped_frame(), true).expect("frame serializes");
    let value: serde_json::Value = serde_json::from_slice(&out).expect("frame is JSON");
    assert_eq!(
        keys(&value),
        ["delta", "rescope", "overflow"]
            .iter()
            .map(ToString::to_string)
            .collect::<BTreeSet<_>>(),
    );
    assert_eq!(
        value["rescope"],
        serde_json::json!({"cause":"meridian/domain.md","unattested":1,"attested":0})
    );
    assert_eq!(value["overflow"], serde_json::json!({"dropped":3}));
    assert_eq!(
        value["delta"]["files"][0]["change"],
        serde_json::json!("unattested")
    );
    assert!(
        value["delta"].get("fingerprint_before").is_some(),
        "the v3 rekey still applies inside delta"
    );
}

/// Pin: reacted v2 frame never grows `effects` (`rev::V2_RESERVED_FIELDS`).
/// Value-skip on empty is not a session gate; registry row is the fix (Law 3).
#[test]
fn a_v2_notification_frame_never_grows_a_post_v2_field() {
    let reacted = frame_of(vec![EffectEnvelope {
        intents: vec![],
        narrowed: vec![],
        findings: vec![],
        how: "echo hi".into(),
    }]);
    assert_eq!(
        keys(&serialize_v2(&reacted)),
        ["delta"]
            .iter()
            .map(ToString::to_string)
            .collect::<BTreeSet<_>>(),
        "a v2 session must never receive `effects` — it postdates its contract"
    );
}

/// Order pin: struct field order, not alphabetical (`Value` round-trip would
/// BTreeMap-reorder and still pass key-set pins).
#[test]
fn a_v2_frame_prints_struct_order_not_alphabetical() {
    let mut out = Vec::new();
    wire_serve::ring::write_frame(&mut out, &frame_of(vec![]), false).expect("serializes");
    let line = String::from_utf8(out).expect("UTF-8");
    assert!(
        line.contains(r#"{"delta":{"seq":1,"root_before":"b3:a","root_after":"b3:b","files":[]}}"#),
        "the v2 frame prints struct order, byte-for-byte as the frozen contract does: {line}"
    );

    // Demoted path must keep the same order.
    let mut out = Vec::new();
    let reacted = frame_of(vec![EffectEnvelope {
        intents: vec![],
        narrowed: vec![],
        findings: vec![],
        how: "echo hi".into(),
    }]);
    wire_serve::ring::write_frame(&mut out, &reacted, false).expect("serializes");
    let line = String::from_utf8(out).expect("UTF-8");
    assert!(
        line.contains(r#"{"delta":{"seq":1,"root_before":"b3:a","root_after":"b3:b","files":[]}}"#),
        "a demoted v2 frame is byte-identical to a frame that never had effects: {line}"
    );
}

/// Control: v3 still receives `effects` (projection, not unconditional delete).
#[test]
fn a_v3_session_still_receives_effects() {
    let reacted = frame_of(vec![EffectEnvelope {
        intents: vec![],
        narrowed: vec![],
        findings: vec![],
        how: "echo hi".into(),
    }]);
    let mut out = Vec::new();
    wire_serve::ring::write_frame(&mut out, &reacted, true).expect("serializes");
    let value: serde_json::Value = serde_json::from_slice(&out).expect("JSON");
    assert!(
        keys(&value).contains("effects"),
        "v3 is entitled to the reaction plane: {value}"
    );
}

// Vintage/provenance split in `demote_v2`.

fn cas_error() -> wire::ErrorBody {
    wire::ErrorBody::new(wire::ErrorCode::CasMismatch)
}

fn error_response(error: wire::ErrorBody) -> wire::Response {
    wire::Response {
        id: Some(1),
        ok: false,
        payload: wire::ResponsePayload::Error { error },
    }
}

/// Vintage: post-v2 field demotes without authorship mark (`rung` absent).
#[test]
fn a_post_v2_field_is_demoted_even_with_no_authorship_mark() {
    let mut error = cas_error();
    error.new_fingerprint = Some(wire::NodeRev("beef000000000000".into()));
    let demoted = wire_serve::rev::demote_v2(&error_response(error))
        .expect("a post-v2 field is demoted on its vintage alone");
    let wire::ResponsePayload::Error { error } = &demoted.payload else {
        panic!("still an error payload");
    };
    assert!(
        error.new_fingerprint.is_none(),
        "vintage is a schema fact, not a question about who wrote the value"
    );
}

/// Early return: nothing post-v2 ⇒ no demotion (does not exercise strip block).
#[test]
fn a_refusal_with_nothing_post_v2_is_not_demoted_at_all() {
    let mut error = cas_error();
    error.message = Some("plain refusal wording".into());
    error.path = Some(wire::Path("plan.md".into()));
    assert!(
        wire_serve::rev::demote_v2(&error_response(error)).is_none(),
        "no post-v2 field and no ladder mark ⇒ the early return, before any strip"
    );
}

/// Provenance: non-ladder refusal keeps v2-legal `message`/`path` under demotion.
/// Fixture forces demotion via post-v2 field without `rung` so strip runs.
#[test]
fn a_non_ladder_refusal_keeps_its_v2_legal_message_and_path() {
    let mut error = cas_error();
    // No rung + post-v2 field → demotion reaches strip.
    error.new_fingerprint = Some(wire::NodeRev("beef000000000000".into()));
    error.message = Some("plain refusal wording".into());
    error.path = Some(wire::Path("plan.md".into()));

    let demoted = wire_serve::rev::demote_v2(&error_response(error))
        .expect("the post-v2 field forces a demotion, so the strip block runs");
    let wire::ResponsePayload::Error { error } = &demoted.payload else {
        panic!("still an error payload");
    };
    assert!(
        error.new_fingerprint.is_none(),
        "vintage: the post-v2 field goes"
    );
    assert_eq!(
        error.message.as_deref(),
        Some("plain refusal wording"),
        "provenance: a v2-LEGAL message SURVIVES a demotion it did not author"
    );
    assert_eq!(
        error.path.as_ref().map(|p| p.0.as_str()),
        Some("plan.md"),
        "provenance: so does its path"
    );
}

/// Control: ladder-authored `message`/`path` still strip with `rung`.
#[test]
fn the_ladders_own_message_and_path_are_still_stripped() {
    let mut error = cas_error();
    error.rung = Some(2);
    error.message = Some("ladder teaching text".into());
    error.path = Some(wire::Path("plan.md".into()));
    let demoted =
        wire_serve::rev::demote_v2(&error_response(error)).expect("a ladder envelope demotes");
    let wire::ResponsePayload::Error { error } = &demoted.payload else {
        panic!("still an error payload");
    };
    assert!(error.rung.is_none(), "the rung is post-v2");
    assert!(
        error.message.is_none() && error.path.is_none(),
        "the ladder's own teaching slots go with it"
    );
}
