//! Teaching-family gates: r3 gap 6a (heading-path miss parity) and the r5
//! script-birth survivor (cards `refusal-teaching-gaps-r3`,
//! `sweep-survivors-r5`).
//!
//! - Gap 6a: a block-anchor miss offers nearest live ids; a heading-path miss
//!   offered only the generic toc remedy and never said *root-anchored* — the
//!   actual defect in the measured probe (`at: "Fence"`, a real heading, not
//!   root-anchored). Parity: the heading lane teaches from the same toc the
//!   engine already projected.
//! - r5 F2: the birth refusal taught `wire create` / `mrd new` only — doors a
//!   face caller cannot reach. Register law: fitted suggestions by entry.
//!
//! Pins properties, not bytes. Codes stay frozen.

use std::collections::BTreeMap;
use wire::{Edit, EditShape, ErrorCode, HpathSeg, Path as WPath, SecRef};
use wire_serve::write::{SpliceArgs, splice};

/// A page whose `Fence` heading is real but nested — addressing it bare
/// (not root-anchored) is the measured gap-6a probe.
const NESTED: &str = "\
---
title: T
---
# Top

intro

## Fence

body
";

fn hpath(segs: &[&str]) -> SecRef {
    SecRef::Hpath {
        hpath: segs
            .iter()
            .map(|h| HpathSeg {
                h: (*h).to_owned(),
                n: None,
            })
            .collect(),
    }
}

fn args_for(path: String, target: SecRef) -> SpliceArgs {
    SpliceArgs {
        premises: Vec::new(),
        id: None,
        origin: wire_serve::guard::Origin::InProcess,
        path: WPath(path),
        actor: None,
        now: None,
        receipt: None,
        if_root: None,
        dry: false,
        force: false,
        edits: vec![Edit {
            target,
            edit: EditShape::Match {
                old: "body".into(),
                new: "grown".into(),
            },
            if_node_rev: None,
        }],
        plan_edits: Vec::new(),
        pin: None,
        fields: BTreeMap::default(),
    }
}

fn miss_message(target: SecRef) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("f.md"), NESTED).expect("fixture");
    let root = fs::WorkspaceRoot(dir.path().canonicalize().expect("canonicalize"));
    let err = splice(&root, None, &args_for("f.md".into(), target), &[], None)
        .expect_err("the selector misses");
    assert_eq!(err.code, ErrorCode::RefNotFound);
    err.message
        .as_deref()
        .expect("the refusal is a sentence, not a bare code")
        .to_owned()
}

/// Gap 6a shape 1 — the asked heading EXISTS deeper in the tree. The refusal
/// names the root-anchoring rule and the live full path.
#[test]
fn hpath_miss_on_a_nested_heading_names_root_anchoring_and_the_full_path() {
    let m = miss_message(hpath(&["Fence"]));
    assert!(
        m.contains("root-anchored"),
        "names the root-anchoring rule: {m}"
    );
    assert!(m.contains("Top/Fence"), "offers the live full path: {m}");
}

/// Gap 6a shape 2 — a typo'd heading gets a nearest-match offer, the parity
/// the block-anchor lane already has.
#[test]
fn hpath_miss_with_a_typo_offers_nearest_live_heading_paths() {
    let m = miss_message(hpath(&["Fnce"]));
    assert!(
        m.contains("nearest live heading paths"),
        "offers nearest matches: {m}"
    );
    assert!(
        m.contains("Top/Fence"),
        "the near heading rides as its full path: {m}"
    );
}

/// r5 F2 — the birth refusal fits every entry: the serving face's put door
/// (ref + full body) joins `wire create` / `mrd new`, phrased by
/// applicability, reason first.
#[test]
fn birth_refusal_offers_the_face_write_door_and_fits_each_entry() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = fs::WorkspaceRoot(dir.path().canonicalize().expect("canonicalize"));
    let err = wire_serve::load_doc(&root, &WPath("missing.md".into()))
        .map(|_| ())
        .expect_err("a missing file refuses");
    assert_eq!(err.code, ErrorCode::FileNotFound);
    let m = err
        .message
        .as_deref()
        .expect("the refusal is a sentence, not a bare code");
    // Reason stays: a write to a missing path never births it.
    assert!(m.contains("never births"), "keeps the reason: {m}");
    // Fitted suggestions, one per entry the caller may actually hold.
    assert!(
        m.contains("whichever fits"),
        "suggestions ride by applicability, not as one demand: {m}"
    );
    assert!(
        m.contains("put") && m.contains("body"),
        "names the face's own birth door (put: ref + full body): {m}"
    );
    assert!(m.contains("`create`"), "keeps the wire door: {m}");
    assert!(m.contains("mrd new"), "keeps the CLI door: {m}");
}
