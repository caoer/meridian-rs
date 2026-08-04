//! C3 — **the v2 splice-response key set.** `body.armed.effects` postdates
//! frozen v2 and must not reach a v2 session.
//!
//! Third sighting of the All-Hands #1 class and the first on the plain
//! splice-response path. One producer, two exits: `write.rs` fills a single
//! `armed_effects` and writes it to BOTH the ring frame (closed by U20b's
//! `NotificationRoot` row) and this response body.
//!
//! # The oracle
//! The frozen §4.4/§5.2 worked frame (`docs/wire-contract-v2.md`) prints
//! `armed` as exactly `{path, edits}`. `effects` is absent from it, so it has a
//! VINTAGE answer — it postdates the freeze — and is a registry row rather than
//! an authorship-keyed strip.
//!
//! # The control, and why it is stated before it is written (All-Hands #3)
//! The two worlds this suite must tell apart:
//! - **v2 session, reaction armed** → `armed` prints `{path, edits}`.
//! - **v3 session, reaction armed** → `armed` prints `{path, edits, effects}`.
//!
//! Their outputs DIFFER, and the differing key is the one under test. A pin
//! whose two arms printed the same key set would be a decoration: it could not
//! distinguish the healthy plane from the leaking one.

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

/// A splice response whose batch armed a reaction — the shape the leak rides.
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

/// Serialize the way a host's v2 branch does: demote, then write the typed
/// value. This is the production path (`sidecar::write_response`,
/// `registry::wire_line`), not a re-implementation of it.
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

/// **THE LEAK TEST.** Exhaustive, not a subset check: a `contains`-style
/// assertion passes while a v3 field rides a v2 envelope, which is the whole
/// reason this class survived three sightings.
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

/// **THE CONTROL — the other world.** A v3 session is entitled to the reaction
/// plane, so its `armed` DOES carry `effects`.
///
/// Stated as a diff, per All-Hands #3: this arm's key set must differ from the
/// v2 arm's by exactly `effects`. If the two arms printed the same set, the pin
/// above would be blind and would keep passing greenly forever.
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

/// A splice that armed NO reaction is untouched: demotion is a projection of
/// what is there, never an unconditional edit of the shape.
#[test]
fn a_splice_that_armed_nothing_is_not_demoted() {
    let response = splice_response(vec![]);
    assert!(
        wire_serve::rev::demote_v2(&response).is_none(),
        "nothing post-v2 present ⇒ no demotion at all"
    );
}
