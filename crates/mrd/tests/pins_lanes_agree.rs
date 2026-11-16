//! `mrd status` and `mrd check` take their pin rows from the resident daemon
//! (`docs/wire-contract.md` § A.13) when one answers and colour them in
//! process when none does, and the two lanes print ONE answer: stdout
//! byte-identical, exit code identical, on both faces. The daemon lane is
//! proven to have served (no degrade voice) and the in-process lane to have
//! degraded, so neither comparison passes by both runs taking one lane.
//!
//! Two gate cases ride the same sandbox: a pinned target edited in place right
//! after the daemon observed it refuses the next `check --commit-gate`, and a
//! staged interval whose index bytes diverge from the worktree is assessed at
//! the INDEX bytes although a daemon holding the worktree bytes is running.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

mod common;

struct Sandbox {
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = common::reap_daemon(&self.home, &self.cache_home);
    }
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
    fn run(&self, cwd: &Path, args: &[&str], no_daemon: bool) -> Output {
        let mut cmd = common::mrd_command(&self.home, &self.cache_home);
        cmd.args(args)
            .current_dir(cwd)
            .env_remove("MERIDIAN_WORKSPACE");
        if no_daemon {
            // A binary that cannot exist: the spawn fails at once and the
            // verb degrades to the in-process lane.
            cmd.env(
                "MERIDIAN_DAEMON_BIN",
                self.tmp.path().join("no-such-daemon"),
            );
        }
        cmd.output().expect("spawn mrd")
    }

    /// A committed git workspace declared by `mrd init`: `claim.md` pins
    /// `source.md` through the real `mrd pin` door, so the pin is green and
    /// its blob anchored.
    fn workspace(&self) -> PathBuf {
        let ws = self.tmp.path().join("project");
        std::fs::create_dir_all(&ws).expect("mkdir");
        git(&ws, &["init", "-q"]);
        std::fs::write(ws.join("source.md"), SOURCE_V1).expect("source");
        std::fs::write(ws.join("claim.md"), "# Claim\n\ndraws from source\n").expect("claim");
        let init = self.run(&ws, &["init"], true);
        assert!(init.status.success(), "init: {}", text(&init.stderr));
        let pin = self.run(
            &ws,
            &["pin", "claim.md", "source.md#Source/Guideline"],
            false,
        );
        assert!(pin.status.success(), "pin: {}", text(&pin.stderr));
        commit_all(&ws);
        ws
    }
}

const SOURCE_V1: &str = "# Source\n\n## Guideline\n\nthe pinned body v1\n";
const SOURCE_V2: &str = "# Source\n\n## Guideline\n\nthe pinned body v2\n";
const DEGRADE_VOICE: &str = "mrd: source: ephemeral";

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid"])
        .args(args)
        .output()
        .expect("spawn git");
    assert!(out.status.success(), "git {args:?}: {}", text(&out.stderr));
}

fn commit_all(dir: &Path) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "--no-verify", "-m", "fixture"]);
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn the_daemon_lane_and_the_in_process_lane_print_one_answer() {
    let sb = sandbox();
    let ws = sb.workspace();
    // A second holder claiming a page in a root no machine binds: the grey,
    // exit-1 leg of `check`, on both lanes.
    std::fs::write(
        ws.join("ghost.md"),
        text(&std::fs::read(ws.join("claim.md")).expect("claim"))
            .replace("[[source]]", "[[nowhere:source]]"),
    )
    .expect("ghost");
    commit_all(&ws);

    let invocations: [&[&str]; 6] = [
        &["status"],
        &["status", "--json"],
        &["check"],
        &["check", "--json"],
        &["check", "--commit-gate"],
        &["check", "--commit-gate", "--json"],
    ];
    let mut warm_runs = Vec::new();
    for args in invocations {
        let warm = sb.run(&ws, args, false);
        let warm_err = text(&warm.stderr);
        assert!(
            !warm_err.contains(DEGRADE_VOICE),
            "{args:?}: the daemon lane must serve this run, stderr: {warm_err}"
        );
        warm_runs.push(warm);
    }
    // The daemon the warm runs spawned still answers on this socket; reap it
    // so the cold runs find no daemon at all.
    let _ = common::reap_daemon(&sb.home, &sb.cache_home);
    for (args, warm) in invocations.iter().zip(warm_runs) {
        let cold = sb.run(&ws, args, true);
        let cold_err = text(&cold.stderr);
        assert!(
            cold_err.contains(DEGRADE_VOICE),
            "{args:?}: the in-process lane must degrade on this run, stderr: {cold_err}"
        );
        assert_eq!(
            text(&warm.stdout),
            text(&cold.stdout),
            "{args:?}: one answer from both lanes"
        );
        assert_eq!(warm.status.code(), cold.status.code(), "{args:?}");
    }
    let check = sb.run(&ws, &["check"], true);
    assert!(
        text(&check.stdout).contains("unmounted") || text(&check.stderr).contains("unmounted"),
        "the fixture's grey leg is the unmounted root: {}{}",
        text(&check.stdout),
        text(&check.stderr)
    );
}

#[test]
fn the_gate_sees_an_edit_made_right_after_the_daemon_looked() {
    let sb = sandbox();
    let ws = sb.workspace();

    let green = sb.run(&ws, &["check", "--commit-gate"], false);
    assert!(
        green.status.success(),
        "a held pin passes the gate: {}{}",
        text(&green.stdout),
        text(&green.stderr)
    );
    assert!(!text(&green.stderr).contains(DEGRADE_VOICE));

    // Same size, in place, at once: the shape a stat compare cannot see.
    // Staged too, so the index and the worktree coincide and the gate takes
    // the daemon's rows.
    std::fs::write(ws.join("source.md"), SOURCE_V2).expect("edit");
    git(&ws, &["add", "source.md"]);
    let red = sb.run(&ws, &["check", "--commit-gate"], false);
    let said = format!("{}{}", text(&red.stdout), text(&red.stderr));
    assert!(
        !said.contains(DEGRADE_VOICE),
        "the daemon lane served: {said}"
    );
    assert_eq!(red.status.code(), Some(1), "the gate refuses: {said}");
    assert!(said.contains("content-drifted"), "{said}");
}

#[test]
fn a_diverging_staged_interval_is_assessed_at_the_index_bytes() {
    let sb = sandbox();
    let ws = sb.workspace();
    // Warm the daemon on the worktree bytes.
    let green = sb.run(&ws, &["check", "--commit-gate"], false);
    assert!(green.status.success(), "{}", text(&green.stderr));

    // The index carries the drifted source; the worktree is put back. A
    // daemon answers green for these worktree bytes, and the commit would
    // record the drift.
    std::fs::write(ws.join("source.md"), SOURCE_V2).expect("edit");
    git(&ws, &["add", "source.md"]);
    std::fs::write(ws.join("source.md"), SOURCE_V1).expect("restore");

    let gated = sb.run(&ws, &["check", "--commit-gate"], false);
    let said = format!("{}{}", text(&gated.stdout), text(&gated.stderr));
    assert_eq!(
        gated.status.code(),
        Some(1),
        "the staged bytes drifted: {said}"
    );
    assert!(
        said.contains("staged"),
        "the refusal names its interval: {said}"
    );
    assert!(said.contains("content-drifted"), "{said}");
}
