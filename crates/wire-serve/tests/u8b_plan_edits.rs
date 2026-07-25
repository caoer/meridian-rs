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
        None,
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
        None,
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
        None,
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

/// S4b/D11: `set_property` through the plan face composes `{key}: {value}` into
/// the frontmatter line, so a multi-line value FORGES keys — this path skips
/// `check_write`'s rebuild, so it must refuse on its own. Both spellings refuse
/// `bad_request` and the file stays byte-identical.
#[test]
fn plan_set_property_refuses_multiline_values_and_writes_nothing() {
    let (dir, root) = ws(&[("card.md", DOC)]);

    // Forged-key spelling: no ": " in the value, so the conditional quote never
    // fires — before the fix this landed a literal `injected:pwned` FM line.
    let err = splice(
        &root,
        0,
        &plan_args(
            "card.md",
            vec![PlanEdit::SetProperty {
                key: "status".into(),
                value: "closed\ninjected:pwned".into(),
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

    // Quoted spelling: the mid-value ": " DOES trigger the single quote, and a
    // single-quoted YAML scalar cannot escape the raw newline — same refusal.
    let err = splice(
        &root,
        0,
        &plan_args(
            "card.md",
            vec![PlanEdit::SetProperty {
                key: "status".into(),
                value: "closed\nowner: mallory".into(),
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
        0,
        &plan_args(
            "card.md",
            vec![
                PlanEdit::SetProperty {
                    key: "owner".into(),
                    value: "alice".into(),
                },
                PlanEdit::SetProperty {
                    key: "status".into(),
                    value: "closed\ninjected:pwned".into(),
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

/// Finding 5 / R5: the KEY is composed into `{key}: {value}` exactly as the
/// value is, so it has the SAME fallible owner (`policy::defs::yaml_safe_key`
/// → `SafeKey`) — and BOTH doors discharge it. The pre-flight (`rebuild`) and
/// the committer (`splice.plan_edits`, which skips `check_write`) refuse the
/// identical edit; before the fix the committer accepted it and forged two
/// frontmatter lines. The assert is the REFUSAL itself: a fix that sanitized
/// the key into something legal and committed would fail here.
#[test]
fn plan_set_property_refuses_forged_keys_at_both_doors_and_writes_nothing() {
    // The reviewer's repro, verbatim (review 2026-07-25-0845-64d761b1 § #5).
    const SEED: &str = "---\ntitle: Plan\n---\n\n# Plan\n\nbody\n";
    const FORGED: &str = "ti tle: forged\nevil";

    // Door 1 — the `check_write` pre-flight over the same edit.
    let prev = model::build(SEED.to_string(), syntax::parse(SEED));
    let err = policy::defs::rebuild(
        &prev,
        &[policy::defs::PlanEdit {
            op: "set_property".into(),
            target: FORGED.into(),
            body: "x".into(),
            ..policy::defs::PlanEdit::default()
        }],
        &|raw| model::build(raw.to_string(), syntax::parse(raw)),
    )
    .expect_err("the pre-flight refuses a forged key");
    assert_eq!(
        err.render(),
        "E_FAIL_LOUD: invalid frontmatter key \"ti tle: forged\\nevil\" — a property key is [A-Za-z0-9_-]+ (single line, no spaces or ':')"
    );

    // Door 2 — the committer, reached with NO pre-flight in front of it.
    let (dir, root) = ws(&[("plan.md", SEED)]);
    let err = splice(
        &root,
        0,
        &plan_args(
            "plan.md",
            vec![PlanEdit::SetProperty {
                key: FORGED.into(),
                value: "x".into(),
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
            "invalid frontmatter key \"ti tle: forged\\nevil\" — a property key is [A-Za-z0-9_-]+ (single line, no spaces or ':')"
        )
    );

    // A refused batch REFUSES WHOLE: the legal sibling lands nothing either.
    let err = splice(
        &root,
        0,
        &plan_args(
            "plan.md",
            vec![
                PlanEdit::SetProperty {
                    key: "title".into(),
                    value: "Rewritten".into(),
                },
                PlanEdit::SetProperty {
                    key: FORGED.into(),
                    value: "x".into(),
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
        0,
        &plan_args(
            "plan.md",
            vec![PlanEdit::SetProperty {
                key: "review-state_2".into(),
                value: "pending".into(),
            }],
        ),
        &[],
        None,
    )
    .expect("a legal key still commits");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("plan.md")).expect("read"),
        "---\ntitle: Plan\nreview-state_2: pending\n---\n\n# Plan\n\nbody\n"
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
        None,
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
        None,
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
