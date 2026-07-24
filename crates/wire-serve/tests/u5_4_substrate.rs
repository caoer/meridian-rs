//! U5.4 remaining substrate debts — the merge-gate acceptance suite.
//!
//! Two named gates, each falsification-verified in the body:
//!
//! - **Gate 1 — heading-rename round-trip preserves block ids + reddens
//!   hpath-pinned dependents correctly.** `query::plan_rename` (the rung-5 gap,
//!   the heading-rename op) is applied through the strict writer
//!   (`model::validate_batch` → `fs::apply_batch`, the query charter's "model
//!   validation + fs execution"). After the rename: every `^block-id` survives
//!   (`preserves_block_ids`), an hpath pin reddens `red(selector-unresolved)`, a
//!   body-`^block-id` pin stays green (U2.2 `classify_edge`), and renaming BACK
//!   restores the original bytes → the hpath pin greens again (true round-trip).
//!   Falsification: a body-DROPPING rename loses `^abc` — the block-id assertion
//!   discriminates.
//!
//! - **Gate 2 — the general N-edit close batch lands atomically with one rev
//!   mint.** A 3-edit `splice` (a frontmatter `upsert` property + two body
//!   `match`es) commits through `wire_serve::write::splice` in ONE root advance,
//!   one receipt line. Failure case (the falsification): one invalid edit refuses
//!   the WHOLE batch — the card is byte-identical (the valid `status: closed`
//!   never lands), no receipt, no rev minted.

use std::collections::BTreeMap;
use std::path::Path as FsPath;

use model::selector::{Color, RedReason, Selector, classify_edge};
use model::{
    CorpusIndex, Document, HpathSeg as MHpathSeg, NodeRev, Ref, SpliceRequest, SpliceVerdict,
};
use query::{RenamePlan, plan_rename};
use wire::{Edit, EditShape, Path as WPath, PutAt, ReceiptAddr, ResponseBody, SecRef};
use wire_serve::write::{SpliceArgs, splice};

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Build a temp workspace from `(path, content)` files.
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

/// The borrowed corpus (index + docs) `plan_rename` reads.
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

/// Apply a [`RenamePlan`] through the strict writer: validate each `(path, batch)`
/// splice and land it atomically via `fs::apply_batch` (the query charter's
/// "model validation + fs execution"). A refusal panics loud — the plan is
/// supposed to validate.
fn apply_plan(root: &fs::WorkspaceRoot, plan: &RenamePlan) {
    for (path, batch) in &plan.edits {
        let doc = build_doc(&read(root, path));
        match model::validate_batch(&doc, None, batch, None) {
            SpliceVerdict::Validated(sealed) => {
                fs::apply_batch(root, FsPath::new(path), None, &sealed, doc.raw.as_bytes())
                    .expect("apply_batch lands the rename splice");
            }
            other => panic!("rename splice for {path} did not validate: {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Gate 1 — heading-rename round-trip: block ids + selector colors
// ---------------------------------------------------------------------------

/// The heading rename preserves every `^block-id`, reddens the hpath pin to the
/// old name (`red(selector-unresolved)`), keeps the body-`^block-id` pin green,
/// and — renamed BACK — restores the original bytes so the hpath pin greens
/// again. Every color is asserted, none assumed.
#[test]
fn gate1_heading_rename_preserves_block_ids_and_reddens_hpath_pins() {
    let original = "# Foo\n\nalpha body ^abc\n\nbeta body\n";
    let (_d, root) = ws(&[("page.md", original)]);

    // Pins captured against the PRE-rename document.
    let before = build_doc(original);
    let pinned_hpath = node_rev(&before, &hpath(&["Foo"]));
    let pinned_block = node_rev(&before, &Ref::anchor("abc").unwrap());

    // Rename # Foo -> # Bar through the strict writer.
    let (index, docs) = corpus(&root, &["page.md"]);
    let plan = plan_rename(&index, &docs, "page.md", &hpath(&["Foo"]), "Bar");
    apply_plan(&root, &plan);

    let after_raw = read(&root, "page.md");
    let after = build_doc(&after_raw);

    // Block id survives the rename — present in bytes AND still resolvable.
    assert!(
        after_raw.contains("^abc"),
        "the block id survives: {after_raw:?}"
    );
    assert!(
        model::resolve(&after, &Ref::anchor("abc").unwrap()).is_ok(),
        "the ^abc anchor still resolves after the rename"
    );
    // The heading text actually changed.
    assert!(after_raw.contains("# Bar"), "heading renamed to Bar");
    assert!(!after_raw.contains("# Foo"), "the old heading text is gone");

    // hpath pin reddens: the old selector resolves to nothing.
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

    // block pin stays green: the body block is byte-untouched, rev unchanged.
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

    // Round-trip: rename BACK restores byte-identical content → the hpath pin
    // greens again (the section rev is heading-inclusive, so an exact restore is
    // an exact rev).
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

/// FALSIFICATION for gate 1's block-id guard: a rename that DROPS the section
/// body (a naive whole-section `put`) loses `^abc` — so the gate's block-id
/// assertion is load-bearing (it fails under a body-dropping rename), and the
/// hpath-pin `red` is what a broken rename would still leave green.
#[test]
fn gate1_falsification_body_dropping_rename_loses_block_id() {
    let original = "# Foo\n\nalpha body ^abc\n\nbeta body\n";
    let (_d, root) = ws(&[("page.md", original)]);

    // A broken "rename" that replaces the whole section with just a heading line
    // (put at:all) — the shape plan_rename deliberately does NOT emit.
    let doc = build_doc(original);
    let bad = SpliceRequest {
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
    )
    .expect("apply");

    let after_raw = read(&root, "page.md");
    // The guard the real rename upholds is exactly this negation: a body-dropping
    // rename LOSES the block id (so `preserves_block_ids` above truly discriminates).
    assert!(
        !after_raw.contains("^abc"),
        "the body-dropping rename drops the block id — the gate-1 assertion is load-bearing"
    );
}

/// The rung-5 op is a heading rename WITH backlink fixup: every wikilink/embed
/// whose fragment is the renamed heading is rewritten (`#Foo` → `#Bar`),
/// alias-preserving, while a `^block-id` link is left untouched (its anchor
/// survives the rename). Cross-file and sub-section links both covered.
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

    // And the heading itself renamed, block id preserved.
    let page_after = read(&root, "page.md");
    assert!(page_after.contains("# Bar") && page_after.contains("^abc"));
}

// ---------------------------------------------------------------------------
// Gate 2 — the general N-edit close batch: atomic, one rev mint
// ---------------------------------------------------------------------------

const CARD: &str = "---\ntitle: draft\nstatus: open\nowner: alice\n---\n\n# Task\n\n- [ ] item one ^t1\n\nclosing note ^done\n";

/// The canonical close batch: a frontmatter `upsert` property + two body
/// `match`es, mixed in ONE splice. Distinct-target, disjoint spans.
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
    }
}

#[test]
fn gate2_n_edit_close_batch_lands_atomically_with_one_rev_mint() {
    let (_d, root) = ws(&[("card.md", CARD)]);

    let outcome =
        splice(&root, 0, &close_args(close_edits()), &[]).expect("the close batch commits");

    // ONE root advance = one rev mint; ONE delta batch at seq 1.
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
    // ONE delta (one batch, one seq) carries the content card plus the receipt it
    // rides — the atomicity fact is the single seq/root advance, not the file count.
    assert!(
        frame.delta.files.iter().any(|f| f.path.0 == "card.md"),
        "the one batch carries the content card"
    );

    // All three edits landed together.
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

    // Exactly one receipt row (the close's audit record) — one journal-grade line.
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

/// FALSIFICATION for gate 2's atomicity: one invalid edit (a `match` with no
/// occurrence) refuses the WHOLE batch — zero partial splice, no rev minted. The
/// load-bearing assertion is that the VALID `status: closed` never lands: a
/// non-atomic writer would leak it, and this test would fail.
#[test]
fn gate2_falsification_one_invalid_edit_refuses_the_whole_batch() {
    let (_d, root) = ws(&[("card.md", CARD)]);

    // Edit 2 now cannot match — the batch must refuse as a whole.
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

    let result = splice(&root, 0, &close_args(edits), &[]);
    assert!(result.is_err(), "one invalid edit refuses the whole batch");

    // ZERO partial splice: the card is byte-identical — the valid property edit
    // NEVER landed (the atomicity proof).
    let card = read(&root, "card.md");
    assert_eq!(card, CARD, "no partial splice — the card is byte-identical");
    assert!(
        card.contains("status: open"),
        "the valid status upsert did NOT leak"
    );

    // No rev minted: the receipt file was never created.
    assert!(
        !root.0.join("receipts.md").exists(),
        "a refused batch writes no receipt (no rev minted)"
    );
}
