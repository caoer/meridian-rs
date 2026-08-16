//! The card's instrumented gate (write-path-overlay + bug-remove-corpus-snapshot):
//! ZERO full-corpus reads inside the flock on every guarded write door.
//!
//! `fs::fold_count()` counts `fs::domain_snapshot` passes — the read+fold
//! primitive both retired flock-held corpus reads rode. The write doors ride
//! the resident tree (observation + own-write overlay). Remove's referential
//! check uses `query::backlinks` / `query::lock_pin_referrers` over
//! `fs::hash_domain` bytes, never `domain_snapshot`. The counter is
//! process-global, which is why this file holds exactly ONE test and nothing
//! else: within this test binary no other fold can race the measured window.

use std::path::Path;

use wire::{Edit, EditShape, HpathSeg, NodeRev, Path as WirePath, SecRef};
use wire_serve::write::{
    CreateArgs, LockWriteArgs, RemoveArgs, SpliceArgs, SpliceSetArgs, create, lock_write, remove,
    splice, splice_set,
};

fn page_body(word: &str) -> String {
    format!("# Alpha\n\n## Beta\n\nship by {word}\n")
}

fn page(dir: &tempfile::TempDir, rel: &str, word: &str) {
    let abs = dir.path().join(rel);
    std::fs::create_dir_all(abs.parent().expect("parent")).expect("mkdir");
    std::fs::write(abs, page_body(word)).expect("write");
}

fn live_rev(root: &fs::WorkspaceRoot, rel: &str) -> NodeRev {
    NodeRev(
        fs::load(root, Path::new(rel))
            .expect("load")
            .root
            .node_rev
            .0
            .clone(),
    )
}

fn match_edit(old: &str, new: &str) -> Edit {
    Edit {
        target: SecRef::Hpath {
            hpath: vec![
                HpathSeg {
                    h: "Alpha".into(),
                    n: None,
                },
                HpathSeg {
                    h: "Beta".into(),
                    n: None,
                },
            ],
        },
        edit: EditShape::Match {
            old: old.into(),
            new: new.into(),
        },
        if_node_rev: None,
    }
}

fn splice_args(path: &str, old: &str, new: &str) -> SpliceArgs {
    SpliceArgs {
        id: None,
        origin: wire_serve::guard::Origin::InProcess,
        path: WirePath(path.into()),
        actor: Some("alice".into()),
        now: None,
        receipt: None,
        if_root: None,
        dry: false,
        force: false,
        edits: vec![match_edit(old, new)],
        plan_edits: Vec::new(),
        pin: None,
    }
}

fn create_args(path: &str, body: &str) -> CreateArgs {
    CreateArgs {
        id: None,
        path: WirePath(path.into()),
        body: body.into(),
        actor: Some("alice".into()),
        now: None,
        if_root: None,
        dry: false,
    }
}

fn remove_args(path: &str, if_file_rev: NodeRev) -> RemoveArgs {
    RemoveArgs {
        id: None,
        path: WirePath(path.into()),
        if_file_rev: Some(if_file_rev),
        actor: Some("alice".into()),
        now: None,
        if_root: None,
        dry: false,
    }
}

fn lock_args(path: &str, if_file_rev: NodeRev) -> LockWriteArgs {
    LockWriteArgs {
        id: None,
        path: WirePath(path.into()),
        lock: lock::Lock::new(),
        actor: None,
        now: None,
        if_root: None,
        if_file_rev,
        dry: false,
    }
}

fn set_args(files: Vec<wire::SpliceFile>) -> SpliceSetArgs {
    SpliceSetArgs {
        id: None,
        files,
        origin: wire_serve::guard::Origin::InProcess,
        actor: None,
        now: None,
        receipt: None,
        if_root: None,
        dry: false,
        force: false,
    }
}

fn assert_no_fold(label: &str, before: u64) {
    assert_eq!(
        fs::fold_count() - before,
        0,
        "{label} runs zero domain_snapshot passes"
    );
}

/// Every guarded door — create, splice, `splice.set`, `lock_write`, remove
/// (cold and warm), domain-config — runs zero `domain_snapshot` passes.
/// The oracle cross-check (an explicit `ambient_root` full fold) runs
/// OUTSIDE the measured windows and proves the last served token still
/// equals the old-law disk fold.
#[allow(clippy::too_many_lines)]
#[test]
fn a_guarded_write_runs_zero_full_corpus_reads() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    for i in 0..16 {
        page(&dir, &format!("bystander{i}.md"), "x");
    }
    std::fs::create_dir_all(dir.path().join("notes")).expect("mkdir");
    page(&dir, "notes/plan.md", "August");
    page(&dir, "notes/second.md", "August");
    page(&dir, "notes/victim_cold.md", "gone");
    page(&dir, "notes/victim_warm.md", "gone");

    // Cold window: the first door call in this process on this root is a
    // remove. Its observation reads members through the resident cache;
    // the referential check lists+reads bytes and never folds.
    let cold_rev = live_rev(&root, "notes/victim_cold.md");
    let folds_before = fs::fold_count();
    remove(
        &root,
        None,
        &remove_args("notes/victim_cold.md", cold_rev),
        &[],
    )
    .expect("cold remove");
    assert_no_fold("a COLD remove", folds_before);

    let folds_before = fs::fold_count();
    create(&root, None, &create_args("notes/new.md", "# New\n"), &[]).expect("create");
    assert_no_fold("create", folds_before);

    let folds_before = fs::fold_count();
    splice(
        &root,
        None,
        &splice_args("notes/plan.md", "August", "w1"),
        &[],
        None,
    )
    .expect("splice");
    assert_no_fold("splice", folds_before);

    let folds_before = fs::fold_count();
    splice_set(
        &root,
        None,
        &set_args(vec![
            wire::SpliceFile {
                path: WirePath("notes/plan.md".into()),
                edits: vec![match_edit("w1", "w2")],
                plan_edits: Vec::new(),
            },
            wire::SpliceFile {
                path: WirePath("notes/second.md".into()),
                edits: vec![match_edit("August", "w2")],
                plan_edits: Vec::new(),
            },
        ]),
        &[],
    )
    .expect("splice.set");
    assert_no_fold("splice.set", folds_before);

    let plan_rev = live_rev(&root, "notes/plan.md");
    let folds_before = fs::fold_count();
    lock_write(&root, None, &lock_args("notes/plan.md", plan_rev)).expect("lock_write");
    assert_no_fold("lock_write", folds_before);

    let folds_before = fs::fold_count();
    let config = create(
        &root,
        None,
        &create_args(
            fs::domain::DOMAIN_CONFIG_PATH,
            "---\nignore:\n  - \"unused/**\"\n---\n# Domain\n",
        ),
        &[],
    )
    .expect("domain-config");
    assert_no_fold("domain-config", folds_before);

    // Warm window: the cache already observed this root.
    let warm_rev = live_rev(&root, "notes/victim_warm.md");
    let folds_before = fs::fold_count();
    let warm = remove(
        &root,
        None,
        &remove_args("notes/victim_warm.md", warm_rev),
        &[],
    )
    .expect("warm remove");
    assert_no_fold("a WARM remove", folds_before);

    // Oracle, outside the windows: the last served token is the old-law fold
    // of what actually landed (interim served-token law, merged plan §6 step 3).
    let oracle = wire_serve::ambient_root(&root).expect("oracle fold");
    assert_eq!(warm.root_after.as_ref().expect("warm root_after"), &oracle);
    assert_eq!(
        config.root_after.expect("config root_after"),
        warm.root_before,
        "the chain holds across the last two writes"
    );
}
