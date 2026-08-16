//! The frozen-v2 key-set pin suite, type plane: what each frozen-v2 response
//! TYPE admits, with every optional field populated.
//!
//! The live plane rides the daemon socket — the one wire door since the
//! sidecar host's DROP (hosts ruling, wire-contract §3.3, 2026-08-06); its
//! served shapes are pinned by `crates/registry/tests/v3_key_set_pins.rs`.
//! Neither plane subsumes the other: a field added to a frozen struct reddens
//! this file even before any path populates it, while a flipped population
//! rule on an existing `Option` reddens only a live-serve pin, with
//! `git diff -- crates/wire/` empty.
//!
//! Every assertion is `assert_eq!` on the full sorted key list — a subset check
//! is byte-identical in the passing and the failing world.
//!
//! Where a list exceeds the frozen §-reference in its doc comment, the extra
//! key is annotated with its authority.

use std::collections::BTreeMap;
use wire::{
    Armed, ArmedEdit, Delta, DeltaFile, DeltaFrame, DeltaNode, ErrorBody, ErrorCode, FileChange,
    FileLinks, HpathSeg, Info, Node, NodeChange, NodeKind, NodeRev, Path, ReceiptFact, Response,
    ResponseBody, ResponsePayload, Root, SecRef, Severity, Span, TocNode, Verdict,
};

/// The pin primitive: exhaustive `assert_eq!` on the full sorted key list.
#[track_caller]
fn pin_keys<T: serde::Serialize>(value: &T, expected: &[&str], what: &str) {
    let v = serde_json::to_value(value).expect("serializes");
    let obj = v
        .as_object()
        .unwrap_or_else(|| panic!("{what} is not a JSON object: {v}"));
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys, expected,
        "frozen v2 {what} key set drifted — a field was added to or removed \
         from the type. Value: {v}"
    );
}

fn rev() -> NodeRev {
    NodeRev("33d5b0e1b27cb48b".into())
}

fn root() -> Root {
    Root("b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9".into())
}

fn path() -> Path {
    Path("notes/plan.md".into())
}

fn hpath() -> Vec<HpathSeg> {
    vec![HpathSeg {
        h: "Goals".into(),
        n: Some(2),
    }]
}

fn target() -> SecRef {
    SecRef::Hpath { hpath: hpath() }
}

// ---------------------------------------------------------------------------
// The §2.1 address grammar — the shape every ref-carrying surface echoes
// ---------------------------------------------------------------------------

/// v2 §2.1 as amended (decision 20): the segment is the object form `{h, n?}`
/// in both directions. `n` rides only on a disambiguating segment.
#[test]
fn hpath_segment_key_set_is_frozen() {
    pin_keys(&hpath()[0], &["h", "n"], "hpath segment (maximal)");
    pin_keys(
        &HpathSeg {
            h: "Goals".into(),
            n: None,
        },
        &["h"],
        "hpath segment (bare)",
    );
}

// ---------------------------------------------------------------------------
// §3.2 hello
// ---------------------------------------------------------------------------

/// Frozen §3.2 prints `{proto, server, caps, root}`. The type admits three
/// more: `storage` and `workspace`, the resident-daemon binding facts populated
/// by the daemon host only (no `docs/` amendment declares them — their standing
/// on a v2 handshake rests on the type's own doc); and `identity`, the v3-only
/// build identity from `docs/wire-contract.md`, which the daemon populates under
/// a negotiated v3 session only.
#[test]
fn hello_body_key_set_is_frozen_plus_the_two_daemon_binding_fields_and_identity() {
    let hello = ResponseBody::Hello {
        proto: 1,
        server: "meridian-sidecar/2.0".into(),
        caps: vec!["toc".into()],
        root: Some(root()),
        storage: Some("/drawer".into()),
        workspace: Some("/ws".into()),
        identity: Some(wire::Identity {
            build: "6c4b1f0a".into(),
        }),
    };
    pin_keys(
        &hello,
        &[
            "caps",
            "identity",
            "proto",
            "root",
            "server",
            "storage",
            "workspace",
        ],
        "Hello body (maximal)",
    );
}

// ---------------------------------------------------------------------------
// §4.1 toc
// ---------------------------------------------------------------------------

/// Frozen §4.1: `path` + `file_rev` + ambient `root` + `nodes`.
#[test]
fn toc_body_key_set_is_frozen() {
    let toc = ResponseBody::Toc {
        path: path(),
        file_rev: rev(),
        root: root(),
        nodes: vec![],
    };
    pin_keys(&toc, &["file_rev", "nodes", "path", "root"], "Toc body");
}

/// Frozen §4.1 row, maximal: the union of the frontmatter, heading and anchor
/// row classes, so a new row field appears here even if no class emits it yet.
#[test]
fn toc_row_key_set_is_frozen() {
    let row = TocNode {
        kind: "heading".into(),
        level: Some(2),
        hpath: Some(hpath()),
        anchor: Some("r-000042".into()),
        span: Span(49, 72),
        content_span: Some(Span(55, 72)),
        node_rev: rev(),
        text_prefix_16b: "## Q3\n\nship by A".into(),
        keys: Some(vec!["title".into()]),
    };
    pin_keys(
        &row,
        &[
            "anchor",
            "content_span",
            "hpath",
            "keys",
            "kind",
            "level",
            "node_rev",
            "span",
            "text_prefix_16b",
        ],
        "TocNode (maximal)",
    );
}

// ---------------------------------------------------------------------------
// §4.2 cat · §4.3 extract · §4.5 resolve
// ---------------------------------------------------------------------------

/// Frozen §4.2: the full span bytes and the rev over precisely those bytes.
#[test]
fn cat_body_key_set_is_frozen() {
    let cat = ResponseBody::Cat {
        span: Span(49, 72),
        node_rev: rev(),
        content: "## Q3\n".into(),
    };
    pin_keys(&cat, &["content", "node_rev", "span"], "Cat body");
}

/// Frozen §4.3: the node inventory is `path` + `nodes`.
#[test]
fn extract_body_key_set_is_frozen() {
    let nodes = ResponseBody::Nodes {
        path: path(),
        nodes: vec![],
    };
    pin_keys(&nodes, &["nodes", "path"], "Nodes body");
}

/// The frozen v1 §5.2 node object, maximal — including the two v3-additive
/// host-face fields (`n`, `words`) that share the struct. `Option` +
/// `skip_serializing_if` is not a version gate, so only the live half can say
/// they stay off a v2 wire; this pin says what the type admits, so a third such
/// field cannot be added silently.
///
/// The sanitized joined address `hpath_text` was removed by decision 14:
/// `sanitize_heading` is many-to-one, so no consumer could invert it back into
/// something `put` accepts. `hpath` carries the same address as segments.
#[test]
fn extract_node_key_set_is_frozen_plus_the_v3_host_face_pair() {
    let node = Node {
        kind: NodeKind::Heading,
        span: Span(20, 136),
        text_prefix_16b: "# Goals\n\nShip th".into(),
        hpath: Some(hpath()),
        unterminated: Some(true),
        info: Some(Info::Frontmatter {
            keys: vec!["title".into()],
        }),
        node_rev: Some(rev()),
        n: Some("1.2".into()),
        words: Some(12),
    };
    pin_keys(
        &node,
        &[
            "hpath",
            "info",
            "kind",
            "n",
            "node_rev",
            "span",
            "text_prefix_16b",
            "unterminated",
            "words",
        ],
        "Node (maximal)",
    );
}

/// The frozen v1 §5.2 per-kind `info` payloads, one pin per shape.
#[test]
fn extract_info_key_sets_are_frozen() {
    pin_keys(
        &Info::Frontmatter {
            keys: vec!["title".into()],
        },
        &["keys"],
        "Info::Frontmatter",
    );
    pin_keys(
        &Info::Fence {
            info_string: "rust".into(),
        },
        &["info_string"],
        "Info::Fence",
    );
    pin_keys(
        &Info::Wikilink {
            target: "2026-07-18".into(),
            heading: Some("Goals".into()),
            block: Some("r-000042".into()),
            alias: Some("receipts".into()),
        },
        &["alias", "block", "heading", "target"],
        "Info::Wikilink (maximal)",
    );
    pin_keys(
        &Info::Callout {
            r#type: "note".into(),
            fold: "+".into(),
        },
        &["fold", "type"],
        "Info::Callout",
    );
    pin_keys(
        &Info::Task {
            checked: true,
            depth: 1,
        },
        &["checked", "depth"],
        "Info::Task",
    );
}

/// Frozen §4.5: location facts only — no rev field exists on this type to
/// return (D-C2, the mint partition). A rev added here appears in the list.
#[test]
fn resolve_body_key_set_is_frozen_and_carries_no_rev() {
    let resolve = ResponseBody::Resolve {
        dest: path(),
        span: Span(49, 72),
        content: Some("## Q3\n".into()),
    };
    pin_keys(&resolve, &["content", "dest", "span"], "Resolve body");
}

// ---------------------------------------------------------------------------
// §4.4 splice — the ONE write response shape
// ---------------------------------------------------------------------------

/// Frozen §4.4 splice body: `armed`, `receipt`, `root_before`, `root_after`,
/// `seq`, `dry`, `verdicts`. The type admits one more — `pin`, the `splice.pin`
/// fact, v3-only at decode (the live half pins that).
#[test]
fn splice_body_key_set_is_frozen_plus_the_v3_pin_fact() {
    let splice = ResponseBody::Splice {
        armed: Armed {
            path: path(),
            file_rev_after: None,
            edits: vec![],
            effects: vec![],
        },
        receipt: Some(ReceiptFact {
            path: path(),
            anchor: "r-000042".into(),
            node_rev: rev(),
            span_after: Span(26, 248),
        }),
        root_before: root(),
        root_after: Some(root()),
        seq: Some(1),
        dry: Some(true),
        verdicts: vec![],
        pin: None,
    };
    pin_keys(
        &splice,
        &[
            "armed",
            "dry",
            "receipt",
            "root_after",
            "root_before",
            "seq",
            "verdicts",
        ],
        "Splice body (frozen fields)",
    );
}

/// `Armed` — v2 §4.4 as amended (decision 21). The type admits the frozen
/// `{path, edits}` plus two declared passengers, neither demoted nor v3-split:
///
/// - `file_rev_after` — the whole-file rev after a committed splice, so a
///   client learns it without a follow-up `toc`; latency only, correctness
///   stays fingerprint and `root_after`; absent on dry, since nothing was
///   written. Same family as [`DeltaFile::file_rev_after`].
/// - `effects` — reaction envelopes under `body.armed.effects`, omitted when
///   empty (`docs/wire-contract.md`).
#[test]
fn armed_key_set_is_frozen_plus_two_passengers() {
    let armed = Armed {
        path: path(),
        file_rev_after: Some(rev()),
        edits: vec![ArmedEdit {
            target: target(),
            node_rev_before: rev(),
            node_rev_after: rev(),
            span_after: Span(49, 75),
        }],
        effects: vec![],
    };
    // `effects` skips when empty, so the maximal form re-inserts the key.
    let mut v = serde_json::to_value(&armed).expect("serializes");
    v.as_object_mut()
        .expect("object")
        .insert("effects".into(), serde_json::json!([]));
    let mut keys: Vec<&str> = v
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["edits", "effects", "file_rev_after", "path"],
        "frozen v2 Armed key set drifted: {v}"
    );
}

/// The §4.4 write response's other members: one armed edit (target identity in
/// the §2.1 grammar, rev transition, span after) and the §6.3 receipt fact.
#[test]
fn armed_edit_and_receipt_fact_key_sets_are_frozen() {
    pin_keys(
        &ArmedEdit {
            target: target(),
            node_rev_before: rev(),
            node_rev_after: rev(),
            span_after: Span(49, 75),
        },
        &["node_rev_after", "node_rev_before", "span_after", "target"],
        "ArmedEdit",
    );
    pin_keys(
        &ReceiptFact {
            path: path(),
            anchor: "r-000042".into(),
            node_rev: rev(),
            span_after: Span(26, 248),
        },
        &["anchor", "node_rev", "path", "span_after"],
        "ReceiptFact",
    );
}

/// v2 §11.1: the rules-as-data verdict — `policy`'s `Violation` projected into
/// the grammar. The shape rides every splice response as `[]`, so a field added
/// here changes bytes the frozen contract prints.
///
/// Type plane only: the value is hand-built, so it says nothing about the wire.
/// The wire half died with the sidecar host (§3.3 DROP): the daemon serves no
/// rule packs, so no live Verdict is servable (see the Deliberate gaps note in
/// `crates/registry/tests/v3_key_set_pins.rs`) and this type pin is the
/// standing guard.
#[test]
fn verdict_key_set_is_frozen() {
    let verdict = Verdict {
        rule: "blurb-required".into(),
        severity: Severity::Warn,
        path: path(),
        hpath: Some(hpath()),
        span: Span(20, 150),
        node_rev: rev(),
        message: "section has no blurb line".into(),
    };
    pin_keys(
        &verdict,
        &[
            "hpath", "message", "node_rev", "path", "rule", "severity", "span",
        ],
        "Verdict",
    );
}

// ---------------------------------------------------------------------------
// §4.6 links · §4.7 root / diff
// ---------------------------------------------------------------------------

/// Frozen §4.6 under the §10.1 staleness triple.
#[test]
fn links_body_and_file_key_sets_are_frozen() {
    let mut files = BTreeMap::new();
    files.insert("notes/plan.md".to_string(), FileLinks::default());
    let links = ResponseBody::Links {
        as_of_root: root(),
        live_root: root(),
        changes_seq: 2,
        files: files.clone(),
        excluded: Vec::new(),
    };
    // §4.6: `excluded` is omitted when empty, so a workspace whose hash domain
    // is its whole md tree carries the frozen v2 key set byte for byte.
    pin_keys(
        &links,
        &["as_of_root", "changes_seq", "files", "live_root"],
        "Links body",
    );
    // And it is PRESENT the moment the enumeration left something out — the
    // enumerator clause is a wire fact, not a CLI courtesy (§12.1).
    let with_excluded = ResponseBody::Links {
        as_of_root: root(),
        live_root: root(),
        changes_seq: 2,
        files,
        excluded: vec![".github/README.md".to_string()],
    };
    pin_keys(
        &with_excluded,
        &[
            "as_of_root",
            "changes_seq",
            "excluded",
            "files",
            "live_root",
        ],
        "Links body naming an excluded population",
    );
    pin_keys(
        &FileLinks::default(),
        &["resolved", "unresolved"],
        "FileLinks",
    );
}

/// Frozen §4.7: `{root, seq}` and `{batches}`. The B-01 `sub` ack rides the
/// same body plus `tree_instance` — additive, absent everywhere else, so the
/// frozen form stays byte-identical.
#[test]
fn root_and_diff_body_key_sets_are_frozen() {
    pin_keys(
        &ResponseBody::Root {
            root: root(),
            seq: 2,
            tree_instance: None,
        },
        &["root", "seq"],
        "Root body",
    );
    pin_keys(
        &ResponseBody::Root {
            root: root(),
            seq: 2,
            tree_instance: Some("1f0.2a.0".into()),
        },
        &["root", "seq", "tree_instance"],
        "Root body (sub ack, B-01)",
    );
    pin_keys(
        &ResponseBody::Diff { batches: vec![] },
        &["batches"],
        "Diff body",
    );
}

// ---------------------------------------------------------------------------
// §7.1 the Delta noun — born frozen
// ---------------------------------------------------------------------------

/// Frozen §7.1, maximal: `renamed` carries `from_path`, `created` drops
/// `file_rev_before`, `deleted` drops `file_rev_after` — the union is pinned so
/// a new file-grain fact cannot appear in any of those classes unseen.
/// `DeltaFrame.effects` rides beside `delta`, omitted when empty.
#[test]
fn delta_frame_file_and_node_key_sets_are_frozen() {
    let node = DeltaNode {
        target: target(),
        change: NodeChange::Edited,
        node_rev_before: Some(rev()),
        node_rev_after: Some(rev()),
        span_after: Some(Span(49, 75)),
    };
    // `DeltaNode` flattens its SecRef, so the grammar's key rides the node.
    pin_keys(
        &node,
        &[
            "change",
            "hpath",
            "node_rev_after",
            "node_rev_before",
            "span_after",
        ],
        "DeltaNode (hpath form, maximal)",
    );
    pin_keys(
        &DeltaNode {
            target: SecRef::Anchor {
                anchor: "r-000042".into(),
            },
            change: NodeChange::Added,
            node_rev_before: None,
            node_rev_after: Some(rev()),
            span_after: Some(Span(26, 248)),
        },
        &["anchor", "change", "node_rev_after", "span_after"],
        "DeltaNode (anchor form)",
    );

    let file = DeltaFile {
        path: path(),
        change: FileChange::Renamed,
        from_path: Some(Path("notes/old.md".into())),
        file_rev_before: Some(rev()),
        file_rev_after: Some(rev()),
        nodes: vec![node],
    };
    pin_keys(
        &file,
        &[
            "change",
            "file_rev_after",
            "file_rev_before",
            "from_path",
            "nodes",
            "path",
        ],
        "DeltaFile (maximal)",
    );

    let delta = Delta {
        seq: 1,
        root_before: root(),
        root_after: root(),
        actor: Some("agent:b0864fb2".into()),
        now: Some("2026-07-18T20:31:04Z".into()),
        files: vec![file],
    };
    pin_keys(
        &delta,
        &["actor", "files", "now", "root_after", "root_before", "seq"],
        "Delta (maximal)",
    );

    // `effects` skips when empty, so the maximal frame re-inserts the key.
    let frame = DeltaFrame {
        delta,
        effects: vec![],
        rescope: None,
        overflow: None,
    };
    pin_keys(&frame, &["delta"], "DeltaFrame (no reaction output)");
    let mut v = serde_json::to_value(&frame).expect("serializes");
    v.as_object_mut()
        .expect("object")
        .insert("effects".into(), serde_json::json!([]));
    let mut keys: Vec<&str> = v
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["delta", "effects"],
        "the DeltaFrame admits exactly the frozen `delta` plus the \
         amendment-declared `effects`: {v}"
    );
}

// ---------------------------------------------------------------------------
// §3.1 the frame envelope · §8 the error envelope
// ---------------------------------------------------------------------------

/// §3.1: a response frame is `id` + `ok` + exactly one flattened payload key.
#[test]
fn response_frame_key_sets_are_frozen() {
    let ok = Response {
        id: Some(42),
        ok: true,
        payload: ResponsePayload::Body {
            body: ResponseBody::Root {
                root: root(),
                seq: 2,
                tree_instance: None,
            },
        },
    };
    pin_keys(&ok, &["body", "id", "ok"], "Response (ok)");
    let err = Response {
        id: None,
        ok: false,
        payload: ResponsePayload::Error {
            error: ErrorBody::new(ErrorCode::BadRequest),
        },
    };
    pin_keys(&err, &["error", "id", "ok"], "Response (error)");
}

/// §8: `code` + the required closed `recovery` class + optional `message` +
/// code-specific extras beside them, never nested further. Maximal here, so a
/// new extra appears in the list the moment it is declared.
///
/// `rung`, `diff`, `new_content`, `new_fingerprint` are the v3-additive
/// mismatch-recovery ladder (`rung` is the authorship mark `rev::demote_v2`
/// keys off to strip the authored slots from a v2 frame); `scope` and
/// `uncovered` are the §5.4/§5.5 scoped-guard family's extras, reserved the
/// same way.
#[test]
fn error_body_key_set_is_frozen_plus_the_v3_ladder_extras() {
    let err = ErrorBody {
        code: ErrorCode::CasMismatch,
        recovery: wire::Recovery::Refresh,
        message: Some("m".into()),
        path: Some(path()),
        supported: Some(vec![1]),
        expected: Some(rev()),
        actual: Some(rev()),
        required: Some(root()),
        as_of_root: Some(root()),
        live_root: Some(root()),
        stage: Some(2),
        dest: Some(path()),
        candidates: Some(vec![target()]),
        unknown_kinds: Some(vec!["bogus".into()]),
        id_raw: Some("\"7\"".into()),
        matches: Some(2),
        family: Some(wire::WouldCorruptFamily::ContainmentLost),
        lost: Some(vec![hpath()]),
        target: Some(target()),
        cause: Some("io".into()),
        overlap: Some(vec![target()]),
        rung: Some(1),
        diff: Some("@@".into()),
        new_content: Some("bytes".into()),
        new_fingerprint: Some(rev()),
        referrers: Some(vec![wire::Referrer {
            path: "notes/fan.md".into(),
            kind: wire::ReferrerKind::Wikilink,
            count: 1,
        }]),
        scope: Some("notes".into()),
        uncovered: Some(vec!["section \"Goals\"".into()]),
    };
    pin_keys(
        &err,
        &[
            "actual",
            "as_of_root",
            "candidates",
            "cause",
            "code",
            "dest",
            "diff",
            "expected",
            "family",
            "id_raw",
            "live_root",
            "lost",
            "matches",
            "message",
            "new_content",
            "new_fingerprint",
            "overlap",
            "path",
            "recovery",
            "referrers",
            "required",
            "rung",
            "scope",
            "stage",
            "supported",
            "target",
            "uncovered",
            "unknown_kinds",
        ],
        "ErrorBody (maximal)",
    );
    pin_keys(
        &ErrorBody::new(ErrorCode::UnknownOp),
        &["code", "recovery"],
        "ErrorBody (bare)",
    );
}

// ---------------------------------------------------------------------------
// The pin primitive's own control
// ---------------------------------------------------------------------------

/// Twin of `u27_v2_key_set_pins.rs::the_pin_primitive_rejects_a_superset` — the
/// two `pin_keys` copies cannot be merged, so each is held to the contract in
/// the crate where it lives.
#[test]
fn the_pin_primitive_rejects_a_superset() {
    #[derive(serde::Serialize)]
    struct Two {
        a: u8,
        b: u8,
    }
    let two = Two { a: 1, b: 2 };
    let caught = std::panic::catch_unwind(|| {
        pin_keys(&two, &["a"], "pin-primitive self-control");
    });
    assert!(
        caught.is_err(),
        "pin_keys accepted a key set with an EXTRA key — it is no longer \
         exhaustive, and every pin in this file is now a decoration"
    );
    pin_keys(&two, &["a", "b"], "pin-primitive self-control");
}
