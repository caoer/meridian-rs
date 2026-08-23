//! The gate on [`fs::domain_fold`]: it is `domain_snapshot(root).1`, always.
//!
//! `domain_fold` exists only to skip the `DomainFiles` allocation the run
//! plane drops on the next line. The instant its token differs from
//! `domain_snapshot`'s, two observations of one tree disagree — and the
//! disagreement would surface as a receipt `root_pin` nobody can reproduce
//! with the snapshotting instrument. So the equality IS the feature, and it
//! has to hold over the shapes that make a fold's key order and member set
//! non-trivial, not just over a flat happy-path corpus.

use std::path::Path;

use fs::WorkspaceRoot;

fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, contents).unwrap();
}

/// Both instruments, one tree, one assertion.
fn assert_agree(root: &WorkspaceRoot, why: &str) {
    let (_, snapshot_root) = fs::domain_snapshot(root).unwrap();
    let fold_root = fs::domain_fold(root).unwrap();
    assert_eq!(
        fold_root, snapshot_root,
        "domain_fold must equal domain_snapshot's root: {why}"
    );
}

/// The ordinary corpus, plus the two things that decide a fold's key order:
/// nested directories (so the walk yields a non-flat sequence) and sibling
/// names whose byte order differs from any per-directory listing order.
#[test]
fn fold_equals_snapshot_root_on_a_nested_corpus() {
    let tmp = tempfile::tempdir().unwrap();
    let root_path = tmp.path();
    write(root_path, "MERIDIAN.md", "# root\n");
    write(root_path, "notes/plan.md", "# Plan\n");
    write(root_path, "notes/b.md", "# b\n");
    write(root_path, "notes/a.md", "# a\n");
    write(root_path, "year=2026/month=08/deep/leaf.md", "# leaf\n");
    write(root_path, "year=2026/month=07/deep/leaf.md", "# other\n");
    // Non-md and dot-segment members: in neither instrument's domain, so a
    // divergence here would mean the two walks disagree about the member SET.
    write(root_path, "mdfs_config.yaml", "version: 0\nignore: []\n");
    write(root_path, ".github/README.md", "# CI notes\n");

    assert_agree(
        &WorkspaceRoot(root_path.to_path_buf()),
        "nested corpus with unordered siblings and out-of-domain files",
    );
}

/// The empty domain — the degenerate fold. `served_root` over no leaves still
/// mints a token, and the two paths must mint the SAME one rather than one of
/// them short-circuiting.
#[test]
fn fold_equals_snapshot_root_on_an_empty_domain() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "mdfs_config.yaml", "version: 0\nignore: []\n");

    assert_agree(
        &WorkspaceRoot(tmp.path().to_path_buf()),
        "a domain with no hashed members",
    );
}

/// Above `PARALLEL_READ_FLOOR` (64) both sweeps go multi-threaded and merge
/// per-chunk results. A chunking or merge-order defect in the fold-only sweep
/// shows up here and nowhere else — the smaller corpora above stay serial.
#[test]
fn fold_equals_snapshot_root_above_the_parallel_read_floor() {
    let tmp = tempfile::tempdir().unwrap();
    let root_path = tmp.path();
    for i in 0..300 {
        // Spread across directories so the walk order is not the name order.
        write(
            root_path,
            &format!("d{:02}/m{:03}.md", i % 7, i),
            &format!("# member {i}\n"),
        );
    }

    assert_agree(
        &WorkspaceRoot(root_path.to_path_buf()),
        "300 members — both sweeps run parallel and merge chunks",
    );
}

/// The token must TRACK the corpus, not merely match once: a fold that cached
/// or short-circuited would still pass the equality above while reporting a
/// stale root. Move the tree, re-fold, and require both that the two agree
/// again and that the value actually changed.
#[test]
fn fold_tracks_a_moved_corpus() {
    let tmp = tempfile::tempdir().unwrap();
    let root_path = tmp.path();
    write(root_path, "notes/plan.md", "# Plan\n");
    let root = WorkspaceRoot(root_path.to_path_buf());

    let before = fs::domain_fold(&root).unwrap();
    assert_agree(&root, "before the corpus moved");

    write(root_path, "notes/plan.md", "# Plan, revised\n");
    let after = fs::domain_fold(&root).unwrap();
    assert_ne!(
        before, after,
        "an edited member must move the fold-only root"
    );
    assert_agree(&root, "after the corpus moved");

    write(root_path, "notes/second.md", "# Second\n");
    let grown = fs::domain_fold(&root).unwrap();
    assert_ne!(after, grown, "a new member must move the fold-only root");
    assert_agree(&root, "after a member was added");
}

/// A member that cannot be read refuses the whole fold, exactly as it refuses
/// the whole snapshot — same failure, so a fold-only caller never proceeds on
/// a corpus the snapshotting caller would have rejected.
#[cfg(unix)]
#[test]
fn an_unreadable_member_refuses_both_instruments() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let root_path = tmp.path();
    write(root_path, "notes/plan.md", "# Plan\n");
    write(root_path, "notes/sealed.md", "# Sealed\n");
    let sealed = root_path.join("notes/sealed.md");
    std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o000)).unwrap();

    let root = WorkspaceRoot(root_path.to_path_buf());
    let snapshot = fs::domain_snapshot(&root);
    let fold = fs::domain_fold(&root);

    // Running as root defeats the permission bit; then neither refuses and
    // the gate has nothing to say. Assert the agreement either way.
    match (snapshot, fold) {
        (Err(s), Err(f)) => assert_eq!(
            s.kind(),
            f.kind(),
            "both instruments must refuse an unreadable member the same way"
        ),
        (Ok((_, s)), Ok(f)) => {
            assert_eq!(f, s, "when the member is readable the roots still agree");
        }
        (s, f) => panic!("instruments disagreed about refusing: {s:?} vs {f:?}"),
    }

    std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o644)).unwrap();
}
