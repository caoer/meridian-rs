//! U11: guarded `lock_write` path (locate/create, EOF placement, CAS, flock,
//! atomic replace, corrupt fail-loud). Format law lives in `crates/lock`.
//!
//!
//!
//!

use wire::{ErrorCode, NodeRev, Path as WPath, Recovery};
use wire_serve::write::{LockWriteArgs, lock_write};

const PAGE: &str = "---\ntitle: Pinning\n---\n# Claims\n\nthe claim body ^c1\n";

fn ws(files: &[(&str, &str)]) -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    for (path, content) in files {
        let full = dir.path().join(path);
        if let Some(p) = full.parent() {
            std::fs::create_dir_all(p).expect("mkdir");
        }
        std::fs::write(&full, content).expect("fixture");
    }
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    (dir, root)
}

fn read(root: &fs::WorkspaceRoot, rel: &str) -> String {
    std::fs::read_to_string(root.0.join(rel)).expect("read back")
}

fn file_rev(root: &fs::WorkspaceRoot, rel: &str) -> NodeRev {
    let doc = fs::load(root, std::path::Path::new(rel)).expect("load");
    NodeRev(doc.root.node_rev.0)
}

/// Real root-node fingerprint (`fp1.span2.b3.<64hex>`), not synthetic.
///
fn minted_fingerprint(root: &fs::WorkspaceRoot) -> String {
    let doc = fs::load(root, std::path::Path::new("page.md")).expect("load");
    model::fingerprint::fingerprint(&doc, &doc.root)
        .expect("page.md has content")
        .into_string()
}

/// Sample R4 pin (object, hash, `path: []`, real fingerprint).
///
///
fn sample_lock(root: &fs::WorkspaceRoot) -> lock::Lock {
    let mut l = lock::Lock::new();
    l.upsert_pin(lock::PinEntry::new(
        "page",
        "9ae3f1c0deadbeef9ae3f1c0deadbeef9ae3f1c0",
        lock::Selector::Path(Vec::new()),
        &minted_fingerprint(root),
    ));
    l
}

fn args(root: &fs::WorkspaceRoot, l: &lock::Lock, dry: bool) -> LockWriteArgs {
    LockWriteArgs {
        id: None,
        path: WPath("page.md".into()),
        lock: l.clone(),
        actor: Some("engine:test".into()),
        now: Some("2026-07-24T12:00:00Z".into()),
        if_root: None,
        if_file_rev: file_rev(root, "page.md"),
        dry,
    }
}

fn fence_count(raw: &str) -> usize {
    raw.matches("```meridian-lock").count()
}

/// Birth: one lock at EOF (placement law), round-trip, root advances.
///
///
#[test]
fn birth_lands_at_eof_and_round_trips() {
    let (_d, root) = ws(&[("page.md", PAGE)]);
    let l = sample_lock(&root);

    let out = lock_write(&root, None, &args(&root, &l, false)).expect("birth lands");
    assert!(out.created, "no lock existed — this is a birth");
    assert_ne!(
        out.root_before,
        out.root_after
            .clone()
            .expect("real write advances the root"),
        "lock-is-content: the write moves the world root"
    );

    let after = read(&root, "page.md");
    // Placement law: original content intact at the head, exactly one blank
    // line separator, block at EOF, file ends with one terminator.
    assert!(after.starts_with(PAGE), "original page bytes untouched");
    assert!(
        after[PAGE.len()..].starts_with('\n'),
        "one blank line separates content from the lock block"
    );
    assert!(
        after.ends_with("```\n"),
        "block closes the file with one terminator"
    );
    assert_eq!(fence_count(&after), 1, "exactly one meridian-lock block");

    // Round-trip: the committed block finds and parses back to what was written.
    let doc = fs::load(&root, std::path::Path::new("page.md")).expect("re-load");
    let found = lock::find(&doc)
        .expect("clean lock state")
        .expect("the block is found");
    assert_eq!(found.lock, l, "disk round-trip preserves the lock object");

    // The write reports a real whole-file rev transition (the lock is content).
    assert_ne!(
        out.file_rev_before, out.file_rev_after,
        "lock-is-content: the page's whole-file rev moved"
    );
}

/// Update: in-place replace, still one block; surrounding content byte-identical.
///
///
#[test]
fn update_replaces_in_place_exactly_one_block() {
    let (_d, root) = ws(&[("page.md", PAGE)]);
    let mut l = sample_lock(&root);
    lock_write(&root, None, &args(&root, &l, false)).expect("birth");

    // A SECOND, distinct claim on the same object: R4's anchor form, a `^id` as
    // the sole path element. `upsert_pin` keys on (object, selector), so this
    // appends beside the whole-body pin rather than replacing it.
    l.upsert_pin(lock::PinEntry::new(
        "page",
        "9ae3f1c0deadbeef9ae3f1c0deadbeef9ae3f1c0",
        lock::Selector::Path(vec!["^c1".into()]),
        "fp1.span2.b3.0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    ));
    let out = lock_write(&root, None, &args(&root, &l, false)).expect("update lands");
    assert!(!out.created, "the block existed — replaced in place");

    let after = read(&root, "page.md");
    assert_eq!(fence_count(&after), 1, "still exactly one block");
    assert!(
        after.starts_with(PAGE),
        "content around the block untouched"
    );
    let doc = fs::load(&root, std::path::Path::new("page.md")).expect("re-load");
    let found = lock::find(&doc).expect("clean").expect("found");
    assert_eq!(found.lock, l, "the updated lock (two pins) round-trips");
}

/// Same lock rewrite is byte-stable (file rev / root hold).
///
///
#[test]
fn rewriting_same_lock_is_byte_stable() {
    let (_d, root) = ws(&[("page.md", PAGE)]);
    let l = sample_lock(&root);
    lock_write(&root, None, &args(&root, &l, false)).expect("birth");
    let bytes_1 = read(&root, "page.md");

    let out = lock_write(&root, None, &args(&root, &l, false)).expect("idempotent rewrite");
    assert_eq!(read(&root, "page.md"), bytes_1, "byte-stable");
    assert_eq!(
        out.file_rev_before, out.file_rev_after,
        "identical bytes — the whole-file rev holds still"
    );
}

/// CAS: stale `if_file_rev` → `cas_mismatch`; page untouched.
///
#[test]
fn cas_drift_refuses_citing_revs() {
    let (_d, root) = ws(&[("page.md", PAGE)]);
    let l = sample_lock(&root);
    let stale = args(&root, &l, false); // rev captured NOW

    // The page drifts under the plan.
    std::fs::write(root.0.join("page.md"), format!("{PAGE}\ndrift\n")).expect("drift");
    let live = file_rev(&root, "page.md");

    let err = lock_write(&root, None, &stale).expect_err("stale rev must refuse");
    assert_eq!(err.code, ErrorCode::CasMismatch);
    assert_eq!(err.recovery, Recovery::Refresh);
    assert_eq!(err.expected.as_ref(), Some(&stale.if_file_rev), "rev READ");
    assert_eq!(err.actual.as_ref(), Some(&live), "rev FOUND");
    assert_eq!(fence_count(&read(&root, "page.md")), 0, "nothing landed");
}

/// Dry: no disk write; still reports after-rev and `created`.
///
#[test]
fn dry_writes_nothing() {
    let (_d, root) = ws(&[("page.md", PAGE)]);
    let l = sample_lock(&root);
    let out = lock_write(&root, None, &args(&root, &l, true)).expect("dry reports");
    assert!(out.dry && out.created);
    assert!(out.root_after.is_none() && out.committed.is_none());
    assert_ne!(
        out.file_rev_before, out.file_rev_after,
        "after-rev computed"
    );
    assert_eq!(read(&root, "page.md"), PAGE, "no byte landed");
}

/// D9: held flock → `workspace_busy`/retry; nothing lands.
///
#[test]
fn held_flock_refuses_workspace_busy() {
    let (_d, root) = ws(&[("page.md", PAGE)]);
    let l = sample_lock(&root);
    let a = args(&root, &l, false);
    let _held = fs::WriteLock::acquire(&root).expect("test holds the flock");
    let err = lock_write(&root, None, &a).expect_err("must refuse busy");
    assert_eq!(err.code, ErrorCode::WorkspaceBusy);
    assert_eq!(err.recovery, Recovery::Retry);
    assert_eq!(read(&root, "page.md"), PAGE, "nothing landed");
}

/// Corrupt: two hand-written lock blocks → teaching `bad_request` (#8 §3).
///
///
#[test]
fn two_hand_written_blocks_refuse_loud() {
    let forged =
        format!("{PAGE}\n```meridian-lock\nversion: 1\n```\n\n```meridian-lock\nversion: 1\n```\n");
    let (_d, root) = ws(&[("page.md", &forged)]);
    let l = lock::Lock::new();
    let a = LockWriteArgs {
        id: None,
        path: WPath("page.md".into()),
        lock: l,
        actor: None,
        now: None,
        if_root: None,
        if_file_rev: file_rev(&root, "page.md"),
        dry: false,
    };
    let err = lock_write(&root, None, &a).expect_err("two blocks must refuse");
    assert_eq!(err.code, ErrorCode::BadRequest);
    assert!(
        err.message
            .as_deref()
            .is_some_and(|m| m.contains("sole") && m.contains("meridian-lock")),
        "teaching refusal names the sole-writer law: {:?}",
        err.message
    );
    assert_eq!(
        read(&root, "page.md"),
        forged,
        "the corrupt state is untouched"
    );
}

/// Missing page → `file_not_found` (env).
#[test]
fn missing_page_is_file_not_found() {
    let (_d, root) = ws(&[]);
    let a = LockWriteArgs {
        id: None,
        path: WPath("ghost.md".into()),
        lock: lock::Lock::new(),
        actor: None,
        now: None,
        if_root: None,
        if_file_rev: NodeRev("deadbeefdeadbeef".into()),
        dry: false,
    };
    let err = lock_write(&root, None, &a).expect_err("missing page refuses");
    assert_eq!(err.code, ErrorCode::FileNotFound);
}
