//! U5.4 substrate debts — merge-gate acceptance suite.
//!
//! Gate 1: heading-rename round-trip preserves `^block-id`s, reddens hpath pins,
//! keeps body-block pins green; backlink fixup rewrites heading links only.
//! Gate 2: N-edit close batch lands atomically (one rev mint); one invalid edit
//! refuses the whole batch with no partial write.

use std::collections::BTreeMap;
use std::path::Path as FsPath;

use model::selector::{Color, RedReason, Selector, classify_edge};
use model::{
    CorpusIndex, Document, HpathSeg as MHpathSeg, NodeRev, Ref, SpliceRequest, SpliceVerdict,
};
use query::{RenamePlan, plan_rename};
use wire::{Edit, EditShape, Path as WPath, PutAt, ReceiptAddr, ResponseBody, SecRef};
use wire_serve::write::{SpliceArgs, splice};

/// Temp workspace from `(path, content)` files.
fn ws(files: &[(&str, &str)]) -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    for (path, content) in files {
        let full = dir.path().join(path);
        if let Some(p) = full.parent() {
            std::fs::create_dir_all(p).expect("mkdir");
        }
        std::fs::write(&full, content).expect("write fixture");
    }
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

fn read(root: &fs::WorkspaceRoot, rel: &str) -> String {
    std::fs::read_to_string(root.0.join(rel)).expect("read")
}

fn build_doc(raw: &str) -> Document {
    model::build(raw.to_string(), syntax::parse(raw))
}

/// Borrowed corpus for `plan_rename`.
fn corpus(root: &fs::WorkspaceRoot, files: &[&str]) -> (CorpusIndex, BTreeMap<String, Document>) {
    let mut index = CorpusIndex::new();
    let mut docs = BTreeMap::new();
    for path in files {
        let doc = build_doc(&read(root, path));
        index.insert(path, &doc);
        docs.insert((*path).to_string(), doc);
    }
    (index, docs)
}

fn hpath(segs: &[&str]) -> Ref {
    Ref::Hpath(
        segs.iter()
            .map(|h| MHpathSeg {
                h: (*h).to_string(),
                n: None,
            })
            .collect(),
    )
}

fn node_rev(doc: &Document, r#ref: &Ref) -> NodeRev {
    model::resolve(doc, r#ref).expect("resolves").node_rev
}

/// Apply [`RenamePlan`] via strict writer (`validate_batch` + `fs::apply_batch`); refusal panics.
fn apply_plan(root: &fs::WorkspaceRoot, plan: &RenamePlan) {
    for (path, batch) in &plan.edits {
        let doc = build_doc(&read(root, path));
        match model::validate_batch(&doc, None, batch, None) {
            SpliceVerdict::Validated(sealed) => {
                fs::apply_batch(
                    root,
                    FsPath::new(path),
                    None,
                    &sealed,
                    doc.raw.as_bytes(),
                    &model::candidate_of_batch(path, &doc.raw, &sealed),
                )
                .expect("apply_batch lands the rename splice");
            }
            other => panic!("rename splice for {path} did not validate: {other:?}"),
        }
    }
}

/// Gate 1: rename preserves block ids, reddens hpath pin, keeps body-block pin green; rename-back greens again.
#[test]
fn gate1_heading_rename_preserves_block_ids_and_reddens_hpath_pins() {
    let original = "# Foo\n\nalpha body ^abc\n\nbeta body\n";
    let (_d, root) = ws(&[("page.md", original)]);

    let before = build_doc(original);
    let pinned_hpath = node_rev(&before, &hpath(&["Foo"]));
    let pinned_block = node_rev(&before, &Ref::anchor("abc").unwrap());

    let (index, docs) = corpus(&root, &["page.md"]);
    let plan = plan_rename(&index, &docs, "page.md", &hpath(&["Foo"]), "Bar");
    apply_plan(&root, &plan);

    let after_raw = read(&root, "page.md");
    let after = build_doc(&after_raw);

    assert!(
        after_raw.contains("^abc"),
        "the block id survives: {after_raw:?}"
    );
    assert!(
        model::resolve(&after, &Ref::anchor("abc").unwrap()).is_ok(),
        "the ^abc anchor still resolves after the rename"
    );
    assert!(after_raw.contains("# Bar"), "heading renamed to Bar");
    assert!(!after_raw.contains("# Foo"), "the old heading text is gone");

    let hpath_color = classify_edge(
        &Selector::Heading(vec!["Foo".to_string()]),
        Some(&pinned_hpath),
        Some(&after),
    );
    assert!(
        matches!(
            hpath_color,
            Color::Red(RedReason::SelectorUnresolved { .. })
        ),
        "an hpath pin to the renamed heading must render red(selector-unresolved): {hpath_color:?}"
    );

    let block_color = classify_edge(
        &Selector::Block("abc".to_string()),
        Some(&pinned_block),
        Some(&after),
    );
    assert_eq!(
        block_color,
        Color::Green,
        "a body-^block-id pin is unaffected by a heading rename (stays green)"
    );

    // Rename back: heading-inclusive rev → exact restore greens hpath pin.
    let (index2, docs2) = corpus(&root, &["page.md"]);
    let back = plan_rename(&index2, &docs2, "page.md", &hpath(&["Bar"]), "Foo");
    apply_plan(&root, &back);
    let restored_raw = read(&root, "page.md");
    assert_eq!(restored_raw, original, "renaming back is byte-identical");
    let restored = build_doc(&restored_raw);
    assert_eq!(
        classify_edge(
            &Selector::Heading(vec!["Foo".to_string()]),
            Some(&pinned_hpath),
            Some(&restored),
        ),
        Color::Green,
        "the restored heading greens the original hpath pin"
    );
    assert!(
        restored_raw.contains("^abc"),
        "block id survives the round-trip"
    );
}

/// Falsification: body-dropping put loses `^abc` — gate 1's block-id assert is load-bearing.
#[test]
fn gate1_falsification_body_dropping_rename_loses_block_id() {
    let original = "# Foo\n\nalpha body ^abc\n\nbeta body\n";
    let (_d, root) = ws(&[("page.md", original)]);

    // Broken rename shape plan_rename does not emit (put at:all → heading only).
    let doc = build_doc(original);
    let bad = SpliceRequest {
        engine: None,
        if_root: None,
        edits: vec![model::Edit {
            target: hpath(&["Foo"]),
            edit: model::EditKind::Put {
                at: model::PutAt::All,
                text: "# Bar\n".to_string(),
            },
            if_node_rev: None,
        }],
    };
    let SpliceVerdict::Validated(sealed) = model::validate_batch(&doc, None, &bad, None) else {
        panic!("the body-dropping put validates (it is a legal splice, just wrong)");
    };
    fs::apply_batch(
        &root,
        FsPath::new("page.md"),
        None,
        &sealed,
        doc.raw.as_bytes(),
        &model::candidate_of_batch("page.md", &doc.raw, &sealed),
    )
    .expect("apply");

    let after_raw = read(&root, "page.md");
    assert!(
        !after_raw.contains("^abc"),
        "the body-dropping rename drops the block id — the gate-1 assertion is load-bearing"
    );
}

/// Backlink fixup: heading links rewritten (alias preserved); `^block-id` links untouched.
#[test]
fn gate1_backlink_fixup_rewrites_heading_links_not_block_links() {
    let page = "# Foo\n\nbody ^abc\n";
    let other = "# Links\n\nsee [[page#Foo]]\n\n## Sub\n\nalso [[page#Foo|note]]\n\n## Block\n\npinned [[page#^abc]]\n";
    let (_d, root) = ws(&[("page.md", page), ("other.md", other)]);

    let (index, docs) = corpus(&root, &["page.md", "other.md"]);
    let plan = plan_rename(&index, &docs, "page.md", &hpath(&["Foo"]), "Bar");
    apply_plan(&root, &plan);

    let other_after = read(&root, "other.md");
    assert!(
        other_after.contains("[[page#Bar]]"),
        "bare heading link rewritten: {other_after:?}"
    );
    assert!(
        other_after.contains("[[page#Bar|note]]"),
        "aliased heading link rewritten, alias preserved"
    );
    assert!(
        !other_after.contains("#Foo"),
        "no stale heading link survives"
    );
    assert!(
        other_after.contains("[[page#^abc]]"),
        "the ^block-id link is left untouched — the anchor survives the rename"
    );

    let page_after = read(&root, "page.md");
    assert!(page_after.contains("# Bar") && page_after.contains("^abc"));
}

const CARD: &str = "---\ntitle: draft\nstatus: open\nowner: alice\n---\n\n# Task\n\n- [ ] item one ^t1\n\nclosing note ^done\n";

/// Canonical close batch: fm upsert + two body matches in one splice.
fn close_edits() -> Vec<Edit> {
    vec![
        Edit {
            target: SecRef::FmKey {
                fm_key: "status".to_string(),
            },
            edit: EditShape::Put {
                at: PutAt::Upsert,
                text: "closed".to_string(),
            },
            if_node_rev: None,
        },
        Edit {
            target: SecRef::Anchor {
                anchor: "t1".to_string(),
            },
            edit: EditShape::Match {
                old: "[ ]".to_string(),
                new: "[x]".to_string(),
            },
            if_node_rev: None,
        },
        Edit {
            target: SecRef::Anchor {
                anchor: "done".to_string(),
            },
            edit: EditShape::Match {
                old: "closing note".to_string(),
                new: "closed and verified".to_string(),
            },
            if_node_rev: None,
        },
    ]
}

fn close_args(edits: Vec<Edit>) -> SpliceArgs {
    SpliceArgs {
        id: Some(7),
        origin: wire_serve::guard::Origin::Cli,
        path: WPath("card.md".to_string()),
        actor: Some("alice".to_string()),
        now: Some("2026-07-23T12:00:00Z".to_string()),
        receipt: Some(ReceiptAddr {
            path: WPath("receipts.md".to_string()),
            anchor: "close-1".to_string(),
        }),
        if_root: None,
        dry: false,
        force: false,
        edits,
        plan_edits: Vec::new(),
        pin: None,
    }
}

#[test]
fn gate2_n_edit_close_batch_lands_atomically_with_one_rev_mint() {
    let (_d, root) = ws(&[("card.md", CARD)]);

    let outcome =
        splice(&root, 0, &close_args(close_edits()), &[], None).expect("the close batch commits");

    // One root advance / one delta batch at seq 1.
    let ResponseBody::Splice {
        armed,
        root_before,
        root_after,
        seq,
        ..
    } = &outcome.body
    else {
        panic!("a splice returns a Splice body: {:?}", outcome.body);
    };
    let root_after = root_after
        .as_ref()
        .expect("a real commit advances the root");
    assert_ne!(
        root_before, root_after,
        "exactly one rev mint (the root advanced)"
    );
    assert_eq!(*seq, Some(1), "one batch → seq 0 -> 1");
    assert_eq!(armed.edits.len(), 3, "all three edits armed as ONE batch");

    let frame = outcome.committed.expect("a real commit emits one delta");
    assert_eq!(
        frame.delta.seq, 1,
        "one delta = one batch = one root advance"
    );
    assert!(
        frame.delta.files.iter().any(|f| f.path.0 == "card.md"),
        "the one batch carries the content card"
    );

    let card = read(&root, "card.md");
    assert!(
        card.contains("status: closed"),
        "property upsert landed: {card:?}"
    );
    assert!(
        card.contains("- [x] item one ^t1"),
        "checkbox body edit landed"
    );
    assert!(
        card.contains("closed and verified ^done"),
        "note body edit landed"
    );
    assert!(
        card.contains("^t1") && card.contains("^done"),
        "block ids intact"
    );

    let receipts = read(&root, "receipts.md");
    let rows: Vec<&str> = receipts
        .lines()
        .filter(|l| l.contains("^close-1"))
        .collect();
    assert_eq!(rows.len(), 1, "exactly one receipt line: {receipts:?}");
    assert!(
        rows[0].contains("edits=3"),
        "the receipt records all three edits"
    );
}

/// Falsification: one invalid match refuses whole batch — valid status upsert must not leak.
#[test]
fn gate2_falsification_one_invalid_edit_refuses_the_whole_batch() {
    let (_d, root) = ws(&[("card.md", CARD)]);

    let mut edits = close_edits();
    edits[1] = Edit {
        target: SecRef::Anchor {
            anchor: "t1".to_string(),
        },
        edit: EditShape::Match {
            old: "[ NOPE ]".to_string(),
            new: "[x]".to_string(),
        },
        if_node_rev: None,
    };

    let result = splice(&root, 0, &close_args(edits), &[], None);
    assert!(result.is_err(), "one invalid edit refuses the whole batch");

    let card = read(&root, "card.md");
    assert_eq!(card, CARD, "no partial splice — the card is byte-identical");
    assert!(
        card.contains("status: open"),
        "the valid status upsert did NOT leak"
    );

    assert!(
        !root.0.join("receipts.md").exists(),
        "a refused batch writes no receipt (no rev minted)"
    );
}
