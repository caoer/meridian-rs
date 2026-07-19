//! W2-AMEND contract fixtures, executable against the FROZEN text
//! (`18-02-meridian-rs/results/wire-contract-v2.md`, decision 014):
//! hpath dual-serialization + deviation row, the D-C5 discrimination fixture,
//! the §8 recovery-binding table, and the worked §4.1/§4.2/§4.5/§5.2 frames
//! asserted value-for-value (byte-exact dispatch fixtures are D2-DISPATCH's;
//! this file pins the SHAPES the types serialize to).

use serde_json::{Value, json};

fn seg(h: &str) -> wire::HpathSeg {
    wire::HpathSeg {
        h: h.into(),
        n: None,
    }
}

// ---------------------------------------------------------------------------
// gate 2 — hpath dual-serialization + deviation row
// ---------------------------------------------------------------------------

/// v2 §2.1: `{"h":"Goals"}` ≡ the v1 bare string on the way IN; the object
/// form is the only form on the way OUT.
#[test]
fn hpath_dual_deserialization_both_forms_one_value() {
    let v1_form: Vec<wire::HpathSeg> = serde_json::from_value(json!(["Goals", "Q3"])).unwrap();
    let v2_form: Vec<wire::HpathSeg> =
        serde_json::from_value(json!([{"h":"Goals"}, {"h":"Q3"}])).unwrap();
    assert_eq!(v1_form, v2_form);
    assert_eq!(v1_form, vec![seg("Goals"), seg("Q3")]);

    // the occurrence index rides only the object form (v2 §2.1, 1-based)
    let with_n: Vec<wire::HpathSeg> = serde_json::from_value(json!([{"h":"Beta","n":2}])).unwrap();
    assert_eq!(
        with_n,
        vec![wire::HpathSeg {
            h: "Beta".into(),
            n: Some(2)
        }]
    );

    // serialization is the object form, and ONLY the object form
    assert_eq!(
        serde_json::to_value(&v1_form).unwrap(),
        json!([{"h":"Goals"}, {"h":"Q3"}])
    );
}

/// The deviation row, executable: the one v2 touch on the FROZEN node object.
/// A v1-dialect consumer (`hpath: Vec<String>`) FAILS on v2 bytes — loud,
/// never a silent reinterpretation.
#[test]
fn hpath_deviation_row_v1_dialect_fails_loud() {
    let v2_bytes = json!([{"h":"Goals"}, {"h":"Q3"}]);
    assert!(serde_json::from_value::<Vec<String>>(v2_bytes).is_err());
}

// ---------------------------------------------------------------------------
// gate 3 — D-C5 discrimination fixture
// ---------------------------------------------------------------------------

/// v2 §4.3 (D-C5): unknown `kinds` → `bad_request{unknown_kinds}`, loud —
/// reversing v1's "unknown names match nothing". A v1-dialect client (flat
/// `"error":"bad_request"` string envelope) FAILS to decode the v2 frame.
#[test]
fn dc5_unknown_kinds_refusal_discriminates_v1_dialect() {
    // v1 dialect: `error` was the code STRING at top level. Its decode of the
    // v2 frame must fail loud instead of silently matching nothing.
    #[derive(serde::Deserialize)]
    struct V1ErrorEnvelope {
        #[allow(dead_code)]
        error: String,
    }

    let mut error = wire::ErrorBody::new(wire::ErrorCode::BadRequest);
    error.unknown_kinds = Some(vec!["headding".into()]);
    let frame = wire::Response {
        id: Some(9),
        ok: false,
        payload: wire::ResponsePayload::Error { error },
    };
    let v2 = serde_json::to_value(&frame).unwrap();
    assert_eq!(
        v2,
        json!({"id":9,"ok":false,"error":{
            "code":"bad_request","recovery":"fix","unknown_kinds":["headding"]}})
    );

    assert!(serde_json::from_value::<V1ErrorEnvelope>(v2).is_err());
}

// ---------------------------------------------------------------------------
// gate 4 — §8 recovery bindings, verbatim, on every frame from birth
// ---------------------------------------------------------------------------

/// The frozen §8 table for every code present at this rung, including the two
/// DECLARED rebinds (`unsupported_proto` fix→respawn now; `root_mismatch`
/// →resync joins with W3-AMEND) — §18 ledger row 4.
#[test]
fn recovery_bindings_match_frozen_table() {
    use wire::{ErrorCode as C, Recovery as R};
    let table = [
        (C::BadRequest, R::Fix),
        (C::UnknownOp, R::Fix),
        (C::BadPath, R::Fix),
        (C::AmbiguousRef, R::Fix),
        (C::NotFound, R::Env), // v1 code, retired at W4-AMEND; env until then
        (C::InvalidUtf8, R::Env),
        (C::CasMismatch, R::Refresh),
        (C::RefNotFound, R::Refresh),
        (C::RootMismatch, R::Resync), // the declared rebind (was refresh)
        (C::RootUnknown, R::Resync),
        (C::BadFrame, R::Respawn),
        (C::UnsupportedProto, R::Respawn), // the declared rebind (was fix)
        (C::Internal, R::Respawn),
    ];
    for (code, class) in table {
        assert_eq!(code.recovery(), class, "{code:?}");
        // the constructor carries the binding — no frame without recovery
        assert_eq!(wire::ErrorBody::new(code).recovery, class);
    }
}

// ---------------------------------------------------------------------------
// the worked frames, value-for-value from the FROZEN text
// ---------------------------------------------------------------------------

/// v2 §4.1, the worked ANCHOR toc row (the late-law addition): the block
/// echoes as its HOST kind (`list_item`, outside the closed extract enum)
/// keyed by its `anchor` ref, carrying its own rev; the lone top-level
/// heading's rev equals `file_rev`.
#[test]
fn worked_anchor_toc_frame_matches_contract() {
    let frame = wire::Response {
        id: Some(4),
        ok: true,
        payload: wire::ResponsePayload::Body {
            body: wire::ResponseBody::Toc {
                path: wire::Path("receipts/2026-07-18.md".into()),
                file_rev: wire::NodeRev("2731acfa39bbb92c".into()),
                root: wire::Root(
                    "b3:10769ae1c77f5646750f3f52df2d055156b411145a02b8361ecd32af1357a1b7".into(),
                ),
                nodes: vec![
                    wire::TocNode {
                        kind: "heading".into(),
                        level: Some(1),
                        hpath: Some(vec![seg("Receipts — 2026-07-18")]),
                        anchor: None,
                        span: wire::Span(0, 249),
                        content_span: Some(wire::Span(26, 249)),
                        node_rev: wire::NodeRev("2731acfa39bbb92c".into()),
                        text_prefix_16b: "# Receipts — 2".into(),
                        keys: None,
                    },
                    wire::TocNode {
                        kind: "list_item".into(),
                        level: None,
                        hpath: None,
                        anchor: Some("r-000042".into()),
                        span: wire::Span(26, 248),
                        content_span: None,
                        node_rev: wire::NodeRev("639a2dca46f6fcc8".into()),
                        text_prefix_16b: "- splice notes/p".into(),
                        keys: None,
                    },
                ],
            },
        },
    };
    let expected: Value = json!({
        "id":4,"ok":true,"body":{
            "path":"receipts/2026-07-18.md","file_rev":"2731acfa39bbb92c",
            "root":"b3:10769ae1c77f5646750f3f52df2d055156b411145a02b8361ecd32af1357a1b7",
            "nodes":[
                {"kind":"heading","level":1,"hpath":[{"h":"Receipts — 2026-07-18"}],
                 "span":[0,249],"content_span":[26,249],
                 "node_rev":"2731acfa39bbb92c","text_prefix_16b":"# Receipts — 2"},
                {"kind":"list_item","anchor":"r-000042","span":[26,248],
                 "node_rev":"639a2dca46f6fcc8","text_prefix_16b":"- splice notes/p"}]}
    });
    assert_eq!(serde_json::to_value(&frame).unwrap(), expected);
    // and the frame reads back into the same typed value
    assert_eq!(
        serde_json::from_value::<wire::Response>(expected).unwrap(),
        frame
    );
}

/// v2 §4.2: what you read is exactly what is hashed.
#[test]
fn worked_cat_frame_matches_contract() {
    let request: wire::Request = serde_json::from_value(json!({
        "id":3,"op":"cat","path":"notes/plan.md",
        "sec":{"hpath":[{"h":"Goals"},{"h":"Q3"}]}
    }))
    .unwrap();
    assert_eq!(
        request.op,
        wire::Op::Cat {
            path: wire::Path("notes/plan.md".into()),
            sec: Some(wire::SecRef::Hpath {
                hpath: vec![seg("Goals"), seg("Q3")]
            }),
        }
    );

    let frame = wire::Response {
        id: Some(3),
        ok: true,
        payload: wire::ResponsePayload::Body {
            body: wire::ResponseBody::Cat {
                span: wire::Span(49, 72),
                node_rev: wire::NodeRev("33d5b0e1b27cb48b".into()),
                content: "## Q3\n\nship by August\n\n".into(),
            },
        },
    };
    assert_eq!(
        serde_json::to_value(&frame).unwrap(),
        json!({"id":3,"ok":true,"body":{"span":[49,72],"node_rev":"33d5b0e1b27cb48b",
            "content":"## Q3\n\nship by August\n\n"}})
    );
}

/// v2 §4.5: the walk plane's response has NO rev field (D-C2); `dest` rides
/// every stage-2 outcome, success or failure; stage 1 misses carry no dest.
#[test]
fn worked_resolve_frames_match_contract() {
    let ok = wire::Response {
        id: Some(70),
        ok: true,
        payload: wire::ResponsePayload::Body {
            body: wire::ResponseBody::Resolve {
                dest: wire::Path("notes/plan.md".into()),
                span: wire::Span(49, 75),
                content: None,
            },
        },
    };
    assert_eq!(
        serde_json::to_value(&ok).unwrap(),
        json!({"id":70,"ok":true,"body":{"dest":"notes/plan.md","span":[49,75]}})
    );

    let mut stage2 = wire::ErrorBody::new(wire::ErrorCode::RefNotFound);
    stage2.stage = Some(2);
    stage2.dest = Some(wire::Path("notes/plan.md".into()));
    let miss2 = wire::Response {
        id: Some(73),
        ok: false,
        payload: wire::ResponsePayload::Error { error: stage2 },
    };
    assert_eq!(
        serde_json::to_value(&miss2).unwrap(),
        json!({"id":73,"ok":false,"error":{"code":"ref_not_found","recovery":"refresh",
            "stage":2,"dest":"notes/plan.md"}})
    );

    let mut stage1 = wire::ErrorBody::new(wire::ErrorCode::RefNotFound);
    stage1.stage = Some(1);
    let miss1 = wire::Response {
        id: Some(74),
        ok: false,
        payload: wire::ResponsePayload::Error { error: stage1 },
    };
    assert_eq!(
        serde_json::to_value(&miss1).unwrap(),
        json!({"id":74,"ok":false,"error":{"code":"ref_not_found","recovery":"refresh","stage":1}})
    );
}

/// v2 §5.2: the failure split — `cas_mismatch` carries refresh + both revs.
#[test]
fn worked_cas_mismatch_frame_matches_contract() {
    let mut error = wire::ErrorBody::new(wire::ErrorCode::CasMismatch);
    error.expected = Some(wire::NodeRev("33d5b0e1b27cb48b".into()));
    error.actual = Some(wire::NodeRev("41f643f034e5681f".into()));
    let frame = wire::Response {
        id: Some(88),
        ok: false,
        payload: wire::ResponsePayload::Error { error },
    };
    let expected = json!({"id":88,"ok":false,"error":{"code":"cas_mismatch","recovery":"refresh",
        "expected":"33d5b0e1b27cb48b","actual":"41f643f034e5681f"}});
    assert_eq!(serde_json::to_value(&frame).unwrap(), expected);
    assert_eq!(
        serde_json::from_value::<wire::Response>(expected).unwrap(),
        frame
    );
}

// ---------------------------------------------------------------------------
// the §2.1 grammar — three forms, no other
// ---------------------------------------------------------------------------

#[test]
fn sec_ref_three_mint_forms_roundtrip() {
    let cases = [
        (
            wire::SecRef::Hpath {
                hpath: vec![
                    seg("Goals"),
                    wire::HpathSeg {
                        h: "Beta".into(),
                        n: Some(2),
                    },
                ],
            },
            json!({"hpath":[{"h":"Goals"},{"h":"Beta","n":2}]}),
        ),
        (
            wire::SecRef::Anchor {
                anchor: "r-000042".into(),
            },
            json!({"anchor":"r-000042"}),
        ),
        (
            wire::SecRef::FmKey {
                fm_key: "title".into(),
            },
            json!({"fm_key":"title"}),
        ),
    ];
    for (typed, wire_json) in cases {
        assert_eq!(serde_json::to_value(&typed).unwrap(), wire_json);
        assert_eq!(
            serde_json::from_value::<wire::SecRef>(wire_json).unwrap(),
            typed
        );
    }
}

// ---------------------------------------------------------------------------
// W3-AMEND — §4.7 root/diff + the root_mismatch scope-deviation fixture
// ---------------------------------------------------------------------------

/// v2 §4.7 worked: `{"op":"root"}` takes no parameters; the response carries
/// root + seq.
#[test]
fn worked_root_frames_match_contract() {
    let request: wire::Request = serde_json::from_value(json!({"id":90,"op":"root"})).unwrap();
    assert_eq!(request.op, wire::Op::Root);
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        json!({"id":90,"op":"root"})
    );

    let frame = wire::Response {
        id: Some(90),
        ok: true,
        payload: wire::ResponsePayload::Body {
            body: wire::ResponseBody::Root {
                root: wire::Root(
                    "b3:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68".into(),
                ),
                seq: 2,
            },
        },
    };
    assert_eq!(
        serde_json::to_value(&frame).unwrap(),
        json!({"id":90,"ok":true,"body":{
            "root":"b3:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68",
            "seq":2}})
    );
}

/// v2 §4.7 worked: the diff REQUEST shape, frozen now (the Delta-bearing
/// response body lands with the Delta noun, D3-DELTA).
#[test]
fn worked_diff_request_matches_contract() {
    let request: wire::Request = serde_json::from_value(json!({
        "id":95,"op":"diff",
        "from_root":"b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9",
        "to_root":"b3:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68"
    }))
    .unwrap();
    assert_eq!(
        request.op,
        wire::Op::Diff {
            from_root: wire::Root(
                "b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9".into()
            ),
            to_root: wire::Root(
                "b3:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68".into()
            ),
        }
    );
}

/// Ledger flag 2, the deviation row EXECUTABLE (§18 row 2, WAIVED): the
/// repo's reserved shape carried `expected/actual/scope/changed`; the frozen
/// contract ships `{expected,actual,changed}` — NO `scope` key, discriminated
/// here so the drop can never regress silently. A v1-dialect reader expecting
/// `scope` finds the key absent.
#[test]
fn root_mismatch_scope_drop_deviation_fixture() {
    let mut error = wire::ErrorBody::new(wire::ErrorCode::RootMismatch);
    error.expected = Some(wire::NodeRev(
        "b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9".into(),
    ));
    error.actual = Some(wire::NodeRev(
        "b3:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68".into(),
    ));
    error.changed = Some(vec![wire::Path("notes/plan.md".into())]);
    let frame = wire::Response {
        id: Some(96),
        ok: false,
        payload: wire::ResponsePayload::Error { error },
    };
    let v = serde_json::to_value(&frame).unwrap();
    assert_eq!(
        v,
        json!({"id":96,"ok":false,"error":{
            "code":"root_mismatch","recovery":"resync",
            "expected":"b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9",
            "actual":"b3:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68",
            "changed":["notes/plan.md"]}})
    );
    // field-for-field: exactly the frozen keys, scope ABSENT
    let keys: Vec<&str> = v["error"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert!(!keys.contains(&"scope"));
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    assert_eq!(
        sorted,
        ["actual", "changed", "code", "expected", "recovery"]
    );
}
