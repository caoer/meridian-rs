//! `ResponseBody` is an UNTAGGED enum: the field shape alone discriminates on
//! the wire. Adding the birth body (`create`) therefore carries a real hazard —
//! a new variant can silently capture another op's frame, or be captured by an
//! earlier one, and the failure is a MIS-PARSE rather than an error.
//!
//! These tests pin the discrimination in both directions. They are the standing
//! guard for the next field added to either body: widen `Create` or `Splice`
//! carelessly and the round-trip below stops resolving to the variant it names.

use serde_json::json;
use wire::{NodeRev, Path, ResponseBody, Root};

fn create_body() -> ResponseBody {
    ResponseBody::Create {
        path: Path("notes/newborn.md".into()),
        file_rev_after: NodeRev("f3c6d9b647936581".into()),
        root_before: Root("b3:aaa".into()),
        root_after: Some(Root("b3:bbb".into())),
        seq: Some(1),
        dry: None,
        verdicts: Vec::new(),
    }
}

/// The birth body round-trips to ITSELF — no earlier variant captures it.
#[test]
fn the_create_body_round_trips_as_create() {
    let wire_bytes = serde_json::to_value(create_body()).expect("serializes");
    let back: ResponseBody = serde_json::from_value(wire_bytes.clone()).expect("round-trips");
    assert!(
        matches!(back, ResponseBody::Create { .. }),
        "the birth frame resolved to another variant — untagged capture: {wire_bytes}"
    );
    assert_eq!(
        serde_json::to_value(&back).expect("re-serializes"),
        wire_bytes,
        "the round-trip is byte-stable"
    );
}

/// A dry birth's `root_after: null` is CONTRACTUAL (the same absence-vs-null
/// rule `splice` carries), so it must survive the round trip as null rather
/// than vanishing — and must still resolve to `Create`.
#[test]
fn a_dry_birth_keeps_its_null_root_after() {
    let dry = ResponseBody::Create {
        path: Path("rehearsal.md".into()),
        file_rev_after: NodeRev("a5172fcd1c0ce8fb".into()),
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
    assert!(matches!(back, ResponseBody::Create { .. }));
}

/// The other direction: `Create` must not capture a `splice` frame. `armed` is
/// what makes a splice a splice, and `Create` has no such field — so a splice
/// body must still resolve to `Splice` now that `Create` sits in the enum.
#[test]
fn the_create_body_does_not_capture_a_splice_frame() {
    let splice = json!({
        "armed": {"path": "notes/plan.md", "file_rev_after": "f3c6d9b647936581", "edits": []},
        "root_before": "b3:aaa",
        "root_after": "b3:bbb",
        "seq": 1,
        "verdicts": []
    });
    let back: ResponseBody = serde_json::from_value(splice.clone()).expect("splice decodes");
    assert!(
        matches!(back, ResponseBody::Splice { .. }),
        "the splice frame was captured by another variant: {splice}"
    );
}

/// And `Create` must not capture the read-plane frames that also carry `path`.
/// `Toc` is the close one — same `path`, a rev, and a root — separated only by
/// `nodes` vs `file_rev_after`.
#[test]
fn neither_toc_nor_create_captures_the_other() {
    let toc = json!({"path": "a.md", "file_rev": "f3c6d9b647936581",
                     "root": "b3:aaa", "nodes": []});
    let back: ResponseBody = serde_json::from_value(toc.clone()).expect("toc decodes");
    assert!(
        matches!(back, ResponseBody::Toc { .. }),
        "the toc frame was captured by the birth body: {toc}"
    );

    let birth = serde_json::to_value(create_body()).expect("serializes");
    let back: ResponseBody = serde_json::from_value(birth.clone()).expect("create decodes");
    assert!(
        matches!(back, ResponseBody::Create { .. }),
        "the birth frame was captured by a read body: {birth}"
    );
}

// ---------------------------------------------------------------------------
// The 2.3 correction's gate: the journal removal is a v3 RESPONSE-shape change
// ---------------------------------------------------------------------------

/// **`wire::Delta` is UNCHANGED by the journal removal — asserted, not assumed.**
///
/// The obvious reading of "drop `journal_anchor`" is that it is a `Delta` field.
/// It is not: `journal_anchor` lived on `ResponseBody::Create` (a v3 response
/// body) and on the three `wire_serve::write` outcome structs. `Delta` never
/// carried it, so removing it touches the v3 response shape and leaves the FROZEN
/// v2 notification surface alone.
///
/// This test is what makes that a verified claim rather than a reading. It pins
/// `Delta`'s serialized key set exactly: a key added to or removed from `Delta`
/// fails here, which is the alarm that a journal-shaped edit reached frozen v2.
#[test]
fn the_delta_shape_is_untouched_by_the_journal_removal() {
    let delta = wire::Delta {
        seq: 7,
        root_before: Root("b3:aaa".into()),
        root_after: Root("b3:bbb".into()),
        actor: Some("agent:alice".into()),
        now: Some("2026-08-03T00:00:00Z".into()),
        files: Vec::new(),
    };
    let v = serde_json::to_value(&delta).expect("Delta serializes");
    let obj = v.as_object().expect("Delta is a JSON object");

    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["actor", "files", "now", "root_after", "root_before", "seq"],
        "frozen v2 Delta carries exactly these keys: {v}"
    );
    assert!(
        obj.keys().all(|k| !k.contains("journal")),
        "no journal-shaped key ever rode Delta: {v}"
    );
}
