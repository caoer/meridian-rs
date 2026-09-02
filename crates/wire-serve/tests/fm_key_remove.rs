//! `remove` — the identity-plane door on `fm_key` (§ A.6.6).
//!
//! The defect it closes, stated as a count: R4 ratifies THREE frontmatter
//! states (`absent ≠ empty ≠ set`); the read face serves all three, the props1
//! digest distinguishes all three, the def plane judges all three — and the
//! write plane reached ABSENT only by rewriting the whole document, which names
//! no key and arms no fact about one. So the fleet wrote `k: ""` meaning "gone"
//! (5,700+ rows measured standing in that state on one live corpus), and every
//! liveness screen testing for absence read them as present.
//!
//! These tests pin the door's laws, not its convenience: the region carries the
//! terminator, the LAST key refuses, an absent key refuses, a non-`fm_key`
//! target refuses, and the armed fact names the key that died.

use std::collections::BTreeMap;
use std::path::PathBuf;

use wire::{Edit, EditShape, HpathSeg, Path as WPath, PlanEdit, ReceiptAddr, ResponseBody, SecRef};
use wire_serve::write::{SpliceArgs, splice};

const CARD: &str = "---\ntitle: Plan\nhooks: \"\"\nstatus: open\n---\n# Goals\n\n- ship\n";

fn ws(files: &[(&str, &str)]) -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    for (rel, body) in files {
        std::fs::write(dir.path().join(rel), body).expect("seed");
    }
    let root = fs::WorkspaceRoot(PathBuf::from(dir.path()));
    (dir, root)
}

fn args(edits: Vec<Edit>, plan_edits: Vec<PlanEdit>) -> SpliceArgs {
    SpliceArgs {
        premises: Vec::new(),
        id: None,
        origin: wire_serve::guard::Origin::InProcess,
        path: WPath("card.md".into()),
        actor: Some("agent:ba67afc4".into()),
        now: Some("2026-08-26T12:00:00Z".into()),
        receipt: Some(ReceiptAddr {
            path: WPath("receipts.md".into()),
            anchor: "r-000001".into(),
        }),
        if_root: None,
        dry: false,
        force: true,
        edits,
        plan_edits,
        pin: None,
        fields: BTreeMap::default(),
    }
}

fn remove(key: &str) -> Edit {
    Edit {
        target: SecRef::FmKey { fm_key: key.into() },
        edit: EditShape::Remove {},
        if_node_rev: None,
    }
}

fn body_of(outcome: &wire_serve::write::SpliceOutcome) -> &wire::Armed {
    let ResponseBody::Splice { armed, .. } = &outcome.body else {
        panic!("a splice returns a Splice body: {:?}", outcome.body);
    };
    armed
}

/// The core law: the struck region carries the key's line terminator, so the
/// key does not leave a blank line standing in its place. A `§1` leaf span
/// EXCLUDES that byte, so a region equal to the span would leave it — which is
/// exactly what `put{at:"all"}` with empty text does before it refuses.
#[test]
fn remove_strikes_the_key_line_and_its_terminator() {
    let (dir, root) = ws(&[("card.md", CARD)]);
    splice(
        &root,
        None,
        &args(vec![remove("hooks")], Vec::new()),
        &[],
        None,
    )
    .expect("the removal commits");

    let after = std::fs::read_to_string(dir.path().join("card.md")).expect("reads back");
    assert_eq!(
        after, "---\ntitle: Plan\nstatus: open\n---\n# Goals\n\n- ship\n",
        "the key line is gone WHOLE — no blank line left inside the block"
    );
    assert!(
        !after.contains("\n\n---\n"),
        "a blank line inside the frontmatter block is the failure this door exists to avoid:\n{after}"
    );
}

/// The armed fact NAMES THE KEY THAT DIED — the whole reason this shape exists
/// rather than a whole-document rewrite (which arms `target:{"hpath":[]}` and
/// says nothing about any key). A.6.3a′ settled this for the create side; this is
/// the retire side finally carrying the same identity.
#[test]
fn the_armed_fact_names_the_key_and_arms_the_no_node_token() {
    let (dir, root) = ws(&[("card.md", CARD)]);
    let outcome = splice(
        &root,
        None,
        &args(vec![remove("hooks")], Vec::new()),
        &[],
        None,
    )
    .expect("the removal commits");
    let armed = body_of(&outcome);

    assert_eq!(armed.edits.len(), 1);
    assert!(
        matches!(&armed.edits[0].target, SecRef::FmKey { fm_key } if fm_key == "hooks"),
        "the fact names the removed key, not the document: {:?}",
        armed.edits[0].target
    );
    // blake3("")[:16] — A.6.3a′'s no-node token, read in the other direction.
    assert_eq!(
        armed.edits[0].node_rev_after.0, "af1349b9f5f9a1a6",
        "a removal arms the no-node token as its AFTER rev"
    );
    assert_ne!(
        armed.edits[0].node_rev_before.0, armed.edits[0].node_rev_after.0,
        "the transition must be real — a removal that claims nothing moved is the \
         unauditable receipt A.6.3a′ outlawed"
    );
    assert_eq!(
        armed.edits[0].span_after.0, armed.edits[0].span_after.1,
        "the node is gone, so its span is zero-width"
    );

    let receipt = std::fs::read_to_string(dir.path().join("receipts.md")).expect("receipt written");
    assert!(
        receipt.contains(" hooks remove "),
        "the receipt renders the caller's own shape:\n{receipt}"
    );
}

/// **The last key REFUSES, and the measurement is why.** Neither outcome was
/// available. Leaving bare fences: `---\n---\n` is NOT frontmatter to this
/// engine (`syntax::parse` mints a `Frontmatter` node only from a pulldown
/// `MetadataBlock`, which an empty block does not raise), so the next property
/// write synthesizes a SECOND block above them. Carrying the fences: the def
/// plane refuses any blockless document outright — `unreadable frontmatter:
/// <nil>`, "never forceable".
///
/// So the emptying removal could not commit either way, and the only question
/// was where it dies. It dies at the door naming the last key, instead of three
/// layers down in a conformance message about NESTED frontmatter that
/// misdiagnoses the write that drew it.
#[test]
fn removing_the_last_key_refuses_and_names_it_as_the_last() {
    let (dir, root) = ws(&[("card.md", "---\nonly: me\n---\n\n# Goals\n\n- ship\n")]);
    let before = std::fs::read_to_string(dir.path().join("card.md")).expect("seed");
    let err = splice(
        &root,
        None,
        &args(vec![remove("only")], Vec::new()),
        &[],
        None,
    )
    .expect_err("the block's last key refuses");
    assert_eq!(err.code, wire::ErrorCode::BadRequest, "{err:?}");
    let msg = err.message.clone().unwrap_or_default();
    assert!(msg.contains("only"), "the refusal names the key: {msg}");
    assert!(
        msg.contains("only key") && msg.contains("op:\"remove\""),
        "it names WHY (last key) and the executable escape (retire the record): {msg}"
    );
    assert!(
        !msg.contains("nested"),
        "the door's own refusal, not the conformance plane's misdiagnosis: {msg}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("card.md")).expect("reads back"),
        before,
        "a refused batch writes nothing"
    );
}

/// The same removal COMMITS the moment the block holds a second key — so the
/// refusal above is the last-key rule biting, not `remove` being broken for
/// single-key records generally.
#[test]
fn the_same_key_removes_cleanly_once_the_block_holds_another() {
    let (dir, root) = ws(&[(
        "card.md",
        "---\nonly: me\nkeep: yes\n---\n\n# Goals\n\n- ship\n",
    )]);
    splice(
        &root,
        None,
        &args(vec![remove("only")], Vec::new()),
        &[],
        None,
    )
    .expect("with a survivor present the removal commits");
    let after = std::fs::read_to_string(dir.path().join("card.md")).expect("reads back");
    assert_eq!(
        after, "---\nkeep: yes\n---\n\n# Goals\n\n- ship\n",
        "{after}"
    );
}

/// **The uniformity witness** (ruling
/// `blockless-property-set-synthesizes-on-every-lane`, 2026-08-26). A blockless
/// document takes a property SET on BOTH lanes — the plan lane and the native
/// `at:"upsert"` door — and lands one well-formed block either way.
///
/// This test began as the pin on the OPPOSITE fact: the plan lane refused where
/// the native door committed, and I escalated the divergence rather than
/// overturning a ruled rule from an implementation seat. It now witnesses the
/// ruling. If a lane diverges again, this fails and says which one.
#[test]
fn a_blockless_document_takes_a_property_set_on_both_lanes() {
    // Lane 1: the plan lane, which used to refuse here.
    let (dir, root) = ws(&[("card.md", "# Goals\n\n- ship\n")]);
    splice(
        &root,
        None,
        &args(
            Vec::new(),
            vec![PlanEdit::SetProperty {
                key: "status".into(),
                value: "open".into(),
                rev: None,
            }],
        ),
        &[],
        None,
    )
    .expect("the plan lane synthesizes the block");
    let plan_lane = std::fs::read_to_string(dir.path().join("card.md")).expect("reads back");
    assert_eq!(
        plan_lane, "---\nstatus: open\n---\n# Goals\n\n- ship\n",
        "{plan_lane}"
    );

    // Lane 2: the native door, on an identical blockless document.
    let (dir2, root2) = ws(&[("card.md", "# Goals\n\n- ship\n")]);
    splice(
        &root2,
        None,
        &args(
            vec![Edit {
                target: SecRef::FmKey {
                    fm_key: "status".into(),
                },
                edit: EditShape::Put {
                    at: wire::PutAt::Upsert,
                    text: "open".into(),
                },
                if_node_rev: None,
            }],
            Vec::new(),
        ),
        &[],
        None,
    )
    .expect("the native door synthesizes the block");
    let native = std::fs::read_to_string(dir2.path().join("card.md")).expect("reads back");

    assert_eq!(
        plan_lane, native,
        "one plane, one answer — the two lanes must land the SAME bytes, not merely both succeed"
    );
}

/// A SHADOWED duplicate counts as a survivor. § A.6.3a records that `fm_key`
/// addresses the FIRST occurrence, so striking it makes the second reachable —
/// the block still has a key, and the write un-shadows a line rather than
/// emptying a block. A map-length test would get this wrong (the map holds one
/// entry) which is why the survivor scan is byte-level.
#[test]
fn a_shadowed_duplicate_key_keeps_the_block_alive() {
    let (dir, root) = ws(&[("card.md", "---\ndup: first\ndup: second\n---\n\n# G\n\nx\n")]);
    splice(
        &root,
        None,
        &args(vec![remove("dup")], Vec::new()),
        &[],
        None,
    )
    .expect("the removal commits");
    let after = std::fs::read_to_string(dir.path().join("card.md")).expect("reads back");
    assert_eq!(
        after, "---\ndup: second\n---\n\n# G\n\nx\n",
        "the shadow line survives and the block stands:\n{after}"
    );
}

/// The duplicate case that trips the write-past-its-span guard, isolated.
/// Two BYTE-IDENTICAL shadow lines: striking the first leaves an
/// identical-bytes node at the same address, so the guard would read "the rev
/// did not move" and refuse a removal that landed exactly right. The guard asks
/// "did the node you named receive the bytes?" — a removal names a node it means
/// to END, so the question does not apply and the shape is exempt on that
/// PREMISE, never on its spelling.
#[test]
fn removing_one_of_two_identical_shadow_lines_commits() {
    let (dir, root) = ws(&[("card.md", "---\ndup: same\ndup: same\n---\n\n# G\n\nx\n")]);
    splice(
        &root,
        None,
        &args(vec![remove("dup")], Vec::new()),
        &[],
        None,
    )
    .expect("an identical-shadow removal is not a stalled transition");
    let after = std::fs::read_to_string(dir.path().join("card.md")).expect("reads back");
    assert_eq!(
        after, "---\ndup: same\n---\n\n# G\n\nx\n",
        "exactly one line struck:\n{after}"
    );
}

/// A block (multi-line) value goes WHOLE — the grain span covers every indented
/// continuation line, and the removal carries all of it plus the terminator.
#[test]
fn a_block_value_is_removed_whole() {
    let (dir, root) = ws(&[(
        "card.md",
        "---\ntitle: Plan\ntags:\n  - a\n  - b\nstatus: open\n---\n\n# G\n\nx\n",
    )]);
    splice(
        &root,
        None,
        &args(vec![remove("tags")], Vec::new()),
        &[],
        None,
    )
    .expect("the removal commits");
    let after = std::fs::read_to_string(dir.path().join("card.md")).expect("reads back");
    assert_eq!(
        after, "---\ntitle: Plan\nstatus: open\n---\n\n# G\n\nx\n",
        "no orphaned continuation line survives its key:\n{after}"
    );
}

/// **Removal is NOT idempotent, on purpose.** A verb that reports success for a
/// key it never saw cannot be told apart from a verb that worked — so a typo'd
/// key name and a finished job would return the same frame. That is the exact
/// silent-success failure this whole card is about, and success-on-absent would
/// re-import it into the fix.
#[test]
fn removing_an_absent_key_refuses_ref_not_found() {
    let (dir, root) = ws(&[("card.md", CARD)]);
    let before = std::fs::read_to_string(dir.path().join("card.md")).expect("seed");
    let err = splice(
        &root,
        None,
        &args(vec![remove("nosuchkey")], Vec::new()),
        &[],
        None,
    )
    .expect_err("an absent key refuses");
    assert_eq!(err.code, wire::ErrorCode::RefNotFound, "{err:?}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("card.md")).expect("reads back"),
        before,
        "a refused batch writes nothing"
    );
}

/// The shape is fenced to the frontmatter plane. A section and an anchor
/// already retire through the parent's content slot NAMING that parent, so a
/// `remove` there would be a second spelling of a capability that exists — the
/// thing §4.4 forbids. The refusal says which plane the target is, so the
/// caller learns the rule rather than the row.
#[test]
fn remove_on_a_section_target_refuses_and_names_the_plane() {
    let (_dir, root) = ws(&[("card.md", CARD)]);
    let err = splice(
        &root,
        None,
        &args(
            vec![Edit {
                target: SecRef::Hpath {
                    hpath: vec![HpathSeg {
                        h: "Goals".into(),
                        n: None,
                    }],
                },
                edit: EditShape::Remove {},
                if_node_rev: None,
            }],
            Vec::new(),
        ),
        &[],
        None,
    )
    .expect_err("a section target refuses");
    assert_eq!(err.code, wire::ErrorCode::BadRequest, "{err:?}");
    let msg = err.message.clone().unwrap_or_default();
    assert!(msg.contains("fm_key"), "{msg}");
    assert!(
        msg.contains("section") && msg.contains("parent's content slot"),
        "the refusal teaches the reachable retire door for THAT plane: {msg}"
    );
}

/// One key, one intent per batch. The lowered regions would overlap and the
/// kernel WOULD refuse — with the generic overlap remedy, which tells the
/// caller which byte ranges collided rather than which of their two orders the
/// engine could not honour.
#[test]
fn setting_and_removing_one_key_in_a_batch_refuses_by_name() {
    let (_dir, root) = ws(&[("card.md", CARD)]);
    let err = splice(
        &root,
        None,
        &args(
            Vec::new(),
            vec![
                PlanEdit::SetProperty {
                    key: "hooks".into(),
                    value: "v".into(),
                    rev: None,
                },
                PlanEdit::RemoveProperty {
                    key: "hooks".into(),
                    rev: None,
                },
            ],
        ),
        &[],
        None,
    )
    .expect_err("one key carries one intent");
    assert_eq!(err.code, wire::ErrorCode::BadRequest, "{err:?}");
    let msg = err.message.clone().unwrap_or_default();
    assert!(
        msg.contains("hooks") && msg.contains("one intent"),
        "the refusal names the contradiction, not the byte ranges: {msg}"
    );
}

/// The plan lane reaches the same door — the lane the `properties` map, the
/// Starlark `put(props=)` form and every host face enter through. Without this
/// the engine would carry a verb the fleet cannot call.
#[test]
fn the_plan_lane_removes_and_arms_the_same_per_key_fact() {
    let (dir, root) = ws(&[("card.md", CARD)]);
    let outcome = splice(
        &root,
        None,
        &args(
            Vec::new(),
            vec![PlanEdit::RemoveProperty {
                key: "hooks".into(),
                rev: None,
            }],
        ),
        &[],
        None,
    )
    .expect("the plan-lane removal commits");
    let armed = body_of(&outcome);
    assert_eq!(armed.edits.len(), 1);
    assert!(
        matches!(&armed.edits[0].target, SecRef::FmKey { fm_key } if fm_key == "hooks"),
        "the plan lane keeps the per-key identity: {:?}",
        armed.edits[0].target
    );
    let after = std::fs::read_to_string(dir.path().join("card.md")).expect("reads back");
    assert_eq!(
        after,
        "---\ntitle: Plan\nstatus: open\n---\n# Goals\n\n- ship\n"
    );
}

/// **The state transition the write plane could not make.** R4's three states
/// are ratified and the props1 digest has always distinguished them — this
/// asserts a WRITE finally moves a live key from `Scalar` to `Absent`, with the
/// `""` starting state that is the fleet's substitute for it. The `""` leg is
/// the control: it proves the emptied state is `Scalar`, not `Absent`, so the
/// screens that read it as "set" were reading correctly and the defect was
/// never theirs.
#[test]
fn remove_moves_the_key_from_scalar_to_absent() {
    use model::fingerprint::{PropValue, classify_property};

    fn fm_map(raw: &str) -> model::YamlMap {
        fn find(node: &model::Node) -> Option<model::YamlMap> {
            if let model::NodeKind::Frontmatter { map } = &node.kind {
                return Some(map.clone());
            }
            node.children.iter().find_map(find)
        }
        find(&model::build(raw.to_owned(), syntax::parse(raw)).root).expect("has frontmatter")
    }

    let (dir, root) = ws(&[("card.md", CARD)]);
    let before = std::fs::read_to_string(dir.path().join("card.md")).expect("seed");
    assert_eq!(
        classify_property(&fm_map(&before), "hooks"),
        PropValue::Scalar("\"\"".into()),
        "the emptied state the fleet writes is a VALUE — this is why an absence screen \
         reads it as present, and why emptying was never a substitute for removing"
    );

    splice(
        &root,
        None,
        &args(vec![remove("hooks")], Vec::new()),
        &[],
        None,
    )
    .expect("the removal commits");

    let after = std::fs::read_to_string(dir.path().join("card.md")).expect("reads back");
    assert_eq!(
        classify_property(&fm_map(&after), "hooks"),
        PropValue::Absent,
        "the write plane now reaches the third state"
    );
    assert_ne!(
        model::fingerprint::properties_fingerprint(&fm_map(&before), &["hooks".to_owned()]),
        model::fingerprint::properties_fingerprint(&fm_map(&after), &["hooks".to_owned()]),
        "a pin whose properties selector names this key must SEE the removal"
    );
}

/// §3.2's strict wall, at the new shape's own grain. `remove` takes NO fields,
/// and a caller who sent one meant `put` — silently ignoring it would remove a
/// key where a set was intended, which is the guard-you-believe-is-armed trap
/// the wall exists to kill, pointed at content instead of a guard.
#[test]
fn the_decode_wall_refuses_a_field_inside_remove_and_teaches_the_set_door() {
    let v: serde_json::Value =
        serde_json::from_str(r#"[{"target":{"fm_key":"hooks"},"edit":{"remove":{"text":"x"}}}]"#)
            .expect("fixture parses");
    let err = wire_serve::decode::decode_edits(&v, wire_serve::decode::Laws::Full)
        .expect_err("a field inside `remove` refuses");
    let msg = err.message.clone().unwrap_or_default();
    assert!(
        msg.contains("text"),
        "the refusal names the offending field: {msg}"
    );
    assert!(
        msg.contains("upsert"),
        "and points at the door that DOES take a value: {msg}"
    );

    // The shape itself decodes, so the refusal above is the wall biting and not
    // a dead instrument.
    let ok: serde_json::Value =
        serde_json::from_str(r#"[{"target":{"fm_key":"hooks"},"edit":{"remove":{}}}]"#)
            .expect("fixture parses");
    let edits = wire_serve::decode::decode_edits(&ok, wire_serve::decode::Laws::Full)
        .expect("`{}` is the shape");
    assert!(
        matches!(edits[0].edit, EditShape::Remove {}),
        "{:?}",
        edits[0]
    );
}

/// A set and a remove of DIFFERENT keys compose in one batch, and each arms its
/// own fact. The set group runs first, then the retire group; both sorted, so a
/// mixed batch's armed rows are stable.
#[test]
fn a_set_and_a_remove_of_different_keys_compose() {
    let (dir, root) = ws(&[("card.md", CARD)]);
    let outcome = splice(
        &root,
        None,
        &args(
            Vec::new(),
            vec![
                PlanEdit::SetProperty {
                    key: "status".into(),
                    value: "done".into(),
                    rev: None,
                },
                PlanEdit::RemoveProperty {
                    key: "hooks".into(),
                    rev: None,
                },
            ],
        ),
        &[],
        None,
    )
    .expect("the mixed batch commits");
    let armed = body_of(&outcome);
    assert_eq!(armed.edits.len(), 2, "two intents, two facts");
    let after = std::fs::read_to_string(dir.path().join("card.md")).expect("reads back");
    assert_eq!(
        after,
        "---\ntitle: Plan\nstatus: done\n---\n# Goals\n\n- ship\n"
    );
}
