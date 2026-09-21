//! The CLI lane's own reading of §10.1's counter (wire-contract §18 row 12).
//!
//! The ledger's first reading blamed this lane for minting no Delta. It mints:
//! every write verb is a wire client on the daemon's door, so `mrd put` is
//! numbered like any other wire write and its own `--json` frame prints the
//! `seq` it was given. What was missing was the other half — `mrd fingerprint`
//! and `mrd links` answered `0` on both sides of that write, which is what a
//! poller reads.

use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::Value;

mod common;

const DOC: &str = "# Alpha\n\none two three\n\n## Beta\n\nfour five\n";
const BETA_EDIT: &str = r#"[{"target":{"hpath":[{"h":"Alpha"},{"h":"Beta"}]},"edit":{"match":{"old":"four five","new":"four five six"}}}]"#;
const BETA_AGAIN: &str = r#"[{"target":{"hpath":[{"h":"Alpha"},{"h":"Beta"}]},"edit":{"match":{"old":"four five six","new":"four five six seven"}}}]"#;

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

impl Drop for Sandbox {
    fn drop(&mut self) {
        common::reap_daemon(&self.home, &self.cache_home);
    }
}

impl Sandbox {
    fn workspace(&self) -> PathBuf {
        let ws = self.tmp.path().join("project");
        std::fs::create_dir_all(&ws).expect("mkdir");
        std::fs::write(ws.join("doc.md"), DOC).expect("doc");
        let out = self.run(&ws, &["init"], None);
        assert!(out.status.success(), "init: {}", stderr(&out));
        ws
    }

    /// One `mrd` invocation against the auto-spawning daemon.
    fn run(&self, cwd: &Path, args: &[&str], stdin: Option<&str>) -> Output {
        let mut cmd = common::mrd_command(&self.home, &self.cache_home);
        cmd.args(args)
            .current_dir(cwd)
            .env_remove("MERIDIAN_WORKSPACE")
            .env("MERIDIAN_DAEMON_BIN", mrd_bin());
        if let Some(bytes) = stdin {
            let mut child = cmd
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("spawn mrd");
            common::feed_stdin(&mut child, bytes.as_bytes());
            child.wait_with_output().expect("wait")
        } else {
            cmd.output().expect("spawn mrd")
        }
    }

    fn json(&self, cwd: &Path, args: &[&str], stdin: Option<&str>) -> Value {
        let out = self.run(cwd, args, stdin);
        assert!(
            out.status.success(),
            "{args:?}: stdout={} stderr={}",
            stdout(&out),
            stderr(&out)
        );
        serde_json::from_str(&stdout(&out))
            .unwrap_or_else(|e| panic!("{args:?} answers JSON ({e}): {}", stdout(&out)))
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}
fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// The gap as an operator met it: put, then ask the two read faces what
/// changed. Both answered `0` while the fingerprint moved under them.
///
/// *Mutation:* publish a literal `0` from the daemon's `links` / mint arms
/// again and every counter assertion after the first put reads 0.
#[test]
fn a_cli_put_is_visible_to_the_cli_read_faces() {
    let sb = sandbox();
    let ws = sb.workspace();

    let before = sb.json(&ws, &["fingerprint", "--json"], None);
    assert_eq!(
        before["mint"]["seq"], 0,
        "a fresh epoch has numbered nothing: {before}"
    );

    let put = sb.json(
        &ws,
        &["put", "doc.md", "--force", "--json"],
        Some(BETA_EDIT),
    );
    assert_eq!(put["put"]["seq"], 1, "the CLI lane mints its Delta: {put}");

    let after = sb.json(&ws, &["fingerprint", "--json"], None);
    assert_eq!(
        after["mint"]["seq"], 1,
        "and the mint publishes it: {after}"
    );
    assert_eq!(
        after["mint"]["fingerprint"], put["put"]["fingerprint_after"],
        "in that fingerprint's own tense: {after}"
    );
    assert_ne!(
        after["mint"]["fingerprint"], before["mint"]["fingerprint"],
        "the write moved the world: {after}"
    );
    assert!(
        after["mint"]["tree_instance"]
            .as_str()
            .is_some_and(|i| !i.is_empty()),
        "beside the epoch it is numbered in: {after}"
    );

    let links = sb.json(&ws, &["links", "--json"], None);
    assert_eq!(
        links["links"]["changes_seq"], 1,
        "and so does the view face: {links}"
    );
    assert_eq!(
        links["links"]["tree_instance"], after["mint"]["tree_instance"],
        "one epoch across both faces: {links}"
    );

    let second = sb.json(
        &ws,
        &["put", "doc.md", "--force", "--json"],
        Some(BETA_AGAIN),
    );
    assert_eq!(second["put"]["seq"], 2);
    let third = sb.json(&ws, &["fingerprint", "--json"], None);
    assert_eq!(
        third["mint"]["seq"], 2,
        "a poller reading the mint sees every CLI write: {third}"
    );
}
