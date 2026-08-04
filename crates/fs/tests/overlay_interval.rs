//! **The interval overlay (F1): a snapshot of bytes that are not the worktree's,
//! folded by the same fold.**
//!
//! `domain_snapshot` reads the worktree; a pre-commit fence is asked about the git
//! index. `overlay_snapshot` is how a caller holding the other interval's bytes
//! folds them through ONE fold rather than a second one of its own — and the gate
//! that matters most here is the boring one: **an overlay that changes nothing must
//! fold to the same root.** If it does not, every governed commit greys, because
//! the fold no longer matches the `root_after` a receipt recorded.

use std::path::Path;

use fs::domain::Domain;
use fs::{WorkspaceRoot, domain_snapshot, overlay_snapshot};

fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, contents).unwrap();
}

/// A corpus whose paths are deliberately adjacent in the orderings that could
/// disagree — `a/b.md` against `a.md`, and a nested tree — because the fold is
/// order-sensitive and `PathBuf` ordering is component-wise while a plain string
/// ordering is byte-wise.
fn corpus(root: &Path) {
    write(root, "a.md", "# A\n");
    write(root, "a/b.md", "# A/B\n");
    write(root, "a-1.md", "# A-1\n");
    write(root, "notes/deep/plan.md", "# Plan\n");
    write(root, "zz.md", "# ZZ\n");
}

/// **THE INVARIANT: an overlay that changes nothing folds to the SAME root.**
///
/// This is the one that decides whether the fence is usable at all. The staged
/// interval's fold is compared against the `root_after` a receipt recorded over the
/// worktree, so a fold that differs merely because the files were re-keyed and
/// re-sorted would refuse every governed commit — a fence that blocks everything,
/// which satisfies its refusal clause and is useless.
#[test]
fn an_empty_overlay_folds_to_the_same_root_as_the_worktree_snapshot() {
    let tmp = tempfile::tempdir().unwrap();
    corpus(tmp.path());
    let root = WorkspaceRoot(tmp.path().to_path_buf());
    let domain = Domain::new();
    let (files, fold) = domain_snapshot(&root).unwrap();

    let (overlaid, overlaid_fold) = overlay_snapshot(&files, &[], &domain);
    assert_eq!(
        overlaid_fold.0, fold.0,
        "the same bytes must fold to the same root, or the baseline compare refuses \
         a tree that is current"
    );
    assert_eq!(
        overlaid, files,
        "and the file list is identical, in the SAME ORDER — the fold is \
         order-sensitive, so this is the property the equality above rests on"
    );
}

/// **An overlay that restates the worktree's own bytes is also a no-op** — the
/// ordinary case where a path is staged and identical, and the producer reports it
/// anyway.
#[test]
fn an_overlay_restating_the_same_bytes_changes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    corpus(tmp.path());
    let root = WorkspaceRoot(tmp.path().to_path_buf());
    let domain = Domain::new();
    let (files, fold) = domain_snapshot(&root).unwrap();

    let same: Vec<(String, Option<Vec<u8>>)> = files
        .iter()
        .map(|(rel, bytes)| (rel.clone(), Some(bytes.clone())))
        .collect();
    let (_, overlaid_fold) = overlay_snapshot(&files, &same, &domain);
    assert_eq!(overlaid_fold.0, fold.0, "restating bytes is not a change");
}

/// **Replacing bytes MOVES the root, and the replacement is what gets folded** —
/// the arm that proves the overlay is not silently ignored. Without it every gate
/// above passes over a function that returns its input.
#[test]
fn replacing_one_path_folds_the_replacement_and_moves_the_root() {
    let tmp = tempfile::tempdir().unwrap();
    corpus(tmp.path());
    let root = WorkspaceRoot(tmp.path().to_path_buf());
    let domain = Domain::new();
    let (files, fold) = domain_snapshot(&root).unwrap();

    let (overlaid, overlaid_fold) = overlay_snapshot(
        &files,
        &[("a/b.md".to_owned(), Some(b"# A/B FORGED\n".to_vec()))],
        &domain,
    );
    assert_ne!(overlaid_fold.0, fold.0, "different bytes, different root");
    assert_eq!(
        overlaid
            .iter()
            .find(|(rel, _)| rel == "a/b.md")
            .map(|(_, bytes)| bytes.as_slice()),
        Some(&b"# A/B FORGED\n"[..]),
        "and the OVERLAY's bytes are the ones in the snapshot"
    );

    // The same replacement, written to disk, folds to the same root — the overlay
    // describes a real tree, not a private arithmetic of its own.
    write(tmp.path(), "a/b.md", "# A/B FORGED\n");
    let (_, on_disk) = domain_snapshot(&root).unwrap();
    assert_eq!(
        overlaid_fold.0, on_disk.0,
        "THE ADJUDICATOR: an overlay of bytes X folds exactly as a worktree \
         holding X does"
    );
}

/// **A removal drops the file from the interval** — a path this commit deletes is
/// not part of the tree it records.
#[test]
fn a_removal_drops_the_path_from_the_interval() {
    let tmp = tempfile::tempdir().unwrap();
    corpus(tmp.path());
    let root = WorkspaceRoot(tmp.path().to_path_buf());
    let domain = Domain::new();
    let (files, _) = domain_snapshot(&root).unwrap();

    let (overlaid, overlaid_fold) = overlay_snapshot(&files, &[("a.md".to_owned(), None)], &domain);
    assert!(
        !overlaid.iter().any(|(rel, _)| rel == "a.md"),
        "the removed path is gone from the snapshot"
    );

    std::fs::remove_file(tmp.path().join("a.md")).unwrap();
    let (_, on_disk) = domain_snapshot(&root).unwrap();
    assert_eq!(
        overlaid_fold.0, on_disk.0,
        "and it folds as a worktree without that file does"
    );
}

/// **A path OUTSIDE the hash domain is ignored.** The producer reports whatever
/// git says diverges (code, assets, dot paths), and neither interval hashes those.
///
/// The reserved journal was a third case here — root-excluded by NAMED law rather
/// than by the md-only floor or the dot rule. That carve-out is retired with the
/// journal (ZT 2026-08-02): `meridian/journal.md` is an ordinary in-domain page
/// now, so it WOULD move the root, and asserting otherwise would be false.
#[test]
fn paths_outside_the_hash_domain_are_ignored() {
    let tmp = tempfile::tempdir().unwrap();
    corpus(tmp.path());
    let root = WorkspaceRoot(tmp.path().to_path_buf());
    let domain = Domain::new();
    let (files, fold) = domain_snapshot(&root).unwrap();

    let (overlaid, overlaid_fold) = overlay_snapshot(
        &files,
        &[
            ("src/main.rs".to_owned(), Some(b"fn main() {}\n".to_vec())),
            (".github/ci.md".to_owned(), Some(b"# dot path\n".to_vec())),
        ],
        &domain,
    );
    assert_eq!(
        overlaid_fold.0, fold.0,
        "none of these is in the hash domain, so none of them moves the root"
    );
    assert_eq!(overlaid, files, "and none of them enters the snapshot");
}
