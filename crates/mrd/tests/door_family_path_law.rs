//! THE DOOR-FAMILY PATH LAW — the §1 admission binds every door the caller
//! names a path at (`docs/wire-contract.md` §12.1, line 878: "The rule binds a
//! DOOR FAMILY, and the family is every door the caller NAMES A PATH AT").
//!
//! The defect this gate exists to prevent, measured 2026-08-15 at `073d184f1`
//! and re-measured at `cac5bc8b`: handed the identical absolute path the read
//! door refuses by §1, `walk` WALKED it, `links` SERVED it, `repair` accepted
//! it and scanned 0, `realise` OPENED it, and `run` EXECUTED it — including a
//! page entirely OUTSIDE the workspace, writing the receipt into the
//! workspace's own `receipts/run.md`. Six doors also spoke six different
//! refusals for one bare-name mistake, and `realise` leaked a raw `os error 2`.
//!
//! Two family gates:
//! - one LAW: every door refuses the out-of-domain spelling, before any corpus
//!   is read, any file opened, any task run;
//! - one TEACHING: the same mistake gets the same fitted sentence at every
//!   door — the respell computed by the one shipped implementation
//!   (`wire_serve::write::relative_respelling`), earned and never guessed.

use std::path::{Path, PathBuf};
use std::process::Output;
mod common;

/// A run-plane task page: the marker only ever prints if a door EXECUTES it,
/// so its absence from stdout is the proof the admission fired first.
const JOB: &str =
    "---\ntask.hello: \"[[#^t-1]]\"\n---\n# Job\n\n```bash\necho HELLO-FROM-JOB\n```\n^t-1\n";
const EVIL: &str = "---\ntask.evil: \"[[#^t-1]]\"\n---\n# Evil\n\n```bash\necho RAN-OUTSIDE-THE-WORKSPACE\n```\n^t-1\n";
const GUIDE: &str = "# Guide\n\nA healthy page at the root.\n";

/// The one family sentence for the one bare-name mistake — identical bytes at
/// every door (the read door's landed teaching, `crate::path_law`).
const FAMILY_RESPELL: &str = "Did you mean `a/b/c/JOB.md`? — that file exists relative to your \
                              cwd, and refs are spelled from the workspace root, never from \
                              where you stand.";

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
    /// Every run is hermetic: spawn-impossible daemon (deterministic in-process
    /// answers), isolated cache and home, no ambient workspace override.
    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        common::mrd_command(&self.home, &self.cache_home)
            .args(args)
            .current_dir(cwd)
            .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
            .env_remove("MERIDIAN_WORKSPACE")
            .output()
            .expect("spawn mrd")
    }

    /// A git-anchored workspace whose run-capable page lives NESTED, so a
    /// caller standing beside the page spells a ref the root-anchored grammar
    /// cannot find — ZT's receipt shape, carried to the whole family.
    fn workspace(&self) -> PathBuf {
        let ws = self.tmp.path().join("ws");
        std::fs::create_dir_all(ws.join(".git")).expect("git anchor");
        std::fs::create_dir_all(ws.join("a/b/c")).expect("nested dirs");
        std::fs::write(ws.join("guide.md"), GUIDE).expect("guide");
        std::fs::write(ws.join("a/b/c/JOB.md"), JOB).expect("job page");
        std::fs::canonicalize(&ws).expect("canonical ws")
    }

    /// A page OUTSIDE every workspace, task-capable, beside a marker of its
    /// own so a receipt write anywhere is unmistakable.
    fn outside_page(&self) -> PathBuf {
        let outside = self.tmp.path().join("outside");
        std::fs::create_dir_all(&outside).expect("outside dir");
        std::fs::write(outside.join("EVIL.md"), EVIL).expect("evil page");
        std::fs::canonicalize(&outside)
            .expect("canonical outside")
            .join("EVIL.md")
    }
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}
fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}
fn code(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}

/// No run receipt anywhere under the workspace — the outside-page defect wrote
/// one into `receipts/run.md`, so its absence is asserted over the whole tree.
fn assert_no_receipt(ws: &Path) {
    let mut stack = vec![ws.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read_dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
            } else {
                assert_ne!(
                    path.file_name().and_then(|n| n.to_str()),
                    Some("run.md"),
                    "a refused run must write NO receipt, found {}",
                    path.display()
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Gate 1 — one LAW: the absolute spelling refuses at every door, with the
// family teaching and the fitted respell.
// ---------------------------------------------------------------------------

/// The analyst's six-door table, re-run as a gate. Every door refuses the
/// absolute spelling the read door refuses, speaks the §1 family voice with
/// its OWN name, and — the path lying inside the workspace — teaches the
/// respell the one shipped computation produces.
#[test]
fn an_absolute_path_refuses_at_every_door_in_the_family_voice() {
    let sb = sandbox();
    let ws = sb.workspace();
    let abs = ws.join("a/b/c/JOB.md");
    let abs = abs.to_str().expect("utf8 abs path");
    let cwd = ws.join("a/b");

    let doors: &[(&str, Vec<&str>)] = &[
        ("read", vec!["read", abs]),
        ("walk", vec!["walk", abs]),
        ("links", vec!["links", abs]),
        ("repair", vec!["repair", abs, "--dry"]),
        ("realise", vec!["realise", abs, "--dry"]),
        ("run", vec!["run", abs, "hello"]),
    ];
    for (door, args) in doors {
        let out = sb.run(&cwd, args);
        let err = stderr(&out);
        assert_ne!(
            code(&out),
            0,
            "{door}: an absolute spelling must refuse, got exit 0: {}",
            stdout(&out)
        );
        assert!(
            err.contains("is not a workspace-relative path"),
            "{door}: the refusal speaks the §1 family voice: {err:?}"
        );
        assert!(
            err.contains(&format!("the {door} door admits only workspace-relative")),
            "{door}: the family voice names the door the caller knocked on: {err:?}"
        );
        assert!(
            err.contains("respell it as `a/b/c/JOB.md`"),
            "{door}: a path inside the workspace earns the fitted respell — one \
             implementation, every door: {err:?}"
        );
    }

    // The run door's refusal is an ADMISSION refusal: nothing executed.
    let run_out = sb.run(&cwd, &["run", abs, "hello"]);
    assert!(
        !stdout(&run_out).contains("HELLO-FROM-JOB"),
        "the task must not run on a refused spelling: {}",
        stdout(&run_out)
    );
    assert_no_receipt(&ws);
}

// ---------------------------------------------------------------------------
// Gate 2 — the outside-workspace run: refuses, executes nothing, writes no
// receipt. The measured defect ran the task AND receipted it into the
// workspace's own `receipts/run.md`.
// ---------------------------------------------------------------------------

#[test]
fn a_page_outside_the_workspace_neither_runs_nor_mints_a_receipt() {
    let sb = sandbox();
    let ws = sb.workspace();
    let evil = sb.outside_page();
    let evil = evil.to_str().expect("utf8 outside path");

    let out = sb.run(&ws, &["run", evil, "evil"]);

    assert_eq!(
        code(&out),
        2,
        "an out-of-workspace page is a bad invocation (exit 2): {}",
        stderr(&out)
    );
    assert!(
        !stdout(&out).contains("RAN-OUTSIDE-THE-WORKSPACE"),
        "the outside task must not execute: {}",
        stdout(&out)
    );
    assert!(
        stderr(&out).contains("the run door admits only workspace-relative"),
        "the refusal teaches the §1 family voice: {:?}",
        stderr(&out)
    );
    assert!(
        stderr(&out).contains("no receipt was written"),
        "the run door's consequence clause states the receipt fact: {:?}",
        stderr(&out)
    );
    // An outside path earns NO respell — it lies inside no workspace.
    assert!(
        !stderr(&out).contains("respell it as"),
        "no fitted spelling exists for a page outside the workspace: {:?}",
        stderr(&out)
    );
    assert_no_receipt(&ws);
}

// ---------------------------------------------------------------------------
// Gate 3 — one TEACHING: the identical bare-name mistake gets the identical
// fitted sentence at all six doors.
// ---------------------------------------------------------------------------

/// From a cwd beside the page, every door's miss carries the SAME respell
/// sentence, byte for byte — six voices for one mistake was the measured
/// defect, one sentence is the gate.
#[test]
fn a_bare_name_miss_teaches_the_same_fitted_respell_at_every_door() {
    let sb = sandbox();
    let ws = sb.workspace();
    let cwd = ws.join("a/b/c");

    let doors: &[(&str, Vec<&str>)] = &[
        ("read", vec!["read", "JOB.md"]),
        ("walk", vec!["walk", "JOB.md"]),
        ("links", vec!["links", "JOB.md"]),
        ("repair", vec!["repair", "JOB.md", "--dry"]),
        ("realise", vec!["realise", "JOB.md", "--dry"]),
        ("run", vec!["run", "JOB.md", "hello"]),
    ];
    for (door, args) in doors {
        let out = sb.run(&cwd, args);
        let err = stderr(&out);
        assert_ne!(
            code(&out),
            0,
            "{door}: the bare-name miss refuses: {}",
            stdout(&out)
        );
        assert!(
            err.contains(FAMILY_RESPELL),
            "{door}: the miss carries the family sentence, byte-identical — one \
             mistake, one teaching: {err:?}"
        );
    }
}

/// The teaching is earned, never guessed (the read door's law, family-wide):
/// a ref that names nothing on disk gets each door's plain miss, and the
/// realise door's miss no longer leaks the raw io error.
#[test]
fn a_ref_that_exists_nowhere_is_suggested_nothing_and_leaks_no_errno() {
    let sb = sandbox();
    let ws = sb.workspace();

    for args in [
        vec!["walk", "gone.md"],
        vec!["realise", "gone.md", "--dry"],
        vec!["repair", "gone.md", "--dry"],
        vec!["run", "gone.md", "hello"],
    ] {
        let out = sb.run(&ws, &args);
        let err = stderr(&out);
        assert_ne!(code(&out), 0, "{args:?}: a genuine miss refuses");
        assert!(
            !err.contains("Did you mean"),
            "{args:?}: no candidate exists, so nothing is suggested: {err:?}"
        );
    }

    let realise = sb.run(&ws, &["realise", "gone.md", "--dry"]);
    let err = stderr(&realise);
    assert!(
        !err.contains("os error"),
        "the realise miss speaks the family voice, never the raw errno: {err:?}"
    );
    assert!(
        err.contains("no file at gone.md under the workspace root"),
        "the realise miss states the door's own fact: {err:?}"
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
