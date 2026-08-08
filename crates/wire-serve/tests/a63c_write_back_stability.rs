//! § A.6.3c — spelling preservation on a semantic no-op, at the wire doors.
//!
//! **The oscillation this pins (review gate d5654f18, P2).** `ccc-cli` writes
//! the fleet-canonical `owner: "3f9a1c07"`; the engine reads it as `3f9a1c07`;
//! a read-modify-write that writes that same value back re-emitted the plain
//! spelling, moving `prop_rev`, `span` and the `props1` fingerprint on a
//! semantic no-op — two writers churning one tree. § A.6.3c keeps the stored
//! spelling when the stored bytes already decode to the caller's string.
//!
//! Byte identity of the whole FILE is the assertion, because § A.6.2's guard
//! planes are computed over source bytes: identical bytes ⇒ nothing moved.

use std::path::PathBuf;

use wire::{Edit, EditShape, Path as WPath, PlanEdit, PutAt, ResponseBody, SecRef};
use wire_serve::write::{SpliceArgs, splice};

fn ws(files: &[(&str, &str)]) -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    for (rel, body) in files {
        let p = dir.path().join(rel);
        std::fs::write(p, body).expect("seed");
    }
    let root = fs::WorkspaceRoot(PathBuf::from(dir.path()));
    (dir, root)
}

fn plan_args(path: &str, plan_edits: Vec<PlanEdit>) -> SpliceArgs {
    SpliceArgs {
        id: None,
        origin: wire_serve::guard::Origin::InProcess,
        path: WPath(path.into()),
        actor: Some("agent:alice".into()),
        now: Some("2026-07-24T12:00:00Z".into()),
        receipt: None,
        if_root: None,
        dry: false,
        force: false,
        edits: Vec::new(),
        plan_edits,
        pin: None,
    }
}

fn native_args(path: &str, edits: Vec<Edit>) -> SpliceArgs {
    SpliceArgs {
        edits,
        plan_edits: Vec::new(),
        pin: None,
        ..plan_args(path, Vec::new())
    }
}

/// The fleet-canonical card: quoted spellings exactly as `ccc-cli` lands them.
const DOC: &str = "---\ntype: task\nstatus: \"doing\"\nowner: \"3f9a1c07\"\n---\n\n# Memo\n\nbody\n";

fn upsert(key: &str, text: &str) -> Edit {
    Edit {
        target: SecRef::FmKey {
            fm_key: key.into(),
        },
        edit: EditShape::Put {
            at: PutAt::Upsert,
            text: text.into(),
        },
        if_node_rev: None,
    }
}

/// The plan `set_property` door: writing back the exact value the read law
/// serves leaves the file byte-identical, and the armed edit's own before/after
/// node revs agree — the `prop_rev` half of the claim, witnessed at the face.
#[test]
fn plan_set_property_write_back_is_byte_stable() {
    let (dir, root) = ws(&[("card.md", DOC)]);
    let out = splice(
        &root,
        None,
        &plan_args(
            "card.md",
            vec![PlanEdit::SetProperty {
                key: "owner".into(),
                value: "3f9a1c07".into(),
                rev: None,
            }],
        ),
        &[],
        None,
    )
    .expect("the no-op write commits");

    let bytes = std::fs::read_to_string(dir.path().join("card.md")).expect("read back");
    assert_eq!(bytes, DOC, "a semantic no-op keeps the stored spelling");

    let ResponseBody::Splice { armed, .. } = &out.body else {
        panic!("splice body");
    };
    for e in &armed.edits {
        assert_eq!(
            e.node_rev_before.0, e.node_rev_after.0,
            "identical bytes ⇒ the armed node rev must not move"
        );
    }
}

/// The `put{at:"upsert"}` door — the other § A.6.3a value-plane door — under
/// the same no-op: stored quoted spelling, caller sends the served value.
#[test]
fn upsert_write_back_is_byte_stable() {
    let (dir, root) = ws(&[("card.md", DOC)]);
    splice(
        &root,
        None,
        &native_args("card.md", vec![upsert("owner", "3f9a1c07")]),
        &[],
        None,
    )
    .expect("the no-op upsert commits");

    let bytes = std::fs::read_to_string(dir.path().join("card.md")).expect("read back");
    assert_eq!(bytes, DOC, "a semantic no-op upsert keeps the stored spelling");
}

/// Two keys in one plan batch stay a no-op together — the group lowering
/// preserves each key's stored spelling independently.
#[test]
fn a_two_key_write_back_batch_is_byte_stable() {
    let (dir, root) = ws(&[("card.md", DOC)]);
    splice(
        &root,
        None,
        &plan_args(
            "card.md",
            vec![
                PlanEdit::SetProperty {
                    key: "owner".into(),
                    value: "3f9a1c07".into(),
                    rev: None,
                },
                PlanEdit::SetProperty {
                    key: "status".into(),
                    value: "doing".into(),
                    rev: None,
                },
            ],
        ),
        &[],
        None,
    )
    .expect("the no-op batch commits");
    let bytes = std::fs::read_to_string(dir.path().join("card.md")).expect("read back");
    assert_eq!(bytes, DOC);
}

/// The § A.6.3c exclusions at the wire face: a stored NULL spelling is never
/// preserved — the text-equal write-back lands the quoted canonical form, so
/// the write of a string LANDS a string (R4).
#[test]
fn a_stored_null_spelling_still_re_encodes_at_both_doors() {
    const NULLED: &str = "---\ntype: task\nowner: ~\n---\n\n# Memo\n\nbody\n";

    let (dir, root) = ws(&[("card.md", NULLED)]);
    splice(
        &root,
        None,
        &plan_args(
            "card.md",
            vec![PlanEdit::SetProperty {
                key: "owner".into(),
                value: "~".into(),
                rev: None,
            }],
        ),
        &[],
        None,
    )
    .expect("the write commits");
    let bytes = std::fs::read_to_string(dir.path().join("card.md")).expect("read back");
    assert!(
        bytes.contains(r#"owner: "~""#),
        "the null spelling re-encodes, never preserves: {bytes:?}"
    );

    let (dir, root) = ws(&[("card.md", NULLED)]);
    splice(
        &root,
        None,
        &native_args("card.md", vec![upsert("owner", "~")]),
        &[],
        None,
    )
    .expect("the upsert commits");
    let bytes = std::fs::read_to_string(dir.path().join("card.md")).expect("read back");
    assert!(
        bytes.contains(r#"owner: "~""#),
        "the upsert door re-encodes the null spelling too: {bytes:?}"
    );
}
