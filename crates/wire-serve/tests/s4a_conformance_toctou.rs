//! S4a (D4): I4 def-conformance internalized into `splice` under D9 flock.
//!
//! Closes check→apply TOCTOU: `splice` re-runs `check_write::verdict` on live
//! pre-image. Fixture: `Answer` required-before-terminal; terminal write legal
//! with Answer filled, refuses when foreign writer empties Answer.

use wire::{Edit, EditShape, ErrorCode, Path as WPath, PutAt, Recovery, ResponseBody, SecRef};
use wire_serve::write::{SpliceArgs, splice};

/// Unique kind `s4aprobe` (avoids `$UCC_HOME/defs` / parent-walk shadow).
const DEF: &str = "---\ntype: def\ndefines: s4aprobe\nversion: 1\n---\n\n# Properties\n\n```yaml\ntype:      {shape: line, required: true, default: s4aprobe}\nstatus:    {shape: line, required: true, suggest: [open], terminal: [done]}\nclosed_at: {shape: iso, stamp: close}\n```\n^properties\n\n# Sections\n\n## section: Answer\n```yaml\nrequired-before-terminal: true\n```\n";

/// Host-read record: Answer non-empty, open; empty `closed_at` → repair stamp.
const REC: &str =
    "---\ntype: s4aprobe\nstatus: open\nclosed_at:\n---\n\n# Answer\n\nthe answer body\n";

/// Foreign writer (G2, outside flock) empties Answer.
const REC_DRIFTED: &str = "---\ntype: s4aprobe\nstatus: open\nclosed_at:\n---\n\n# Answer\n\n";

/// Def layer + record workspace.
fn workspace() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(dir.path().join("defs")).expect("defs dir");
    std::fs::write(dir.path().join("defs/s4aprobe.md"), DEF).expect("def");
    std::fs::write(dir.path().join("rec.md"), REC).expect("record");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

/// Pending write: native `fm_key` upsert `status: done` (same intent as pre-flight).
fn splice_args(dry: bool) -> SpliceArgs {
    SpliceArgs {
        premises: Vec::new(),
        id: None,
        origin: wire_serve::guard::Origin::InProcess,
        path: WPath("rec.md".into()),
        actor: Some("w1234567".into()),
        now: Some("2026-07-25T09:00:00Z".into()),
        receipt: None,
        if_root: None,
        dry,
        force: false,
        edits: vec![Edit {
            target: SecRef::FmKey {
                fm_key: "status".into(),
            },
            edit: EditShape::Put {
                at: PutAt::Upsert,
                text: "done".into(),
            },
            if_node_rev: None,
        }],
        plan_edits: Vec::new(),
        pin: None,
        fields: Default::default(),
    }
}

/// Host pre-flight (`check_write` / put-plan `set_property`).
fn pre_flight(root: &fs::WorkspaceRoot) -> ResponseBody {
    let raw = std::fs::read_to_string(root.0.join("rec.md")).expect("read prev");
    let prev = wire_serve::check_write::build_doc(&raw);
    wire_serve::check_write::check_write(
        &prev,
        &root.0.join("rec.md").display().to_string(),
        "w1234567",
        "2026-07-25T09:00:00Z",
        &[wire::CheckWriteEdit {
            op: "set_property".into(),
            at: vec![wire::HpathSeg {
                h: "status".into(),
                n: None,
            }],
            find: String::new(),
            body: "done".into(),
            rev: String::new(),
            all: false,
        }],
    )
}

fn refusal(body: &ResponseBody) -> Option<&wire::CheckWriteRefuse> {
    match body {
        ResponseBody::CheckWrite { refuse, .. } => refuse.as_ref(),
        other => panic!("check_write must answer a CheckWrite body: {other:?}"),
    }
}

/// TOCTOU: pre-flight pass + foreign empty Answer → apply refuses, no write.
#[test]
fn foreign_write_between_check_and_apply_can_no_longer_split_the_verdict() {
    let (dir, root) = workspace();

    // Round-trip 1: pre-flight PASSes (Answer non-empty).
    assert!(
        refusal(&pre_flight(&root)).is_none(),
        "pre-flight must PASS on the bytes the host read"
    );

    // Window: foreign writer empties Answer.
    std::fs::write(dir.path().join("rec.md"), REC_DRIFTED).expect("foreign write");

    // Apply: live pre-image fails required-before-terminal; nothing lands.
    let err = splice(&root, None, &splice_args(false), &[], None)
        .expect_err("the internalized verdict must refuse the drifted write");
    assert_eq!(
        err.code,
        ErrorCode::BadRequest,
        "typed refusal, closed §8 taxonomy: {:?}",
        err.message
    );
    assert_eq!(err.recovery, Recovery::Fix, "bad_request ⇒ fix");
    assert_eq!(
        err.path.as_ref().map(|p| p.0.as_str()),
        Some("rec.md"),
        "the refusal names the refused file"
    );
    let message = err.message.as_deref().unwrap_or_default();
    assert!(
        message.starts_with("E_CONFORMANCE: "),
        "the ladder's code rides the frame verbatim: {message}"
    );
    assert!(
        message.contains("# Answer: must be non-empty before a terminal status"),
        "the ladder's teaching rides the frame verbatim: {message}"
    );

    assert_eq!(
        std::fs::read_to_string(dir.path().join("rec.md")).expect("read after"),
        REC_DRIFTED,
        "the refusal lands NOTHING — the foreign writer's bytes are untouched"
    );
}

/// Control: undrifted same request lands (gate, not blanket refuse).
#[test]
fn undrifted_write_still_lands() {
    let (dir, root) = workspace();

    assert!(refusal(&pre_flight(&root)).is_none(), "pre-flight passes");
    splice(&root, None, &splice_args(false), &[], None).expect("the conforming write must land");

    let after = std::fs::read_to_string(dir.path().join("rec.md")).expect("read after");
    assert!(
        after.contains("status: done"),
        "the conforming write committed: {after}"
    );
}

/// Dry run refuses where real write would (no split-verdict rehearsal).
#[test]
fn dry_run_refuses_where_the_real_write_would() {
    let (dir, root) = workspace();
    std::fs::write(dir.path().join("rec.md"), REC_DRIFTED).expect("drifted fixture");

    let err = splice(&root, None, &splice_args(true), &[], None)
        .expect_err("a dry rehearsal must refuse the non-conforming write");
    assert_eq!(err.code, ErrorCode::BadRequest, "typed: {:?}", err.message);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("rec.md")).expect("read after"),
        REC_DRIFTED,
        "dry writes nothing"
    );
}

/// One owner: standalone `check_write` and internalized splice agree on message.
#[test]
fn standalone_op_and_internalized_run_agree_on_the_drifted_pre_image() {
    let (dir, root) = workspace();
    std::fs::write(dir.path().join("rec.md"), REC_DRIFTED).expect("drifted fixture");

    let op_refusal = refusal(&pre_flight(&root))
        .expect("the standalone op refuses the drifted pre-image too")
        .clone();
    assert_eq!(op_refusal.class, "verdict", "ladder refusal, not a rebuild");

    let err = splice(&root, None, &splice_args(false), &[], None).expect_err("splice refuses");
    assert_eq!(
        err.message.as_deref().unwrap_or_default(),
        format!(
            "{}: {} — {}",
            op_refusal.code, op_refusal.message, op_refusal.remedy
        ),
        "both entry points render the SAME ladder verdict"
    );
}

/// No def layer: ordinary write still lands (undeclared ≠ contract).
#[test]
fn no_def_layer_leaves_ordinary_writes_alone() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("rec.md"), REC).expect("record");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());

    splice(&root, None, &splice_args(false), &[], None).expect("no def anywhere ⇒ the write lands");
    assert!(
        std::fs::read_to_string(dir.path().join("rec.md"))
            .expect("read after")
            .contains("status: done"),
        "an undeclared kind is not a record contract"
    );
}
