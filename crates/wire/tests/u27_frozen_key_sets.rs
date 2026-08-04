//! **U27 — the frozen-v2 key-set pin suite (TYPE plane).**
//!
//! The second half of the U27 detector pair. Its sibling,
//! `crates/sidecar/tests/u27_v2_key_set_pins.rs`, pins what a LIVE v2 session
//! actually serves; this file pins what each frozen-v2 response TYPE admits,
//! with **every optional field populated**.
//!
//! # Why both halves exist — neither subsumes the other
//!
//! The two worlds each half tells apart are different:
//!
//! - The LIVE half distinguishes "this v2 session serves exactly the frozen
//!   keys" from "it serves one more (or one fewer)". It sees a changed
//!   POPULATION RULE on an existing `Option` — a leak that leaves
//!   `git diff -- crates/wire/` completely empty — and it sees nothing about a
//!   field no live path populates yet.
//! - This TYPE half distinguishes "the frozen shape has these fields" from "a
//!   field was added to (or removed from) it". It sees the field the moment it
//!   is declared, before any path populates it, and it is blind to which
//!   session vintage ends up receiving it.
//!
//! A v3-additive field added to a frozen struct and populated only on the v3
//! path reddens THIS file and not the live one. A population rule flipped so a
//! v2 session starts receiving an existing optional field reddens the LIVE file
//! and not this one. That is why the pair is the unit and not either file.
//!
//! # The form is load-bearing
//!
//! Every assertion is `assert_eq!` on the FULL sorted key list. A `contains` or
//! subset check is byte-identical in the passing and the failing world — it is
//! a decoration, not a control (All-Hands #3).
//!
//! # What a key beyond the frozen contract means here
//!
//! Where this file's list exceeds the frozen §-reference in its doc comment,
//! the extra key is annotated with its authority: an amendment doc, or **a U27
//! finding** where no authority exists. Nothing here silently blesses an
//! unauthorized field — the finding is named in the pin and reported on the
//! card.

use std::collections::BTreeMap;
use wire::{
    Armed, ArmedEdit, Delta, DeltaFile, DeltaFrame, DeltaNode, ErrorBody, ErrorCode, FileChange,
    FileLinks, HpathSeg, Info, Node, NodeChange, NodeKind, NodeRev, Path, ReceiptFact, Response,
    ResponseBody, ResponsePayload, Root, SecRef, Severity, Span, TocNode, Verdict,
};

/// **The pin primitive.** EXHAUSTIVE `assert_eq!` on the full sorted key list.
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

/// v2 §2.1 as amended (decision 20): the segment is the OBJECT form `{h, n?}`
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

/// Frozen §3.2 prints `{proto, server, caps, root}`. The type admits two more:
/// `storage` and `workspace`, the resident-daemon binding facts, declared in
/// the type's own doc as optional additive fields under the §3.2 evolution law
/// and populated by the daemon host only (the sidecar emits neither — see the
/// live half).
///
/// **U27 finding 3:** neither field is declared in any `docs/` amendment, so
/// their standing on a v2 handshake rests on a code comment. Reported, not
/// fixed here.
#[test]
fn hello_body_key_set_is_frozen_plus_the_two_daemon_binding_fields() {
    let hello = ResponseBody::Hello {
        proto: 1,
        server: "meridian-sidecar/2.0".into(),
        caps: vec!["toc".into()],
        root: Some(root()),
        storage: Some("/drawer".into()),
        workspace: Some("/ws".into()),
    };
    pin_keys(
        &hello,
        &["caps", "proto", "root", "server", "storage", "workspace"],
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

/// Frozen §4.1 row: the complete write kit. Every optional is populated here,
/// so the pin is the union of the frontmatter, heading and anchor row classes
/// — a NEW row field appears in this list even if no class emits it yet.
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

/// The frozen v1 §5.2 node object, maximal — INCLUDING the two v3-additive
/// host-face fields (`n`, `words`) that share the struct. They are the
/// All-Hands #1 sighting's own family: `Option` + `skip_serializing_if` is not
/// a version gate, so only the live half can say they stay off a v2 wire. This
/// pin says what the type admits, so a THIRD such field cannot be added
/// silently.
///
/// **It was a TRIO when U27 measured it, and it is a PAIR here.** U27 branched
/// from a tree that predates the removal, so its pin named a field the assembly
/// does not have, and the pin fired at the assembly gate.
///
/// **THE AUTHORISATION FOR THIS EDIT, so a reader can tell it from a quiet
/// loosening: DECISION 14.** `hpath_text` — the sanitized joined ADDRESS — was
/// removed as ruled work: it was a string address on a machine surface, and a
/// lossy one, since `sanitize_heading` is many-to-one and no consumer could
/// invert it back into something `put` accepts. `hpath` carries the same address
/// as SEGMENTS and round-trips; the joined human spelling is the render plane's
/// to derive.
///
/// The pin was NOT wrong and was NOT loosened. This suite exists so that any
/// key-set change must be DELIBERATE, and it met a change that was — so the
/// correct outcome is the pin updated WITH its citation, in the same change as
/// the edit. A firing that met an UNRULED change would be a finding to report,
/// not a pin to update. Neither branch was wrong about its own tree; only the
/// merge can state the live key set.
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

/// Frozen §4.5: location facts only — and NO rev field exists on this type to
/// return (D-C2, the mint partition as a type-level fact). This pin is that
/// law's executable form: a rev field added here appears in the list.
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
/// `seq`, `dry`, `verdicts`. The type admits one more — `pin`, the stage-2 S7
/// `splice.pin` fact, v3-only at decode (a v2 session cannot mint one; the
/// live half pins that).
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

/// **`Armed` — v2 §4.4 AS AMENDED (requirements decision 21, ZT, 2026-08-04;
/// personal freeze authority per v2 §18).** The type admits exactly the frozen
/// `{path, edits}` plus two declared passengers:
///
/// - `file_rev_after` — ratified ON V2 by decision 21. ZT's semantics: the
///   whole-file rev AFTER a committed splice, so a client learns the new file
///   rev WITHOUT A FOLLOW-UP TOC; latency only, correctness stays fingerprint
///   and `root_after`; ABSENT ON DRY, because nothing was written; same family
///   as [`DeltaFile::file_rev_after`] and a subsequent `toc` `file_rev`.
/// - `effects` — `docs/wire-contract-v2-effects-amendment.md`, reaction
///   envelopes under `body.armed.effects`, omitted when empty.
///
/// Neither is demoted and neither is v3-split. The live half
/// (`u27_v2_key_set_pins.rs`) pins the served forms, committed and dry.
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
    // `effects` is `skip_serializing_if = "Vec::is_empty"`, so the maximal
    // form needs a populated envelope — the empty vec would hide the field.
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

/// The uncontested members of the §4.4 write response: one armed edit (target
/// identity in THE §2.1 grammar, rev transition, span after) and the §6.3
/// receipt fact. Neither is touched by the `armed` dispute — the doc and the
/// fixture agree on both, so both are minted.
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

/// v2 §11.1: the rules-as-data verdict — `policy`'s `Violation` verbatim,
/// projected into THE grammar. The shape rode every splice response as `[]`
/// from birth, so a field added here changes bytes the frozen contract prints.
///
/// TYPE-plane only, and deliberately so. This value is hand-built, which makes
/// it a statement about the struct and NOT about the wire — U27 first shipped
/// it as the shape's only pin, on the mistaken belief that no live path could
/// serve a verdict. `crates/sidecar/tests/u27_v2_key_set_pins.rs::
/// verdict_key_set_is_frozen_on_the_wire` is the wire half, taken from a real
/// pack through the real serve loop. Read the two together: this one catches a
/// field added to the struct, that one catches what a v2 client receives.
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
        files,
    };
    pin_keys(
        &links,
        &["as_of_root", "changes_seq", "files", "live_root"],
        "Links body",
    );
    pin_keys(
        &FileLinks::default(),
        &["resolved", "unresolved"],
        "FileLinks",
    );
}

/// Frozen §4.7: `{root, seq}` and `{batches}`.
#[test]
fn root_and_diff_body_key_sets_are_frozen() {
    pin_keys(
        &ResponseBody::Root {
            root: root(),
            seq: 2,
        },
        &["root", "seq"],
        "Root body",
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
///
/// `DeltaFrame.effects` is declared by the effects amendment ("A `DeltaFrame`
/// may carry an `effects` array beside `delta`", omitted when empty).
#[test]
fn delta_frame_file_and_node_key_sets_are_frozen() {
    let node = DeltaNode {
        target: target(),
        change: NodeChange::Edited,
        node_rev_before: Some(rev()),
        node_rev_after: Some(rev()),
        span_after: Some(Span(49, 75)),
    };
    // `DeltaNode` FLATTENS its SecRef, so the grammar's key rides the node.
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

    // `effects` skips when empty, so the maximal frame is asserted on the
    // re-inserted key — the frozen `delta` sibling is what must not move.
    let frame = DeltaFrame {
        delta,
        effects: vec![],
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

/// §8: `code` + the REQUIRED closed `recovery` class + optional `message` +
/// code-specific extras beside them, never nested further. Maximal here, so a
/// new extra appears in the list the moment it is declared.
///
/// The last four — `rung`, `diff`, `new_content`, `new_fingerprint` — are U11's
/// **v3-additive** mismatch-recovery ladder. `rung` is the authorship mark
/// `rev::demote_v2` keys off to strip all six authored slots from a v2 frame;
/// the live half is what proves the stripping happens.
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
        changed: Some(vec![path()]),
        required: Some(root()),
        as_of_root: Some(root()),
        live_root: Some(root()),
        stage: Some(2),
        dest: Some(path()),
        candidates: Some(vec![target()]),
        unknown_kinds: Some(vec!["bogus".into()]),
        id_raw: Some("\"7\"".into()),
        matches: Some(2),
        lost: Some(vec![hpath()]),
        cause: Some("io".into()),
        overlap: Some(vec![target()]),
        rung: Some(1),
        diff: Some("@@".into()),
        new_content: Some("bytes".into()),
        new_fingerprint: Some(rev()),
    };
    pin_keys(
        &err,
        &[
            "actual",
            "as_of_root",
            "candidates",
            "cause",
            "changed",
            "code",
            "dest",
            "diff",
            "expected",
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
            "required",
            "rung",
            "stage",
            "supported",
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
// The pin primitive's own control (All-Hands #43)
// ---------------------------------------------------------------------------

/// **The twin of `u27_v2_key_set_pins.rs::the_pin_primitive_rejects_a_superset`.**
/// See that test for why the two `pin_keys` copies exist and cannot be merged;
/// this one holds THIS crate's copy to the same contract, so a drift in either
/// is caught where it happens rather than by whoever notices later.
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
