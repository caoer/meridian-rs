//! Exit-code taxonomy gates (dogfood P3-b) — the documented 1-vs-2 law, routed.
//!
//! The triad every engine-backed verb documents: 0 clean / 1 the ENGINE
//! refused (its message, verbatim) / 2 the CLI's OWN refusal, issued before
//! any engine contact. The dogfood run caught engine-originated `bad_request`
//! refusals (a §4.4 batch overlap, a multi-line upsert value) leaving through
//! exit 2 — the CLI-misuse leg — so a script could not tell "fix your
//! invocation" from "read the engine's refusal". These gates hold the split.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

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
            // Spawn-impossible: the write path runs in-process,
            // deterministically — no resident daemon ever starts.
            .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
            .env_remove("MERIDIAN_WORKSPACE");
        cmd
    }

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
        let init = self
            .command(&ws, &["init"])
            .output()
            .expect("spawn mrd init");
        assert!(init.status.success(), "init: {}", stderr(&init));
        ws
    }
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn code(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}

/// Gate (P3-b) — a §4.4 batch overlap is the ENGINE refusing a well-formed
/// invocation: the CLI parsed the flags, the decoder accepted the batch, and
/// the engine's region law said no. That is the findings leg (exit 1), never
/// the CLI-misuse leg — a script seeing 2 re-reads its own command line, which
/// is exactly the wrong fix here.
#[test]
fn a_batch_overlap_is_the_engines_refusal_at_exit_1() {
    let sb = sandbox();
    let ws = sb.workspace();
    let before = std::fs::read_to_string(ws.join("doc.md")).expect("read");
    // Two match edits on one target whose replaced regions share "two":
    // each `old` is unique in the span, so only the §4.4 region law refuses.
    let edits = serde_json::to_string(&serde_json::json!([
        {
            "target": {"hpath": [{"h": "Alpha"}]},
            "edit": {"match": {"old": "one two", "new": "X"}},
        },
        {
            "target": {"hpath": [{"h": "Alpha"}]},
            "edit": {"match": {"old": "two three", "new": "Y"}},
        },
    ]))
    .expect("edits json");
    let out = sb.run_stdin(&ws, &["put", "doc.md"], &edits);
    let err = stderr(&out);
    assert!(err.contains("edits["), "the overlap refusal names the edits:\n{err}");
    assert_eq!(
        code(&out),
        1,
        "an engine refusal is exit 1, not CLI misuse:\n{err}"
    );
    assert_eq!(
        std::fs::read_to_string(ws.join("doc.md")).expect("read back"),
        before,
        "a refusal writes nothing"
    );
}

/// Gate (P3-b) — a multi-line `upsert` value is refused by the ENGINE's
/// value-plane law (§ A.6.3a), after the CLI accepted a well-formed batch:
/// exit 1, same leg as every other engine refusal.
#[test]
fn a_multi_line_upsert_value_is_the_engines_refusal_at_exit_1() {
    let sb = sandbox();
    let ws = sb.workspace();
    let edits = serde_json::to_string(&serde_json::json!([{
        "target": {"fm_key": "title"},
        "edit": {"put": {"at": "upsert", "text": "line one\nline two"}},
    }]))
    .expect("edits json");
    let out = sb.run_stdin(&ws, &["put", "doc.md"], &edits);
    assert_eq!(
        code(&out),
        1,
        "the engine's value-plane refusal is exit 1:\n{}",
        stderr(&out)
    );
}

/// The other half of the law, held in the same file so the split stays
/// legible: the CLI's OWN refusals — issued before any engine contact — keep
/// exit 2. These are the invocations where "fix your command line" is the fix.
#[test]
fn the_clis_own_refusals_keep_exit_2() {
    let sb = sandbox();
    let ws = sb.workspace();
    let good = r#"[{"target":{"hpath":[{"h":"Alpha"}]},"edit":{"match":{"old":"one","new":"1"}}}]"#;
    for (args, stdin) in [
        // An unknown flag.
        (vec!["put", "doc.md", "--nope"], good),
        // Malformed stdin: refused by the decoder, zero engine contact.
        (vec!["put", "doc.md"], "not json at all"),
        // A contradictory flag pair.
        (vec!["put", "doc.md", "--dry", "--validate"], good),
    ] {
        let out = sb.run_stdin(&ws, &args, stdin);
        assert_eq!(
            code(&out),
            2,
            "{args:?} is the CLI's own refusal: {}",
            stderr(&out)
        );
    }
}
