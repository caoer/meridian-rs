//! Gates for `mrd test --history` (U1.6), driving the REAL binary
//! (`CARGO_BIN_EXE_mrd`) over a REAL git repo built in a tempdir, with the
//! embedded seed convention (`reviewer-not-owner`) as the law.
//!
//! The seeded history exercises all three fidelity classes and both golden-list
//! outcomes:
//! - **r-000001** create by `agent:bob` — before absent (root commit) ⇒ class **A
//!   structural**; reviewer ≠ owner ⇒ passes.
//! - **r-000002** splice by `agent:alice` (the owner) — both sides recovered ⇒
//!   class **B full-bytes**; actor == owner ⇒ **would-refuse**.
//! - **r-000003** splice by `agent:bob` — both sides recovered ⇒ class **B**;
//!   reviewer ≠ owner ⇒ passes.
//! - **r-000004** splice on `tasks/ghost.md`, which never existed in git ⇒ class
//!   **C grey**; counted, never run.
//!
//! The invariants:
//! - an UNDECLARED would-refuse item (r-000002 absent from GOLDEN.md) FAILS the
//!   run (exit 1);
//! - once declared in GOLDEN.md the run passes (exit 0) with the reason rendered;
//! - the class-C grey count is asserted;
//! - `mrd rules replay` no longer parses (the retired verb, decision #8).

use std::path::Path;
use std::process::{Command, Output};

use policy::{ConventionFiles, seed_convention_files};

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

/// Run the mrd binary with `args`, returning the raw output.
fn mrd(args: &[&str]) -> Output {
    Command::new(mrd_bin())
        .args(args)
        .output()
        .expect("spawn mrd")
}

fn stdout(out: &Output) -> String {
    String::from_utf8(out.stdout.clone()).expect("utf8 stdout")
}

fn stderr(out: &Output) -> String {
    String::from_utf8(out.stderr.clone()).expect("utf8 stderr")
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("exit code")
}

/// Run a git command in `dir`, asserting success.
fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Write `body` to `dir/rel`, creating parent directories.
fn write(dir: &Path, rel: &str, body: &str) {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, body).unwrap();
}

/// Commit everything in the working tree with `message`.
fn commit(dir: &Path, message: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", message]);
}

/// A journal row line carrying the tokens `parse_rows` needs (roots + anchor).
fn row(op: &str, path: &str, actor: &str, seq: u64, rb: &str, ra: &str, extra: &str) -> String {
    format!(
        "- op={op} path={path} actor={actor} root_before={rb} root_after={ra} {extra} ^r-{seq:06}"
    )
}

const FIX_OPEN: &str = "\
---
owner: agent:alice
status: open
---

# Fix parser

The dialect parser drops trailing anchors on the last line.
";

const FIX_CLOSED: &str = "\
---
owner: agent:alice
status: closed
---

# Fix parser

The dialect parser drops trailing anchors on the last line.
";

const FIX_CLOSED_NOTE: &str = "\
---
owner: agent:alice
status: closed
---

# Fix parser

The dialect parser drops trailing anchors on the last line.

Reviewed by agent:bob.
";

/// Build the seeded git workspace: four commits, a journal of four rows, and the
/// embedded seed convention written into `conventions/reviewer-not-owner/`.
/// Returns the tempdir (kept alive by the caller).
fn seeded_workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tmpdir");
    let ws = dir.path();

    git(ws, &["init", "-q", "-b", "main"]);
    git(ws, &["config", "user.email", "test@meridian.local"]);
    git(ws, &["config", "user.name", "mrd-test"]);

    // The seed convention on disk (its exact shipped CHECK bytes).
    let seed = seed_convention_files();
    let check_md = seed.read("CHECK.md").expect("seed CHECK.md");
    write(ws, "conventions/reviewer-not-owner/CHECK.md", &check_md);

    // C0 — create fix-parser (by bob) + journal row r-000001 (create).
    write(ws, "tasks/fix-parser.md", FIX_OPEN);
    let j1 = format!(
        "# Receipt journal\n\n{}\n",
        row(
            "create",
            "tasks/fix-parser.md",
            "agent:bob",
            1,
            "b3:0",
            "b3:1",
            "before=absent after=aaaa edits=0",
        )
    );
    write(ws, "meridian/journal.md", &j1);
    commit(ws, "C0 create fix-parser by bob");

    // C1 — alice (the owner) closes her own task + r-000002 (splice).
    write(ws, "tasks/fix-parser.md", FIX_CLOSED);
    let j2 = format!(
        "{}{}\n",
        j1,
        row(
            "splice",
            "tasks/fix-parser.md",
            "agent:alice",
            2,
            "b3:1",
            "b3:2",
            "edits=1 status:put:aaaa->bbbb",
        )
    );
    write(ws, "meridian/journal.md", &j2);
    commit(ws, "C1 alice closes her own task");

    // C2 — bob (a reviewer) edits fix-parser + r-000003 (splice).
    write(ws, "tasks/fix-parser.md", FIX_CLOSED_NOTE);
    let j3 = format!(
        "{}{}\n",
        j2,
        row(
            "splice",
            "tasks/fix-parser.md",
            "agent:bob",
            3,
            "b3:2",
            "b3:3",
            "edits=1 Fix parser:put:bbbb->cccc",
        )
    );
    write(ws, "meridian/journal.md", &j3);
    commit(ws, "C2 bob edits fix-parser");

    // C3 — a row for a path that never existed in git (class C grey).
    let j4 = format!(
        "{}{}\n",
        j3,
        row(
            "splice",
            "tasks/ghost.md",
            "agent:alice",
            4,
            "b3:3",
            "b3:4",
            "edits=1 Ghost:put:0000->1111",
        )
    );
    write(ws, "meridian/journal.md", &j4);
    commit(ws, "C3 ghost row for a path absent from git");

    dir
}

/// The undeclared would-refuse item (r-000002) fails the run; the report shows it
/// failing, names the journal span, counts every fidelity class, and the grey
/// count is asserted.
#[test]
fn undeclared_would_refuse_fails_the_run() {
    let dir = seeded_workspace();
    let ws = dir.path().to_str().unwrap();

    let out = mrd(&[
        "test",
        "--history",
        ws,
        "--convention",
        "reviewer-not-owner",
    ]);
    let so = stdout(&out);

    assert_eq!(
        code(&out),
        1,
        "an undeclared would-refuse item is a finding (exit 1):\n{so}\n{}",
        stderr(&out)
    );

    // The would-refuse item is shown failing, cited by anchor, and teaches the rule.
    assert!(so.contains("r-000002"), "the firing item is named:\n{so}");
    assert!(
        so.contains("UNDECLARED would-refuse"),
        "the item is shown failing:\n{so}"
    );
    assert!(
        so.contains("reviewer must not be the owner"),
        "the refusal message is rendered:\n{so}"
    );

    // The report names its journal span.
    assert!(
        so.contains("journal span: ^r-000001..^r-000004"),
        "the report names its journal span:\n{so}"
    );

    // Every fidelity class is counted; the class-C grey count is asserted.
    assert!(
        so.contains("B full-bytes=2"),
        "two rows reconstruct full-bytes:\n{so}"
    );
    assert!(
        so.contains("A structural=1"),
        "one row reconstructs structurally (the create):\n{so}"
    );
    assert!(
        so.contains("C grey=1"),
        "one row is grey — counted, never guessed:\n{so}"
    );
}

/// Declaring the item in GOLDEN.md (through the ordinary write door) turns the
/// finding into a pass, with the declared reason rendered.
#[test]
fn declared_item_passes_with_reason_rendered() {
    let dir = seeded_workspace();
    let ws = dir.path();
    let wss = ws.to_str().unwrap();

    // Triage: add the exception row to the golden page (an ordinary in-tree edit).
    let golden = "\
---
convention: reviewer-not-owner
---

# Golden list — reviewer-not-owner

- item=r-000002 reason=\"legacy self-close predates the reviewer-not-owner rule\"
";
    write(ws, "conventions/reviewer-not-owner/GOLDEN.md", golden);

    let out = mrd(&[
        "test",
        "--history",
        wss,
        "--convention",
        "reviewer-not-owner",
    ]);
    let so = stdout(&out);

    assert_eq!(
        code(&out),
        0,
        "a declared would-refuse item passes (exit 0):\n{so}\n{}",
        stderr(&out)
    );
    assert!(
        so.contains("would-refuse — declared"),
        "the item is rendered declared:\n{so}"
    );
    assert!(
        so.contains("legacy self-close predates the reviewer-not-owner rule"),
        "the declared reason is rendered:\n{so}"
    );
    assert!(
        !so.contains("would-refuse — UNDECLARED"),
        "no undeclared finding remains:\n{so}"
    );
    assert!(
        so.contains("0 UNDECLARED would-refuse"),
        "the summary shows zero undeclared findings:\n{so}"
    );
    // Grey is still counted even on a clean run.
    assert!(
        so.contains("C grey=1"),
        "the grey count survives a clean run:\n{so}"
    );
}

/// The JSON surface carries the fidelity tally and per-row verdicts (§4 preamble:
/// `--json` alongside the human table).
#[test]
fn json_surface_carries_fidelity_and_verdicts() {
    let dir = seeded_workspace();
    let ws = dir.path().to_str().unwrap();

    let out = mrd(&[
        "test",
        "--history",
        ws,
        "--convention",
        "reviewer-not-owner",
        "--json",
    ]);
    let so = stdout(&out);
    let v: serde_json::Value = serde_json::from_str(&so).expect("json parses");

    assert_eq!(v["fidelity"]["full_bytes"], 2);
    assert_eq!(v["fidelity"]["structural"], 1);
    assert_eq!(v["fidelity"]["grey"], 1);
    assert_eq!(v["summary"]["undeclared"], 1);
    assert_eq!(v["journal_span"]["first"], "r-000001");
    assert_eq!(v["journal_span"]["last"], "r-000004");
}

/// A golden exception row with no declared reason is a malformed golden list — a
/// declared exception must state why (exit 2, loud).
#[test]
fn golden_exception_without_a_reason_is_refused() {
    let dir = seeded_workspace();
    let ws = dir.path();
    let wss = ws.to_str().unwrap();
    write(
        ws,
        "conventions/reviewer-not-owner/GOLDEN.md",
        "- item=r-000002 no reason here\n",
    );

    let out = mrd(&[
        "test",
        "--history",
        wss,
        "--convention",
        "reviewer-not-owner",
    ]);
    assert_eq!(code(&out), 2, "a reasonless exception is a tool failure");
    assert!(
        stderr(&out).contains("reason"),
        "the refusal names the missing reason: {}",
        stderr(&out)
    );
}

/// Decision #8: `mrd rules replay` is retired the SAME release — no alias, no
/// shim. The verb no longer parses; the CLI refuses it loudly (exit 2).
#[test]
fn mrd_rules_replay_no_longer_parses() {
    let out = mrd(&["rules", "replay", "--rules", "x", "--snapshots", "y"]);
    assert_eq!(code(&out), 2, "the retired verb is a tool failure");
    assert!(
        stderr(&out).contains("unknown subcommand: rules"),
        "the CLI refuses the retired verb: {}",
        stderr(&out)
    );
}
