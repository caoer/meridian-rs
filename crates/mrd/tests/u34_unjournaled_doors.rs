//! **U34 — does a GOVERNED write through an unjournaled door make the installed
//! fence refuse the next commit?** A MEASUREMENT, not a fix (S3-R12(b)).
//!
//! Two byte-landing doors survive U31/U32 without a journal row:
//!
//! 1. **`mrd realise --truth file`** — deploys `conventions/INDEX.md`. U31 sealed
//!    it behind the candidate and `fs::replace_file`; U31's own comment says the
//!    journal row is still missing. *Sealing and journaling are different
//!    properties* (S3-R12(a)).
//! 2. **the run-plane `apply_batch`** — `executor::apply` lands page bytes through
//!    `fs::apply_batch` directly, never through the wire splice choke-point where
//!    U32's `render_row` call lives (`crates/wire-serve/src/write.rs:2105` is the
//!    only production journal writer in the tree).
//!
//! # What "the installed fence" is here
//! U15 has not landed, so this harness installs the RATIFIED fence verbatim: a
//! `.git/hooks/pre-commit` that execs `mrd check` and lets its exit status decide
//! the commit. Every leg then attempts a REAL `git commit` — never an assertion
//! about a function.
//!
//! # Both arms, or the measurement is not a measurement (S3-R8(c))
//! [`the_installed_fence_accepts_governed_work_and_refuses_an_out_of_band_edit`]
//! is the instrument's positive control: on the identical corpus the fence must
//! ACCEPT a governed commit and REFUSE an out-of-band one. Without it a door leg
//! reporting "refused" is indistinguishable from a fence that refuses everything.
//!
//! # The assert is the STATE CHANGE (R40)
//! An exit code says a command ran. Each door leg asserts what it DID: the bytes
//! that landed, the tree root that moved, and the journal row that does or does
//! not exist — then reads the fence's answer as the consequence.
//!
//! # THE MEASURED ANSWER
//! **Both doors: the fence REFUSES.** Each leg reaches its own green baseline
//! (`mrd check` exit 0, and a governed commit the fence lets through), drives its
//! door, and lands at `grey(cannot-assess)` on both detector lines, `mrd check`
//! exit 1, `git commit` exit 1 — with `git commit --no-verify` the only route
//! forward. The asserts below PIN that measurement so a later change to either
//! door reddens here rather than passing silently.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use fs::WorkspaceRoot;
use fs::domain::RESERVED_JOURNAL_PATH;
use policy::{CheckLimits, ConventionFiles, Enforcement, arm, generate_index, sweep};
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
        git(&ws, &["config", "user.email", "u34@example.invalid"]);
        git(&ws, &["config", "user.name", "u34"]);
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

    /// The operator's ESCAPE — the same commit with the fence bypassed. S3-R6
    /// ratified `--force` as the escape; for a git pre-commit hook it is spelled
    /// `--no-verify`.
    fn commit_bypassing_fence(&self, ws: &Path, message: &str) -> (i32, String) {
        self.commit(ws, message, &["--no-verify"])
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

/// **The consequence the ruling reasons about, measured** (S3-R12(b) / R35): once
/// the fence has refused a fully governed commit, the operator's only route
/// forward is the bypass. Asserting it is what makes "this trains the user to
/// `--force` habitually" a measurement rather than a worry.
fn assert_the_only_route_forward_is_the_bypass(sb: &Sandbox, ws: &Path, door: &str) {
    let head_before = git_head(ws);
    let (escape_code, escape_said) = sb.commit_bypassing_fence(ws, "the escape: --no-verify");
    let head_after = git_head(ws);
    assert_eq!(
        escape_code, 0,
        "{door}: the bypass lands the same commit the fence refused: {escape_said}"
    );
    assert_ne!(
        head_before, head_after,
        "{door}: R40 — HEAD moved, so the commit the fence refused is now history"
    );
    eprintln!("   escape: git commit --no-verify EXIT 0 · HEAD {head_before} -> {head_after}\n");
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

/// Every journal row on disk, read the way `check`'s detectors read it.
fn rows(root: &WorkspaceRoot) -> Vec<ParsedRow> {
    let page = std::fs::read_to_string(root.0.join(RESERVED_JOURNAL_PATH)).unwrap_or_default();
    parse_rows(&page)
}

#[cfg(unix)]
fn inode(path: &Path) -> u64 {
    use std::os::unix::fs::MetadataExt as _;
    std::fs::metadata(path).expect("stat").ino()
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
/// and a door leg reporting "refused" against an uncalibrated fence is not
/// evidence. Same corpus, both answers.
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

// ── DOOR 1 — `mrd realise --truth file` ──────────────────────────────────────

/// The convention slug the door's corpus arms.
const SLUG: &str = "reviewer-not-owner";

/// A loadable `CHECK.md` body, varied by `marker` so two versions hash
/// differently. `paths: tasks/**` keeps the armed law off every page this
/// corpus writes — the measurement is about the INDEX deploy, not about a gate.
fn check_md(marker: &str) -> String {
    format!(
        "---\npaths:\n  - tasks/**\n---\n\n# {SLUG} {marker}\n\n\
         ```starlark\ndef check_change(change):\n    pass\n```\n"
    )
}

/// A one-file convention accessor (`CHECK.md` → body) for `policy::sweep`.
struct MemConv(String);
impl ConventionFiles for MemConv {
    fn read(&self, rel: &str) -> std::io::Result<String> {
        if rel == "CHECK.md" {
            Ok(self.0.clone())
        } else {
            Err(std::io::Error::new(std::io::ErrorKind::NotFound, rel))
        }
    }
    fn exists(&self, rel: &str) -> bool {
        rel == "CHECK.md"
    }
}

/// The attested INDEX arming `SLUG` at Block, pinned to `check`'s live rev.
/// **There is no `mrd` verb that arms a convention** — this is the corpus
/// builder the card names, driven through the policy API the door reads.
fn armed_index(check: &str) -> String {
    let swept = sweep(&MemConv(check.to_owned()), SLUG, CheckLimits::default()).expect("sweeps");
    let rev = swept.rev().to_owned();
    let armed = arm(swept, &rev, Enforcement::Block).expect("arms at live rev");
    generate_index(&[armed])
}

/// **DOOR 1.** A governed `mrd realise --truth file` deploys the armed policy
/// INDEX through `fs::replace_file` — sealed by U31, unjournaled by charter.
/// Then the installed fence is asked for the next commit.
#[test]
fn door_1_realise_truth_file_then_the_installed_fence() {
    let sb = sandbox();
    // The divergence the door resolves: the INDEX pins v1, the live law is v2.
    let ws = sb.corpus(
        "door1-realise-truth-file",
        &[
            (".meridian.toml", ""),
            ("conventions/INDEX.md", &armed_index(&check_md("v1"))),
            (
                &format!("conventions/{SLUG}/CHECK.md"),
                &check_md("v2-edited"),
            ),
        ],
    );
    let root = root_of(&ws);
    green_baseline(&sb, &ws);

    // ── the pre-door state, so the state change is measured and not asserted.
    let index_path = ws.join("conventions/INDEX.md");
    let before_bytes = read(&ws, "conventions/INDEX.md");
    let before_ino = inode(&index_path);
    let before_root = live_root(&root);
    let before_rows = rows(&root);

    // ── THE DOOR, through its own verb.
    let door = sb.run(&ws, &["realise", "--truth", "file"]);
    assert_eq!(code(&door), 0, "the governed deploy runs: {}", said(&door));

    // ── R40: the STATE CHANGE, three disk facts.
    let after_bytes = read(&ws, "conventions/INDEX.md");
    let after_ino = inode(&index_path);
    let after_root = live_root(&root);
    let after_rows = rows(&root);
    assert_ne!(
        before_bytes, after_bytes,
        "(1) BYTES LANDED — the door rewrote the armed INDEX"
    );
    assert_ne!(
        before_ino, after_ino,
        "(1b) through the atomic candidate write (U31): a new inode"
    );
    assert_ne!(
        before_root, after_root,
        "(2) THE TREE ROOT MOVED — {before_root} -> {after_root}"
    );
    assert_eq!(
        before_rows.len(),
        after_rows.len(),
        "(3) NO JOURNAL ROW — the door is unjournaled: {after_rows:#?}"
    );
    assert_ne!(
        after_rows.last().expect("the baseline row").root_after,
        after_root,
        "so the baseline is now STALE: the last row's root_after is not the live tree"
    );

    // ── the measurement: what does `mrd check` say, and does the fence refuse?
    let check = sb.run(&ws, &["check"]);
    let (commit_code, commit_said) =
        sb.commit_through_fence(&ws, "governed: mrd realise --truth file");
    eprintln!(
        "── DOOR 1 · mrd realise --truth file ────\n\
         tree root {before_root} -> {after_root}\n\
         journal rows {} -> {} (no new row)\n\
         --- mrd check ---\n{}\
         mrd check EXIT {}\n\
         --- git commit (installed fence) ---\n{commit_said}\
         git commit EXIT {commit_code}\n",
        before_rows.len(),
        after_rows.len(),
        said(&check),
        code(&check),
    );

    // The measured answer, pinned so a later change to either door is visible.
    assert_eq!(
        code(&check),
        1,
        "MEASURED: check refuses after the governed deploy: {}",
        said(&check)
    );
    assert!(
        said(&check).contains("cannot-assess"),
        "and it refuses GREY, not red — the door leaves the trace an out-of-band \
         edit leaves (S3-R12(a)): {}",
        said(&check)
    );
    assert_ne!(
        commit_code, 0,
        "MEASURED: the installed fence REFUSES the commit whose only write was \
         governed: {commit_said}"
    );
    assert_the_only_route_forward_is_the_bypass(&sb, &ws, "door 1");
}

// ── DOOR 2 — the run-plane `apply_batch` ─────────────────────────────────────

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

/// **DOOR 2.** A governed `mrd run` lands page bytes through the run plane's
/// `fs::apply_batch` — never through the wire splice choke-point where U32's
/// journal row is written. Then the installed fence is asked for the next commit.
///
/// `realise.apply` faults on every attempt (the stage-4 card
/// `s4-realise-apply-invocation-id`), so the door is reached by `mrd run` — the
/// same `executor::apply` entry point, one caller over.
#[test]
fn door_2_run_plane_apply_batch_then_the_installed_fence() {
    let sb = sandbox();
    let ws = sb.corpus("door2-run-plane", &[("tasks.md", RUN_PAGE)]);
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
    assert_eq!(
        before_rows.len(),
        after_rows.len(),
        "(3) NO JOURNAL ROW — `grep -c journal crates/run/src/executor.rs` = 0: {after_rows:#?}"
    );
    assert_ne!(
        after_rows.last().expect("the baseline row").root_after,
        after_root,
        "so the baseline is now STALE: the last row's root_after is not the live tree"
    );

    // ── the measurement.
    let check = sb.run(&ws, &["check"]);
    let (commit_code, commit_said) = sb.commit_through_fence(&ws, "governed: mrd run apply_batch");
    eprintln!(
        "── DOOR 2 · run-plane apply_batch ───────\n\
         tree root {before_root} -> {after_root}\n\
         journal rows {} -> {} (no new row)\n\
         --- mrd check ---\n{}\
         mrd check EXIT {}\n\
         --- git commit (installed fence) ---\n{commit_said}\
         git commit EXIT {commit_code}\n",
        before_rows.len(),
        after_rows.len(),
        said(&check),
        code(&check),
    );

    assert_eq!(
        code(&check),
        1,
        "MEASURED: check refuses after the governed run: {}",
        said(&check)
    );
    assert!(
        said(&check).contains("cannot-assess"),
        "and it refuses GREY, not red: {}",
        said(&check)
    );
    assert_ne!(
        commit_code, 0,
        "MEASURED: the installed fence REFUSES the commit whose only write was \
         governed: {commit_said}"
    );
    assert_the_only_route_forward_is_the_bypass(&sb, &ws, "door 2");
}
