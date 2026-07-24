//! M1 U8b end-to-end gates: `splice` with `plan_edits` — the engine-lowered
//! put path — lands the SAME disk bytes the Go daemon's emulation built, and
//! the whole flow (flock → lower → validate → commit → Delta) behaves as one
//! native splice.
//!
//! The equivalence law under test: for every plan batch, `plan_edits` through
//! the lowering == the native batch the host used to construct — byte-for-byte
//! on disk, same armed shape, same root advance.

use std::path::PathBuf;

use wire::{Edit, EditShape, HpathSeg, Path as WPath, PlanEdit, PutAt, ResponseBody, SecRef};
use wire_serve::write::{SpliceArgs, splice};

fn ws(files: &[(&str, &str)]) -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    for (rel, body) in files {
        let p = dir.path().join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(p, body).expect("seed");
    }
    let root = fs::WorkspaceRoot(PathBuf::from(dir.path()));
    (dir, root)
}

fn plan_args(path: &str, plan_edits: Vec<PlanEdit>) -> SpliceArgs {
    SpliceArgs {
        id: None,
        path: WPath(path.into()),
        actor: Some("agent:alice".into()),
        now: Some("2026-07-24T12:00:00Z".into()),
        receipt: None,
        if_root: None,
        dry: false,
        force: false,
        edits: Vec::new(),
        plan_edits,
    }
}

fn native_args(path: &str, edits: Vec<Edit>) -> SpliceArgs {
    SpliceArgs {
        edits,
        plan_edits: Vec::new(),
        ..plan_args(path, Vec::new())
    }
}

fn seg(h: &str) -> HpathSeg {
    HpathSeg {
        h: h.into(),
        n: None,
    }
}

const DOC: &str = "---\nstatus: open\nowner: d\n---\n# Memo\n\nbody line\n\n## Tasks\n\n- item one\n- item two\n\n# Archive\n\nold\n";

/// THE equivalence law: a mixed plan batch (property + append + create) lands
/// byte-identical disk state to the native batch the Go host used to build
/// for the same plans — same emulation bytes, engine-lowered.
#[test]
fn plan_batch_equals_the_host_built_native_batch() {
    // Workspace A: the plan-level form.
    let (da, ra) = ws(&[("card.md", DOC)]);
    let out_a = splice(
        &ra,
        0,
        &plan_args(
            "card.md",
            vec![
                PlanEdit::SetProperty {
                    key: "status".into(),
                    value: "closed".into(),
                },
                PlanEdit::Append {
                    hpath: "Memo/Tasks".into(),
                    body: "- item three".into(),
                },
                PlanEdit::Create {
                    parent_hpath: "Archive".into(),
                    title: "Log".into(),
                    body: "created".into(),
                },
            ],
        ),
        &[],
    )
    .expect("plan splice commits");

    // Workspace B: the native batch the Go emulation would have built —
    // props group first (Put{all} on the fm_key), then the disciplined
    // append payload, then the parent-append heading text.
    let (db, rb) = ws(&[("card.md", DOC)]);
    let out_b = splice(
        &rb,
        0,
        &native_args(
            "card.md",
            vec![
                Edit {
                    target: SecRef::FmKey {
                        fm_key: "status".into(),
                    },
                    edit: EditShape::Put {
                        at: PutAt::All,
                        text: "status: closed".into(),
                    },
                    if_node_rev: None,
                },
                Edit {
                    target: SecRef::Hpath {
                        hpath: vec![seg("Memo"), seg("Tasks")],
                    },
                    edit: EditShape::Put {
                        at: PutAt::End,
                        text: "- item three\n".into(),
                    },
                    if_node_rev: None,
                },
                Edit {
                    target: SecRef::Hpath {
                        hpath: vec![seg("Archive")],
                    },
                    edit: EditShape::Put {
                        at: PutAt::End,
                        text: "\n## Log\n\ncreated\n".into(),
                    },
                    if_node_rev: None,
                },
            ],
        ),
        &[],
    )
    .expect("native splice commits");

    let bytes_a = std::fs::read(da.path().join("card.md")).expect("read A");
    let bytes_b = std::fs::read(db.path().join("card.md")).expect("read B");
    assert_eq!(
        String::from_utf8_lossy(&bytes_a),
        String::from_utf8_lossy(&bytes_b),
        "plan_edits and the host-built native batch land IDENTICAL bytes"
    );

    // Same armed shape: the lowered batch arms 1:1 with the native one.
    let (ResponseBody::Splice { armed: aa, .. }, ResponseBody::Splice { armed: ab, .. }) =
        (&out_a.body, &out_b.body)
    else {
        panic!("splice bodies");
    };
    assert_eq!(aa.edits.len(), ab.edits.len(), "1:1 armed edits");
    assert_eq!(
        aa.file_rev_after, ab.file_rev_after,
        "same post-write whole-file rev"
    );
}

/// The full plan flow lands the expected literal bytes (an anchor an auditor
/// can eyeball): CAS-guarded whole-section rewrite + match-all RMW.
#[test]
fn replace_section_and_match_all_land_expected_bytes() {
    let (dir, root) = ws(&[("card.md", DOC)]);

    // Read the live section rev the way a caller would (armed facts domain):
    // resolve Tasks' node rev from the model directly.
    let doc = fs::load(&root, std::path::Path::new("card.md")).expect("load");
    let target = model::Ref::Hpath(vec![
        model::HpathSeg {
            h: "Memo".into(),
            n: None,
        },
        model::HpathSeg {
            h: "Tasks".into(),
            n: None,
        },
    ]);
    let tasks_rev = model::resolve(&doc, &target).expect("resolves").node_rev.0;

    splice(
        &root,
        0,
        &plan_args(
            "card.md",
            vec![PlanEdit::Match {
                hpath: "Memo/Tasks".into(),
                old: "item".into(),
                new: "task".into(),
                all: true,
                rev: Some(tasks_rev),
            }],
        ),
        &[],
    )
    .expect("match-all commits");

    let after = std::fs::read_to_string(dir.path().join("card.md")).expect("read");
    assert_eq!(
        after,
        "---\nstatus: open\nowner: d\n---\n# Memo\n\nbody line\n\n## Tasks\n\n- task one\n- task two\n\n# Archive\n\nold\n"
    );

    // Whole-section rewrite under the fresh rev.
    let doc = fs::load(&root, std::path::Path::new("card.md")).expect("load");
    let fresh = model::resolve(&doc, &target).expect("resolves").node_rev.0;
    splice(
        &root,
        0,
        &plan_args(
            "card.md",
            vec![PlanEdit::ReplaceSection {
                hpath: "Memo/Tasks".into(),
                body: "- done".into(),
                rev: Some(fresh),
            }],
        ),
        &[],
    )
    .expect("replace_section commits");
    let after = std::fs::read_to_string(dir.path().join("card.md")).expect("read");
    assert_eq!(
        after,
        "---\nstatus: open\nowner: d\n---\n# Memo\n\nbody line\n\n## Tasks\n- done\n# Archive\n\nold\n"
    );
}

/// A stale rev on a plan `replace_section` refuses `cas_mismatch` through the
/// UNCHANGED native CAS path (the lowering threads `rev` → `if_node_rev`; the
/// guard itself is the frozen engine's).
#[test]
fn plan_rev_threads_into_the_native_cas_guard() {
    let (dir, root) = ws(&[("card.md", DOC)]);
    let err = splice(
        &root,
        0,
        &plan_args(
            "card.md",
            vec![PlanEdit::ReplaceSection {
                hpath: "Memo/Tasks".into(),
                body: "- clobber".into(),
                rev: Some("0000000000000000".into()),
            }],
        ),
        &[],
    )
    .expect_err("stale rev refuses");
    assert_eq!(err.code, wire::ErrorCode::CasMismatch);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("card.md")).expect("read"),
        DOC,
        "nothing landed"
    );
}

/// The two golden MUST-CARRY refusals fire from the ENGINE now, with the file
/// untouched: p-replace-on-block + p-create-top (message = the Go-face bytes
/// minus the host's `put: ` prefix).
#[test]
fn golden_target_class_refusals_fire_engine_side() {
    let (dir, root) = ws(&[("card.md", "# Tasks\n\n- [ ] one ^task1\n")]);

    let err = splice(
        &root,
        0,
        &plan_args(
            "card.md",
            vec![PlanEdit::Match {
                hpath: "^task1".into(),
                old: "one".into(),
                new: "two".into(),
                all: false,
                rev: None,
            }],
        ),
        &[],
    )
    .expect_err("block replace target refuses");
    assert_eq!(
        err.message.as_deref(),
        Some(r#"no section addressed by "^task1""#)
    );

    let err = splice(
        &root,
        0,
        &plan_args(
            "card.md",
            vec![PlanEdit::Create {
                parent_hpath: String::new(),
                title: "Brand".into(),
                body: "b".into(),
            }],
        ),
        &[],
    )
    .expect_err("top-level create refuses");
    assert_eq!(
        err.message.as_deref(),
        Some(r#"cannot place new section "Brand" — its parent is not in the document"#)
    );

    assert_eq!(
        std::fs::read_to_string(dir.path().join("card.md")).expect("read"),
        "# Tasks\n\n- [ ] one ^task1\n",
        "refusals leave the file untouched"
    );
}
