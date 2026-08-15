//! Contract fixtures, executable against the frozen text
//! (`docs/wire-contract.md`): hpath serialization + deviation row, the
//! unknown-kinds discrimination fixture, the §8 recovery-binding table, and the
//! worked §4.1/§4.2/§4.5/§5.2 frames asserted value-for-value. This file pins
//! the shapes the types serialize to, not byte-exact dispatch.

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

/// v2 §2.1 as amended (decision 20): the object form is the only form in both
/// directions. The retired v1 bare string is refused loud, with its message
/// asserted so a restored tolerance breaks this test rather than passing
/// silently.
#[test]
fn hpath_object_form_only_both_directions_v1_bare_string_refused() {
    // IN: the object form, with and without the occurrence index (1-based).
    let v2_form: Vec<wire::HpathSeg> =
        serde_json::from_value(json!([{"h":"Goals"}, {"h":"Q3"}])).unwrap();
    assert_eq!(v2_form, vec![seg("Goals"), seg("Q3")]);
    let with_n: Vec<wire::HpathSeg> = serde_json::from_value(json!([{"h":"Beta","n":2}])).unwrap();
    assert_eq!(
        with_n,
        vec![wire::HpathSeg {
            h: "Beta".into(),
            n: Some(2)
        }]
    );

    // IN: the v1 bare string is refused — message text asserted.
    let e = serde_json::from_value::<Vec<wire::HpathSeg>>(json!(["Goals", "Q3"]))
        .expect_err("the retired v1 bare-string segment must be refused");
    assert!(
        e.to_string().contains(wire::HPATH_SEG_V1_REFUSAL),
        "refusal must name the retired v1 form, got: {e}"
    );

    // The bare form cannot carry `n` at all, so the dual grammar was strictly
    // less expressive.
    assert!(
        serde_json::from_value::<Vec<wire::HpathSeg>>(json!(["Beta"])).is_err(),
        "no bare spelling of {{h:Beta,n:2}} exists — that is why the dual grammar died"
    );

    // OUT: serialization is the object form only.
    assert_eq!(
        serde_json::to_value(&v2_form).unwrap(),
        json!([{"h":"Goals"}, {"h":"Q3"}])
    );
}

/// The deviation row: a v1-dialect consumer (`hpath: Vec<String>`) fails loud
/// on v2 bytes, never a silent reinterpretation.
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
/// `"error":"bad_request"` string envelope) fails to decode the v2 frame.
#[test]
fn dc5_unknown_kinds_refusal_discriminates_v1_dialect() {
    // v1 dialect: `error` was the code string at top level.
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

/// The frozen §8 table for every code, including the two declared rebinds
/// (`unsupported_proto` fix→respawn, `root_mismatch` refresh→resync).
#[test]
fn recovery_bindings_match_frozen_table() {
    use wire::{ErrorCode as C, Recovery as R};
    let table = [
        (C::BadRequest, R::Fix),
        (C::UnknownOp, R::Fix),
        (C::BadPath, R::Fix),
        (C::NoMatch, R::Fix),
        (C::NotUnique, R::Fix),
        (C::WouldCorrupt, R::Fix),
        (C::AmbiguousRef, R::Fix),
        (C::FileNotFound, R::Env),
        (C::IoError, R::Env),
        (C::InvalidUtf8, R::Env),
        (C::CasMismatch, R::Refresh),
        (C::RefNotFound, R::Refresh),
        (C::RootMismatch, R::Resync), // the declared rebind (was refresh)
        (C::RootUnknown, R::Resync),
        (C::LockTimeout, R::Retry),
        (C::StaleView, R::Retry), // §10.2: retryable, never silent (Q5-LINKS)
        (C::BadFrame, R::Respawn),
        (C::UnsupportedProto, R::Respawn), // the declared rebind (was fix)
        (C::Internal, R::Respawn),
        // Refusal-amendment codes, additive by the tolerant-code law — each
        // statically bound to one existing class.
        (C::ConventionFault, R::Env), // row 6: fail-closed on the armed law
        (C::ArmedDrift, R::Refresh),  // row 7: report-rev ≠ armed-rev
        (C::CorpusRace, R::Retry),    // benign mid-request race: reap/delete
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

/// v2 §4.1, the worked anchor toc row: the block echoes as its host kind
/// (`list_item`, outside the closed extract enum) keyed by its `anchor` ref,
/// carrying its own rev; the lone top-level heading's rev equals `file_rev`.
#[test]
fn worked_anchor_toc_frame_matches_contract() {
    let frame = wire::Response {
        id: Some(4),
        ok: true,
        payload: wire::ResponsePayload::Body {
            body: wire::ResponseBody::Toc {
                path: wire::Path("receipts/2026-07-18.md".into()),
                file_rev: wire::NodeRev("51ad6428f5b5a898".into()),
                root: wire::Root(
                    "b3:7f3b44376c719be236279e168c22fa2f4d346cd6e5da5bcf0784adb72e7c1f12".into(),
                ),
                nodes: vec![
                    wire::TocNode {
                        kind: "heading".into(),
                        level: Some(1),
                        hpath: Some(vec![seg("Receipts — 2026-07-18")]),
                        anchor: None,
                        span: wire::Span(0, 287),
                        content_span: Some(wire::Span(26, 287)),
                        node_rev: wire::NodeRev("51ad6428f5b5a898".into()),
                        text_prefix_16b: "# Receipts — 2".into(),
                        keys: None,
                    },
                    wire::TocNode {
                        kind: "list_item".into(),
                        level: None,
                        hpath: None,
                        anchor: Some("r-000042".into()),
                        span: wire::Span(26, 286),
                        content_span: None,
                        node_rev: wire::NodeRev("60bbee70d4a63a48".into()),
                        text_prefix_16b: "- splice notes/p".into(),
                        keys: None,
                    },
                ],
            },
        },
    };
    let expected: Value = json!({
        "id":4,"ok":true,"body":{
            "path":"receipts/2026-07-18.md","file_rev":"51ad6428f5b5a898",
            "root":"b3:7f3b44376c719be236279e168c22fa2f4d346cd6e5da5bcf0784adb72e7c1f12",
            "nodes":[
                {"kind":"heading","level":1,"hpath":[{"h":"Receipts — 2026-07-18"}],
                 "span":[0,287],"content_span":[26,287],
                 "node_rev":"51ad6428f5b5a898","text_prefix_16b":"# Receipts — 2"},
                {"kind":"list_item","anchor":"r-000042","span":[26,286],
                 "node_rev":"60bbee70d4a63a48","text_prefix_16b":"- splice notes/p"}]}
    });
    assert_eq!(serde_json::to_value(&frame).unwrap(), expected);
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

/// v2 §4.5: the walk plane's response has no rev field (D-C2); `dest` rides
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
                    "b3:6e866e13b5e65ef9961c050f8a621cf1980b00ee293be650deef5f4dbc6823f0".into(),
                ),
                seq: 2,
                tree_instance: None,
            },
        },
    };
    assert_eq!(
        serde_json::to_value(&frame).unwrap(),
        json!({"id":90,"ok":true,"body":{
            "root":"b3:6e866e13b5e65ef9961c050f8a621cf1980b00ee293be650deef5f4dbc6823f0",
            "seq":2}})
    );
}

/// v2 §4.7 worked: the diff request shape.
#[test]
fn worked_diff_request_matches_contract() {
    let request: wire::Request = serde_json::from_value(json!({
        "id":95,"op":"diff",
        "from_root":"b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9",
        "to_root":"b3:6e866e13b5e65ef9961c050f8a621cf1980b00ee293be650deef5f4dbc6823f0"
    }))
    .unwrap();
    assert_eq!(
        request.op,
        wire::Op::Diff {
            from_root: wire::Root(
                "b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9".into()
            ),
            to_root: wire::Root(
                "b3:6e866e13b5e65ef9961c050f8a621cf1980b00ee293be650deef5f4dbc6823f0".into()
            ),
        }
    );
}

/// The deviation row (§18 row 2): the frozen contract ships
/// `{expected,actual}` — no `scope` key, and no `changed` since the
/// 2026-08-10 strike — so a v1-dialect reader expecting either finds it
/// absent.
#[test]
fn root_mismatch_scope_drop_deviation_fixture() {
    let mut error = wire::ErrorBody::new(wire::ErrorCode::RootMismatch);
    error.expected = Some(wire::NodeRev(
        "b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9".into(),
    ));
    error.actual = Some(wire::NodeRev(
        "b3:6e866e13b5e65ef9961c050f8a621cf1980b00ee293be650deef5f4dbc6823f0".into(),
    ));
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
            "actual":"b3:6e866e13b5e65ef9961c050f8a621cf1980b00ee293be650deef5f4dbc6823f0"}})
    );
    // exactly the frozen keys, scope absent
    let keys: Vec<&str> = v["error"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert!(!keys.contains(&"scope"));
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    // `changed` STRUCK 2026-08-10 (§18 row 2; ZT decision 19). The frozen
    // baseline moves AS DATA with its ruling pointer — a freeze is satisfied by
    // a visible ruled diff, never by immutability forever. This fixture was the
    // ONLY thing in the workspace that ever set the field.
    assert_eq!(sorted, ["actual", "code", "expected", "recovery"]);
}

// ---------------------------------------------------------------------------
// W4-AMEND — §4.4 batch splice, receipts (§6), the not_found retirement
// ---------------------------------------------------------------------------

/// v2 §4.4 worked (id 42): the fully-guarded batch request and the armed-facts
/// response — receipts are per-request and never a wire requirement (§6.1),
/// one root advance covering both files (D-C3).
#[test]
fn worked_splice_frames_match_contract() {
    let request: wire::Request = serde_json::from_value(json!({
        "id":42,"op":"splice","path":"notes/plan.md",
        "actor":"agent:b0864fb2","now":"2026-07-18T20:31:04Z",
        "receipt":{"path":"receipts/2026-07-18.md","anchor":"r-000042"},
        "if_root":"b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9",
        "edits":[
            {"target":{"hpath":[{"h":"Goals"},{"h":"Q3"}]},
             "edit":{"match":{"old":"ship by August","new":"ship by September"}},
             "if_node_rev":"33d5b0e1b27cb48b"}]
    }))
    .unwrap();
    let wire::Op::Splice {
        receipt,
        if_root,
        edits,
        ..
    } = &request.op
    else {
        panic!("splice op")
    };
    assert_eq!(
        receipt.as_ref().unwrap().anchor,
        "r-000042",
        "receipt named per-request"
    );
    assert!(if_root.is_some());
    assert_eq!(
        edits[0].edit,
        wire::EditShape::Match {
            old: "ship by August".into(),
            new: "ship by September".into()
        }
    );

    let frame = wire::Response {
        id: Some(42),
        ok: true,
        payload: wire::ResponsePayload::Body {
            body: wire::ResponseBody::Splice {
                armed: wire::Armed {
                    path: wire::Path("notes/plan.md".into()),
                    // the same post-write file rev as E3's delta for
                    // notes/plan.md — one fact, two frames
                    file_rev_after: Some(wire::NodeRev("a9794a262e67ed02".into())),
                    edits: vec![wire::ArmedEdit {
                        target: wire::SecRef::Hpath {
                            hpath: vec![seg("Goals"), seg("Q3")],
                        },
                        node_rev_before: wire::NodeRev("33d5b0e1b27cb48b".into()),
                        node_rev_after: wire::NodeRev("41f643f034e5681f".into()),
                        span_after: wire::Span(49, 75),
                    }],
                    effects: vec![],
                },
                receipt: Some(wire::ReceiptFact {
                    path: wire::Path("receipts/2026-07-18.md".into()),
                    anchor: "r-000042".into(),
                    node_rev: wire::NodeRev("60bbee70d4a63a48".into()),
                    span_after: wire::Span(26, 286),
                }),
                root_before: wire::Root(
                    "b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9".into(),
                ),
                root_after: Some(wire::Root(
                    "b3:7f3b44376c719be236279e168c22fa2f4d346cd6e5da5bcf0784adb72e7c1f12".into(),
                )),
                seq: Some(1),
                dry: None,
                verdicts: vec![],
                // an absent pin serializes away — the frozen v2 response
                // bytes below are the proof
                pin: None,
            },
        },
    };
    assert_eq!(
        serde_json::to_value(&frame).unwrap(),
        json!({"id":42,"ok":true,"body":{
            "armed":{"path":"notes/plan.md","file_rev_after":"a9794a262e67ed02","edits":[
                {"target":{"hpath":[{"h":"Goals"},{"h":"Q3"}]},
                 "node_rev_before":"33d5b0e1b27cb48b","node_rev_after":"41f643f034e5681f",
                 "span_after":[49,75]}]},
            "receipt":{"path":"receipts/2026-07-18.md","anchor":"r-000042",
                       "node_rev":"60bbee70d4a63a48","span_after":[26,286]},
            "root_before":"b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9",
            "root_after":"b3:7f3b44376c719be236279e168c22fa2f4d346cd6e5da5bcf0784adb72e7c1f12",
            "seq":1,"verdicts":[]}})
    );
}

/// v2 §4.4 worked (id 57): `at:end` is raw byte concatenation — the wire
/// carries `text` exactly as given, its own separators and none synthesized —
/// and the request is legally guardless.
#[test]
fn worked_append_at_end_raw_concat_matches_contract() {
    let request: wire::Request = serde_json::from_value(json!({
        "id":57,"op":"splice","path":"notes/plan.md",
        "actor":"agent:b0864fb2","now":"2026-07-18T20:33:41Z",
        "receipt":{"path":"receipts/2026-07-18.md","anchor":"r-000043"},
        "edits":[{"target":{"hpath":[{"h":"Goals"},{"h":"Q4"}]},
                  "edit":{"put":{"at":"end","text":"- new item\n"}}}]
    }))
    .unwrap();
    let wire::Op::Splice { if_root, edits, .. } = &request.op else {
        panic!("splice op")
    };
    assert!(if_root.is_none(), "guardless is legal at the wire forever");
    assert_eq!(
        edits[0].edit,
        wire::EditShape::Put {
            at: wire::PutAt::End,
            text: "- new item\n".into(), // leading/trailing bytes are the caller's
        }
    );
    assert!(edits[0].if_node_rev.is_none());
    assert_eq!(
        serde_json::to_value(&request).unwrap()["edits"][0]["edit"]["put"],
        json!({"at":"end","text":"- new item\n"})
    );
}

/// v2 §4.4 worked (id 60): dry — same response shape, `root_after` literally
/// null (serialized, not skipped), no receipt, no seq, `dry:true`.
#[test]
fn worked_dry_splice_frame_matches_contract() {
    let frame = wire::Response {
        id: Some(60),
        ok: true,
        payload: wire::ResponsePayload::Body {
            body: wire::ResponseBody::Splice {
                armed: wire::Armed {
                    path: wire::Path("notes/plan.md".into()),
                    // dry — no post-write file rev, so the key is skipped
                    file_rev_after: None,
                    edits: vec![wire::ArmedEdit {
                        target: wire::SecRef::FmKey {
                            fm_key: "title".into(),
                        },
                        node_rev_before: wire::NodeRev("fa77480c79a853bc".into()),
                        node_rev_after: wire::NodeRev("fb49e9df2257fab8".into()),
                        span_after: wire::Span(4, 18),
                    }],
                    effects: vec![],
                },
                receipt: None,
                root_before: wire::Root(
                    "b3:6e866e13b5e65ef9961c050f8a621cf1980b00ee293be650deef5f4dbc6823f0".into(),
                ),
                root_after: None,
                seq: None,
                dry: Some(true),
                verdicts: vec![],
                // an absent pin serializes away — the frozen v2 response
                // bytes below are the proof
                pin: None,
            },
        },
    };
    let v = serde_json::to_value(&frame).unwrap();
    assert_eq!(
        v,
        json!({"id":60,"ok":true,"body":{
            "armed":{"path":"notes/plan.md","edits":[
                {"target":{"fm_key":"title"},
                 "node_rev_before":"fa77480c79a853bc","node_rev_after":"fb49e9df2257fab8",
                 "span_after":[4,18]}]},
            "root_before":"b3:6e866e13b5e65ef9961c050f8a621cf1980b00ee293be650deef5f4dbc6823f0",
            "root_after":null,"dry":true,"verdicts":[]}})
    );
    assert!(
        v["body"].as_object().unwrap().contains_key("root_after"),
        "root_after is SERIALIZED null on dry, never skipped"
    );
}

/// v2 §5.2 worked (ids 89/91): the failure split's fix half — `no_match` and
/// `not_unique{matches}`.
#[test]
fn worked_no_match_and_not_unique_frames_match_contract() {
    let mut e = wire::ErrorBody::new(wire::ErrorCode::NoMatch);
    e.matches = Some(0);
    let no_match = wire::Response {
        id: Some(89),
        ok: false,
        payload: wire::ResponsePayload::Error { error: e },
    };
    assert_eq!(
        serde_json::to_value(&no_match).unwrap(),
        json!({"id":89,"ok":false,"error":{"code":"no_match","recovery":"fix","matches":0}})
    );

    let mut e = wire::ErrorBody::new(wire::ErrorCode::NotUnique);
    e.matches = Some(2);
    let not_unique = wire::Response {
        id: Some(91),
        ok: false,
        payload: wire::ResponsePayload::Error { error: e },
    };
    assert_eq!(
        serde_json::to_value(&not_unique).unwrap(),
        json!({"id":91,"ok":false,"error":{"code":"not_unique","recovery":"fix","matches":2}})
    );
}

/// The `not_found` retirement (§18 row 6): the v1 code string no longer
/// parses, and its two successors carry the meanings apart.
#[test]
fn not_found_retirement_deviation_fixture() {
    assert!(serde_json::from_value::<wire::ErrorCode>(json!("not_found")).is_err());
    assert_eq!(
        serde_json::from_value::<wire::ErrorCode>(json!("file_not_found")).unwrap(),
        wire::ErrorCode::FileNotFound
    );
    assert_eq!(
        serde_json::from_value::<wire::ErrorCode>(json!("ref_not_found")).unwrap(),
        wire::ErrorCode::RefNotFound
    );
}

// ---------------------------------------------------------------------------
// W4-ACTOR — §9 actor/now as wire inputs, never ambient
// ---------------------------------------------------------------------------

/// Malformed `now` → `bad_request`. The §9 format law is the
/// [`wire::now_is_rfc3339`] predicate; a dispatcher answers every rejection
/// with the fix-class envelope asserted here.
#[test]
fn malformed_now_is_bad_request() {
    // the contract's own worked values pass
    for valid in [
        "2026-07-18T20:31:04Z",
        "2026-07-18T20:33:41Z",
        "2026-07-18t20:31:04z",
        "2026-07-18T20:31:04.250Z",
        "2026-07-18T20:31:04+08:00",
        "2026-12-31T23:59:60-05:30", // leap second, negative offset
        "2028-02-29T00:00:00Z",      // leap day
    ] {
        assert!(wire::now_is_rfc3339(valid), "{valid} must validate");
    }
    for malformed in [
        "yesterday",
        "2026-07-18 20:31:04Z",     // space, not T
        "2026-07-18T20:31:04",      // no offset
        "2026-13-01T00:00:00Z",     // month 13
        "2026-02-30T00:00:00Z",     // Feb 30
        "2027-02-29T00:00:00Z",     // non-leap Feb 29
        "2026-07-18T24:00:00Z",     // hour 24
        "2026-07-18T20:31:04.Z",    // empty fraction
        "2026-07-18T20:31:04+8:00", // one-digit offset hour
        "2026-07-18T20:31:04Z extra",
        "1721334664", // unix seconds
    ] {
        assert!(!wire::now_is_rfc3339(malformed), "{malformed} must refuse");
        // the refusal a dispatcher answers with: bad_request, fix class
        let e = wire::ErrorBody::new(wire::ErrorCode::BadRequest);
        assert_eq!(e.recovery, wire::Recovery::Fix);
    }
}

/// Absent inputs produce absent facts — a splice frame without `actor`/`now`
/// serializes without those keys and reads back to `None`, so the engine
/// records nothing it wasn't told (§9). Receipt-side twin: the `receipt`
/// crate's `absent_inputs_render_absent_facts`.
#[test]
fn absent_actor_now_absent_on_the_wire() {
    let request = wire::Request {
        id: Some(7),
        op: wire::Op::Splice {
            path: wire::Path("notes/plan.md".into()),
            actor: None,
            now: None,
            receipt: None,
            if_root: None,
            dry: None,
            force: None,
            edits: vec![wire::Edit {
                target: wire::SecRef::FmKey {
                    fm_key: "title".into(),
                },
                edit: wire::EditShape::Match {
                    old: "Plan".into(),
                    new: "Plan v2".into(),
                },
                if_node_rev: None,
            }],
            // empty plan_edits serializes away, keeping the frozen v2 request
            // bytes this test pins byte-identical
            plan_edits: Vec::new(),
            // same law for `pin` — absent, so it never reaches the wire
            pin: None,
        },
    };
    let v = serde_json::to_value(&request).unwrap();
    let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    assert!(!keys.contains(&"actor"), "absent actor: no key on the wire");
    assert!(!keys.contains(&"now"), "absent now: no key on the wire");
    let back: wire::Request = serde_json::from_value(v).unwrap();
    assert_eq!(back, request);
}

// ---------------------------------------------------------------------------
// D3-DELTA — the §7.1 worked delta frames + §4.7/§7.3 replay ≡ live
// ---------------------------------------------------------------------------

/// E3's delta, every value from the frozen §7.1 frame (node-grain at birth,
/// decision 012 — no `keys` slot exists to serialize).
fn e3_delta() -> wire::DeltaFrame {
    wire::DeltaFrame {
        effects: vec![],
        rescope: None,
        overflow: None,
        delta: wire::Delta {
            seq: 1,
            root_before: wire::Root(
                "b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9".into(),
            ),
            root_after: wire::Root(
                "b3:7f3b44376c719be236279e168c22fa2f4d346cd6e5da5bcf0784adb72e7c1f12".into(),
            ),
            actor: Some("agent:b0864fb2".into()),
            now: Some("2026-07-18T20:31:04Z".into()),
            files: vec![
                wire::DeltaFile {
                    path: wire::Path("notes/plan.md".into()),
                    change: wire::FileChange::Modified,
                    from_path: None,
                    file_rev_before: Some(wire::NodeRev("e3c4acaceb75b907".into())),
                    file_rev_after: Some(wire::NodeRev("a9794a262e67ed02".into())),
                    nodes: vec![wire::DeltaNode {
                        target: wire::SecRef::Hpath {
                            hpath: vec![seg("Goals"), seg("Q3")],
                        },
                        change: wire::NodeChange::Edited,
                        node_rev_before: Some(wire::NodeRev("33d5b0e1b27cb48b".into())),
                        node_rev_after: Some(wire::NodeRev("41f643f034e5681f".into())),
                        span_after: Some(wire::Span(49, 75)),
                    }],
                },
                wire::DeltaFile {
                    path: wire::Path("receipts/2026-07-18.md".into()),
                    change: wire::FileChange::Modified,
                    from_path: None,
                    file_rev_before: Some(wire::NodeRev("920a40c4ee23d37c".into())),
                    file_rev_after: Some(wire::NodeRev("51ad6428f5b5a898".into())),
                    nodes: vec![wire::DeltaNode {
                        target: wire::SecRef::Anchor {
                            anchor: "r-000042".into(),
                        },
                        change: wire::NodeChange::Added,
                        node_rev_before: None,
                        node_rev_after: Some(wire::NodeRev("60bbee70d4a63a48".into())),
                        span_after: Some(wire::Span(26, 286)),
                    }],
                },
            ],
        },
    }
}

fn e3_delta_json() -> Value {
    json!({"delta":{
     "seq":1,
     "root_before":"b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9",
     "root_after":"b3:7f3b44376c719be236279e168c22fa2f4d346cd6e5da5bcf0784adb72e7c1f12",
     "actor":"agent:b0864fb2","now":"2026-07-18T20:31:04Z",
     "files":[
      {"path":"notes/plan.md","change":"modified",
       "file_rev_before":"e3c4acaceb75b907","file_rev_after":"a9794a262e67ed02",
       "nodes":[{"hpath":[{"h":"Goals"},{"h":"Q3"}],"change":"edited",
                 "node_rev_before":"33d5b0e1b27cb48b","node_rev_after":"41f643f034e5681f",
                 "span_after":[49,75]}]},
      {"path":"receipts/2026-07-18.md","change":"modified",
       "file_rev_before":"920a40c4ee23d37c","file_rev_after":"51ad6428f5b5a898",
       "nodes":[{"anchor":"r-000042","change":"added",
                 "node_rev_after":"60bbee70d4a63a48","span_after":[26,286]}]}]}})
}

/// E4's delta, every value from the frozen §7.1 frame.
fn e4_delta() -> wire::DeltaFrame {
    wire::DeltaFrame {
        effects: vec![],
        rescope: None,
        overflow: None,
        delta: wire::Delta {
            seq: 2,
            root_before: wire::Root(
                "b3:7f3b44376c719be236279e168c22fa2f4d346cd6e5da5bcf0784adb72e7c1f12".into(),
            ),
            root_after: wire::Root(
                "b3:6e866e13b5e65ef9961c050f8a621cf1980b00ee293be650deef5f4dbc6823f0".into(),
            ),
            actor: Some("agent:b0864fb2".into()),
            now: Some("2026-07-18T20:33:41Z".into()),
            files: vec![
                wire::DeltaFile {
                    path: wire::Path("notes/plan.md".into()),
                    change: wire::FileChange::Modified,
                    from_path: None,
                    file_rev_before: Some(wire::NodeRev("a9794a262e67ed02".into())),
                    file_rev_after: Some(wire::NodeRev("5f27a2814b517680".into())),
                    nodes: vec![wire::DeltaNode {
                        target: wire::SecRef::Hpath {
                            hpath: vec![seg("Goals"), seg("Q4")],
                        },
                        change: wire::NodeChange::Edited,
                        node_rev_before: Some(wire::NodeRev("4b8bc385a58da0e0".into())),
                        node_rev_after: Some(wire::NodeRev("f43203a1f0b4c9a3".into())),
                        span_after: Some(wire::Span(75, 150)),
                    }],
                },
                wire::DeltaFile {
                    path: wire::Path("receipts/2026-07-18.md".into()),
                    change: wire::FileChange::Modified,
                    from_path: None,
                    file_rev_before: Some(wire::NodeRev("51ad6428f5b5a898".into())),
                    file_rev_after: Some(wire::NodeRev("6cb0e939ce2edf5a".into())),
                    nodes: vec![wire::DeltaNode {
                        target: wire::SecRef::Anchor {
                            anchor: "r-000043".into(),
                        },
                        change: wire::NodeChange::Added,
                        node_rev_before: None,
                        node_rev_after: Some(wire::NodeRev("5c6ca7ec00ae279e".into())),
                        span_after: Some(wire::Span(287, 549)),
                    }],
                },
            ],
        },
    }
}

fn e4_delta_json() -> Value {
    json!({"delta":{
     "seq":2,
     "root_before":"b3:7f3b44376c719be236279e168c22fa2f4d346cd6e5da5bcf0784adb72e7c1f12",
     "root_after":"b3:6e866e13b5e65ef9961c050f8a621cf1980b00ee293be650deef5f4dbc6823f0",
     "actor":"agent:b0864fb2","now":"2026-07-18T20:33:41Z",
     "files":[
      {"path":"notes/plan.md","change":"modified",
       "file_rev_before":"a9794a262e67ed02","file_rev_after":"5f27a2814b517680",
       "nodes":[{"hpath":[{"h":"Goals"},{"h":"Q4"}],"change":"edited",
                 "node_rev_before":"4b8bc385a58da0e0","node_rev_after":"f43203a1f0b4c9a3",
                 "span_after":[75,150]}]},
      {"path":"receipts/2026-07-18.md","change":"modified",
       "file_rev_before":"51ad6428f5b5a898","file_rev_after":"6cb0e939ce2edf5a",
       "nodes":[{"anchor":"r-000043","change":"added",
                 "node_rev_after":"5c6ca7ec00ae279e","span_after":[287,549]}]}]}})
}

/// The two §7.1 worked delta notification frames, value-for-value, both
/// directions (serialize + read back).
#[test]
fn worked_e3_e4_delta_frames_match_contract() {
    for (frame, expected) in [(e3_delta(), e3_delta_json()), (e4_delta(), e4_delta_json())] {
        assert_eq!(serde_json::to_value(&frame).unwrap(), expected);
        assert_eq!(
            serde_json::from_value::<wire::DeltaFrame>(expected).unwrap(),
            frame
        );
    }
}

/// The E3 notification frame's bytes from before the additive `effects` field
/// existed: an empty effect set must keep them.
#[test]
fn no_effects_e3_frame_preserves_pre_c5_bytes() {
    let bytes = serde_json::to_vec(&e3_delta()).unwrap();
    assert_eq!(
        bytes,
        br#"{"delta":{"seq":1,"root_before":"b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9","root_after":"b3:7f3b44376c719be236279e168c22fa2f4d346cd6e5da5bcf0784adb72e7c1f12","actor":"agent:b0864fb2","now":"2026-07-18T20:31:04Z","files":[{"path":"notes/plan.md","change":"modified","file_rev_before":"e3c4acaceb75b907","file_rev_after":"a9794a262e67ed02","nodes":[{"hpath":[{"h":"Goals"},{"h":"Q3"}],"change":"edited","node_rev_before":"33d5b0e1b27cb48b","node_rev_after":"41f643f034e5681f","span_after":[49,75]}]},{"path":"receipts/2026-07-18.md","change":"modified","file_rev_before":"920a40c4ee23d37c","file_rev_after":"51ad6428f5b5a898","nodes":[{"anchor":"r-000042","change":"added","node_rev_after":"60bbee70d4a63a48","span_after":[26,286]}]}]}}"#
    );
}

#[test]
fn populated_effects_are_pre_delivery_additive_and_future_extensible() {
    #[derive(serde::Deserialize)]
    struct LegacyDeltaFrame {
        delta: wire::Delta,
    }

    const RECEIPT: &str = "tasks/x.md#^r-6794ce82d1d5aff1";

    let mut frame = e3_delta();
    let frozen_delta = frame.delta.clone();
    frame.effects = vec![wire::EffectEnvelope {
        intents: vec![wire::Intent {
            rule_id: "task-review-notify".into(),
            seq: 0,
            action: "notify".into(),
            target: Some("e4201e72".into()),
            severity: Some("info".into()),
            payload: Some("review requested".into()),
            receipt: RECEIPT.into(),
        }],
        narrowed: vec![],
        findings: vec![],
        how: "how:\n  route: channel-review\n".into(),
    }];

    let value = serde_json::to_value(&frame).unwrap();
    assert_eq!(value["effects"][0]["intents"][0]["receipt"], RECEIPT);
    assert!(
        !value.to_string().contains("delivered"),
        "delivery state is not representable on the armed-intent wire shape"
    );
    assert_eq!(
        value["delta"],
        serde_json::to_value(&frozen_delta).unwrap(),
        "the additive sibling must not reshape Delta"
    );

    let legacy: LegacyDeltaFrame = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(legacy.delta, frozen_delta);

    let mut future = value;
    future["effects"][0]
        .as_object_mut()
        .unwrap()
        .insert("wake_at".into(), json!("2026-07-18T12:50:00Z"));
    let current: wire::DeltaFrame = serde_json::from_value(future).unwrap();
    assert_eq!(
        current, frame,
        "future wake_at is an additive envelope field"
    );
}

/// §4.7/§7.3 replay ≡ live: the diff response body carries the same frame
/// objects as the live notifications — `diff(R0,R2)` is the two worked deltas,
/// byte-identical, in one `batches` array. No second diff dialect.
#[test]
fn worked_diff_response_batches_are_the_live_frames() {
    let response = wire::Response {
        id: Some(95),
        ok: true,
        payload: wire::ResponsePayload::Body {
            body: wire::ResponseBody::Diff {
                batches: vec![e3_delta(), e4_delta()],
            },
        },
    };
    let expected = json!({"id":95,"ok":true,"body":{
        "batches":[e3_delta_json(), e4_delta_json()]}});
    assert_eq!(serde_json::to_value(&response).unwrap(), expected);
    assert_eq!(
        serde_json::from_value::<wire::Response>(expected).unwrap(),
        response
    );
}

/// External changes produce deltas with `actor`/`now` absent (§7.1) — no key
/// on the wire, reading back to `None` — and a `DeltaNode` serializes no
/// `keys` key (decision 012: node-grain frozen at birth).
#[test]
fn external_delta_absent_actor_now_and_no_keys_slot() {
    let frame = wire::DeltaFrame {
        delta: wire::Delta {
            seq: 3,
            root_before: e4_delta().delta.root_after.clone(),
            root_after: e4_delta().delta.root_before.clone(),
            actor: None,
            now: None,
            files: e3_delta().delta.files.clone(),
        },
        effects: vec![],
        rescope: None,
        overflow: None,
    };
    let v = serde_json::to_value(&frame).unwrap();
    let delta = v["delta"].as_object().unwrap();
    assert!(!delta.contains_key("actor"));
    assert!(!delta.contains_key("now"));
    for file in v["delta"]["files"].as_array().unwrap() {
        for node in file["nodes"].as_array().unwrap() {
            assert!(
                !node.as_object().unwrap().contains_key("keys"),
                "row-12 purge: node-grain at birth, no keys slot"
            );
        }
    }
    let back: wire::DeltaFrame = serde_json::from_value(v).unwrap();
    assert_eq!(back, frame);
}

// ---------------------------------------------------------------------------
// Q5-LINKS: §4.6 worked exchange + §10.2 worked refusal + §10.1 honest tense
// ---------------------------------------------------------------------------

const R0: &str = "b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9";
const R2: &str = "b3:6e866e13b5e65ef9961c050f8a621cf1980b00ee293be650deef5f4dbc6823f0";

/// v2 §4.6, the worked exchange — id 80, the printed triple plus the
/// resolved/unresolved per-edge counts, round-tripped through the typed
/// vocabulary both directions.
#[test]
fn worked_links_frame_matches_contract() {
    let printed = json!({
        "id":80,"ok":true,"body":{
         "as_of_root":R2,
         "live_root":R2,
         "changes_seq":2,
         "files":{"notes/plan.md":{
           "resolved":{"receipts/2026-07-18.md":1},
           "unresolved":{"roadmap":1}}}}
    });
    let typed: wire::Response = serde_json::from_value(printed.clone()).unwrap();
    let wire::ResponsePayload::Body {
        body: wire::ResponseBody::Links { ref files, .. },
    } = typed.payload
    else {
        panic!("worked §4.6 frame decodes as the Links body")
    };
    assert_eq!(files["notes/plan.md"].resolved["receipts/2026-07-18.md"], 1);
    assert_eq!(files["notes/plan.md"].unresolved["roadmap"], 1);
    assert_eq!(serde_json::to_value(&typed).unwrap(), printed);
}

/// v2 §10.2, the worked refusal — id 81: the client demanded R0, the world is
/// at R2 — `stale_view`, retry class, `required` plus the sampled world beside
/// it, no message (the extras carry the whole diagnosis).
#[test]
fn worked_stale_view_refusal_matches_contract() {
    let printed = json!({
        "id":81,"ok":false,"error":{"code":"stale_view","recovery":"retry",
         "required":R0,
         "as_of_root":R2,
         "live_root":R2}
    });
    let typed: wire::Response = serde_json::from_value(printed.clone()).unwrap();
    let wire::ResponsePayload::Error { ref error } = typed.payload else {
        panic!("refusal decodes as the error envelope")
    };
    assert_eq!(error.code, wire::ErrorCode::StaleView);
    assert_eq!(error.recovery, wire::Recovery::Retry);
    assert_eq!(error.required.as_ref().unwrap().0, R0);
    assert_eq!(serde_json::to_value(&typed).unwrap(), printed);
}

/// §10.1 honest-tense law: a frame where `as_of_root ≠ live_root` — the corpus
/// moved while the answer was computed — is a legal success frame. Nothing in
/// the vocabulary asserts the roots equal or bounds their distance.
#[test]
fn honest_tense_divergent_triple_is_permitted_never_bounded() {
    let divergent = json!({
        "id":86,"ok":true,"body":{
         "as_of_root":R0,
         "live_root":R2,
         "changes_seq":1,
         "files":{"notes/plan.md":{"resolved":{},"unresolved":{"roadmap":2}}}}
    });
    let typed: wire::Response = serde_json::from_value(divergent.clone()).unwrap();
    assert!(typed.ok, "a stale view is a view, not an error");
    let wire::ResponsePayload::Body {
        body:
            wire::ResponseBody::Links {
                ref as_of_root,
                ref live_root,
                ..
            },
    } = typed.payload
    else {
        panic!("divergent triple decodes as the Links body")
    };
    assert_ne!(as_of_root, live_root, "the divergence under test");
    // the absent bound is the §10.1 law: the frame round-trips unchanged
    assert_eq!(serde_json::to_value(&typed).unwrap(), divergent);
}

/// `FileLinks` pinned as a key set, not by value: the two cross-root keys
/// (`resolved_rooted`, `refused`) skip when empty, so a worked-value sweep
/// would pass whether or not they leaked into a v2 frame. A single-root corpus
/// must serialize exactly the two keys the frozen contract names.
#[test]
fn an_ambient_file_links_serializes_exactly_the_frozen_v2_key_set() {
    let ambient = wire::FileLinks {
        resolved: [("receipts/2026-07-18.md".to_owned(), 1u64)]
            .into_iter()
            .collect(),
        unresolved: [("roadmap".to_owned(), 1u64)].into_iter().collect(),
        ..Default::default()
    };
    let value = serde_json::to_value(&ambient).unwrap();
    let keys: Vec<&str> = value
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        ["resolved", "unresolved"],
        "a corpus with no cross-root edges serializes the FROZEN v2 key set — \
         the U21 keys are absent, not empty objects: {value}"
    );

    // The negative half: without it the pin above would still pass if
    // `resolved_rooted` were deleted outright.
    let rooted = wire::FileLinks {
        resolved_rooted: [(
            "sessions".to_owned(),
            [("notes.md".to_owned(), 1u64)].into_iter().collect(),
        )]
        .into_iter()
        .collect(),
        ..Default::default()
    };
    let value = serde_json::to_value(&rooted).unwrap();
    assert!(
        value.get("resolved_rooted").is_some(),
        "a corpus WITH a cross-root edge does carry the key: {value}"
    );
}
