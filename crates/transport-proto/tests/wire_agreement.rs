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

fn receipt_addr_to_pb(r: wire::ReceiptAddr) -> pb::ReceiptAddr {
    let wire::ReceiptAddr { path, anchor } = r;
    pb::ReceiptAddr {
        path: path.0,
        anchor,
    }
}

fn put_at_to_pb(a: wire::PutAt) -> pb::PutAt {
    match a {
        wire::PutAt::All => pb::PutAt::All,
        wire::PutAt::Content => pb::PutAt::Content,
        wire::PutAt::End => pb::PutAt::End,
        wire::PutAt::Upsert => pb::PutAt::Upsert,
    }
}

fn edit_shape_to_pb(e: wire::EditShape) -> pb::edit_shape::Shape {
    match e {
        wire::EditShape::Match { old, new } => {
            pb::edit_shape::Shape::Match(pb::MatchEdit { old, new })
        }
        wire::EditShape::Put { at, text } => pb::edit_shape::Shape::Put(pb::PutEdit {
            at: put_at_to_pb(at).into(),
            text,
        }),
    }
}

fn edit_to_pb(e: wire::Edit) -> pb::Edit {
    let wire::Edit {
        target,
        edit,
        if_node_rev,
    } = e;
    pb::Edit {
        target: Some(sec_ref_to_pb(target)),
        edit: Some(pb::EditShape {
            shape: Some(edit_shape_to_pb(edit)),
        }),
        if_node_rev: if_node_rev.map(|r| r.0),
    }
}

fn armed_edit_to_pb(e: wire::ArmedEdit) -> pb::ArmedEdit {
    let wire::ArmedEdit {
        target,
        node_rev_before,
        node_rev_after,
        span_after,
    } = e;
    pb::ArmedEdit {
        target: Some(sec_ref_to_pb(target)),
        node_rev_before: node_rev_before.0,
        node_rev_after: node_rev_after.0,
        span_after: Some(span_to_pb(span_after)),
    }
}

fn armed_to_pb(a: wire::Armed) -> pb::Armed {
    let wire::Armed {
        path,
        edits,
        file_rev_after,
    } = a;
    pb::Armed {
        path: path.0,
        edits: edits.into_iter().map(armed_edit_to_pb).collect(),
        file_rev_after: file_rev_after.map(|r| r.0),
    }
}

fn receipt_fact_to_pb(r: wire::ReceiptFact) -> pb::ReceiptFact {
    let wire::ReceiptFact {
        path,
        anchor,
        node_rev,
        span_after,
    } = r;
    pb::ReceiptFact {
        path: path.0,
        anchor,
        node_rev: node_rev.0,
        span_after: Some(span_to_pb(span_after)),
    }
}

fn verdict_to_pb(v: wire::Verdict) -> pb::Verdict {
    let wire::Verdict {
        rule,
        severity,
        path,
        hpath,
        span,
        node_rev,
        message,
    } = v;
    pb::Verdict {
        rule,
        severity: severity_to_pb(severity).into(),
        path: path.0,
        // `hpath` empty ≡ a rule-level finding (wire `None`); proto3 repeated
        // carries no optional, so absence maps to the empty list (round-trips).
        hpath: hpath
            .unwrap_or_default()
            .into_iter()
            .map(hpath_seg_to_pb)
            .collect(),
        span: Some(span_to_pb(span)),
        node_rev: node_rev.0,
        message,
    }
}

fn severity_to_pb(s: wire::Severity) -> pb::Severity {
    match s {
        wire::Severity::Error => pb::Severity::Error,
        wire::Severity::Warn => pb::Severity::Warn,
        wire::Severity::Info => pb::Severity::Info,
    }
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
        wire::ErrorCode::NoMatch => pb::ErrorCode::NoMatch,
        wire::ErrorCode::NotUnique => pb::ErrorCode::NotUnique,
        wire::ErrorCode::WouldCorrupt => pb::ErrorCode::WouldCorrupt,
        wire::ErrorCode::LockTimeout => pb::ErrorCode::LockTimeout,
        wire::ErrorCode::FileNotFound => pb::ErrorCode::FileNotFound,
        wire::ErrorCode::IoError => pb::ErrorCode::IoError,
        wire::ErrorCode::InvalidUtf8 => pb::ErrorCode::InvalidUtf8,
        wire::ErrorCode::Internal => pb::ErrorCode::Internal,
        wire::ErrorCode::CasMismatch => pb::ErrorCode::CasMismatch,
        wire::ErrorCode::RefNotFound => pb::ErrorCode::RefNotFound,
        wire::ErrorCode::AmbiguousRef => pb::ErrorCode::AmbiguousRef,
        wire::ErrorCode::RootMismatch => pb::ErrorCode::RootMismatch,
        wire::ErrorCode::RootUnknown => pb::ErrorCode::RootUnknown,
        wire::ErrorCode::StaleView => pb::ErrorCode::StaleView,
        wire::ErrorCode::DaemonOnly => pb::ErrorCode::DaemonOnly,
        wire::ErrorCode::ConventionFault => pb::ErrorCode::ConventionFault,
        wire::ErrorCode::ArmedDrift => pb::ErrorCode::ArmedDrift,
        wire::ErrorCode::BindingBreak => pb::ErrorCode::BindingBreak,
        wire::ErrorCode::IndexIntegrity => pb::ErrorCode::IndexIntegrity,
        wire::ErrorCode::WriteConflict => pb::ErrorCode::WriteConflict,
        wire::ErrorCode::WorkspaceBusy => pb::ErrorCode::WorkspaceBusy,
        // Stage-2 S7: v3-pin-path-only codes, outside the proto agreement
        // surface — the same rule `plan_edits` follows. No v2 sample can
        // carry one, so the frozen proto enum is untouched.
        wire::ErrorCode::ReadMintRequired | wire::ErrorCode::PinTargetMissing => {
            panic!("{c:?} is v3-pin-face only — not in the proto agreement")
        }
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
        // v3-additive JSON-face addressing facts (M1 U2): deliberately
        // OUTSIDE the proto agreement surface — the pb mirror stays
        // v2-shaped; a future amendment adds them to both sides at once.
        n: _,
        hpath_text: _,
        words: _,
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
        wire::Op::Hello {
            proto,
            client,
            contract,
            workspace,
        } => pb::request::Op::Hello(pb::HelloRequest {
            proto,
            client,
            contract,
            workspace,
        }),
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
            actor,
            now,
            receipt,
            if_root,
            dry,
            force,
            edits,
            plan_edits,
            pin,
        } => {
            // v3-only JSON-face field (M1 U8b): outside the proto agreement
            // surface, same rule as `read`/`check_write` — the v2 samples
            // never carry it.
            assert!(
                plan_edits.is_empty(),
                "`plan_edits` is v3-JSON-face only — not in the proto agreement"
            );
            // Stage-2 S7 `splice.pin`: same rule.
            assert!(
                pin.is_none(),
                "`pin` is v3-JSON-face only — not in the proto agreement"
            );
            pb::request::Op::Splice(pb::SpliceRequest {
                path: path.0,
                actor,
                now,
                receipt: receipt.map(receipt_addr_to_pb),
                if_root: if_root.map(|r| r.0),
                dry,
                force,
                edits: edits.into_iter().map(edit_to_pb).collect(),
            })
        }
        wire::Op::Root => pb::request::Op::Root(pb::RootRequest {}),
        wire::Op::Diff { from_root, to_root } => pb::request::Op::Diff(pb::DiffRequest {
            from_root: from_root.0,
            to_root: to_root.0,
        }),
        wire::Op::Links { path, require_root } => pb::request::Op::Links(pb::LinksRequest {
            path: path.map(|p| p.0),
            require_root: require_root.map(|r| r.0),
        }),
        wire::Op::Sub { from_seq } => pb::request::Op::Sub(pb::SubRequest { from_seq }),
        wire::Op::ViewPath { cwd, fresh } => {
            pb::request::Op::ViewPath(pb::ViewPathRequest { cwd, fresh })
        }
        // The BIRTH op is mirrored because it MUTATES THE CORPUS. Ruled
        // 2026-07-26 (`decisions/2026-07-26-proto-mirror-ruling.md`, bronze):
        // every op that mutates the corpus and advances the root MUST be
        // mirrored, so the binary path can always perform every governed
        // mutation. A transport that could splice but not birth would carry a
        // lame contract. The "pb mirror stays v2-shaped" reading was found to
        // be no law at all — it bootstrapped from the `read` arm citing itself.
        wire::Op::Create {
            path,
            body,
            actor,
            now,
            if_root,
            dry,
        } => pb::request::Op::Create(pb::CreateRequest {
            path: path.0,
            body,
            actor,
            now,
            if_root: if_root.map(|r| r.0),
            dry,
        }),
        // A HOST-FACE op: DEFERRED, not excluded by principle (ruling §3,
        // `decisions/2026-07-26-proto-mirror-ruling.md`). `read` mutates
        // nothing, so the mandatory-membership rule does not reach it — but
        // nothing bars it either. It joins BOTH SIDES AT ONCE via a future
        // amendment if the binary path comes to need it, which is what this
        // arm's own wording has promised since M1. No sample feeds it today.
        wire::Op::Read { .. } => {
            unreachable!("composed `read` is v3-JSON-face only — not in the proto agreement")
        }
        // Also host-face — it computes a verdict and never writes. Deferred on
        // the same terms as `read` above, not excluded by rule.
        wire::Op::CheckWrite { .. } => {
            unreachable!("`check_write` is v3-JSON-face only — not in the proto agreement")
        }
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
        matches,
        lost,
        cause,
        overlap,
        required,
        as_of_root,
        live_root,
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
        matches,
        // empty ≡ absent: a would_corrupt names ≥1 lost hpath
        lost: lost
            .unwrap_or_default()
            .into_iter()
            .map(|segs| pb::HpathRef {
                segs: segs.into_iter().map(hpath_seg_to_pb).collect(),
            })
            .collect(),
        cause,
        // empty ≡ absent: an overlap names ≥2 targets
        overlap: overlap
            .unwrap_or_default()
            .into_iter()
            .map(sec_ref_to_pb)
            .collect(),
        required: required.map(|r| r.0),
        as_of_root: as_of_root.map(|r| r.0),
        live_root: live_root.map(|r| r.0),
    }
}

#[allow(clippy::too_many_lines)] // one exhaustive match arm per response body — the drift pin
fn body_to_pb(b: wire::ResponseBody) -> pb::response::Body {
    match b {
        wire::ResponseBody::Hello {
            proto,
            server,
            caps,
            root,
            storage,
            workspace,
        } => pb::response::Body::Hello(pb::HelloResponse {
            proto,
            server,
            caps,
            root: root.map(|r| r.0),
            storage,
            workspace,
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
        // The birth reply — mirrored with its request under the mandatory
        // mutation-membership rule (see `op_to_pb`'s `Op::Create` arm).
        wire::ResponseBody::Create {
            path,
            file_rev_after,
            root_before,
            root_after,
            seq,
            dry,
            journal_anchor,
            verdicts,
        } => pb::response::Body::Create(pb::CreateResponse {
            path: path.0,
            file_rev_after: file_rev_after.0,
            root_before: root_before.0,
            root_after: root_after.map(|r| r.0),
            seq,
            dry,
            journal_anchor,
            verdicts: verdicts.into_iter().map(verdict_to_pb).collect(),
        }),
        wire::ResponseBody::Splice {
            armed,
            receipt,
            root_before,
            root_after,
            seq,
            dry,
            verdicts,
            pin,
        } => {
            // Stage-2 S7: the pin FACT is v3-JSON-face only, like the request
            // field that mints it.
            assert!(
                pin.is_none(),
                "`pin` is v3-JSON-face only — not in the proto agreement"
            );
            pb::response::Body::Splice(pb::SpliceResponse {
                armed: Some(armed_to_pb(armed)),
                receipt: receipt.map(receipt_fact_to_pb),
                root_before: root_before.0,
                root_after: root_after.map(|r| r.0),
                seq,
                dry,
                verdicts: verdicts.into_iter().map(verdict_to_pb).collect(),
            })
        }
        wire::ResponseBody::Root { root, seq } => {
            pb::response::Body::Root(pb::RootResponse { root: root.0, seq })
        }
        wire::ResponseBody::Diff { batches } => pb::response::Body::Diff(pb::DiffResponse {
            batches: batches.into_iter().map(delta_frame_to_pb).collect(),
        }),
        wire::ResponseBody::Links {
            as_of_root,
            live_root,
            changes_seq,
            files,
        } => pb::response::Body::Links(pb::LinksResponse {
            as_of_root: as_of_root.0,
            live_root: live_root.0,
            changes_seq,
            files: files
                .into_iter()
                .map(|(p, f)| (p, file_links_to_pb(f)))
                .collect(),
        }),
        wire::ResponseBody::ViewPath {
            path,
            as_of_root,
            live_root,
            changes_seq,
            state,
            live_source,
            stale,
            refresh_in_progress,
            last_error,
        } => pb::response::Body::ViewPath(pb::ViewPathResponse {
            path,
            as_of_root: as_of_root.0,
            live_root: live_root.0,
            changes_seq,
            state: view_state_to_pb(state).into(),
            live_source: view_live_source_to_pb(live_source).into(),
            stale,
            refresh_in_progress,
            last_error: last_error.map(refresh_error_to_pb),
        }),
        // v3-only JSON-face body (M1 U4a2): outside the proto agreement
        // surface — the pb mirror stays v2-shaped, no sample feeds it here.
        wire::ResponseBody::Read { .. } => {
            unreachable!("composed `read` is v3-JSON-face only — not in the proto agreement")
        }
        // v3-only JSON-face body (M1 U8c): same rule as `read`.
        wire::ResponseBody::CheckWrite { .. } => {
            unreachable!("`check_write` is v3-JSON-face only — not in the proto agreement")
        }
    }
}

fn view_state_to_pb(s: wire::ViewState) -> pb::ViewState {
    match s {
        wire::ViewState::FreshAtSample => pb::ViewState::FreshAtSample,
        wire::ViewState::Stale => pb::ViewState::Stale,
        wire::ViewState::Raced => pb::ViewState::Raced,
    }
}

fn view_live_source_to_pb(s: wire::ViewLiveSource) -> pb::ViewLiveSource {
    match s {
        wire::ViewLiveSource::Fold => pb::ViewLiveSource::Fold,
        wire::ViewLiveSource::Watch => pb::ViewLiveSource::Watch,
        wire::ViewLiveSource::None => pb::ViewLiveSource::None,
    }
}

fn refresh_error_code_to_pb(c: wire::RefreshErrorCode) -> pb::RefreshErrorCode {
    match c {
        wire::RefreshErrorCode::ParseError => pb::RefreshErrorCode::ParseError,
        wire::RefreshErrorCode::DiskFull => pb::RefreshErrorCode::DiskFull,
        wire::RefreshErrorCode::Oom => pb::RefreshErrorCode::Oom,
        wire::RefreshErrorCode::Timeout => pb::RefreshErrorCode::Timeout,
        wire::RefreshErrorCode::Io => pb::RefreshErrorCode::Io,
    }
}

fn refresh_error_to_pb(e: wire::RefreshError) -> pb::RefreshError {
    let wire::RefreshError {
        code,
        unix,
        fingerprint_attempted,
        message,
    } = e;
    pb::RefreshError {
        code: refresh_error_code_to_pb(code).into(),
        unix,
        fingerprint_attempted: fingerprint_attempted.map(|r| r.0),
        message,
    }
}

fn file_links_to_pb(f: wire::FileLinks) -> pb::FileLinks {
    let wire::FileLinks {
        resolved,
        unresolved,
    } = f;
    pb::FileLinks {
        resolved: resolved.into_iter().collect(),
        unresolved: unresolved.into_iter().collect(),
    }
}

fn file_links_from_pb(f: pb::FileLinks) -> wire::FileLinks {
    let pb::FileLinks {
        resolved,
        unresolved,
    } = f;
    wire::FileLinks {
        resolved: resolved.into_iter().collect(),
        unresolved: unresolved.into_iter().collect(),
    }
}

fn file_change_to_pb(c: wire::FileChange) -> pb::FileChange {
    match c {
        wire::FileChange::Created => pb::FileChange::Created,
        wire::FileChange::Modified => pb::FileChange::Modified,
        wire::FileChange::Deleted => pb::FileChange::Deleted,
        wire::FileChange::Renamed => pb::FileChange::Renamed,
    }
}

fn node_change_to_pb(c: wire::NodeChange) -> pb::NodeChange {
    match c {
        wire::NodeChange::Added => pb::NodeChange::Added,
        wire::NodeChange::Edited => pb::NodeChange::Edited,
        wire::NodeChange::Removed => pb::NodeChange::Removed,
    }
}

fn delta_node_to_pb(n: wire::DeltaNode) -> pb::DeltaNode {
    let wire::DeltaNode {
        target,
        change,
        node_rev_before,
        node_rev_after,
        span_after,
    } = n;
    pb::DeltaNode {
        target: Some(sec_ref_to_pb(target)),
        change: node_change_to_pb(change) as i32,
        node_rev_before: node_rev_before.map(|r| r.0),
        node_rev_after: node_rev_after.map(|r| r.0),
        span_after: span_after.map(span_to_pb),
    }
}

fn delta_file_to_pb(f: wire::DeltaFile) -> pb::DeltaFile {
    let wire::DeltaFile {
        path,
        change,
        from_path,
        file_rev_before,
        file_rev_after,
        nodes,
    } = f;
    pb::DeltaFile {
        path: path.0,
        change: file_change_to_pb(change) as i32,
        from_path: from_path.map(|p| p.0),
        file_rev_before: file_rev_before.map(|r| r.0),
        file_rev_after: file_rev_after.map(|r| r.0),
        nodes: nodes.into_iter().map(delta_node_to_pb).collect(),
    }
}

fn delta_frame_to_pb(f: wire::DeltaFrame) -> pb::DeltaFrame {
    let wire::Delta {
        seq,
        root_before,
        root_after,
        actor,
        now,
        files,
    } = f.delta;
    pb::DeltaFrame {
        delta: Some(pb::Delta {
            seq,
            root_before: root_before.0,
            root_after: root_after.0,
            actor,
            now,
            files: files.into_iter().map(delta_file_to_pb).collect(),
        }),
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

fn receipt_addr_from_pb(r: pb::ReceiptAddr) -> wire::ReceiptAddr {
    let pb::ReceiptAddr { path, anchor } = r;
    wire::ReceiptAddr {
        path: wire::Path(path),
        anchor,
    }
}

fn put_at_from_pb(a: pb::PutAt) -> wire::PutAt {
    match a {
        pb::PutAt::Unspecified => panic!("UNSPECIFIED never crosses the seam"),
        pb::PutAt::All => wire::PutAt::All,
        pb::PutAt::Content => wire::PutAt::Content,
        pb::PutAt::End => wire::PutAt::End,
        pb::PutAt::Upsert => wire::PutAt::Upsert,
    }
}

fn edit_shape_from_pb(e: pb::edit_shape::Shape) -> wire::EditShape {
    match e {
        pb::edit_shape::Shape::Match(pb::MatchEdit { old, new }) => {
            wire::EditShape::Match { old, new }
        }
        pb::edit_shape::Shape::Put(pb::PutEdit { at, text }) => wire::EditShape::Put {
            at: put_at_from_pb(pb::PutAt::try_from(at).expect("known position")),
            text,
        },
    }
}

fn edit_from_pb(e: pb::Edit) -> wire::Edit {
    let pb::Edit {
        target,
        edit,
        if_node_rev,
    } = e;
    wire::Edit {
        target: sec_ref_from_pb(target.expect("target is required")),
        edit: edit_shape_from_pb(
            edit.expect("edit is required")
                .shape
                .expect("shape oneof set"),
        ),
        if_node_rev: if_node_rev.map(wire::NodeRev),
    }
}

fn armed_edit_from_pb(e: pb::ArmedEdit) -> wire::ArmedEdit {
    let pb::ArmedEdit {
        target,
        node_rev_before,
        node_rev_after,
        span_after,
    } = e;
    wire::ArmedEdit {
        target: sec_ref_from_pb(target.expect("target is required")),
        node_rev_before: wire::NodeRev(node_rev_before),
        node_rev_after: wire::NodeRev(node_rev_after),
        span_after: span_from_pb(span_after.expect("span is required")),
    }
}

fn armed_from_pb(a: pb::Armed) -> wire::Armed {
    let pb::Armed {
        path,
        edits,
        file_rev_after,
    } = a;
    wire::Armed {
        path: wire::Path(path),
        edits: edits.into_iter().map(armed_edit_from_pb).collect(),
        file_rev_after: file_rev_after.map(wire::NodeRev),
    }
}

fn receipt_fact_from_pb(r: pb::ReceiptFact) -> wire::ReceiptFact {
    let pb::ReceiptFact {
        path,
        anchor,
        node_rev,
        span_after,
    } = r;
    wire::ReceiptFact {
        path: wire::Path(path),
        anchor,
        node_rev: wire::NodeRev(node_rev),
        span_after: span_from_pb(span_after.expect("span is required")),
    }
}

fn verdict_from_pb(v: pb::Verdict) -> wire::Verdict {
    let pb::Verdict {
        rule,
        severity,
        path,
        hpath,
        span,
        node_rev,
        message,
    } = v;
    wire::Verdict {
        rule,
        severity: severity_from_pb(pb::Severity::try_from(severity).expect("known severity")),
        path: wire::Path(path),
        hpath: if hpath.is_empty() {
            None
        } else {
            Some(hpath.into_iter().map(hpath_seg_from_pb).collect())
        },
        span: span
            .map(span_from_pb)
            .expect("a verdict carries a span (§11.1)"),
        node_rev: wire::NodeRev(node_rev),
        message,
    }
}

fn severity_from_pb(s: pb::Severity) -> wire::Severity {
    match s {
        // proto3 zero-value convention only — no wire counterpart; the strict
        // seam refuses it LOUD rather than silently defaulting (P6-VERDICTS
        // precision). Pinned by `unspecified_severity_refuses_loud`.
        pb::Severity::Unspecified => panic!("SEVERITY_UNSPECIFIED never crosses the seam"),
        pb::Severity::Error => wire::Severity::Error,
        pb::Severity::Warn => wire::Severity::Warn,
        pb::Severity::Info => wire::Severity::Info,
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
        pb::ErrorCode::NoMatch => wire::ErrorCode::NoMatch,
        pb::ErrorCode::NotUnique => wire::ErrorCode::NotUnique,
        pb::ErrorCode::WouldCorrupt => wire::ErrorCode::WouldCorrupt,
        pb::ErrorCode::LockTimeout => wire::ErrorCode::LockTimeout,
        pb::ErrorCode::FileNotFound => wire::ErrorCode::FileNotFound,
        pb::ErrorCode::IoError => wire::ErrorCode::IoError,
        pb::ErrorCode::InvalidUtf8 => wire::ErrorCode::InvalidUtf8,
        pb::ErrorCode::Internal => wire::ErrorCode::Internal,
        pb::ErrorCode::CasMismatch => wire::ErrorCode::CasMismatch,
        pb::ErrorCode::RefNotFound => wire::ErrorCode::RefNotFound,
        pb::ErrorCode::AmbiguousRef => wire::ErrorCode::AmbiguousRef,
        pb::ErrorCode::RootMismatch => wire::ErrorCode::RootMismatch,
        pb::ErrorCode::RootUnknown => wire::ErrorCode::RootUnknown,
        pb::ErrorCode::StaleView => wire::ErrorCode::StaleView,
        pb::ErrorCode::DaemonOnly => wire::ErrorCode::DaemonOnly,
        pb::ErrorCode::ConventionFault => wire::ErrorCode::ConventionFault,
        pb::ErrorCode::ArmedDrift => wire::ErrorCode::ArmedDrift,
        pb::ErrorCode::BindingBreak => wire::ErrorCode::BindingBreak,
        pb::ErrorCode::IndexIntegrity => wire::ErrorCode::IndexIntegrity,
        pb::ErrorCode::WriteConflict => wire::ErrorCode::WriteConflict,
        pb::ErrorCode::WorkspaceBusy => wire::ErrorCode::WorkspaceBusy,
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
        // v3-additive JSON-face addressing facts (M1 U2): outside the proto
        // agreement surface — the pb mirror stays v2-shaped.
        n: None,
        hpath_text: None,
        words: None,
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
        pb::request::Op::Hello(pb::HelloRequest {
            proto,
            client,
            contract,
            workspace,
        }) => wire::Op::Hello {
            proto,
            client,
            contract,
            workspace,
        },
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
            actor,
            now,
            receipt,
            if_root,
            dry,
            force,
            edits,
        }) => wire::Op::Splice {
            path: wire::Path(path),
            actor,
            now,
            receipt: receipt.map(receipt_addr_from_pb),
            if_root: if_root.map(wire::Root),
            dry,
            force,
            edits: edits.into_iter().map(edit_from_pb).collect(),
            // v3-only JSON-face field (M1 U8b): the pb mirror stays v2-shaped.
            plan_edits: Vec::new(),
            pin: None,
        },
        pb::request::Op::Root(pb::RootRequest {}) => wire::Op::Root,
        pb::request::Op::Diff(pb::DiffRequest { from_root, to_root }) => wire::Op::Diff {
            from_root: wire::Root(from_root),
            to_root: wire::Root(to_root),
        },
        pb::request::Op::Links(pb::LinksRequest { path, require_root }) => wire::Op::Links {
            path: path.map(wire::Path),
            require_root: require_root.map(wire::Root),
        },
        pb::request::Op::Sub(pb::SubRequest { from_seq }) => wire::Op::Sub { from_seq },
        pb::request::Op::ViewPath(pb::ViewPathRequest { cwd, fresh }) => {
            wire::Op::ViewPath { cwd, fresh }
        }
        pb::request::Op::Create(pb::CreateRequest {
            path,
            body,
            actor,
            now,
            if_root,
            dry,
        }) => wire::Op::Create {
            path: wire::Path(path),
            body,
            actor,
            now,
            if_root: if_root.map(wire::Root),
            dry,
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
        matches,
        lost,
        cause,
        overlap,
        required,
        as_of_root,
        live_root,
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
        matches,
        lost: if lost.is_empty() {
            None
        } else {
            Some(
                lost.into_iter()
                    .map(|h| h.segs.into_iter().map(hpath_seg_from_pb).collect())
                    .collect(),
            )
        },
        cause,
        overlap: if overlap.is_empty() {
            None
        } else {
            Some(overlap.into_iter().map(sec_ref_from_pb).collect())
        },
        required: required.map(wire::Root),
        as_of_root: as_of_root.map(wire::Root),
        live_root: live_root.map(wire::Root),
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

#[allow(clippy::too_many_lines)] // one exhaustive match arm per response body — the drift pin
fn body_from_pb(b: pb::response::Body) -> wire::ResponseBody {
    match b {
        pb::response::Body::Hello(pb::HelloResponse {
            proto,
            server,
            caps,
            root,
            storage,
            workspace,
        }) => wire::ResponseBody::Hello {
            proto,
            server,
            caps,
            root: root.map(wire::Root),
            storage,
            workspace,
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
        pb::response::Body::Splice(pb::SpliceResponse {
            armed,
            receipt,
            root_before,
            root_after,
            seq,
            dry,
            verdicts,
        }) => wire::ResponseBody::Splice {
            armed: armed_from_pb(armed.expect("armed is required")),
            receipt: receipt.map(receipt_fact_from_pb),
            root_before: wire::Root(root_before),
            root_after: root_after.map(wire::Root),
            seq,
            dry,
            verdicts: verdicts.into_iter().map(verdict_from_pb).collect(),
            pin: None,
        },
        pb::response::Body::Create(pb::CreateResponse {
            path,
            file_rev_after,
            root_before,
            root_after,
            seq,
            dry,
            journal_anchor,
            verdicts,
        }) => wire::ResponseBody::Create {
            path: wire::Path(path),
            file_rev_after: wire::NodeRev(file_rev_after),
            root_before: wire::Root(root_before),
            root_after: root_after.map(wire::Root),
            seq,
            dry,
            journal_anchor,
            verdicts: verdicts.into_iter().map(verdict_from_pb).collect(),
        },
        pb::response::Body::Root(pb::RootResponse { root, seq }) => wire::ResponseBody::Root {
            root: wire::Root(root),
            seq,
        },
        pb::response::Body::Diff(pb::DiffResponse { batches }) => wire::ResponseBody::Diff {
            batches: batches.into_iter().map(delta_frame_from_pb).collect(),
        },
        pb::response::Body::Links(pb::LinksResponse {
            as_of_root,
            live_root,
            changes_seq,
            files,
        }) => wire::ResponseBody::Links {
            as_of_root: wire::Root(as_of_root),
            live_root: wire::Root(live_root),
            changes_seq,
            files: files
                .into_iter()
                .map(|(p, f)| (p, file_links_from_pb(f)))
                .collect(),
        },
        pb::response::Body::ViewPath(pb::ViewPathResponse {
            path,
            as_of_root,
            live_root,
            changes_seq,
            state,
            live_source,
            stale,
            refresh_in_progress,
            last_error,
        }) => wire::ResponseBody::ViewPath {
            path,
            as_of_root: wire::Root(as_of_root),
            live_root: wire::Root(live_root),
            changes_seq,
            state: view_state_from_pb(pb::ViewState::try_from(state).expect("known view state")),
            live_source: view_live_source_from_pb(
                pb::ViewLiveSource::try_from(live_source).expect("known live source"),
            ),
            stale,
            refresh_in_progress,
            last_error: last_error.map(refresh_error_from_pb),
        },
        pb::response::Body::Error(_) => unreachable!("payload_from_pb routes the error arm"),
    }
}

fn view_state_from_pb(s: pb::ViewState) -> wire::ViewState {
    match s {
        pb::ViewState::FreshAtSample => wire::ViewState::FreshAtSample,
        pb::ViewState::Stale => wire::ViewState::Stale,
        pb::ViewState::Raced => wire::ViewState::Raced,
        pb::ViewState::Unspecified => panic!("VIEW_STATE_UNSPECIFIED never crosses the seam"),
    }
}

fn view_live_source_from_pb(s: pb::ViewLiveSource) -> wire::ViewLiveSource {
    match s {
        pb::ViewLiveSource::Fold => wire::ViewLiveSource::Fold,
        pb::ViewLiveSource::Watch => wire::ViewLiveSource::Watch,
        pb::ViewLiveSource::None => wire::ViewLiveSource::None,
        pb::ViewLiveSource::Unspecified => {
            panic!("VIEW_LIVE_SOURCE_UNSPECIFIED never crosses the seam")
        }
    }
}

fn refresh_error_code_from_pb(c: pb::RefreshErrorCode) -> wire::RefreshErrorCode {
    match c {
        pb::RefreshErrorCode::ParseError => wire::RefreshErrorCode::ParseError,
        pb::RefreshErrorCode::DiskFull => wire::RefreshErrorCode::DiskFull,
        pb::RefreshErrorCode::Oom => wire::RefreshErrorCode::Oom,
        pb::RefreshErrorCode::Timeout => wire::RefreshErrorCode::Timeout,
        pb::RefreshErrorCode::Io => wire::RefreshErrorCode::Io,
        pb::RefreshErrorCode::Unspecified => {
            panic!("REFRESH_ERROR_CODE_UNSPECIFIED never crosses the seam")
        }
    }
}

fn refresh_error_from_pb(e: pb::RefreshError) -> wire::RefreshError {
    let pb::RefreshError {
        code,
        unix,
        fingerprint_attempted,
        message,
    } = e;
    wire::RefreshError {
        code: refresh_error_code_from_pb(
            pb::RefreshErrorCode::try_from(code).expect("known refresh code"),
        ),
        unix,
        fingerprint_attempted: fingerprint_attempted.map(wire::Root),
        message,
    }
}

fn file_change_from_pb(c: i32) -> wire::FileChange {
    match pb::FileChange::try_from(c).expect("known file change") {
        pb::FileChange::Created => wire::FileChange::Created,
        pb::FileChange::Modified => wire::FileChange::Modified,
        pb::FileChange::Deleted => wire::FileChange::Deleted,
        pb::FileChange::Renamed => wire::FileChange::Renamed,
        pb::FileChange::Unspecified => panic!("unspecified file change"),
    }
}

fn node_change_from_pb(c: i32) -> wire::NodeChange {
    match pb::NodeChange::try_from(c).expect("known node change") {
        pb::NodeChange::Added => wire::NodeChange::Added,
        pb::NodeChange::Edited => wire::NodeChange::Edited,
        pb::NodeChange::Removed => wire::NodeChange::Removed,
        pb::NodeChange::Unspecified => panic!("unspecified node change"),
    }
}

fn delta_node_from_pb(n: pb::DeltaNode) -> wire::DeltaNode {
    let pb::DeltaNode {
        target,
        change,
        node_rev_before,
        node_rev_after,
        span_after,
    } = n;
    wire::DeltaNode {
        target: sec_ref_from_pb(target.expect("target is required")),
        change: node_change_from_pb(change),
        node_rev_before: node_rev_before.map(wire::NodeRev),
        node_rev_after: node_rev_after.map(wire::NodeRev),
        span_after: span_after.map(span_from_pb),
    }
}

fn delta_file_from_pb(f: pb::DeltaFile) -> wire::DeltaFile {
    let pb::DeltaFile {
        path,
        change,
        from_path,
        file_rev_before,
        file_rev_after,
        nodes,
    } = f;
    wire::DeltaFile {
        path: wire::Path(path),
        change: file_change_from_pb(change),
        from_path: from_path.map(wire::Path),
        file_rev_before: file_rev_before.map(wire::NodeRev),
        file_rev_after: file_rev_after.map(wire::NodeRev),
        nodes: nodes.into_iter().map(delta_node_from_pb).collect(),
    }
}

fn delta_frame_from_pb(f: pb::DeltaFrame) -> wire::DeltaFrame {
    let pb::Delta {
        seq,
        root_before,
        root_after,
        actor,
        now,
        files,
    } = f.delta.expect("delta is required");
    wire::DeltaFrame {
        delta: wire::Delta {
            seq,
            root_before: wire::Root(root_before),
            root_after: wire::Root(root_after),
            actor,
            now,
            files: files.into_iter().map(delta_file_from_pb).collect(),
        },
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
        // v3-additive JSON-face facts: not part of the proto agreement.
        n: None,
        hpath_text: None,
        words: None,
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

/// The three §4.4 batch shapes: fully guarded match edit, guardless
/// `at:end` append, dry run with `fm_key`/anchor targets.
fn sample_splice_requests() -> Vec<wire::Op> {
    vec![
        // splice: the fully-guarded §4.4 worked frame (id 42 family)
        wire::Op::Splice {
            path: wire::Path("notes/plan.md".into()),
            actor: Some("agent:b0864fb2".into()),
            now: Some("2026-07-18T20:31:04Z".into()),
            receipt: Some(wire::ReceiptAddr {
                path: wire::Path("receipts/2026-07-18.md".into()),
                anchor: "r-000042".into(),
            }),
            if_root: Some(wire::Root(
                "b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9".into(),
            )),
            dry: None,
            force: None,
            edits: vec![wire::Edit {
                target: wire::SecRef::Hpath {
                    hpath: vec![seg("Goals"), seg("Q3")],
                },
                edit: wire::EditShape::Match {
                    old: "ship by August".into(),
                    new: "ship by September".into(),
                },
                if_node_rev: Some(wire::NodeRev("33d5b0e1b27cb48b".into())),
            }],
            plan_edits: Vec::new(),
            pin: None,
        },
        // splice: guardless append (legal at the wire forever) — put at:end
        wire::Op::Splice {
            path: wire::Path("notes/plan.md".into()),
            actor: None,
            now: None,
            receipt: None,
            if_root: None,
            dry: None,
            force: None,
            edits: vec![wire::Edit {
                target: wire::SecRef::Hpath {
                    hpath: vec![seg("Goals"), seg("Q4")],
                },
                edit: wire::EditShape::Put {
                    at: wire::PutAt::End,
                    text: "- new item\n".into(),
                },
                if_node_rev: None,
            }],
            plan_edits: Vec::new(),
            pin: None,
        },
        // splice: dry run, fm_key target, put at:all and at:content coverage
        wire::Op::Splice {
            path: wire::Path("notes/plan.md".into()),
            actor: None,
            now: None,
            receipt: None,
            if_root: None,
            dry: Some(true),
            force: None,
            edits: vec![
                wire::Edit {
                    target: wire::SecRef::FmKey {
                        fm_key: "title".into(),
                    },
                    edit: wire::EditShape::Put {
                        at: wire::PutAt::All,
                        text: "title: Plan v2".into(),
                    },
                    if_node_rev: None,
                },
                wire::Edit {
                    target: wire::SecRef::Anchor {
                        anchor: "r-000042".into(),
                    },
                    edit: wire::EditShape::Put {
                        at: wire::PutAt::Content,
                        text: "replacement content\n".into(),
                    },
                    if_node_rev: None,
                },
                // put at:upsert — the fm-key create-or-replace verb; text is the
                // VALUE, the server composes `{key}: {value}` (design W-8).
                wire::Edit {
                    target: wire::SecRef::FmKey {
                        fm_key: "status".into(),
                    },
                    edit: wire::EditShape::Put {
                        at: wire::PutAt::Upsert,
                        text: "active".into(),
                    },
                    if_node_rev: None,
                },
            ],
            plan_edits: Vec::new(),
            pin: None,
        },
    ]
}

#[expect(
    clippy::too_many_lines,
    reason = "one flat sample per request shape — the coverage IS the length; grouping them into helpers hides which shapes are covered"
)]
fn sample_requests() -> Vec<wire::Request> {
    let ops = vec![
        wire::Op::Hello {
            proto: 1,
            client: Some("agreement-test".into()),
            contract: Some("v3".into()),
            workspace: Some("/home/zt/wiki".into()), // resident-engine handshake target
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
        wire::Op::Root,
        wire::Op::Diff {
            from_root: wire::Root(
                "b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9".into(),
            ),
            to_root: wire::Root(
                "b3:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68".into(),
            ),
        },
        // links: the §4.6 worked request (path), the §10.2 strict form
        // (require_root), and the whole-corpus form (both absent)
        wire::Op::Links {
            path: Some(wire::Path("notes/plan.md".into())),
            require_root: None,
        },
        wire::Op::Links {
            path: Some(wire::Path("notes/plan.md".into())),
            require_root: Some(wire::Root(
                "b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9".into(),
            )),
        },
        wire::Op::Links {
            path: None,
            require_root: None,
        },
        // sub (§4.7 push path): live-only anchor and a catchup position
        wire::Op::Sub { from_seq: 0 },
        wire::Op::Sub { from_seq: 2 },
        // view_path (§Q2): the default forward, the bounded --fresh, and the
        // explicit fresh:false — the optional-bool wrapper's reason to exist.
        wire::Op::ViewPath {
            cwd: "/home/zt/wiki".into(),
            fresh: None,
        },
        wire::Op::ViewPath {
            cwd: "/home/zt/wiki".into(),
            fresh: Some(true),
        },
        wire::Op::ViewPath {
            cwd: "/home/zt/wiki".into(),
            fresh: Some(false),
        },
        // create (the birth op): the fully-guarded frame, the bare frame, and
        // the rehearsal — the optional-bool and optional-string wrappers all
        // exercised in both states, so a wrapper that silently collapsed
        // absent-to-default would fail the round trip.
        wire::Op::Create {
            path: wire::Path("notes/newborn.md".into()),
            body: "---\ntype: note\n---\n\n# Newborn\n".into(),
            actor: Some("agent:b0864fb2".into()),
            now: Some("2026-07-18T20:31:04Z".into()),
            if_root: Some(wire::Root(
                "b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9".into(),
            )),
            dry: None,
        },
        wire::Op::Create {
            path: wire::Path("notes/bare.md".into()),
            body: "# Bare\n".into(),
            actor: None,
            now: None,
            if_root: None,
            dry: None,
        },
        wire::Op::Create {
            path: wire::Path("notes/rehearsal.md".into()),
            body: String::new(), // an empty body is a legal newborn
            actor: None,
            now: None,
            if_root: None,
            dry: Some(true),
        },
    ];
    ops.into_iter()
        .chain(sample_splice_requests())
        .enumerate()
        .map(|(i, op)| wire::Request {
            id: (i > 0).then_some(i as u64),
            op,
        })
        .collect()
}

/// The §10.2 worked extras: the demanded root + the world as sampled (no
/// message — the extras carry the whole diagnosis).
fn stale_view_extras(e: &mut wire::ErrorBody) {
    e.message = None;
    e.required = Some(wire::Root(
        "b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9".into(),
    ));
    e.as_of_root = Some(wire::Root(
        "b3:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68".into(),
    ));
    e.live_root = Some(wire::Root(
        "b3:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68".into(),
    ));
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
        wire::ErrorCode::NoMatch,
        wire::ErrorCode::NotUnique,
        wire::ErrorCode::WouldCorrupt,
        wire::ErrorCode::LockTimeout,
        wire::ErrorCode::FileNotFound,
        wire::ErrorCode::IoError,
        wire::ErrorCode::InvalidUtf8,
        wire::ErrorCode::Internal,
        wire::ErrorCode::CasMismatch,
        wire::ErrorCode::RefNotFound,
        wire::ErrorCode::AmbiguousRef,
        wire::ErrorCode::StaleView,
        wire::ErrorCode::DaemonOnly,
    ];
    codes
        .into_iter()
        .map(|code| {
            let mut e = wire::ErrorBody::new(code);
            e.message = Some(format!("{code:?}"));
            e.path = matches!(
                code,
                wire::ErrorCode::BadPath | wire::ErrorCode::FileNotFound
            )
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
            e.matches = match code {
                wire::ErrorCode::NoMatch => Some(0), // 0 is a COUNT, not absence
                wire::ErrorCode::NotUnique => Some(2),
                _ => None,
            };
            e.lost = matches!(code, wire::ErrorCode::WouldCorrupt)
                .then(|| vec![vec![seg("Goals"), seg("Q3")]]);
            e.cause = matches!(code, wire::ErrorCode::IoError)
                .then(|| "EACCES: permission denied".into());
            e.overlap = matches!(code, wire::ErrorCode::BadRequest).then(|| {
                vec![
                    wire::SecRef::Hpath {
                        hpath: vec![seg("Goals")],
                    },
                    wire::SecRef::Hpath {
                        hpath: vec![seg("Goals"), seg("Q3")],
                    },
                ]
            });
            if matches!(code, wire::ErrorCode::StaleView) {
                stale_view_extras(&mut e);
            }
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

/// The two §4.4 response shapes: armed with receipt+seq, and dry
/// (`root_after` null, no receipt, no seq).
fn sample_splice_bodies() -> Vec<wire::ResponseBody> {
    vec![
        // splice: the §4.4 worked armed response (receipt + seq)
        wire::ResponseBody::Splice {
            armed: wire::Armed {
                path: wire::Path("notes/plan.md".into()),
                file_rev_after: Some(wire::NodeRev("a9794a262e67ed02".into())),
                edits: vec![wire::ArmedEdit {
                    target: wire::SecRef::Hpath {
                        hpath: vec![seg("Goals"), seg("Q3")],
                    },
                    node_rev_before: wire::NodeRev("33d5b0e1b27cb48b".into()),
                    node_rev_after: wire::NodeRev("41f643f034e5681f".into()),
                    span_after: wire::Span(49, 75),
                }],
            },
            receipt: Some(wire::ReceiptFact {
                path: wire::Path("receipts/2026-07-18.md".into()),
                anchor: "r-000042".into(),
                node_rev: wire::NodeRev("639a2dca46f6fcc8".into()),
                span_after: wire::Span(26, 248),
            }),
            root_before: wire::Root(
                "b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9".into(),
            ),
            root_after: Some(wire::Root(
                "b3:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68".into(),
            )),
            seq: Some(1),
            dry: None,
            // the frozen §11.1 worked verdict rides the roundtrip — proves the
            // inhabited shape crosses the seam byte-identically (P6-VERDICTS).
            verdicts: vec![wire::Verdict {
                rule: "blurb-required".into(),
                severity: wire::Severity::Warn,
                path: wire::Path("notes/plan.md".into()),
                hpath: Some(vec![seg("Goals")]),
                span: wire::Span(20, 150),
                node_rev: wire::NodeRev("5a8faa717fbcdb04".into()),
                message: "section has no blurb line".into(),
            }],
            // S7: the pin fact is v3-JSON-face only — the proto samples never
            // carry one.
            pin: None,
        },
        // splice: the dry shape — root_after null, no receipt, no seq
        wire::ResponseBody::Splice {
            armed: wire::Armed {
                path: wire::Path("notes/plan.md".into()),
                // dry — nothing written, so no post-write file rev
                file_rev_after: None,
                edits: vec![wire::ArmedEdit {
                    target: wire::SecRef::FmKey {
                        fm_key: "title".into(),
                    },
                    node_rev_before: wire::NodeRev("fa77480c79a853bc".into()),
                    node_rev_after: wire::NodeRev("fb49e9df2257fab8".into()),
                    span_after: wire::Span(4, 18),
                }],
            },
            receipt: None,
            root_before: wire::Root(
                "b3:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68".into(),
            ),
            root_after: None,
            seq: None,
            dry: Some(true),
            verdicts: vec![],
            pin: None,
        },
    ]
}

/// The two birth response shapes: a landed birth (root advanced, seq, journal
/// anchor, an inhabited verdict) and a rehearsal (`root_after` null, no seq, no
/// anchor). Both states of every optional are covered, so a wrapper that
/// collapsed absent into default would fail the round trip.
fn sample_create_bodies() -> Vec<wire::ResponseBody> {
    vec![
        wire::ResponseBody::Create {
            path: wire::Path("notes/newborn.md".into()),
            file_rev_after: wire::NodeRev("a9794a262e67ed02".into()),
            root_before: wire::Root(
                "b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9".into(),
            ),
            root_after: Some(wire::Root(
                "b3:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68".into(),
            )),
            seq: Some(1),
            dry: None,
            journal_anchor: Some("r-000001".into()),
            verdicts: vec![wire::Verdict {
                rule: "blurb-required".into(),
                severity: wire::Severity::Warn,
                path: wire::Path("notes/newborn.md".into()),
                hpath: Some(vec![seg("Newborn")]),
                span: wire::Span(20, 150),
                node_rev: wire::NodeRev("5a8faa717fbcdb04".into()),
                message: "section has no blurb line".into(),
            }],
        },
        // The rehearsal: nothing landed, so no advanced root, no Delta seq and
        // no journal row — but the rev IS still computable from the spec.
        wire::ResponseBody::Create {
            path: wire::Path("notes/rehearsal.md".into()),
            file_rev_after: wire::NodeRev("fb49e9df2257fab8".into()),
            root_before: wire::Root(
                "b3:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68".into(),
            ),
            root_after: None,
            seq: None,
            dry: Some(true),
            journal_anchor: None,
            verdicts: vec![],
        },
    ]
}

/// D3-DELTA: the diff arm — batches over the full §7.1 surface (both
/// §2.1 identities, absent-vs-present optionals, every change class, an
/// external delta with `actor`/`now` absent, renamed with `from_path`).
fn sample_diff_body() -> wire::ResponseBody {
    wire::ResponseBody::Diff {
        batches: vec![
            wire::DeltaFrame {
                delta: wire::Delta {
                    seq: 1,
                    root_before: wire::Root("b3:aa".into()),
                    root_after: wire::Root("b3:bb".into()),
                    actor: Some("agent:b0864fb2".into()),
                    now: Some("2026-07-18T20:31:04Z".into()),
                    files: vec![wire::DeltaFile {
                        path: wire::Path("notes/plan.md".into()),
                        change: wire::FileChange::Modified,
                        from_path: None,
                        file_rev_before: Some(wire::NodeRev("e3c4acaceb75b907".into())),
                        file_rev_after: Some(wire::NodeRev("a9794a262e67ed02".into())),
                        nodes: vec![
                            wire::DeltaNode {
                                target: wire::SecRef::Hpath {
                                    hpath: vec![wire::HpathSeg {
                                        h: "Goals".into(),
                                        n: Some(2),
                                    }],
                                },
                                change: wire::NodeChange::Edited,
                                node_rev_before: Some(wire::NodeRev("33d5b0e1b27cb48b".into())),
                                node_rev_after: Some(wire::NodeRev("41f643f034e5681f".into())),
                                span_after: Some(wire::Span(49, 75)),
                            },
                            wire::DeltaNode {
                                target: wire::SecRef::Anchor {
                                    anchor: "r-000042".into(),
                                },
                                change: wire::NodeChange::Added,
                                node_rev_before: None,
                                node_rev_after: Some(wire::NodeRev("639a2dca46f6fcc8".into())),
                                span_after: Some(wire::Span(26, 248)),
                            },
                            wire::DeltaNode {
                                target: wire::SecRef::FmKey {
                                    fm_key: "title".into(),
                                },
                                change: wire::NodeChange::Removed,
                                node_rev_before: Some(wire::NodeRev("fa77480c79a853bc".into())),
                                node_rev_after: None,
                                span_after: None,
                            },
                        ],
                    }],
                },
            },
            wire::DeltaFrame {
                // external change: actor/now ABSENT (§7.1 law, A8)
                delta: wire::Delta {
                    seq: 2,
                    root_before: wire::Root("b3:bb".into()),
                    root_after: wire::Root("b3:cc".into()),
                    actor: None,
                    now: None,
                    files: vec![
                        wire::DeltaFile {
                            path: wire::Path("notes/renamed.md".into()),
                            change: wire::FileChange::Renamed,
                            from_path: Some(wire::Path("notes/old.md".into())),
                            file_rev_before: Some(wire::NodeRev("aaaaaaaaaaaaaaaa".into())),
                            file_rev_after: Some(wire::NodeRev("bbbbbbbbbbbbbbbb".into())),
                            nodes: vec![],
                        },
                        wire::DeltaFile {
                            path: wire::Path("notes/new.md".into()),
                            change: wire::FileChange::Created,
                            from_path: None,
                            file_rev_before: None,
                            file_rev_after: Some(wire::NodeRev("cccccccccccccccc".into())),
                            nodes: vec![],
                        },
                        wire::DeltaFile {
                            path: wire::Path("notes/gone.md".into()),
                            change: wire::FileChange::Deleted,
                            from_path: None,
                            file_rev_before: Some(wire::NodeRev("dddddddddddddddd".into())),
                            file_rev_after: None,
                            nodes: vec![],
                        },
                    ],
                },
            },
            // empty-batches twin lives implicitly: Diff{batches:[]} is
            // the truthful rung-3 answer — pinned in the sidecar suite.
        ],
    }
}

/// The §Q2 `view_path` replies across the seam: `stale` is ALWAYS null (a
/// pre-open hint is never a verdict, B5+C3), every `ViewState`
/// (`FRESH_AT_SAMPLE`/STALE/RACED) and every `ViewLiveSource` (watch/none/fold)
/// cross once, and the OD7 `last_error` rides every `RefreshErrorCode` — so a
/// wire-or-proto drift on any of them stops the roundtrip compiling or passing.
fn sample_view_path_bodies() -> Vec<wire::ResponseBody> {
    const VIEW: &str = "/home/zt/.cache/meridian/abc123/v1/view.duckdb";
    // The pre-open hint variety: (state, live_source) pairs the daemon emits
    // (watch), plus the never-emitted-but-seam-legal fold/none for coverage.
    let hints = [
        (wire::ViewState::FreshAtSample, wire::ViewLiveSource::Watch),
        (wire::ViewState::Stale, wire::ViewLiveSource::None),
        (wire::ViewState::Raced, wire::ViewLiveSource::Fold),
    ];
    // Every OD7 failure class crosses once (mirrors sample_error_payloads).
    let codes = [
        wire::RefreshErrorCode::ParseError,
        wire::RefreshErrorCode::DiskFull,
        wire::RefreshErrorCode::Oom,
        wire::RefreshErrorCode::Timeout,
        wire::RefreshErrorCode::Io,
    ];
    let mut bodies = Vec::new();
    for (state, live_source) in hints {
        bodies.push(wire::ResponseBody::ViewPath {
            path: VIEW.into(),
            as_of_root: wire::Root("b3:aa".into()),
            // STALE/RACED carry distinct fingerprints; FRESH_AT_SAMPLE equal.
            live_root: wire::Root(
                if state == wire::ViewState::FreshAtSample {
                    "b3:aa"
                } else {
                    "b3:bb"
                }
                .into(),
            ),
            changes_seq: 0,
            state,
            live_source,
            stale: None,
            refresh_in_progress: false,
            last_error: None,
        });
    }
    for code in codes {
        bodies.push(wire::ResponseBody::ViewPath {
            path: VIEW.into(),
            as_of_root: wire::Root("b3:aa".into()),
            live_root: wire::Root("b3:bb".into()),
            changes_seq: 3,
            state: wire::ViewState::Stale,
            live_source: wire::ViewLiveSource::Watch,
            stale: None,
            refresh_in_progress: false,
            last_error: Some(wire::RefreshError {
                code,
                unix: 1_753_070_400,
                // Exercise the optional-absent path too (a parse failure may not
                // reach a fingerprint).
                fingerprint_attempted: if code == wire::RefreshErrorCode::ParseError {
                    None
                } else {
                    Some(wire::Root("b3:cc".into()))
                },
                message: "rebuild failed".into(),
            }),
        });
    }
    bodies
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
            storage: Some("/home/zt/.cache/meridian/abc123/v1".into()), // the pinned drawer
            workspace: Some("/home/zt/wiki".into()), // the root that actually bound
        },
        wire::ResponseBody::Hello {
            proto: 1,
            server: "meridian-sidecar".into(),
            caps: vec!["hello".into()],
            root: None,      // the engine may not have walked yet
            storage: None,   // a workspace-less handshake pins nothing
            workspace: None, // ...and so binds no root to name
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
        wire::ResponseBody::Root { root, seq: 2 },
        sample_diff_body(),
        // links (§4.6 + the §10.1 triple): a resolving edge, a dangling edge,
        // an empty entry (link-less files never vanish), and an honest-tense
        // divergence (as_of_root ≠ live_root — legal, never an error)
        wire::ResponseBody::Links {
            as_of_root: wire::Root(
                "b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9".into(),
            ),
            live_root: wire::Root(
                "b3:83b4ba591c0291d9f2a05428cac38e5820858fbb9c47720ab352344ddccc8f68".into(),
            ),
            changes_seq: 2,
            files: [
                (
                    "notes/plan.md".to_string(),
                    wire::FileLinks {
                        resolved: [("receipts/2026-07-18.md".to_string(), 1)].into(),
                        unresolved: [("roadmap".to_string(), 1)].into(),
                    },
                ),
                (
                    "receipts/2026-07-18.md".to_string(),
                    wire::FileLinks::default(),
                ),
            ]
            .into(),
        },
    ];
    let mut payloads: Vec<wire::ResponsePayload> = bodies
        .into_iter()
        .chain(sample_splice_bodies())
        .chain(sample_create_bodies())
        .chain(sample_view_path_bodies())
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

/// P6-VERDICTS strict-decode precision: a proto `Verdict` whose `severity` is the
/// proto3 zero-value (`SEVERITY_UNSPECIFIED` — no `wire::Severity` counterpart) is
/// REFUSED loud at the seam, never silently defaulted to a real severity. The
/// negative twin of the §11.1 worked-shape roundtrip sample above.
#[test]
#[should_panic(expected = "SEVERITY_UNSPECIFIED never crosses the seam")]
fn unspecified_severity_refuses_loud() {
    let v = pb::Verdict {
        rule: "blurb-required".into(),
        severity: pb::Severity::Unspecified.into(),
        path: "notes/plan.md".into(),
        hpath: vec![],
        span: Some(pb::Span {
            start: 20,
            end: 150,
        }),
        node_rev: "5a8faa717fbcdb04".into(),
        message: "section has no blurb line".into(),
    };
    let _ = verdict_from_pb(v);
}
