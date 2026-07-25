//! S4a (plan D4): the I4 def-conformance verdict is INTERNALIZED into `splice`
//! under the D9 write flock, so a foreign writer can no longer split the verdict
//! from the write it authorized.
//!
//! The window this closes: a host ran `check_write` (round-trip 1), got a PASS,
//! then ran `splice` (round-trip 2). Anything that landed on the target in
//! between changed the pre-image, so the ladder had judged bytes the write no
//! longer wrote. `splice` now re-runs the SAME ladder (`check_write::verdict` —
//! one owner) over the `doc` it loaded under the flock and the `after_doc` it is
//! about to commit; the verdict that authorizes bytes is that one.
//!
//! Fixture shape: a def whose `Answer` section is `required-before-terminal`
//! (severity `error` — never forceable). The pending write flips `status` to a
//! terminal value. With `Answer` populated that write is legal; once a foreign
//! writer empties `Answer`, the SAME write introduces the violation.

use wire::{Edit, EditShape, ErrorCode, Path as WPath, PutAt, Recovery, ResponseBody, SecRef};
use wire_serve::write::{SpliceArgs, splice};

/// The kind is deliberately unique (`s4aprobe`): `discover_layers` walks to the
/// filesystem root and also consults `$UCC_HOME/defs`, so a common kind name
/// could be shadowed by a real def on the developer's machine.
const DEF: &str = "---\ntype: def\ndefines: s4aprobe\nversion: 1\n---\n\n# Properties\n\n```yaml\ntype:      {shape: line, required: true, default: s4aprobe}\nstatus:    {shape: line, required: true, suggest: [open], terminal: [done]}\nclosed_at: {shape: iso, stamp: close}\n```\n^properties\n\n# Sections\n\n## section: Answer\n```yaml\nrequired-before-terminal: true\n```\n";

/// The record as the host read it: `Answer` non-empty, `status: open`. The empty
/// `closed_at` makes the terminal transition a REPAIR case — the ladder plans a
/// close stamp, which resolves the `status ∈ terminal ⟺ closed_at set`
/// biconditional and leaves only the section law to judge.
const REC: &str =
    "---\ntype: s4aprobe\nstatus: open\nclosed_at:\n---\n\n# Answer\n\nthe answer body\n";

/// What a FOREIGN writer (not a cooperating meridian writer — the flock does not
/// reach it, decision G2) leaves behind: `Answer` emptied.
const REC_DRIFTED: &str = "---\ntype: s4aprobe\nstatus: open\nclosed_at:\n---\n\n# Answer\n\n";

/// A workspace with the def layer and the record in place.
fn workspace() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(dir.path().join("defs")).expect("defs dir");
    std::fs::write(dir.path().join("defs/s4aprobe.md"), DEF).expect("def");
    std::fs::write(dir.path().join("rec.md"), REC).expect("record");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

/// The pending write, in both vocabularies: the host's pre-flight speaks the
/// put-plan `set_property`, the wire `splice` speaks the native `fm_key` upsert.
/// Same intent — `status: done` — so the two verdicts are comparable.
fn splice_args(dry: bool) -> SpliceArgs {
    SpliceArgs {
        id: None,
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
    }
}

/// The host's pre-flight (round-trip 1), served exactly as both hosts serve it.
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
            at: "status".into(),
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

/// **THE TOCTOU test.** Pre-flight PASSES, a foreign writer lands between check
/// and apply, and the apply now refuses — where it used to commit bytes the
/// verdict never evaluated.
#[test]
fn foreign_write_between_check_and_apply_can_no_longer_split_the_verdict() {
    let (dir, root) = workspace();

    // Round-trip 1: the host checks. `Answer` is non-empty, so going terminal is
    // legal — the host is now authorized to write.
    assert!(
        refusal(&pre_flight(&root)).is_none(),
        "pre-flight must PASS on the bytes the host read"
    );

    // The window: a foreign writer empties `Answer`. Nothing about the host's
    // plan changed — it still says `status: done`.
    std::fs::write(dir.path().join("rec.md"), REC_DRIFTED).expect("foreign write");

    // Round-trip 2: the apply. The internalized verdict judges the LIVE
    // pre-image, sees the write introduce `def/required-before-terminal`, and
    // refuses before any byte lands.
    let err = splice(&root, 0, &splice_args(false), &[], None)
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

/// The control: with no foreign writer, the very same request lands. The
/// internalized verdict gates, it does not blanket-refuse.
#[test]
fn undrifted_write_still_lands() {
    let (dir, root) = workspace();

    assert!(refusal(&pre_flight(&root)).is_none(), "pre-flight passes");
    splice(&root, 0, &splice_args(false), &[], None).expect("the conforming write must land");

    let after = std::fs::read_to_string(dir.path().join("rec.md")).expect("read after");
    assert!(
        after.contains("status: done"),
        "the conforming write committed: {after}"
    );
}

/// A dry run refuses exactly where the real write does — a rehearsal that
/// answered "would succeed" here would hand the host back the same split
/// verdict, one round-trip later.
#[test]
fn dry_run_refuses_where_the_real_write_would() {
    let (dir, root) = workspace();
    std::fs::write(dir.path().join("rec.md"), REC_DRIFTED).expect("drifted fixture");

    let err = splice(&root, 0, &splice_args(true), &[], None)
        .expect_err("a dry rehearsal must refuse the non-conforming write");
    assert_eq!(err.code, ErrorCode::BadRequest, "typed: {:?}", err.message);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("rec.md")).expect("read after"),
        REC_DRIFTED,
        "dry writes nothing"
    );
}

/// One owner, two entry points: run over the SAME drifted pre-image, the
/// standalone op and the internalized run reach the identical verdict. If the
/// ladder ever forked, this is what would catch it.
#[test]
fn standalone_op_and_internalized_run_agree_on_the_drifted_pre_image() {
    let (dir, root) = workspace();
    std::fs::write(dir.path().join("rec.md"), REC_DRIFTED).expect("drifted fixture");

    let op_refusal = refusal(&pre_flight(&root))
        .expect("the standalone op refuses the drifted pre-image too")
        .clone();
    assert_eq!(op_refusal.class, "verdict", "ladder refusal, not a rebuild");

    let err = splice(&root, 0, &splice_args(false), &[], None).expect_err("splice refuses");
    assert_eq!(
        err.message.as_deref().unwrap_or_default(),
        format!(
            "{}: {} — {}",
            op_refusal.code, op_refusal.message, op_refusal.remedy
        ),
        "both entry points render the SAME ladder verdict"
    );
}

/// A workspace with no def layer is untouched by the internalization: the
/// undeclared kind passes (undeclared ≠ contract), so ordinary writes on
/// ordinary pages keep landing.
#[test]
fn no_def_layer_leaves_ordinary_writes_alone() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("rec.md"), REC).expect("record");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());

    splice(&root, 0, &splice_args(false), &[], None).expect("no def anywhere ⇒ the write lands");
    assert!(
        std::fs::read_to_string(dir.path().join("rec.md"))
            .expect("read after")
            .contains("status: done"),
        "an undeclared kind is not a record contract"
    );
}
