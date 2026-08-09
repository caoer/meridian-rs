//! The `--json` refusal envelope is the law of the FACE, not of one verb
//! (`docs/status.md` § teaching rows: *"A `--json` face answers `{workspace, error}` on EVERY
//! leg that can refuse"*). `440245b3` moved the envelope into `engine::json_refusal` and then
//! wired only the two verbs that had motivated it, leaving `pin` (four refusal legs) and
//! `retire mark` (the splice leg) serving ZERO stdout bytes at exit 1 — indistinguishable, to a
//! parsing agent, from success with no output.
//!
//! This suite gates the FAMILY, not those two doors: every door below is driven at a real
//! exit-1 leg, and each one must serve the envelope. A door added later that mints a frameless
//! engine refusal cannot compile (`engine::refusal_fail` is private), and a door that regresses
//! its envelope fails here.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use std::io::Write;

/// A fingerprint no workspace can be standing at, so a guarded write refuses on the world guard.
const BOGUS_ROOT: &str = "b3:0000000000000000000000000000000000000000000000000000000000000000";

struct Sandbox {
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
}

/// One workspace, born per test. `MERIDIAN_WORKSPACE` anchors it explicitly so the resolution
/// ladder never walks out to an ancestor, and `XDG_CACHE_HOME` gives this test its own drawer —
/// the registry socket is keyed by cache root, so a shared one lets a foreign engine answer.
fn sandbox() -> (Sandbox, PathBuf) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let sb = Sandbox {
        cache_home: tmp.path().join("xdg-cache"),
        home: tmp.path().join("home"),
        tmp,
    };
    std::fs::create_dir_all(&sb.home).expect("home");
    let ws = sb.tmp.path().join("project");
    std::fs::create_dir_all(&ws).expect("mkdir");
    std::fs::write(ws.join("claim.md"), "# Claim\n\nA claim.\n").expect("claim");
    std::fs::write(ws.join("source.md"), "# Source\n\n## Note\n\ntext here\n").expect("source");
    (sb, ws)
}

fn run(sb: &Sandbox, cwd: &Path, args: &[&str]) -> Output {
    run_with_stdin(sb, cwd, args, "")
}

fn run_with_stdin(sb: &Sandbox, cwd: &Path, args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_mrd"))
        .args(args)
        .current_dir(cwd)
        .env("XDG_CACHE_HOME", &sb.cache_home)
        .env("HOME", &sb.home)
        // Spawn-impossible: no resident daemon ever starts, so the engine under test is the
        // binary this suite built and nothing else can answer.
        .env("MERIDIAN_DAEMON_BIN", "/nonexistent/mrd-daemon")
        .env("MERIDIAN_WORKSPACE", cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn mrd");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("wait mrd")
}

/// The envelope assertion, one door at a time: exit 1, `{workspace, error{code}}` on stdout, the
/// human sentence still on stderr. The stdout byte count is asserted non-zero explicitly because
/// ZERO BYTES is the exact defect — a caller cannot tell it from success with no output.
fn assert_envelope(label: &str, out: &Output) -> serde_json::Value {
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(
        out.status.code(),
        Some(1),
        "{label}: an engine refusal is the findings leg; stderr was {stderr}"
    );
    assert!(
        !stdout.is_empty(),
        "{label}: served 0 stdout bytes — the absent frame IS the defect. stderr: {stderr}"
    );
    let frame: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("{label}: stdout is not JSON: {e}\n{stdout}"));
    assert!(
        frame.get("workspace").and_then(|v| v.as_str()).is_some(),
        "{label}: the envelope names its workspace: {frame}"
    );
    assert!(
        frame
            .pointer("/error/code")
            .and_then(|v| v.as_str())
            .is_some(),
        "{label}: the envelope carries the engine's §8 error code: {frame}"
    );
    assert!(
        stderr.starts_with("mrd: "),
        "{label}: the human line on stderr is unchanged by the envelope: {stderr}"
    );
    frame
}

/// The other direction of every control pair: the HUMAN face prints nothing on stdout at the same
/// leg. Without this, an envelope that leaked into the human face would pass the suite above.
fn assert_human_face_is_silent(label: &str, out: &Output) {
    assert_eq!(
        out.status.code(),
        Some(1),
        "{label}: still the findings leg"
    );
    assert!(
        out.stdout.is_empty(),
        "{label}: the human face says nothing on stdout; the envelope is the `--json` face's"
    );
}

/// `read` — the door `440245b3` did wire. The positive control: it proves the detector fires, so
/// a zero at another door is a measured absence and not a broken probe.
#[test]
fn control_read_serves_the_envelope() {
    let (sb, ws) = sandbox();
    let out = run(&sb, &ws, &["read", "nosuch.md", "--json"]);
    let frame = assert_envelope("read file_not_found", &out);
    assert_eq!(frame.pointer("/error/code").unwrap(), "file_not_found");
    assert_human_face_is_silent(
        "read file_not_found (human)",
        &run(&sb, &ws, &["read", "nosuch.md"]),
    );
}

/// `put` — the second wired door, and the sharpest comparator for `pin`: the SAME error code, at
/// the SAME workspace, in the SAME in-process model.
#[test]
fn control_put_serves_the_envelope() {
    let (sb, ws) = sandbox();
    let batch = r#"[{"target":{"hpath":[{"h":"Claim"}]},"edit":{"match":{"old":"A claim","new":"A case"}}}]"#;
    let out = run_with_stdin(&sb, &ws, &["put", "nosuch.md", "--json"], batch);
    let frame = assert_envelope("put file_not_found", &out);
    assert_eq!(frame.pointer("/error/code").unwrap(), "file_not_found");
}

/// `pin` — ALL FOUR refusal legs, not the one that motivated the card. Each was 0 stdout bytes
/// before the frameless caller at `pin_cmd.rs` was routed through the face's helper.
#[test]
fn pin_serves_the_envelope_on_every_refusal_leg() {
    let (sb, ws) = sandbox();

    let page_absent = run(
        &sb,
        &ws,
        &["pin", "nosuch.md", "source.md#Source/Note", "--json"],
    );
    assert_eq!(
        assert_envelope("pin page absent", &page_absent)
            .pointer("/error/code")
            .unwrap(),
        "file_not_found",
        "the same code `read` and `put` frame, at pin's door"
    );

    let target_absent = run(&sb, &ws, &["pin", "claim.md", "nosuch.md#Alpha", "--json"]);
    assert_envelope("pin target absent", &target_absent);

    let selector_absent = run(
        &sb,
        &ws,
        &["pin", "claim.md", "source.md#Source/NoSuch", "--json"],
    );
    assert_envelope("pin selector absent", &selector_absent);

    // The fourth leg: this tree is not a git work tree, so the R4 blob lookup fails `io_error`.
    // It is the only pin leg whose whole content lives in the error body's `cause`.
    let io = run(
        &sb,
        &ws,
        &[
            "pin",
            "claim.md",
            "source.md#Source/Note",
            "--dry",
            "--json",
        ],
    );
    let frame = assert_envelope("pin io_error", &io);
    assert_eq!(frame.pointer("/error/code").unwrap(), "io_error");
    assert!(
        frame
            .pointer("/error/cause")
            .and_then(|v| v.as_str())
            .is_some(),
        "the envelope carries the cause a parsing agent would otherwise read off prose"
    );

    assert_human_face_is_silent(
        "pin page absent (human)",
        &run(&sb, &ws, &["pin", "nosuch.md", "source.md#Source/Note"]),
    );
}

/// `pin` renders the `io_error` cause ONCE. Two renderers carried it after `440245b3` added the
/// cause to the shared helper while `pin_cmd`'s own wrapper still appended it — the doubling was
/// measured against release `93184797`, which renders it once.
#[test]
fn pin_renders_its_cause_exactly_once() {
    let (sb, ws) = sandbox();
    let out = run(
        &sb,
        &ws,
        &[
            "pin",
            "claim.md",
            "source.md#Source/Note",
            "--dry",
            "--json",
        ],
    );
    let frame = assert_envelope("pin io_error", &out);
    let cause = frame
        .pointer("/error/cause")
        .and_then(|v| v.as_str())
        .expect("io_error carries a cause")
        .to_owned();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        stderr.matches(cause.as_str()).count(),
        1,
        "the cause is rendered once on the human line, never once inline and once parenthesised: {stderr}"
    );
}

/// `retire mark` — the splice leg at `retire_cmd.rs`, the card's highest-value unknown. It is
/// REACHABLE (not dead code): a clean declaration plus a world guard that cannot hold refuses
/// `fingerprint_mismatch` at exit 1.
#[test]
fn retire_mark_serves_the_envelope_on_its_splice_leg() {
    let (sb, ws) = sandbox();
    std::fs::write(
        ws.join("notes.md"),
        "# Notes\n\nThe oldname appears here in prose.\n\n## Holding\n\nReplaced content lives here.\n",
    )
    .expect("notes");
    std::fs::write(
        ws.join("retire.md"),
        "# Retirement\n\n```meridian-retire\nversion: 1\nid: demo-one\nterm: oldname\nreplacer: newname\ncontrol: prose\nholding:\n  path: notes.md\n  hpath:\n    - h: Notes\n    - h: Holding\nroute: notes.md\n```\n",
    )
    .expect("retire block");

    // The fixture must reach the splice, so prove the declaration parses clean first: a report
    // carrying refusals would never enter the write branch, and this test would pass on the
    // wrong leg.
    let report = run(&sb, &ws, &["retire", "report", "--json"]);
    let body: serde_json::Value =
        serde_json::from_slice(&report.stdout).expect("report serves its own envelope");
    assert_eq!(
        body.get("refusals")
            .and_then(|v| v.as_array())
            .map(Vec::len),
        Some(0),
        "the fixture declares one clean retirement, so `mark` reaches the splice: {body}"
    );

    let out = run(
        &sb,
        &ws,
        &["retire", "mark", "--expect-root", BOGUS_ROOT, "--json"],
    );
    assert_envelope("retire mark world guard", &out);
    assert_human_face_is_silent(
        "retire mark world guard (human)",
        &run(&sb, &ws, &["retire", "mark", "--expect-root", BOGUS_ROOT]),
    );
}
