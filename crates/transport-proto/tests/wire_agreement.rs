//! The drift pin (law 3 as amended): `wire` is the canonical contract, the
//! `.proto` transcribes it, and THIS FILE is what makes the transcription
//! unbreakable by construction — every wire type is converted through
//! exhaustive struct destructuring and exhaustive matches, so adding a wire
//! variant or field stops compiling here until `meridian.proto` catches up
//! (and vice versa: a proto-only oneof arm breaks the reverse match).
//!
//! The runtime half proves the schema loses nothing: samples covering every
//! op, node kind, info shape, response body, and error code round-trip
//! wire → proto → encoded frame → proto → wire, `assert_eq` with the original.
//!
//! These conversion fns are test-local on purpose: the lib-level seam joins
//! with sidecar proto negotiation, and this file is its reference when it does.

use transport_proto::pb;

// ---------------------------------------------------------------------------
// wire → proto
// ---------------------------------------------------------------------------

fn span_to_pb(s: wire::Span) -> pb::Span {
    let wire::Span(start, end) = s;
    pb::Span { start, end }
}

fn kind_to_pb(k: wire::NodeKind) -> pb::NodeKind {
    match k {
        wire::NodeKind::Frontmatter => pb::NodeKind::Frontmatter,
        wire::NodeKind::Heading => pb::NodeKind::Heading,
        wire::NodeKind::Fence => pb::NodeKind::Fence,
        wire::NodeKind::InlineCode => pb::NodeKind::InlineCode,
        wire::NodeKind::Anchor => pb::NodeKind::Anchor,
        wire::NodeKind::Wikilink => pb::NodeKind::Wikilink,
        wire::NodeKind::Embed => pb::NodeKind::Embed,
        wire::NodeKind::Callout => pb::NodeKind::Callout,
        wire::NodeKind::Task => pb::NodeKind::Task,
        wire::NodeKind::Table => pb::NodeKind::Table,
        wire::NodeKind::Comment => pb::NodeKind::Comment,
    }
}

fn code_to_pb(c: wire::ErrorCode) -> pb::ErrorCode {
    match c {
        wire::ErrorCode::BadFrame => pb::ErrorCode::BadFrame,
        wire::ErrorCode::BadRequest => pb::ErrorCode::BadRequest,
        wire::ErrorCode::UnknownOp => pb::ErrorCode::UnknownOp,
        wire::ErrorCode::UnsupportedProto => pb::ErrorCode::UnsupportedProto,
        wire::ErrorCode::BadPath => pb::ErrorCode::BadPath,
        wire::ErrorCode::NotFound => pb::ErrorCode::NotFound,
        wire::ErrorCode::InvalidUtf8 => pb::ErrorCode::InvalidUtf8,
        wire::ErrorCode::Internal => pb::ErrorCode::Internal,
        wire::ErrorCode::CasMismatch => pb::ErrorCode::CasMismatch,
    }
}

fn info_to_pb(i: wire::Info) -> pb::info::Info {
    match i {
        wire::Info::Frontmatter { keys } => {
            pb::info::Info::Frontmatter(pb::FrontmatterInfo { keys })
        }
        wire::Info::Fence { info_string } => pb::info::Info::Fence(pb::FenceInfo { info_string }),
        wire::Info::Wikilink {
            target,
            heading,
            block,
            alias,
        } => pb::info::Info::Wikilink(pb::WikilinkInfo {
            target,
            heading,
            block,
            alias,
        }),
        wire::Info::Callout { r#type, fold } => {
            pb::info::Info::Callout(pb::CalloutInfo { r#type, fold })
        }
        wire::Info::Task { checked, depth } => {
            pb::info::Info::Task(pb::TaskInfo { checked, depth })
        }
    }
}

fn node_to_pb(n: wire::Node) -> pb::Node {
    let wire::Node {
        kind,
        span,
        text_prefix_16b,
        hpath,
        unterminated,
        info,
        node_rev,
    } = n;
    pb::Node {
        kind: kind_to_pb(kind).into(),
        span: Some(span_to_pb(span)),
        text_prefix_16b,
        // empty ≡ absent: a heading always carries ≥1 segment (contract §5.2)
        hpath: hpath.unwrap_or_default(),
        unterminated,
        info: info.map(|i| pb::Info {
            info: Some(info_to_pb(i)),
        }),
        node_rev: node_rev.map(|r| r.0),
    }
}

fn op_to_pb(op: wire::Op) -> pb::request::Op {
    match op {
        wire::Op::Hello { proto, client } => {
            pb::request::Op::Hello(pb::HelloRequest { proto, client })
        }
        wire::Op::Toc { path } => pb::request::Op::Toc(pb::TocRequest { path: path.0 }),
        wire::Op::Extract { path, kinds } => pb::request::Op::Extract(pb::ExtractRequest {
            path: path.0,
            kinds: kinds.map(|kinds| pb::KindFilter { kinds }),
        }),
        wire::Op::Resolve { path, r#ref } => pb::request::Op::Resolve(pb::ResolveRequest {
            path: path.0,
            r#ref,
        }),
        wire::Op::Splice {
            path,
            span,
            if_node_rev,
            text,
        } => pb::request::Op::Splice(pb::SpliceRequest {
            path: path.0,
            span: Some(span_to_pb(span)),
            if_node_rev: if_node_rev.0,
            text,
        }),
        wire::Op::Root { path } => pb::request::Op::Root(pb::RootRequest {
            path: path.map(|p| p.0),
        }),
        wire::Op::Guard { root, path } => pb::request::Op::Guard(pb::GuardRequest {
            root: root.0,
            path: path.map(|p| p.0),
        }),
    }
}

fn request_to_pb(r: wire::Request) -> pb::Request {
    let wire::Request { id, op } = r;
    pb::Request {
        id,
        op: Some(op_to_pb(op)),
    }
}

fn error_to_pb(e: wire::ErrorBody) -> pb::ErrorBody {
    let wire::ErrorBody {
        error,
        message,
        path,
        supported,
        expected,
        actual,
    } = e;
    pb::ErrorBody {
        error: code_to_pb(error).into(),
        message,
        path: path.map(|p| p.0),
        // empty ≡ absent: a sidecar always speaks ≥1 proto (contract §4)
        supported: supported.unwrap_or_default(),
        expected: expected.map(|r| r.0),
        actual: actual.map(|r| r.0),
    }
}

fn body_to_pb(b: wire::ResponseBody) -> pb::response::Body {
    match b {
        wire::ResponseBody::Hello {
            proto,
            server,
            caps,
        } => pb::response::Body::Hello(pb::HelloResponse {
            proto,
            server,
            caps,
        }),
        wire::ResponseBody::Nodes { path, nodes } => pb::response::Body::Nodes(pb::NodesResponse {
            path: path.0,
            nodes: nodes.into_iter().map(node_to_pb).collect(),
        }),
        wire::ResponseBody::Resolve {
            path,
            span,
            node_rev,
            content_span,
        } => pb::response::Body::Resolve(pb::ResolveResponse {
            path: path.0,
            span: Some(span_to_pb(span)),
            node_rev: node_rev.0,
            content_span: content_span.map(span_to_pb),
        }),
        wire::ResponseBody::Splice { span, node_rev } => {
            pb::response::Body::Splice(pb::SpliceResponse {
                span: Some(span_to_pb(span)),
                node_rev: node_rev.0,
            })
        }
        wire::ResponseBody::Root { root } => {
            pb::response::Body::Root(pb::RootResponse { root: root.0 })
        }
        wire::ResponseBody::Guard { root } => {
            pb::response::Body::Guard(pb::GuardResponse { root: root.0 })
        }
        wire::ResponseBody::Error(e) => pb::response::Body::Error(error_to_pb(e)),
    }
}

fn response_to_pb(r: wire::Response) -> pb::Response {
    let wire::Response { id, ok, body } = r;
    pb::Response {
        id,
        ok,
        body: Some(body_to_pb(body)),
    }
}

// ---------------------------------------------------------------------------
// proto → wire (the reverse pin: a proto-only arm breaks these matches)
// ---------------------------------------------------------------------------

fn span_from_pb(s: pb::Span) -> wire::Span {
    let pb::Span { start, end } = s;
    wire::Span(start, end)
}

fn kind_from_pb(k: pb::NodeKind) -> wire::NodeKind {
    match k {
        pb::NodeKind::Unspecified => panic!("UNSPECIFIED never crosses the seam"),
        pb::NodeKind::Frontmatter => wire::NodeKind::Frontmatter,
        pb::NodeKind::Heading => wire::NodeKind::Heading,
        pb::NodeKind::Fence => wire::NodeKind::Fence,
        pb::NodeKind::InlineCode => wire::NodeKind::InlineCode,
        pb::NodeKind::Anchor => wire::NodeKind::Anchor,
        pb::NodeKind::Wikilink => wire::NodeKind::Wikilink,
        pb::NodeKind::Embed => wire::NodeKind::Embed,
        pb::NodeKind::Callout => wire::NodeKind::Callout,
        pb::NodeKind::Task => wire::NodeKind::Task,
        pb::NodeKind::Table => wire::NodeKind::Table,
        pb::NodeKind::Comment => wire::NodeKind::Comment,
    }
}

fn code_from_pb(c: pb::ErrorCode) -> wire::ErrorCode {
    match c {
        pb::ErrorCode::Unspecified => panic!("UNSPECIFIED never crosses the seam"),
        pb::ErrorCode::BadFrame => wire::ErrorCode::BadFrame,
        pb::ErrorCode::BadRequest => wire::ErrorCode::BadRequest,
        pb::ErrorCode::UnknownOp => wire::ErrorCode::UnknownOp,
        pb::ErrorCode::UnsupportedProto => wire::ErrorCode::UnsupportedProto,
        pb::ErrorCode::BadPath => wire::ErrorCode::BadPath,
        pb::ErrorCode::NotFound => wire::ErrorCode::NotFound,
        pb::ErrorCode::InvalidUtf8 => wire::ErrorCode::InvalidUtf8,
        pb::ErrorCode::Internal => wire::ErrorCode::Internal,
        pb::ErrorCode::CasMismatch => wire::ErrorCode::CasMismatch,
    }
}

fn info_from_pb(i: pb::info::Info) -> wire::Info {
    match i {
        pb::info::Info::Frontmatter(pb::FrontmatterInfo { keys }) => {
            wire::Info::Frontmatter { keys }
        }
        pb::info::Info::Fence(pb::FenceInfo { info_string }) => wire::Info::Fence { info_string },
        pb::info::Info::Wikilink(pb::WikilinkInfo {
            target,
            heading,
            block,
            alias,
        }) => wire::Info::Wikilink {
            target,
            heading,
            block,
            alias,
        },
        pb::info::Info::Callout(pb::CalloutInfo { r#type, fold }) => {
            wire::Info::Callout { r#type, fold }
        }
        pb::info::Info::Task(pb::TaskInfo { checked, depth }) => {
            wire::Info::Task { checked, depth }
        }
    }
}

fn node_from_pb(n: pb::Node) -> wire::Node {
    let pb::Node {
        kind,
        span,
        text_prefix_16b,
        hpath,
        unterminated,
        info,
        node_rev,
    } = n;
    wire::Node {
        kind: kind_from_pb(pb::NodeKind::try_from(kind).expect("known kind")),
        span: span_from_pb(span.expect("span is required")),
        text_prefix_16b,
        hpath: if hpath.is_empty() { None } else { Some(hpath) },
        unterminated,
        info: info.map(|i| info_from_pb(i.info.expect("info oneof set"))),
        node_rev: node_rev.map(wire::NodeRev),
    }
}

fn op_from_pb(op: pb::request::Op) -> wire::Op {
    match op {
        pb::request::Op::Hello(pb::HelloRequest { proto, client }) => {
            wire::Op::Hello { proto, client }
        }
        pb::request::Op::Toc(pb::TocRequest { path }) => wire::Op::Toc {
            path: wire::Path(path),
        },
        pb::request::Op::Extract(pb::ExtractRequest { path, kinds }) => wire::Op::Extract {
            path: wire::Path(path),
            kinds: kinds.map(|f| f.kinds),
        },
        pb::request::Op::Resolve(pb::ResolveRequest { path, r#ref }) => wire::Op::Resolve {
            path: wire::Path(path),
            r#ref,
        },
        pb::request::Op::Splice(pb::SpliceRequest {
            path,
            span,
            if_node_rev,
            text,
        }) => wire::Op::Splice {
            path: wire::Path(path),
            span: span_from_pb(span.expect("span is required")),
            if_node_rev: wire::NodeRev(if_node_rev),
            text,
        },
        pb::request::Op::Root(pb::RootRequest { path }) => wire::Op::Root {
            path: path.map(wire::Path),
        },
        pb::request::Op::Guard(pb::GuardRequest { root, path }) => wire::Op::Guard {
            root: wire::Root(root),
            path: path.map(wire::Path),
        },
    }
}

fn request_from_pb(r: pb::Request) -> wire::Request {
    let pb::Request { id, op } = r;
    wire::Request {
        id,
        op: op_from_pb(op.expect("op oneof set")),
    }
}

fn error_from_pb(e: pb::ErrorBody) -> wire::ErrorBody {
    let pb::ErrorBody {
        error,
        message,
        path,
        supported,
        expected,
        actual,
    } = e;
    wire::ErrorBody {
        error: code_from_pb(pb::ErrorCode::try_from(error).expect("known code")),
        message,
        path: path.map(wire::Path),
        supported: if supported.is_empty() {
            None
        } else {
            Some(supported)
        },
        expected: expected.map(wire::NodeRev),
        actual: actual.map(wire::NodeRev),
    }
}

fn body_from_pb(b: pb::response::Body) -> wire::ResponseBody {
    match b {
        pb::response::Body::Hello(pb::HelloResponse {
            proto,
            server,
            caps,
        }) => wire::ResponseBody::Hello {
            proto,
            server,
            caps,
        },
        pb::response::Body::Nodes(pb::NodesResponse { path, nodes }) => wire::ResponseBody::Nodes {
            path: wire::Path(path),
            nodes: nodes.into_iter().map(node_from_pb).collect(),
        },
        pb::response::Body::Resolve(pb::ResolveResponse {
            path,
            span,
            node_rev,
            content_span,
        }) => wire::ResponseBody::Resolve {
            path: wire::Path(path),
            span: span_from_pb(span.expect("span is required")),
            node_rev: wire::NodeRev(node_rev),
            content_span: content_span.map(span_from_pb),
        },
        pb::response::Body::Splice(pb::SpliceResponse { span, node_rev }) => {
            wire::ResponseBody::Splice {
                span: span_from_pb(span.expect("span is required")),
                node_rev: wire::NodeRev(node_rev),
            }
        }
        pb::response::Body::Root(pb::RootResponse { root }) => wire::ResponseBody::Root {
            root: wire::Root(root),
        },
        pb::response::Body::Guard(pb::GuardResponse { root }) => wire::ResponseBody::Guard {
            root: wire::Root(root),
        },
        pb::response::Body::Error(e) => wire::ResponseBody::Error(error_from_pb(e)),
    }
}

fn response_from_pb(r: pb::Response) -> wire::Response {
    let pb::Response { id, ok, body } = r;
    wire::Response {
        id,
        ok,
        body: body_from_pb(body.expect("body oneof set")),
    }
}

// ---------------------------------------------------------------------------
// samples — every op, every node kind + info shape, every body, every code
// ---------------------------------------------------------------------------

fn sample_nodes() -> Vec<wire::Node> {
    use wire::{Info, NodeKind};
    let mk = |kind: NodeKind, hpath: Option<Vec<&str>>, info: Option<Info>| wire::Node {
        kind,
        span: wire::Span(0, 128),
        text_prefix_16b: "sixteen bytes ok".into(),
        hpath: hpath.map(|h| h.into_iter().map(String::from).collect()),
        unterminated: None,
        info,
        node_rev: Some(wire::NodeRev("b3:00ff".into())),
    };
    vec![
        mk(
            NodeKind::Frontmatter,
            None,
            Some(Info::Frontmatter {
                keys: vec!["title".into(), "tags".into()],
            }),
        ),
        mk(NodeKind::Heading, Some(vec!["Section", "Sub"]), None),
        mk(
            NodeKind::Fence,
            None,
            Some(Info::Fence {
                info_string: "rust".into(),
            }),
        ),
        mk(NodeKind::InlineCode, None, None),
        mk(NodeKind::Anchor, None, None),
        mk(
            NodeKind::Wikilink,
            None,
            Some(Info::Wikilink {
                target: "notes/x".into(),
                heading: Some("H".into()),
                block: None,
                alias: Some("alias".into()),
            }),
        ),
        mk(
            NodeKind::Embed,
            None,
            Some(Info::Wikilink {
                target: "img/y.png".into(),
                heading: None,
                block: Some("blk-1".into()),
                alias: None,
            }),
        ),
        mk(
            NodeKind::Callout,
            None,
            Some(Info::Callout {
                r#type: "note".into(),
                fold: "+".into(),
            }),
        ),
        mk(
            NodeKind::Task,
            None,
            Some(Info::Task {
                checked: true,
                depth: 2,
            }),
        ),
        mk(NodeKind::Table, None, None),
        wire::Node {
            unterminated: Some(true),
            ..mk(NodeKind::Comment, None, None)
        },
    ]
}

fn sample_requests() -> Vec<wire::Request> {
    let ops = vec![
        wire::Op::Hello {
            proto: 1,
            client: Some("agreement-test".into()),
        },
        wire::Op::Toc {
            path: wire::Path("tasks/x.md".into()),
        },
        wire::Op::Extract {
            path: wire::Path("notes/y.md".into()),
            kinds: Some(vec!["heading".into(), "task".into()]),
        },
        wire::Op::Extract {
            path: wire::Path("notes/y.md".into()),
            kinds: None, // absent ≠ empty — the KindFilter wrapper's reason to exist
        },
        wire::Op::Resolve {
            path: wire::Path("notes/y.md".into()),
            r#ref: "#Section".into(),
        },
        wire::Op::Splice {
            path: wire::Path("notes/y.md".into()),
            span: wire::Span(10, 90),
            if_node_rev: wire::NodeRev("b3:aa11".into()),
            text: "replacement\n".into(),
        },
        wire::Op::Root { path: None },
        wire::Op::Guard {
            root: wire::Root("b3:88d2aa".into()),
            path: Some(wire::Path("notes".into())),
        },
    ];
    ops.into_iter()
        .enumerate()
        .map(|(i, op)| wire::Request {
            id: (i > 0).then_some(i as u64),
            op,
        })
        .collect()
}

fn sample_responses() -> Vec<wire::Response> {
    let mut bodies = vec![
        wire::ResponseBody::Hello {
            proto: 1,
            server: "meridian-sidecar".into(),
            caps: vec!["hello".into(), "toc".into(), "extract".into()],
        },
        wire::ResponseBody::Nodes {
            path: wire::Path("notes/y.md".into()),
            nodes: sample_nodes(),
        },
        wire::ResponseBody::Resolve {
            path: wire::Path("notes/y.md".into()),
            span: wire::Span(10, 90),
            node_rev: wire::NodeRev("b3:aa11".into()),
            content_span: Some(wire::Span(20, 90)),
        },
        wire::ResponseBody::Splice {
            span: wire::Span(10, 95),
            node_rev: wire::NodeRev("b3:bb22".into()),
        },
        wire::ResponseBody::Root {
            root: wire::Root("b3:88d2aa".into()),
        },
        wire::ResponseBody::Guard {
            root: wire::Root("b3:88d2aa".into()),
        },
    ];
    // every error code crosses the seam once; unsupported_proto carries extras
    for code in [
        wire::ErrorCode::BadFrame,
        wire::ErrorCode::BadRequest,
        wire::ErrorCode::UnknownOp,
        wire::ErrorCode::UnsupportedProto,
        wire::ErrorCode::BadPath,
        wire::ErrorCode::NotFound,
        wire::ErrorCode::InvalidUtf8,
        wire::ErrorCode::Internal,
        wire::ErrorCode::CasMismatch,
    ] {
        bodies.push(wire::ResponseBody::Error(wire::ErrorBody {
            error: code,
            message: Some(format!("{code:?}")),
            path: matches!(code, wire::ErrorCode::BadPath | wire::ErrorCode::NotFound)
                .then(|| wire::Path("bad/../path".into())),
            supported: matches!(code, wire::ErrorCode::UnsupportedProto).then(|| vec![1]),
            expected: matches!(code, wire::ErrorCode::CasMismatch)
                .then(|| wire::NodeRev("b3:aa11".into())),
            actual: matches!(code, wire::ErrorCode::CasMismatch)
                .then(|| wire::NodeRev("b3:cc33".into())),
        }));
    }
    bodies
        .into_iter()
        .enumerate()
        .map(|(i, body)| wire::Response {
            id: (i > 0).then_some(i as u64), // id:0 case → "id":null twin: absence
            ok: !matches!(body, wire::ResponseBody::Error(_)),
            body,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// the pin, exercised
// ---------------------------------------------------------------------------

fn roundtrip(frame: &pb::Frame) -> pb::Frame {
    let mut buf = Vec::new();
    transport_proto::encode(frame, &mut buf).expect("encode");
    let mut input: &[u8] = &buf;
    let decoded = transport_proto::decode(&mut input)
        .expect("decode")
        .expect("one frame");
    assert_eq!(
        transport_proto::decode(&mut input).expect("clean EOF"),
        None
    );
    decoded
}

#[test]
fn every_request_op_survives_wire_proto_wire() {
    for req in sample_requests() {
        let frame = pb::Frame {
            kind: Some(pb::frame::Kind::Request(request_to_pb(req.clone()))),
        };
        let Some(pb::frame::Kind::Request(back)) = roundtrip(&frame).kind else {
            panic!("request frame must decode as request")
        };
        assert_eq!(request_from_pb(back), req);
    }
}

#[test]
fn every_response_body_and_error_code_survives_wire_proto_wire() {
    for resp in sample_responses() {
        let frame = pb::Frame {
            kind: Some(pb::frame::Kind::Response(response_to_pb(resp.clone()))),
        };
        let Some(pb::frame::Kind::Response(back)) = roundtrip(&frame).kind else {
            panic!("response frame must decode as response")
        };
        assert_eq!(response_from_pb(back), resp);
    }
}
