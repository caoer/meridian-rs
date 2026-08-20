//! U6a gates — the effect-shim FD protocol (S6): explicit length-prefix
//! framing, trailer-counted streams, and FAIL CLOSED on every deviation —
//! one bad byte refuses the whole batch.

use effects::{ArgValue, EffectKind, Provenance};
use run::shim::{self, MAX_RECORD_BYTES, MAX_RECORDS, ShimDescriptor, ShimError, ShimStream};

fn stream(bytes: &[u8]) -> ShimStream {
    ShimStream {
        bytes: bytes.to_vec(),
        overflowed: false,
    }
}

fn frame(payload: &str) -> String {
    format!("{}:{payload}\n", payload.len())
}

const SET: &str = r#"{"op":"md.set_field","field":"status","value":"done"}"#;
const APPEND: &str = r#"{"op":"md.append_section","section":"Log","content":"- ran"}"#;

/// Parse ONE record, or the refusal it earned.
fn parse_one(payload: &str) -> Result<ShimDescriptor, ShimError> {
    let bytes = format!("{}end:1\n", frame(payload));
    shim::parse(&stream(bytes.as_bytes())).map(|mut d| {
        assert_eq!(d.len(), 1);
        d.remove(0)
    })
}

#[test]
fn records_and_trailer_parse_in_emission_order() {
    let bytes = format!("{}{}end:2\n", frame(SET), frame(APPEND));
    let got = shim::parse(&stream(bytes.as_bytes())).unwrap();
    assert_eq!(
        got,
        vec![
            ShimDescriptor::SetField {
                field: "status".into(),
                value: "done".into(),
            },
            ShimDescriptor::AppendSection {
                section: "Log".into(),
                content: "- ran".into(),
            },
        ]
    );
}

#[test]
fn a_zero_byte_stream_is_the_empty_batch() {
    assert_eq!(shim::parse(&stream(b"")).unwrap(), vec![]);
}

#[test]
fn an_explicit_empty_stream_parses() {
    assert_eq!(shim::parse(&stream(b"end:0\n")).unwrap(), vec![]);
}

#[test]
fn length_counts_bytes_not_chars() {
    let payload = r#"{"op":"md.set_field","field":"état","value":"réussi"}"#;
    let bytes = format!("{}end:1\n", frame(payload)); // frame() uses str::len — bytes
    let got = shim::parse(&stream(bytes.as_bytes())).unwrap();
    assert_eq!(got.len(), 1);
}

#[test]
fn truncated_payload_fails_closed() {
    // Declares 52 bytes, the stream is cut mid-payload.
    let bytes = format!("52:{}", &SET[..20]);
    assert!(matches!(
        shim::parse(&stream(bytes.as_bytes())),
        Err(ShimError::Truncated { at: 0 })
    ));
}

#[test]
fn a_stream_cut_mid_prefix_fails_closed() {
    assert!(matches!(
        shim::parse(&stream(b"12")),
        Err(ShimError::Truncated { at: 0 })
    ));
}

#[test]
fn a_missing_trailer_fails_closed() {
    // A SIGKILLed emitter cannot land a prefix of its intent (S6).
    let bytes = frame(SET);
    assert!(matches!(
        shim::parse(&stream(bytes.as_bytes())),
        Err(ShimError::MissingTrailer)
    ));
}

#[test]
fn a_wrong_trailer_count_fails_closed() {
    let bytes = format!("{}end:2\n", frame(SET));
    assert!(matches!(
        shim::parse(&stream(bytes.as_bytes())),
        Err(ShimError::CountMismatch {
            declared: 2,
            actual: 1,
        })
    ));
}

#[test]
fn bytes_after_the_trailer_fail_closed() {
    let bytes = format!("{}end:1\nx", frame(SET));
    assert!(matches!(
        shim::parse(&stream(bytes.as_bytes())),
        Err(ShimError::TrailingData { .. })
    ));
}

#[test]
fn a_non_decimal_length_prefix_fails_closed() {
    assert!(matches!(
        shim::parse(&stream(b"xx:{}\n")),
        Err(ShimError::BadFrame { at: 0 })
    ));
}

#[test]
fn a_length_not_landing_on_the_terminator_fails_closed() {
    // Payload is longer than declared — the byte at the declared end is not
    // the record newline.
    let bytes = format!("10:{SET}\n");
    assert!(matches!(
        shim::parse(&stream(bytes.as_bytes())),
        Err(ShimError::BadFrame { at: 0 })
    ));
}

#[test]
fn malformed_json_fails_closed() {
    let bytes = format!("{}end:1\n", frame("{not json"));
    assert!(matches!(
        shim::parse(&stream(bytes.as_bytes())),
        Err(ShimError::Malformed { index: 0, .. })
    ));
}

#[test]
fn a_non_object_payload_fails_closed() {
    let bytes = format!("{}end:1\n", frame("[1,2]"));
    assert!(matches!(
        shim::parse(&stream(bytes.as_bytes())),
        Err(ShimError::Malformed { index: 0, .. })
    ));
}

#[test]
fn non_utf8_payload_fails_closed() {
    let mut bytes: Vec<u8> = b"2:".to_vec();
    bytes.extend_from_slice(&[0xFF, 0xFE]);
    bytes.extend_from_slice(b"\nend:1\n");
    assert!(matches!(
        shim::parse(&stream(&bytes)),
        Err(ShimError::Malformed { index: 0, .. })
    ));
}

#[test]
fn the_shim_admits_md_only_proto_refuses() {
    // Ruling 1: bash has no daemon.*/proto.* emission path.
    let payload = r#"{"op":"proto.send","to":"x","message":"hi"}"#;
    let bytes = format!("{}end:1\n", frame(payload));
    assert!(matches!(
        shim::parse(&stream(bytes.as_bytes())),
        Err(ShimError::UnknownOp { index: 0, ref op }) if op == "proto.send"
    ));
}

#[test]
fn an_extra_key_is_refused_not_ignored() {
    let payload = r#"{"op":"md.set_field","field":"a","value":"b","sneak":"c"}"#;
    let bytes = format!("{}end:1\n", frame(payload));
    assert!(matches!(
        shim::parse(&stream(bytes.as_bytes())),
        Err(ShimError::Malformed { index: 0, ref reason }) if reason.contains("sneak")
    ));
}

#[test]
fn a_missing_required_key_fails_closed() {
    let payload = r#"{"op":"md.set_field","field":"a"}"#;
    let bytes = format!("{}end:1\n", frame(payload));
    assert!(matches!(
        shim::parse(&stream(bytes.as_bytes())),
        Err(ShimError::Malformed { index: 0, ref reason }) if reason.contains("value")
    ));
}

#[test]
fn a_non_string_argument_fails_closed() {
    let payload = r#"{"op":"md.set_field","field":"a","value":7}"#;
    let bytes = format!("{}end:1\n", frame(payload));
    assert!(matches!(
        shim::parse(&stream(bytes.as_bytes())),
        Err(ShimError::Malformed { index: 0, .. })
    ));
}

#[test]
fn an_oversize_declared_record_fails_closed() {
    let bytes = format!("{}:x", MAX_RECORD_BYTES + 1);
    assert!(matches!(
        shim::parse(&stream(bytes.as_bytes())),
        Err(ShimError::RecordTooLarge { .. })
    ));
}

#[test]
fn a_record_count_past_the_cap_fails_closed() {
    use std::fmt::Write;
    let mut bytes = String::new();
    for _ in 0..=MAX_RECORDS {
        bytes.push_str(&frame(SET));
    }
    writeln!(bytes, "end:{}", MAX_RECORDS + 1).unwrap();
    assert!(matches!(
        shim::parse(&stream(bytes.as_bytes())),
        Err(ShimError::TooManyRecords)
    ));
}

#[test]
fn an_overflowed_capture_fails_closed() {
    let s = ShimStream {
        bytes: Vec::new(),
        overflowed: true,
    };
    assert!(matches!(shim::parse(&s), Err(ShimError::Overflow)));
}

#[test]
fn to_effects_stamps_run_provenance_seq_and_depth() {
    let descriptors = vec![
        ShimDescriptor::SetField {
            field: "status".into(),
            value: "done".into(),
        },
        ShimDescriptor::AppendSection {
            section: "Log".into(),
            content: "- ran".into(),
        },
    ];
    let effects = shim::to_effects(&descriptors, "fix-x", "inv-1", "root-abc");
    assert_eq!(effects.len(), 2);
    assert_eq!(effects[0].kind, EffectKind::SetField);
    assert_eq!(effects[1].kind, EffectKind::AppendSection);
    for (i, e) in effects.iter().enumerate() {
        assert_eq!(e.rule_id, "fix-x");
        assert_eq!(e.seq, u32::try_from(i).unwrap());
        assert_eq!(e.depth, 0);
        assert_eq!(
            e.provenance,
            Provenance::Run {
                invocation_id: "inv-1".into(),
                root_at_eval: "root-abc".into(),
            }
        );
        // Run-plane effects have NO idempotency key (a re-run is new intent).
        assert!(e.idempotency_key().is_none());
    }
    assert_eq!(
        effects[0].args.get("field"),
        Some(&ArgValue::Str("status".into()))
    );
    assert_eq!(
        effects[1].args.get("content"),
        Some(&ArgValue::Str("- ran".into()))
    );
}

// ── The birth lane's targeting axis (`base`) ────────────────────────────────
// The bash lane's twin of starlark `create(path=, body=, base=)`: the path is
// the DECLARED relative landing coordinate, targeting rides the separate
// optional `base` (ZT ruling 2026-08-19 #2 — boundary as data). Regression
// guard: before this, `md.create` admitted EXACTLY {path, body}, so the bash
// lane had no way to express targeting at all and every canonical card birth
// through the page refused.

#[test]
fn a_create_without_base_parses_baseless() {
    let got = parse_one(r#"{"op":"md.create","path":"tasks/x.md","body":"hello"}"#).unwrap();
    assert_eq!(
        got,
        ShimDescriptor::Create {
            path: "tasks/x.md".into(),
            body: "hello".into(),
            base: None,
        }
    );
}

#[test]
fn a_create_carries_its_declared_base() {
    let got = parse_one(
        r#"{"op":"md.create","path":"tasks/x.md","body":"hello","base":"field-notes-sessions:year=2026/s"}"#,
    )
    .unwrap();
    assert_eq!(
        got,
        ShimDescriptor::Create {
            path: "tasks/x.md".into(),
            body: "hello".into(),
            base: Some("field-notes-sessions:year=2026/s".into()),
        }
    );
}

#[test]
fn a_bare_relative_base_rides_verbatim() {
    let got =
        parse_one(r#"{"op":"md.create","path":"tasks/x.md","body":"hello","base":"year=2026/s"}"#)
            .unwrap();
    let ShimDescriptor::Create { base, .. } = got else {
        panic!("expected a birth descriptor");
    };
    // The shim judges NOTHING about the base's shape — confinement, rooted
    // spelling, and foreign-root refusals are the resolver's one opinion.
    assert_eq!(base, Some("year=2026/s".into()));
}

#[test]
fn an_unknown_key_on_create_still_fails_closed() {
    let err = parse_one(r#"{"op":"md.create","path":"tasks/x.md","body":"h","target":"s"}"#)
        .expect_err("an extra key refuses the whole batch");
    let ShimError::Malformed { index: 0, reason } = err else {
        panic!("expected Malformed, got {err:?}");
    };
    assert!(reason.contains("unknown key 'target'"), "{reason}");
}

#[test]
fn a_non_string_base_fails_closed() {
    let err = parse_one(r#"{"op":"md.create","path":"tasks/x.md","body":"h","base":7}"#)
        .expect_err("a non-string base refuses");
    assert!(
        matches!(err, ShimError::Malformed { index: 0, .. }),
        "{err:?}"
    );
}

#[test]
fn a_create_missing_a_required_key_fails_closed() {
    let err = parse_one(r#"{"op":"md.create","path":"tasks/x.md","base":"s"}"#)
        .expect_err("body is required");
    let ShimError::Malformed { reason, .. } = err else {
        panic!("expected Malformed");
    };
    assert!(reason.contains("missing 'body'"), "{reason}");
}

#[test]
fn to_effects_carries_base_only_when_declared() {
    let effects = shim::to_effects(
        &[
            ShimDescriptor::Create {
                path: "tasks/a.md".into(),
                body: "a".into(),
                base: Some("year=2026/s".into()),
            },
            ShimDescriptor::Create {
                path: "tasks/b.md".into(),
                body: "b".into(),
                base: None,
            },
        ],
        "fix-x",
        "inv-1",
        "root-abc",
    );
    assert_eq!(effects[0].kind, EffectKind::Create);
    assert_eq!(
        effects[0].args.get("path"),
        Some(&ArgValue::Str("tasks/a.md".into()))
    );
    assert_eq!(
        effects[0].args.get("base"),
        Some(&ArgValue::Str("year=2026/s".into()))
    );
    // Absent, not empty — the resolver reads an absent base as "fall back to
    // ambient" and an empty one as a refusal.
    assert_eq!(effects[1].args.get("base"), None);
}
