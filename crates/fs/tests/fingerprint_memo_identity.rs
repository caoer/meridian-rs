//! The gate on [`fs::domain_fingerprint_memoized`]: it is
//! `domain_snapshot(root).1`, always — over a cold memo, a warm memo, a memo
//! carried across processes, and a corpus that moved under any of them.
//!
//! The entry exists so a fingerprint-only door outside the daemon (`mrd sql`'s
//! post-result `live` sample, merkle-spec §6.7) stops reading and hashing
//! every member for a root it could serve from the drawer memo. The instant
//! its token differs from `domain_snapshot`'s, a `live` sample names a root
//! no byte-derived instrument can reproduce — so the equality IS the feature,
//! and it has to hold over the shapes that make a fold's key order and member
//! set non-trivial, and over the memo states that make WHERE a digest comes
//! from non-trivial.

use std::path::Path;

use fs::WorkspaceRoot;
use fs::digestmemo::DigestMemo;

fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, contents).unwrap();
}

/// Rewrite with a forced timestamp tick, so a change is never hidden inside
/// one filesystem timestamp granule (the memo's disclosed blind spot).
fn rewrite(root: &Path, rel: &str, contents: &str) {
    std::thread::sleep(std::time::Duration::from_millis(10));
    write(root, rel, contents);
}

/// Both instruments, one tree, one assertion — through whatever state `memo`
/// is in. Returns the agreed root.
fn assert_agree(root: &WorkspaceRoot, memo: &mut DigestMemo, why: &str) -> model::MerkleRoot {
    let (_, snapshot_root) = fs::domain_snapshot(root).unwrap();
    let memo_root = fs::domain_fingerprint_memoized(root, memo).unwrap();
    assert_eq!(
        memo_root, snapshot_root,
        "domain_fingerprint_memoized must equal domain_snapshot's root: {why}"
    );
    memo_root
}

/// The ordinary corpus, plus the two things that decide a fold's key order:
/// nested directories (so the walk yields a non-flat sequence) and sibling
/// names whose byte order differs from any per-directory listing order.
/// Cold memo first (every member read), then warm (no member read).
#[test]
fn memoized_fingerprint_equals_snapshot_root_on_a_nested_corpus() {
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
    let root = WorkspaceRoot(root_path.to_path_buf());
    let mut memo = DigestMemo::new();

    let cold = assert_agree(&root, &mut memo, "cold memo, nested corpus");
    assert_eq!(
        memo.misses(),
        6,
        "a cold memo reads every domain member once"
    );
    assert_eq!(memo.hits(), 0);

    let warm = assert_agree(&root, &mut memo, "warm memo, same corpus");
    assert_eq!(warm, cold);
    assert_eq!(
        memo.misses(),
        6,
        "a warm memo over an unchanged tree reads nothing"
    );
    assert_eq!(memo.hits(), 6, "every member served from the memo");
}

/// The empty domain — the degenerate fold. `served_root` over no leaves still
/// mints a token, and the two paths must mint the SAME one rather than one of
/// them short-circuiting.
#[test]
fn memoized_fingerprint_equals_snapshot_root_on_an_empty_domain() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "mdfs_config.yaml", "version: 0\nignore: []\n");
    let root = WorkspaceRoot(tmp.path().to_path_buf());
    let mut memo = DigestMemo::new();

    assert_agree(&root, &mut memo, "a domain with no hashed members, cold");
    assert_agree(&root, &mut memo, "a domain with no hashed members, warm");
    assert_eq!((memo.hits(), memo.misses()), (0, 0));
}

/// Above `PARALLEL_READ_FLOOR` (64) the miss read goes multi-threaded and
/// merges per-chunk results; above the stat floor the identity sweep would
/// too. A chunking or zip-order defect between the stat sweep, the miss list
/// and the read rows shows up here and nowhere else — the smaller corpora
/// above stay serial.
#[test]
fn memoized_fingerprint_equals_snapshot_root_above_the_parallel_read_floor() {
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
    let root = WorkspaceRoot(root_path.to_path_buf());
    let mut memo = DigestMemo::new();

    assert_agree(&root, &mut memo, "300 members, cold — parallel miss read");
    assert_eq!(memo.misses(), 300);
    assert_agree(&root, &mut memo, "300 members, warm");
    assert_eq!(memo.misses(), 300, "the warm pass read nothing");
    assert_eq!(memo.hits(), 300);

    // A partial miss: a handful of movers spread across chunks, so the miss
    // list is a sparse subsequence of the stat sweep and the zip of misses
    // with read rows is the thing under test.
    std::thread::sleep(std::time::Duration::from_millis(10));
    for i in [3, 77, 150, 299] {
        write(
            root_path,
            &format!("d{:02}/m{:03}.md", i % 7, i),
            &format!("# member {i}, revised\n"),
        );
    }
    assert_agree(&root, &mut memo, "300 members, four movers");
    assert_eq!(memo.misses(), 304, "exactly the movers were re-read");
}

/// The token must TRACK the corpus, not merely match once: a memo that kept
/// serving a stale row would still pass the equality above on an unchanged
/// tree while reporting a stale root on a moved one. Move the tree under a
/// warm memo, re-fold, and require both that the two agree again and that
/// the value actually changed.
#[test]
fn memoized_fingerprint_tracks_a_moved_corpus() {
    let tmp = tempfile::tempdir().unwrap();
    let root_path = tmp.path();
    write(root_path, "notes/plan.md", "# Plan\n");
    write(root_path, "notes/other.md", "# Other\n");
    let root = WorkspaceRoot(root_path.to_path_buf());
    let mut memo = DigestMemo::new();

    let before = assert_agree(&root, &mut memo, "before the corpus moved");

    rewrite(root_path, "notes/plan.md", "# Plan, revised\n");
    let after = assert_agree(&root, &mut memo, "after a member was edited");
    assert_ne!(
        before, after,
        "an edited member must move the memoized root"
    );
    assert_eq!(memo.misses(), 3, "only the edited member was re-read");

    write(root_path, "notes/second.md", "# Second\n");
    let grown = assert_agree(&root, &mut memo, "after a member was added");
    assert_ne!(after, grown, "a new member must move the memoized root");

    std::fs::rename(
        root_path.join("notes/second.md"),
        root_path.join("notes/renamed.md"),
    )
    .unwrap();
    let renamed = assert_agree(&root, &mut memo, "after a member was renamed");
    assert_ne!(
        grown, renamed,
        "a rename moves the root (the name is hashed)"
    );

    std::fs::remove_file(root_path.join("notes/renamed.md")).unwrap();
    let shrunk = assert_agree(&root, &mut memo, "after a member was removed");
    assert_eq!(
        shrunk, after,
        "removing the addition restores the root — a vanished member never lingers in the fold"
    );
}

/// The drawer lane: the memo one process saved is the memo the next loads.
/// A round-tripped memo serves the same root with zero reads — the amortising
/// claim the `mrd sql` door makes across invocations.
#[test]
fn a_memo_carried_across_processes_serves_the_same_root_without_reading() {
    let tmp = tempfile::tempdir().unwrap();
    let root_path = tmp.path();
    write(root_path, "a.md", "# A\n");
    write(root_path, "notes/b.md", "# B\n");
    let root = WorkspaceRoot(root_path.to_path_buf());

    let mut first = DigestMemo::new();
    let minted = assert_agree(&root, &mut first, "first process, cold");
    let bytes = first.to_bytes();

    let mut second = DigestMemo::from_bytes(&bytes);
    let served = assert_agree(&root, &mut second, "second process, loaded memo");
    assert_eq!(served, minted);
    assert_eq!(
        second.misses(),
        0,
        "a loaded memo over an unchanged tree reads nothing"
    );
    assert_eq!(second.hits(), 2);
}

/// A member that cannot be read refuses the memoized fold exactly as it
/// refuses the snapshot — same failure, naming the same member — so a
/// fingerprint-only caller never mints a root over a corpus the snapshotting
/// caller would have rejected. Cold (the member is a miss) is the reachable
/// case: a warm memo serves an unreadable-but-unmoved member from its row,
/// which is the memo's disclosed evidence standing, not a divergence.
#[cfg(unix)]
#[test]
fn an_unreadable_member_refuses_the_memoized_fold_like_the_snapshot() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let root_path = tmp.path();
    write(root_path, "notes/plan.md", "# Plan\n");
    write(root_path, "notes/sealed.md", "# Sealed\n");
    let sealed = root_path.join("notes/sealed.md");
    std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o000)).unwrap();
    let root = WorkspaceRoot(root_path.to_path_buf());

    let snapshot = fs::domain_snapshot(&root);
    let memoized = fs::domain_fingerprint_memoized(&root, &mut DigestMemo::new());
    // Running as root defeats the permission bit; then neither refuses and
    // the gate has nothing to say. Assert the agreement either way.
    match (snapshot, memoized) {
        (Err(s), Err(m)) => {
            assert_eq!(s.kind(), m.kind(), "same io::ErrorKind");
            assert_eq!(
                s.to_string(),
                m.to_string(),
                "same rendered refusal, naming the same member"
            );
        }
        (Ok((_, s)), Ok(m)) => {
            assert_eq!(m, s, "readable after all — the roots still agree");
        }
        (s, m) => panic!("instruments disagreed about refusing: {s:?} vs {m:?}"),
    }

    std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o644)).unwrap();
}
