//! Card `engine-io-error-names-no-path`: a corpus walk that cannot enumerate
//! a directory must name WHICH directory, engine-side.
//!
//! Receipt (2026-08-24 ≈01:39–01:46Z, field-notes-sessions): one mode-000
//! session directory made every face refuse `io_error: Permission denied (os
//! error 13)` — workspace-wide, with no path. The failing entry can be
//! anywhere under the root, so the refusal sent an outside seat on a
//! six-minute discriminator hunt (disk-vs-engine, cross-corpus, full-tree
//! effective-`open()`, daemon `ps`) for a fact one word of path collapses to
//! one `ls`.
//!
//! `crates/fs` is the only place the fact exists: `std::fs` errors carry no
//! path, so by the time a refusal reaches the wire seam or the CLI the
//! directory is already unrecoverable. These gates pin the mint, not a face —
//! every face that prints the error inherits it.
//!
//! The locus rides in [`fs::CorpusMemberError`], the crate's existing
//! corpus-scoped refusal, so `kind()` still steers the `NotFound` control-flow
//! splits and `fs::corpus_member_error` reads the locus structurally.

use std::io;
use std::path::Path;

use fs::guard::{GuardError, StepGuard};
use fs::{DomainCache, WorkspaceRoot};

fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, contents).unwrap();
}

/// A workspace with markdown at three depths, so a locked directory is
/// genuinely MID-corpus: readable members sit both above and below it.
fn workspace() -> (tempfile::TempDir, WorkspaceRoot) {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    write(&ws, "a.md", "# A\n");
    write(&ws, "notes/b.md", "# B\n");
    write(&ws, "notes/locked/c.md", "# C\n");
    write(&ws, "notes/locked/deeper/d.md", "# D\n");
    let root = WorkspaceRoot(std::fs::canonicalize(&ws).unwrap());
    (tmp, root)
}

/// An unreadable directory that becomes readable again however the test
/// leaves — returned value, failed assertion, or panic.
///
/// Restoring on the happy path only is not enough: a failing assertion
/// between the chmod and the restore would leak a mode-000 directory into the
/// build tree, which `TempDir`'s own cleanup then cannot remove. The next run
/// inherits litter caused by a test failure, which is the worst moment to add
/// a second, unrelated symptom.
struct Locked<'a>(&'a Path);

impl<'a> Locked<'a> {
    /// Take the directory's permissions away, and PROVE the instrument bites
    /// before any assertion rests on it. A `chmod 000` is a no-op for a
    /// privileged user, and a test that silently passes because its
    /// precondition never held is the green log that means nothing.
    fn new(dir: &'a Path) -> Locked<'a> {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o000)).unwrap();
        assert!(
            std::fs::read_dir(dir).is_err(),
            "PRECONDITION FAILED: {} is still listable at mode 000 — this suite \
             cannot run as a privileged user, and passing here would test nothing",
            dir.display()
        );
        Locked(dir)
    }

    /// Give the permissions back early, for a test that must observe the
    /// engine serving the restored tree.
    fn unlock(&self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(self.0, std::fs::Permissions::from_mode(0o755));
    }
}

impl Drop for Locked<'_> {
    fn drop(&mut self) {
        self.unlock();
    }
}

/// Every walk the engine runs over the corpus, named, so a gate that passes
/// on one entry point cannot be read as a claim about the others.
fn refusals(root: &WorkspaceRoot) -> Vec<(&'static str, io::Error)> {
    let mut out: Vec<(&'static str, io::Error)> = Vec::new();
    out.push(("walk", fs::walk(root).expect_err("the addressable walk")));
    let domain = fs::domain::Domain::load(root).expect("domain config loads");
    out.push((
        "hash_domain",
        fs::hash_domain(root, &domain).expect_err("the hash-domain walk"),
    ));
    out.push((
        "declined_markdown",
        fs::declined_markdown(root).expect_err("the declined walk"),
    ));
    out.push((
        "dot_declined_markdown",
        fs::dot_declined_markdown(root).expect_err("the dot-declined walk"),
    ));
    out.push((
        "DomainCache::root",
        DomainCache::new()
            .root(root)
            .expect_err("the cached observation — the production hot path"),
    ));
    match StepGuard::open(root) {
        Err(GuardError::Io(e)) => out.push(("StepGuard::open", e)),
        other => panic!("the guarded walk must refuse with Io: {other:?}"),
    }
    out
}

/// The card's first Done bullet, engine-side: every corpus walk names the
/// directory it could not list. One gate per walk — the incident hit the
/// cached observation, but a fix proven on one walk says nothing about the
/// six beside it.
#[test]
fn every_corpus_walk_names_the_directory_it_could_not_list() {
    let (_tmp, root) = workspace();
    let locked = root.0.join("notes/locked");
    let locked = Locked::new(&locked);

    for (walk, err) in refusals(&root) {
        let said = err.to_string();
        assert!(
            said.contains("notes/locked"),
            "{walk} refused without naming the directory — the caller must hunt \
             the whole tree from outside: {said:?}"
        );
        assert_eq!(
            err.kind(),
            io::ErrorKind::PermissionDenied,
            "{walk}: the mint must preserve kind() — the NotFound splits across \
             this crate steer control flow, not just messages: {said:?}"
        );
    }

    drop(locked);
}

/// Negative control for the gate above: on a HEALTHY tree these walks
/// succeed, so the assertions discriminate rather than matching a refusal
/// every run would produce anyway.
#[test]
fn a_healthy_tree_mints_no_listing_refusal() {
    let (_tmp, root) = workspace();
    let domain = fs::domain::Domain::load(&root).expect("domain config loads");
    fs::walk(&root).expect("the addressable walk serves a healthy tree");
    fs::hash_domain(&root, &domain).expect("the hash-domain walk serves it");
    DomainCache::new()
        .root(&root)
        .expect("the cached observation serves it");
    let _guard = StepGuard::open(&root).expect("the guarded walk opens on it");
}

/// The recursion clause. Both walks recurse, and a bare `?` on the recursive
/// call re-throws the child's error through every ancestor frame: wrapping on
/// the way UP would name the workspace root (today's behaviour, renamed) or
/// stack one path per frame. The mint belongs at the leaf — the deepest frame
/// is the only one that knows which directory refused.
#[test]
fn the_locus_is_the_innermost_failing_directory_not_the_root() {
    let (_tmp, root) = workspace();
    let locked = root.0.join("notes/locked/deeper");
    let locked = Locked::new(&locked);

    for (walk, err) in refusals(&root) {
        let said = err.to_string();
        assert!(
            said.contains("notes/locked/deeper"),
            "{walk} must name the innermost failing directory: {said:?}"
        );
        assert_eq!(
            said.matches("cannot be listed").count(),
            1,
            "{walk} wrapped once per frame — a refusal carries ONE locus, not a \
             nest of them: {said:?}"
        );
    }

    drop(locked);
}

/// The locus is readable STRUCTURALLY, not only by scraping prose: a face
/// that wants the offending path pulls it off the typed refusal, exactly as
/// it already does for an unreadable member.
#[test]
fn the_offending_directory_is_readable_off_the_typed_refusal() {
    let (_tmp, root) = workspace();
    let locked = root.0.join("notes/locked");
    let locked = Locked::new(&locked);

    let err = DomainCache::new().root(&root).expect_err("the hot path");
    let member = fs::corpus_member_error(&err)
        .unwrap_or_else(|| panic!("the refusal is corpus-scoped and typed: {err}"));
    assert_eq!(member.member, "notes/locked");
    assert_eq!(member.kind, io::ErrorKind::PermissionDenied);
    assert!(
        member.condition.starts_with("cannot be listed"),
        "the condition states the LISTING failed, not a member read: {:?}",
        member.condition
    );

    drop(locked);
}

/// The card's second Done bullet, engine half: recovery needs no restart.
/// The SAME `DomainCache` — the resident memo a warm engine holds across
/// calls — serves the corpus again once the directory is readable, with no
/// new cache and no process restart. A cache that had to be thrown away
/// would make every permission blip an engine bounce.
#[test]
fn recovery_needs_no_restart_the_same_warm_cache_serves_again() {
    let (_tmp, root) = workspace();
    let locked = root.0.join("notes/locked");
    let mut cache = DomainCache::new();

    let before = cache.root(&root).expect("warm the cache on a healthy tree");

    let locked = Locked::new(&locked);
    let refused = cache.root(&root).expect_err("the locked tree refuses");
    assert!(
        refused.to_string().contains("notes/locked"),
        "the refusal names the directory even on a warm cache: {refused}"
    );

    drop(locked);
    let after = cache
        .root(&root)
        .expect("the SAME cache serves again — recovery needs no restart");
    assert_eq!(
        before, after,
        "an unchanged tree folds to the same root across the refusal — the \
         permission blip left no residue in the memo"
    );
}

/// A locked ROOT has no workspace-relative path to name, and a refusal that
/// names nothing is the defect this card is about. It spells itself `.`.
#[test]
fn a_locked_root_names_itself_rather_than_nothing() {
    let (_tmp, root) = workspace();
    let locked = Locked::new(&root.0);

    let err = fs::walk(&root).expect_err("a locked root refuses");
    assert_eq!(
        err.to_string(),
        "the corpus cannot be served: . cannot be listed (Permission denied (os error 13))",
        "the root spells itself, so no refusal names an empty locus"
    );

    drop(locked);
}
