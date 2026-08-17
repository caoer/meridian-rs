//! U8b: `plan_edits` lowering ≡ host-built native batch (bytes, armed, root).
//! Flow: flock → lower → validate → commit → Delta as one native splice.

use std::path::PathBuf;

use std::collections::BTreeMap;
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
        premises: Vec::new(),
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
        fields: BTreeMap::default(),
    }
}

fn native_args(path: &str, edits: Vec<Edit>) -> SpliceArgs {
    SpliceArgs {
        premises: Vec::new(),
        edits,
        plan_edits: Vec::new(),
        pin: None,
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

/// Equivalence: mixed plan (property+append+create) lands same bytes as native.
#[test]
#[allow(clippy::too_many_lines)] // one A/B equivalence script by design
fn plan_batch_equals_the_host_built_native_batch() {
    // A: plan form.
    let (da, ra) = ws(&[("card.md", DOC)]);
    let out_a = splice(
        &ra,
        None,
        &plan_args(
            "card.md",
            vec![
                PlanEdit::SetProperty {
                    key: "status".into(),
                    value: "closed".into(),
                    rev: None,
                },
                PlanEdit::Append {
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
                    body: "- item three".into(),
                    rev: None,
                },
                PlanEdit::Create {
                    parent_hpath: vec![HpathSeg {
                        h: "Archive".into(),
                        n: None,
                    }],
                    title: "Log".into(),
                    body: "created".into(),
                    rev: None,
                },
            ],
        ),
        &[],
        None,
    )
    .expect("plan splice commits");

    // B: native batch (prop Put{all}, append, parent-append create).
    let (db, rb) = ws(&[("card.md", DOC)]);
    let out_b = splice(
        &rb,
        None,
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
                // The § A.3 hygiene composition: the appended list item joins
                // the trailing list flush, so the section's trailing blank
                // line must move — a content-span rewrite, not an end-insert.
                Edit {
                    target: SecRef::Hpath {
                        hpath: vec![seg("Memo"), seg("Tasks")],
                    },
                    edit: EditShape::Put {
                        at: PutAt::Content,
                        text: "\n- item one\n- item two\n- item three\n\n".into(),
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
        None,
    )
    .expect("native splice commits");

    let bytes_a = std::fs::read(da.path().join("card.md")).expect("read A");
    let bytes_b = std::fs::read(db.path().join("card.md")).expect("read B");
    assert_eq!(
        String::from_utf8_lossy(&bytes_a),
        String::from_utf8_lossy(&bytes_b),
        "plan_edits and the host-built native batch land IDENTICAL bytes"
    );

    // Armed 1:1.
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

/// Match-all + `replace_section` land expected literal bytes.
#[test]
fn replace_section_and_match_all_land_expected_bytes() {
    let (dir, root) = ws(&[("card.md", DOC)]);

    // Live Tasks section rev.
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
        None,
        &plan_args(
            "card.md",
            vec![PlanEdit::Match {
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
                old: "item".into(),
                new: "task".into(),
                all: true,
                rev: Some(tasks_rev),
            }],
        ),
        &[],
        None,
    )
    .expect("match-all commits");

    let after = std::fs::read_to_string(dir.path().join("card.md")).expect("read");
    assert_eq!(
        after,
        "---\nstatus: open\nowner: d\n---\n# Memo\n\nbody line\n\n## Tasks\n\n- task one\n- task two\n\n# Archive\n\nold\n"
    );

    // replace_section under fresh rev.
    let doc = fs::load(&root, std::path::Path::new("card.md")).expect("load");
    let fresh = model::resolve(&doc, &target).expect("resolves").node_rev.0;
    splice(
        &root,
        None,
        &plan_args(
            "card.md",
            vec![PlanEdit::ReplaceSection {
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
                body: "- done".into(),
                rev: Some(fresh),
            }],
        ),
        &[],
        None,
    )
    .expect("replace_section commits");
    let after = std::fs::read_to_string(dir.path().join("card.md")).expect("read");
    assert_eq!(
        after,
        "---\nstatus: open\nowner: d\n---\n# Memo\n\nbody line\n\n## Tasks\n\n- done\n\n# Archive\n\nold\n",
        "§ A.3 hygiene: one blank line under the heading, one before the next heading"
    );
}

/// Plan `rev` threads to native CAS (`if_node_rev`); stale → `cas_mismatch`.
#[test]
fn plan_rev_threads_into_the_native_cas_guard() {
    let (dir, root) = ws(&[("card.md", DOC)]);
    let err = splice(
        &root,
        None,
        &plan_args(
            "card.md",
            vec![PlanEdit::ReplaceSection {
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
                body: "- clobber".into(),
                rev: Some("0000000000000000".into()),
            }],
        ),
        &[],
        None,
    )
    .expect_err("stale rev refuses");
    assert_eq!(err.code, wire::ErrorCode::CasMismatch);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("card.md")).expect("read"),
        DOC,
        "nothing landed"
    );
}

/// S4b/D11: multiline `set_property` value forges keys → `bad_request`, no write.
#[test]
fn plan_set_property_refuses_multiline_values_and_writes_nothing() {
    let (dir, root) = ws(&[("card.md", DOC)]);

    // Forged-key spelling (no ": " → no quote).
    let err = splice(
        &root,
        None,
        &plan_args(
            "card.md",
            vec![PlanEdit::SetProperty {
                key: "status".into(),
                value: "closed\ninjected:pwned".into(),
                rev: None,
            }],
        ),
        &[],
        None,
    )
    .expect_err("a multi-line property value refuses");
    assert_eq!(err.code, wire::ErrorCode::BadRequest);
    assert_eq!(
        err.message.as_deref(),
        Some(
            "property value for \"status\" contains a newline — frontmatter values are single-line in v1; put multi-line content in a body section"
        )
    );

    // Quoted spelling (": " triggers quote; newline still refuses).
    let err = splice(
        &root,
        None,
        &plan_args(
            "card.md",
            vec![PlanEdit::SetProperty {
                key: "status".into(),
                value: "closed\nowner: mallory".into(),
                rev: None,
            }],
        ),
        &[],
        None,
    )
    .expect_err("a quoted-scalar injection refuses too");
    assert_eq!(err.code, wire::ErrorCode::BadRequest);

    // A refused batch REFUSES WHOLE: the legal sibling edit lands nothing.
    let err = splice(
        &root,
        None,
        &plan_args(
            "card.md",
            vec![
                PlanEdit::SetProperty {
                    key: "owner".into(),
                    value: "alice".into(),
                    rev: None,
                },
                PlanEdit::SetProperty {
                    key: "status".into(),
                    value: "closed\ninjected:pwned".into(),
                    rev: None,
                },
            ],
        ),
        &[],
        None,
    )
    .expect_err("the batch refuses whole");
    assert_eq!(err.code, wire::ErrorCode::BadRequest);

    assert_eq!(
        std::fs::read_to_string(dir.path().join("card.md")).expect("read"),
        DOC,
        "no bytes reach disk: the file is byte-unchanged after every refusal"
    );
}

/// Everything a legal `set_property` create PROMISES, and nothing more.
///
/// WHERE inside the block the new key lands is deliberately not asserted:
/// §A.6.3 states "Key ORDER inside the block is not a law of this contract —
/// the auditable identity of the write is", and the create arm lands at the
/// upsert insertion point rather than after the last key. A whole-file spelling
/// pins that non-promise, so it reddens whenever the insertion point
/// legitimately moves — which is how the assertion this replaces became an
/// undeclared base red at `b1fcc6e3` (card `base-red-u8b-plan-edits-key-order`).
fn assert_legal_create_landed(after: &str) {
    assert!(
        after.contains("\nreview-state_2: pending\n"),
        "the legal key landed with its value: {after}"
    );
    assert!(
        after.contains("\ntitle: Plan\n"),
        "the pre-existing key survives the create: {after}"
    );
    assert_eq!(
        after.matches("---\n").count(),
        2,
        "one frontmatter block, both delimiters intact: {after}"
    );
    assert!(
        after.ends_with("---\n\n# Plan\n\nbody\n"),
        "the body below the block is untouched: {after}"
    );
}

/// R5: unsafe property KEY refuses at pre-flight and plan commit.
/// Same owner as value (`yaml_safe_key`); assert is the refusal itself.
#[test]
fn plan_set_property_refuses_forged_keys_at_both_doors_and_writes_nothing() {
    const SEED: &str = "---\ntitle: Plan\n---\n\n# Plan\n\nbody\n";
    const FORGED: &str = "ti tle: forged\nevil";

    // Door 1 — the `check_write` pre-flight over the same edit.
    let prev = model::build(SEED.to_string(), syntax::parse(SEED));
    let err = policy::defs::rebuild(
        &prev,
        &[policy::defs::PlanEdit {
            op: "set_property".into(),
            target: vec![policy::defs::Seg {
                h: FORGED.to_string(),
                n: None,
            }],
            body: "x".into(),
            ..policy::defs::PlanEdit::default()
        }],
        &|raw| model::build(raw.to_string(), syntax::parse(raw)),
    )
    .expect_err("the pre-flight refuses a forged key");
    assert_eq!(
        err.render(),
        "E_FAIL_LOUD: invalid frontmatter key \"ti tle: forged\\nevil\" — a property key is dotted segments of [A-Za-z0-9_-]+ (single line, no spaces or ':')"
    );

    // Door 2 — the committer, reached with NO pre-flight in front of it.
    let (dir, root) = ws(&[("plan.md", SEED)]);
    let err = splice(
        &root,
        None,
        &plan_args(
            "plan.md",
            vec![PlanEdit::SetProperty {
                key: FORGED.into(),
                value: "x".into(),
                rev: None,
            }],
        ),
        &[],
        None,
    )
    .expect_err("the committer refuses what the pre-flight refuses");
    assert_eq!(err.code, wire::ErrorCode::BadRequest);
    assert_eq!(
        err.message.as_deref(),
        Some(
            "invalid frontmatter key \"ti tle: forged\\nevil\" — a property key is dotted segments of [A-Za-z0-9_-]+ (single line, no spaces or ':')"
        )
    );

    // A refused batch REFUSES WHOLE: the legal sibling lands nothing either.
    let err = splice(
        &root,
        None,
        &plan_args(
            "plan.md",
            vec![
                PlanEdit::SetProperty {
                    key: "title".into(),
                    value: "Rewritten".into(),
                    rev: None,
                },
                PlanEdit::SetProperty {
                    key: FORGED.into(),
                    value: "x".into(),
                    rev: None,
                },
            ],
        ),
        &[],
        None,
    )
    .expect_err("the batch refuses whole");
    assert_eq!(err.code, wire::ErrorCode::BadRequest);

    assert_eq!(
        std::fs::read_to_string(dir.path().join("plan.md")).expect("read"),
        SEED,
        "no bytes reach disk: the file is byte-unchanged after every refusal"
    );

    // The owner refuses a charset violation, not a legal key: `[A-Za-z0-9_-]+`
    // still commits, so the guard cannot pass by refusing everything.
    splice(
        &root,
        None,
        &plan_args(
            "plan.md",
            vec![PlanEdit::SetProperty {
                key: "review-state_2".into(),
                value: "pending".into(),
                rev: None,
            }],
        ),
        &[],
        None,
    )
    .expect("a legal key still commits");
    assert_legal_create_landed(&std::fs::read_to_string(dir.path().join("plan.md")).expect("read"));
}

/// dogfood r3 f6: the task grammar's flat dotted keys write through the PATCH
/// face. Birth landed `task.index.caps` and `docs/run-plane.md` mandates the
/// dotted spelling, while this face refused it — one key law, three surfaces.
#[test]
fn plan_set_property_writes_the_task_grammars_dotted_keys() {
    const SEED: &str = "---\ntitle: Card\n---\n\n# Tasks\n\nbody\n";
    let (dir, root) = ws(&[("card.md", SEED)]);

    splice(
        &root,
        None,
        &plan_args(
            "card.md",
            vec![
                PlanEdit::SetProperty {
                    key: "task.index".into(),
                    value: "1".into(),
                    rev: None,
                },
                PlanEdit::SetProperty {
                    key: "task.index.caps".into(),
                    value: "md.set_field".into(),
                    rev: None,
                },
            ],
        ),
        &[],
        None,
    )
    .expect("the dotted task keys commit");

    let after = std::fs::read_to_string(dir.path().join("card.md")).expect("read");
    assert!(
        after.contains("task.index: 1\n") && after.contains("task.index.caps: md.set_field\n"),
        "both dotted keys land as their own lines: {after}"
    );

    // A dot is a separator, not a charset member — an empty segment still
    // refuses, and the refusal teaches the dotted law it broke.
    let err = splice(
        &root,
        None,
        &plan_args(
            "card.md",
            vec![PlanEdit::SetProperty {
                key: "task..index".into(),
                value: "x".into(),
                rev: None,
            }],
        ),
        &[],
        None,
    )
    .expect_err("an empty segment names no property");
    assert_eq!(err.code, wire::ErrorCode::BadRequest);
    assert_eq!(
        err.message.as_deref(),
        Some(
            "invalid frontmatter key \"task..index\" — a property key is dotted segments of [A-Za-z0-9_-]+ (single line, no spaces or ':')"
        )
    );
}

/// Golden MUST-CARRY refusals: p-replace-on-block (now the frontmatter twin
/// — F-R4 made every body host writeable, so the host-excluded class lives
/// on the one remaining excluded host) + p-create-top (engine, no write).
#[test]
fn golden_target_class_refusals_fire_engine_side() {
    let (dir, root) = ws(&[(
        "card.md",
        "---\ntitle: x ^fm-c\n---\n# Tasks\n\n- [ ] one ^task1\n",
    )]);

    // The F-R4 half first: the task-hosted id COMMITS on the plan lane.
    splice(
        &root,
        None,
        &plan_args(
            "card.md",
            vec![PlanEdit::Match {
                hpath: vec![HpathSeg {
                    h: "^task1".into(),
                    n: None,
                }],
                old: "one".into(),
                new: "two".into(),
                all: false,
                rev: None,
            }],
        ),
        &[],
        None,
    )
    .expect("a task-hosted id writes since F-R4");

    let err = splice(
        &root,
        None,
        &plan_args(
            "card.md",
            vec![PlanEdit::Match {
                hpath: vec![HpathSeg {
                    h: "^fm-c".into(),
                    n: None,
                }],
                old: "x".into(),
                new: "y".into(),
                all: false,
                rev: None,
            }],
        ),
        &[],
        None,
    )
    .expect_err("a frontmatter caret target refuses");
    assert_eq!(
        err.message.as_deref(),
        Some(
            "no section addressed by \"^fm-c\". No edit was applied; the batch is \
             refused whole. Fix: `^fm-c` exists in this document, but its host is \
             the frontmatter — a caret there is literal YAML, not a block; \
             frontmatter keys are written through the `props` plane, not a block \
             ref."
        )
    );

    let err = splice(
        &root,
        None,
        &plan_args(
            "card.md",
            vec![PlanEdit::Create {
                parent_hpath: vec![],
                title: "Brand".into(),
                body: "b".into(),
                rev: None,
            }],
        ),
        &[],
        None,
    )
    .expect_err("top-level create refuses");
    assert_eq!(
        err.message.as_deref(),
        Some(r#"cannot place new section "Brand" — its parent is not in the document"#)
    );

    assert_eq!(
        std::fs::read_to_string(dir.path().join("card.md")).expect("read"),
        "---\ntitle: x ^fm-c\n---\n# Tasks\n\n- [ ] two ^task1\n",
        "the one committed edit (the F-R4 task write) is all that landed — \
         both refusals moved no bytes"
    );
}
