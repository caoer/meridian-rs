//! Armed facts for a `create` plan-edit: the row names the BORN section, not
//! the parent the lowering appends under (§6.1 op + target identities + rev
//! transitions; §6.4 the facts ARE the receipt's normative content; A.6.3a′
//! precedent — the props plane received this same fix).
//!
//! The measured defect (F2 phase B, engine 7a22e00a): a `create` performs the
//! birth correctly, but the armed row inherits the LOWERING's target — the
//! parent — so three of the four fields describe the parent and
//! `node_rev_before` claims a before-state for a node that had none. No
//! consumer can learn the born section's identity or rev from the answer; the
//! daemon kept a whole fallback read alive purely to recover the born rev.
//!
//! The born row's `node_rev_before` is the empty-input hash (blake3("")[:16] =
//! `af1349b9f5f9a1a6`), the A.6.3a′ teaching-row idiom: not a claim that an
//! empty section existed — the op (the caller's own `create` row, §4.4 1:1
//! alignment) says birth; the token says born-from-nothing.

use std::path::PathBuf;

use wire::{HpathSeg, Path as WPath, PlanEdit, ReceiptAddr, ResponseBody, SecRef, Span};
use wire_serve::write::{SpliceArgs, splice};

/// Parent chain two segments deep, a sibling section after it, one fm key.
const CARD: &str =
    "---\ntitle: Plan\n---\n# Memo\n\nintro\n\n## Notes\n\n- a line\n\n# Archive\n\nold\n";

fn ws(files: &[(&str, &str)]) -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    for (rel, body) in files {
        std::fs::write(dir.path().join(rel), body).expect("seed");
    }
    let root = fs::WorkspaceRoot(PathBuf::from(dir.path()));
    (dir, root)
}

fn args(plan_edits: Vec<PlanEdit>, receipt: bool) -> SpliceArgs {
    SpliceArgs {
        id: None,
        origin: wire_serve::guard::Origin::InProcess,
        path: WPath("card.md".into()),
        actor: Some("agent:b1f01068".into()),
        now: Some("2026-08-12T12:00:00Z".into()),
        receipt: receipt.then(|| ReceiptAddr {
            path: WPath("receipts.md".into()),
            anchor: "r-000001".into(),
        }),
        if_root: None,
        dry: false,
        force: false,
        edits: Vec::new(),
        plan_edits,
        pin: None,
    }
}

fn seg(h: &str) -> HpathSeg {
    HpathSeg {
        h: h.into(),
        n: None,
    }
}

fn parent() -> Vec<HpathSeg> {
    vec![seg("Memo"), seg("Notes")]
}

fn create(title: &str, body: &str) -> PlanEdit {
    PlanEdit::Create {
        parent_hpath: parent(),
        title: title.into(),
        body: body.into(),
        rev: None,
    }
}

/// The armed target as (heading, occurrence) pairs.
fn hseq(target: &SecRef) -> Vec<(String, Option<u32>)> {
    match target {
        SecRef::Hpath { hpath } => hpath.iter().map(|s| (s.h.clone(), s.n)).collect(),
        other => panic!("a section fact names an hpath target, got {other:?}"),
    }
}

fn owned(chain: &[(&str, Option<u32>)]) -> Vec<(String, Option<u32>)> {
    chain.iter().map(|(h, n)| ((*h).to_string(), *n)).collect()
}

/// blake3("")[:16] through the engine's own A.6.3a′ door (`fm_upsert_before`
/// on an absent key), never retyped.
fn empty_rev(root: &fs::WorkspaceRoot) -> wire::NodeRev {
    let doc = fs::load(root, std::path::Path::new("card.md")).expect("load");
    wire::NodeRev(model::fm_upsert_before(&doc, "no-such-key").node_rev.0)
}

/// Resolve a section in the on-disk state: (`node_rev`, span).
fn read_back(
    root: &fs::WorkspaceRoot,
    chain: &[(&str, Option<u32>)],
) -> (wire::NodeRev, Span, String) {
    let doc = fs::load(root, std::path::Path::new("card.md")).expect("load");
    let segs = chain
        .iter()
        .map(|(h, n)| model::HpathSeg {
            h: (*h).to_string(),
            n: *n,
        })
        .collect();
    let t = model::resolve(&doc, &model::Ref::Hpath(segs)).expect("resolves after the write");
    let raw = doc.raw[t.span.clone()].to_string();
    (
        wire::NodeRev(t.node_rev.0),
        Span(t.span.start as u64, t.span.end as u64),
        raw,
    )
}

/// The defect, replayed: one create → one armed fact, naming the section that
/// was born, with a born-from-nothing before-token and the born node's own
/// after facts. Dry and real agree; the receipt reads the same facts back as
/// op `create`.
#[test]
fn a_create_arms_the_born_section_not_the_parent() {
    let (dir, root) = ws(&[("card.md", CARD)]);

    // Dry rehearsal first: the armed row is the same fact the real run arms.
    let dry = splice(
        &root,
        None,
        &SpliceArgs {
            dry: true,
            ..args(vec![create("Fresh", "born body")], true)
        },
        &[],
        None,
    )
    .expect("the rehearsal answers");
    let ResponseBody::Splice {
        armed: dry_armed, ..
    } = &dry.body
    else {
        panic!("a splice returns a Splice body");
    };
    assert_eq!(dry_armed.edits.len(), 1);
    assert_eq!(
        hseq(&dry_armed.edits[0].target),
        owned(&[("Memo", None), ("Notes", None), ("Fresh", None)]),
        "the dry row names the born section"
    );

    let outcome = splice(
        &root,
        None,
        &args(vec![create("Fresh", "born body")], true),
        &[],
        None,
    )
    .expect("the birth commits");
    let ResponseBody::Splice { armed, .. } = &outcome.body else {
        panic!("a splice returns a Splice body");
    };

    assert_eq!(armed.edits.len(), 1, "one intent, one armed fact");
    let fact = &armed.edits[0];
    assert_eq!(
        hseq(&fact.target),
        owned(&[("Memo", None), ("Notes", None), ("Fresh", None)]),
        "the armed fact names the section the caller addressed — the born one, not its parent"
    );

    let empty = empty_rev(&root);
    assert_eq!(
        fact.node_rev_before, empty,
        "a born section has no before-state: the before-token is blake3(\"\")[:16] \
         (af1349b9f5f9a1a6), the A.6.3a′ idiom — got {:?}",
        fact.node_rev_before
    );

    let (born_rev, born_span, born_raw) =
        read_back(&root, &[("Memo", None), ("Notes", None), ("Fresh", None)]);
    assert_eq!(
        fact.node_rev_after, born_rev,
        "node_rev_after is the born node's own rev"
    );
    assert_eq!(
        fact.span_after, born_span,
        "span_after is the born node's own span"
    );
    assert!(born_raw.contains("born body"), "{born_raw}");
    assert_eq!(fact.node_rev_after, dry_armed.edits[0].node_rev_after);

    let after = std::fs::read_to_string(dir.path().join("card.md")).expect("reads back");
    assert!(after.contains("### Fresh"), "level = parent + 1:\n{after}");

    // The receipt is the durable face of the same facts (§6.4): the born
    // identity, op `create`, the born transition.
    let receipt = std::fs::read_to_string(dir.path().join("receipts.md")).expect("receipt written");
    assert!(receipt.contains(" edits=1 "), "{receipt}");
    assert!(
        receipt.contains(
            "target.hpath=[{\"h\":\"Memo\"},{\"h\":\"Notes\"},{\"h\":\"Fresh\"}] create af1349b9f5f9a1a6->"
        ),
        "the receipt names the born section with op `create` and the \
         born-from-nothing before-token:\n{receipt}"
    );
    assert!(
        !receipt.contains("put:end"),
        "no row claims an append the caller never asked for:\n{receipt}"
    );
}

/// A birth beside an existing same-title sibling carries its occurrence: the
/// published address is the read face's own (`n` exactly where ambiguous), so
/// read → verbatim address → read lands on the born node.
#[test]
fn a_duplicate_title_birth_carries_its_occurrence() {
    let seeded = "---\ntitle: Plan\n---\n# Memo\n\nintro\n\n## Notes\n\n- a line\n\n\
                  ### Fresh\n\nthe first one\n\n# Archive\n\nold\n";
    let (_dir, root) = ws(&[("card.md", seeded)]);

    let outcome = splice(
        &root,
        None,
        &args(vec![create("Fresh", "the second one")], false),
        &[],
        None,
    )
    .expect("the birth commits");
    let ResponseBody::Splice { armed, .. } = &outcome.body else {
        panic!("a splice returns a Splice body");
    };

    assert_eq!(armed.edits.len(), 1);
    let fact = &armed.edits[0];
    assert_eq!(
        hseq(&fact.target),
        owned(&[("Memo", None), ("Notes", None), ("Fresh", Some(2))]),
        "the born duplicate is occurrence 2, and the address says so"
    );
    let (born_rev, born_span, born_raw) = read_back(
        &root,
        &[("Memo", None), ("Notes", None), ("Fresh", Some(2))],
    );
    assert_eq!(fact.node_rev_after, born_rev);
    assert_eq!(fact.span_after, born_span);
    assert!(
        born_raw.contains("the second one") && !born_raw.contains("the first one"),
        "occurrence 2 is the born node, not the survivor: {born_raw}"
    );
}

/// Two births of one title in one batch take their occurrences in request
/// order — same-point inserts land in request order (§4.4), and each row's
/// after facts are its own node's.
#[test]
fn two_births_of_one_title_in_one_batch_take_request_order() {
    let (_dir, root) = ws(&[("card.md", CARD)]);

    let outcome = splice(
        &root,
        None,
        &args(
            vec![
                create("Fresh", "first born"),
                create("Fresh", "second born"),
            ],
            false,
        ),
        &[],
        None,
    )
    .expect("both births commit");
    let ResponseBody::Splice { armed, .. } = &outcome.body else {
        panic!("a splice returns a Splice body");
    };

    assert_eq!(armed.edits.len(), 2, "two intents, two armed facts");
    assert_eq!(
        hseq(&armed.edits[0].target),
        owned(&[("Memo", None), ("Notes", None), ("Fresh", Some(1))]),
    );
    assert_eq!(
        hseq(&armed.edits[1].target),
        owned(&[("Memo", None), ("Notes", None), ("Fresh", Some(2))]),
    );
    assert_ne!(
        armed.edits[0].node_rev_after, armed.edits[1].node_rev_after,
        "different bodies, different born revs"
    );
    let (rev1, _, raw1) = read_back(
        &root,
        &[("Memo", None), ("Notes", None), ("Fresh", Some(1))],
    );
    let (rev2, _, raw2) = read_back(
        &root,
        &[("Memo", None), ("Notes", None), ("Fresh", Some(2))],
    );
    assert_eq!(armed.edits[0].node_rev_after, rev1);
    assert_eq!(armed.edits[1].node_rev_after, rev2);
    assert!(raw1.contains("first born"), "{raw1}");
    assert!(raw2.contains("second born"), "{raw2}");
}

/// The composition that kills occurrence arithmetic: an earlier append in the
/// same batch smuggles a same-title heading under the same parent. The born
/// row must name the node the engine PLACED — occurrence 2 — never a count
/// derived from the pre-batch tree (which says 1).
#[test]
fn a_same_batch_smuggled_heading_does_not_misattribute_the_birth() {
    let (_dir, root) = ws(&[("card.md", CARD)]);

    let outcome = splice(
        &root,
        None,
        &args(
            vec![
                PlanEdit::Append {
                    hpath: parent(),
                    body: "### Fresh\n\nsmuggled".into(),
                    rev: None,
                },
                create("Fresh", "born late"),
            ],
            false,
        ),
        &[],
        None,
    )
    .expect("the batch commits");
    let ResponseBody::Splice { armed, .. } = &outcome.body else {
        panic!("a splice returns a Splice body");
    };

    assert_eq!(armed.edits.len(), 2);
    assert_eq!(
        hseq(&armed.edits[0].target),
        owned(&[("Memo", None), ("Notes", None)]),
        "the append addressed the parent; its fact keeps saying so"
    );
    let fact = &armed.edits[1];
    assert_eq!(
        hseq(&fact.target),
        owned(&[("Memo", None), ("Notes", None), ("Fresh", Some(2))]),
        "the smuggled heading landed first, so the BORN node is occurrence 2 — \
         a before-count would misattribute the birth to occurrence 1"
    );
    let (born_rev, _, born_raw) = read_back(
        &root,
        &[("Memo", None), ("Notes", None), ("Fresh", Some(2))],
    );
    assert_eq!(fact.node_rev_after, born_rev);
    assert!(
        born_raw.contains("born late") && !born_raw.contains("smuggled"),
        "the row's node is the born one: {born_raw}"
    );
}

/// The refusal door, pinned (review P13): an earlier append in the same batch
/// opens a code fence at the parent's end, so the reparse swallows the born
/// heading as fence content — no section stands at the placed position, the
/// birth's armed facts are unrepresentable, and the batch refuses whole
/// (`would_corrupt{target_identity}`), bytes unmoved.
///
/// The parent is the LAST section on purpose: a following pre-batch section
/// would be swallowed too and `would_corrupt{containment_lost}` would answer
/// first — this pin is about the birth's own door.
#[test]
fn a_fence_swallowed_birth_refuses_would_corrupt_target_identity() {
    let seeded = "---\ntitle: Plan\n---\n# Memo\n\nintro\n\n## Notes\n\n- a line\n";
    let (dir, root) = ws(&[("card.md", seeded)]);
    let before = std::fs::read(dir.path().join("card.md")).expect("seed");

    let err = splice(
        &root,
        None,
        &args(
            vec![
                PlanEdit::Append {
                    hpath: parent(),
                    body: "```text".into(),
                    rev: None,
                },
                create("Fresh", "born body"),
            ],
            false,
        ),
        &[],
        None,
    )
    .expect_err("a swallowed birth refuses");

    assert_eq!(err.code, wire::ErrorCode::WouldCorrupt);
    assert_eq!(
        err.family,
        Some(wire::WouldCorruptFamily::TargetIdentity),
        "the birth door's own family: {err:?}"
    );
    let msg = err.message.as_deref().unwrap_or_default();
    assert!(
        msg.contains("unrepresentable") && msg.contains("Memo/Notes/Fresh"),
        "the teaching names the address the caller asked to bear: {msg}"
    );

    let after = std::fs::read(dir.path().join("card.md")).expect("read back");
    assert_eq!(before, after, "the batch refused whole — no byte moved");
}

/// The native door is not this fix's: a native `put:end` whose text happens to
/// open a heading addressed the PARENT, and its fact keeps naming the parent
/// with the parent's own transition.
#[test]
fn a_native_end_append_still_arms_the_parent() {
    let (_dir, root) = ws(&[("card.md", CARD)]);

    let doc = fs::load(&root, std::path::Path::new("card.md")).expect("load");
    let parent_rev_before = model::resolve(
        &doc,
        &model::Ref::Hpath(vec![
            model::HpathSeg {
                h: "Memo".into(),
                n: None,
            },
            model::HpathSeg {
                h: "Notes".into(),
                n: None,
            },
        ]),
    )
    .expect("parent resolves")
    .node_rev;

    let outcome = splice(
        &root,
        None,
        &SpliceArgs {
            edits: vec![wire::Edit {
                target: SecRef::Hpath { hpath: parent() },
                edit: wire::EditShape::Put {
                    at: wire::PutAt::End,
                    text: "\n### Log\n\nnative\n".into(),
                },
                if_node_rev: None,
            }],
            ..args(Vec::new(), false)
        },
        &[],
        None,
    )
    .expect("the native append commits");
    let ResponseBody::Splice { armed, .. } = &outcome.body else {
        panic!("a splice returns a Splice body");
    };

    assert_eq!(armed.edits.len(), 1);
    let fact = &armed.edits[0];
    assert_eq!(
        hseq(&fact.target),
        owned(&[("Memo", None), ("Notes", None)]),
        "the native caller addressed the parent; the fact names the parent"
    );
    assert_eq!(
        fact.node_rev_before,
        wire::NodeRev(parent_rev_before.0),
        "the parent's own before rev — never the born-from-nothing token"
    );
    let (parent_rev_after, parent_span, _) = read_back(&root, &[("Memo", None), ("Notes", None)]);
    assert_eq!(fact.node_rev_after, parent_rev_after);
    assert_eq!(fact.span_after, parent_span);
}
