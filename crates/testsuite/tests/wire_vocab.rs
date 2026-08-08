//! The wire-contract vocabulary gate, executable: error codes and node kinds
//! serialize to the contract's exact strings, in the frozen ordinal order.

use wire::{ErrorCode, NodeKind};

#[test]
fn error_codes_match_contract_v2() {
    let codes = [
        (ErrorCode::BadFrame, "bad_frame"),
        (ErrorCode::BadRequest, "bad_request"),
        (ErrorCode::UnknownOp, "unknown_op"),
        (ErrorCode::UnsupportedProto, "unsupported_proto"),
        (ErrorCode::BadPath, "bad_path"),
        (ErrorCode::NoMatch, "no_match"),
        (ErrorCode::NotUnique, "not_unique"),
        (ErrorCode::WouldCorrupt, "would_corrupt"),
        (ErrorCode::LockTimeout, "lock_timeout"),
        (ErrorCode::FileNotFound, "file_not_found"),
        (ErrorCode::IoError, "io_error"),
        (ErrorCode::InvalidUtf8, "invalid_utf8"),
        (ErrorCode::Internal, "internal"),
        (ErrorCode::CasMismatch, "cas_mismatch"),
        (ErrorCode::RefNotFound, "ref_not_found"),
        (ErrorCode::AmbiguousRef, "ambiguous_ref"),
        (ErrorCode::RootMismatch, "root_mismatch"),
        (ErrorCode::RootUnknown, "root_unknown"),
        (ErrorCode::CorpusRace, "corpus_race"),
    ];
    for (code, wire_str) in codes {
        assert_eq!(
            serde_json::to_value(code).unwrap(),
            serde_json::Value::String(wire_str.into())
        );
    }
}

#[test]
fn recovery_classes_match_contract_v2() {
    use wire::Recovery;
    let classes = [
        (Recovery::Fix, "fix"),
        (Recovery::Env, "env"),
        (Recovery::Refresh, "refresh"),
        (Recovery::Retry, "retry"),
        (Recovery::Resync, "resync"),
        (Recovery::Respawn, "respawn"),
    ];
    for (class, wire_str) in classes {
        assert_eq!(
            serde_json::to_value(class).unwrap(),
            serde_json::Value::String(wire_str.into())
        );
    }
}

#[test]
fn node_kinds_match_contract_in_ordinal_order() {
    // v2 §4.3 carries the v1 §5.2 order: sort tiebreak is kind-enum ordinal,
    // the order listed (frontmatter = 0 … comment = 10). Rust discriminant
    // order must agree.
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
    // The v2 §5.2 worked cas_mismatch frame: nested envelope, code +
    // REQUIRED recovery + code-specific extras beside them.
    let line = r#"{"id":88,"ok":false,"error":{"code":"cas_mismatch","recovery":"refresh","expected":"33d5b0e1b27cb48b","actual":"41f643f034e5681f"}}"#;
    let resp: wire::Response = serde_json::from_str(line).unwrap();
    assert!(!resp.ok);
    assert_eq!(resp.id, Some(88));
    let wire::ResponsePayload::Error { error: err } = &resp.payload else {
        panic!("classifies as error envelope")
    };
    assert_eq!(err.code, ErrorCode::CasMismatch);
    assert_eq!(err.recovery, wire::Recovery::Refresh);
    assert_eq!(err.expected, Some(wire::NodeRev("33d5b0e1b27cb48b".into())));
    assert_eq!(
        serde_json::to_value(&resp).unwrap(),
        serde_json::from_str::<serde_json::Value>(line).unwrap()
    );
}
