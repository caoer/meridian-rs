//! U1 end-to-end gates for `mrd read` / `mrd put` — the ratified read/put
//! naming at the CLI face, driving the REAL binary over its process boundary.
//!
//! `read` gates pin the composed-read face (U4a2 leaves + v3 projection) on
//! the DEGRADE path (`MERIDIAN_DAEMON_BIN` points nowhere, so the auto-spawn
//! is spawn-impossible and the answer is deterministic in-process). `put`
//! gates pin the A-S1 obligation observable end-to-end: bytes land through
//! the production splice choke-point, a dry run lands nothing, and every
//! refusal leg exits the triad correctly (1 refused / 2 bad invocation).

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::Value;

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

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

const DOC: &str = "# Alpha\n\none two three\n\n## Beta\n\nfour five\n";

impl Sandbox {
    fn command(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut cmd = Command::new(mrd_bin());
        cmd.args(args)
            .current_dir(cwd)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            // Spawn-impossible: the read path degrades in-process,
            // deterministically — no resident daemon ever starts.
            .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
            .env_remove("MERIDIAN_WORKSPACE");
        cmd
    }

    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        self.command(cwd, args).output().expect("spawn mrd")
    }

    /// Run with `stdin_bytes` piped — the `put` edits channel.
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

    /// A marked workspace holding the two-heading fixture doc.
    fn workspace(&self) -> PathBuf {
        let ws = self.tmp.path().join("project");
        std::fs::create_dir_all(&ws).expect("mkdir");
        std::fs::write(ws.join("doc.md"), DOC).expect("doc");
        let init = self.run(&ws, &["init"]);
        assert!(init.status.success(), "init: {}", stderr(&init));
        ws
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn code(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}

// ---------------------------------------------------------------------------
// read
// ---------------------------------------------------------------------------

/// Gate — default (toc) mode serves the rendered projection verbatim on
/// stdout: the display path the user typed, and both section titles.
#[test]
fn read_toc_serves_the_rendered_projection() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run(&ws, &["read", "doc.md"]);
    assert_eq!(code(&out), 0, "read: {}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("doc.md"), "display path rides: {text}");
    assert!(
        text.contains("Alpha") && text.contains("Beta"),
        "both sections listed: {text}"
    );
}

/// Gate — `--json` carries the v3-projected composed body: `file_rev` +
/// `fingerprint` (the D6 atomicity witness, `fingerprint` vocabulary — never
/// bare `root`), the toc rows with dewey ordinals, and the ephemeral source
/// label (the daemon was spawn-impossible).
#[test]
fn read_json_carries_the_v3_projected_body() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run(&ws, &["read", "doc.md", "--json"]);
    assert_eq!(code(&out), 0, "read --json: {}", stderr(&out));
    let v: Value = serde_json::from_str(&stdout(&out)).expect("json parses");
    assert_eq!(v["source"], "ephemeral", "degrade path answered: {v}");
    let body = &v["read"];
    assert!(body["file_rev"].is_string(), "file_rev rides: {body}");
    assert!(
        body["fingerprint"].is_string(),
        "v3 vocabulary (fingerprint, never root): {body}"
    );
    assert!(body.get("root").is_none(), "bare root never rides: {body}");
    assert!(body["words_total"].is_u64(), "words_total rides: {body}");
    let rows = body["toc"].as_array().expect("toc rows");
    assert_eq!(rows.len(), 2, "two heading rows: {body}");
    assert_eq!(rows[0]["n"], "1", "dewey ordinal on the first row");
    assert_eq!(rows[1]["n"], "1.1", "dewey ordinal on the nested row");
    assert_eq!(rows[1]["hpath"], "Alpha/Beta", "sanitized joined hpath");
}

/// Gate — `--section` implies sections mode and answers the selected
/// section's content.
#[test]
fn read_section_selects_and_serves_content() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run(
        &ws,
        &["read", "doc.md", "--section", "Alpha/Beta", "--json"],
    );
    assert_eq!(code(&out), 0, "read --section: {}", stderr(&out));
    let v: Value = serde_json::from_str(&stdout(&out)).expect("json parses");
    let sections = v["read"]["sections"].as_array().expect("sections");
    assert_eq!(sections.len(), 1);
    assert_eq!(sections[0]["sel"], "Alpha/Beta");
    assert!(
        sections[0]["content"]
            .as_str()
            .expect("content")
            .contains("four five"),
        "the section's content rides: {sections:?}"
    );
}

/// Gate — a `#FRAG` naming no section is the engine's refusal, VERBATIM, at
/// exit 1 (the finding leg — never rewritten client-side).
#[test]
fn read_frag_miss_is_the_engines_verbatim_refusal() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run(&ws, &["read", "doc.md#Nope"]);
    assert_eq!(code(&out), 1, "a refusal is the finding leg");
    assert!(
        stderr(&out).contains("no section at \"Nope\""),
        "the engine's verbatim message: {}",
        stderr(&out)
    );
}

/// Gate — `--mode toc` + `--section` is a LOUD client-side contradiction
/// (exit 2), never the wire's silent ignore.
#[test]
fn read_toc_mode_with_section_refuses_loudly() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run(
        &ws,
        &["read", "doc.md", "--mode", "toc", "--section", "Alpha"],
    );
    assert_eq!(code(&out), 2, "contradiction is a tool failure");
    assert!(
        stderr(&out).contains("--section conflicts with --mode toc"),
        "{}",
        stderr(&out)
    );
}

/// Gate — no PATH is a bad invocation (exit 2).
#[test]
fn read_without_path_is_exit_2() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run(&ws, &["read"]);
    assert_eq!(code(&out), 2);
    assert!(
        stderr(&out).contains("read needs a PATH"),
        "{}",
        stderr(&out)
    );
}

// ---------------------------------------------------------------------------
// put
// ---------------------------------------------------------------------------

/// The one-edit match batch against `Alpha/Beta`, in the wire §4.4 grammar.
fn beta_match(old: &str, new: &str) -> String {
    serde_json::to_string(&serde_json::json!([{
        "target": {"hpath": [{"h": "Alpha"}, {"h": "Beta"}]},
        "edit": {"match": {"old": old, "new": new}},
    }]))
    .expect("edits json")
}

/// Gate — a match edit lands on disk through the choke-point; the human
/// summary reports the committed fingerprint.
#[test]
fn put_match_edit_commits_bytes() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run_stdin(
        &ws,
        &["put", "doc.md"],
        &beta_match("four five", "four five six"),
    );
    assert_eq!(code(&out), 0, "put: {}", stderr(&out));
    let after = std::fs::read_to_string(ws.join("doc.md")).expect("read back");
    assert!(after.contains("four five six"), "the edit landed: {after}");
    let text = stdout(&out);
    assert!(
        text.contains("committed doc.md") && text.contains("fingerprint:"),
        "the human summary names the commit + fingerprint: {text}"
    );
}

/// Gate — `--dry` runs everything except disk: exit 0, bytes untouched.
#[test]
fn put_dry_lands_nothing() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run_stdin(
        &ws,
        &["put", "doc.md", "--dry"],
        &beta_match("four five", "changed"),
    );
    assert_eq!(code(&out), 0, "dry put: {}", stderr(&out));
    assert_eq!(
        std::fs::read_to_string(ws.join("doc.md")).expect("read back"),
        DOC,
        "a dry run writes nothing"
    );
    assert!(stdout(&out).contains("nothing written"), "{}", stdout(&out));
}

/// Gate — `--json` carries the v3-projected splice body (`fingerprint_before`
/// / `fingerprint_after`, never bare `root_*`).
#[test]
fn put_json_speaks_the_v3_vocabulary() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run_stdin(
        &ws,
        &["put", "doc.md", "--json"],
        &beta_match("four five", "four six"),
    );
    assert_eq!(code(&out), 0, "put --json: {}", stderr(&out));
    let v: Value = serde_json::from_str(&stdout(&out)).expect("json parses");
    let body = &v["put"];
    assert!(
        body["fingerprint_after"].is_string(),
        "fingerprint_after rides: {body}"
    );
    assert!(
        body.get("root_after").is_none(),
        "bare root_after never rides: {body}"
    );
}

/// Gate — a match with no occurrence is the engine's typed refusal at exit 1.
#[test]
fn put_no_match_is_the_finding_leg() {
    let sb = sandbox();
    let ws = sb.workspace();
    let before = std::fs::read_to_string(ws.join("doc.md")).expect("read");
    let out = sb.run_stdin(
        &ws,
        &["put", "doc.md"],
        &beta_match("absent text", "anything"),
    );
    assert_eq!(code(&out), 1, "no_match refusal: {}", stderr(&out));
    assert_eq!(
        std::fs::read_to_string(ws.join("doc.md")).expect("read back"),
        before,
        "a refusal leaves the bytes untouched"
    );
}

/// Gate — malformed stdin (not the §4.4 grammar) is a bad invocation, exit 2.
#[test]
fn put_malformed_stdin_is_exit_2() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run_stdin(&ws, &["put", "doc.md"], "not json at all");
    assert_eq!(code(&out), 2);
    assert!(
        stderr(&out).contains("malformed edits JSON"),
        "{}",
        stderr(&out)
    );
}

/// Gate — empty stdin teaches the grammar, exit 2.
#[test]
fn put_empty_stdin_is_exit_2() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run_stdin(&ws, &["put", "doc.md"], "");
    assert_eq!(code(&out), 2);
    assert!(
        stderr(&out).contains("edits JSON on stdin"),
        "{}",
        stderr(&out)
    );
}

/// Gate — a malformed `--now` refuses at the face (the §9 format law), exit 2.
#[test]
fn put_malformed_now_is_exit_2() {
    let sb = sandbox();
    let ws = sb.workspace();
    let out = sb.run_stdin(
        &ws,
        &["put", "doc.md", "--now", "yesterday"],
        &beta_match("four five", "x"),
    );
    assert_eq!(code(&out), 2);
    assert!(
        stderr(&out).contains("--now must be RFC 3339"),
        "{}",
        stderr(&out)
    );
}

/// Gate — the USAGE text teaches both new verbs (printed on any unknown verb).
#[test]
fn usage_teaches_read_and_put() {
    let sb = sandbox();
    let out = sb.run(sb.tmp.path(), &["bogus-verb"]);
    assert_eq!(code(&out), 2);
    let usage = stderr(&out);
    assert!(
        usage.contains("mrd read <PATH>"),
        "usage teaches read:\n{usage}"
    );
    assert!(
        usage.contains("mrd put <PATH>"),
        "usage teaches put:\n{usage}"
    );
}
