//! **S4-R19 — three propositions rode one exit code, and the fence spent the
//! permanent one on every commit.**
//!
//! `mrd check --staged` answers with a deliberate two-part shape: the exit says
//! *"may this proceed?"* and the reason word says why (`check_cmd.rs` § exit
//! triad). A pre-commit fence branches on the code alone, so three facts arrived
//! at it as one byte:
//!
//! | | proposition | about | permanence |
//! |---|---|---|---|
//! | **P1** | the journal chain has a break | the **past** | permanent by design |
//! | **P2** | this index carries an out-of-band write | **this interval** | per-commit |
//! | **P3** | the journal cannot date the tree | **evidence availability** | per-state |
//!
//! Past the first break P1 is permanently true, so the exit stopped varying with
//! what was staged. A guard whose verdict no longer depends on the thing it guards
//! carries zero information about it, and the only remaining paths are
//! `MRD_HOOK_FORCE` and `--no-verify` — so the P2 enforcement is destroyed as well.
//! **That is the defect: not that the break is permanent (it is, correctly), but
//! that a permanent fact about the ledger's history is spent as a per-commit
//! verdict about bytes it never examined.**
//!
//! `--commit-gate` asks the per-commit question instead. This unit measures that
//! it discriminates.
//!
//! # THE INSTRUMENT, and why every arm here can fail
//! The subject is a check that could not discriminate, so an arm that passes both
//! before and after the change is decoration. Two devices make these arms
//! falsifiable:
//!
//! 1. **The pre-fix invocation is measured in the same run.** `mrd check --staged`
//!    is unchanged and still present, so every arm asserts BOTH codes over ONE
//!    corpus. The discrimination is the DIFFERENCE between them
//!    ([`arm1_chain_broken_plus_governed_staged_write_commits_and_tells_the_break`]
//!    asserts `--staged` is 1 while `--commit-gate` is 0). A fix that unblocked by
//!    weakening the verb would move both codes and fail the `--staged` half.
//! 2. **Acceptance and refusal arms in the same run** (S3-R99). A fence that
//!    permits and a fence that is silently broken look identical from the caller,
//!    so arms 3 and 4 prove it still says NO beside the arms that prove it says
//!    YES. Arm 4 is the specificity arm: it fails if the gate was implemented as
//!    "ignore the journal", which would trade a stuck-closed fence for a
//!    stuck-open one.
//!
//! # What each arm needs broken to fail
//! - **1** — delete the scoped question, or let the worktree's chain break reach
//!   the exit again: exit 1, no commit.
//! - **2** — unblock by disabling the gate: `--staged` stops refusing anything.
//! - **3, 4** — gate on the journal alone, or ignore it: an out-of-band write
//!   lands in history.
//! - **5** — let the break heal or be acknowledged away: unscoped `check` goes
//!   green.
//! - **6** — spell the weakened pass `green`: the operator banks a claim the
//!   evidence does not support.
//! - **7** — fold `NoRecord` into the tampering leg: every fresh clone is accused.
//! - **8** — fail open on an absent record: an unvouched corpus commits freely.
//!
//! # The fence here is a TEST FIXTURE, not `mrd hook`
//! `crates/mrd/src/hook.rs` is out of this card's scope and is owned by a card
//! running in parallel. This harness installs its own `pre-commit` that execs the
//! scoped question — exactly as `u35_journaled_doors` installs one that execs
//! `mrd check` — so the real-`git commit` arms are driven with zero edits to the
//! hook plane.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use fs::WorkspaceRoot;
use fs::domain::RESERVED_JOURNAL_PATH;
use receipt::journal::{ParsedRow, parse_rows};

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
    /// one commit, then the fence. **Nothing here writes a journal row.**
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

    /// One governed write through the engine's own door — `mrd pin` journals its
    /// row (U32), so this is what "produced by a governed write" means here.
    fn governed_write(&self, ws: &Path, claim: &str) {
        let pin = self.run(ws, &["pin", claim, "source.md#Source/Guideline"]);
        assert_eq!(code(&pin), 0, "the governed write lands: {}", said(&pin));
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

/// stdout+stderr together — the render rides stdout, the refusal and the STANDING
/// REPORT ride stderr, and "what did the operator see" is a question about both.
fn said(out: &Output) -> String {
    format!(
        "{}{}",
        stdout(out),
        String::from_utf8_lossy(&out.stderr).into_owned()
    )
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
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

fn root_of(ws: &Path) -> WorkspaceRoot {
    WorkspaceRoot(workspace::canonicalize(ws).expect("canonicalize"))
}

fn journal_page(root: &WorkspaceRoot) -> String {
    std::fs::read_to_string(root.0.join(RESERVED_JOURNAL_PATH)).unwrap_or_default()
}

fn rows(root: &WorkspaceRoot) -> Vec<ParsedRow> {
    parse_rows(&journal_page(root))
}

/// **Break the chain the way a hand-inserted row breaks it** — alter the last
/// row's `root_before` so it no longer continues the row before it.
///
/// The journal is root-EXCLUDED from the hash domain, so this moves no tree fold:
/// the last row's `root_after` still dates the live tree, the baseline stays
/// CURRENT, and the trace therefore reaches `check_chain` and reddens. That is P1
/// in its pure form — a finding about the PAST, with the present fully governed.
///
/// Returns the anchor of the row the break is cited at.
fn break_the_chain(root: &WorkspaceRoot) -> String {
    let page = journal_page(root);
    let parsed = parse_rows(&page);
    assert!(
        parsed.len() >= 2,
        "a chain break needs a PAIR to fail: {parsed:#?}"
    );
    let victim = parsed.last().expect("the last row").clone();
    let broken = page.replace(
        &format!("root_before={}", victim.root_before),
        "root_before=b3:FORGED_PREDECESSOR",
    );
    assert_ne!(broken, page, "the fixture actually altered the page");
    std::fs::write(root.0.join(RESERVED_JOURNAL_PATH), broken).expect("write journal");
    victim.anchor
}

/// The corpus every chain-break arm starts from: two governed writes, the chain
/// broken between them, and the break CONFIRMED red before anything is measured.
/// Returns the workspace and the anchor the break cites.
fn chain_broken_corpus(sb: &Sandbox, name: &str) -> (PathBuf, String) {
    let ws = sb.corpus(name);
    sb.governed_write(&ws, "claim.md");
    sb.governed_write(&ws, "claim2.md");
    let root = root_of(&ws);
    let anchor = break_the_chain(&root);

    // The fixture is only a fixture if it produced the state it claims to.
    let unscoped = sb.run(&ws, &["check"]);
    assert_eq!(
        code(&unscoped),
        1,
        "FIXTURE: the chain is broken, so unscoped check refuses: {}",
        said(&unscoped)
    );
    assert!(
        said(&unscoped).contains(&anchor),
        "FIXTURE: and it cites the broken row {anchor}: {}",
        said(&unscoped)
    );
    (ws, anchor)
}

// ── the instrument's own control ─────────────────────────────────────────────

/// **The gate can both accept and refuse.** Without this the whole file could be
/// satisfied by a gate wired to a constant, and a constant is precisely the defect
/// under test wearing the other sign.
#[test]
fn the_gate_accepts_governed_work_and_refuses_an_out_of_band_edit() {
    let sb = sandbox();
    let ws = sb.corpus("control");
    sb.governed_write(&ws, "claim.md");

    let accepts = sb.run(&ws, &["check", "--commit-gate"]);
    assert_eq!(
        code(&accepts),
        0,
        "governed work passes the gate: {}",
        said(&accepts)
    );

    write(
        &ws,
        "source.md",
        "# Source\n\n## Guideline\n\nrewritten by hand\n",
    );
    // STAGED, because the gate reads the index — which is the state a pre-commit
    // fence actually fires in, and the reason the interval exists at all (F1).
    git(&ws, &["add", "-A"]);
    let refuses = sb.run(&ws, &["check", "--commit-gate"]);
    assert_eq!(
        code(&refuses),
        1,
        "and an out-of-band edit does not: {}",
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
/// still carries governed bytes, and those are the bytes a commit would record.
///
/// This is F1 working in the direction it was built for — *"the bytes on your disk
/// are fine"* and *"the bytes you are about to commit are not"* are different
/// instructions, and so are their converses. The forgery is refused the instant it
/// is staged, which [`the_gate_accepts_governed_work_and_refuses_an_out_of_band_edit`]
/// measures directly above.
///
/// Measured while writing this unit: the first draft of that control asserted a
/// refusal here and failed, because it read the worktree's forgery as if it were
/// the commit's. Left as an arm so the next reader does not repeat it.
#[test]
fn an_unstaged_out_of_band_edit_does_not_gate_a_governed_index() {
    let sb = sandbox();
    let ws = sb.corpus("boundary");
    sb.governed_write(&ws, "claim.md");
    write(
        &ws,
        "source.md",
        "# Source\n\n## Guideline\n\nrewritten by hand, and NOT staged\n",
    );

    let gated = sb.run(&ws, &["check", "--commit-gate"]);
    assert_eq!(
        code(&gated),
        0,
        "the index carries governed bytes, and the index is what a commit records: {}",
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

// ── ACCEPTANCE ARMS ──────────────────────────────────────────────────────────

/// **ARM 1 — THE REDDEN.** A chain-broken history plus a fully governed staged
/// write: the commit LANDS, and the standing break is told anyway.
///
/// This is the arm that fails against the pre-fix behaviour, and it says so in the
/// same run: `mrd check --staged` — the shipped invocation, unchanged — refuses
/// this exact corpus with exit 1, while the scoped question accepts it with exit 0.
/// **The assertion is the DIFFERENCE**, so it cannot be satisfied by a gate that
/// merely stopped refusing: arm 2 and arms 3-4 hold that door shut.
#[test]
fn arm1_chain_broken_plus_governed_staged_write_commits_and_tells_the_break() {
    let sb = sandbox();
    let (ws, anchor) = chain_broken_corpus(&sb, "arm1");

    // A third governed write, on top of the broken history. Fully governed: the
    // engine wrote every byte of it.
    sb.governed_write(&ws, "claim3.md");

    // THE PRE-FIX BEHAVIOUR, measured live in this same run.
    let staged = sb.run(&ws, &["check", "--staged"]);
    assert_eq!(
        code(&staged),
        1,
        "PRE-FIX: `check --staged` refuses this corpus — the permanent P1 spent on a \
         per-commit question: {}",
        said(&staged)
    );

    // THE SCOPED QUESTION over the identical bytes.
    let gated = sb.run(&ws, &["check", "--commit-gate"]);
    assert_eq!(
        code(&gated),
        0,
        "the interval a commit records IS accounted for, so the gate permits: {}",
        said(&gated)
    );
    assert_ne!(
        code(&staged),
        code(&gated),
        "THE DISCRIMINATION: one corpus, two questions, two answers — this is the \
         whole deliverable, and it is what a stale exit code could not express"
    );

    // C2 — downgrade the blocking, never the telling.
    let told = stderr(&gated);
    assert!(
        told.contains(&anchor),
        "the STANDING BREAK is told on the passing run, citing the same row {anchor}: {told}"
    );
    assert!(
        told.contains("STANDING"),
        "and it is named a standing fact, not a verdict about the commit: {told}"
    );

    // R40 — the assert is the STATE CHANGE, not the exit code.
    let head_before = git_head(&ws);
    let (commit_code, commit_said) = sb.commit(&ws, "governed work over a broken chain");
    let head_after = git_head(&ws);
    assert_eq!(
        commit_code, 0,
        "and a REAL commit lands through the fence: {commit_said}"
    );
    assert_ne!(
        head_before, head_after,
        "HEAD moved — the governed commit is history: {commit_said}"
    );
    assert!(
        commit_said.contains(&anchor),
        "the operator was told the standing break at commit time too: {commit_said}"
    );
}

/// **ARM 2 — a clean chain still passes.** Guards against a fix that unblocks by
/// disabling the gate: here BOTH questions must answer 0, so a gate that learned to
/// say "yes" to everything is invisible to arm 1 but not to arms 3-4, and a verb
/// that broke `--staged` outright is caught right here.
#[test]
fn arm2_clean_chain_plus_governed_staged_write_passes_both_questions() {
    let sb = sandbox();
    let ws = sb.corpus("arm2");
    sb.governed_write(&ws, "claim.md");

    let staged = sb.run(&ws, &["check", "--staged"]);
    let gated = sb.run(&ws, &["check", "--commit-gate"]);
    assert_eq!(code(&staged), 0, "the shipped question: {}", said(&staged));
    assert_eq!(code(&gated), 0, "and the scoped one: {}", said(&gated));
    assert!(
        stderr(&gated).is_empty(),
        "an intact record has NO standing break to tell — a report printed \
         unconditionally would be noise that teaches operators to ignore it: {}",
        stderr(&gated)
    );

    let (commit_code, commit_said) = sb.commit(&ws, "governed work over a clean chain");
    assert_eq!(commit_code, 0, "the commit lands: {commit_said}");
}

// ── REFUSAL ARMS, in the same run ────────────────────────────────────────────

/// **ARM 3 — a clean chain plus an out-of-band write is refused**, naming the
/// interval. The per-commit proposition P2 must survive the change untouched; if
/// the scoped question were "ignore the journal", this arm is where that shows.
#[test]
fn arm3_clean_chain_plus_out_of_band_write_is_refused() {
    let sb = sandbox();
    let ws = sb.corpus("arm3");
    sb.governed_write(&ws, "claim.md");

    write(
        &ws,
        "source.md",
        "# Source\n\n## Guideline\n\nrewritten by hand, through no writer\n",
    );
    // Staged: this is the state the fence fires in, and the bytes a commit records.
    git(&ws, &["add", "-A"]);

    let gated = sb.run(&ws, &["check", "--commit-gate"]);
    assert_eq!(
        code(&gated),
        1,
        "the bytes being committed were NOT produced by a governed write: {}",
        said(&gated)
    );
    assert!(
        said(&gated).contains("no governed write ever produced"),
        "and the refusal says which condition failed: {}",
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

/// **ARM 4 — THE SPECIFICITY ARM.** A chain-broken history plus an out-of-band
/// write staged in the INDEX, with the worktree restored to its governed bytes
/// (the F1 shape). The gate must still refuse.
///
/// It fails if the scoped question was implemented as "ignore the journal", which
/// would trade a stuck-closed fence for a stuck-open one — the same defect with the
/// sign flipped, and a strictly worse outcome than the one this card started from.
/// It also fails if the gate read the WORKTREE here: the worktree is fully governed
/// at this instant, so a gate pointed at the wrong interval passes cleanly while a
/// forgery goes into history.
#[test]
fn arm4_chain_broken_plus_out_of_band_staged_write_is_still_refused() {
    let sb = sandbox();
    let (ws, _anchor) = chain_broken_corpus(&sb, "arm4");
    let governed = std::fs::read_to_string(ws.join("source.md")).expect("governed bytes");

    // Stage a forgery, then restore the worktree: the index carries bytes no
    // governed write produced, while the bytes on disk are perfectly fine.
    write(
        &ws,
        "source.md",
        "# Source\n\n## Guideline\n\nforged, then hidden from the worktree\n",
    );
    git(&ws, &["add", "source.md"]);
    write(&ws, "source.md", &governed);

    let gated = sb.run(&ws, &["check", "--commit-gate"]);
    assert_eq!(
        code(&gated),
        1,
        "the INDEX carries a forgery, so the gate refuses even though the worktree \
         is governed: {}",
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

// ── NEVER-HEALS ARMS ─────────────────────────────────────────────────────────

/// **ARM 5 — the break never heals.** After arm 1's commit lands, the unscoped verb
/// still reports it, red, citing the same row anchor. No acknowledgement state
/// exists to clear it, and a governed write on top does not launder it.
///
/// Fails the moment the downgrade is implemented as healing rather than as scoping.
#[test]
fn arm5_the_break_never_heals_after_a_gated_commit_lands() {
    let sb = sandbox();
    let (ws, anchor) = chain_broken_corpus(&sb, "arm5");
    sb.governed_write(&ws, "claim3.md");

    let (commit_code, commit_said) = sb.commit(&ws, "governed work over a broken chain");
    assert_eq!(commit_code, 0, "arm 1's commit lands: {commit_said}");

    let after = sb.run(&ws, &["check"]);
    assert_eq!(
        code(&after),
        1,
        "the unscoped claim 'this corpus's write history is intact' is dead PERMANENTLY: {}",
        said(&after)
    );
    assert!(
        said(&after).contains(&anchor),
        "and it cites the SAME row {anchor} it always did: {}",
        said(&after)
    );

    // A further governed write does not launder it either.
    sb.governed_write(&ws, "claim2.md");
    let later = sb.run(&ws, &["check"]);
    assert_eq!(code(&later), 1, "still refusing: {}", said(&later));
    assert!(
        said(&later).contains(&anchor),
        "still citing {anchor}: {}",
        said(&later)
    );
}

/// **ARM 6 — a gated pass over a broken record is NEVER spelled green.** With the
/// chain broken, the record that accounted for the interval may itself hold a
/// forged row, so the acceptance is genuinely weaker and must be rendered as the
/// weaker claim it is. Hiding that would be this card's defect wearing the other
/// sign.
#[test]
fn arm6_a_gated_pass_over_a_broken_record_is_not_spelled_green() {
    let sb = sandbox();
    let (ws, _anchor) = chain_broken_corpus(&sb, "arm6");
    sb.governed_write(&ws, "claim3.md");

    let gated = sb.run(&ws, &["check", "--commit-gate"]);
    assert_eq!(code(&gated), 0, "it passes: {}", said(&gated));

    let verdict = stdout(&gated)
        .lines()
        .find(|l| l.trim_start().starts_with("commit-gate:"))
        .expect("the gate states its verdict")
        .to_owned();
    assert!(
        verdict.contains("accounted(unvouched-record)"),
        "the pass carries the WEAKER word: {verdict}"
    );
    assert!(
        !verdict.contains("green"),
        "and never the strong one — an operator must not bank a claim the evidence \
         does not support: {verdict}"
    );

    // The same fact on the --json face, where a machine reads it.
    let json = sb.run(&ws, &["check", "--commit-gate", "--json"]);
    let value: serde_json::Value =
        serde_json::from_str(&stdout(&json)).expect("the gated json face parses");
    let gate = &value["commit_gate"];
    assert_eq!(
        gate["permits"],
        true,
        "the commit is permitted: {}",
        stdout(&json)
    );
    assert_eq!(
        gate["record_vouches"],
        false,
        "AND the record does not vouch for itself — two facts, never one byte: {}",
        stdout(&json)
    );
}

/// **The shipped `--json` face is untouched when the question is not asked.** The
/// gate block is ABSENT without `--commit-gate`, per this face's own law — *an
/// absent field reads as "not checked"* — and a `null` would instead claim the gate
/// was asked and had nothing to say.
///
/// Measured while writing this unit: emitting `"commit_gate": null`
/// unconditionally broke two `check_e2e` arms that assert the object key for key.
/// They were right to break, and the shape was what was wrong.
#[test]
fn the_shipped_json_face_gains_nothing_until_the_gate_is_asked() {
    let sb = sandbox();
    let ws = sb.corpus("json-shape");
    sb.governed_write(&ws, "claim.md");

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
}

// ── DRIFT ARMS — the ruled cache-resident journal ─────────────────────────────

/// **ARM 7 — an empty record is distinguishable from a chain break, and does not
/// accuse.** Under the ruled cache-resident JSONL journal a fresh clone carries
/// zero rows, so P3 stops being an edge case and becomes the common one. It must
/// refuse — unknown is not clean — but it must refuse as an ABSENCE OF EVIDENCE,
/// not as tampering, and it must not read like arm 1's corpus.
///
/// Fails if `NoRecord` is folded into the tampering leg, which is exactly what a
/// bare `is_prefix_of` + `accounts_for` pair does: both conditions are false over an
/// empty record, so a naive pair spells "there is no ledger" as "you forged this",
/// and accuses every fresh clone.
#[test]
fn arm7_an_empty_record_refuses_without_accusing_and_is_not_a_chain_break() {
    let sb = sandbox();
    let ws = sb.corpus("arm7");
    let root = root_of(&ws);
    assert!(
        rows(&root).is_empty(),
        "FIXTURE: no governed write has happened, so the record is empty"
    );

    let gated = sb.run(&ws, &["check", "--commit-gate"]);
    assert_eq!(
        code(&gated),
        1,
        "unknown is not clean — it fails CLOSED: {}",
        said(&gated)
    );
    let told = said(&gated);
    assert!(
        told.contains("carries no row"),
        "it says the evidence is absent: {told}"
    );
    assert!(
        told.contains("tampered with nothing") || told.contains("not a finding against them"),
        "and says plainly that this is not an accusation: {told}"
    );
    assert!(
        told.contains("grey(cannot-assess)"),
        "carrying the reason word for an unanswerable question: {told}"
    );
    assert!(
        !told.contains("no governed write ever produced"),
        "and NOT the forgery wording — that is the accusation this arm exists to \
         keep off a fresh clone: {told}"
    );

    // DISTINGUISHABLE from arm 1's chain-break corpus, which is the claim.
    let (broken_ws, _) = chain_broken_corpus(&sb, "arm7-contrast");
    sb.governed_write(&broken_ws, "claim3.md");
    let broken = sb.run(&broken_ws, &["check", "--commit-gate"]);
    assert_ne!(
        code(&gated),
        code(&broken),
        "P3 and P1 reach different answers now — under the shipped verb both were \
         exit 1, which is the conflation this card names"
    );
}

/// **ARM 8 — an empty record plus an out-of-band write still fails closed.** The
/// widened grey leg must not become a hole: with nothing recorded, nothing is
/// accounted for, and a workspace that can vouch for nothing may commit nothing.
#[test]
fn arm8_an_empty_record_plus_an_out_of_band_write_fails_closed() {
    let sb = sandbox();
    let ws = sb.corpus("arm8");
    write(
        &ws,
        "source.md",
        "# Source\n\n## Guideline\n\nrewritten by hand\n",
    );
    git(&ws, &["add", "-A"]);

    let gated = sb.run(&ws, &["check", "--commit-gate"]);
    assert_eq!(
        code(&gated),
        1,
        "no record accounts for anything, so nothing passes: {}",
        said(&gated)
    );

    let head_before = git_head(&ws);
    let (commit_code, commit_said) = sb.commit(&ws, "out-of-band over an empty record");
    assert_ne!(commit_code, 0, "the fence REFUSES: {commit_said}");
    assert_eq!(
        head_before,
        git_head(&ws),
        "R40 — nothing entered history: {commit_said}"
    );
}
