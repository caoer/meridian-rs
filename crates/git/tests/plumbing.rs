//! Fixture-repo gates for the git plumbing organ (plan unit S5).
//!
//! Every gate here runs a REAL `git` against a real repository built in a
//! tmpdir — the plumbing's whole job is to report what git says, so a mocked
//! git would prove nothing. The composed three-state gate drives this crate's
//! facts into `receipt::anchor::ObjectAnchor`, the classifier that consumes
//! them, so both halves of S5 are proven together.

use std::fs;
use std::path::Path;
use std::process::Command;

use git::{GitFail, Repo};
use receipt::anchor::{ObjectAnchor, ObjectAnchorFacts};
use tempfile::TempDir;

/// Run a fixture-setup git command, isolated from the operator's global and
/// system config so the fixture is the same repository on every machine.
fn git_in(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("LC_ALL", "C")
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "fixture `git {}` failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .expect("git stdout is utf-8")
        .trim()
        .to_owned()
}

/// An initialized repository with a commit identity, no hooks, no signing.
fn empty_repo() -> TempDir {
    let dir = tempfile::tempdir().expect("tmpdir");
    git_in(dir.path(), &["-c", "init.defaultBranch=main", "init", "-q"]);
    git_in(dir.path(), &["config", "user.name", "s5 fixture"]);
    git_in(dir.path(), &["config", "user.email", "s5@fixture.invalid"]);
    git_in(dir.path(), &["config", "commit.gpgsign", "false"]);
    dir
}

fn write(dir: &Path, name: &str, body: &str) {
    fs::write(dir.join(name), body).expect("write fixture file");
}

fn commit(dir: &Path, message: &str) {
    git_in(dir, &["add", "-A"]);
    git_in(dir, &["commit", "-q", "--no-verify", "-m", message]);
}

/// **Gate — the three anchoring states, each against a real repository.**
///
/// - `anchored`: committed, so a ref reaches the blob.
/// - `pending-anchor`: written eagerly (`hash-object -w`), reachable from
///   nothing — the vibe blob.
/// - `never-anchored`: hashed read-only, so the object database never saw it.
#[test]
fn three_anchor_states_against_a_fixture_repo() {
    let repo_dir = empty_repo();
    let root = repo_dir.path();
    let repo = Repo::at(root);

    write(root, "anchored.md", "committed body\n");
    commit(root, "anchor it");

    write(root, "pending.md", "vibe body\n");
    write(root, "never.md", "never written body\n");

    let anchored_oid = repo.blob_oid(Path::new("anchored.md")).expect("blob oid");
    let pending_oid = repo
        .write_blob(Path::new("pending.md"))
        .expect("eager write");
    let never_oid = repo.blob_oid(Path::new("never.md")).expect("blob oid");
    assert_ne!(anchored_oid, pending_oid);
    assert_ne!(pending_oid, never_oid);

    // ONE reachable set for the whole check, ONE batched presence query.
    let reachable = repo.reachable_objects().expect("reachable set");
    let oids = [
        anchored_oid.as_str(),
        pending_oid.as_str(),
        never_oid.as_str(),
    ];
    let info = repo.object_info(&oids).expect("batched presence");

    let states: Vec<ObjectAnchor> = oids
        .iter()
        .zip(&info)
        .map(|(oid, present)| {
            ObjectAnchor::classify(&ObjectAnchorFacts {
                object_present: present.is_some(),
                reachable_from_commit: reachable.contains(oid),
            })
        })
        .collect();

    assert_eq!(
        states,
        vec![
            ObjectAnchor::Anchored,
            ObjectAnchor::PendingAnchor,
            ObjectAnchor::NeverAnchored,
        ],
        "committed / eagerly-written / never-written blobs classify as the three states"
    );

    // The states are distinguishable by their own facts, not just by order.
    assert!(reachable.contains(&anchored_oid));
    assert!(!reachable.contains(&pending_oid));
    assert!(!reachable.contains(&never_oid));
    assert!(info[0].is_some() && info[1].is_some() && info[2].is_none());
    assert_eq!(info[1].as_ref().expect("pending info").kind, "blob");
    assert_eq!(
        info[1].as_ref().expect("pending info").size,
        "vibe body\n".len() as u64,
        "the batched answer carries the byte size the vibe-debt gauge sums"
    );

    // Committing the pending blob anchors it — the state is a fact about the
    // world, not a label stored anywhere.
    commit(root, "anchor the vibe blob");
    let reachable = repo.reachable_objects().expect("reachable set");
    assert_eq!(
        ObjectAnchor::classify(&ObjectAnchorFacts {
            object_present: true,
            reachable_from_commit: reachable.contains(&pending_oid),
        }),
        ObjectAnchor::Anchored
    );
}

/// **Gate — the clean-filter pitfall.** `git hash-object` over a work-tree path
/// applies the path's `.gitattributes` filters, so the oid it returns is the
/// blob git actually stores (`HEAD:<path>`). Hashing the raw bytes instead
/// (`--no-filters`) yields a DIFFERENT id that appears in no object database —
/// every reachability answer over it would say never-anchored forever.
#[test]
fn blob_oid_matches_the_committed_blob_under_a_clean_filter() {
    let repo_dir = empty_repo();
    let root = repo_dir.path();
    git_in(root, &["config", "filter.upper.clean", "tr a-z A-Z"]);
    write(root, ".gitattributes", "*.md filter=upper\n");
    write(root, "note.md", "vibe\n");
    commit(root, "filtered");

    let repo = Repo::at(root);
    let ours = repo.blob_oid(Path::new("note.md")).expect("blob oid");
    let committed = git_in(root, &["rev-parse", "HEAD:note.md"]);
    assert_eq!(
        ours, committed,
        "the operational blob sha IS the blob git stored"
    );

    let raw = git_in(root, &["hash-object", "--no-filters", "--", "note.md"]);
    assert_ne!(
        ours, raw,
        "the fixture really has a clean filter in play, so the assertion above is not vacuous"
    );

    // And the stored blob is the FILTERED content — the assumption named in the
    // crate docs, stated as an assertion.
    assert_eq!(git_in(root, &["cat-file", "-p", &ours]), "VIBE");
}

/// **Gate — honest degradation, git absent.** Every entry point returns the
/// typed [`GitFail::Spawn`]; not one of them invents a sha.
#[test]
fn absent_git_degrades_honestly_and_mints_no_sha() {
    let repo_dir = empty_repo();
    let root = repo_dir.path();
    write(root, "note.md", "body\n");
    let repo = Repo::at_with_program(root, "definitely-not-git-b7f3");

    let oid = "0".repeat(40);
    let fails: Vec<GitFail> = vec![
        repo.blob_oid(Path::new("note.md")).unwrap_err(),
        repo.write_blob(Path::new("note.md")).unwrap_err(),
        repo.reachable_objects().expect_err("no reachable set"),
        repo.object_exists(&oid).unwrap_err(),
        repo.object_info(&[oid.as_str()]).expect_err("no info"),
    ];
    for fail in &fails {
        assert!(
            matches!(fail, GitFail::Spawn { .. }),
            "an absent git is GitFail::Spawn, got {fail:?}"
        );
    }
}

/// **Gate — honest degradation, not a repository.** Including the read-only
/// blob compute: git alone would answer it (content-addressing needs no repo),
/// but that oid carries no attribute context and could never be anchored, so
/// the handle refuses instead of minting it.
#[test]
fn a_non_repository_root_degrades_honestly_and_mints_no_sha() {
    let dir = tempfile::tempdir().expect("tmpdir");
    let root = dir.path();
    write(root, "note.md", "body\n");
    let repo = Repo::at(root);

    let oid = "0".repeat(40);
    let fails: Vec<GitFail> = vec![
        repo.blob_oid(Path::new("note.md")).unwrap_err(),
        repo.write_blob(Path::new("note.md")).unwrap_err(),
        repo.reachable_objects().expect_err("no reachable set"),
        repo.object_exists(&oid).unwrap_err(),
    ];
    for fail in &fails {
        assert!(
            matches!(fail, GitFail::NotARepo { .. }),
            "a non-repository root is GitFail::NotARepo, got {fail:?}"
        );
    }
}

/// **Gate — the reachable set is computed ONCE per check, never per blob.**
/// The handle runs a shim that logs every argv before exec'ing the real git, so
/// this counts actual invocations: classifying three blobs spends exactly one
/// `rev-list` and one `cat-file`.
#[cfg(unix)]
#[test]
fn one_rev_list_and_one_cat_file_per_check_not_per_blob() {
    use std::os::unix::fs::PermissionsExt;

    let repo_dir = empty_repo();
    let root = repo_dir.path();
    write(root, "a.md", "a\n");
    write(root, "b.md", "b\n");
    write(root, "c.md", "c\n");
    commit(root, "three files");

    let log = repo_dir.path().join("git-argv.log");
    let shim = repo_dir.path().join("git-shim.sh");
    fs::write(
        &shim,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> {}\nexec git \"$@\"\n",
            log.display()
        ),
    )
    .expect("write shim");
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).expect("chmod shim");

    let repo = Repo::at_with_program(root, &shim);
    let oids: Vec<String> = ["a.md", "b.md", "c.md"]
        .iter()
        .map(|f| repo.blob_oid(Path::new(f)).expect("blob oid"))
        .collect();

    // The check itself: one reachable set, one batched presence query, three
    // classifications.
    let reachable = repo.reachable_objects().expect("reachable set");
    let refs: Vec<&str> = oids.iter().map(String::as_str).collect();
    let info = repo.object_info(&refs).expect("batched presence");
    for (oid, present) in oids.iter().zip(&info) {
        assert_eq!(
            ObjectAnchor::classify(&ObjectAnchorFacts {
                object_present: present.is_some(),
                reachable_from_commit: reachable.contains(oid),
            }),
            ObjectAnchor::Anchored
        );
    }

    let argv = fs::read_to_string(&log).expect("shim log");
    let count = |needle: &str| argv.lines().filter(|l| l.contains(needle)).count();
    assert_eq!(count("rev-list"), 1, "ONE rev-list per check:\n{argv}");
    assert_eq!(count("cat-file"), 1, "ONE cat-file per check:\n{argv}");
    assert_eq!(
        count("--objects --all"),
        1,
        "the reachable set is computed once, not per blob:\n{argv}"
    );
}

/// The batched presence query answers in input order, and a repository with no
/// commits has an empty reachable set (not a failure).
#[test]
fn object_info_answers_in_input_order_and_an_empty_repo_reaches_nothing() {
    let repo_dir = empty_repo();
    let root = repo_dir.path();

    let empty = Repo::at(root).reachable_objects().expect("empty set");
    assert!(empty.is_empty(), "a repo with no commits reaches nothing");
    assert_eq!(empty.len(), 0);

    write(root, "one.txt", "one\n");
    write(root, "two.txt", "two two\n");
    commit(root, "two files");
    let repo = Repo::at(root);
    let one = repo.blob_oid(Path::new("one.txt")).expect("oid");
    let two = repo.blob_oid(Path::new("two.txt")).expect("oid");
    let absent = "0".repeat(40);

    let info = repo
        .object_info(&[two.as_str(), absent.as_str(), one.as_str()])
        .expect("batched");
    assert_eq!(info.len(), 3);
    assert_eq!(info[0].as_ref().expect("two present").size, 8);
    assert!(info[1].is_none(), "the absent object answers None");
    assert_eq!(info[2].as_ref().expect("one present").size, 4);
}

/// A ref name is refused before git is spawned — `cat-file` would resolve
/// `HEAD` and answer a question nobody asked (a false `anchored`).
#[test]
fn a_ref_name_is_refused_as_a_bad_oid() {
    let repo_dir = empty_repo();
    write(repo_dir.path(), "note.md", "body\n");
    commit(repo_dir.path(), "one");
    let repo = Repo::at(repo_dir.path());

    let fail = repo.object_exists("HEAD").expect_err("HEAD is not an oid");
    assert!(
        matches!(&fail, GitFail::BadOid { oid } if oid == "HEAD"),
        "expected BadOid, got {fail:?}"
    );
    // …and the same repository answers the real oid honestly.
    let oid = repo.blob_oid(Path::new("note.md")).expect("oid");
    assert!(repo.object_exists(&oid).expect("presence"));
}

/// **Gate — ONE oid, THREE object stores, THREE anchoring states (U13).**
///
/// The ratified cross-root addressing §4: *"the blob-anchoring check runs against
/// THAT root's git repo — six roots, six object stores, one law."* This is that
/// sentence as an assertion, and it is deliberately built on the SAME object id:
/// identical bytes content-address identically, so the oid cannot be what
/// distinguishes the answers — **only which store was asked can.** A check that
/// silently ran against one ambient repository could not produce three different
/// states here, whatever it did with the root prefix.
///
/// Both arms of the per-root claim ride together (S3-R8(c)): the blob is found
/// anchored where it IS anchored, and it degrades to `never-anchored` in the
/// store that does not hold it — never borrowing another store's answer.
#[test]
fn one_oid_three_stores_three_anchor_states() {
    // Identical bytes in three separate repositories.
    const BODY: &str = "one law, several object stores\n";
    let anchored_dir = empty_repo();
    let pending_dir = empty_repo();
    let absent_dir = empty_repo();
    for dir in [&anchored_dir, &pending_dir, &absent_dir] {
        write(dir.path(), "x.md", BODY);
    }

    // Store 1: committed — a ref reaches the blob.
    commit(anchored_dir.path(), "anchor it here");
    // Store 2: the eager vibe write — in the database, reachable from nothing.
    let pending = Repo::at(pending_dir.path());
    let pending_oid = pending.write_blob(Path::new("x.md")).expect("eager write");
    // Store 3: hashed read-only — the object database never saw it.
    let absent = Repo::at(absent_dir.path());
    let absent_oid = absent.blob_oid(Path::new("x.md")).expect("read-only oid");

    let anchored = Repo::at(anchored_dir.path());
    let anchored_oid = anchored.blob_oid(Path::new("x.md")).expect("oid");
    assert_eq!(
        (anchored_oid.as_str(), pending_oid.as_str()),
        (absent_oid.as_str(), absent_oid.as_str()),
        "identical bytes are ONE oid — the stores are the only variable",
    );
    let oid = anchored_oid;

    // One law, applied three times against three handles. Both facts of each
    // pair come from the handle they were gathered with.
    let classify = |repo: &Repo| {
        let reachable = repo.reachable_objects().expect("reachable set");
        let present = repo.object_exists(&oid).expect("presence");
        ObjectAnchor::classify(&ObjectAnchorFacts {
            object_present: present,
            reachable_from_commit: reachable.contains(&oid),
        })
    };
    assert_eq!(classify(&anchored), ObjectAnchor::Anchored);
    assert_eq!(classify(&pending), ObjectAnchor::PendingAnchor);
    assert_eq!(classify(&absent), ObjectAnchor::NeverAnchored);
}

/// The handle is a handle: two roots, two handles, one code path (seam rule
/// D12). Nothing here discovers or caches "the" repository, so a later
/// per-root git repo plugs in by constructing another `Repo`.
#[test]
fn two_repo_handles_answer_independently() {
    let first = empty_repo();
    let second = empty_repo();
    write(first.path(), "note.md", "first body\n");
    commit(first.path(), "first");
    write(second.path(), "note.md", "second body\n");
    commit(second.path(), "second");

    let a = Repo::at(first.path());
    let b = Repo::at(second.path());
    let a_oid = a.blob_oid(Path::new("note.md")).expect("oid");
    let b_oid = b.blob_oid(Path::new("note.md")).expect("oid");
    assert_ne!(a_oid, b_oid);

    assert!(a.object_exists(&a_oid).expect("presence"));
    assert!(
        !b.object_exists(&a_oid).expect("presence"),
        "the other root's blob is absent here — no shared or ambient repo state"
    );
    assert_eq!(a.root(), first.path());
    assert_eq!(b.root(), second.path());
}

// ── the INDEX: what a commit is about to record (F1) ─────────────────────────

/// **Gate — a fully-staged tree reports NO divergence.** The empty answer is what
/// tells a caller the two intervals coincide, so a spurious entry here would make
/// every ordinary commit pay for a second assessment (and, worse, read as a
/// divergence that is not one).
#[test]
fn a_fully_staged_tree_reports_no_index_divergence() {
    let dir = empty_repo();
    write(dir.path(), "page.md", "# Page\n\nbody\n");
    commit(dir.path(), "seed");
    let repo = Repo::at(dir.path());
    assert!(
        repo.staged_divergence().expect("ask git").is_empty(),
        "nothing is modified: the index and the worktree are the same interval"
    );

    write(dir.path(), "page.md", "# Page\n\nedited\n");
    git_in(dir.path(), &["add", "page.md"]);
    assert!(
        repo.staged_divergence().expect("ask git").is_empty(),
        "staged and identical: still one interval"
    );
}

/// **Gate — the INDEX's bytes come back when the worktree is restored.** This is
/// F1's exact state: staged forgery, worktree restored byte-exact. A caller reading
/// the worktree sees the governed bytes; only this answer carries what a commit
/// would record.
#[test]
fn the_index_bytes_come_back_when_the_worktree_is_restored() {
    let dir = empty_repo();
    write(dir.path(), "page.md", "governed\n");
    commit(dir.path(), "seed");
    write(dir.path(), "page.md", "FORGED\n");
    git_in(dir.path(), &["add", "page.md"]);
    write(dir.path(), "page.md", "governed\n");

    let divergence = Repo::at(dir.path()).staged_divergence().expect("ask git");
    assert_eq!(divergence.len(), 1, "one path diverges: {divergence:?}");
    let (path, content) = &divergence[0];
    assert_eq!(path, "page.md");
    assert_eq!(
        content.as_deref(),
        Some(&b"FORGED\n"[..]),
        "the bytes are the INDEX's — the ones `git show HEAD:page.md` would print \
         after the commit"
    );
}

/// **Gate — a staged DELETION is reported as a removal, even with the worktree copy
/// still on disk.** `git rm --cached` leaves the file present and untracked, so
/// `diff-files` cannot see it: without the index-vs-HEAD half of the query the
/// commit would drop a governed page while the check still read it.
#[test]
fn a_staged_deletion_is_reported_as_a_removal_with_the_file_still_on_disk() {
    let dir = empty_repo();
    write(dir.path(), "page.md", "governed\n");
    commit(dir.path(), "seed");
    git_in(dir.path(), &["rm", "-q", "--cached", "page.md"]);
    assert!(
        dir.path().join("page.md").exists(),
        "the fixture IS the subject: the worktree copy is still there"
    );

    let divergence = Repo::at(dir.path()).staged_divergence().expect("ask git");
    assert_eq!(
        divergence,
        vec![("page.md".to_owned(), None)],
        "None means THIS COMMIT REMOVES IT, whatever the worktree still holds"
    );
}

/// **Gate — an unborn HEAD does not fault.** A fresh repository has no HEAD to diff
/// against, and a caller must get an answer rather than an error: the fence runs on
/// the very first commit too.
#[test]
fn an_unborn_head_answers_without_faulting() {
    let dir = empty_repo();
    write(dir.path(), "page.md", "first\n");
    git_in(dir.path(), &["add", "page.md"]);
    assert!(
        Repo::at(dir.path())
            .staged_divergence()
            .expect("an unborn HEAD is not a fault")
            .is_empty(),
        "staged and identical to the worktree: one interval"
    );
}

/// **Gate — a non-repository degrades TYPED, never as a silent empty answer.**
/// `NotARepo` is what lets a caller say "there is no index here" instead of "the
/// index agrees", which are different facts.
#[test]
fn a_non_repository_returns_not_a_repo_rather_than_an_empty_answer() {
    let dir = tempfile::tempdir().expect("tmpdir");
    write(dir.path(), "page.md", "loose\n");
    assert!(
        matches!(
            Repo::at(dir.path()).staged_divergence(),
            Err(GitFail::NotARepo { .. })
        ),
        "a directory that is not a repository has no index to compare"
    );
}

/// **Gate (U22) — the batch stream carries the OID beside the bytes, and the two
/// halves belong to the same object.**
///
/// The lost-pin repair recovers content from history and must then name the
/// durable object carrying it. Re-hashing the recovered bytes locally would ask
/// a different question (`hash-object --path` applies the clean filter), so the
/// id has to be git's own — read from the header git already printed. This gate
/// holds it against `rev-parse`, the independent answer.
#[test]
fn the_batch_stream_answers_with_gits_own_oid_for_the_bytes_it_returns() {
    let repo_dir = empty_repo();
    let root = repo_dir.path();
    let repo = Repo::at(root);

    write(root, "page.md", "first\n");
    commit(root, "one");
    let first = git_in(root, &["rev-parse", "HEAD"]);
    write(root, "page.md", "second\n");
    commit(root, "two");
    let second = git_in(root, &["rev-parse", "HEAD"]);

    let specs = [
        format!("{first}:page.md"),
        format!("{second}:page.md"),
        format!("{second}:absent.md"),
    ];
    let refs: Vec<&str> = specs.iter().map(String::as_str).collect();
    let answers = repo.blobs_with_oids_at(&refs).expect("batch");

    assert_eq!(answers.len(), 3, "one answer per spec, in input order");
    let one = answers[0].as_ref().expect("the first version is readable");
    let two = answers[1].as_ref().expect("the second version is readable");
    assert_eq!(one.bytes, b"first\n", "the bytes are the recorded ones");
    assert_eq!(two.bytes, b"second\n");
    assert_eq!(
        one.oid,
        git_in(root, &["rev-parse", &format!("{first}:page.md")]),
        "the oid is git's own answer for that spec"
    );
    assert_eq!(
        two.oid,
        git_in(root, &["rev-parse", &format!("{second}:page.md")])
    );
    assert_ne!(one.oid, two.oid, "and it varies with the object");
    assert!(
        answers[2].is_none(),
        "a spec that resolves to nothing is a real answer, not a failure"
    );
}

/// **Gate (U22) — `blobs_at` and `blobs_with_oids_at` are ONE read.** The bytes
/// half is a projection of the pair, so the two entry points cannot drift into
/// answering differently about one spec.
#[test]
fn the_bytes_face_is_a_projection_of_the_oid_bearing_one() {
    let repo_dir = empty_repo();
    let root = repo_dir.path();
    let repo = Repo::at(root);
    write(root, "page.md", "body\n");
    commit(root, "one");
    let head = git_in(root, &["rev-parse", "HEAD"]);

    let specs = [format!("{head}:page.md"), format!("{head}:absent.md")];
    let refs: Vec<&str> = specs.iter().map(String::as_str).collect();
    let bytes = repo.blobs_at(&refs).expect("bytes");
    let pairs = repo.blobs_with_oids_at(&refs).expect("pairs");
    let projected: Vec<Option<Vec<u8>>> = pairs.into_iter().map(|p| p.map(|b| b.bytes)).collect();
    assert_eq!(bytes, projected);
}
