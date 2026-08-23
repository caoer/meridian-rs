//! `ResponseBody` is an untagged enum: field shape alone discriminates on the
//! wire, so a new or widened variant can silently capture another op's frame —
//! and the failure is a mis-parse, not an error. These tests pin the
//! discrimination in both directions.

use serde_json::json;
use wire::{ErrorBody, ErrorCode, NodeRev, Path, ResponseBody, Root, ABSENT_REV};

fn create_body() -> ResponseBody {
    ResponseBody::Create {
        path: Path("notes/newborn.md".into()),
        file_rev_after: NodeRev("f3c6d9b647936581".into()),
        root_before: Root("b3:aaa".into()),
        root_after: Some(Root("b3:bbb".into())),
        seq: Some(1),
        dry: None,
        verdicts: Vec::new(),
        intents: None,
    }
}

/// The birth body round-trips to itself — no earlier variant captures it.
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

/// A dry birth's `root_after` is contractually null, not absent (the same
/// absence-vs-null rule `splice` carries), and still resolves to `Create`.
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
        intents: None,
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

/// `Create` must not capture a `splice` frame — `armed` is the discriminator,
/// and `Create` has no such field.
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

/// `Toc` is the closest read-plane frame — same `path`, a rev and a root,
/// separated only by `nodes` vs `file_rev_after`.
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
// The occupancy discriminator: which `cas_mismatch` means "already there"
// ---------------------------------------------------------------------------

/// A `cas_mismatch` frame with the given `expected`, as `wire_serve` mints one.
fn cas_mismatch(expected: Option<&str>, actual: &str) -> ErrorBody {
    let mut e = ErrorBody::new(ErrorCode::CasMismatch);
    e.expected = expected.map(|r| NodeRev(r.into()));
    e.actual = Some(NodeRev(actual.into()));
    e
}

/// The create-CAS — `expected` is the empty document's rev — IS the occupancy
/// finding. This is the ONE frame a fill-if-absent caller may read as benign.
#[test]
fn the_create_cas_is_the_occupancy_finding() {
    let occupied = cas_mismatch(Some(ABSENT_REV), "db0a7db4e8635f23");
    assert!(
        occupied.is_path_occupied(),
        "the create door's refusal must read as occupancy: {occupied:?}"
    );
}

/// The negative: a `cas_mismatch` whose `expected` is a REAL rev is the
/// drift/remove-CAS — "the file moved under your plan" — and must NOT read as
/// benign-exists. Keying on `code` alone (what every consumer did before this
/// discriminator existed) turns this frame into a silent "it was already
/// there", reporting a birth that never happened.
#[test]
fn a_non_create_cas_mismatch_is_not_occupancy() {
    let drift = cas_mismatch(Some("0000000000000000"), "db0a7db4e8635f23");
    assert_eq!(
        drift.code,
        ErrorCode::CasMismatch,
        "the fixture is genuinely the same code — that is the whole hazard"
    );
    assert!(
        !drift.is_path_occupied(),
        "a drift/remove-CAS refusal read as benign already-exists: {drift:?}"
    );
}

/// The guard plane's `AlreadyBorn` carries NO `expected` at all. It is a benign
/// already-exists, but it is a splice-path refusal the create door never mints,
/// so the discriminator reads it false — fail-closed, an error rather than a
/// silent "already there".
#[test]
fn a_cas_mismatch_without_expected_is_not_occupancy() {
    let already_born = cas_mismatch(None, "db0a7db4e8635f23");
    assert!(
        !already_born.is_path_occupied(),
        "an `expected`-less cas_mismatch must not read as the create-door CAS: {already_born:?}"
    );
}

/// Only `cas_mismatch` can be occupancy — the `expected`/`actual` pair also
/// rides `root_mismatch`, and the code gates the comparison.
#[test]
fn only_a_cas_mismatch_can_be_occupancy() {
    let mut root_mismatch = ErrorBody::new(ErrorCode::RootMismatch);
    root_mismatch.expected = Some(NodeRev(ABSENT_REV.into()));
    assert!(
        !root_mismatch.is_path_occupied(),
        "another code carrying the absent token must not read as occupancy"
    );
}

// ---------------------------------------------------------------------------
// The journal removal is a v3 response-shape change
// ---------------------------------------------------------------------------

/// `journal_anchor` lived on `ResponseBody::Create` and the `wire_serve::write`
/// outcome structs, never on `Delta` — so removing it leaves the frozen v2
/// notification surface alone. This pins `Delta`'s key set exactly, so a
/// journal-shaped edit reaching frozen v2 fails here.
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
