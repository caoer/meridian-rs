//! U20b — **the v2 key-set detector for NOTIFICATION frames.**
//!
//! Cloned from the shape that caught the U11 leak
//! (`u11_mismatch_ladder.rs::a_frozen_v2_session_never_grows_a_field_from_the_ladder`):
//! assert the EXACT KEY SET of a serialized frame, never its values. A v3-only
//! field in a v2 envelope changes no value, so a value-pinning sweep is green
//! and blind to it — key-set pinning is the only detector for this class.
//!
//! Notification frames need their own detector because they are response-side by
//! construction and are not replies: no "the client asked for this shape"
//! reasoning protects them. A subscriber receives whatever the host emits.

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
    }
}

fn serialize_v2(frame: &DeltaFrame) -> serde_json::Value {
    let mut out = Vec::new();
    wire_serve::ring::write_frame(&mut out, frame, false).expect("frame serializes");
    serde_json::from_slice(&out).expect("frame is JSON")
}

/// The frozen v2 notification shape: `{"delta":{…}}` and nothing else — no `id`
/// (§3.1 classification is what makes it a Notification rather than a response).
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

/// **The leak, now closed.** `DeltaFrame::effects` is
/// `#[serde(skip_serializing_if = "Vec::is_empty")]`, which skips on an empty
/// VALUE and never on a v2 SESSION — so before the v2-reserved-field registry,
/// the moment a reaction fired a v2 subscriber received `effects`, a field that
/// postdates its contract:
///
/// ```text
/// V2 BARE   : {"delta":{…}}
/// V2 REACTED: {"delta":{…},"effects":[{"intents":[],…,"how":"echo hi"}]}
/// ```
///
/// The fix is one row in `rev::V2_RESERVED_FIELDS`, consulted by the v2
/// projection at both hosts (Law 3). This test WAS this file's `#[ignore]`d
/// known-red while the mechanism was with the advisor; it is the mutation proof
/// for the row, and reverting the row reddens it exactly as it did then.
///
/// Restoring conformance with ratified frozen v2 — not amending it. The defect
/// predates U20b (the sidecar has served v2 `sub` through this serializer since
/// T5-SUB); it heals at both hosts because the table is shared.
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

/// **The order pin — what a key-SET assertion structurally cannot see.**
///
/// The v2 demotion strips reserved fields on the TYPED value. The obvious
/// "simplification" is to serialize to `Value`, remove the key, and write that
/// back. It would pass every key-set pin in this file and still break the frozen
/// contract at the byte level: `serde_json::Map` is a `BTreeMap` here
/// (`preserve_order` is off), so a `Value` round-trip ALPHABETIZES keys, turning
/// the frozen `seq, root_before, root_after, files` into `files, root_after,
/// root_before, seq`.
///
/// So this asserts the serialized ORDER, which is the only assertion that
/// catches it.
#[test]
fn a_v2_frame_prints_struct_order_not_alphabetical() {
    let mut out = Vec::new();
    wire_serve::ring::write_frame(&mut out, &frame_of(vec![]), false).expect("serializes");
    let line = String::from_utf8(out).expect("UTF-8");
    assert!(
        line.contains(r#"{"delta":{"seq":1,"root_before":"b3:a","root_after":"b3:b","files":[]}}"#),
        "the v2 frame prints struct order, byte-for-byte as the frozen contract does: {line}"
    );

    // The same must hold on the DEMOTED path — the branch that had to touch the
    // frame is exactly the one that could reorder it.
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

/// A v3 subscriber still receives the reaction plane — the demotion is a v2
/// PROJECTION, not a deletion. Without this, stripping `effects` unconditionally
/// would pass every other test in this file.
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

// ---------------------------------------------------------------------------
// The vintage/provenance split in `demote_v2` (leader ruling, 2026-08-04)
// ---------------------------------------------------------------------------

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

/// **VINTAGE holds without a rung.** The four ladder extras postdate frozen v2,
/// so a v2 session must not see them whoever wrote the value — the registry
/// answers a schema question, not an authorship one.
///
/// Unreachable through production today (`ladder::enrich` is the sole author of
/// all six slots and always sets `rung`), which is exactly why it is pinned
/// directly: the factoring exists so the vintage half keeps holding if a later
/// unit sets one of these without a rung, and only a constructed envelope can
/// prove that now.
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

/// **PROVENANCE protects the v2-LEGAL fields.** `message` and `path` are legal
/// v2 vocabulary, so an ordinary refusal keeps them. Only the ladder's own are
/// stripped, and `rung` is what proves the ladder wrote them.
///
/// This is the assertion that a registry row for `message` would break — the
/// regression that the "migrate all six to the table" order would have shipped.
#[test]
fn a_non_ladder_refusal_keeps_its_v2_legal_message_and_path() {
    let mut error = cas_error();
    error.message = Some("plain refusal wording".into());
    error.path = Some(wire::Path("plan.md".into()));
    assert!(
        wire_serve::rev::demote_v2(&error_response(error)).is_none(),
        "a refusal with no post-v2 field and no ladder mark is not demoted at all"
    );
}

/// The ladder's own `message`/`path` still go, so a demoted v2 `cas_mismatch`
/// prints exactly what it always printed. The control for the test above: it
/// proves the provenance path can still fire, so "keeps its message" is not
/// passing because nothing ever strips one.
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
