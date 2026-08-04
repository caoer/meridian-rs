//! **S4-R19 — what `--commit-gate` still proves, re-derived after the journal
//! died. This file is a REWRITE, not a repair, and the axis it measures is now
//! ONE-DIMENSIONAL.**
//!
//! # The card this file was built for
//! Three propositions rode one exit code, and the fence spent the permanent one on
//! every commit:
//!
//! | | proposition | about | permanence |
//! |---|---|---|---|
//! | **P1** | the journal chain has a break | the **past** | permanent by design |
//! | **P2** | this index carries an out-of-band write | **this interval** | per-commit |
//! | **P3** | the journal cannot date the tree | **evidence availability** | per-state |
//!
//! Past the first break P1 was permanently true, so the exit stopped varying with
//! what was staged — a guard whose verdict no longer depends on the thing it guards
//! carries zero information about it. `--commit-gate` asked the per-commit question
//! instead, and the old 8-arm matrix measured that it discriminated across the axis
//! **chain-break × staged-write**.
//!
//! # WHY THE MATRIX COLLAPSED (ZT, 2026-08-03)
//! *"Engine does not have memory. It should not have. History is pin to git when we
//! lock. Anything between locks is not history."*
//!
//! **P1 and P3 are both journal propositions and both are gone.** The gate reads the
//! PIN PLANE and nothing else (U5's derivation note § 6): `permits() ==
//! !pins.is_red() && !pins.cannot_assess()`. So the chain-break axis has no
//! antecedent — `break_the_chain` forged a `root_before` inside a file that does not
//! exist — and **half the old matrix was measuring a dimension that no longer has
//! two values.**
//!
//! P2 survives whole, and its DETECTOR changed underneath it: an out-of-band edit is
//! caught because the rewritten file is a **pinned target**, so the pin plane reads
//! `red content-drifted`. Same proposition, different plane answering.
//!
//! **A 4-arm matrix that honestly covers one axis beats an 8-arm one with four dead
//! arms.** The axis is: *does the interval a commit records carry a pin that no
//! longer holds?*
//!
//! # THE OLD ARMS, each accounted for — none weakened into passing
//! | old arm | disposition |
//! |---|---|
//! | 1 — chain-broken + governed staged write commits, break told | **DELETED.** Both halves are chain. Nothing tells a break. |
//! | 2 — clean chain + governed write passes both questions | **SURVIVES** as [`a_governed_corpus_passes_both_questions`]. |
//! | 3 — clean chain + out-of-band write refused | **SURVIVES** as [`an_out_of_band_write_to_pinned_content_is_refused`]; only the reason STRING moved (`no governed write ever produced` was baseline vocabulary). |
//! | 4 — the specificity arm, index vs worktree | **SURVIVES** as [`a_forged_index_is_refused_even_when_the_worktree_is_clean`]. Its real subject was always the INTERVAL; the chain was scenery, so it is rebuilt on a clean corpus. |
//! | 5 — the break never heals | **DELETED.** No break to heal. |
//! | 6 — a gated pass over a broken record is not spelled green | **PARTLY SURVIVES** as [`the_passing_word_names_the_plane_that_answered`]: the vocabulary claim outlived its corpus, and the word to hold honest is now `pins-hold`. |
//! | 7 — an empty record refuses without accusing | **REVERSED, BY RULING** — see [`a_pinless_corpus_passes_by_default`]. |
//! | 8 — empty record + out-of-band write fails closed | **REVERSED, same antecedent** — [`a_pinless_corpus_passes_even_with_an_out_of_band_write`]. |
//!
//! # THE INSTRUMENT, and why every arm here can still fail
//! The subject is a gate that must DISCRIMINATE, so an arm that passes whatever the
//! gate does is decoration. Two devices keep these arms falsifiable:
//!
//! 1. **Acceptance and refusal over ONE corpus** (S3-R99). A gate that permits and a
//!    gate that is silently broken look identical from the caller, so the refusal
//!    arms sit beside acceptance arms built from the same fixture mutated.
//! 2. **The unscoped question is measured in the same run** where the two can
//!    differ, so an arm cannot be satisfied by a verb that stopped refusing
//!    anything.
//!
//! # The fence here is a TEST FIXTURE, not the shipped one
//! This harness places its own `pre-commit` that execs the scoped question, so these
//! arms measure `--commit-gate` itself and not the contract built on it.
//! `crates/mrd/tests/skill_hook_emit.rs` holds the shipped document;
//! `crates/mrd/tests/hook_plane_fence.rs` drives the real emitted body.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// The binary every drive goes through — the shipped CLI, never a library call.
fn mrd_bin() -> PathBuf {
    std::env::var_os("MRD_BIN")
        .map_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_mrd")), PathBuf::from)
}

// ── the sandbox ──────────────────────────────────────────────────────────────

struct Sandbox {
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
}

fn sandbox() -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_home = tmp.path().join("xdg-cache");
    let home = tmp.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    Sandbox {
        tmp,
        cache_home,
        home,
    }
}

impl Sandbox {
    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        Command::new(mrd_bin())
            .args(args)
            .current_dir(cwd)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env_remove("MERIDIAN_WORKSPACE")
            .output()
            .expect("spawn mrd")
    }

    /// A `pre-commit` that execs **the scoped question** and hands git its exit
    /// status. Zero markdown semantics — an adapter over the engine.
    fn install_fence(&self, ws: &Path) {
        let hook = ws.join(".git/hooks/pre-commit");
        std::fs::create_dir_all(hook.parent().expect("hooks dir")).expect("mkdir hooks");
        std::fs::write(
            &hook,
            format!(
                "#!/bin/sh\n\
                 XDG_CACHE_HOME='{cache}' HOME='{home}' exec '{mrd}' check --commit-gate\n",
                cache = self.cache_home.display(),
                home = self.home.display(),
                mrd = mrd_bin().display(),
            ),
        )
        .expect("write hook");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755))
                .expect("chmod +x");
        }
    }

    /// A real git repo with a pinnable source and three claim pages, `mrd init`,
    /// one commit, then the fence. **Nothing here declares a pin** — that is
    /// [`Sandbox::pin_a_claim`]'s job, and the difference is now the whole axis.
    fn corpus(&self, name: &str) -> PathBuf {
        let ws = self.tmp.path().join(name);
        std::fs::create_dir_all(&ws).expect("mkdir");
        git(&ws, &["init", "-q"]);
        git(&ws, &["config", "user.email", "r19@example.invalid"]);
        git(&ws, &["config", "user.name", "r19"]);
        git(&ws, &["config", "commit.gpgsign", "false"]);
        write(&ws, "source.md", "# Source\n\n## Guideline\n\nthe body\n");
        for claim in ["claim.md", "claim2.md", "claim3.md"] {
            write(&ws, claim, "# Claim\n\nwe rely on the guideline.\n");
        }
        let init = self.run(&ws, &["init"]);
        assert!(init.status.success(), "mrd init: {}", said(&init));
        git(&ws, &["add", "-A"]);
        git(&ws, &["commit", "-qm", "corpus"]);
        self.install_fence(&ws);
        ws
    }

    /// One governed write through the engine's own door: `mrd pin` declares a claim
    /// against a section of `source.md`.
    ///
    /// **Renamed from `governed_write`.** The old name meant *"a write that journals
    /// a row"*, and there are no rows — keeping it would have left the file's
    /// central fixture named after the vanished plane. What makes this a governed
    /// write is that it went through the door; what makes it matter HERE is that it
    /// leaves a PIN, which is the only thing the gate reads.
    /// # The pin is COMMITTED, and that is not incidental — it is the axis
    /// The gate reads **the interval a commit records**. A pin that has been made
    /// but not staged is not in that interval: the index still carries the page
    /// WITHOUT its lock, so the bytes a commit would record declare nothing, and
    /// the gate correctly reports zero coverage.
    ///
    /// Measured while writing this file — two arms failed on exactly that, and the
    /// engine was right both times. Left recorded here so the next reader does not
    /// re-derive it as a defect: `--require-pins` refusing a workspace whose pin is
    /// not staged yet is the flag working, not the flag misfiring.
    ///
    /// `--no-verify` because this is SETUP: a fixture step that ran the fence would
    /// make every corpus depend on the thing under test.
    fn pin_a_claim(&self, ws: &Path, claim: &str) {
        let pin = self.run(ws, &["pin", claim, "source.md#Source/Guideline"]);
        assert_eq!(code(&pin), 0, "the governed write lands: {}", said(&pin));
        git(ws, &["add", "-A"]);
        git(ws, &["commit", "--no-verify", "-qm", "pin"]);
    }

    fn commit(&self, ws: &Path, message: &str) -> (i32, String) {
        let add = Command::new("git")
            .arg("-C")
            .arg(ws)
            .args(["add", "-A"])
            .output()
            .expect("git add");
        assert!(add.status.success(), "git add: {}", said(&add));
        self.commit_without_adding(ws, message)
    }

    /// Commit whatever the INDEX already carries — no `git add`, so a staged
    /// forgery whose worktree was restored stays diverged (the F1 shape).
    fn commit_without_adding(&self, ws: &Path, message: &str) -> (i32, String) {
        let out = Command::new("git")
            .arg("-C")
            .arg(ws)
            .args(["commit", "-m", message])
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env_remove("MERIDIAN_WORKSPACE")
            .output()
            .expect("git commit");
        (out.status.code().expect("exit code"), said(&out))
    }
}

// ── small helpers ────────────────────────────────────────────────────────────

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("git runs in the test environment");
    assert!(status.success(), "git {args:?}");
}

fn write(ws: &Path, rel: &str, body: &str) {
    let path = ws.join(rel);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir -p");
    std::fs::write(path, body).expect("write fixture");
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// stdout+stderr together — the render rides stdout, the refusal rides stderr, and
/// "what did the operator see" is a question about both.
fn said(out: &Output) -> String {
    format!(
        "{}{}",
        stdout(out),
        String::from_utf8_lossy(&out.stderr).into_owned()
    )
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("exit code")
}

fn git_head(ws: &Path) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(ws)
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .expect("git rev-parse");
    if out.status.success() {
        stdout(&out).trim().to_owned()
    } else {
        "(unborn)".to_owned()
    }
}

/// Rewrite the PINNED section by hand — an out-of-band edit through no meridian
/// writer, to content a lock names. This is what the surviving detector detects.
fn rewrite_pinned_content(ws: &Path, marker: &str) {
    write(
        ws,
        "source.md",
        &format!("# Source\n\n## Guideline\n\n{marker}\n"),
    );
}

// ── the instrument's own control ─────────────────────────────────────────────

/// **The gate can both accept and refuse.** Without this the whole file could be
/// satisfied by a gate wired to a constant, and a constant is precisely the defect
/// this card started from wearing the other sign.
#[test]
fn the_gate_accepts_pinned_work_and_refuses_an_out_of_band_edit() {
    let sb = sandbox();
    let ws = sb.corpus("control");
    sb.pin_a_claim(&ws, "claim.md");

    let accepts = sb.run(&ws, &["check", "--commit-gate"]);
    assert_eq!(
        code(&accepts),
        0,
        "a corpus whose pins hold passes the gate: {}",
        said(&accepts)
    );

    rewrite_pinned_content(&ws, "rewritten by hand");
    // STAGED, because the gate reads the index — which is the state a pre-commit
    // fence actually fires in, and the reason the interval exists at all (F1).
    git(&ws, &["add", "-A"]);
    let refuses = sb.run(&ws, &["check", "--commit-gate"]);
    assert_eq!(
        code(&refuses),
        1,
        "and an out-of-band edit to pinned content does not: {}",
        said(&refuses)
    );
    assert_ne!(
        code(&accepts),
        code(&refuses),
        "the instrument VARIES with its input — the one property the shipped fence lost"
    );
}

/// **A BOUNDARY, asserted so a later reader finds it stated rather than
/// surprising.** An out-of-band edit that is NOT staged does not refuse: the index
/// still carries clean bytes, and those are the bytes a commit would record.
///
/// This is F1 working in the direction it was built for, and **it is untouched by
/// the ruling** — the interval is a question about git, not about engine memory.
#[test]
fn an_unstaged_out_of_band_edit_does_not_gate_a_clean_index() {
    let sb = sandbox();
    let ws = sb.corpus("boundary");
    sb.pin_a_claim(&ws, "claim.md");
    rewrite_pinned_content(&ws, "rewritten by hand, and NOT staged");

    let gated = sb.run(&ws, &["check", "--commit-gate"]);
    assert_eq!(
        code(&gated),
        0,
        "the index carries clean bytes, and the index is what a commit records: {}",
        said(&gated)
    );
    assert!(
        stdout(&gated).contains("gated on: staged"),
        "and the render names the interval it answered for, so the pass is locatable: {}",
        stdout(&gated)
    );

    // The same corpus, one `git add` later: refused.
    git(&ws, &["add", "-A"]);
    let staged_now = sb.run(&ws, &["check", "--commit-gate"]);
    assert_eq!(
        code(&staged_now),
        1,
        "staging the forgery is what makes it the commit's problem: {}",
        said(&staged_now)
    );
}

// ── ACCEPTANCE ───────────────────────────────────────────────────────────────

/// **Was ARM 2, and it survives untouched in substance.** A corpus whose pins hold
/// passes BOTH questions, so a gate that learned to say "no" to everything is caught
/// right here, and a verb whose unscoped form broke outright is caught beside it.
#[test]
fn a_governed_corpus_passes_both_questions() {
    let sb = sandbox();
    let ws = sb.corpus("clean");
    sb.pin_a_claim(&ws, "claim.md");

    let staged = sb.run(&ws, &["check", "--staged"]);
    let gated = sb.run(&ws, &["check", "--commit-gate"]);
    assert_eq!(code(&staged), 0, "the unscoped question: {}", said(&staged));
    assert_eq!(code(&gated), 0, "and the scoped one: {}", said(&gated));

    // An ordinary edit to a page nothing pins — the commonest commit there is, and
    // the one a gate must never stand in the way of.
    write(
        &ws,
        "claim2.md",
        "# Claim\n\nwe rely on it, and said so twice.\n",
    );
    let (commit_code, commit_said) = sb.commit(&ws, "work over a corpus whose pins hold");
    assert_eq!(commit_code, 0, "the commit lands: {commit_said}");
}

// ── REFUSAL, in the same run ─────────────────────────────────────────────────

/// **Was ARM 3.** An out-of-band write to pinned content is refused, naming the
/// interval. The per-commit proposition P2 survives the ruling whole.
///
/// # What changed here is the DETECTOR and therefore the WORDS, not the verdict
/// The arm used to require the refusal to say `"no governed write ever produced"` —
/// `Accounted::ForeignBytes`, the journal-derived leg, which asserted that no
/// recorded write produced these bytes. That vocabulary is gone with the record.
///
/// The refusal is now the pin plane's and carries the pin plane's reason word,
/// `red content-drifted`, which is the SAME word `mrd walk` and `mrd status` print
/// over the same corpus (one computer, `view::walk::lock_pin_colors`). The exit code
/// and the subject are unmoved; only the sentence naming the cause moved, because a
/// different plane is the one answering.
#[test]
fn an_out_of_band_write_to_pinned_content_is_refused() {
    let sb = sandbox();
    let ws = sb.corpus("out-of-band");
    sb.pin_a_claim(&ws, "claim.md");

    rewrite_pinned_content(&ws, "rewritten by hand, through no writer");
    // Staged: this is the state the fence fires in, and the bytes a commit records.
    git(&ws, &["add", "-A"]);

    let gated = sb.run(&ws, &["check", "--commit-gate"]);
    assert_eq!(
        code(&gated),
        1,
        "the bytes being committed contradict a pin this corpus declares: {}",
        said(&gated)
    );
    assert!(
        said(&gated).contains("content-drifted"),
        "and the refusal says which condition failed, in the pin plane's own reason \
         word: {}",
        said(&gated)
    );
    assert!(
        said(&gated).contains("claim.md"),
        "citing the page whose claim was broken, so the operator can locate it: {}",
        said(&gated)
    );

    let head_before = git_head(&ws);
    let (commit_code, commit_said) = sb.commit(&ws, "out-of-band: a shell rewrite");
    assert_ne!(commit_code, 0, "the fence REFUSES: {commit_said}");
    assert_eq!(
        head_before,
        git_head(&ws),
        "R40 — HEAD did not move, so nothing entered history: {commit_said}"
    );
}

/// **Was ARM 4 — THE SPECIFICITY ARM, and its subject was never the chain.** An
/// out-of-band write staged in the INDEX with the worktree restored to clean bytes
/// (the F1 shape). The gate must still refuse.
///
/// It fails if the gate read the WORKTREE here: the worktree is spotless at this
/// instant, so a gate pointed at the wrong interval passes cleanly while a forgery
/// goes into history.
///
/// **Rebuilt on a clean corpus.** The old arm started from `chain_broken_corpus`,
/// but the break was scenery — it was there to prove the gate ignored P1 while still
/// catching P2. There is no P1 to ignore, so the scenery is struck and the arm
/// measures its actual subject with nothing else in the frame.
#[test]
fn a_forged_index_is_refused_even_when_the_worktree_is_clean() {
    let sb = sandbox();
    let ws = sb.corpus("forged-index");
    sb.pin_a_claim(&ws, "claim.md");
    let clean = std::fs::read_to_string(ws.join("source.md")).expect("clean bytes");

    // Stage a forgery, then restore the worktree: the index carries bytes that
    // contradict the pin, while the bytes on disk are perfectly fine.
    rewrite_pinned_content(&ws, "forged, then hidden from the worktree");
    git(&ws, &["add", "source.md"]);
    write(&ws, "source.md", &clean);

    // The CONTROL that makes this arm mean what it says: the worktree really is
    // clean, so a refusal below can only have come from the index.
    let worktree_only = sb.run(&ws, &["check"]);
    assert_eq!(
        code(&worktree_only),
        0,
        "the worktree is spotless — if this refused, the arm below would prove \
         nothing about intervals: {}",
        said(&worktree_only)
    );

    let gated = sb.run(&ws, &["check", "--commit-gate"]);
    assert_eq!(
        code(&gated),
        1,
        "the INDEX carries a forgery, so the gate refuses even though the worktree \
         is clean: {}",
        said(&gated)
    );
    assert!(
        said(&gated).contains("staged"),
        "and it names the interval the refusal came from: {}",
        said(&gated)
    );

    let head_before = git_head(&ws);
    let (commit_code, commit_said) = sb.commit_without_adding(&ws, "forged index");
    assert_ne!(commit_code, 0, "the fence REFUSES: {commit_said}");
    assert_eq!(
        head_before,
        git_head(&ws),
        "R40 — nothing entered history: {commit_said}"
    );
}

// ── THE REVERSED ARMS — ruled, recorded, and given the flag they needed ──────

/// **Was ARM 7, and its assertion is INVERTED BY RULING.** A corpus that declares no
/// pin PASSES `--commit-gate`: exit 0, verdict `pins-hold`.
///
/// # The old arm, and why its doctrine did not simply lose
/// It asserted exit 1, under the name *"an empty record refuses without accusing"* —
/// unknown is not clean, fail CLOSED. **That reading was journal mechanics, not
/// independent doctrine:** an empty record meant no baseline, no baseline meant a
/// grey write-history plane, and the grey gated the exit. The antecedent died with
/// the journal by ruling, so this is a mechanism with no input left being removed —
/// not a safety principle being overturned.
///
/// # Zero pins is VACUOUS TRUTH, not unknown
/// Fail-closed protects the case where the gate CANNOT ASSESS something it claims to
/// gate. That case is a grey pin or an unaskable object store, and **it still fails
/// closed, untouched.** A corpus with no pins has declared nothing; the gate's whole
/// question is *"does the world still match the pins"*, and over zero pins that is
/// vacuously yes. Nothing is unknown because nothing was asked.
///
/// # The old doctrine survives as the CALLER'S choice, and the next arm holds it
/// A shell script cannot read a disclosure line, so `--require-pins` puts the
/// fail-closed reading in the EXIT CODE for callers that want it. It is opt-in
/// because a fail-closed default would turn this gate into a coverage mandate nobody
/// ruled and make it un-adoptable on every vault that has not started pinning.
///
/// This is **U5 corpus row 09**, tagged [LAW], and the sharpest movement in the
/// corpus alongside row 08.
#[test]
fn a_pinless_corpus_passes_by_default() {
    let sb = sandbox();
    let ws = sb.corpus("pinless");

    let gated = sb.run(&ws, &["check", "--commit-gate"]);
    assert_eq!(
        code(&gated),
        0,
        "[LAW] corpus row 09: nothing is claimed, so nothing is unknown: {}",
        said(&gated)
    );
    let told = said(&gated);
    assert!(
        told.contains("pins-hold"),
        "the passing word names the plane that answered: {told}"
    );
    assert!(
        told.contains(check::WRITE_HISTORY_NOT_ASSESSED),
        "AND THE DISCLOSURE RIDES WITH IT — the pass is narrower than the pass this \
         gate once gave, and the line saying so is what keeps a reader from banking \
         the wider one: {told}"
    );

    // The population, on the machine face: `permits: true` over zero pins and over
    // fifty are different assurances (S3-R23(5)).
    let json: serde_json::Value =
        serde_json::from_str(&stdout(&sb.run(&ws, &["check", "--commit-gate", "--json"])))
            .expect("json");
    assert_eq!(
        json["commit_gate"]["permits"], true,
        "permitted: {}",
        json["commit_gate"]
    );
    assert_eq!(
        json["commit_gate"]["pin_coverage"], 0,
        "and the caller is TOLD the population it passed over, so a machine can see \
         what a human reads in the disclosure: {}",
        json["commit_gate"]
    );
}

/// **Was ARM 8 — same antecedent, same reversal.** A pinless corpus plus an
/// out-of-band write still passes, because there is no pin for the write to
/// contradict.
///
/// # This is U5 corpus row 08, and it is the largest reduction in enforcement here
/// The old gate refused this via `Accounted::ForeignBytes`: the record had produced
/// no such root. **It permits now**, because the record is gone and the pin plane is
/// all that gates — and the pin plane has nothing to say about content nobody
/// claimed.
///
/// The forged bytes are caught at the **write door**, and at lock time by git.
/// `check` is at-rest truth, and at rest this world matches its (zero) pins. U5
/// flagged this row for U25 re-verdict; it is ruled, and this arm is where the
/// ruling is executable rather than only written down.
///
/// **The contrast arm is the point:** the same forgery against a corpus that DOES
/// pin the file is refused ([`an_out_of_band_write_to_pinned_content_is_refused`]).
/// The difference is coverage, not the gate going soft.
#[test]
fn a_pinless_corpus_passes_even_with_an_out_of_band_write() {
    let sb = sandbox();
    let ws = sb.corpus("pinless-forged");
    rewrite_pinned_content(&ws, "rewritten by hand, and nothing pins it");
    git(&ws, &["add", "-A"]);

    let gated = sb.run(&ws, &["check", "--commit-gate"]);
    assert_eq!(
        code(&gated),
        0,
        "[LAW] corpus row 08: no pin covers these bytes, so the gate has no claim \
         to check them against: {}",
        said(&gated)
    );

    let head_before = git_head(&ws);
    let (commit_code, commit_said) = sb.commit(&ws, "out-of-band over a pinless corpus");
    assert_eq!(commit_code, 0, "and the commit LANDS: {commit_said}");
    assert_ne!(
        head_before,
        git_head(&ws),
        "R40 — HEAD moved, so this is a real reduction and not a rendering: \
         {commit_said}"
    );
}

/// **THE OPT-IN FLAG: the fail-closed doctrine, preserved as a caller's choice.**
/// `--require-pins` refuses exactly the corpus the two arms above let through, and
/// it refuses it with its OWN word rather than borrowing grey's.
///
/// # Both directions, so the flag is shown to discriminate
/// The same run measures the flag refusing a pinless corpus AND permitting a pinned
/// one. A flag that refused everything would satisfy the first half alone, and would
/// be a strictly worse gate than the one it was added to.
///
/// # Why the word is `no-pin-coverage` and not `grey(cannot-assess)`
/// Grey means *a question was put and could not be answered*. That is not this:
/// every question that was put WAS answered — there were none. Spelling it grey
/// would tell the operator the gate tried and failed, which is the one thing that
/// did not happen.
#[test]
fn require_pins_refuses_a_pinless_corpus_and_permits_a_pinned_one() {
    let sb = sandbox();

    // REFUSAL — no pin declared, and the caller asked to refuse exactly that.
    let bare = sb.corpus("require-pins-bare");
    let strict = sb.run(&bare, &["check", "--commit-gate", "--require-pins"]);
    assert_eq!(
        code(&strict),
        1,
        "the caller asked for coverage and there is none: {}",
        said(&strict)
    );
    assert!(
        said(&strict).contains("no-pin-coverage"),
        "with its own reason word: {}",
        said(&strict)
    );
    assert!(
        !said(&strict).contains("grey(cannot-assess)"),
        "and NOT grey — nothing here was unanswerable, there was simply nothing to \
         ask: {}",
        said(&strict)
    );

    // The default over the SAME corpus, in the same run: this is the discrimination.
    let lax = sb.run(&bare, &["check", "--commit-gate"]);
    assert_eq!(
        code(&lax),
        0,
        "and without the flag the same corpus passes — the strictness is the \
         CALLER'S, not the engine's: {}",
        said(&lax)
    );

    // ACCEPTANCE — the flag is not a gate wired shut.
    let pinned = sb.corpus("require-pins-covered");
    sb.pin_a_claim(&pinned, "claim.md");
    let covered = sb.run(&pinned, &["check", "--commit-gate", "--require-pins"]);
    assert_eq!(
        code(&covered),
        0,
        "a corpus WITH coverage passes under the flag — otherwise it would be a \
         refusal wired to a constant: {}",
        said(&covered)
    );

    // A grey pin still fails closed WITH OR WITHOUT the flag; that leg is not this
    // flag's business and the flag must not be read as having introduced it.
    assert!(
        !said(&covered).contains("no-pin-coverage"),
        "coverage exists, so the flag has nothing to say: {}",
        said(&covered)
    );
}

/// **The flag means nothing without the question, and says so** rather than being
/// silently ignored. A caller who typed `--require-pins` believes a gate is being
/// tightened; answering confidently about a question that was never asked is the one
/// shape a fence's verb must never produce.
#[test]
fn require_pins_without_the_gate_is_a_bad_invocation() {
    let sb = sandbox();
    let ws = sb.corpus("flag-alone");

    let out = sb.run(&ws, &["check", "--require-pins"]);
    assert_eq!(
        code(&out),
        2,
        "a bad invocation rides exit 2, never a verdict code: {}",
        said(&out)
    );
    assert!(
        said(&out).contains("--commit-gate"),
        "and it names the flag it needs: {}",
        said(&out)
    );
}

// ── VOCABULARY AND SHAPE ─────────────────────────────────────────────────────

/// **What survives of ARM 6.** Its corpus (a broken chain) is gone, but its claim
/// was about VOCABULARY — a gated pass must never be spelled in a word stronger than
/// the evidence — and that claim outlived the corpus.
///
/// The word to hold honest changed with the plane. `accounted` and
/// `accounted(unvouched-record)` both asserted something about a RECORD that no
/// longer exists; **a passing word that asserts an unobserved property is the same
/// defect as an enum member that does, wearing a different coat.** `pins-hold` names
/// the plane that actually answered.
///
/// `record_vouches` and `standing_report` are asserted ABSENT: there is no ledger
/// left to vouch for itself, so a key saying whether it does would be answering
/// about nothing.
#[test]
fn the_passing_word_names_the_plane_that_answered() {
    let sb = sandbox();
    let ws = sb.corpus("vocabulary");
    sb.pin_a_claim(&ws, "claim.md");

    let gated = sb.run(&ws, &["check", "--commit-gate"]);
    assert_eq!(code(&gated), 0, "it passes: {}", said(&gated));

    let verdict = stdout(&gated)
        .lines()
        .find(|l| l.trim_start().starts_with("commit-gate:"))
        .expect("the gate states its verdict")
        .to_owned();
    assert!(
        verdict.contains("pins-hold"),
        "the pass names the plane that answered: {verdict}"
    );
    assert!(
        !verdict.contains("accounted"),
        "and never the retired word — it asserted that a RECORD accounted for these \
         bytes, which is a claim this verb can no longer make: {verdict}"
    );
    assert!(
        !verdict.contains("green"),
        "nor the strong one — an operator must not bank a claim wider than the \
         plane that produced it: {verdict}"
    );

    let json: serde_json::Value =
        serde_json::from_str(&stdout(&sb.run(&ws, &["check", "--commit-gate", "--json"])))
            .expect("the gated json face parses");
    let gate = &json["commit_gate"];
    assert_eq!(gate["permits"], true, "the commit is permitted: {gate}");
    assert_eq!(
        gate["verdict"], "pins-hold",
        "the same word on both faces (S3-R6): {gate}"
    );
    assert_eq!(
        gate["gated_planes"],
        serde_json::json!(["pins"]),
        "and the face NAMES the one plane it gated on, so a machine reader learns \
         the narrowing the same way a human does: {gate}"
    );
    assert!(
        gate.get("record_vouches").is_none(),
        "REMOVED with the record it reported on: {gate}"
    );
    assert!(
        gate.get("standing_report").is_none(),
        "likewise — there is no standing break to report: {gate}"
    );
}

/// **The shipped `--json` face is untouched when the question is not asked.** The
/// gate block is ABSENT without `--commit-gate`, per this face's own law — *an
/// absent field reads as "not checked"* — and a `null` would instead claim the gate
/// was asked and had nothing to say.
#[test]
fn the_shipped_json_face_gains_nothing_until_the_gate_is_asked() {
    let sb = sandbox();
    let ws = sb.corpus("json-shape");
    sb.pin_a_claim(&ws, "claim.md");

    let plain: serde_json::Value =
        serde_json::from_str(&stdout(&sb.run(&ws, &["check", "--json"]))).expect("json");
    assert!(
        plain.get("commit_gate").is_none(),
        "the key is absent when the question was never put: {plain}"
    );

    let gated: serde_json::Value =
        serde_json::from_str(&stdout(&sb.run(&ws, &["check", "--commit-gate", "--json"])))
            .expect("json");
    assert!(
        gated.get("commit_gate").is_some(),
        "and present when it was: {gated}"
    );
    // The population keys ride INSIDE the gate block, so the shipped `pins` block
    // is byte-identical and no existing consumer moves.
    assert!(
        plain["pins"].get("pin_coverage").is_none(),
        "the coverage count is the GATE's reading, not a new key on the pin plane: \
         {plain}"
    );
}
