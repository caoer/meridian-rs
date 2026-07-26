//! **U15 — criterion 5: the pre-commit fence, RUN AS AN INSTALLED HOOK IN A REAL
//! REPOSITORY.** Never an asserted function: every leg here installs the real
//! hook through the real `mrd hook install`, then drives a real `git commit` and
//! reads what git did.
//!
//! # The three arms, and why the third is not optional (S3-R8(c))
//! A guard proven only by what it blocks is indistinguishable from a guard that
//! blocks everything, so the refusals are paired with the acceptance:
//!
//! - [`the_fence_accepts_a_commit_whose_writes_were_all_governed`] — the
//!   **ACCEPTANCE**. Without it, a verifier running *pin → out-of-band edit →
//!   commit → rejected* records PASS caused by a false red rather than by the
//!   guard, which is structurally how stage 2's criterion 3 failed.
//! - [`the_fence_refuses_a_commit_carrying_an_out_of_band_write`] — refusal one.
//! - [`the_fence_refuses_a_commit_that_would_strand_an_anchor_obligation`] —
//!   refusal two, **as AMENDED by S3-R71(a)**: the arm binds to the **ORPHAN**
//!   state — no ref reaches the pinned blob AND the file no longer hashes to it,
//!   so nothing holds it and no commit will.
//!
//! # Why that amendment is load-bearing for this file
//! U14 measured one pin's ordinary governed life: `never-anchored` before
//! `git add`; **`pending-anchor` AT HOOK TIME — because `git add` writes the blob
//! and the commit that would anchor it is the one being fenced**; `anchored`
//! after. So pending-anchor is the normal state of every governed commit at the
//! moment the fence runs, and an arm wired to it would refuse the lifecycle
//! rather than a defect. **A lifecycle state is not a defect.** The acceptance
//! arm is what catches that, and it caught it against the criterion's own
//! author-level wording.
//!
//! # The discriminator is the PAIR, never one leg's exit
//! Every refusal leg here has the acceptance leg as its control: the same
//! installed hook, the same binary, the same repository shape — one corpus
//! commits and the other does not. An exit code on its own would be satisfied by
//! a hook that refuses everything, including the one that refuses because `mrd`
//! is missing.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// The binary every drive goes through — the real CLI, never a library call.
fn mrd_bin() -> PathBuf {
    std::env::var_os("MRD_BIN")
        .map_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_mrd")), PathBuf::from)
}

/// A sandbox whose `mrd` is OURS and whose caches are its own.
///
/// The `bin/` directory holds a symlink to the binary under test and is the
/// ONLY thing on the hook's `PATH`. Without it the installed hook would find
/// whatever `mrd` the operator has deployed and this file would measure a
/// different engine — the same trap the brief names for `MERIDIAN_SIDECAR_BIN`.
/// `XDG_CACHE_HOME` and `HOME` point inside the sandbox for the producer-side
/// reason: a non-deployed `mrd` that registers in the shared `~/.cache/meridian`
/// becomes the host's resident daemon and every other read dials it.
struct Sandbox {
    tmp: tempfile::TempDir,
    bin: PathBuf,
    cache_home: PathBuf,
    home: PathBuf,
}

/// The system directories a `git commit` needs to run at all. The sandbox `bin/`
/// is prepended, so OUR `mrd` shadows any deployed one — the ordering is the
/// isolation, and it is the same ordering an operator's own `PATH` has.
const SYSTEM_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

fn sandbox() -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let bin = tmp.path().join("bin");
    let home = tmp.path().join("home");
    let cache_home = tmp.path().join("xdg-cache");
    std::fs::create_dir_all(&bin).expect("bin");
    std::fs::create_dir_all(&home).expect("home");
    std::os::unix::fs::symlink(mrd_bin(), bin.join("mrd")).expect("link mrd onto the hook's PATH");
    Sandbox {
        tmp,
        bin,
        cache_home,
        home,
    }
}

impl Sandbox {
    fn command(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut c = Command::new(mrd_bin());
        c.args(args)
            .current_dir(cwd)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env_remove("MERIDIAN_WORKSPACE");
        c
    }

    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        self.command(cwd, args).output().expect("spawn mrd")
    }

    fn run_stdin(&self, cwd: &Path, args: &[&str], stdin_bytes: &str) -> Output {
        let mut child = self
            .command(cwd, args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn mrd");
        child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(stdin_bytes.as_bytes())
            .expect("write stdin");
        child.wait_with_output().expect("wait mrd")
    }

    /// A real `git commit`, run the way an operator runs it: the hook fires, our
    /// `mrd` is the only one on `PATH`, and the caches are the sandbox's.
    fn commit(&self, ws: &Path, message: &str, extra_env: &[(&str, &str)]) -> Output {
        let mut c = Command::new("git");
        c.arg("-C")
            .arg(ws)
            .args(["commit", "-m", message])
            .env("PATH", self.hook_path())
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env_remove("MERIDIAN_WORKSPACE");
        for (k, v) in extra_env {
            c.env(k, v);
        }
        c.output().expect("spawn git commit")
    }

    /// The `PATH` the hook runs under: ours first, then the system dirs `git`
    /// itself lives in.
    fn hook_path(&self) -> String {
        format!("{}:{SYSTEM_PATH}", self.bin.display())
    }

    /// A commit whose hook runs with a `PATH` that has git but NOT `mrd` — the
    /// "`mrd` is not on PATH at commit time" rescue row, driven rather than
    /// described.
    ///
    /// The precondition is asserted rather than assumed: if a deployed `mrd`
    /// happened to live in a system directory this leg would silently measure
    /// the wrong thing, so it checks that nothing named `mrd` is resolvable
    /// there before it draws any conclusion.
    fn commit_without_mrd(&self, ws: &Path, message: &str) -> Output {
        for dir in SYSTEM_PATH.split(':') {
            assert!(
                !Path::new(dir).join("mrd").exists(),
                "this leg needs a PATH with no `mrd` on it, and {dir} has one — \
                 the assert below would be measuring a different failure"
            );
        }
        Command::new("git")
            .arg("-C")
            .arg(ws)
            .args(["commit", "-m", message])
            .env("PATH", SYSTEM_PATH)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .output()
            .expect("spawn git commit")
    }

    /// A real git repository that is also a meridian workspace, carrying the
    /// pinnable source / claim / plan corpus U32 and U14 both cut from.
    fn corpus(&self, name: &str) -> PathBuf {
        let ws = self.tmp.path().join(name);
        std::fs::create_dir_all(&ws).expect("mkdir");
        git_ok(&ws, &["init", "-q"]);
        git_ok(&ws, &["config", "user.email", "u15@example.invalid"]);
        git_ok(&ws, &["config", "user.name", "u15"]);
        write(
            &ws,
            "source.md",
            "# Source\n\n## Guideline\n\nthe pinned body\n\n## Notes\n\nnot pinned.\n",
        );
        write(&ws, "claim.md", "# Claim\n\nwe rely on the guideline.\n");
        write(&ws, "plan.md", "# Plan\n\n## Goals\n\nalpha beta\n");
        let init = self.run(&ws, &["init"]);
        assert!(init.status.success(), "mrd init: {}", said(&init));
        git_ok(&ws, &["add", "-A"]);
        git_ok(&ws, &["commit", "-qm", "corpus"]);
        ws
    }

    /// Install the fence and assert the STATE CHANGE, not the exit code (R40):
    /// **every door** git dispatches for a commit built from a prepared index
    /// carries an executable fence. Returns `pre-commit`, which is the door the
    /// arms in this file drive.
    ///
    /// The three names are literals rather than a read of `FENCED_HOOKS`: an
    /// assertion parameterised by the set it measures cannot fail when that set
    /// shrinks.
    fn install_fence(&self, ws: &Path) -> PathBuf {
        let out = self.run(ws, &["hook", "install"]);
        assert_eq!(
            out.status.code(),
            Some(0),
            "hook install on a clean repo: {}",
            said(&out)
        );
        let hooks = common_dir(ws).join("hooks");
        for name in ["pre-commit", "pre-merge-commit", "pre-applypatch"] {
            let hook = hooks.join(name);
            assert!(
                hook.exists(),
                "R40 — install exited 0 without writing {}",
                hook.display()
            );
            let mode = std::os::unix::fs::PermissionsExt::mode(
                &std::fs::metadata(&hook).expect("stat hook").permissions(),
            );
            assert_eq!(
                mode & 0o111,
                0o111,
                "chmod+x is the install, not a decoration on it: {name} mode {mode:o}"
            );
        }
        hooks.join("pre-commit")
    }
}

// ── ARM 1 — THE ACCEPTANCE (S3-R8(c)) ────────────────────────────────────────

/// **The fence ACCEPTS a commit whose writes were ALL governed.**
///
/// This is the leg without which every refusal below is unattributable. The
/// corpus is written through the shipped write doors (`mrd pin`, then `mrd put`),
/// so the journal's last row's `root_after` IS the live tree and `mrd check`
/// exits 0 — and the installed hook lets a real `git commit` through.
///
/// It is also where the S3-R71 amendment is visible: at the moment this hook
/// runs, the pinned blob is `pending-anchor` — `git add` wrote it and the commit
/// that would anchor it is the one being fenced. The commit succeeds anyway,
/// because a lifecycle state is not a defect.
#[test]
fn the_fence_accepts_a_commit_whose_writes_were_all_governed() {
    let sb = sandbox();
    let ws = sb.corpus("accepts");
    sb.install_fence(&ws);

    let pin = sb.run(&ws, &["pin", "claim.md", "source.md#Source/Guideline"]);
    assert_eq!(pin.status.code(), Some(0), "mrd pin: {}", said(&pin));
    let put = sb.run_stdin(
        &ws,
        &["put", "plan.md"],
        &goals_match("alpha", "alpha prime"),
    );
    assert_eq!(put.status.code(), Some(0), "mrd put: {}", said(&put));

    let check = sb.run(&ws, &["check"]);
    assert_eq!(
        check.status.code(),
        Some(0),
        "the corpus this arm needs: every byte written by a meridian writer, so \
         check is green BEFORE the hook is asked: {}",
        said(&check)
    );

    let before = head_count(&ws);
    git_ok(&ws, &["add", "-A"]);
    let commit = sb.commit(&ws, "governed", &[]);
    assert!(
        commit.status.success(),
        "THE ACCEPTANCE — a fully governed commit must pass the fence, or the \
         guard is one that blocks everything: {}",
        said(&commit)
    );
    assert_eq!(
        head_count(&ws),
        before + 1,
        "R40 — the state change, not the exit: git actually recorded a commit"
    );
}

// ── ARM 2 — the OUT-OF-BAND refusal ──────────────────────────────────────────

/// **The fence REFUSES a commit carrying an out-of-band write** — the one door
/// the engine cannot see, and the fence's whole reason for existing.
///
/// The control is the acceptance leg above: same hook, same binary, same
/// repository shape. What differs is one plain shell rewrite of a governed file.
#[test]
fn the_fence_refuses_a_commit_carrying_an_out_of_band_write() {
    let sb = sandbox();
    let ws = sb.corpus("refuses-out-of-band");
    sb.install_fence(&ws);

    // Govern the corpus first, so the journal has a current baseline and the
    // refusal below is attributable to the out-of-band edit and nothing else.
    let put = sb.run_stdin(
        &ws,
        &["put", "plan.md"],
        &goals_match("alpha", "alpha prime"),
    );
    assert_eq!(put.status.code(), Some(0), "mrd put: {}", said(&put));
    assert_eq!(
        sb.run(&ws, &["check"]).status.code(),
        Some(0),
        "the baseline is current before the edit — otherwise this leg would be \
         measuring a false red"
    );

    // The out-of-band write: no meridian writer touched this.
    write(&ws, "plan.md", "# Plan\n\n## Goals\n\nrewritten by hand\n");

    let before = head_count(&ws);
    git_ok(&ws, &["add", "-A"]);
    let commit = sb.commit(&ws, "out of band", &[]);
    assert!(
        !commit.status.success(),
        "THE ASSERT IS THE REFUSAL — the fence exists for exactly this write: {}",
        said(&commit)
    );
    assert_eq!(
        head_count(&ws),
        before,
        "R40 — refused means git recorded NO commit, not merely that a hook printed"
    );
    let text = said(&commit);
    assert!(
        text.contains("meridian fence: refusing"),
        "the refusal cites the fence, so an operator knows what stopped them: {text}"
    );
    assert!(
        text.contains("MRD_HOOK_FORCE=1") && text.contains("--no-verify"),
        "and it NAMES THE ESCAPE — a guard whose exit an operator cannot find is \
         one they disable by deleting the tool: {text}"
    );
}

// ── ARM 2b — the INDEX refusal (F1): the interval the commit spans ───────────

/// **The fence REFUSES a commit whose INDEX carries an out-of-band write, with
/// the worktree restored byte-exact** — F1, the false green the shipped fence
/// shipped with.
///
/// # The sequence is the reviewer's, and the byte-exactness is the whole gate
/// `mrd pin` writes a `^facts` anchor into the target, so an undo that
/// reconstructs the file by hand does not restore it — and the fence then refuses
/// for the WRONG REASON (the worktree really did drift), which reads exactly like
/// the defect being absent. **The comfortable answer came from the inexact
/// restore.** Here the governed bytes are CAPTURED and written back verbatim, so
/// the worktree is provably identical and the index is provably not — both
/// asserted below before the commit is attempted.
///
/// The control is [`the_fence_accepts_a_commit_whose_writes_were_all_governed`]:
/// same hook, same binary, same corpus shape, one interval's bytes different.
#[test]
fn the_fence_refuses_a_commit_whose_index_carries_an_out_of_band_write() {
    let sb = sandbox();
    let ws = sb.corpus("refuses-index");
    sb.install_fence(&ws);

    let pin = sb.run(&ws, &["pin", "claim.md", "source.md#Source/Guideline"]);
    assert_eq!(pin.status.code(), Some(0), "mrd pin: {}", said(&pin));
    git_ok(&ws, &["add", "-A"]);
    let seeded = sb.commit(&ws, "governed baseline", &[]);
    assert!(
        seeded.status.success(),
        "the baseline commit must land, or this leg measures a fence that \
         refuses everything: {}",
        said(&seeded)
    );
    assert_eq!(
        sb.run(&ws, &["check"]).status.code(),
        Some(0),
        "BASELINE: the corpus is green before the forge"
    );

    // The byte-exact capture — after `mrd pin`, so it carries the `^facts` anchor.
    let governed = std::fs::read(ws.join("source.md")).expect("capture governed bytes");

    // (1) forge the PINNED section out of band, and prove the verb can see it.
    let forged = String::from_utf8(governed.clone())
        .expect("utf-8 fixture")
        .replace("the pinned body", "FORGED out of band");
    std::fs::write(ws.join("source.md"), &forged).expect("forge");
    assert_eq!(
        sb.run(&ws, &["check"]).status.code(),
        Some(1),
        "the forge is visible while it is in the WORKTREE — this is the sensitivity \
         the leg below shows is interval-bound, not absent"
    );

    // (2) stage it, the way anyone stages their work; (3) restore the worktree.
    git_ok(&ws, &["add", "source.md"]);
    std::fs::write(ws.join("source.md"), &governed).expect("restore byte-exact");

    assert_eq!(
        std::fs::read(ws.join("source.md")).expect("read back"),
        governed,
        "THE WORKTREE IS BYTE-EXACT — if this fails the leg below is measuring an \
         imperfect undo and would 'refuse' for the wrong reason"
    );
    let staged_bytes = git_bytes(&ws, &["show", ":source.md"]);
    assert_ne!(
        staged_bytes, governed,
        "and THE INDEX IS NOT — the divergence is the fixture"
    );
    assert!(
        String::from_utf8_lossy(&staged_bytes).contains("FORGED out of band"),
        "the index carries the forgery itself, not merely different bytes"
    );

    // The assert: the fence speaks about what is BEING COMMITTED.
    let before = head_count(&ws);
    let commit = sb.commit(&ws, "the index carries the forge", &[]);
    assert!(
        !commit.status.success(),
        "THE ASSERT IS THE REFUSAL — git commits the INDEX, so a fence that only \
         reads the worktree passes forged bytes into history: {}",
        said(&commit)
    );
    assert_eq!(
        head_count(&ws),
        before,
        "R40 — refused means git recorded NO commit"
    );
    // And history is the oracle: whatever the hook printed, the bytes are what
    // matter. `git show HEAD:source.md` is where F1 was caught.
    assert!(
        !String::from_utf8_lossy(&git_bytes(&ws, &["show", "HEAD:source.md"]))
            .contains("FORGED out of band"),
        "*** FORGED BYTES IN HISTORY *** — the defect this arm exists for"
    );
    let text = said(&commit);
    assert!(
        text.contains("meridian fence: refusing"),
        "the refusal cites the fence: {text}"
    );
    assert!(
        text.contains("staged"),
        "AND IT NAMES THE INTERVAL (S3-R29): a refusal an operator cannot locate \
         is one they cannot act on, and the worktree here is clean: {text}"
    );
    // S3-R104 — ASSERT THE CAUSE, because this arm has two independent refusing
    // detectors and a later fix could delete one of them without failing anything.
    // The pin plane is the one that names the forgery; the journal plane's grey is
    // the second, and it survives the interval predicate only because a forged tree
    // matches no receipt. Measured post-predicate-fix: both are present.
    assert!(
        text.contains("content-drifted"),
        "the refusal must name WHAT it saw — a pin whose target drifted — or this arm \
         could later pass on a refusal that has nothing to do with the forgery: {text}"
    );
}

/// **The fence still ACCEPTS the same corpus once the forgery is out of BOTH
/// intervals** — the redden pair's other arm, over the exact repository the leg
/// above refused.
///
/// Without it, the refusal above is satisfied by a fence that refuses every
/// commit with anything staged at all — which is what widening an interval most
/// plausibly breaks, and it would brick every governed commit on the fleet.
#[test]
fn the_widened_interval_still_accepts_a_governed_commit_over_the_same_corpus() {
    let sb = sandbox();
    let ws = sb.corpus("accepts-index");
    sb.install_fence(&ws);

    let pin = sb.run(&ws, &["pin", "claim.md", "source.md#Source/Guideline"]);
    assert_eq!(pin.status.code(), Some(0), "mrd pin: {}", said(&pin));
    let governed = std::fs::read(ws.join("source.md")).expect("governed bytes");

    // The refusal, first, so the acceptance below is measured over a corpus this
    // fence genuinely stops — never over one that would have committed anyway.
    let forged = String::from_utf8(governed.clone())
        .expect("utf-8 fixture")
        .replace("the pinned body", "FORGED out of band");
    std::fs::write(ws.join("source.md"), &forged).expect("forge");
    git_ok(&ws, &["add", "source.md"]);
    std::fs::write(ws.join("source.md"), &governed).expect("restore byte-exact");
    assert!(
        !sb.commit(&ws, "refused", &[]).status.success(),
        "the control: this tree is refused"
    );

    // Take the forgery out of the INDEX too, and stage the rest of the governed
    // write with it: `mrd pin` wrote BOTH the lock in `claim.md` and the anchor in
    // `source.md`, and a commit recording one without the other records a tree no
    // receipt vouches for. An ordinary `git add -A` is what a governed commit does.
    git_ok(&ws, &["add", "-A"]);
    assert_eq!(
        git_bytes(&ws, &["show", ":source.md"]),
        governed,
        "the fixture: both intervals now carry the governed bytes"
    );
    let before = head_count(&ws);
    let commit = sb.commit(&ws, "governed", &[]);
    assert!(
        commit.status.success(),
        "THE ACCEPTANCE — the widened interval must let a governed commit through: {}",
        said(&commit)
    );
    assert_eq!(
        head_count(&ws),
        before + 1,
        "R40 — the state change, not the exit"
    );
    assert!(
        String::from_utf8_lossy(&git_bytes(&ws, &["show", "HEAD:source.md"]))
            .contains("the pinned body"),
        "and the governed bytes are what landed"
    );
}

// ── ARM 3 — the ORPHAN refusal (criterion 5 as AMENDED, S3-R71(a)) ───────────

/// **The fence REFUSES a commit that would strand an anchor obligation** — the
/// ORPHAN: no ref reaches the pinned blob AND the file no longer hashes to it,
/// so nothing holds it and no commit will.
///
/// The corpus isolates the anchoring finding from the content one: the
/// out-of-band edit lands in `## Notes`, OUTSIDE the pinned `## Guideline`, so the
/// claim plane stays green and the `objects:` blob — which is whole-file — moves.
#[test]
fn the_fence_refuses_a_commit_that_would_strand_an_anchor_obligation() {
    let sb = sandbox();
    let ws = sb.corpus("refuses-orphan");
    sb.install_fence(&ws);

    let pin = sb.run(
        &ws,
        &["pin", "claim.md", "source.md#Source/Guideline", "--vibe"],
    );
    assert_eq!(pin.status.code(), Some(0), "pin --vibe: {}", said(&pin));
    let recorded = git_out(&ws, &["hash-object", "--", "source.md"]);
    assert_eq!(
        git_out(&ws, &["cat-file", "-t", &recorded]),
        "blob",
        "R40 — the eager --vibe write put the blob in the object database"
    );

    // Move the file OUTSIDE the pinned section: the blob is now held by nothing
    // and no commit of this file will ever anchor it.
    let body = std::fs::read_to_string(ws.join("source.md")).expect("source");
    write(
        &ws,
        "source.md",
        &body.replace("not pinned.", "not pinned, and edited out of band.\n"),
    );
    assert_ne!(
        git_out(&ws, &["hash-object", "--", "source.md"]),
        recorded,
        "R40 — the state this leg turns on: the file no longer hashes to the \
         recorded blob"
    );
    assert!(
        git_out(&ws, &["rev-list", "--objects", "--all"])
            .lines()
            .all(|line| !line.starts_with(&recorded)),
        "and no ref reaches it either — it is held by nothing: THE ORPHAN"
    );

    let before = head_count(&ws);
    git_ok(&ws, &["add", "-A"]);
    let commit = sb.commit(&ws, "orphan", &[]);
    assert!(
        !commit.status.success(),
        "THE ASSERT IS THE REFUSAL — an obligation nothing will satisfy: {}",
        said(&commit)
    );
    assert_eq!(head_count(&ws), before, "R40 — no commit was recorded");
    assert!(
        said(&commit).contains("meridian fence: refusing"),
        "citing its why: {}",
        said(&commit)
    );
}

// ── THE ACCEPTANCE THE FIRST INTERVAL FIX BROKE (found by fbd87f1c) ──────────

/// **A legitimately-governed PARTIAL STAGE is ACCEPTED** — `git add` followed by a
/// further governed write, which on a fleet doing continuous governed writes is the
/// common path, and which `git add -p` reaches on every hunk it leaves behind.
///
/// # The corpus contains no out-of-band write at all
/// `mrd pin` (governed) → `mrd put` edit ONE (governed) → `git add notes.md` →
/// `mrd put` edit TWO (governed). The index now holds the EXACT post-edit-ONE
/// bytes: a state the engine itself wrote and journaled. **The deployed engine
/// accepts this commit (`2 -> 3`); the first interval fix refused it (`2 -> 2`) —
/// a FALSE RED, and the suite did not catch it because no arm asserted the
/// acceptance direction for partial staging.**
///
/// # Why it refused, measured, because the fix turns on it
/// The staged snapshot is INTERNALLY INCONSISTENT and legitimately so: `git add
/// notes.md` stages content without the journal, so the staged tree folds to
/// `^r-000002`'s `root_after` while the staged journal still ends at `^r-000001`.
/// Dating a snapshot against its own journal's LAST row therefore reports
/// *"something advanced the tree that the journal does not account for"* — when the
/// engine wrote every byte of both states.
///
/// **The discriminator is that a legitimate intermediate state matches SOME receipt
/// in the record, and a forgery matches NONE.** The refusal arms above are this
/// arm's control: they must keep refusing in the same run, and
/// [`a_staged_journal_forgery_is_refused_though_the_journal_is_root_excluded`] is
/// the one that proves the widening did not open the journal door.
#[test]
fn a_governed_partial_stage_is_accepted_though_the_journal_has_moved_past_it() {
    let sb = sandbox();
    let ws = sb.corpus("governed-partial-stage");
    sb.install_fence(&ws);

    let pin = sb.run(&ws, &["pin", "claim.md", "source.md#Source/Guideline"]);
    assert_eq!(pin.status.code(), Some(0), "mrd pin: {}", said(&pin));
    git_ok(&ws, &["add", "-A"]);
    assert!(
        sb.commit(&ws, "governed baseline", &[]).status.success(),
        "the baseline commit lands"
    );

    // Governed write ONE, then the ORDINARY partial stage.
    let one = sb.run_stdin(&ws, &["put", "plan.md"], &goals_match("alpha", "alpha-ONE"));
    assert_eq!(one.status.code(), Some(0), "put ONE: {}", said(&one));
    let after_one = std::fs::read(ws.join("plan.md")).expect("post-ONE bytes");
    git_ok(&ws, &["add", "plan.md"]);

    // Governed write TWO — the journal advances past what is staged.
    let two = sb.run_stdin(&ws, &["put", "plan.md"], &goals_match("beta", "beta-TWO"));
    assert_eq!(two.status.code(), Some(0), "put TWO: {}", said(&two));

    // THE FIXTURE'S OWN PRECONDITIONS, so a pass cannot come from a corpus that
    // never reached the state under test.
    assert_eq!(
        git_bytes(&ws, &["show", ":plan.md"]),
        after_one,
        "the index holds the EXACT post-ONE governed bytes — byte-identical, so \
         nothing here is forged"
    );
    assert_ne!(
        std::fs::read(ws.join("plan.md")).expect("worktree"),
        after_one,
        "and the worktree has moved on, or the two intervals do not differ and this \
         arm is vacuous"
    );
    assert_eq!(
        sb.run(&ws, &["check"]).status.code(),
        Some(0),
        "every byte on disk is governed: the worktree interval is green and cannot \
         be what refuses"
    );

    let before = head_count(&ws);
    let commit = sb.commit(&ws, "commit the staged governed state", &[]);
    assert!(
        commit.status.success(),
        "THE ACCEPTANCE — every byte in this commit was written by a governed door, \
         so refusing it is a FALSE RED that brakes `git add` + any further governed \
         write, the common path on a fleet: {}",
        said(&commit)
    );
    assert_eq!(
        head_count(&ws),
        before + 1,
        "R40 — the state change, not the exit"
    );
    assert!(
        String::from_utf8_lossy(&git_bytes(&ws, &["show", "HEAD:plan.md"])).contains("alpha-ONE"),
        "and what landed is the governed intermediate state that was staged"
    );
}

/// **A MIXED two-receipt stage is REFUSED — the predicate is over the TREE, not
/// per path** (ratified S3-R103, superseding R102(a)'s per-path phrasing).
///
/// Two governed writes to two files, then only the SECOND file staged: the index
/// carries `b.md`'s new bytes beside `a.md`'s HEAD bytes — **a combination no
/// governed write ever produced**, though every individual path's bytes came from
/// some receipt. A per-path predicate would accept it; the tree-form predicate
/// refuses, and refusing is the safe direction because the commit would record a
/// tree the record cannot date.
///
/// This arm is what separates the two readings, so a later narrowing to per-path
/// fails here rather than shipping.
#[test]
fn a_mixed_two_receipt_stage_is_refused_because_the_predicate_is_over_the_tree() {
    let sb = sandbox();
    let ws = sb.corpus("mixed-two-receipt");
    // The two files are committed BEFORE the fence is installed, exactly as
    // `corpus()` commits its own: writing them is an out-of-band write, and the
    // fence would refuse that commit for the right reason and mask this arm.
    write(&ws, "a.md", "# A\n\n## Log\n\nalpha\n");
    write(&ws, "b.md", "# B\n\n## Log\n\nbeta\n");
    git_ok(&ws, &["add", "-A"]);
    git_ok(&ws, &["commit", "-qm", "two files"]);
    sb.install_fence(&ws);

    let edit = |file: &str, heading: &str, old: &str, new: &str| {
        let batch = serde_json::to_string(&serde_json::json!([{
            "target": {"hpath": [{"h": heading}, {"h": "Log"}]},
            "edit": {"match": {"old": old, "new": new}},
        }]))
        .expect("edits json");
        let out = sb.run_stdin(&ws, &["put", file], &batch);
        assert_eq!(out.status.code(), Some(0), "mrd put {file}: {}", said(&out));
    };
    edit("a.md", "A", "alpha", "alpha-ONE"); // receipt N   : tree (a@1, b@0)
    edit("b.md", "B", "beta", "beta-TWO"); // receipt N+1 : tree (a@1, b@1)

    // Stage ONLY b.md: the index becomes (a@0 from HEAD, b@1) — a tree that was
    // never on disk and that no receipt recorded.
    git_ok(&ws, &["add", "b.md"]);
    assert!(
        !String::from_utf8_lossy(&git_bytes(&ws, &["show", ":a.md"])).contains("alpha-ONE"),
        "the fixture IS the subject: the index still holds a.md's PRE-write bytes"
    );
    assert!(
        String::from_utf8_lossy(&git_bytes(&ws, &["show", ":b.md"])).contains("beta-TWO"),
        "beside b.md's post-write bytes — each governed, the COMBINATION never produced"
    );

    let before = head_count(&ws);
    let commit = sb.commit(&ws, "mixed two-receipt stage", &[]);
    assert!(
        !commit.status.success(),
        "THE PREDICATE IS OVER THE TREE: every path's bytes came from a receipt, and \
         the tree they compose did not. A per-path reading would accept this: {}",
        said(&commit)
    );
    assert_eq!(head_count(&ws), before, "R40 — no commit was recorded");
}

// ── THE OTHER DOORS ONTO THE SAME INTERVAL GAP (F1 names them) ───────────────

/// **`git commit <pathspec>` records a THIRD interval, and asking git is what
/// makes the fence see it.**
///
/// `git commit -- <paths>` is `--only`: the commit tree is **HEAD plus the named
/// paths' worktree bytes**, ignoring the index. So a governed write to a file the
/// commit does not name is LEFT BEHIND, and the tree that lands is one no receipt
/// ever produced — while the worktree is entirely governed and a worktree check is
/// honestly green.
///
/// Here `mrd pin` writes `claim.md`'s lock and `source.md`'s anchor, `mrd put`
/// writes `plan.md`, nothing is staged, and the commit names `plan.md` alone. The
/// recorded tree carries the governed `plan.md` and **HEAD's lock-less
/// `claim.md`**.
///
/// **Measured on the deployed engine: the commit LANDS, `1 -> 2`.** It is refused
/// here because git materialises that tree into a temporary index and hands the
/// hook `GIT_INDEX_FILE` — so a fence that ASKS GIT what is being committed sees
/// the third interval for free, where one reading `.git/index` by hand would not.
///
/// *An earlier version of this arm staged a forgery and committed `-- source.md`.
/// It passed on the DEFECTIVE engine too — `--only` takes the worktree bytes for a
/// named path, so there was nothing to commit and git refused for its own reasons.
/// A gate that passes on the engine carrying the defect measures nothing.*
#[test]
fn a_pathspec_commit_is_refused_over_the_tree_git_builds_for_it() {
    let sb = sandbox();
    let ws = sb.corpus("pathspec");
    sb.install_fence(&ws);

    let pin = sb.run(&ws, &["pin", "claim.md", "source.md#Source/Guideline"]);
    assert_eq!(pin.status.code(), Some(0), "mrd pin: {}", said(&pin));
    let put = sb.run_stdin(
        &ws,
        &["put", "plan.md"],
        &goals_match("alpha", "alpha prime"),
    );
    assert_eq!(put.status.code(), Some(0), "mrd put: {}", said(&put));
    assert_eq!(
        sb.run(&ws, &["check"]).status.code(),
        Some(0),
        "THE FIXTURE'S POINT: every byte on disk is governed, so the worktree \
         interval is honestly green and cannot be what refuses below"
    );

    let before = head_count(&ws);
    let commit = Command::new("git")
        .arg("-C")
        .arg(&ws)
        .args(["commit", "-m", "pathspec: plan.md only", "--", "plan.md"])
        .env("PATH", sb.hook_path())
        .env("XDG_CACHE_HOME", &sb.cache_home)
        .env("HOME", &sb.home)
        .env_remove("MERIDIAN_WORKSPACE")
        .output()
        .expect("git commit <pathspec>");
    assert!(
        !commit.status.success(),
        "the tree this commit records — HEAD's lock-less claim.md beside a governed \
         plan.md — is one no receipt produced: {}",
        said(&commit)
    );
    assert_eq!(head_count(&ws), before, "R40 — no commit was recorded");
    assert!(
        String::from_utf8_lossy(&git_bytes(&ws, &["show", "HEAD:plan.md"])).contains("alpha beta"),
        "and HEAD still carries the pre-write plan.md — nothing landed"
    );
    // S3-R104 — THIS ARM HAS EXACTLY ONE REFUSING DETECTOR, so it names it. The pin
    // plane is GREEN here (no lock is drifted); the refusal is the journal plane
    // being unable to date a tree the record never produced. Measured after the
    // interval predicate widened: the grey survives, because HEAD's lock-less
    // claim.md beside a governed plan.md is a combination no receipt recorded. If a
    // later change turns that grey green, this assert fails instead of the arm
    // silently passing on nothing.
    let text = said(&commit);
    assert!(
        text.contains(check::GREY_CANNOT_ASSESS),
        "the ONE detector that refuses this arm must still be the one refusing: {text}"
    );
    assert!(
        text.contains("staged"),
        "and it must be the STAGED interval that says so — the worktree is green: {text}"
    );
}

/// **A write landing BETWEEN `git add` AND HOOK FIRE is refused** — the case F1
/// calls the normal one on a common dir with a live fleet, where another agent
/// writes while a commit is in flight.
///
/// The concurrent writer is simulated where it is deterministic: the bytes change
/// after staging and before the hook runs, which is exactly the state such a race
/// leaves. **A sleep-and-hope race would prove less** — it would pass when it
/// happened to lose.
///
/// Here the WORKTREE holds the ungoverned bytes and the index holds governed ones,
/// so the refusal comes from the worktree interval — the mirror of
/// [`the_fence_refuses_a_commit_whose_index_carries_an_out_of_band_write`], and the
/// reason BOTH intervals are assessed rather than the index replacing the
/// worktree.
#[test]
fn a_write_landing_after_git_add_is_refused_by_the_other_interval() {
    let sb = sandbox();
    let ws = sb.corpus("interleaved");
    sb.install_fence(&ws);
    let put = sb.run_stdin(
        &ws,
        &["put", "plan.md"],
        &goals_match("alpha", "alpha prime"),
    );
    assert_eq!(put.status.code(), Some(0), "mrd put: {}", said(&put));
    git_ok(&ws, &["add", "-A"]);

    // The other agent writes, after the stage, before the hook.
    write(
        &ws,
        "plan.md",
        "# Plan\n\n## Goals\n\nanother agent was here\n",
    );

    let before = head_count(&ws);
    let commit = sb.commit(&ws, "raced", &[]);
    assert!(
        !commit.status.success(),
        "a commit taken while an ungoverned write is on disk is not one anybody \
         vouched for: {}",
        said(&commit)
    );
    assert_eq!(head_count(&ws), before, "R40 — no commit was recorded");
}

/// **A forged RESERVED JOURNAL in the index is refused** — the journal is
/// root-EXCLUDED from the hash domain by named law, so it is the one file whose
/// bytes no fold covers, and the interval overlay has to pick it out by hand.
///
/// Staged forgery, worktree restored byte-exact: without the journal's own
/// interval being read from the index, the chain would be recomputed over the
/// worktree's honest journal while the commit recorded the forged one.
#[test]
fn a_staged_journal_forgery_is_refused_though_the_journal_is_root_excluded() {
    let sb = sandbox();
    let ws = sb.corpus("journal-index");
    sb.install_fence(&ws);
    // TWO governed writes, so the journal carries two rows: chain continuity is a
    // property of a row PAIR, and a single-row journal has no link to break.
    for (old, new) in [("alpha", "alpha prime"), ("beta", "beta prime")] {
        let put = sb.run_stdin(&ws, &["put", "plan.md"], &goals_match(old, new));
        assert_eq!(put.status.code(), Some(0), "mrd put: {}", said(&put));
    }
    git_ok(&ws, &["add", "-A"]);
    assert!(
        sb.commit(&ws, "governed baseline", &[]).status.success(),
        "the baseline commit lands"
    );

    let journal = ws.join("meridian").join("journal.md");
    let honest = std::fs::read(&journal).expect("the journal exists after a governed write");
    let rows = String::from_utf8(honest.clone()).expect("utf-8 journal");
    assert_eq!(
        rows.lines().filter(|l| l.contains("root_after=")).count(),
        2,
        "the fixture IS the subject: two rows, so there is a link to break"
    );
    // Break the LINK, not the last root: the last row's `root_after` still dates
    // the live tree, so the baseline is CURRENT and the refusal below is the chain
    // recompute finding a forged row — not a stale-baseline grey standing in for it.
    let first_after = field(rows.lines().next().expect("row 1"), "root_after=");
    let forged = rows.replace(
        &format!("root_before={first_after}"),
        "root_before=b3:0000000000000000000000000000000000000000000000000000000000000000",
    );
    assert_ne!(forged, rows, "the fixture must actually change the journal");
    std::fs::write(&journal, &forged).expect("forge the journal");
    git_ok(&ws, &["add", "meridian/journal.md"]);
    std::fs::write(&journal, &honest).expect("restore byte-exact");
    assert_eq!(
        std::fs::read(&journal).expect("read back"),
        honest,
        "the worktree journal is byte-exact"
    );

    let before = head_count(&ws);
    let commit = sb.commit(&ws, "the index carries a forged journal", &[]);
    assert!(
        !commit.status.success(),
        "the journal being outside the merkle domain is exactly why it needs its \
         own interval read: {}",
        said(&commit)
    );
    assert_eq!(head_count(&ws), before, "R40 — no commit was recorded");
}

/// **A VERSION SKEW — this fence, an OLDER `mrd` on PATH — fails CLOSED and names
/// the skew.**
///
/// The hook resolves its engine at commit time and never bakes one in (D11), so a
/// fence written by a new engine can be run against an old one. The old engine
/// answers `unknown flag: --staged` and exits 2, and **the fence then refuses every
/// commit** — measured on the deployed `980008813ff69586…` by the re-verifier's
/// harness, which puts no engine on `PATH`. That is the ordinary state of a
/// cutover, not a corner.
///
/// It must fail CLOSED — falling back to a plain `mrd check` would restore the F1
/// false green — but a bare "exited 2" leaves an operator with a bricked repository
/// and no idea why. The stand-in for the old engine is a script that answers
/// exactly as one does: exit 2 with `unknown flag`.
#[test]
fn a_fence_run_against_an_older_engine_refuses_and_names_the_skew() {
    let sb = sandbox();
    let ws = sb.corpus("version-skew");
    sb.install_fence(&ws);

    // An `mrd` that behaves like an engine predating `--staged`: exit 2 on the
    // flag. Everything else about the fence is unchanged, so the refusal below is
    // attributable to the skew and nothing else.
    let old = sb.tmp.path().join("old-bin");
    std::fs::create_dir_all(&old).expect("old bin dir");
    std::fs::write(
        old.join("mrd"),
        "#!/bin/sh\nfor a in \"$@\"; do\n  if [ \"$a\" = \"--staged\" ]; then\n    \
         echo 'mrd: unknown flag: --staged' >&2\n    exit 2\n  fi\ndone\nexit 0\n",
    )
    .expect("write old mrd");
    std::fs::set_permissions(
        old.join("mrd"),
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .expect("chmod");

    git_ok(&ws, &["add", "-A"]);
    let before = head_count(&ws);
    let commit = Command::new("git")
        .arg("-C")
        .arg(&ws)
        .args(["commit", "-m", "under an older engine"])
        .env("PATH", format!("{}:{SYSTEM_PATH}", old.display()))
        .env("XDG_CACHE_HOME", &sb.cache_home)
        .env("HOME", &sb.home)
        .env_remove("MERIDIAN_WORKSPACE")
        .output()
        .expect("git commit");
    assert!(
        !commit.status.success(),
        "FAIL CLOSED: an engine that cannot be asked about the index cannot vouch \
         for the commit, and falling back to the worktree question is the defect \
         this unit removed: {}",
        said(&commit)
    );
    assert_eq!(head_count(&ws), before, "R40 — no commit was recorded");
    let text = said(&commit);
    assert!(
        text.contains("OLDER than this fence"),
        "AND IT NAMES THE SKEW — a bricked repository with only `exited 2` to read \
         is how a guard gets deleted: {text}"
    );
    assert!(
        text.contains("command -v mrd") && text.contains("mrd hook status"),
        "and it names the two commands that DECIDE the cause, rather than accusing \
         (an unreadable workspace also exits 2): {text}"
    );
}

// ── the ESCAPES, and the state change that proves each one ───────────────────

/// **`--force` is honoured, in its hook spelling**, and so is git's own
/// `--no-verify`. Both run against the SAME corpus arm 2 proved refused, so each
/// one is measured as an escape from a real refusal rather than over a corpus
/// that would have committed anyway.
#[test]
fn both_escapes_carry_a_commit_the_fence_refused() {
    let sb = sandbox();
    let ws = sb.corpus("escapes");
    sb.install_fence(&ws);
    let put = sb.run_stdin(
        &ws,
        &["put", "plan.md"],
        &goals_match("alpha", "alpha prime"),
    );
    assert_eq!(put.status.code(), Some(0), "mrd put: {}", said(&put));
    write(&ws, "plan.md", "# Plan\n\n## Goals\n\nrewritten by hand\n");
    git_ok(&ws, &["add", "-A"]);

    // The control: without an escape this same tree is refused.
    let refused = sb.commit(&ws, "refused", &[]);
    assert!(
        !refused.status.success(),
        "the escapes below would prove nothing over a tree that commits anyway: {}",
        said(&refused)
    );

    let before = head_count(&ws);
    let forced = sb.commit(&ws, "forced", &[("MRD_HOOK_FORCE", "1")]);
    assert!(
        forced.status.success(),
        "MRD_HOOK_FORCE is the ratified --force escape in its hook spelling: {}",
        said(&forced)
    );
    assert_eq!(
        head_count(&ws),
        before + 1,
        "R40 — the escape produced a commit, it did not merely print"
    );

    // And git's own escape, on a fresh refusal.
    write(
        &ws,
        "plan.md",
        "# Plan\n\n## Goals\n\nrewritten by hand again\n",
    );
    git_ok(&ws, &["add", "-A"]);
    let before = head_count(&ws);
    let out = Command::new("git")
        .arg("-C")
        .arg(&ws)
        .args(["commit", "--no-verify", "-m", "no-verify"])
        .env("PATH", sb.hook_path())
        .env("XDG_CACHE_HOME", &sb.cache_home)
        .env("HOME", &sb.home)
        .output()
        .expect("git commit --no-verify");
    assert!(
        out.status.success(),
        "git's own escape skips the hook entirely: {}",
        said(&out)
    );
    assert_eq!(head_count(&ws), before + 1, "R40 — it committed");
}

/// **`mrd` not on PATH at commit time: the hook FAILS CLOSED with a teaching
/// message**, never a silent pass. An unverifiable commit is not a verified one.
#[test]
fn the_fence_fails_closed_when_mrd_is_not_on_path() {
    let sb = sandbox();
    let ws = sb.corpus("no-mrd");
    sb.install_fence(&ws);
    write(&ws, "plan.md", "# Plan\n\n## Goals\n\nanything\n");
    git_ok(&ws, &["add", "-A"]);

    let before = head_count(&ws);
    let commit = sb.commit_without_mrd(&ws, "no mrd on path");
    assert!(
        !commit.status.success(),
        "silently passing here would make the fence a decoration on every machine \
         where the engine is not installed: {}",
        said(&commit)
    );
    assert_eq!(head_count(&ws), before, "R40 — no commit was recorded");
    let text = said(&commit);
    assert!(
        text.contains("is not on PATH") && text.contains("fails CLOSED"),
        "and it TEACHES rather than failing obscurely: {text}"
    );
    assert!(
        text.contains("mrd hook uninstall"),
        "naming the exit, because a guard with no exit is one an operator \
         disables by deleting the tool: {text}"
    );
}

// ── UNINSTALL — the exit, asserted as a state change (R40) ───────────────────

/// **Uninstall works**, and the assert is a STATE CHANGE in both halves: the hook
/// file is gone, AND the repository commits again the tree that was refused.
///
/// The second half is what makes this more than a file deletion: an uninstall
/// that removed the file while leaving something else fencing would pass a
/// file-existence assert and fail the operator.
#[test]
fn uninstall_removes_the_fence_and_the_repository_commits_again() {
    let sb = sandbox();
    let ws = sb.corpus("uninstall");
    let hook = sb.install_fence(&ws);
    let put = sb.run_stdin(
        &ws,
        &["put", "plan.md"],
        &goals_match("alpha", "alpha prime"),
    );
    assert_eq!(put.status.code(), Some(0), "mrd put: {}", said(&put));
    write(&ws, "plan.md", "# Plan\n\n## Goals\n\nrewritten by hand\n");
    git_ok(&ws, &["add", "-A"]);

    // The control: the fence is genuinely refusing this tree right now.
    assert!(
        !sb.commit(&ws, "refused", &[]).status.success(),
        "an uninstall over a tree that commits anyway proves nothing"
    );

    let out = sb.run(&ws, &["hook", "uninstall"]);
    assert_eq!(out.status.code(), Some(0), "uninstall: {}", said(&out));
    assert!(
        !hook.exists(),
        "R40 — uninstall exited 0 with {} still on disk",
        hook.display()
    );

    let before = head_count(&ws);
    let commit = sb.commit(&ws, "after uninstall", &[]);
    assert!(
        commit.status.success(),
        "the repository must commit again — a guard with no exit is one an \
         operator disables by deleting the tool: {}",
        said(&commit)
    );
    assert_eq!(head_count(&ws), before + 1, "R40 — it committed");
}

/// Uninstall **refuses a `pre-commit` this engine did not write** — the overwrite
/// defect wearing the other sign. The foreign file is still on disk afterwards.
#[test]
fn uninstall_refuses_a_hook_the_engine_did_not_write() {
    let sb = sandbox();
    let ws = sb.corpus("uninstall-foreign");
    let hooks = common_dir(&ws).join("hooks");
    std::fs::create_dir_all(&hooks).expect("hooks dir");
    let foreign = hooks.join("pre-commit");
    std::fs::write(&foreign, "#!/bin/sh\n# LEFTHOOK: do not remove\nexit 0\n").expect("write");

    let out = sb.run(&ws, &["hook", "uninstall"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "refusing on the finding leg: {}",
        said(&out)
    );
    assert!(
        foreign.exists(),
        "R40 — the file this engine does not own is still there"
    );
    assert!(
        said(&out).contains("foreign-hook") && said(&out).contains("LEFTHOOK"),
        "naming the existing file's own words rather than guessing at a tool: {}",
        said(&out)
    );
}

// ── the PER-ROOT refusals: D11 and D12, driven ───────────────────────────────

/// **A `pre-commit` that already exists is refused BY DEFAULT, naming the file.**
/// This is `field-notes`'s measured state (four live lefthook hooks) reproduced as
/// a fixture, because the operator root itself may not be written to by a test.
#[test]
fn install_refuses_an_existing_foreign_pre_commit_naming_it() {
    let sb = sandbox();
    let ws = sb.corpus("foreign-hook");
    let hooks = common_dir(&ws).join("hooks");
    std::fs::create_dir_all(&hooks).expect("hooks dir");
    let foreign = hooks.join("pre-commit");
    let body = "#!/bin/sh\n# LEFTHOOK FILE — do not edit\nlefthook run pre-commit\n";
    std::fs::write(&foreign, body).expect("write");

    let out = sb.run(&ws, &["hook", "install"]);
    assert_eq!(out.status.code(), Some(1), "refused: {}", said(&out));
    assert_eq!(
        std::fs::read_to_string(&foreign).expect("read back"),
        body,
        "R40 — NEVER silently overwrite a file the engine does not own: the \
         bytes are byte-identical after the refusal"
    );
    let text = said(&out);
    assert!(
        text.contains("foreign-hook"),
        "the reason word names the observed state: {text}"
    );
    assert!(
        text.contains(&foreign.display().to_string()),
        "and the refusal NAMES THE EXISTING FILE: {text}"
    );
}

/// **`core.hooksPath` set: refused with the reason, never a silent no-op
/// install** (D11). The fixture reproduces `ccc-statusd`'s measured state — the
/// redirect points OUTSIDE the repository at a directory that already carries its
/// own `pre-commit`, so installing anyway would write into another checkout's
/// hook directory.
#[test]
fn install_refuses_a_root_whose_hooks_path_redirects_elsewhere() {
    let sb = sandbox();
    let ws = sb.corpus("hooks-path");
    let elsewhere = sb.tmp.path().join("other-checkout").join(".githooks");
    std::fs::create_dir_all(&elsewhere).expect("elsewhere");
    std::fs::write(elsewhere.join("pre-commit"), "#!/bin/sh\nexit 0\n").expect("their hook");
    git_ok(
        &ws,
        &["config", "core.hooksPath", &elsewhere.display().to_string()],
    );

    let out = sb.run(&ws, &["hook", "install"]);
    assert_eq!(out.status.code(), Some(1), "refused: {}", said(&out));
    assert!(
        !common_dir(&ws).join("hooks").join("pre-commit").exists(),
        "R40 — and it wrote NOTHING: a file git would never run is worse than \
         no file, because it reads as installed"
    );
    let text = said(&out);
    assert!(
        text.contains("hooks-path-redirected") && text.contains(&elsewhere.display().to_string()),
        "the reason word plus the path git will actually use: {text}"
    );
    assert!(
        text.contains("another checkout's hook directory"),
        "and the stronger true thing: the redirect target already has a \
         pre-commit, so installing would write into someone else's repo: {text}"
    );
}

/// **A submodule refuses loudly with a named reason** (D12). Nothing in this
/// engine can compute `<super>/.git/modules/<name>/hooks`, so it refuses rather
/// than installing where git will not look.
///
/// # This arm's member was CONSTRUCTED, and that is the point
/// The card's population named `ccc-statusd` for this arm; re-measurement found
/// `.git` there is a DIRECTORY — it is a superproject, not a submodule, and its
/// real refusal is `hooks-path-redirected`. A gate asserting "refused twice"
/// there would have been satisfied by the hooksPath refusal alone while proving
/// nothing about D12. So the member is built here (U8's `undeclared-probe` root
/// is the shipped precedent): a real `git submodule add`, a real superproject, a
/// real refusal. **An arm whose population empties gets a member constructed,
/// not deleted.**
#[test]
fn install_refuses_a_submodule_naming_the_superproject() {
    let sb = sandbox();
    let inner = sb.corpus("submodule-inner");
    let outer = sb.corpus("submodule-outer");

    let added = Command::new("git")
        .arg("-C")
        .arg(&outer)
        // `--force` only steps past an ambient ignore rule the operator's global
        // git config may carry over a tempdir path; it changes nothing about
        // what is built. The fixture is adjudicated by git's OWN answer below,
        // never by the fact that this command succeeded.
        .args([
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            "--force",
        ])
        .arg(&inner)
        .arg("vendor/inner")
        .output()
        .expect("git submodule add");
    assert!(
        added.status.success(),
        "the fixture IS the assert's subject — a real submodule: {}",
        said(&added)
    );
    let sub = outer.join("vendor").join("inner");
    assert_eq!(
        git_out(&sub, &["rev-parse", "--show-superproject-working-tree"]),
        outer
            .canonicalize()
            .expect("canonical outer")
            .to_string_lossy(),
        "R40 — git itself calls this a submodule; the fixture is not asserted by \
         its own construction"
    );

    let out = sb.run(&sub, &["hook", "install"]);
    assert_eq!(out.status.code(), Some(1), "refused: {}", said(&out));
    let text = said(&out);
    assert!(text.contains("submodule"), "the reason word: {text}");
    assert!(
        text.contains(".git/modules/"),
        "and it names WHY nothing here can reach that hook dir: {text}"
    );
}

/// **A root that is not a git repository at all**: `MERIDIAN_WORKSPACE` anchors
/// a non-git tree and the cwd default accepts one, so this is a SUPPORTED
/// workspace state — the refusal names it as such rather than as a fault in the
/// workspace. (The retired marker tier used to be the reason; the property
/// outlived it.)
#[test]
fn install_refuses_a_root_that_is_not_a_git_repository() {
    let sb = sandbox();
    let ws = sb.tmp.path().join("not-git");
    std::fs::create_dir_all(&ws).expect("mkdir");
    write(&ws, "page.md", "# Page\n\nbody\n");
    let init = sb.run(&ws, &["init"]);
    assert!(init.status.success(), "mrd init: {}", said(&init));

    let out = sb.run(&ws, &["hook", "install"]);
    assert_eq!(out.status.code(), Some(1), "refused: {}", said(&out));
    let text = said(&out);
    assert!(text.contains("not-a-git-repo"), "the reason word: {text}");
    assert!(
        text.contains("supported state"),
        "and it says so — this is not an error condition of the workspace: {text}"
    );
}

// ── the WORKTREE edge (D11): N workspaces, ONE hook dir ──────────────────────

/// **The worktree case installs per git COMMON dir**, and the one installed hook
/// fences the linked worktree it is committing from.
///
/// This is D11 ruled and then run: the install from a linked worktree lands in
/// the MAIN repository's `hooks/`, not in the worktree's own git dir — and the
/// hook still fires for a commit made in the linked worktree, because it reads
/// the committing worktree from git's working directory instead of baking a path
/// in at install time.
#[test]
fn a_linked_worktree_installs_into_the_common_dir_and_is_fenced_by_it() {
    let sb = sandbox();
    let main = sb.corpus("worktree-main");
    let linked = sb.tmp.path().join("worktree-linked");
    git_ok(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "side",
            &linked.display().to_string(),
        ],
    );
    let init = sb.run(&linked, &["init"]);
    assert!(
        init.status.success(),
        "mrd init in the worktree: {}",
        said(&init)
    );

    // The three senses of "root", measured apart before anything is installed.
    let linked_git_dir = PathBuf::from(git_out(&linked, &["rev-parse", "--absolute-git-dir"]));
    let linked_common = common_dir(&linked);
    assert_ne!(
        linked_git_dir, linked_common,
        "the fixture IS the subject: a linked worktree's git dir is NOT its \
         common dir, which is the whole reason D11 had to pick a side"
    );

    let out = sb.run(&linked, &["hook", "install"]);
    assert_eq!(out.status.code(), Some(0), "install: {}", said(&out));
    let in_common = linked_common.join("hooks").join("pre-commit");
    assert!(
        in_common.exists(),
        "D11 — the hook lands in the COMMON dir {}",
        in_common.display()
    );
    assert!(
        !linked_git_dir.join("hooks").join("pre-commit").exists(),
        "and NOT in the worktree's own git dir, where git would never look for it"
    );

    // And it fences the linked worktree: an out-of-band write there is refused.
    let put = sb.run_stdin(
        &linked,
        &["put", "plan.md"],
        &goals_match("alpha", "alpha prime"),
    );
    assert_eq!(put.status.code(), Some(0), "mrd put: {}", said(&put));
    write(
        &linked,
        "plan.md",
        "# Plan\n\n## Goals\n\nrewritten by hand\n",
    );
    git_ok(&linked, &["add", "-A"]);
    let before = head_count(&linked);
    let commit = sb.commit(&linked, "from the worktree", &[]);
    assert!(
        !commit.status.success(),
        "ONE hook file, installed once, fences whichever worktree is committing: {}",
        said(&commit)
    );
    assert_eq!(head_count(&linked), before, "R40 — no commit was recorded");
}

/// Install is **idempotent and says which it was**: a second install over this
/// engine's own fence reports `already-installed` rather than pretending a fresh
/// write, and never trips the foreign-hook refusal on its own artifact.
#[test]
fn a_second_install_reports_already_installed_rather_than_refusing_itself() {
    let sb = sandbox();
    let ws = sb.corpus("idempotent");
    sb.install_fence(&ws);
    let again = sb.run(&ws, &["hook", "install"]);
    assert_eq!(
        again.status.code(),
        Some(0),
        "second install: {}",
        said(&again)
    );
    assert!(
        stdout(&again).contains("already-installed"),
        "the two are different facts about the disk and are reported apart: {}",
        stdout(&again)
    );
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn git_ok(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git runs in the test environment");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A git answer as RAW BYTES — `git show :path` / `git show HEAD:path`, where a
/// lossy string conversion would hide exactly the byte difference under test.
fn git_bytes(dir: &Path, args: &[&str]) -> Vec<u8> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git runs in the test environment");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

fn git_out(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git runs in the test environment");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// The common git dir, absolute — the directory the fence is installed per.
fn common_dir(dir: &Path) -> PathBuf {
    let text = git_out(
        dir,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    );
    PathBuf::from(text)
}

/// How many commits `HEAD` reaches. **The state change every refusal leg asserts**
/// (R40): a hook that printed a refusal while git committed anyway would pass an
/// exit-code assert and fail the operator.
fn head_count(dir: &Path) -> usize {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-list", "--count", "HEAD"])
        .output()
        .expect("git rev-list");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0)
}

fn write(ws: &Path, rel: &str, body: &str) {
    if let Some(parent) = ws.join(rel).parent() {
        std::fs::create_dir_all(parent).expect("mkdir -p");
    }
    std::fs::write(ws.join(rel), body).expect("write fixture");
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// stdout+stderr together — the render rides stdout, the refusal rides stderr,
/// and what an operator SEES is the union.
fn said(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// One `key=value` field of a journal row, by its `key=` prefix — the row grammar
/// is space-separated, so the value runs to the next space.
fn field(row: &str, key: &str) -> String {
    let rest = row.split_once(key).expect("the row carries the field").1;
    rest.split_whitespace()
        .next()
        .expect("the field carries a value")
        .to_owned()
}

/// A one-edit `match` batch in the wire §4.4 grammar, against `Plan/Goals`.
fn goals_match(old: &str, new: &str) -> String {
    serde_json::to_string(&serde_json::json!([{
        "target": {"hpath": [{"h": "Plan"}, {"h": "Goals"}]},
        "edit": {"match": {"old": old, "new": new}},
    }]))
    .expect("edits json")
}
