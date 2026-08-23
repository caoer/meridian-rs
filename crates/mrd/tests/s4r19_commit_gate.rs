//! S4-R19 — `mrd check --commit-gate`: does the interval a commit records
//! carry a pin that no longer holds?
//!
//! The gate reads the pin plane and nothing else: `permits() ==
//! !pins.is_red() && !pins.cannot_assess()`. An out-of-band edit is caught
//! because the rewritten file is a pinned target, so the pin plane reads
//! `red content-drifted`.
//!
//! Refusal arms sit beside acceptance arms built from the same fixture, so no
//! arm can be satisfied by a gate wired to a constant.
//!
//! The fence here is a test fixture, not the shipped one: the harness places
//! its own `pre-commit` that execs the scoped question, so these arms measure
//! `--commit-gate` itself. `skill_hook_emit.rs` holds the shipped document;
//! `hook_plane_fence.rs` drives the real emitted body.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
mod common;

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
        common::mrd_command(&self.home, &self.cache_home)
            .args(args)
            .current_dir(cwd)
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

    /// A real git repo with a pinnable source and three claim pages,
    /// `mrd init`, one commit, then the fence. Nothing here declares a pin —
    /// that is [`Sandbox::pin_a_claim`]'s job.
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

    /// One governed write through the engine's own door: `mrd pin` declares a
    /// claim against a section of `source.md`, then commits it. The pin is the
    /// only thing the gate reads, and the committed interval is the axis.
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

/// Rewrite the pinned section by hand — an out-of-band edit through no
/// meridian writer, to content a lock names.
fn rewrite_pinned_content(ws: &Path, marker: &str) {
    write(
        ws,
        "source.md",
        &format!("# Source\n\n## Guideline\n\n{marker}\n"),
    );
}

// ── the instrument's own control ─────────────────────────────────────────────

/// The gate can both accept and refuse — without this the whole file could be
/// satisfied by a gate wired to a constant.
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
    // Staged, because the gate reads the index — the state a pre-commit fence
    // actually fires in.
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

/// Boundary: an out-of-band edit that is not staged does not refuse — the
/// index still carries clean bytes, and those are the bytes a commit would
/// record.
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

/// A corpus whose pins hold passes both questions, so a gate that learned to
/// say "no" to everything is caught here.
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

/// An out-of-band write to pinned content is refused, naming the interval.
/// The refusal carries the pin plane's reason word, `red content-drifted` —
/// the same word `mrd walk` and `mrd status` print over the same corpus.
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

/// The specificity arm: an out-of-band write staged in the index with the
/// worktree restored to clean bytes. The gate must still refuse.
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

/// Ruled: a corpus that declares no pin passes `--commit-gate` — exit 0,
/// verdict `pins-hold`. Zero pins is vacuous truth, not unknown; fail-closed
/// protects the case where the gate cannot assess something it claims to
/// gate, and there is nothing here to assess.
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

/// A pinless corpus plus an out-of-band write still passes: there is no pin
/// for the write to contradict, and the pin plane is all that gates. The
/// contrast arm is the point — the same forgery against a corpus that does
/// pin the file is refused
/// ([`an_out_of_band_write_to_pinned_content_is_refused`]). The difference is
/// coverage, not the gate going soft.
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

/// The opt-in flag: `--require-pins` refuses exactly the corpus the two arms
/// above let through, with its own reason word rather than a borrowed grey.
/// The same run measures the flag refusing a pinless corpus and permitting a
/// pinned one, so the flag is shown to discriminate.
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

    // A grey pin still fails closed with or without the flag; that leg is not
    // this flag's business.
    assert!(
        !said(&covered).contains("no-pin-coverage"),
        "coverage exists, so the flag has nothing to say: {}",
        said(&covered)
    );
}

/// The flag means nothing without the question, and says so rather than being
/// silently ignored.
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

/// A gated pass must never be spelled in a word stronger than the evidence;
/// the word to hold honest is `pins-hold`.
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

/// The shipped `--json` face is untouched when the question is not asked: the
/// gate block is absent without `--commit-gate` — an absent field reads as
/// "not checked", where a `null` would claim the gate was asked.
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

impl Drop for Sandbox {
    fn drop(&mut self) {
        // Reap the daemon this sandbox auto-spawned (common::reap_daemon documents
        // the fixture daemon strategy). Runs before the TempDir fields drop, so
        // the pidfile is still on disk; never panics.
        let _ = common::reap_daemon(&self.home, &self.cache_home);
    }
}
