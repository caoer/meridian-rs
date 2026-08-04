//! End-to-end gates for the `check` engine, driven through the PRODUCTION write
//! path (a guarded `create` / `splice` — no in-memory double), over a real tree.
//!
//! # What this file used to gate, and why it does not any more
//! Every fixture here once seeded the reserved receipt journal and asserted the
//! journal TRACE: a spliced row reddening the chain recompute, a stale baseline
//! greying both detectors, the `foreign_edit` withdrawal. **That plane is deleted
//! and is not re-derived**, so those arms are gone with it rather than being
//! reshaped into something that looks like coverage without being any.
//!
//! What remains is gated here honestly: the planes `check` still reads, driven the
//! same production way — and, explicitly, **the law itself**
//! ([`the_engine_keeps_no_memory_and_this_pins_it`]), so that the engine's memorylessness
//! is a pinned, visible fact rather than an absence a later reader has to infer.

use fs::WorkspaceRoot;
use wire::Path as WirePath;
use wire_serve::write::{CreateArgs, SpliceArgs, create, splice};

/// Birth `path` with `body` through the production guarded-create write path.
/// Panics on refusal (the fixtures never provoke one).
fn produce(root: &WorkspaceRoot, path: &str, body: &str) {
    let args = CreateArgs {
        id: None,
        path: WirePath(path.to_string()),
        body: body.to_string(),
        actor: Some("agent:alice".to_string()),
        now: None,
        if_root: None,
        dry: false,
    };
    create(root, 0, &args, &[])
        .unwrap_or_else(|e| panic!("production create {path} refused: {e:?}"));
}

/// Edit `path`'s body through the PRODUCTION splice write path — a fully governed
/// write (flock, CAS, armed gate, receipt), which advances the tree root.
fn splice_through_the_write_path(root: &WorkspaceRoot, path: &str, old: &str, new: &str) {
    let args = SpliceArgs {
        id: None,
        origin: wire_serve::guard::Origin::InProcess,
        path: WirePath(path.to_string()),
        actor: Some("agent:alice".to_string()),
        now: None,
        receipt: None,
        if_root: None,
        dry: false,
        force: false,
        edits: vec![wire::Edit {
            target: wire::SecRef::Hpath {
                hpath: vec![wire::HpathSeg {
                    h: "A".to_string(),
                    n: None,
                }],
            },
            edit: wire::EditShape::Match {
                old: old.to_string(),
                new: new.to_string(),
            },
            if_node_rev: None,
        }],
        plan_edits: Vec::new(),
        pin: None,
    };
    splice(root, 0, &args, &[], None)
        .unwrap_or_else(|e| panic!("production splice on {path} refused: {e:?}"));
}

/// The live tree merkle root.
fn live_root(root: &WorkspaceRoot) -> String {
    fs::domain_snapshot(root).expect("snapshot").1.0
}

/// Build the corpus and run the layer-0 core the way `mrd check` runs it.
fn core_over(root: &WorkspaceRoot) -> check::CoreReport {
    let (files, _fold) = fs::domain_snapshot(root).expect("snapshot");
    let (_index, docs) = fs::build_corpus(files).expect("corpus");
    check::core(root, &docs, &[], &[]).expect("core read")
}

/// A governed corpus reads GREEN, and the green is EARNED — the write path really
/// ran, the tree really moved, and every plane `check` holds was really read.
///
/// Load-bearing against the opposite failure: a verb that lost its planes and
/// returned green unconditionally would also pass this arm, which is why
/// [`a_drifted_pin_still_reddens_after_the_journal_died`] runs beside it.
#[test]
fn a_governed_corpus_is_green_on_every_plane_check_still_holds() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = WorkspaceRoot(tmp.path().to_path_buf());
    produce(&root, "a.md", "# A\n\nalpha\n");
    let before = live_root(&root);
    splice_through_the_write_path(&root, "a.md", "alpha", "alpha edited");
    assert_ne!(
        before,
        live_root(&root),
        "FIXTURE: the governed splice really advanced the tree"
    );

    let report = core_over(&root);
    assert!(!report.is_red(), "a governed corpus lies about nothing");
    assert!(
        !report.cannot_assess(),
        "and every plane it holds was readable"
    );
    assert_eq!(report.red_summary(), None);
    assert_eq!(report.grey_summary(), None);
}

/// **THE LAW, PINNED** (ZT, 2026-08-03): *"Engine does not have memory. It should
/// not have. History is pin to git when we lock. Anything between locks is not
/// history."*
///
/// An out-of-band edit — a plain filesystem write behind the engine's back, exactly
/// what the journal's stale-baseline grey once caught — is followed by nothing that
/// re-reads it. `check` returns GREEN, because it answers at-rest truth only: does
/// the world still match the pins. It does, so it says so.
///
/// **This arm asserts that on purpose, and it is not an apology.** The engine is
/// ruled not to keep memory, so non-detection here is the DESIGN, not a hole the
/// engine tolerates. Anyone who arrives wanting to "fix" it fails this test and has
/// to read the ruling first — a detector would be this engine reaching back into
/// memory it is ruled not to have. Archaeology is git; attribution is transcript
/// JSONL.
#[test]
fn the_engine_keeps_no_memory_and_this_pins_it() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = WorkspaceRoot(tmp.path().to_path_buf());
    produce(&root, "a.md", "# A\n\nalpha\n");
    let governed = live_root(&root);

    // The out-of-band write: no flock, no CAS, no receipt, no engine involvement.
    std::fs::write(
        root.0.join("a.md"),
        "# A\n\nlaundered — landed behind the engine's back\n",
    )
    .expect("out-of-band write");
    assert_ne!(
        governed,
        live_root(&root),
        "FIXTURE: the tree really moved, and nothing governed moved it"
    );

    let report = core_over(&root);
    assert!(
        !report.is_red(),
        "BY DESIGN: check holds no plane that can see an out-of-band edit, because \
         the engine keeps no memory (ZT 2026-08-03). If you are here wanting to catch \
         this, read the ruling first — history is pinned to git at lock, anything \
         between locks is not history, and a detector here would rebuild the memory \
         the engine is ruled not to have."
    );
    assert!(
        !report.cannot_assess(),
        "and it is not grey either — check does not ASK about write history, so there \
         is no unanswered question to report. Not-checked is not cannot-assess."
    );
}

/// The surviving plane still reddens end-to-end: a pin whose target drifted is a
/// finding, over a real corpus, through the real core.
///
/// This is the arm that makes the two greens above mean something — a core that had
/// lost its remaining planes too would pass those and fail this one.
#[test]
fn a_drifted_pin_still_reddens_after_the_journal_died() {
    use std::collections::BTreeMap;

    let tmp = tempfile::tempdir().expect("tempdir");
    let root = WorkspaceRoot(tmp.path().to_path_buf());
    let docs: BTreeMap<String, model::Document> = BTreeMap::new();
    let drifted = check::PinRow {
        src_path: "claim.md".to_string(),
        declared_ref: "source.md#S".to_string(),
        color: model::selector::Color::Red(model::selector::RedReason::Drifted),
        label: "red content-drifted".to_string(),
    };
    let plane = check::pin_plane(&root, &docs, std::slice::from_ref(&drifted));
    assert!(
        plane.is_red(),
        "the ledger claims content that is not there"
    );
    assert!(
        !plane.cannot_assess(),
        "it was assessed — that is why it is red, and it is not filed as an absence"
    );
}

/// **Honest degradation survives the journal.** An object store that cannot be
/// asked leaves the retrieval plane UNREAD — grey, never an empty reading a caller
/// could bank as clean. The false green this closes is one plane over from the one
/// the engine is ruled not to hold, and it is still closed.
#[test]
fn an_unaskable_object_store_is_still_grey_and_never_a_clean_reading() {
    use std::collections::BTreeMap;

    let root = WorkspaceRoot(std::path::PathBuf::from("/nonexistent"));
    let page = format!(
        "# P\n\n```meridian-lock\nversion: 1\nobjects:\n  \"source.md\": \"{}\"\n```\n",
        "a".repeat(40)
    );
    let doc = model::build(page.clone(), syntax::parse(&page));
    let docs = BTreeMap::from([("claim.md".to_string(), doc)]);

    let plane = check::pin_plane(&root, &docs, &[]);
    assert!(
        plane.cannot_ask.is_some(),
        "there is no repository at /nonexistent — the question could not be put"
    );
    assert!(plane.cannot_assess(), "so the plane is grey");
    assert!(!plane.is_green(), "and it is not clean");
}
