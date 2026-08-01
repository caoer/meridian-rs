//! **U35 — a governed write through the run-plane byte-landing door now leaves a
//! journal row, so the installed fence accepts the commit.** This file IS U34's
//! harness with its expectations INVERTED: U34 measured the refusal, U35 removes
//! it, and the inversion is this unit's proof (S3-R12(b)).
//!
//! # What U34 measured, and what changed
//! Two byte-landing doors survived U31/U32 without a journal row:
//!
//! 1. **`mrd realise --truth file`** — deployed `conventions/INDEX.md` through
//!    `fs::replace_file`. **That door is GONE**: the registration cutover deleted
//!    the INDEX and `policy::binding::converge` with it, so the verb it hung on
//!    no longer exists. Its U35 leg went with it — a journal-row gate over a door
//!    that cannot be opened proves nothing, and keeping the leg would have meant
//!    keeping the door. The convergence law is re-owed at the ARM disk edge, and
//!    its journaling is owed there with it.
//! 2. **the run-plane `apply_batch`** — `executor::apply` lands page bytes
//!    through `fs::apply_batch`, never through the wire splice choke-point where
//!    U32's `render_row` call lives. This is the door this file still gates.
//!
//! U34 drove both over the shipped binary with the ratified fence installed and
//! measured, for each: `mrd check` `grey(cannot-assess)` EXIT 1, `git commit`
//! REFUSED, `git commit --no-verify` the operator's only route forward. U35 gives
//! the surviving door its row through the SAME row writer
//! (`receipt::journal::render_row`, U32's), so the last row's `root_after` IS the
//! live tree again and the fence accepts. **Every `assert_ne!(commit_code, 0)`
//! below was an `assert_eq!` in the other direction before this unit** — that
//! flip is the deliverable.
//!
//! # What "the installed fence" is here
//! U15 has not landed, so this harness installs the RATIFIED fence verbatim: a
//! `.git/hooks/pre-commit` that execs `mrd check` and lets its exit status decide
//! the commit. Every leg then attempts a REAL `git commit` — never an assertion
//! about a function.
//!
//! # BOTH arms, per door (S3-R8(c), gate 1)
//! A fence proven only by what it accepts is as uninformative as one proven only
//! by what it blocks. The door leg therefore drives the governed write (green,
//! commit ACCEPTED) **and then** an out-of-band shell rewrite on the same corpus
//! (grey, commit REFUSED). Two assertions, one door, plus
//! [`the_installed_fence_accepts_governed_work_and_refuses_an_out_of_band_edit`]
//! as the instrument's own control.
//!
//! # The assert is the STATE CHANGE (R40)
//! An exit code says a command ran. Each door leg asserts what it DID: the bytes
//! that landed, the tree root that moved, the journal row that now EXISTS with
//! `root_before` = the pre-door tree and `root_after` = the live tree — *that
//! staleness was the mechanism; its absence is the fix* — and then reads the
//! fence's answer as the consequence.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use fs::WorkspaceRoot;
use fs::domain::RESERVED_JOURNAL_PATH;
use receipt::journal::{ParsedRow, parse_rows};

/// The binary every drive goes through — the shipped CLI, never a library call.
/// `MRD_BIN` names another artifact (the fixv convention), so the same asserts can
/// be pointed at an older build.
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

    /// **The ratified fence, installed for real**: a `pre-commit` hook that execs
    /// `mrd check` and hands git its exit status. Zero markdown semantics — an
    /// adapter over the engine (U15's ratified shape).
    fn install_fence(&self, ws: &Path) {
        let hook = ws.join(".git/hooks/pre-commit");
        std::fs::create_dir_all(hook.parent().expect("hooks dir")).expect("mkdir hooks");
        std::fs::write(
            &hook,
            format!(
                "#!/bin/sh\n\
                 # the ratified stage-3 fence: an adapter over `mrd check`\n\
                 XDG_CACHE_HOME='{cache}' HOME='{home}' exec '{mrd}' check\n",
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
        assert!(hook.exists(), "the fence file is on disk");
    }

    /// The corpus both doors are cut from: a real git repo, a pinnable source
    /// section, a claim page, an editable plan page, `mrd init`, one commit —
    /// then the fence installed. **Nothing here writes a journal row.**
    fn corpus(&self, name: &str, extra: &[(&str, &str)]) -> PathBuf {
        let ws = self.tmp.path().join(name);
        std::fs::create_dir_all(&ws).expect("mkdir");
        git(&ws, &["init", "-q"]);
        git(&ws, &["config", "user.email", "u35@example.invalid"]);
        git(&ws, &["config", "user.name", "u35"]);
        git(&ws, &["config", "commit.gpgsign", "false"]);
        write(&ws, "source.md", "# Source\n\n## Guideline\n\nthe body\n");
        write(&ws, "claim.md", "# Claim\n\nwe rely on the guideline.\n");
        for (rel, body) in extra {
            write(&ws, rel, body);
        }
        let init = self.run(&ws, &["init"]);
        assert!(init.status.success(), "mrd init: {}", said(&init));
        git(&ws, &["add", "-A"]);
        git(&ws, &["commit", "-qm", "corpus"]);
        self.install_fence(&ws);
        ws
    }

    /// Attempt a REAL commit through the installed fence. Returns the commit's
    /// exit code and everything the operator would see (the hook's output rides
    /// git's streams).
    fn commit_through_fence(&self, ws: &Path, message: &str) -> (i32, String) {
        self.commit(ws, message, &[])
    }

    fn commit(&self, ws: &Path, message: &str, extra: &[&str]) -> (i32, String) {
        let add = Command::new("git")
            .arg("-C")
            .arg(ws)
            .args(["add", "-A"])
            .output()
            .expect("git add");
        assert!(add.status.success(), "git add: {}", said(&add));
        let out = Command::new("git")
            .arg("-C")
            .arg(ws)
            .args(["commit", "-m", message])
            .args(extra)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env_remove("MERIDIAN_WORKSPACE")
            .output()
            .expect("git commit");
        (out.status.code().expect("exit code"), said(&out))
    }
}

/// **Gate 4 — the `--no-verify` escape is no longer needed** (S3-R12(b) / R35).
/// U34's twin of this helper asserted the opposite: that the bypass was the
/// operator's ONLY route forward once the fence had refused a fully governed
/// commit. Here the ordinary commit is attempted with NO extra flag, and both the
/// exit code and the moved HEAD are read (R40 — an exit code alone says only that
/// git ran).
fn assert_the_fence_accepts_without_the_bypass(
    sb: &Sandbox,
    ws: &Path,
    door: &str,
    message: &str,
) -> String {
    let head_before = git_head(ws);
    let (commit_code, commit_said) = sb.commit_through_fence(ws, message);
    let head_after = git_head(ws);
    assert_eq!(
        commit_code, 0,
        "{door}: the installed fence ACCEPTS the governed commit, with NO --no-verify: {commit_said}"
    );
    assert_ne!(
        head_before, head_after,
        "{door}: R40 — HEAD moved, so the governed commit is history: {commit_said}"
    );
    eprintln!("   commit: git commit EXIT 0 · HEAD {head_before} -> {head_after}\n");
    commit_said
}

/// **Gate 1's other arm, per door** (S3-R8(c)): the same corpus, one plain shell
/// rewrite of the pinned section through no meridian writer at all, and the fence
/// must still REFUSE. Journaling the doors must not turn the fence into one that
/// accepts everything.
fn assert_an_out_of_band_edit_still_refuses(sb: &Sandbox, ws: &Path, door: &str) {
    write(
        ws,
        "source.md",
        "# Source\n\n## Guideline\n\nthe body, rewritten by hand\n",
    );
    let check = sb.run(ws, &["check"]);
    assert_eq!(
        code(&check),
        1,
        "{door}: an out-of-band edit still leaves check refusing: {}",
        said(&check)
    );
    assert!(
        said(&check).contains("cannot-assess"),
        "{door}: and it refuses GREY — the journal cannot date a tree it never \
         receipted: {}",
        said(&check)
    );
    let (commit_code, commit_said) = sb.commit_through_fence(ws, "out-of-band: a shell rewrite");
    assert_ne!(
        commit_code, 0,
        "{door}: the installed fence REFUSES the out-of-band commit: {commit_said}"
    );
    eprintln!(
        "   out-of-band arm: mrd check EXIT {} · git commit EXIT {commit_code}\n",
        code(&check)
    );
}

/// **R40, the state change this unit exists to produce**: the door appended
/// exactly ONE row, and that row DATES the write — `root_before` is the tree the
/// door started from, `root_after` is the tree it produced, and `root_after` is
/// the live tree now. The last fact is the mechanism: U34 measured
/// `last.root_after != live_root`, which is precisely what made the baseline stale
/// and the fence refuse.
fn assert_the_row_dates_the_write(
    root: &WorkspaceRoot,
    rows_around: (&[ParsedRow], &[ParsedRow]),
    identity: (&str, &str, &str),
    roots_around: (&str, &str),
) {
    let (before_rows, after_rows) = rows_around;
    let (op, path, actor) = identity;
    let (root_before, root_after) = roots_around;
    assert_eq!(
        after_rows.len(),
        before_rows.len() + 1,
        "the door appended exactly one journal row: {after_rows:#?}"
    );
    let row = after_rows.last().expect("the door's row");
    assert_eq!(row.op, op, "the row names the op that produced it");
    assert_eq!(row.path, path, "the row names the file the bytes landed in");
    assert_eq!(
        row.actor.as_deref(),
        Some(actor),
        "the row records the actor exactly as the door gave it"
    );
    assert_eq!(
        row.root_before, root_before,
        "the row's root_before IS the tree the door started from"
    );
    assert_eq!(
        row.root_after, root_after,
        "the row's root_after IS the tree the door produced"
    );
    assert_eq!(
        row.root_after,
        live_root(root),
        "AND the last row's root_after IS the live tree — the staleness U34 \
         measured is gone (R40)"
    );
    assert_eq!(
        before_rows.last().expect("the baseline row").root_after,
        row.root_before,
        "the row CONTINUES the chain from the baseline row"
    );
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

fn read(ws: &Path, rel: &str) -> String {
    std::fs::read_to_string(ws.join(rel)).expect("read")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// stdout+stderr together — the render rides stdout, the refusal rides stderr,
/// and "what did the operator see" is a question about both.
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

/// The current commit sha, or `"(unborn)"` before the first commit.
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

/// The live workspace tree root — what the journal's last row must account for.
fn live_root(root: &WorkspaceRoot) -> String {
    fs::domain_snapshot(root).expect("snapshot").1.0
}

/// The reserved journal page's raw bytes (empty when it does not exist yet).
fn journal_page(root: &WorkspaceRoot) -> String {
    std::fs::read_to_string(root.0.join(RESERVED_JOURNAL_PATH)).unwrap_or_default()
}

/// Every journal row on disk, read the way `check`'s detectors read it.
fn rows(root: &WorkspaceRoot) -> Vec<ParsedRow> {
    parse_rows(&journal_page(root))
}

/// Drive the corpus to the GREEN baseline every door leg starts from: one
/// governed `mrd pin` (which journals under U32), `mrd check` exit 0, and a
/// commit the fence lets through. Returns the quoted baseline render.
fn green_baseline(sb: &Sandbox, ws: &Path) -> String {
    let root = root_of(ws);
    assert!(
        rows(&root).is_empty(),
        "the corpus journals nothing before the baseline pin"
    );

    let pin = sb.run(ws, &["pin", "claim.md", "source.md#Source/Guideline"]);
    assert_eq!(
        code(&pin),
        0,
        "the baseline pin is governed: {}",
        said(&pin)
    );
    let baseline_rows = rows(&root);
    assert_eq!(
        baseline_rows.len(),
        1,
        "the governed splice journaled its row (U32): {baseline_rows:#?}"
    );
    assert_eq!(
        baseline_rows[0].root_after,
        live_root(&root),
        "the baseline is CURRENT — the row's root_after IS the live tree"
    );

    let check = sb.run(ws, &["check"]);
    assert_eq!(
        code(&check),
        0,
        "GATE 1 — the baseline is green before any door runs: {}",
        said(&check)
    );
    let render = stdout(&check);
    assert!(
        render.contains("chain: green") && render.contains("foreign_edit: none"),
        "the baseline render is the honest green: {render}"
    );

    let (commit_code, commit_said) = sb.commit_through_fence(ws, "governed: the baseline pin");
    assert_eq!(
        commit_code, 0,
        "and the installed fence ACCEPTS the governed commit: {commit_said}"
    );
    eprintln!(
        "── BASELINE ─────────────────────────────\n{render}\
               mrd check EXIT 0 · git commit EXIT {commit_code}\n"
    );
    render
}

// ── the instrument's positive control ────────────────────────────────────────

/// **The fence can both accept and refuse.** A fence proven only by what it
/// blocks is indistinguishable from a fence that blocks everything (S3-R8(c)),
/// and a door leg reporting "accepted" against an uncalibrated fence is not
/// evidence either. Same corpus, both answers.
#[test]
fn the_installed_fence_accepts_governed_work_and_refuses_an_out_of_band_edit() {
    let sb = sandbox();
    let ws = sb.corpus("control", &[]);
    green_baseline(&sb, &ws);

    // The refusal arm: one plain shell rewrite of the pinned section, through no
    // meridian writer at all — the human-in-Obsidian case the fence exists for.
    write(
        &ws,
        "source.md",
        "# Source\n\n## Guideline\n\nthe body, rewritten by hand\n",
    );
    let check = sb.run(&ws, &["check"]);
    assert_ne!(
        code(&check),
        0,
        "an out-of-band edit leaves check non-zero: {}",
        said(&check)
    );

    let (commit_code, commit_said) = sb.commit_through_fence(&ws, "out-of-band: a shell rewrite");
    assert_ne!(
        commit_code, 0,
        "GATE — the installed fence REFUSES the out-of-band commit: {commit_said}"
    );
    eprintln!(
        "── CONTROL (out-of-band) ────────────────\n{}\
         mrd check EXIT {} · git commit EXIT {commit_code}\n{commit_said}\n",
        said(&check),
        code(&check),
    );
}

// ── the surviving door — the run-plane `apply_batch` ─────────────────────────

/// A page carrying one starlark task whose `md.set_field` effect lands page bytes
/// through `executor::apply` → `fs::apply_batch` — the production run-plane path,
/// driven by the shipped `mrd run`.
const RUN_PAGE: &str = "\
---
task.fix-note: \"[[#^note-1]]\"
task.fix-note.caps: md.set_field
task.fix-note.args: value
---

# Tasks

```starlark
def run(ctx):
    set_field(field = \"status\", value = ctx.args[0])
```
^note-1
";

/// **THE DOOR.** A governed `mrd run` lands page bytes through the run plane's
/// `fs::apply_batch` — never through the wire splice choke-point where U32's
/// journal row is written — and journals its own row through the SAME row writer.
/// The installed fence is then asked for the next commit, and it accepts.
///
/// `realise.apply` faults on every attempt (the stage-4 card
/// `s4-realise-apply-invocation-id`), so the door is reached by `mrd run` — the
/// same `executor::apply` entry point, one caller over.
#[test]
fn run_plane_apply_batch_then_the_installed_fence() {
    let sb = sandbox();
    let ws = sb.corpus("run-plane", &[("tasks.md", RUN_PAGE)]);
    let root = root_of(&ws);
    green_baseline(&sb, &ws);

    let before_page = read(&ws, "tasks.md");
    let before_root = live_root(&root);
    let before_rows = rows(&root);

    // ── THE DOOR, through the shipped run verb.
    let door = sb.run(&ws, &["run", "tasks.md", "fix-note", "--", "done"]);
    assert_eq!(code(&door), 0, "the governed run applies: {}", said(&door));

    // ── R40: the STATE CHANGE.
    let after_page = read(&ws, "tasks.md");
    let after_root = live_root(&root);
    let after_rows = rows(&root);
    assert!(
        after_page.contains("status: done"),
        "(1) BYTES LANDED — the batch spliced the field in: {after_page}"
    );
    assert_ne!(before_page, after_page, "(1b) the page changed");
    assert_ne!(
        before_root, after_root,
        "(2) THE TREE ROOT MOVED — {before_root} -> {after_root}"
    );
    // (3) THE JOURNAL ROW — U34 asserted `grep -c journal crates/run/src/
    // executor.rs` = 0 and no row appeared.
    assert_the_row_dates_the_write(
        &root,
        (&before_rows, &after_rows),
        ("run", "tasks.md", "run:fix-note"),
        (&before_root, &after_root),
    );
    let line = journal_page(&root)
        .lines()
        .last()
        .expect("the door's row")
        .to_owned();

    // ── the consequence: `mrd check` is green and the fence accepts.
    let check = sb.run(&ws, &["check"]);
    eprintln!(
        "── the run-plane door · apply_batch ────\n\
         tree root {before_root} -> {after_root}\n\
         journal rows {} -> {} (+1: {line})\n\
         --- mrd check ---\n{}\
         mrd check EXIT {}\n",
        before_rows.len(),
        after_rows.len(),
        said(&check),
        code(&check),
    );
    assert_eq!(
        code(&check),
        0,
        "MEASURED (inverted from U34): check is GREEN after the governed run: {}",
        said(&check)
    );
    assert!(
        stdout(&check).contains("chain: green") && stdout(&check).contains("foreign_edit: none"),
        "and it renders the honest green: {}",
        stdout(&check)
    );
    assert_the_fence_accepts_without_the_bypass(
        &sb,
        &ws,
        "the run-plane door",
        "governed: mrd run apply_batch",
    );

    // ── the other arm: the fence has NOT become one that accepts everything.
    assert_an_out_of_band_edit_still_refuses(&sb, &ws, "the run-plane door");
}
