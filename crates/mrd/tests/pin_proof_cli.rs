//! `mrd pin --fingerprint` — the § A.3 proof law at the CLI face, driving the
//! real binary over its process boundary.
//!
//! The engine-side gate is proven in `crates/wire-serve/tests/s7_pin.rs`
//! (`a_cli_supplied_token_is_still_verified`); these gates prove the CLI door
//! can reach it: the flag carries the token onto the wire, a wrong token
//! refuses without writing, the live token lands, and the proofless pin the
//! local-operator trust door allows stays allowed.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::Value;

mod common;

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

struct Sandbox {
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        common::reap_daemon(&self.cache_home);
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

const TARGET: &str = "# Guide\n\n## Note\n\nsome text worth attesting\n";
const PAGE: &str = "# Plan\n\na claim drawn from the guide.\n";

impl Sandbox {
    /// Writes are IPC: this path auto-spawns the test binary as the daemon
    /// (`MERIDIAN_DAEMON_BIN` = this `mrd`), same as `read_put_cli.rs`.
    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        Command::new(mrd_bin())
            .args(args)
            .current_dir(cwd)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env("MERIDIAN_DAEMON_BIN", mrd_bin())
            .env_remove("MERIDIAN_WORKSPACE")
            .output()
            .expect("spawn mrd")
    }

    /// A git-backed marked workspace: the pinned target and the pinning page.
    /// Git is real because the R4 blob plane refuses a pin it cannot anchor —
    /// outside a git work tree, `mrd pin` refuses entirely.
    fn workspace(&self) -> PathBuf {
        let ws = self.tmp.path().join("project");
        std::fs::create_dir_all(&ws).expect("mkdir");
        git(&ws, &["init", "-q"]);
        git(&ws, &["config", "user.email", "pin@example.invalid"]);
        git(&ws, &["config", "user.name", "pin"]);
        std::fs::write(ws.join("guide.md"), TARGET).expect("guide");
        std::fs::write(ws.join("plan.md"), PAGE).expect("plan");
        let init = self.run(&ws, &["init"]);
        assert!(init.status.success(), "init: {}", stderr(&init));
        ws
    }
}

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

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("exit code")
}

fn read_file(ws: &Path, rel: &str) -> String {
    std::fs::read_to_string(ws.join(rel)).expect("read file")
}

/// The proof source: the `fingerprint` the sections read serves for the
/// selector — the token § A.3 says a later pin of that section carries back.
fn live_token(sb: &Sandbox, ws: &Path) -> String {
    let out = sb.run(
        ws,
        &["read", "guide.md", "--section", "Guide/Note", "--json"],
    );
    assert_eq!(code(&out), 0, "sections read: {}", stderr(&out));
    let v: Value = serde_json::from_str(&stdout(&out)).expect("json parses");
    v["read"]["sections"][0]["fingerprint"]
        .as_str()
        .expect("a served section carries its proof token")
        .to_owned()
}

/// Same shape, wrong value: flip the token's last character so the mutation
/// can never collide with the live token.
fn mutated(token: &str) -> String {
    let mut wrong = token.to_owned();
    let last = wrong.pop().expect("token is non-empty");
    wrong.push(if last == '0' { '1' } else { '0' });
    wrong
}

/// Gate — a well-formed wrong token refuses exit 1, names the proof mismatch,
/// and does not silently replace: the pin already on the page survives
/// byte-for-byte, and the target is untouched.
#[test]
fn a_wrong_supplied_token_refuses_and_does_not_silently_replace() {
    let sb = sandbox();
    let ws = sb.workspace();
    let token = live_token(&sb, &ws);

    // A real claim first, so the wrong token below has something to replace.
    let landed = sb.run(
        &ws,
        &[
            "pin",
            "plan.md",
            "guide.md#Guide/Note",
            "--fingerprint",
            &token,
        ],
    );
    assert_eq!(
        code(&landed),
        0,
        "the live token lands: {}",
        stderr(&landed)
    );
    let page_before = read_file(&ws, "plan.md");
    let target_before = read_file(&ws, "guide.md");
    assert!(
        page_before.contains("meridian-lock"),
        "the landed pin wrote the lock block:\n{page_before}"
    );

    let out = sb.run(
        &ws,
        &[
            "pin",
            "plan.md",
            "guide.md#Guide/Note",
            "--fingerprint",
            &mutated(&token),
        ],
    );
    assert_eq!(
        code(&out),
        1,
        "refused, not bad invocation: {}",
        stderr(&out)
    );
    assert!(
        stderr(&out).contains("the carried proof does not match"),
        "the refusal names the mismatch: {}",
        stderr(&out)
    );
    assert_eq!(
        read_file(&ws, "plan.md"),
        page_before,
        "the live claim was not replaced"
    );
    assert_eq!(
        read_file(&ws, "guide.md"),
        target_before,
        "the target was not touched"
    );
}

/// Gate — the token the sections read served is verified and the pin lands:
/// exit 0, and the lock block records that same token.
#[test]
fn the_live_token_is_verified_and_lands() {
    let sb = sandbox();
    let ws = sb.workspace();
    let token = live_token(&sb, &ws);

    let out = sb.run(
        &ws,
        &[
            "pin",
            "plan.md",
            "guide.md#Guide/Note",
            "--fingerprint",
            &token,
        ],
    );
    assert_eq!(code(&out), 0, "a correct token passes: {}", stderr(&out));
    assert!(
        stdout(&out).contains("pinned"),
        "the human line confirms: {}",
        stdout(&out)
    );
    let page = read_file(&ws, "plan.md");
    assert!(
        page.contains("meridian-lock") && page.contains(&token),
        "the lock block carries the verified token:\n{page}"
    );
}

/// Gate — the local-operator trust door is unchanged: a proofless CLI pin
/// still lands (trust excuses absence, never a wrong token).
#[test]
fn a_proofless_pin_stays_allowed() {
    let sb = sandbox();
    let ws = sb.workspace();

    let out = sb.run(&ws, &["pin", "plan.md", "guide.md#Guide/Note"]);
    assert_eq!(code(&out), 0, "proofless still allowed: {}", stderr(&out));
    assert!(
        read_file(&ws, "plan.md").contains("meridian-lock"),
        "the proofless pin landed its claim"
    );
}
