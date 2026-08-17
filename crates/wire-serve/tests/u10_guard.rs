//! U10 gates: fingerprint-or-force at the wire-origin splice intake.
//!
//! The law (R1.1): via the wire, no content change reaches disk without its
//! fingerprint; `force` is the ONLY bypass. Every scope rule gets a named test.

use std::path::PathBuf;

use wire::{
    Edit, EditShape, ErrorCode, HpathSeg, NodeRev, Path as WPath, PlanEdit, PutAt, ResponseBody,
    SecRef,
};
use wire_serve::guard::Origin;
use wire_serve::write::{SpliceArgs, splice};

const DOC: &str =
    "---\nstatus: open\nowner: d\n---\n# Memo\n\nbody line\n\n## Tasks\n\n- item one\n";

fn ws() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("memo.md"), DOC).expect("seed");
    let root = fs::WorkspaceRoot(PathBuf::from(dir.path()));
    (dir, root)
}

fn args(origin: Origin) -> SpliceArgs {
    SpliceArgs {
        premises: Vec::new(),
        id: None,
        origin,
        path: WPath("memo.md".into()),
        actor: Some("agent:alice".into()),
        now: Some("2026-08-03T12:00:00Z".into()),
        receipt: None,
        if_root: None,
        dry: false,
        force: false,
        edits: Vec::new(),
        plan_edits: Vec::new(),
        pin: None,
        fields: Default::default(),
    }
}

fn seg(h: &str) -> HpathSeg {
    HpathSeg {
        h: h.into(),
        n: None,
    }
}

fn tasks() -> SecRef {
    SecRef::Hpath {
        hpath: vec![seg("Memo"), seg("Tasks")],
    }
}

/// The section's live `sec_rev` — what a caller reads before it writes.
fn sec_rev(root: &fs::WorkspaceRoot, sec: SecRef) -> NodeRev {
    let doc = fs::load(root, std::path::Path::new("memo.md")).expect("load");
    match wire_serve::read::cat(&doc, Some(sec)).expect("cat") {
        ResponseBody::Cat { node_rev, .. } => node_rev,
        other => panic!("cat returned {other:?}"),
    }
}

/// The document's own fingerprint — the FILE-grain token `set_property` wants.
fn file_rev(root: &fs::WorkspaceRoot) -> String {
    let doc = fs::load(root, std::path::Path::new("memo.md")).expect("load");
    doc.root.node_rev.0.clone()
}

fn append_edit(rev: Option<NodeRev>) -> Edit {
    Edit {
        target: tasks(),
        edit: EditShape::Put {
            at: PutAt::End,
            text: "- item two\n".into(),
        },
        if_node_rev: rev,
    }
}

fn replace_edit(rev: Option<NodeRev>) -> Edit {
    Edit {
        target: tasks(),
        edit: EditShape::Match {
            old: "item one".into(),
            new: "item ONE".into(),
        },
        if_node_rev: rev,
    }
}

/// The guard's OWN refusal contract — its own assertion, never a loosening of
/// `assert_refusal_contract` (the precedent `assert_both_planes_contract` set).
/// Four properties plus one negative: the subject, the cause AT ITS GRAIN, the
/// partial state, a runnable fix — and never an internal mode name.
fn assert_guard_contract(ctx: &str, message: &str, grain: &str) {
    assert!(
        message.contains("memo.md"),
        "{ctx}: names the file it is talking about: {message}"
    );
    assert!(
        message.contains("changes existing content with no fingerprint"),
        "{ctx}: names the cause: {message}"
    );
    assert!(
        message.contains(grain),
        "{ctx}: names the GRAIN the guard is demanded at ({grain}): {message}"
    );
    assert!(
        message.contains("No edit was applied; the batch is refused whole."),
        "{ctx}: discloses the partial state: {message}"
    );
    assert!(
        message.contains("Fix:") && message.contains("mrd read memo.md --json"),
        "{ctx}: the fix names a RUNNABLE COMMAND that mints the token: {message}"
    );
    assert!(
        message.contains("`force`"),
        "{ctx}: names the ONE sanctioned bypass: {message}"
    );
    assert!(
        !message.contains("mode toc") && !message.contains("mode sections"),
        "{ctx}: never names an internal mode name: {message}"
    );
}

// ── The core law ────────────────────────────────────────────────────────────

#[test]
fn wire_edit_on_existing_content_without_a_guard_is_refused() {
    let (_d, root) = ws();
    let a = SpliceArgs {
        premises: Vec::new(),
        edits: vec![replace_edit(None)],
        ..args(Origin::Wire)
    };
    let err = splice(&root, None, &a, &[], None).expect_err("refuses");

    assert_eq!(err.code, ErrorCode::GuardRequired);
    assert_eq!(err.recovery, wire::Recovery::Fix);
    assert_eq!(err.path.as_ref().map(|p| p.0.as_str()), Some("memo.md"));
    let message = err.message.as_deref().expect("a teaching message");
    assert_guard_contract("guard_required", message, "NODE grain");
    assert!(
        message.contains("section \"Memo/Tasks\""),
        "names the subject exactly: {message}"
    );
    // Nothing landed.
    assert_eq!(
        std::fs::read_to_string(root.0.join("memo.md")).expect("read"),
        DOC
    );
}

#[test]
fn wire_edit_with_the_right_if_node_rev_succeeds() {
    let (_d, root) = ws();
    let rev = sec_rev(&root, tasks());
    let a = SpliceArgs {
        premises: Vec::new(),
        edits: vec![replace_edit(Some(rev))],
        ..args(Origin::Wire)
    };
    splice(&root, None, &a, &[], None).expect("a guarded wire write lands");
    assert!(
        std::fs::read_to_string(root.0.join("memo.md"))
            .expect("read")
            .contains("item ONE")
    );
}

#[test]
fn wire_edit_with_force_succeeds_and_names_the_bypassed_planes() {
    let (_d, root) = ws();
    let a = SpliceArgs {
        premises: Vec::new(),
        edits: vec![replace_edit(None)],
        force: true,
        ..args(Origin::Wire)
    };
    let out = splice(&root, None, &a, &[], None).expect("force is the sanctioned bypass");

    let ResponseBody::Splice { verdicts, .. } = &out.body else {
        panic!("not a splice body");
    };
    let named: Vec<&str> = verdicts.iter().map(|v| v.message.as_str()).collect();
    assert!(
        verdicts.iter().any(|v| v.rule == "fingerprint-or-force"),
        "the forced write renders the plane it bypassed: {named:?}"
    );
    assert!(
        named
            .iter()
            .any(|m| m.contains("content fingerprint (node grain)")
                && m.contains("section \"Memo/Tasks\"")),
        "the render NAMES the plane and its subject: {named:?}"
    );
    assert!(
        std::fs::read_to_string(root.0.join("memo.md"))
            .expect("read")
            .contains("item ONE"),
        "a forced write lands its bytes"
    );
}

/// S3 regression: the law is CONTENT-CHANGE-scoped, never replace-shaped — an
/// append changes existing content and is guarded like any other change.
#[test]
fn s3_append_on_existing_content_is_guarded() {
    let (_d, root) = ws();
    let a = SpliceArgs {
        premises: Vec::new(),
        edits: vec![append_edit(None)],
        ..args(Origin::Wire)
    };
    let err = splice(&root, None, &a, &[], None).expect_err("append is a content change");
    assert_eq!(err.code, ErrorCode::GuardRequired);

    // And it lands with its fingerprint, like every other content change.
    let rev = sec_rev(&root, tasks());
    let ok = SpliceArgs {
        premises: Vec::new(),
        edits: vec![append_edit(Some(rev))],
        ..args(Origin::Wire)
    };
    splice(&root, None, &ok, &[], None).expect("a guarded append lands");
}

/// A plan `append` carries its own node token (R4), so a wire-origin plan
/// append can satisfy the guard through its own face.
#[test]
fn a_plan_append_carrying_its_section_rev_lands() {
    let (_d, root) = ws();
    let rev = sec_rev(&root, tasks());
    let a = SpliceArgs {
        premises: Vec::new(),
        plan_edits: vec![PlanEdit::Append {
            hpath: vec![
                HpathSeg {
                    h: "Memo".into(),
                    n: None,
                },
                HpathSeg {
                    h: "Tasks".into(),
                    n: None,
                },
            ],
            body: "- item two".into(),
            rev: Some(rev.0),
        }],
        ..args(Origin::Wire)
    };
    splice(&root, None, &a, &[], None).expect("a guarded plan append lands");
    assert!(
        std::fs::read_to_string(root.0.join("memo.md"))
            .expect("read")
            .contains("- item two"),
        "the append writes its bytes"
    );
}

/// R4 builds a door; it does not loosen the guard. No `rev` still refuses.
#[test]
fn a_plan_append_without_a_rev_still_refuses() {
    let (_d, root) = ws();
    let a = SpliceArgs {
        premises: Vec::new(),
        plan_edits: vec![PlanEdit::Append {
            hpath: vec![
                HpathSeg {
                    h: "Memo".into(),
                    n: None,
                },
                HpathSeg {
                    h: "Tasks".into(),
                    n: None,
                },
            ],
            body: "- item two".into(),
            rev: None,
        }],
        ..args(Origin::Wire)
    };
    let err = splice(&root, None, &a, &[], None).expect_err("an unguarded append refuses");
    assert_eq!(err.code, ErrorCode::GuardRequired);
    assert!(
        !std::fs::read_to_string(root.0.join("memo.md"))
            .expect("read")
            .contains("- item two"),
        "nothing landed"
    );
}

/// And a WRONG token is not a token: the threaded `rev` reaches CAS, so a stale
/// append refuses there rather than writing over someone else's change.
#[test]
fn a_plan_append_with_a_stale_rev_does_not_write() {
    let (_d, root) = ws();
    let a = SpliceArgs {
        premises: Vec::new(),
        plan_edits: vec![PlanEdit::Append {
            hpath: vec![
                HpathSeg {
                    h: "Memo".into(),
                    n: None,
                },
                HpathSeg {
                    h: "Tasks".into(),
                    n: None,
                },
            ],
            body: "- item two".into(),
            rev: Some("not-this-sections-rev".into()),
        }],
        ..args(Origin::Wire)
    };
    let err = splice(&root, None, &a, &[], None).expect_err("a stale append token refuses");
    assert_eq!(err.code, ErrorCode::CasMismatch);
    assert!(
        !std::fs::read_to_string(root.0.join("memo.md"))
            .expect("read")
            .contains("- item two"),
        "nothing landed"
    );
}

/// A wire write that stripped the `force` flag AND the rev is not a way in:
/// `force` is the one bypass, and it is explicit.
#[test]
fn a_stale_fingerprint_still_refuses_at_cas() {
    let (_d, root) = ws();
    let a = SpliceArgs {
        premises: Vec::new(),
        edits: vec![replace_edit(Some(NodeRev("not-this-documents-rev".into())))],
        ..args(Origin::Wire)
    };
    let err = splice(&root, None, &a, &[], None).expect_err("a stale token refuses");
    assert_eq!(err.code, ErrorCode::CasMismatch);
}

/// `force` is wired to CAS, not only the armed gate.
#[test]
fn force_is_wired_to_cas() {
    let (_d, root) = ws();
    let a = SpliceArgs {
        premises: Vec::new(),
        edits: vec![replace_edit(Some(NodeRev("not-this-documents-rev".into())))],
        force: true,
        ..args(Origin::Wire)
    };
    splice(&root, None, &a, &[], None)
        .expect("force bypasses the fingerprint plane, stale token and all");
}

// ── Births, frontmatter, the empty batch, the exemptions ────────────────────

/// Births are guarded by ABSENCE, not by fingerprint: the whole-file birth op
/// carries no fingerprint and refuses `cas_mismatch` on an occupied path.
#[test]
fn birth_is_guarded_by_absence_not_fingerprint() {
    let (_d, root) = ws();
    let born = wire_serve::write::create(
        &root,
        None,
        &wire_serve::write::CreateArgs {
            id: None,
            path: WPath("fresh.md".into()),
            body: "# Fresh\n".into(),
            actor: Some("agent:alice".into()),
            now: Some("2026-08-03T12:00:00Z".into()),
            if_root: None,
            dry: false,
            fields: Default::default(),
        },
        &[],
    );
    assert!(born.is_ok(), "a birth needs no fingerprint: {born:?}");

    let clobber = wire_serve::write::create(
        &root,
        None,
        &wire_serve::write::CreateArgs {
            id: None,
            path: WPath("fresh.md".into()),
            body: "# Again\n".into(),
            actor: Some("agent:alice".into()),
            now: Some("2026-08-03T12:00:00Z".into()),
            if_root: None,
            dry: false,
            fields: Default::default(),
        },
        &[],
    )
    .expect_err("an occupied path refuses");
    assert_eq!(clobber.code, ErrorCode::CasMismatch);
}

/// A plan `create` is a section birth: guarded by absence at the section grain,
/// because lowering has already turned it into a parent-append.
#[test]
fn plan_create_is_guarded_by_absence() {
    let (_d, root) = ws();
    let already = SpliceArgs {
        premises: Vec::new(),
        plan_edits: vec![PlanEdit::Create {
            parent_hpath: vec![HpathSeg {
                h: "Memo".into(),
                n: None,
            }],
            title: "Tasks".into(),
            body: "x\n".into(),
            rev: None,
        }],
        ..args(Origin::Wire)
    };
    let err = splice(&root, None, &already, &[], None).expect_err("that section is already there");
    assert_eq!(err.code, ErrorCode::CasMismatch);
    let message = err.message.as_deref().expect("a message");
    assert!(
        message.contains("guarded by ABSENCE") && message.contains("Fix:"),
        "the birth refusal teaches: {message}"
    );

    let fresh = SpliceArgs {
        premises: Vec::new(),
        plan_edits: vec![PlanEdit::Create {
            parent_hpath: vec![HpathSeg {
                h: "Memo".into(),
                n: None,
            }],
            title: "Notes".into(),
            body: "x\n".into(),
            rev: None,
        }],
        ..args(Origin::Wire)
    };
    splice(&root, None, &fresh, &[], None)
        .expect("an absent section is born without a fingerprint");
}

#[test]
fn set_properties_demands_the_doc_root_token() {
    let (_d, root) = ws();
    let bare = SpliceArgs {
        premises: Vec::new(),
        plan_edits: vec![PlanEdit::SetProperty {
            key: "status".into(),
            value: "closed".into(),
            rev: None,
        }],
        ..args(Origin::Wire)
    };
    let err =
        splice(&root, None, &bare, &[], None).expect_err("frontmatter needs the file-grain token");
    assert_eq!(err.code, ErrorCode::GuardRequired);
    let message = err.message.as_deref().expect("a teaching message");
    assert_guard_contract("set_properties", message, "FILE grain");
    assert!(
        message.contains("frontmatter semantics are file-scoped"),
        "the refusal teaches WHY the grain is the file: {message}"
    );

    let token = file_rev(&root);
    let guarded = SpliceArgs {
        premises: Vec::new(),
        plan_edits: vec![PlanEdit::SetProperty {
            key: "status".into(),
            value: "closed".into(),
            rev: Some(token),
        }],
        ..args(Origin::Wire)
    };
    splice(&root, None, &guarded, &[], None).expect("with the doc-root token it lands");
    assert!(
        std::fs::read_to_string(root.0.join("memo.md"))
            .expect("read")
            .contains("status: closed")
    );
}

#[test]
fn set_properties_with_a_stale_doc_root_token_refuses() {
    let (_d, root) = ws();
    let stale = SpliceArgs {
        premises: Vec::new(),
        plan_edits: vec![PlanEdit::SetProperty {
            key: "status".into(),
            value: "closed".into(),
            rev: Some("some-other-documents-rev".into()),
        }],
        ..args(Origin::Wire)
    };
    let err = splice(&root, None, &stale, &[], None).expect_err("a stale file token refuses");
    assert_eq!(err.code, ErrorCode::CasMismatch);
}

/// `mrd pin` is an EMPTY batch. Per-edit scope is what leaves it untouched — a
/// batch-grain guard would have broken the pin verb outright.
#[test]
fn an_empty_batch_is_unaffected() {
    let (_d, root) = ws();
    let pin_shaped = args(Origin::Wire);
    let out = splice(&root, None, &pin_shaped, &[], None);
    assert!(
        out.is_ok(),
        "an edit-less splice has nothing to guard: {out:?}"
    );
}

/// The in-process path is not a wire door, so the ruling does not reach it.
/// This is SCOPE, not a trust class: no door is exempted for who is behind it.
#[test]
fn the_in_process_path_is_outside_the_rulings_reach() {
    let (_d, root) = ws();
    let a = SpliceArgs {
        premises: Vec::new(),
        edits: vec![replace_edit(None)],
        ..args(Origin::InProcess)
    };
    splice(&root, None, &a, &[], None).expect("not a wire door — the ruling does not govern it");
    assert!(
        std::fs::read_to_string(root.0.join("memo.md"))
            .expect("read")
            .contains("item ONE")
    );
}

/// The plan face spells the guard differently, so its fix clause must name the
/// slot THAT face has. `append` has no rev field at all: a message sending the
/// caller to one would be a refusal pointing at a door that does not open.
#[test]
fn the_fix_clause_names_the_slot_the_caller_actually_has() {
    let (_d, root) = ws();

    let plan_match = SpliceArgs {
        premises: Vec::new(),
        plan_edits: vec![PlanEdit::Match {
            hpath: vec![
                HpathSeg {
                    h: "Memo".into(),
                    n: None,
                },
                HpathSeg {
                    h: "Tasks".into(),
                    n: None,
                },
            ],
            old: "item one".into(),
            new: "item ONE".into(),
            all: false,
            rev: None,
        }],
        ..args(Origin::Wire)
    };
    let err = splice(&root, None, &plan_match, &[], None).expect_err("refuses");
    let message = err.message.as_deref().expect("a message");
    assert_guard_contract("plan match", message, "NODE grain");
    assert!(
        message.contains("`rev` on the plan edit"),
        "the plan face is told about `rev`, not `if_node_rev`: {message}"
    );

    let plan_append = SpliceArgs {
        premises: Vec::new(),
        plan_edits: vec![PlanEdit::Append {
            hpath: vec![
                HpathSeg {
                    h: "Memo".into(),
                    n: None,
                },
                HpathSeg {
                    h: "Tasks".into(),
                    n: None,
                },
            ],
            body: "- item two".into(),
            rev: None,
        }],
        ..args(Origin::Wire)
    };
    let err = splice(&root, None, &plan_append, &[], None).expect_err("append is guarded (S3)");
    let message = err.message.as_deref().expect("a message");
    assert_guard_contract("plan append", message, "NODE grain");
    // `append` carries its own `rev` (R4), so the fix names that field — the
    // same door `match` is sent to.
    assert!(
        message.contains("`rev` on the plan edit"),
        "the append face is told about its own `rev`: {message}"
    );
    assert!(
        !message.contains("`append` carries no rev field"),
        "the retired unslotted clause must be gone: {message}"
    );
}

/// A guard mounted at plan lowering would be MCP-only: a native `edits`
/// payload never goes through lowering, so a field rename walks around it.
/// The guard mounts post-lowering at the intake — the bypass is closed.
#[test]
fn the_field_rename_bypass_is_closed() {
    let (_d, root) = ws();
    // A payload that never touches `crate::plan::lower`: native `edits` only.
    let native = SpliceArgs {
        premises: Vec::new(),
        edits: vec![replace_edit(None)],
        plan_edits: Vec::new(),
        ..args(Origin::Wire)
    };
    assert!(native.plan_edits.is_empty(), "this payload is not lowered");

    let err = splice(&root, None, &native, &[], None).expect_err("the native face is guarded too");
    assert_eq!(
        err.code,
        ErrorCode::GuardRequired,
        "a native payload must not walk around the guard"
    );
}

// ── Frame legality vs semantic refusal ──────────────────────────────────────
//
// A guardless splice is still a legal frame that DECODES; guard fields stay
// schema-optional. Only the write is refused, semantically, after decode.
// These two tests are the seam.

/// A guardless splice frame is STILL A LEGAL FRAME: it decodes clean. Guard
/// fields stay schema-optional — the refusal must never be a decode failure.
#[test]
fn a_guardless_frame_is_still_a_legal_frame() {
    let frame = serde_json::json!({
        "op": "splice",
        "path": "memo.md",
        "edits": [{
            "target": {"hpath": [{"h": "Memo"}, {"h": "Tasks"}]},
            "edit": {"match": {"old": "item one", "new": "item ONE"}},
        }],
    });
    let obj = frame.as_object().expect("frame object");

    for rev in [wire_serve::rev::Rev::V2, wire_serve::rev::Rev::V3] {
        let decoded = wire_serve::decode::decode(obj, rev);
        assert!(
            decoded.is_ok(),
            "a guardless splice must DECODE under {rev:?} — guard fields stay \
             schema-optional (decision 007's surviving half): {:?}",
            decoded.err()
        );
    }
}

/// …and the guard's answer is a SEMANTIC refusal on the write, never
/// frame-illegality. `guard_required` is a fix-class refusal about the WRITE;
/// `bad_frame`/`bad_request` would say the caller's frame was malformed, which
/// under this ruling would be a lie and a violation.
#[test]
fn the_refusal_is_semantic_never_frame_illegality() {
    let (_d, root) = ws();
    let a = SpliceArgs {
        premises: Vec::new(),
        edits: vec![replace_edit(None)],
        ..args(Origin::Wire)
    };
    let err = splice(&root, None, &a, &[], None).expect_err("the WRITE is refused");

    assert_eq!(err.code, ErrorCode::GuardRequired);
    assert_ne!(
        err.code,
        ErrorCode::BadFrame,
        "the frame was well-formed; saying otherwise violates the ruling"
    );
    assert_ne!(
        err.code,
        ErrorCode::BadRequest,
        "the request was well-formed; the WRITE is what is refused"
    );
}

/// Ordering: a target that does not resolve is not this rung's to answer —
/// the resolution rung refuses (`ref_not_found` / `ambiguous_ref`) so the
/// caller's real mistake is named, not buried behind a fingerprint demand.
/// This pins the unit (the end-to-end driver died with the sidecar host —
/// §3.3 DROP, 2026-08-06).
#[test]
fn a_target_that_does_not_resolve_is_not_this_rungs_to_answer() {
    let (_d, root) = ws();
    let dangling = SpliceArgs {
        premises: Vec::new(),
        edits: vec![Edit {
            target: SecRef::Anchor {
                anchor: "no-such-anchor".into(),
            },
            edit: EditShape::Match {
                old: "item one".into(),
                new: "item ONE".into(),
            },
            if_node_rev: None,
        }],
        ..args(Origin::Wire)
    };
    let err = splice(&root, None, &dangling, &[], None).expect_err("refuses");
    assert_eq!(
        err.code,
        ErrorCode::RefNotFound,
        "the selector's own refusal speaks, not the guard: {:?}",
        err.message
    );

    // The same target, once it exists, IS the guard's to answer.
    let real = SpliceArgs {
        premises: Vec::new(),
        edits: vec![replace_edit(None)],
        ..args(Origin::Wire)
    };
    assert_eq!(
        splice(&root, None, &real, &[], None)
            .expect_err("refuses")
            .code,
        ErrorCode::GuardRequired,
        "existing content is guarded"
    );
}

// ── Law A-1 at the create door (`create.rev`, § A.3) ────────────────────────
//
// `{h, n}` binds by position among identical texts: between a caller's read
// and its create, a same-titled sibling insert re-binds `n`, and the
// child-absence check cannot see it. A create under an `n`-bearing parent
// therefore demands the CALLER's parent rev (or `force`). Occurrence floor
// only: rev-free creates at unique parents stay legal (the existing
// `plan_create_is_guarded_by_absence` pins that, unmodified).

/// Two identical-texted siblings — the occurrence class made flesh.
const DUP: &str = "# Memo\n\nbody\n\n## Tasks\n\n- alpha\n\n## Tasks\n\n- beta\n";

fn dup_ws() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("dup.md"), DUP).expect("seed");
    let root = fs::WorkspaceRoot(PathBuf::from(dir.path()));
    (dir, root)
}

fn dup_args() -> SpliceArgs {
    SpliceArgs {
        premises: Vec::new(),
        path: WPath("dup.md".into()),
        ..args(Origin::Wire)
    }
}

/// The SECOND `Tasks` under `Memo` — an occurrence-addressed parent.
fn occurrence_parent() -> Vec<HpathSeg> {
    vec![
        HpathSeg {
            h: "Memo".into(),
            n: None,
        },
        HpathSeg {
            h: "Tasks".into(),
            n: Some(2),
        },
    ]
}

/// The occurrence parent's live `sec_rev` on `dup.md`.
fn dup_parent_rev(root: &fs::WorkspaceRoot) -> NodeRev {
    let doc = fs::load(root, std::path::Path::new("dup.md")).expect("load");
    let sec = SecRef::Hpath {
        hpath: occurrence_parent(),
    };
    match wire_serve::read::cat(&doc, Some(sec)).expect("cat") {
        ResponseBody::Cat { node_rev, .. } => node_rev,
        other => panic!("cat returned {other:?}"),
    }
}

fn occurrence_create(rev: Option<String>) -> PlanEdit {
    PlanEdit::Create {
        parent_hpath: occurrence_parent(),
        title: "Kid".into(),
        body: "x".into(),
        rev,
    }
}

/// The demand: rev absent, `n`-bearing parent → `guard_required`, teaching the
/// slot (`create.rev`), the occurrence why, and the toc read that mints the
/// token. Nothing lands.
#[test]
fn a_create_under_an_occurrence_parent_without_the_parent_rev_refuses() {
    let (_d, root) = dup_ws();
    let a = SpliceArgs {
        premises: Vec::new(),
        plan_edits: vec![occurrence_create(None)],
        ..dup_args()
    };
    let err = splice(&root, None, &a, &[], None).expect_err("an unguardable birth refuses");

    assert_eq!(err.code, ErrorCode::GuardRequired);
    assert_eq!(err.recovery, wire::Recovery::Fix);
    let message = err.message.as_deref().expect("a teaching message");
    assert!(
        message.contains("section \"Memo/Tasks#2\""),
        "names the occurrence parent exactly: {message}"
    );
    assert!(
        message.contains("OCCURRENCE-addressed parent"),
        "names the class that makes the demand non-negotiable: {message}"
    );
    assert!(
        message.contains("`rev` on the `create`") && message.contains("PARENT section's `sec_rev`"),
        "the fix names the slot THIS face has — the create row's `rev`, minted \
         from the parent: {message}"
    );
    assert!(
        message.contains("Fix:") && message.contains("mrd read dup.md --json"),
        "the fix names a RUNNABLE COMMAND: {message}"
    );
    assert!(
        message.contains("No edit was applied; the batch is refused whole."),
        "discloses the partial state: {message}"
    );
    assert!(
        message.contains("`force`"),
        "names the ONE sanctioned bypass: {message}"
    );
    assert_eq!(
        std::fs::read_to_string(root.0.join("dup.md")).expect("read"),
        DUP,
        "nothing landed"
    );
}

/// The honored token: the caller's parent rev threads to the lowered append's
/// `if_node_rev`, and the birth lands inside the SECOND `Tasks`.
#[test]
fn a_create_under_an_occurrence_parent_with_the_parent_rev_lands() {
    let (_d, root) = dup_ws();
    let rev = dup_parent_rev(&root);
    let a = SpliceArgs {
        premises: Vec::new(),
        plan_edits: vec![occurrence_create(Some(rev.0))],
        ..dup_args()
    };
    splice(&root, None, &a, &[], None).expect("a guarded occurrence birth lands");
    let text = std::fs::read_to_string(root.0.join("dup.md")).expect("read");
    assert!(
        text.contains("### Kid"),
        "the child is born at parent depth + 1: {text}"
    );
    assert!(
        text.ends_with("- beta\n\n### Kid\n\nx\n"),
        "the birth appends inside the SECOND `Tasks`, the one the rev vouches for: {text}"
    );
}

/// A stale parent rev is a rebind observed: the threaded token reaches CAS and
/// refuses `cas_mismatch` — the conflict answer, never a silent land.
#[test]
fn a_create_under_an_occurrence_parent_with_a_stale_rev_refuses_at_cas() {
    let (_d, root) = dup_ws();
    let a = SpliceArgs {
        premises: Vec::new(),
        plan_edits: vec![occurrence_create(Some("not-the-parents-rev".into()))],
        ..dup_args()
    };
    let err = splice(&root, None, &a, &[], None).expect_err("a stale parent token refuses");
    assert_eq!(err.code, ErrorCode::CasMismatch);
    assert_eq!(
        std::fs::read_to_string(root.0.join("dup.md")).expect("read"),
        DUP,
        "nothing landed"
    );
}

/// Occurrence-free parents: an OFFERED rev is honored (CAS) wherever present —
/// fresh lands, stale refuses. The demand stays at the occurrence floor; this
/// is the §5.3 ratchet's named future amendment, not this law.
#[test]
fn a_create_at_a_unique_parent_honors_an_offered_rev() {
    let (_d, root) = ws();
    let parent = SecRef::Hpath {
        hpath: vec![seg("Memo"), seg("Tasks")],
    };
    let fresh = sec_rev(&root, parent);
    let ok = SpliceArgs {
        premises: Vec::new(),
        plan_edits: vec![PlanEdit::Create {
            parent_hpath: vec![seg("Memo"), seg("Tasks")],
            title: "Kid".into(),
            body: "x".into(),
            rev: Some(fresh.0),
        }],
        ..args(Origin::Wire)
    };
    splice(&root, None, &ok, &[], None).expect("a fresh offered rev lands");
    assert!(
        std::fs::read_to_string(root.0.join("memo.md"))
            .expect("read")
            .contains("### Kid")
    );

    let stale = SpliceArgs {
        premises: Vec::new(),
        plan_edits: vec![PlanEdit::Create {
            parent_hpath: vec![seg("Memo"), seg("Tasks")],
            title: "Another".into(),
            body: "x".into(),
            rev: Some("not-the-parents-rev".into()),
        }],
        ..args(Origin::Wire)
    };
    let err = splice(&root, None, &stale, &[], None).expect_err("a stale offered rev refuses");
    assert_eq!(err.code, ErrorCode::CasMismatch);
}

/// `force` bypasses the create demand exactly as it bypasses every
/// fingerprint-plane demand: loud, with the bypassed plane named against the
/// parent subject.
#[test]
fn force_bypasses_the_create_demand_and_names_the_parent() {
    let (_d, root) = dup_ws();
    let a = SpliceArgs {
        premises: Vec::new(),
        plan_edits: vec![occurrence_create(None)],
        force: true,
        ..dup_args()
    };
    let out = splice(&root, None, &a, &[], None).expect("force is the sanctioned bypass");
    let ResponseBody::Splice { verdicts, .. } = &out.body else {
        panic!("not a splice body");
    };
    let named: Vec<&str> = verdicts.iter().map(|v| v.message.as_str()).collect();
    assert!(
        named
            .iter()
            .any(|m| m.contains("content fingerprint (node grain)")
                && m.contains("section \"Memo/Tasks#2\"")),
        "the forced birth NAMES the bypassed plane and the parent: {named:?}"
    );
    assert!(
        std::fs::read_to_string(root.0.join("dup.md"))
            .expect("read")
            .contains("### Kid"),
        "a forced write lands its bytes"
    );
}
