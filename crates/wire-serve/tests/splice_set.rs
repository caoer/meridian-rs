//! §4.4 SET-form gates (`splice.set`): an N-file commit lands whole with one
//! fingerprint advance, one receipt entry naming every file, and one Delta of
//! N+1 files; a validation refusal anywhere lands NOTHING (bytes asserted
//! untouched, named member); dry rehearses everything except disk; the strict
//! decode walls hold the form closed.

use std::path::PathBuf;

use serde_json::{Map, Value, json};
use wire::{
    Edit, EditShape, ErrorCode, HpathSeg, Path as WPath, ReceiptAddr, ResponseBody, SecRef,
};
use wire_serve::decode::decode;
use wire_serve::guard::Origin;
use wire_serve::rev::Rev;
use wire_serve::write::{SpliceSetArgs, splice_set};

/// Three sibling corpus files, same shape, distinct bodies.
fn body(i: usize) -> String {
    format!("# Note\n\n## Body\n\nalpha {i} old\n")
}

fn ws(n: usize) -> (tempfile::TempDir, fs::WorkspaceRoot, Vec<String>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut rels = Vec::new();
    for i in 1..=n {
        let rel = format!("notes/f{i}.md");
        std::fs::create_dir_all(dir.path().join("notes")).unwrap();
        std::fs::write(dir.path().join(&rel), body(i)).unwrap();
        rels.push(rel);
    }
    let root = fs::WorkspaceRoot(PathBuf::from(dir.path()));
    (dir, root, rels)
}

fn seg(h: &str) -> HpathSeg {
    HpathSeg {
        h: h.into(),
        n: None,
    }
}

fn body_ref() -> SecRef {
    SecRef::Hpath {
        hpath: vec![seg("Note"), seg("Body")],
    }
}

/// One member editing its file's `alpha {i} old` to `alpha {i} new`.
fn member(rel: &str, i: usize) -> wire::SpliceFile {
    wire::SpliceFile {
        path: WPath(rel.to_string()),
        edits: vec![Edit {
            target: body_ref(),
            edit: EditShape::Match {
                old: format!("alpha {i} old"),
                new: format!("alpha {i} new"),
            },
            if_node_rev: None,
        }],
        plan_edits: Vec::new(),
    }
}

fn set_args(files: Vec<wire::SpliceFile>, receipt: Option<ReceiptAddr>) -> SpliceSetArgs {
    SpliceSetArgs {
        premises: Vec::new(),
        id: Some(7),
        files,
        origin: Origin::Wire,
        actor: Some("agent:alice".into()),
        now: Some("2026-08-14T12:00:00Z".into()),
        receipt,
        if_root: None,
        dry: false,
        // The gates below exercise commit mechanics, not the
        // fingerprint-or-force ratchet (guard-family suites own that law).
        force: true,
    }
}

/// GATE: an N-file set plus receipt lands whole — every member's new bytes,
/// one root advance, one Delta carrying N content files + the receipt, one
/// receipt line naming every member, per-file armed groups in the response.
#[test]
fn set_lands_whole_one_frame_one_receipt() {
    let (dir, root, rels) = ws(3);
    let receipt = ReceiptAddr {
        path: WPath("receipts/log.md".into()),
        anchor: "r-000042".into(),
    };
    let files: Vec<wire::SpliceFile> = rels
        .iter()
        .enumerate()
        .map(|(i, rel)| member(rel, i + 1))
        .collect();

    let out =
        splice_set(&root, None, &set_args(files, Some(receipt)), &[]).expect("the set lands whole");

    for (i, rel) in rels.iter().enumerate() {
        assert_eq!(
            std::fs::read_to_string(dir.path().join(rel)).unwrap(),
            body(i + 1).replace(
                &format!("alpha {} old", i + 1),
                &format!("alpha {} new", i + 1)
            ),
            "member {} holds its new bytes",
            i + 1
        );
    }
    let frame = out.committed.expect("a real commit emits one frame");
    assert_eq!(
        frame.delta.files.len(),
        4,
        "ONE Delta carries every content file plus the receipt (N+1)"
    );
    assert_eq!(
        frame.delta.files[3].path.0, "receipts/log.md",
        "receipt is the last delta file (§7.1 print order, receipt last)"
    );
    assert_ne!(
        frame.delta.root_before, frame.delta.root_after,
        "one fingerprint advance covers the whole set"
    );
    let receipt_bytes = std::fs::read_to_string(dir.path().join("receipts/log.md")).unwrap();
    assert!(
        receipt_bytes.contains("splice.set files=3")
            && rels.iter().all(|rel| receipt_bytes.contains(rel.as_str()))
            && receipt_bytes.contains("^r-000042"),
        "ONE receipt line names every member under one anchor: {receipt_bytes}"
    );
    assert_eq!(
        receipt_bytes.matches("splice.set").count(),
        1,
        "exactly one receipt entry for the whole set"
    );
    let ResponseBody::SpliceSet {
        armed,
        receipt,
        root_after,
        seq,
        dry,
        ..
    } = out.body
    else {
        panic!("the set answers the SpliceSet body");
    };
    assert_eq!(armed.len(), 3, "per-file armed groups, request order");
    assert!(
        armed.iter().zip(&rels).all(|(a, rel)| a.path.0 == *rel),
        "armed groups keep request order"
    );
    assert!(
        armed.iter().all(|a| a.file_rev_after.is_some()),
        "every group carries its post-write file rev"
    );
    assert!(receipt.is_some(), "the receipt fact rides the response");
    assert!(root_after.is_some(), "root_after present on a real commit");
    assert!(seq.is_some() && dry.is_none());
}

/// GATE (validate-all-then-apply): a mid-set validation refusal — member 2's
/// match text does not occur — answers for the WHOLE request with nothing
/// landed, and the refusal names the member.
#[test]
fn mid_set_refusal_lands_nothing() {
    let (dir, root, rels) = ws(3);
    let mut files: Vec<wire::SpliceFile> = rels
        .iter()
        .enumerate()
        .map(|(i, rel)| member(rel, i + 1))
        .collect();
    files[1].edits[0].edit = EditShape::Match {
        old: "text that occurs nowhere".into(),
        new: "x".into(),
    };

    let err = splice_set(&root, None, &set_args(files, None), &[])
        .expect_err("member 2's no-match refuses the set");

    assert_eq!(err.code, ErrorCode::NoMatch);
    let msg = err.message.as_deref().unwrap_or_default();
    assert!(
        msg.contains("files[1]") && msg.contains("notes/f2.md"),
        "the refusal names the measuring member: {msg}"
    );
    for (i, rel) in rels.iter().enumerate() {
        assert_eq!(
            std::fs::read_to_string(dir.path().join(rel)).unwrap(),
            body(i + 1),
            "member {}'s bytes untouched — nothing landed",
            i + 1
        );
    }
}

/// GATE: dry rehearses everything except disk — same armed groups, no frame,
/// `root_after` null, no receipt written.
#[test]
fn set_dry_writes_nothing() {
    let (dir, root, rels) = ws(2);
    let files: Vec<wire::SpliceFile> = rels
        .iter()
        .enumerate()
        .map(|(i, rel)| member(rel, i + 1))
        .collect();
    let mut args = set_args(
        files,
        Some(ReceiptAddr {
            path: WPath("receipts/log.md".into()),
            anchor: "r-000001".into(),
        }),
    );
    args.dry = true;

    let out = splice_set(&root, None, &args, &[]).expect("a rehearsal answers");

    assert!(out.committed.is_none(), "dry emits no frame");
    let ResponseBody::SpliceSet {
        armed,
        root_after,
        dry,
        receipt,
        ..
    } = out.body
    else {
        panic!("the set answers the SpliceSet body");
    };
    assert_eq!(armed.len(), 2);
    assert_eq!(dry, Some(true));
    assert!(root_after.is_none(), "root_after null on dry (§4.4)");
    assert!(receipt.is_none(), "no receipt fact on dry");
    for (i, rel) in rels.iter().enumerate() {
        assert_eq!(
            std::fs::read_to_string(dir.path().join(rel)).unwrap(),
            body(i + 1),
            "dry writes nothing"
        );
    }
    assert!(
        !dir.path().join("receipts/log.md").exists(),
        "dry writes no receipt file"
    );
}

/// GATE: the in-door set walls — fewer than two members, duplicate paths, a
/// receipt path that is also a member — refuse before the flock.
#[test]
fn set_door_walls_refuse() {
    let (_dir, root, rels) = ws(2);

    let one = set_args(vec![member(&rels[0], 1)], None);
    let err = splice_set(&root, None, &one, &[]).expect_err("one member refuses");
    assert_eq!(err.code, ErrorCode::BadRequest);

    let dup = set_args(vec![member(&rels[0], 1), member(&rels[0], 1)], None);
    let err = splice_set(&root, None, &dup, &[]).expect_err("duplicate member refuses");
    assert_eq!(err.code, ErrorCode::BadRequest);

    let clobber = set_args(
        vec![member(&rels[0], 1), member(&rels[1], 2)],
        Some(ReceiptAddr {
            path: WPath(rels[0].clone()),
            anchor: "r-000001".into(),
        }),
    );
    let err = splice_set(&root, None, &clobber, &[]).expect_err("receipt==member refuses");
    assert_eq!(err.code, ErrorCode::BadRequest);
}

/// The batch-bound measurement receipt (run explicitly:
/// `cargo test -p wire-serve --test splice_set --release -- --ignored bench`).
/// One sealed set commit per N over KB-scale files — wall time ≈ flock-hold
/// time (the flock spans the whole call), the axis a bound decision prices:
/// cooperating writers refuse `workspace_busy` immediately while it is held.
#[test]
#[ignore = "measurement receipt, not a gate — run with --ignored"]
#[allow(clippy::cast_precision_loss)] // ms/file display arithmetic
fn bench_set_commit_scaling() {
    for n in [103usize, 653, 1024, 4096] {
        let (_dir, root, rels) = ws(n);
        let files: Vec<wire::SpliceFile> = rels
            .iter()
            .enumerate()
            .map(|(i, rel)| member(rel, i + 1))
            .collect();
        let receipt = ReceiptAddr {
            path: WPath("receipts/log.md".into()),
            anchor: format!("r-{n:06}"),
        };
        let started = std::time::Instant::now();
        let out = splice_set(&root, None, &set_args(files, Some(receipt)), &[])
            .expect("the bench set lands");
        let wall = started.elapsed();
        let frame = out.committed.expect("one frame");
        assert_eq!(frame.delta.files.len(), n + 1);
        println!(
            "set-commit N={n}: wall {:?} ({:.2} ms/file), one frame of {} files",
            wall,
            wall.as_secs_f64() * 1000.0 / n as f64,
            frame.delta.files.len()
        );
    }
}

// ── the strict decode walls (§3.2) ──────────────────────────────────────────

fn obj(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        other => panic!("fixture must be an object, got {other}"),
    }
}

fn set_frame() -> Value {
    json!({
        "op": "splice",
        "files": [
            {"path": "a.md", "edits": [{"target": {"anchor": "x1"}, "edit": {"put": {"at": "end", "text": "s"}}}]},
            {"path": "b.md", "plan_edits": [{"append": {"hpath": [{"h": "Log"}], "body": "row"}}]},
        ],
    })
}

/// The set form decodes on v3: two members, one `Op::SpliceSet`.
#[test]
fn decode_set_form_v3() {
    let op = decode(&obj(set_frame()), Rev::V3).expect("the set form decodes");
    let wire::Op::SpliceSet { files, .. } = op else {
        panic!("files[] decodes to the set op, got {op:?}");
    };
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].path.0, "a.md");
    assert!(files[1].edits.is_empty() && !files[1].plan_edits.is_empty());
}

/// v2 never admits `files` — the frozen field wall refuses it.
#[test]
fn decode_set_form_refused_on_v2() {
    let err = decode(&obj(set_frame()), Rev::V2).expect_err("v2 refuses `files`");
    assert_eq!(err.code, ErrorCode::BadRequest);
}

/// Strictly one form or the other: `files` beside `path`, `edits`,
/// `plan_edits`, or `pin` is `bad_request` at decode.
#[test]
fn decode_set_form_excludes_single_form_fields() {
    for (key, value) in [
        ("path", json!("a.md")),
        ("edits", json!([])),
        ("plan_edits", json!([])),
        (
            "pin",
            json!({"target": "a.md", "selector": {"anchor": "x"}}),
        ),
    ] {
        let mut frame = set_frame();
        frame[key] = value;
        let err = decode(&obj(frame), Rev::V3).expect_err("mixed forms refuse");
        assert_eq!(err.code, ErrorCode::BadRequest);
        assert!(
            err.message.as_deref().unwrap_or_default().contains(key),
            "the refusal names `{key}`"
        );
    }
}

/// The member walls: fewer than two members; duplicate paths; a member with
/// both `edits` and `plan_edits`; a member with neither.
#[test]
fn decode_set_member_walls() {
    let mut frame = set_frame();
    frame["files"] = json!([{"path": "a.md", "edits": [{"target": {"anchor": "x1"}, "edit": {"put": {"at": "end", "text": "s"}}}]}]);
    assert_eq!(
        decode(&obj(frame), Rev::V3)
            .expect_err("one member refuses")
            .code,
        ErrorCode::BadRequest
    );

    let mut frame = set_frame();
    frame["files"][1]["path"] = json!("a.md");
    let err = decode(&obj(frame), Rev::V3).expect_err("duplicate path refuses");
    assert!(
        err.message
            .as_deref()
            .unwrap_or_default()
            .contains("pairwise distinct"),
        "duplicate-path teaching: {:?}",
        err.message
    );

    let mut frame = set_frame();
    frame["files"][0]["plan_edits"] = json!([{"append": {"hpath": [{"h": "Log"}], "body": "row"}}]);
    assert_eq!(
        decode(&obj(frame), Rev::V3)
            .expect_err("both batches refuse")
            .code,
        ErrorCode::BadRequest
    );

    let mut frame = set_frame();
    frame["files"][0] = json!({"path": "a.md"});
    assert_eq!(
        decode(&obj(frame), Rev::V3)
            .expect_err("batch-less member refuses")
            .code,
        ErrorCode::BadRequest
    );
}
