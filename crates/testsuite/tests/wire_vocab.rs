//! The wire-contract vocabulary gate, executable: error codes and node kinds
//! serialize to the contract's exact strings, in the frozen ordinal order.

use wire::{ErrorCode, NodeKind};

#[test]
fn error_codes_match_contract_v1() {
    let codes = [
        (ErrorCode::BadFrame, "bad_frame"),
        (ErrorCode::BadRequest, "bad_request"),
        (ErrorCode::UnknownOp, "unknown_op"),
        (ErrorCode::UnsupportedProto, "unsupported_proto"),
        (ErrorCode::BadPath, "bad_path"),
        (ErrorCode::NotFound, "not_found"),
        (ErrorCode::InvalidUtf8, "invalid_utf8"),
        (ErrorCode::Internal, "internal"),
        (ErrorCode::CasMismatch, "cas_mismatch"),
    ];
    for (code, wire_str) in codes {
        assert_eq!(
            serde_json::to_value(code).unwrap(),
            serde_json::Value::String(wire_str.into())
        );
    }
}

#[test]
fn node_kinds_match_contract_v1_in_ordinal_order() {
    // Contract §5.2: sort tiebreak is kind-enum ordinal, the order listed
    // (frontmatter = 0 … comment = 10). Rust discriminant order must agree.
    let kinds = [
        (NodeKind::Frontmatter, "frontmatter"),
        (NodeKind::Heading, "heading"),
        (NodeKind::Fence, "fence"),
        (NodeKind::InlineCode, "inline-code"),
        (NodeKind::Anchor, "anchor"),
        (NodeKind::Wikilink, "wikilink"),
        (NodeKind::Embed, "embed"),
        (NodeKind::Callout, "callout"),
        (NodeKind::Task, "task"),
        (NodeKind::Table, "table"),
        (NodeKind::Comment, "comment"),
    ];
    for (ordinal, (kind, wire_str)) in kinds.iter().enumerate() {
        assert_eq!(
            serde_json::to_value(kind).unwrap(),
            serde_json::Value::String((*wire_str).into())
        );
        assert_eq!(*kind as usize, ordinal, "{wire_str} ordinal");
    }
}

#[test]
fn contract_example_error_envelope_roundtrips() {
    // The vision's canonical cas_mismatch line, verbatim from contract §4.
    let line =
        r#"{"id":3,"ok":false,"error":"cas_mismatch","expected":"c71d09","actual":"5e2f77"}"#;
    let resp: wire::Response = serde_json::from_str(line).unwrap();
    assert!(!resp.ok);
    assert_eq!(resp.id, Some(3));
    let wire::ResponseBody::Error(err) = &resp.body else {
        panic!("classifies as error envelope")
    };
    assert_eq!(err.error, ErrorCode::CasMismatch);
    assert_eq!(err.expected, Some(wire::NodeRev("c71d09".into())));
    assert_eq!(
        serde_json::to_value(&resp).unwrap(),
        serde_json::from_str::<serde_json::Value>(line).unwrap()
    );
}
