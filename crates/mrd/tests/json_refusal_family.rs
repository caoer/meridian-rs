//! The `--json` refusal envelope is the law of the FACE, not of one verb
//! (`docs/status.md` § teaching rows: *"A `--json` face answers `{workspace, error}` on EVERY
//! leg that can refuse"*). `440245b3` moved the envelope into `engine::json_refusal` and then
//! wired only the two verbs that had motivated it, leaving `pin` (four refusal legs) and
//! `retire mark` (the splice leg) serving ZERO stdout bytes at exit 1 — indistinguishable, to a
//! parsing agent, from success with no output.
//!
//! This suite gates the FAMILY, not those two doors: every door below is driven at a real
//! refusal leg, and each one must serve the envelope.
//!
//! ⛔ THIS HEADER USED TO CLAIM: *"A door added later that mints a frameless engine refusal
//! cannot compile (`engine::refusal_fail` is private)."* **THAT WAS FALSE WHEN IT WAS WRITTEN,
//! and `mrd repair` was the standing counter-example** — a door reaches `wire_serve` directly,
//! converts the `ErrorBody` to a `Fail` by hand, and never touches `engine` at all. Privacy
//! gates the doors that ASK the helper; it cannot reach a door that never asks. The suite's real
//! guarantee is the enumeration below, and the enumeration is only as good as its criterion:
//! *every `--json`-accepting verb with an engine-refusal seam whose leg does not route the frame*
//! — measured over the `wire_serve::` call sites in `crates/mrd/src`, not over a list of doors.
//!
//! ⚠️ And the frame does not imply the findings leg. `read`/`put`/`pin`/`retire mark` spell an
//! engine refusal as exit 1; `repair` reserves exit 1 for a TRUE LOSS, `retire report`'s
//! corpus leg is a tool failure at exit 2, and `links` reserves exit 1 for a REFUSED EDGE.
//! Each case below states the exit it expects.
//!
//! 📌 THE ENUMERATION, per seam and at a NAMED TREE STATE, because "how many members" is not
//! answerable without both (`413602e8`, 2026-08-09: the discriminating question is not how many
//! but at which tree state, counted by VERB or by SEAM). At `b84ccf91`, counted by SEAM, the
//! criterion held THREE frameless engine-refusal seams: `repair_cmd.rs:534` (`lock_write`),
//! `retire_cmd.rs:932` (`domain_snapshot`) — both closed by the fix this suite was written
//! beside — and `engine.rs` `in_process_links` (`links_rooted`), closed here. `retire` is the
//! reason a per-VERB ledger cannot answer: it has TWO engine-refusal seams and routes one.
//!
//! ⚠️ `links` HAS EXACTLY ONE TERMINAL REFUSAL SEAM, and that is a property of its dispatch
//! rather than an omission in this suite: `engine::try_daemon_links` answers `None` on ANY
//! daemon-path failure and `answer_links` degrades, so the warm path never refuses. A
//! two-face family table over `links` would look unswept and be complete.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use std::io::Write;

mod common;

/// A fingerprint no workspace can be standing at, so a guarded write refuses on the world guard.
const BOGUS_ROOT: &str = "b3:0000000000000000000000000000000000000000000000000000000000000000";

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
        .env("MERIDIAN_DAEMON_BIN", env!("CARGO_BIN_EXE_mrd"))
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

/// The envelope law at the findings leg — the common case, and every door wired by `440245b3`.
/// `{workspace, error{code}}` on stdout, the human sentence still on stderr. The stdout byte
/// count is asserted non-zero explicitly because ZERO BYTES is the exact defect — a caller
/// cannot tell it from success with no output.
fn assert_envelope(label: &str, out: &Output) -> serde_json::Value {
    assert_envelope_at(label, out, 1)
}

/// The envelope law at a STATED exit code.
///
/// This helper used to hardcode exit 1 with the comment *"an engine refusal is the findings
/// leg"*. **That is true of the doors it was written for and false as a general law**, and the
/// assumption cost something: it makes routing every frameless door through
/// `engine::json_refusal` look correct, and `mrd repair` reserves EXIT 1 FOR A TRUE LOSS. A door
/// whose triad spells an engine refusal as a TOOL failure still owes the frame; the frame and
/// the exit code are two judgements. Callers state which one they mean.
fn assert_envelope_at(label: &str, out: &Output, exit: i32) -> serde_json::Value {
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(
        out.status.code(),
        Some(exit),
        "{label}: expected exit {exit} — the envelope never moves a verb's exit triad; \
         stderr was {stderr}"
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
    assert_human_face_is_silent_at(label, out, 1);
}

fn assert_human_face_is_silent_at(label: &str, out: &Output, exit: i32) {
    assert_eq!(
        out.status.code(),
        Some(exit),
        "{label}: expected exit {exit}"
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

/// `retire report` — THE SECOND MEMBER, and the one a per-verb check scores CLEAN.
///
/// `retire` already calls `engine::json_refusal` at its `mark` splice leg (the test above), so
/// any "does this verb route the frame" predicate reads it as done. **The corpus leg at
/// `retire_cmd.rs` § `run` did not**: `wire_serve::domain_snapshot` returns a §8 `ErrorBody` and
/// the leg converted it to prose with no frame. A file-level boolean is a list in disguise.
///
/// It carried a SECOND defect the envelope question hides. The leg rendered
/// `e.message.unwrap_or_default()`, and `domain_snapshot`'s `io_error` arm sets `cause` and
/// **never** `message` — so the one arm this leg actually reports printed
/// `cannot read the corpus: ` with NOTHING after the colon, discarding a cause the engine had
/// measured. Both halves are asserted here; fixing either alone leaves this red.
///
/// Exit 2, not 1: a corpus that cannot be read is a TOOL failure, the same as the workspace legs
/// beside it. `retire`'s exit 1 means a refusal or an open retirement.
#[test]
fn retire_report_serves_the_envelope_on_its_corpus_leg() {
    let (sb, ws) = sandbox();

    // CONTROL, and it must pass in BOTH the fixed and the unfixed tree: the same verb, the same
    // face, a readable corpus. Without it a zero below could mean `retire report` is broken
    // outright rather than frameless at one leg.
    let ok = run(&sb, &ws, &["retire", "report", "--json"]);
    assert_eq!(
        ok.status.code(),
        Some(0),
        "control: a readable corpus reports clean; stderr {}",
        String::from_utf8_lossy(&ok.stderr)
    );
    assert!(
        !ok.stdout.is_empty(),
        "control: the success face serves JSON, so an empty stdout below is the ABSENT FRAME \
         and not a mute verb"
    );

    // Make one served file unreadable so `domain_snapshot` fails on it.
    let victim = ws.join("claim.md");
    let mut perms = std::fs::metadata(&victim).expect("stat").permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o000);
    std::fs::set_permissions(&victim, perms).expect("chmod");

    let out = run(&sb, &ws, &["retire", "report", "--json"]);

    // Restore before asserting so a failure cannot leave an unreadable file behind.
    let mut back = std::fs::metadata(&victim).expect("stat").permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut back, 0o644);
    std::fs::set_permissions(&victim, back).expect("chmod back");

    // PRECONDITION, asserted rather than assumed: running as root defeats the chmod, and a
    // fixture that cannot build its condition must FAIL LOUD, never skip. A silent skip here
    // would report a clean suite over a leg nobody exercised.
    assert_ne!(
        out.status.code(),
        Some(0),
        "precondition: the unreadable file must break the corpus read — running as root, or on \
         a filesystem ignoring mode bits, this fixture proves nothing and says so"
    );

    let frame = assert_envelope_at("retire report corpus leg", &out, 2);
    assert_eq!(
        frame.pointer("/error/code").unwrap(),
        "io_error",
        "the corpus leg's §8 code rides the frame: {frame}"
    );

    // The second half: the human line must NAME the cause, not trail off after its colon.
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let line = stderr
        .lines()
        .find(|l| l.contains("cannot read the corpus"))
        .unwrap_or_else(|| panic!("the corpus refusal names itself: {stderr}"));
    assert!(
        !line.trim_end().ends_with(':'),
        "the refusal trails off after its colon — `message` alone is empty on the `io_error` \
         arm and the measured cause was dropped: {line:?}"
    );
    assert!(
        line.contains("io_error"),
        "the shared renderer inlines the engine's cause behind its code: {line:?}"
    );

    assert_human_face_is_silent_at(
        "retire report corpus leg (human)",
        &{
            let mut p = std::fs::metadata(&victim).expect("stat").permissions();
            std::os::unix::fs::PermissionsExt::set_mode(&mut p, 0o000);
            std::fs::set_permissions(&victim, p).expect("chmod");
            let human = run(&sb, &ws, &["retire", "report"]);
            let mut b = std::fs::metadata(&victim).expect("stat").permissions();
            std::os::unix::fs::PermissionsExt::set_mode(&mut b, 0o644);
            std::fs::set_permissions(&victim, b).expect("chmod back");
            human
        },
        2,
    );
}

/// `links` — THE THIRD SEAM, and the one the fix beside this suite never reached.
///
/// `engine.rs`'s `in_process_links` hands `wire_serve::read::links_rooted`'s `ErrorBody` straight
/// to `Fail::tool(render_wire_error(&e))`: the human sentence is fine and stdout is EMPTY, which
/// is the whole defect — a parsing agent cannot tell it from success with no output.
///
/// ⚠️ EXIT 2 IS ASSERTED, NOT CORRECTED, AND THE DISTINCTION IS DELIBERATE. `links` spells its
/// OWN finding — a refused edge — as `Fail::findings` at exit 1 (`engine.rs` `run_command`), so
/// routing this engine refusal through `json_refusal` would tell a script the corpus holds a bad
/// edge when the read never completed: the same trap `repair` fell into, one door over.
/// ⛔ WHETHER THIS LEG SHOULD BE EXIT 1 IS A LAW QUESTION AND IS DELIBERATELY NOT SETTLED HERE.
/// `docs/status.md:317` scopes the engine-refusal-is-exit-1 triad to *"read / put / pin"* BY
/// NAME, and `links` declares no triad of its own anywhere in `crates/mrd/src` — so the claim
/// that this leg is misclassified has no named law behind it yet. Charter 03 routes that to the
/// advisor rather than to a fix directive. THE FRAME IS OWED EITHER WAY, which is why closing
/// the envelope does not wait on the exit question.
#[test]
fn links_serves_the_envelope_when_the_corpus_read_refuses() {
    let (sb, ws) = sandbox();

    // CONTROL, and it must hold in the fixed AND the unfixed tree: this verb's `--json` SUCCESS
    // face works here. Without it a zero below could mean a mute verb or an unreachable degrade
    // rather than an absent frame — and this control exercises the SAME degrade path as the
    // subject, not merely the same binary (the daemon is spawn-impossible in this harness, so
    // both go in-process).
    let ok = run(&sb, &ws, &["links", "--json"]);
    assert_eq!(
        ok.status.code(),
        Some(0),
        "control: the whole-corpus edge map answers at exit 0; stderr {}",
        String::from_utf8_lossy(&ok.stderr)
    );
    let ok_frame: serde_json::Value =
        serde_json::from_slice(&ok.stdout).expect("control: the success face serves JSON");
    assert!(
        ok_frame.get("links").is_some(),
        "control: the success face is the edge map, so an empty stdout below is the ABSENT \
         FRAME and not a mute verb: {ok_frame}"
    );

    // The subject: a path the corpus does not hold. `links_rooted` refuses through
    // `links_nonmember` → `load_doc`, so this is an ordinary `file_not_found` on the seam —
    // a VALUE inside an invocation whose SHAPE is already legal, which is what makes it the
    // engine's refusal and not the CLI's.
    let out = run(&sb, &ws, &["links", "no-such.md", "--json"]);
    let frame = assert_envelope_at("links file_not_found", &out, 2);
    assert_eq!(
        frame.pointer("/error/code").unwrap(),
        "file_not_found",
        "the seam's §8 code rides the frame: {frame}"
    );

    // The envelope names the workspace the CALLER passed, so a refusal and a success from the
    // same invocation agree on that string. `in_process_links` canonicalises internally and the
    // frame deliberately does not use that value.
    assert_eq!(
        frame.get("workspace").and_then(|v| v.as_str()),
        ok_frame.get("workspace").and_then(|v| v.as_str()),
        "refusal and success name the same workspace: {frame} vs {ok_frame}"
    );

    // The other direction: the human face stays silent on stdout at the same leg, or an envelope
    // leaking into it would pass every assertion above.
    assert_human_face_is_silent_at(
        "links file_not_found (human)",
        &run(&sb, &ws, &["links", "no-such.md"]),
        2,
    );
}
