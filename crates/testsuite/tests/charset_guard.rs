//! CHARSET-GUARD discrimination gate (ruling 011 / wire-contract-v2 §2.4).
//!
//! The one block-id charset `[A-Za-z0-9-]`, both planes. This module rides the
//! `data/charset-guard/discrimination.json` pack authored for the downstream
//! units: every per-mint-position `_`-bearing `input_id` is refused by the
//! `model::Ref::anchor` mint-guard (`Err(BadAnchorId)`), every `clean_accepts`
//! id mints, and the pack-pinned walk answer (obsidian-compat WX-6) still says
//! what the pack claims — the same id the mint plane refuses loud, the walk
//! plane drops to `ref_not_found` (decision 013 consequence 3: both conform).

use model::{BadAnchorId, Ref};
use serde_json::Value;
use std::fs;

fn pack() -> Value {
    let path = testsuite::charset_guard_dir().join("discrimination.json");
    let raw = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse discrimination.json: {e}"))
}

#[test]
fn every_mint_refusal_input_is_refused_by_the_guard() {
    let pack = pack();
    let refusals = pack["mint_refusals"]
        .as_array()
        .expect("mint_refusals array");
    assert!(!refusals.is_empty(), "pack carries mint-refusal inputs");
    for entry in refusals {
        let id = entry["input_id"].as_str().expect("input_id string");
        assert!(
            id.contains('_'),
            "each mint-refusal input carries the disallowed `_`: {id:?}"
        );
        assert_eq!(
            entry["mint_disposition"].as_str(),
            Some("bad_request"),
            "pack disposition for {id:?}"
        );
        assert_eq!(
            Ref::anchor(id),
            Err(BadAnchorId { id: id.to_string() }),
            "mint-guard refuses {id:?} (-> bad_request downstream)"
        );
    }
}

#[test]
fn every_clean_id_mints() {
    let pack = pack();
    let clean = pack["clean_accepts"]
        .as_array()
        .expect("clean_accepts array");
    for id in clean {
        let id = id.as_str().expect("clean id string");
        assert_eq!(
            Ref::anchor(id),
            Ok(Ref::Anchor(id.to_string())),
            "clean id {id:?} mints"
        );
    }
}

#[test]
fn walk_answer_pin_matches_the_obsidian_compat_oracle() {
    let pack = pack();
    let pin = &pack["walk_answer_pin"];
    let id = pin["same_id_via_walk"].as_str().expect("same_id_via_walk");
    let probe_id = pin["probe"].as_str().expect("probe");

    // the SAME id is refused loud on the mint plane ...
    assert!(Ref::anchor(id).is_err(), "mint plane refuses {id:?}");

    // ... and the walk plane follows the app: the referenced obsidian-compat
    // WX-6 answer is `ref_not_found` (silent drop), authored by the oracle, not
    // hand-written here.
    let oracle_path = testsuite::gt_pack_dir().join("obsidian-compat/resolution.expected.json");
    let oracle: Value = serde_json::from_str(
        &fs::read_to_string(&oracle_path).unwrap_or_else(|e| panic!("read oracle: {e}")),
    )
    .expect("oracle json");
    let probe = oracle["probes"]
        .as_array()
        .expect("probes array")
        .iter()
        .find(|p| p["id"].as_str() == Some(probe_id))
        .unwrap_or_else(|| panic!("probe {probe_id} present in obsidian-compat pack"));

    assert!(
        probe["ref"].as_str().is_some_and(|r| r.contains('_')),
        "the pinned walk probe carries a `_` id"
    );
    assert_eq!(
        probe["answer"]["ok"].as_bool(),
        Some(false),
        "walk drops the `_` id"
    );
    assert_eq!(
        probe["answer"]["error"]["code"].as_str(),
        Some("ref_not_found"),
        "walk answer is ref_not_found (app silent-drop)"
    );
    // the pack's copy of the answer agrees with the oracle it points at.
    assert_eq!(pin["answer"], probe["answer"], "pack pin == oracle answer");
}

#[test]
fn underscore_exemption_fixtures_are_present_unmigrated() {
    // Assert-by-listing (011 § Addendum): the in-repo `_` fixture stays verbatim
    // — the WX-6 probe id in blocks.md, never re-minted to a dash form.
    let blocks = testsuite::gt_pack_dir().join("obsidian-compat/walkvault/blocks.md");
    let text = fs::read_to_string(&blocks).unwrap_or_else(|e| panic!("read blocks.md: {e}"));
    assert!(
        text.contains("^under-probe_x"),
        "the `_`-bearing WX-6 probe stays in blocks.md (exempt, not migrated)"
    );
    assert!(
        !text.contains("^under-probe-x"),
        "it was NOT re-minted to a dash form"
    );
}

/// W4-AMEND gate 4 (re-homed CHARSET-GUARD splice-target position, the
/// STAGED-OPEN follow-up): a `_`-bearing SPLICE-TARGET anchor is refused
/// `bad_request`. The wire type admits the bytes (newtypes name, never
/// validate — the frame must DECODE so the refusal can be loud, not a parse
/// mystery); the mint-guard refuses them; the answer is the fix-class §8
/// envelope. Rides the pack's per-position refusal inputs.
#[test]
fn underscore_splice_target_anchor_is_bad_request() {
    let pack = pack();
    let refusals = pack["mint_refusals"]
        .as_array()
        .expect("mint_refusals array");
    let splice_targets: Vec<&str> = refusals
        .iter()
        .filter(|e| e["position"].as_str() == Some("splice target anchor"))
        .map(|e| e["input_id"].as_str().expect("input_id string"))
        .collect();
    assert!(
        !splice_targets.is_empty(),
        "pack carries the splice-target position"
    );
    for id in splice_targets {
        // 1. the frame decodes: the wire type admits the bytes
        let frame = serde_json::json!({
            "id": 9, "op": "splice", "path": "notes/plan.md",
            "edits": [{
                "target": {"anchor": id},
                "edit": {"match": {"old": "a", "new": "b"}}
            }]
        });
        let req: wire::Request = serde_json::from_value(frame).expect("wire admits the bytes");
        let wire::Op::Splice { edits, .. } = &req.op else {
            panic!("splice op")
        };
        assert_eq!(
            edits[0].target,
            wire::SecRef::Anchor { anchor: id.into() },
            "target decodes verbatim"
        );

        // 2. the mint-guard refuses the id at the target position
        assert_eq!(
            Ref::anchor(id),
            Err(BadAnchorId { id: id.to_string() }),
            "mint-guard refuses splice-target {id:?}"
        );

        // 3. the refusal on the wire: bad_request, fix class (§8)
        let refusal = wire::ErrorBody::new(wire::ErrorCode::BadRequest);
        assert_eq!(refusal.recovery, wire::Recovery::Fix);
        assert_eq!(
            serde_json::to_value(wire::Response {
                id: req.id,
                ok: false,
                payload: wire::ResponsePayload::Error { error: refusal },
            })
            .unwrap(),
            serde_json::json!({"id":9,"ok":false,
                "error":{"code":"bad_request","recovery":"fix"}})
        );
    }
}
