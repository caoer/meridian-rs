//! The death reply (`ResponseBody::Remove`) in the untagged enum: field shape
//! alone discriminates on the wire, so a new variant can silently capture
//! another op's frame — and the failure is a mis-parse, not an error. These
//! tests pin the discrimination in both directions, the `create_shape.rs`
//! pattern applied to the twin door.

use serde_json::json;
use wire::{NodeRev, Path, ResponseBody, Root};

fn remove_body() -> ResponseBody {
    ResponseBody::Remove {
        path: Path("notes/departed.md".into()),
        file_rev_before: NodeRev("e3c4acaceb75b907".into()),
        root_before: Root("b3:aaa".into()),
        root_after: Some(Root("b3:bbb".into())),
        seq: Some(4),
        dry: None,
        verdicts: Vec::new(),
    }
}

/// The death body round-trips to itself — no earlier variant captures it.
#[test]
fn the_remove_body_round_trips_as_remove() {
    let wire_bytes = serde_json::to_value(remove_body()).expect("serializes");
    let back: ResponseBody = serde_json::from_value(wire_bytes.clone()).expect("round-trips");
    assert!(
        matches!(back, ResponseBody::Remove { .. }),
        "the death frame resolved to another variant — untagged capture: {wire_bytes}"
    );
    assert_eq!(
        serde_json::to_value(&back).expect("re-serializes"),
        wire_bytes,
        "the round-trip is byte-stable"
    );
}

/// A dry death's `root_after` is contractually null, not absent (the same
/// absence-vs-null rule `splice` and `create` carry), and still resolves to
/// `Remove`.
#[test]
fn a_dry_death_keeps_its_null_root_after() {
    let dry = ResponseBody::Remove {
        path: Path("rehearsal.md".into()),
        file_rev_before: NodeRev("a5172fcd1c0ce8fb".into()),
        root_before: Root("b3:aaa".into()),
        root_after: None,
        seq: None,
        dry: Some(true),
        verdicts: Vec::new(),
    };
    let v = serde_json::to_value(&dry).expect("serializes");
    assert!(
        v.get("root_after").is_some_and(serde_json::Value::is_null),
        "`root_after` is always serialized, null on a rehearsal: {v}"
    );
    assert!(
        v.get("seq").is_none(),
        "a rehearsal emits no Delta, so `seq` is ABSENT: {v}"
    );
    let back: ResponseBody = serde_json::from_value(v).expect("round-trips");
    assert!(matches!(back, ResponseBody::Remove { .. }));
}

/// `Remove` must not capture the birth frame: `file_rev_after` is `Create`'s
/// discriminator and `Remove` has no such field — and the other way around,
/// `file_rev_before` is `Remove`'s and `Create` has no such field.
#[test]
fn birth_and_death_frames_do_not_capture_each_other() {
    let birth = json!({
        "path": "notes/newborn.md",
        "file_rev_after": "f3c6d9b647936581",
        "root_before": "b3:aaa",
        "root_after": "b3:bbb",
        "seq": 1,
        "verdicts": []
    });
    let back: ResponseBody = serde_json::from_value(birth.clone()).expect("birth decodes");
    assert!(
        matches!(back, ResponseBody::Create { .. }),
        "the birth frame was captured by another variant: {birth}"
    );

    let death = json!({
        "path": "notes/departed.md",
        "file_rev_before": "e3c4acaceb75b907",
        "root_before": "b3:aaa",
        "root_after": "b3:bbb",
        "seq": 4,
        "verdicts": []
    });
    let back: ResponseBody = serde_json::from_value(death.clone()).expect("death decodes");
    assert!(
        matches!(back, ResponseBody::Remove { .. }),
        "the death frame was captured by another variant: {death}"
    );
}
