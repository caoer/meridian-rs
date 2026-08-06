//! U15 — criterion 5: the commit fence, RUN AS A PLACED HOOK IN A REAL
//! REPOSITORY. Never an asserted function: every leg places the fence
//! `mrd skill hook` emits — extracted from that verbs stdout exactly as its
//! document tells a reader to extract it ([`Sandbox::place_fence`]) — then drives
//! a real `git commit` and reads what git did.
//!
//! # The three arms
//!
//! A guard proven only by what it blocks is indistinguishable from a guard that
//! blocks everything, so the refusals are paired with the acceptance (S3-R8(c)):
//! - [`the_fence_accepts_a_commit_whose_writes_were_all_governed`] — the
//!   ACCEPTANCE, without which every refusal below is unattributable.
//! - [`the_fence_refuses_a_commit_carrying_an_out_of_band_write`].
//! - [`the_fence_refuses_a_commit_that_would_strand_an_anchor_obligation`],
//!   bound to the ORPHAN state (S3-R71(a)): no ref reaches the pinned blob AND
//!   the file no longer hashes to it, so nothing holds it and no commit will.
//!
//! That last binding is load-bearing. `pending-anchor` is the normal state of
//! every governed commit at the moment the fence runs — `git add` writes the blob
//! and the commit that would anchor it is the one being fenced — so an arm wired
//! to it would refuse the lifecycle rather than a defect.

use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// The binary every drive goes through — the real CLI, never a library call.
fn mrd_bin() -> PathBuf {
    std::env::var_os("MRD_BIN")
        .map_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_mrd")), PathBuf::from)
}

/// A sandbox whose `mrd` is OURS and whose caches are its own. The `bin/`
/// directory holds a symlink to the binary under test and leads the hooks `PATH`.
struct Sandbox {
    tmp: tempfile::TempDir,
    bin: PathBuf,
    cache_home: PathBuf,
    home: PathBuf,
}

/// The system directories a `git commit` needs to run at all. The sandbox `bin/` is prepended,
/// so OUR `mrd` shadows any deployed one — the ordering is the isolation, and it is the same
/// ordering an operators own `PATH` has.
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
    /// described. The precondition is asserted: a deployed `mrd` in a system
    /// directory would make this leg measure a different failure.
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

    /// Declare a pin AND LAND IT, so the interval a commit records carries the
    /// lock — `mrd pin` writes the lock into `claim.md` and leaves it
    /// UNCOMMITTED. Returns the governed bytes of the pinned file.
    fn pin_and_land(&self, ws: &Path) -> Vec<u8> {
        let pin = self.run(ws, &["pin", "claim.md", "source.md#Source/Guideline"]);
        assert_eq!(pin.status.code(), Some(0), "mrd pin: {}", said(&pin));
        git_ok(ws, &["add", "-A"]);
        git_ok(ws, &["commit", "--no-verify", "-qm", "pin"]);
        std::fs::read(ws.join("source.md")).expect("governed bytes")
    }

    /// Place the fence the way `mrd skill hook`s document says to, asserting the
    /// STATE CHANGE rather than an exit code (R40): every door git dispatches for
    /// a commit built from a prepared index carries an executable fence. Returns
    /// `pre-commit`, the door the arms in this file drive.
    fn place_fence(&self, ws: &Path) -> PathBuf {
        let out = self.run(ws, &["skill", "hook"]);
        assert_eq!(out.status.code(), Some(0), "mrd skill hook: {}", said(&out));
        let body = fence_body(&stdout(&out));
        let hooks = common_dir(ws).join("hooks");
        std::fs::create_dir_all(&hooks).expect("hooks dir");
        for name in ["pre-commit", "pre-merge-commit", "pre-applypatch"] {
            let hook = hooks.join(name);
            std::fs::write(&hook, &body).expect("place the fence");
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755))
                .expect("chmod +x — a hook git cannot execute is a hook git skips");
            let mode = std::fs::metadata(&hook)
                .expect("stat hook")
                .permissions()
                .mode();
            assert_eq!(
                mode & 0o111,
                0o111,
                "chmod+x is part of placing the fence, not a decoration on it: \
                 {name} mode {mode:o}"
            );
        }
        hooks.join("pre-commit")
    }
}

/// Make the corpus one the gate genuinely refuses: rewrite the PINNED section
/// out of band, so the pin plane reads `red content-drifted`.
fn drift_the_pin(ws: &Path) {
    write(
        ws,
        "source.md",
        "# Source\n\n## Guideline\n\nOUT OF BAND\n\n## Notes\n\nnot pinned.\n",
    );
}

/// The fence body, extracted the way the document says to extract it: the one fenced block, and
/// it is the file. `crates/mrd/tests/skill_hook_emit.rs` holds the document to there being
/// exactly one.
fn fence_body(doc: &str) -> String {
    let mut lines = doc.lines();
    let mut body = String::new();
    lines
        .by_ref()
        .find(|l| l.starts_with("```"))
        .expect("the document carries a fenced block");
    for line in lines {
        if line.starts_with("```") {
            return body;
        }
        body.push_str(line);
        body.push('\n');
    }
    panic!("the fenced block is never closed");
}

// ── ARM 1 — THE ACCEPTANCE (S3-R8(c)) ────────────────────────────────────────

/// The fence ACCEPTS a commit whose writes were ALL governed — the leg without
/// which every refusal below is unattributable. The corpus is written through
/// the shipped write doors (`mrd pin`, then `mrd put`), so `mrd check` exits 0
/// before the hook is asked.
#[test]
fn the_fence_accepts_a_commit_whose_writes_were_all_governed() {
    let sb = sandbox();
    let ws = sb.corpus("accepts");
    sb.place_fence(&ws);

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

/// The fence REFUSES a commit carrying an out-of-band write — the one door the
/// engine cannot see, and the fence's whole reason for existing.
///
/// The control is the acceptance leg above: same hook, same binary, same
/// repository shape. What differs is one plain shell rewrite of a governed file.
///
/// The rewrite has to land on content the corpus CLAIMS, because a claim is the
/// only thing the gate can check a byte against. A laundered edit to unclaimed
/// content is not caught here and is not caught anywhere — the ruled design,
/// recorded in `check_e2e.rs`.
#[test]
fn the_fence_refuses_a_commit_carrying_an_out_of_band_write() {
    let sb = sandbox();
    let ws = sb.corpus("refuses-out-of-band");
    sb.place_fence(&ws);

    // Declare and land a pin, so the corpus CLAIMS something the edit can break.
    sb.pin_and_land(&ws);
    assert_eq!(
        sb.run(&ws, &["check"]).status.code(),
        Some(0),
        "the corpus is clean before the edit — otherwise this leg would be \
         measuring a false red"
    );

    // The out-of-band write: no meridian writer touched this.
    drift_the_pin(&ws);

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

/// The fence REFUSES a commit whose INDEX carries an out-of-band write, with the
/// worktree restored byte-exact (F1).
///
/// The byte-exactness is the whole gate: `mrd pin` writes a `^facts` anchor into
/// the target, so an undo that reconstructs the file by hand does not restore it
/// and the fence refuses for the WRONG REASON. Here the governed bytes are
/// CAPTURED and written back verbatim, so the worktree is provably identical and
/// the index provably not — both asserted before the commit is attempted. The
/// control is [`the_fence_accepts_a_commit_whose_writes_were_all_governed`].
#[test]
fn the_fence_refuses_a_commit_whose_index_carries_an_out_of_band_write() {
    let sb = sandbox();
    let ws = sb.corpus("refuses-index");
    sb.place_fence(&ws);

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
    // History is the oracle: whatever the hook printed, the bytes are what matter.
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
    // S3-R104 — assert the CAUSE: this arm has two independent refusing detectors,
    // and a later fix could delete one without failing anything.
    assert!(
        text.contains("content-drifted"),
        "the refusal must name WHAT it saw — a pin whose target drifted — or this arm \
         could later pass on a refusal that has nothing to do with the forgery: {text}"
    );
}

/// The fence still ACCEPTS the same corpus once the forgery is out of BOTH
/// intervals, over the exact repository the leg above refused. Without it, that
/// refusal is satisfied by a fence that refuses every commit with anything staged
/// at all — what widening an interval most plausibly breaks.
#[test]
fn the_widened_interval_still_accepts_a_governed_commit_over_the_same_corpus() {
    let sb = sandbox();
    let ws = sb.corpus("accepts-index");
    sb.place_fence(&ws);

    // LANDED, not merely declared: until the lock is in the interval a commit
    // records, the bytes a commit would record claim nothing and the forgery below
    // is invisible to the gate.
    let governed = sb.pin_and_land(&ws);

    // The refusal first, so the acceptance below is measured over a corpus this
    // fence genuinely stops.
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
    // receipt vouches for. The edit to an unpinned page is what makes the
    // acceptance a REAL commit rather than an empty one git refuses on its own.
    write(&ws, "plan.md", "# Plan\n\n## Goals\n\nalpha beta gamma\n");
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

/// The fence REFUSES a commit that would strand an anchor obligation — the
/// ORPHAN: no ref reaches the pinned blob AND the file no longer hashes to it.
/// The corpus isolates the anchoring finding from the content one: the
/// out-of-band edit lands in `Notes`, OUTSIDE the pinned `Guideline`, so the
/// claim plane stays green while the whole-file `objects:` blob moves.
#[test]
fn the_fence_refuses_a_commit_that_would_strand_an_anchor_obligation() {
    let sb = sandbox();
    let ws = sb.corpus("refuses-orphan");
    sb.place_fence(&ws);

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

/// A legitimately-governed PARTIAL STAGE is ACCEPTED — `git add` followed by a
/// further governed write, which `git add -p` reaches on every hunk it leaves
/// behind.
///
/// The corpus contains no out-of-band write at all: `mrd pin` → `mrd put` edit
/// ONE → `git add notes.md` → `mrd put` edit TWO, all governed, so the index
/// holds the EXACT post-edit-ONE bytes — a state the engine itself wrote and
/// journaled. That snapshot is internally inconsistent and legitimately so:
/// `git add notes.md` stages content without the journal, so the staged tree
/// folds to `^r-000002`'s `root_after` while the staged journal still ends at
/// `^r-000001`.
///
/// The discriminator is that a legitimate intermediate state matches SOME
/// receipt in the record, and a forgery matches NONE. The refusal arms above are this
/// arm's control: they must keep refusing in the same run, and
/// [`a_staged_journal_forgery_is_refused_though_the_journal_is_root_excluded`] is
/// the one that proves the widening did not open the journal door.
#[test]
fn a_governed_partial_stage_is_accepted_though_the_journal_has_moved_past_it() {
    let sb = sandbox();
    let ws = sb.corpus("governed-partial-stage");
    sb.place_fence(&ws);

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

    // The fixture's own preconditions, so a pass cannot come from a corpus that
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

/// The tree-form predicate is gone, and this arm asserts its absence. Two
/// governed writes to two files, then only the SECOND staged: the index carries
/// `b.md`s new bytes beside `a.md`s HEAD bytes — a combination no governed write
/// ever produced, though every individual paths bytes came from some receipt. The
/// tree-form predicate refused that; a per-path reading does not.
#[test]
fn a_mixed_two_receipt_stage_now_commits_because_the_tree_predicate_is_gone() {
    let sb = sandbox();
    let ws = sb.corpus("mixed-two-receipt");
    // The two files are committed BEFORE the fence is installed: writing them is
    // an out-of-band write, which the fence would refuse for the right reason and
    // mask this arm.
    write(&ws, "a.md", "# A\n\n## Log\n\nalpha\n");
    write(&ws, "b.md", "# B\n\n## Log\n\nbeta\n");
    git_ok(&ws, &["add", "-A"]);
    git_ok(&ws, &["commit", "-qm", "two files"]);
    sb.place_fence(&ws);

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
        commit.status.success(),
        "THE REDUCTION, ASSERTED: this tree is one no governed write ever produced, \
         and the fence lets it through. The predicate that refused it read the \
         RECORD, and the engine keeps none. If this ever refuses again, a tree-form \
         predicate came back and S3-R103 needs re-ruling rather than re-deriving: {}",
        said(&commit)
    );
    assert_eq!(
        head_count(&ws),
        before + 1,
        "R40 — the commit is REAL, so this is a reduction in enforcement and not a \
         rendering change"
    );
    // The corpus is not refusable for some other reason — otherwise this arm would
    // be asserting a pass it got by accident.
    assert_eq!(
        sb.run(&ws, &["check", "--commit-gate"]).status.code(),
        Some(0),
        "and the gate had no complaint to make: nothing here declares a pin, so \
         there is nothing for it to check the composed tree against"
    );
}

// ── THE OTHER DOORS ONTO THE SAME INTERVAL GAP (F1 names them) ───────────────

/// `git commit <pathspec>` records a THIRD interval: `git commit -- <paths>` is
/// `--only`, so the commit tree is HEAD plus the named paths worktree bytes,
/// ignoring the index. A governed write to a file the commit does not name is
/// LEFT BEHIND, and the tree that lands is one no receipt ever produced — while
/// the worktree is entirely governed and a worktree check is honestly green.
/// Here the commit names `plan.md` alone, so the recorded tree carries the
/// governed `plan.md` beside HEADs lock-less `claim.md`.
#[test]
fn a_pathspec_commit_now_lands_because_its_only_detector_was_the_record() {
    let sb = sandbox();
    let ws = sb.corpus("pathspec");
    sb.place_fence(&ws);

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
    // This arms only refusing detector was the journal plane, and that plane is
    // deleted: the pin plane genuinely has no complaint about this tree.
    assert!(
        commit.status.success(),
        "THE REDUCTION, ASSERTED: `git commit -- <pathspec>` is `--only`, so this \
         commit records HEAD's lock-less claim.md beside a governed plan.md — a \
         third interval, and a tree no governed write produced. It LANDS now. The \
         detector that refused it was the record, and the engine keeps none: {}",
        said(&commit)
    );
    assert_eq!(
        head_count(&ws),
        before + 1,
        "R40 — the commit is REAL. This is lost enforcement, recorded rather than \
         discovered later by someone who trusted the fence to cover pathspec commits"
    );
    // The interval machinery itself still works — asserted so this arm does not
    // read as "the fence stopped seeing pathspec commits".
    let gated = sb.run(&ws, &["check", "--commit-gate"]);
    assert_eq!(
        gated.status.code(),
        Some(0),
        "the gate ran and had nothing to say — it is not blind to the interval, it \
         has no claim to check this tree against: {}",
        said(&gated)
    );
    assert!(
        !said(&gated).contains(check::GREY_CANNOT_ASSESS),
        "and specifically NOT grey: nothing here is unassessable, there is simply \
         no pin covering the combination: {}",
        said(&gated)
    );
}

/// A write landing BETWEEN `git add` and hook fire does not block the governed
/// index; the same bytes, STAGED, are refused. The pair is the contract — either
/// half alone is satisfied by a fence that does the wrong thing. The concurrent
/// writer is simulated where it is deterministic: the bytes change after staging
/// and before the hook runs, exactly the state such a race leaves.
///
/// `--commit-gate` names ONE interval — the one a commit records — because a
/// finding from the other swamps a clean answer about the bytes actually being
/// committed (S4-R19).
#[test]
fn a_write_landing_after_git_add_does_not_block_the_governed_index() {
    let sb = sandbox();
    let ws = sb.corpus("interleaved");
    sb.place_fence(&ws);
    sb.pin_and_land(&ws);

    // A governed write, staged: this is the commit in flight.
    let put = sb.run_stdin(
        &ws,
        &["put", "plan.md"],
        &goals_match("alpha", "alpha prime"),
    );
    assert_eq!(put.status.code(), Some(0), "mrd put: {}", said(&put));
    git_ok(&ws, &["add", "-A"]);

    // The other agent writes, after the stage and before the hook, and it breaks a
    // pin — so the bytes on disk are genuinely bad and the gate CAN see them.
    drift_the_pin(&ws);
    assert_eq!(
        sb.run(&ws, &["check"]).status.code(),
        Some(1),
        "the fixture IS the subject: the WORKTREE is now refusable, so a gate \
         reading the wrong interval would block this commit"
    );

    // ACCEPTANCE — the index holds clean bytes, and those are what commits.
    let before = head_count(&ws);
    let commit = sb.commit(&ws, "raced", &[]);
    assert!(
        commit.status.success(),
        "the interval this commit records holds its pins, and the gate reads THAT \
         interval — not the racing write it will not record: {}",
        said(&commit)
    );
    assert_eq!(head_count(&ws), before + 1, "R40 — the commit was recorded");

    // REFUSAL, same run, same bytes — staged this time. Without this half the arm
    // is satisfied by a gate that accepts everything.
    git_ok(&ws, &["add", "-A"]);
    let before = head_count(&ws);
    let refused = sb.commit(&ws, "and now it is staged", &[]);
    assert!(
        !refused.status.success(),
        "the ungoverned write is refused the moment it is part of what a commit \
         records — nothing that breaks a declared pin reaches history: {}",
        said(&refused)
    );
    assert_eq!(head_count(&ws), before, "R40 — no commit was recorded");
}

/// The reserved journal is an ORDINARY PAGE, and staging a forged one moves
/// nothing: no root exclusion means no special index read, and nothing dates a
/// tree against it.
#[test]
fn a_staged_journal_forgery_moves_nothing_because_the_path_is_an_ordinary_page() {
    let sb = sandbox();
    let ws = sb.corpus("journal-index");
    sb.place_fence(&ws);

    let former = ws.join("meridian").join("journal.md");
    assert!(
        !former.exists(),
        "no governed write mints one: the ledger this arm forged does not exist"
    );

    // A forged journal-shaped page, in the shipped row grammar, with a broken LINK
    // — byte-for-byte the shape the old arm refused.
    std::fs::create_dir_all(former.parent().expect("meridian dir")).expect("mkdir");
    std::fs::write(
        &former,
        "# Receipt journal\n\
         - op=splice path=plan.md root_before=b3:R0 root_after=b3:R1 edits=1 ^r-000001\n\
         - op=splice path=plan.md root_before=b3:FORGED root_after=b3:R2 edits=1 ^r-000002\n",
    )
    .expect("write the forged page");
    git_ok(&ws, &["add", "-A"]);

    let before = head_count(&ws);
    let commit = sb.commit(&ws, "the index carries a forged journal", &[]);
    assert!(
        commit.status.success(),
        "THE REDUCTION, ASSERTED: a forged ledger with a broken link is just a page \
         now. Nothing reads it, nothing carves it out of the domain, and nothing \
         refuses it: {}",
        said(&commit)
    );
    assert_eq!(
        head_count(&ws),
        before + 1,
        "R40 — the commit is REAL, so the forged page genuinely reached history"
    );
    assert!(
        !said(&commit).contains("r-000002"),
        "and no surface cited the forged row, because no surface parsed it: {}",
        said(&commit)
    );
}

/// A VERSION SKEW — this fence, an OLDER `mrd` on PATH — fails CLOSED and names
/// the skew.
///
/// The hook resolves its engine at commit time and never bakes one in (D11), so a
/// fence placed from a new engine's document can be run against an old one that
/// answers `unknown flag: --commit-gate` and exits 2. Failing closed is required
/// — falling back to an unscoped `mrd check` would restore the F1 false green —
/// but a bare "exited 2" leaves an operator with a bricked repository and no idea
/// why. The stand-in for the old engine answers exactly as one does.
#[test]
fn a_fence_run_against_an_older_engine_refuses_and_names_the_skew() {
    let sb = sandbox();
    let ws = sb.corpus("version-skew");
    sb.place_fence(&ws);

    // An `mrd` that behaves like an engine predating `--commit-gate`: exit 2 on
    // the flag. Everything else is unchanged, so the refusal below is attributable
    // to the skew.
    let old = sb.tmp.path().join("old-bin");
    std::fs::create_dir_all(&old).expect("old bin dir");
    std::fs::write(
        old.join("mrd"),
        "#!/bin/sh\nfor a in \"$@\"; do\n  if [ \"$a\" = \"--commit-gate\" ]; then\n    \
         echo 'mrd: unknown flag: --commit-gate' >&2\n    exit 2\n  fi\ndone\nexit 0\n",
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
        text.contains("command -v mrd") && text.contains("mrd check"),
        "and it names the two commands that DECIDE the cause, rather than accusing \
         (an unreadable workspace also exits 2): {text}"
    );
}

// ── the ESCAPES, and the state change that proves each one ───────────────────

/// `--force` is honoured, in its hook spelling, and so is gits own `--no-verify`.
/// Both run against the SAME corpus arm 2 proved refused, so each is measured as
/// an escape from a real refusal.
#[test]
fn both_escapes_carry_a_commit_the_fence_refused() {
    let sb = sandbox();
    let ws = sb.corpus("escapes");
    sb.place_fence(&ws);
    // A CONSTRUCTED refusing tree: a landed pin, then an out-of-band rewrite of
    // the content it claims.
    sb.pin_and_land(&ws);
    drift_the_pin(&ws);
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

/// `mrd` not on PATH at commit time: the hook FAILS CLOSED with a teaching
/// message, never a silent pass. An unverifiable commit is not a verified one.
#[test]
fn the_fence_fails_closed_when_mrd_is_not_on_path() {
    let sb = sandbox();
    let ws = sb.corpus("no-mrd");
    sb.place_fence(&ws);
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
        text.contains("remove:") && text.contains("rm \"$0\""),
        "naming the exit, because a guard with no exit is one an operator \
         disables by deleting the tool — and with no uninstaller to name, the \
         fence has to say how to delete itself: {text}"
    );
}

// ── the WORKTREE edge (D11): N workspaces, ONE hook dir ──────────────────────

/// Placing per git COMMON dir from a linked worktree lands in the MAIN
/// repositorys `hooks/`, and that one file fences the linked worktree it is
/// committing from (D11).
#[test]
fn a_linked_worktree_places_into_the_common_dir_and_is_fenced_by_it() {
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

    // Placed from the LINKED worktree: `place_fence` resolves the common dir the
    // way the document tells its reader to, so where it lands is the instructions
    // answer and not the tests.
    sb.place_fence(&linked);
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
    // The refusing tree is CONSTRUCTED (a landed pin, then a rewrite of the content
    // it claims) because this arms subject is WHICH HOOK FILE FIRES.
    sb.pin_and_land(&linked);
    drift_the_pin(&linked);
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

/// How many commits `HEAD` reaches — the state change every refusal leg asserts
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

/// A one-edit `match` batch in the wire §4.4 grammar, against `Plan/Goals`.
fn goals_match(old: &str, new: &str) -> String {
    serde_json::to_string(&serde_json::json!([{
        "target": {"hpath": [{"h": "Plan"}, {"h": "Goals"}]},
        "edit": {"match": {"old": old, "new": new}},
    }]))
    .expect("edits json")
}
