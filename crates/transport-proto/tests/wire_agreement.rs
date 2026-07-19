//! The drift pin (law 3 as amended): `wire` is the canonical contract, the
//! `.proto` transcribes it, and THIS FILE is what makes the transcription
//! unbreakable by construction — every wire type is converted through
//! exhaustive struct destructuring and exhaustive matches, so adding a wire
//! variant or field stops compiling here until `meridian.proto` catches up
//! (and vice versa: a proto-only oneof arm breaks the reverse match).
//!
//! The runtime half proves the schema loses nothing: samples covering every
//! op, node kind, info shape, toc row shape, `SecRef` form, response body, and
//! error code round-trip wire → proto → encoded frame → proto → wire,
//! `assert_eq` with the original.
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

fn hpath_seg_to_pb(s: wire::HpathSeg) -> pb::HpathSeg {
    let wire::HpathSeg { h, n } = s;
    pb::HpathSeg { h, n }
}

fn sec_ref_to_pb(r: wire::SecRef) -> pb::SecRef {
    let form = match r {
        wire::SecRef::Hpath { hpath } => pb::sec_ref::Form::Hpath(pb::HpathRef {
            segs: hpath.into_iter().map(hpath_seg_to_pb).collect(),
        }),
        wire::SecRef::Anchor { anchor } => pb::sec_ref::Form::Anchor(anchor),
        wire::SecRef::FmKey { fm_key } => pb::sec_ref::Form::FmKey(fm_key),
    };
    pb::SecRef { form: Some(form) }
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
        wire::ErrorCode::RefNotFound => pb::ErrorCode::RefNotFound,
        wire::ErrorCode::AmbiguousRef => pb::ErrorCode::AmbiguousRef,
        wire::ErrorCode::RootMismatch => pb::ErrorCode::RootMismatch,
        wire::ErrorCode::RootUnknown => pb::ErrorCode::RootUnknown,
    }
}

fn recovery_to_pb(r: wire::Recovery) -> pb::Recovery {
    match r {
        wire::Recovery::Fix => pb::Recovery::Fix,
        wire::Recovery::Env => pb::Recovery::Env,
        wire::Recovery::Refresh => pb::Recovery::Refresh,
        wire::Recovery::Retry => pb::Recovery::Retry,
        wire::Recovery::Resync => pb::Recovery::Resync,
        wire::Recovery::Respawn => pb::Recovery::Respawn,
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
        // empty ≡ absent: a heading always carries ≥1 segment (contract §2.1)
        hpath: hpath
            .unwrap_or_default()
            .into_iter()
            .map(hpath_seg_to_pb)
            .collect(),
        unterminated,
        info: info.map(|i| pb::Info {
            info: Some(info_to_pb(i)),
        }),
        node_rev: node_rev.map(|r| r.0),
    }
}

fn toc_node_to_pb(n: wire::TocNode) -> pb::TocNode {
    let wire::TocNode {
        kind,
        level,
        hpath,
        anchor,
        span,
        content_span,
        node_rev,
        text_prefix_16b,
        keys,
    } = n;
    pb::TocNode {
        kind,
        level,
        // empty ≡ absent: a heading row always carries ≥1 segment
        hpath: hpath
            .unwrap_or_default()
            .into_iter()
            .map(hpath_seg_to_pb)
            .collect(),
        anchor,
        span: Some(span_to_pb(span)),
        content_span: content_span.map(span_to_pb),
        node_rev: node_rev.0,
        text_prefix_16b,
        // wrapper: present-but-empty vs absent is contractual (fm rows carry
        // keys even when empty)
        keys: keys.map(|keys| pb::KeyList { keys }),
    }
}

fn op_to_pb(op: wire::Op) -> pb::request::Op {
    match op {
        wire::Op::Hello { proto, client } => {
            pb::request::Op::Hello(pb::HelloRequest { proto, client })
        }
        wire::Op::Toc { path } => pb::request::Op::Toc(pb::TocRequest { path: path.0 }),
        wire::Op::Cat { path, sec } => pb::request::Op::Cat(pb::CatRequest {
            path: path.0,
            sec: sec.map(sec_ref_to_pb),
        }),
        wire::Op::Extract { path, kinds } => pb::request::Op::Extract(pb::ExtractRequest {
            path: path.0,
            kinds: kinds.map(|kinds| pb::KindFilter { kinds }),
        }),
        wire::Op::Resolve {
            from,
            r#ref,
            content,
        } => pb::request::Op::Resolve(pb::ResolveRequest {
            from: from.0,
            r#ref,
            content,
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
        wire::Op::Root => pb::request::Op::Root(pb::RootRequest {}),
        wire::Op::Diff { from_root, to_root } => pb::request::Op::Diff(pb::DiffRequest {
            from_root: from_root.0,
            to_root: to_root.0,
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
        code,
        recovery,
        message,
        path,
        supported,
        expected,
        actual,
        changed,
        stage,
        dest,
        candidates,
        unknown_kinds,
        id_raw,
    } = e;
    pb::ErrorBody {
        code: code_to_pb(code).into(),
        recovery: recovery_to_pb(recovery).into(),
        message,
        path: path.map(|p| p.0),
        // empty ≡ absent: a sidecar always speaks ≥1 proto
        supported: supported.unwrap_or_default(),
        expected: expected.map(|r| r.0),
        actual: actual.map(|r| r.0),
        stage,
        dest: dest.map(|p| p.0),
        // empty ≡ absent: ambiguity means ≥2 candidates
        candidates: candidates
            .unwrap_or_default()
            .into_iter()
            .map(sec_ref_to_pb)
            .collect(),
        // empty ≡ absent: a D-C5 refusal names ≥1 unknown kind
        unknown_kinds: unknown_kinds.unwrap_or_default(),
        id_raw,
        // empty ≡ absent: a root_mismatch names its drift
        changed: changed
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.0)
            .collect(),
    }
}

fn body_to_pb(b: wire::ResponseBody) -> pb::response::Body {
    match b {
        wire::ResponseBody::Hello {
            proto,
            server,
            caps,
            root,
        } => pb::response::Body::Hello(pb::HelloResponse {
            proto,
            server,
            caps,
            root: root.map(|r| r.0),
        }),
        wire::ResponseBody::Toc {
            path,
            file_rev,
            root,
            nodes,
        } => pb::response::Body::Toc(pb::TocResponse {
            path: path.0,
            file_rev: file_rev.0,
            root: root.0,
            nodes: nodes.into_iter().map(toc_node_to_pb).collect(),
        }),
        wire::ResponseBody::Nodes { path, nodes } => pb::response::Body::Nodes(pb::NodesResponse {
            path: path.0,
            nodes: nodes.into_iter().map(node_to_pb).collect(),
        }),
        wire::ResponseBody::Cat {
            span,
            node_rev,
            content,
        } => pb::response::Body::Cat(pb::CatResponse {
            span: Some(span_to_pb(span)),
            node_rev: node_rev.0,
            content,
        }),
        wire::ResponseBody::Resolve {
            dest,
            span,
            content,
        } => pb::response::Body::Resolve(pb::ResolveResponse {
            dest: dest.0,
            span: Some(span_to_pb(span)),
            content,
        }),
        wire::ResponseBody::Splice { span, node_rev } => {
            pb::response::Body::Splice(pb::SpliceResponse {
                span: Some(span_to_pb(span)),
                node_rev: node_rev.0,
            })
        }
        wire::ResponseBody::Root { root, seq } => {
            pb::response::Body::Root(pb::RootResponse { root: root.0, seq })
        }
    }
}

fn payload_to_pb(p: wire::ResponsePayload) -> pb::response::Body {
    match p {
        wire::ResponsePayload::Body { body } => body_to_pb(body),
        wire::ResponsePayload::Error { error } => pb::response::Body::Error(error_to_pb(error)),
    }
}

fn response_to_pb(r: wire::Response) -> pb::Response {
    let wire::Response { id, ok, payload } = r;
    pb::Response {
        id,
        ok,
        body: Some(payload_to_pb(payload)),
    }
}

// ---------------------------------------------------------------------------
// proto → wire (the reverse pin: a proto-only arm breaks these matches)
// ---------------------------------------------------------------------------

fn span_from_pb(s: pb::Span) -> wire::Span {
    let pb::Span { start, end } = s;
    wire::Span(start, end)
}

fn hpath_seg_from_pb(s: pb::HpathSeg) -> wire::HpathSeg {
    let pb::HpathSeg { h, n } = s;
    wire::HpathSeg { h, n }
}

fn sec_ref_from_pb(r: pb::SecRef) -> wire::SecRef {
    match r.form.expect("form oneof set") {
        pb::sec_ref::Form::Hpath(pb::HpathRef { segs }) => wire::SecRef::Hpath {
            hpath: segs.into_iter().map(hpath_seg_from_pb).collect(),
        },
        pb::sec_ref::Form::Anchor(anchor) => wire::SecRef::Anchor { anchor },
        pb::sec_ref::Form::FmKey(fm_key) => wire::SecRef::FmKey { fm_key },
    }
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
        pb::ErrorCode::RefNotFound => wire::ErrorCode::RefNotFound,
        pb::ErrorCode::AmbiguousRef => wire::ErrorCode::AmbiguousRef,
        pb::ErrorCode::RootMismatch => wire::ErrorCode::RootMismatch,
        pb::ErrorCode::RootUnknown => wire::ErrorCode::RootUnknown,
    }
}

fn recovery_from_pb(r: pb::Recovery) -> wire::Recovery {
    match r {
        pb::Recovery::Unspecified => panic!("UNSPECIFIED never crosses the seam"),
        pb::Recovery::Fix => wire::Recovery::Fix,
        pb::Recovery::Env => wire::Recovery::Env,
        pb::Recovery::Refresh => wire::Recovery::Refresh,
        pb::Recovery::Retry => wire::Recovery::Retry,
        pb::Recovery::Resync => wire::Recovery::Resync,
        pb::Recovery::Respawn => wire::Recovery::Respawn,
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
        hpath: if hpath.is_empty() {
            None
        } else {
            Some(hpath.into_iter().map(hpath_seg_from_pb).collect())
        },
        unterminated,
        info: info.map(|i| info_from_pb(i.info.expect("info oneof set"))),
        node_rev: node_rev.map(wire::NodeRev),
    }
}

fn toc_node_from_pb(n: pb::TocNode) -> wire::TocNode {
    let pb::TocNode {
        kind,
        level,
        hpath,
        anchor,
        span,
        content_span,
        node_rev,
        text_prefix_16b,
        keys,
    } = n;
    wire::TocNode {
        kind,
        level,
        hpath: if hpath.is_empty() {
            None
        } else {
            Some(hpath.into_iter().map(hpath_seg_from_pb).collect())
        },
        anchor,
        span: span_from_pb(span.expect("span is required")),
        content_span: content_span.map(span_from_pb),
        node_rev: wire::NodeRev(node_rev),
        text_prefix_16b,
        keys: keys.map(|k| k.keys),
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
        pb::request::Op::Cat(pb::CatRequest { path, sec }) => wire::Op::Cat {
            path: wire::Path(path),
            sec: sec.map(sec_ref_from_pb),
        },
        pb::request::Op::Extract(pb::ExtractRequest { path, kinds }) => wire::Op::Extract {
            path: wire::Path(path),
            kinds: kinds.map(|f| f.kinds),
        },
        pb::request::Op::Resolve(pb::ResolveRequest {
            from,
            r#ref,
            content,
        }) => wire::Op::Resolve {
            from: wire::Path(from),
            r#ref,
            content,
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
        pb::request::Op::Root(pb::RootRequest {}) => wire::Op::Root,
        pb::request::Op::Diff(pb::DiffRequest { from_root, to_root }) => wire::Op::Diff {
            from_root: wire::Root(from_root),
            to_root: wire::Root(to_root),
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
        code,
        recovery,
        message,
        path,
        supported,
        expected,
        actual,
        changed,
        stage,
        dest,
        candidates,
        unknown_kinds,
        id_raw,
    } = e;
    wire::ErrorBody {
        code: code_from_pb(pb::ErrorCode::try_from(code).expect("known code")),
        recovery: recovery_from_pb(pb::Recovery::try_from(recovery).expect("known class")),
        message,
        path: path.map(wire::Path),
        supported: if supported.is_empty() {
            None
        } else {
            Some(supported)
        },
        expected: expected.map(wire::NodeRev),
        actual: actual.map(wire::NodeRev),
        stage,
        dest: dest.map(wire::Path),
        candidates: if candidates.is_empty() {
            None
        } else {
            Some(candidates.into_iter().map(sec_ref_from_pb).collect())
        },
        unknown_kinds: if unknown_kinds.is_empty() {
            None
        } else {
            Some(unknown_kinds)
        },
        id_raw,
        changed: if changed.is_empty() {
            None
        } else {
            Some(changed.into_iter().map(wire::Path).collect())
        },
    }
}

fn payload_from_pb(b: pb::response::Body) -> wire::ResponsePayload {
    match b {
        pb::response::Body::Error(e) => wire::ResponsePayload::Error {
            error: error_from_pb(e),
        },
        success => wire::ResponsePayload::Body {
            body: body_from_pb(success),
        },
    }
}

fn body_from_pb(b: pb::response::Body) -> wire::ResponseBody {
    match b {
        pb::response::Body::Hello(pb::HelloResponse {
            proto,
            server,
            caps,
            root,
        }) => wire::ResponseBody::Hello {
            proto,
            server,
            caps,
            root: root.map(wire::Root),
        },
        pb::response::Body::Toc(pb::TocResponse {
            path,
            file_rev,
            root,
            nodes,
        }) => wire::ResponseBody::Toc {
            path: wire::Path(path),
            file_rev: wire::NodeRev(file_rev),
            root: wire::Root(root),
            nodes: nodes.into_iter().map(toc_node_from_pb).collect(),
        },
        pb::response::Body::Nodes(pb::NodesResponse { path, nodes }) => wire::ResponseBody::Nodes {
            path: wire::Path(path),
            nodes: nodes.into_iter().map(node_from_pb).collect(),
        },
        pb::response::Body::Cat(pb::CatResponse {
            span,
            node_rev,
            content,
        }) => wire::ResponseBody::Cat {
            span: span_from_pb(span.expect("span is required")),
            node_rev: wire::NodeRev(node_rev),
            content,
        },
        pb::response::Body::Resolve(pb::ResolveResponse {
            dest,
            span,
            content,
        }) => wire::ResponseBody::Resolve {
            dest: wire::Path(dest),
            span: span_from_pb(span.expect("span is required")),
            content,
        },
        pb::response::Body::Splice(pb::SpliceResponse { span, node_rev }) => {
            wire::ResponseBody::Splice {
                span: span_from_pb(span.expect("span is required")),
                node_rev: wire::NodeRev(node_rev),
            }
        }
        pb::response::Body::Root(pb::RootResponse { root, seq }) => wire::ResponseBody::Root {
            root: wire::Root(root),
            seq,
        },
        pb::response::Body::Error(_) => unreachable!("payload_from_pb routes the error arm"),
    }
}

fn response_from_pb(r: pb::Response) -> wire::Response {
    let pb::Response { id, ok, body } = r;
    wire::Response {
        id,
        ok,
        payload: payload_from_pb(body.expect("body oneof set")),
    }
}

// ---------------------------------------------------------------------------
// samples — every op, node kind + info shape, toc row shape, SecRef form,
// body, and code
// ---------------------------------------------------------------------------

fn seg(h: &str) -> wire::HpathSeg {
    wire::HpathSeg {
        h: h.into(),
        n: None,
    }
}

fn sample_nodes() -> Vec<wire::Node> {
    use wire::{Info, NodeKind};
    let mk = |kind: NodeKind, hpath: Option<Vec<wire::HpathSeg>>, info: Option<Info>| wire::Node {
        kind,
        span: wire::Span(0, 128),
        text_prefix_16b: "sixteen bytes ok".into(),
        hpath,
        unterminated: None,
        info,
        node_rev: Some(wire::NodeRev("26796ebec5d0bf1a".into())),
    };
    vec![
        mk(
            NodeKind::Frontmatter,
            None,
            Some(Info::Frontmatter {
                keys: vec!["title".into(), "tags".into()],
            }),
        ),
        mk(
            NodeKind::Heading,
            Some(vec![
                seg("Section"),
                wire::HpathSeg {
                    h: "Beta".into(),
                    n: Some(2), // occurrence index survives the seam
                },
            ]),
            None,
        ),
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

/// The three §4.1 row shapes: frontmatter (keys — incl. the present-but-empty
/// case the `KeyList` wrapper exists for), heading (level/hpath/`content_span`),
/// and the anchor-bearing host-block row (open kind string).
fn sample_toc_nodes() -> Vec<wire::TocNode> {
    vec![
        wire::TocNode {
            kind: "frontmatter".into(),
            level: None,
            hpath: None,
            anchor: None,
            span: wire::Span(0, 20),
            content_span: None,
            node_rev: wire::NodeRev("26796ebec5d0bf1a".into()),
            text_prefix_16b: "---\ntitle: Plan\n".into(),
            keys: Some(vec!["title".into()]),
        },
        wire::TocNode {
            kind: "frontmatter".into(),
            level: None,
            hpath: None,
            anchor: None,
            span: wire::Span(0, 8),
            content_span: None,
            node_rev: wire::NodeRev("ffffffffffffffff".into()),
            text_prefix_16b: "---\n---\n".into(),
            keys: Some(vec![]), // present-but-empty ≠ absent
        },
        wire::TocNode {
            kind: "heading".into(),
            level: Some(2),
            hpath: Some(vec![seg("Goals"), seg("Q3")]),
            anchor: None,
            span: wire::Span(49, 72),
            content_span: Some(wire::Span(55, 72)),
            node_rev: wire::NodeRev("33d5b0e1b27cb48b".into()),
            text_prefix_16b: "## Q3\n\nship by A".into(),
            keys: None,
        },
        wire::TocNode {
            kind: "list_item".into(), // the §4.1 worked anchor row: open kind
            level: None,
            hpath: None,
            anchor: Some("r-000042".into()),
            span: wire::Span(26, 248),
            content_span: None,
            node_rev: wire::NodeRev("639a2dca46f6fcc8".into()),
            text_prefix_16b: "- splice notes/p".into(),
            keys: None,
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
        // cat: all three SecRef forms + the whole-file case
        wire::Op::Cat {
            path: wire::Path("notes/plan.md".into()),
            sec: Some(wire::SecRef::Hpath {
                hpath: vec![seg("Goals"), seg("Q3")],
            }),
        },
        wire::Op::Cat {
            path: wire::Path("receipts/2026-07-18.md".into()),
            sec: Some(wire::SecRef::Anchor {
                anchor: "r-000042".into(),
            }),
        },
        wire::Op::Cat {
            path: wire::Path("notes/plan.md".into()),
            sec: Some(wire::SecRef::FmKey {
                fm_key: "title".into(),
            }),
        },
        wire::Op::Cat {
            path: wire::Path("notes/plan.md".into()),
            sec: None, // whole file + file_rev
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
            from: wire::Path("notes/plan.md".into()),
            r#ref: "plan#Goals#Q3".into(),
            content: None,
        },
        wire::Op::Resolve {
            from: wire::Path("notes/plan.md".into()),
            r#ref: "2026-07-18".into(),
            content: Some(true),
        },
        wire::Op::Splice {
            path: wire::Path("notes/y.md".into()),
            span: wire::Span(10, 90),
            if_node_rev: wire::NodeRev("33d5b0e1b27cb48b".into()),
            text: "replacement\n".into(),
        },
        wire::Op::Root,
        wire::Op::Diff {
            from_root: wire::Root(
                "b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9".into(),
            ),
            to_root: wire::Root(
                "b3:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68".into(),
            ),
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

/// Every error code crosses the seam once, recovery bound per §8;
/// code-specific extras ride their codes.
fn sample_error_payloads() -> Vec<wire::ResponsePayload> {
    let codes = [
        wire::ErrorCode::RootMismatch,
        wire::ErrorCode::RootUnknown,
        wire::ErrorCode::BadFrame,
        wire::ErrorCode::BadRequest,
        wire::ErrorCode::UnknownOp,
        wire::ErrorCode::UnsupportedProto,
        wire::ErrorCode::BadPath,
        wire::ErrorCode::NotFound,
        wire::ErrorCode::InvalidUtf8,
        wire::ErrorCode::Internal,
        wire::ErrorCode::CasMismatch,
        wire::ErrorCode::RefNotFound,
        wire::ErrorCode::AmbiguousRef,
    ];
    codes
        .into_iter()
        .map(|code| {
            let mut e = wire::ErrorBody::new(code);
            e.message = Some(format!("{code:?}"));
            e.path = matches!(code, wire::ErrorCode::BadPath | wire::ErrorCode::NotFound)
                .then(|| wire::Path("bad/../path".into()));
            e.supported = matches!(code, wire::ErrorCode::UnsupportedProto).then(|| vec![1]);
            e.expected = matches!(code, wire::ErrorCode::CasMismatch)
                .then(|| wire::NodeRev("33d5b0e1b27cb48b".into()));
            e.actual = matches!(code, wire::ErrorCode::CasMismatch)
                .then(|| wire::NodeRev("41f643f034e5681f".into()));
            e.stage = matches!(code, wire::ErrorCode::RefNotFound).then_some(2);
            e.dest = matches!(code, wire::ErrorCode::RefNotFound)
                .then(|| wire::Path("notes/plan.md".into()));
            e.candidates = matches!(code, wire::ErrorCode::AmbiguousRef).then(|| {
                vec![
                    wire::SecRef::Hpath {
                        hpath: vec![wire::HpathSeg {
                            h: "Beta".into(),
                            n: Some(1),
                        }],
                    },
                    wire::SecRef::Hpath {
                        hpath: vec![wire::HpathSeg {
                            h: "Beta".into(),
                            n: Some(2),
                        }],
                    },
                ]
            });
            e.unknown_kinds =
                matches!(code, wire::ErrorCode::BadRequest).then(|| vec!["headding".into()]);
            e.id_raw = matches!(code, wire::ErrorCode::BadRequest).then(|| "3e0".into());
            if matches!(code, wire::ErrorCode::RootMismatch) {
                e.expected = Some(wire::NodeRev(
                    "b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9".into(),
                ));
                e.actual = Some(wire::NodeRev(
                    "b3:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68".into(),
                ));
                e.changed = Some(vec![wire::Path("notes/plan.md".into())]);
            }
            wire::ResponsePayload::Error { error: e }
        })
        .collect()
}

fn sample_responses() -> Vec<wire::Response> {
    let root =
        wire::Root("b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9".into());
    let bodies = vec![
        wire::ResponseBody::Hello {
            proto: 1,
            server: "meridian-sidecar".into(),
            caps: vec![
                "hello".into(),
                "toc".into(),
                "cat".into(),
                "resolve.content".into(),
            ],
            root: Some(root.clone()),
        },
        wire::ResponseBody::Hello {
            proto: 1,
            server: "meridian-sidecar".into(),
            caps: vec!["hello".into()],
            root: None, // the engine may not have walked yet
        },
        wire::ResponseBody::Toc {
            path: wire::Path("notes/plan.md".into()),
            file_rev: wire::NodeRev("e3c4acaceb75b907".into()),
            root: root.clone(),
            nodes: sample_toc_nodes(),
        },
        wire::ResponseBody::Nodes {
            path: wire::Path("notes/y.md".into()),
            nodes: sample_nodes(),
        },
        wire::ResponseBody::Cat {
            span: wire::Span(49, 72),
            node_rev: wire::NodeRev("33d5b0e1b27cb48b".into()),
            content: "## Q3\n\nship by August\n\n".into(),
        },
        wire::ResponseBody::Resolve {
            dest: wire::Path("notes/plan.md".into()),
            span: wire::Span(49, 75),
            content: None,
        },
        wire::ResponseBody::Resolve {
            dest: wire::Path("receipts/2026-07-18.md".into()),
            span: wire::Span(0, 474),
            content: Some("…fragment bytes…".into()), // still no rev (D-C2)
        },
        wire::ResponseBody::Splice {
            span: wire::Span(10, 95),
            node_rev: wire::NodeRev("41f643f034e5681f".into()),
        },
        wire::ResponseBody::Root { root, seq: 2 },
    ];
    let mut payloads: Vec<wire::ResponsePayload> = bodies
        .into_iter()
        .map(|body| wire::ResponsePayload::Body { body })
        .collect();
    payloads.extend(sample_error_payloads());
    payloads
        .into_iter()
        .enumerate()
        .map(|(i, payload)| wire::Response {
            id: (i > 0).then_some(i as u64), // id:0 case → "id":null twin: absence
            ok: !matches!(payload, wire::ResponsePayload::Error { .. }),
            payload,
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
