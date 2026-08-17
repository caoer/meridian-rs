//! Armed facts for a `set_property` batch: ONE fact per key, each naming the
//! key it wrote (§6.1 op + target identities + rev transitions; §6.4 the facts
//! ARE the receipt's normative content).
//!
//! The measured defect (dogfood s11-40 / s11-50, engine v1.0.0): a script batch
//! arming `set_property owner` + `set_property status` + an append reported
//! `edits=2` and receipted `title put:end <rev>-><same rev>` — the two property
//! writes collapsed onto the last EXISTING key, so the receipt named an
//! identity the batch never wrote, an op nobody asked for, and a transition
//! claiming nothing moved while two keys landed. A §11 lint asserting receipts
//! against intents would false-negative on every props write.

use std::path::PathBuf;

use std::collections::BTreeMap;
use wire::{HpathSeg, Path as WPath, PlanEdit, ReceiptAddr, ResponseBody, SecRef};
use wire_serve::write::{SpliceArgs, splice};

/// The §0.3 fixture shape: frontmatter carrying ONE key, `title` — the key the
/// old lowering handed every create to.
const CARD: &str = "---\ntitle: Plan\n---\n# Goals\n\n## Q4\n\n- ship\n";

fn ws(files: &[(&str, &str)]) -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    for (rel, body) in files {
        std::fs::write(dir.path().join(rel), body).expect("seed");
    }
    let root = fs::WorkspaceRoot(PathBuf::from(dir.path()));
    (dir, root)
}

fn args(plan_edits: Vec<PlanEdit>) -> SpliceArgs {
    SpliceArgs {
        premises: Vec::new(),
        id: None,
        origin: wire_serve::guard::Origin::InProcess,
        path: WPath("card.md".into()),
        actor: Some("agent:b0864fb2".into()),
        now: Some("2026-08-09T12:00:00Z".into()),
        receipt: Some(ReceiptAddr {
            path: WPath("receipts.md".into()),
            anchor: "r-000001".into(),
        }),
        if_root: None,
        dry: false,
        force: false,
        edits: Vec::new(),
        plan_edits,
        pin: None,
        fields: BTreeMap::default(),
    }
}

fn set(key: &str, value: &str) -> PlanEdit {
    PlanEdit::SetProperty {
        key: key.into(),
        value: value.into(),
        rev: None,
    }
}

fn key_of(target: &SecRef) -> &str {
    match target {
        SecRef::FmKey { fm_key } => fm_key,
        other => panic!("an fm write names an fm_key target, got {other:?}"),
    }
}

/// The s11 batch, replayed: two property creates beside a body append.
/// Three intents → three armed facts, each naming its own key, each with a
/// transition that moved — and the receipt line reads them back.
#[test]
fn a_props_batch_arms_one_fact_per_key_and_the_receipt_reads_them_back() {
    let (dir, root) = ws(&[("card.md", CARD)]);
    let outcome = splice(
        &root,
        None,
        &args(vec![
            set("owner", "agent:b0864fb2"),
            set("status", "open"),
            PlanEdit::Append {
                hpath: vec![
                    HpathSeg {
                        h: "Goals".into(),
                        n: None,
                    },
                    HpathSeg {
                        h: "Q4".into(),
                        n: None,
                    },
                ],
                body: "- answers".into(),
                rev: None,
            },
        ]),
        &[],
        None,
    )
    .expect("the batch commits");

    let ResponseBody::Splice { armed, .. } = &outcome.body else {
        panic!("a splice returns a Splice body: {:?}", outcome.body);
    };

    assert_eq!(
        armed.edits.len(),
        3,
        "three intents, three armed facts — a props write is never folded onto another key"
    );
    assert_eq!(key_of(&armed.edits[0].target), "owner");
    assert_eq!(key_of(&armed.edits[1].target), "status");
    assert!(
        matches!(&armed.edits[2].target, SecRef::Hpath { hpath }
            if hpath.iter().map(|s| s.h.as_str()).eq(["Goals", "Q4"])),
        "the body op keeps its own §2.1 address"
    );
    for fact in &armed.edits {
        assert_ne!(
            fact.node_rev_before, fact.node_rev_after,
            "a fact whose transition is a no-op claims nothing changed where bytes landed: {fact:?}"
        );
    }

    // The key nobody named is not in the facts, and its own bytes are untouched.
    assert!(
        armed
            .edits
            .iter()
            .all(|f| !matches!(&f.target, SecRef::FmKey { fm_key } if fm_key == "title")),
        "`title` was never written — it must not appear in the armed set"
    );
    let after = std::fs::read_to_string(dir.path().join("card.md")).expect("reads back");
    assert!(after.contains("title: Plan"), "card.md after:\n{after}");
    assert!(after.contains("owner: agent:b0864fb2"), "{after}");
    assert!(after.contains("status: open"), "{after}");

    // The receipt is the durable face of the same facts (§6.4).
    let receipt = std::fs::read_to_string(dir.path().join("receipts.md")).expect("receipt written");
    assert!(receipt.contains(" edits=3 "), "receipt line:\n{receipt}");
    assert!(receipt.contains(" owner put:upsert "), "{receipt}");
    assert!(receipt.contains(" status put:upsert "), "{receipt}");
    assert!(
        !receipt.contains(" title "),
        "the receipt must not name a key the batch never wrote:\n{receipt}"
    );
}

/// An UPDATE of an existing key keeps its own identity too — the arm the old
/// lowering already got right, frozen so the fix cannot regress it.
#[test]
fn an_existing_key_update_names_itself() {
    let (dir, root) = ws(&[(
        "card.md",
        "---\ntitle: Plan\nstatus: open\n---\n# Goals\n\nx\n",
    )]);
    let outcome = splice(&root, None, &args(vec![set("status", "done")]), &[], None)
        .expect("the update commits");
    let ResponseBody::Splice { armed, .. } = &outcome.body else {
        panic!("a splice returns a Splice body");
    };
    assert_eq!(armed.edits.len(), 1);
    assert_eq!(key_of(&armed.edits[0].target), "status");
    assert_ne!(
        armed.edits[0].node_rev_before,
        armed.edits[0].node_rev_after
    );
    let receipt = std::fs::read_to_string(dir.path().join("receipts.md")).expect("receipt written");
    assert!(receipt.contains(" status put:all "), "{receipt}");
}
