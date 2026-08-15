//! The card's instrumented gate (write-path-overlay): ZERO full-corpus reads
//! inside the flock on a guarded write.
//!
//! `fs::fold_count()` counts `fs::domain_snapshot` passes — the read+fold
//! primitive both retired flock-held corpus reads rode. The write doors now
//! ride the resident tree (observation + own-write overlay), so a guarded
//! splice must leave the counter untouched. The counter is process-global,
//! which is why this file holds exactly ONE test and nothing else: within
//! this test binary no other fold can race the measured window.

use wire::{Edit, EditShape, HpathSeg, Path, SecRef};
use wire_serve::write::{SpliceArgs, splice};

fn page_body(word: &str) -> String {
    format!("# Alpha\n\n## Beta\n\nship by {word}\n")
}

fn splice_args(path: &str, old: &str, new: &str) -> SpliceArgs {
    SpliceArgs {
        id: None,
        origin: wire_serve::guard::Origin::InProcess,
        path: Path(path.into()),
        actor: Some("alice".into()),
        now: None,
        receipt: None,
        if_root: None,
        dry: false,
        force: false,
        edits: vec![Edit {
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
        }],
        plan_edits: Vec::new(),
        pin: None,
    }
}

/// A guarded splice — cold and warm — runs zero `domain_snapshot` passes:
/// the ~1.5 s two-read mechanism is structurally gone from the write path.
/// The oracle cross-check (an explicit `ambient_root` full fold) runs OUTSIDE
/// the measured windows and proves the served tokens still equal the old-law
/// disk fold.
#[test]
fn a_guarded_write_runs_zero_full_corpus_reads() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    for i in 0..16 {
        std::fs::write(dir.path().join(format!("bystander{i}.md")), page_body("x"))
            .expect("bystander");
    }
    std::fs::create_dir_all(dir.path().join("notes")).expect("mkdir");
    std::fs::write(dir.path().join("notes/plan.md"), page_body("August")).expect("target");

    // Cold window: the first door call in this process. Its observation reads
    // members through the resident cache — never through `domain_snapshot`.
    let folds_before = fs::fold_count();
    let cold = splice(
        &root,
        None,
        &splice_args("notes/plan.md", "August", "w1"),
        &[],
        None,
    )
    .expect("cold guarded splice");
    assert_eq!(
        fs::fold_count() - folds_before,
        0,
        "a COLD guarded write runs zero domain_snapshot passes"
    );

    // Warm window: the steady-state guarded write.
    let folds_before = fs::fold_count();
    let warm = splice(
        &root,
        None,
        &splice_args("notes/plan.md", "w1", "w2"),
        &[],
        None,
    )
    .expect("warm guarded splice");
    assert_eq!(
        fs::fold_count() - folds_before,
        0,
        "a WARM guarded write runs zero domain_snapshot passes"
    );

    // Oracle, outside the windows: the served tokens are the old-law fold of
    // what actually landed (interim served-token law, merged plan §6 step 3).
    let oracle = wire_serve::ambient_root(&root).expect("oracle fold");
    let warm_frame = warm.committed.expect("warm frame");
    assert_eq!(warm_frame.delta.root_after, oracle);
    assert_eq!(
        warm_frame.delta.root_before,
        cold.committed.expect("cold frame").delta.root_after,
        "the chain holds across the two writes"
    );
}
