//! C3 — v2 splice-response key set. `body.armed.effects` is post-v2 and must
//! not reach a v2 session (`docs/wire-contract-v2.md` §4.4/§5.2).
//!
//! Oracle: frozen `armed` is exactly `{path, edits}` (no `effects`).
//! Pins (All-Hands #3): v2 armed → `{path, edits}`; v3 → `{path, edits, effects}`.
//! Arms must differ by exactly `effects` or the leak pin is blind.
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!

use std::collections::BTreeSet;

use wire::{
    Armed, ArmedEdit, EffectEnvelope, NodeRev, Path, Response, ResponseBody, ResponsePayload, Root,
    SecRef, Span,
};

fn armed_effect() -> EffectEnvelope {
    EffectEnvelope {
        intents: vec![],
        narrowed: vec![],
        findings: vec![],
        how: "how:\n  route: { info: channel-review }\n".into(),
    }
}

/// Splice response with a reaction armed — the leak vehicle.
fn splice_response(effects: Vec<EffectEnvelope>) -> Response {
    Response {
        id: Some(42),
        ok: true,
        payload: ResponsePayload::Body {
            body: ResponseBody::Splice {
                armed: Armed {
                    path: Path("notes/plan.md".into()),
                    file_rev_after: None,
                    edits: vec![ArmedEdit {
                        target: SecRef::Hpath { hpath: vec![] },
                        node_rev_before: NodeRev("33d5b0e1b27cb48b".into()),
                        node_rev_after: NodeRev("41f643f034e5681f".into()),
                        span_after: Span(49, 75),
                    }],
                    effects,
                },
                receipt: None,
                root_before: Root("b3:a".into()),
                root_after: Some(Root("b3:b".into())),
                seq: Some(1),
                dry: None,
                verdicts: vec![],
                pin: None,
            },
        },
    }
}

/// Host v2 path: `demote_v2` then serialize (`sidecar::write_response` /
/// `registry::wire_line`).
///
fn v2_wire(response: &Response) -> serde_json::Value {
    let demoted = wire_serve::rev::demote_v2(response);
    serde_json::to_value(demoted.as_ref().unwrap_or(response)).expect("serializes")
}

fn keys(value: &serde_json::Value) -> BTreeSet<String> {
    value.as_object().expect("object").keys().cloned().collect()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(ToString::to_string).collect()
}

/// Leak pin: exact key set (not subset) — `contains` would miss a v3 field on v2.
///
///
#[test]
fn a_v2_splice_response_armed_has_exactly_the_frozen_key_set() {
    let wire = v2_wire(&splice_response(vec![armed_effect()]));
    assert_eq!(
        keys(&wire["body"]["armed"]),
        set(&["path", "edits"]),
        "the frozen §4.4 armed fact is exactly {{path, edits}}: {wire}"
    );
    assert_eq!(
        keys(&wire["body"]),
        set(&["armed", "root_before", "root_after", "seq", "verdicts"]),
        "and the frozen body key set is unchanged around it: {wire}"
    );
}

/// Control: v3 `armed` carries `effects`; diff vs v2 is exactly that field
/// (All-Hands #3). Same key sets ⇒ leak pin above is blind.
///
///
///
///
#[test]
fn a_v3_splice_response_still_carries_the_reaction_plane() {
    let response = splice_response(vec![armed_effect()]);
    let v3 = serde_json::to_value(&response).expect("serializes");
    let v3_keys = keys(&v3["body"]["armed"]);
    assert_eq!(
        v3_keys,
        set(&["path", "edits", "effects"]),
        "v3 keeps it: {v3}"
    );

    let v2_keys = keys(&v2_wire(&response)["body"]["armed"]);
    assert_eq!(
        v3_keys
            .difference(&v2_keys)
            .cloned()
            .collect::<BTreeSet<_>>(),
        set(&["effects"]),
        "the two arms differ by EXACTLY the field under test — that difference \
         is what makes the leak test an instrument rather than a decoration"
    );
}

/// No reaction armed → no demotion (projection of present fields only).
///
#[test]
fn a_splice_that_armed_nothing_is_not_demoted() {
    let response = splice_response(vec![]);
    assert!(
        wire_serve::rev::demote_v2(&response).is_none(),
        "nothing post-v2 present ⇒ no demotion at all"
    );
}
