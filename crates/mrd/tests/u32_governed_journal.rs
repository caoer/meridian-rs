//! **U32 — a governed splice must leave a journal row.** End-to-end over the REAL
//! binary (`CARGO_BIN_EXE_mrd`, or `MRD_BIN` for the redden run against a pre-fix
//! build), driving the shipped CLI only — never a library call.
//!
//! # Both arms, or the unit is not done (S3-R8(c))
//! A guard proven only by what it BLOCKS is indistinguishable from a guard that
//! blocks everything, so every leg here is paired:
//!
//! - [`governed_writes_leave_check_green`] — the **acceptance**: a corpus whose
//!   every byte was written by a meridian writer reads green and exits **0**.
//! - [`an_out_of_band_edit_leaves_check_non_zero`] — the **refusal**: the SAME
//!   corpus plus one plain shell rewrite may not exit 0 and may not print green.
//! - [`the_two_arms_are_distinguishable`] — the pairing itself: before U32 both
//!   corpora produced the identical answer (`grey(cannot-assess)`, exit 1),
//!   because a governed splice and an out-of-writer edit left the identical
//!   trace. They must now differ.
//! - [`a_sequence_of_governed_writes_stays_green`] — **the heal** (S3-R8(d)): the
//!   false red was *permanent*, so one governed write proving green does not
//!   exhibit the staleness the defect is made of. Only a SEQUENCE does.
//!
//! # What the mechanism assert is
//! R40: never an exit status alone. Each leg reads the journal row that now
//! exists and asserts its `op`, its `root_before`/`root_after`, and that the last
//! row's `root_after` IS the live tree root — the baseline being current is the
//! state change U32 delivers, and the exit code is only its consequence.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use fs::WorkspaceRoot;
use fs::domain::RESERVED_JOURNAL_PATH;
use receipt::journal::{ParsedRow, parse_rows};

/// The binary every drive here goes through. `MRD_BIN` names another artifact —
/// the fixv convention (`crates/mrd/tests/s2fix_cross_surface.rs`), reused so the
/// SAME asserts can run against a pre-U32 build: on that binary the acceptance
/// leg and the heal leg must FAIL (a governed splice journals nothing, so the
/// baseline goes stale and check refuses), which is what "reddens" means here.
fn mrd_bin() -> PathBuf {
    std::env::var_os("MRD_BIN")
        .map_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_mrd")), PathBuf::from)
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

impl Sandbox {
    fn command(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut c = Command::new(mrd_bin());
        c.args(args)
            .current_dir(cwd)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env_remove("MERIDIAN_WORKSPACE");
        c
    }

    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        self.command(cwd, args).output().expect("spawn mrd")
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

    /// The one corpus both arms are cut from: a real git repo (a pin asks git
    /// real questions about the pinned blob), a pinnable source section, a
    /// claim page, and an editable plan page — then `mrd init`, then a commit.
    /// **Nothing here writes a journal row**; the writes under test come next.
    fn corpus(&self, name: &str) -> PathBuf {
        let ws = self.tmp.path().join(name);
        std::fs::create_dir_all(&ws).expect("mkdir");
        git(&ws, &["init", "-q"]);
        git(&ws, &["config", "user.email", "u32@example.invalid"]);
        git(&ws, &["config", "user.name", "u32"]);
        write(&ws, "source.md", "# Source\n\n## Guideline\n\nthe body\n");
        write(&ws, "claim.md", "# Claim\n\nwe rely on the guideline.\n");
        write(&ws, "plan.md", "# Plan\n\n## Goals\n\nalpha beta\n");
        let init = self.run(&ws, &["init"]);
        assert!(init.status.success(), "init: {}", stderr(&init));
        git(&ws, &["add", "-A"]);
        git(&ws, &["commit", "-qm", "corpus"]);
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

fn write(ws: &Path, rel: &str, body: &str) {
    std::fs::write(ws.join(rel), body).expect("write fixture");
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// stdout+stderr together — the render rides stdout, the refusal rides stderr,
/// and "is there a green anywhere" is a question about what the operator SEES.
fn said(out: &Output) -> String {
    format!("{}{}", stdout(out), stderr(out))
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

/// A one-edit `match` batch in the wire §4.4 grammar, against `Plan/Goals`.
fn goals_match(old: &str, new: &str) -> String {
    serde_json::to_string(&serde_json::json!([{
        "target": {"hpath": [{"h": "Plan"}, {"h": "Goals"}]},
        "edit": {"match": {"old": old, "new": new}},
    }]))
    .expect("edits json")
}

/// **The state change U32 delivers** (R40), asserted as facts and not as an exit
/// code: the journal grew by exactly one row; that row names `op`, both roots and
/// an `r-NNNNNN` anchor; its `root_before` continues the previous row (or is the
/// genesis row); and its `root_after` IS the live tree root — the baseline the
/// detectors need is now current. Returns the row for the caller to quote.
fn assert_journaled(root: &WorkspaceRoot, before: &[ParsedRow], op: &str, what: &str) -> ParsedRow {
    let after = rows(root);
    assert_eq!(
        after.len(),
        before.len() + 1,
        "{what}: exactly one journal row per guarded write (had {}, now {}): {after:#?}",
        before.len(),
        after.len()
    );
    let row = after.last().expect("the row just appended").clone();
    assert_eq!(row.op, op, "{what}: the row names its op");
    assert!(
        row.anchor.starts_with("r-"),
        "{what}: the row carries its block anchor: {row:?}"
    );
    if let Some(prev) = before.last() {
        assert_eq!(
            row.root_before, prev.root_after,
            "{what}: the row continues the chain — root_before == the prior row's root_after"
        );
    }
    assert_ne!(
        row.root_before, row.root_after,
        "{what}: a write that landed bytes moved the tree root"
    );
    assert_eq!(
        row.root_after,
        live_root(root),
        "{what}: THE FIX — the row's root_after IS the live tree, so the baseline is current"
    );
    row
}

// ── ARM 1 — the ACCEPTANCE ───────────────────────────────────────────────────

/// **A governed splice leaves `mrd check` GREEN.** Two governed writes through the
/// shipped CLI — `mrd pin` (a pin-only splice, the D7 shape that journaled nothing)
/// and `mrd put` (a batch splice) — and after each one the journal carries a row
/// whose `root_after` is the live tree, so both layer-0 detectors have a current
/// baseline and `check` exits 0.
///
/// **This is the leg that reddens on a pre-U32 binary**: there the splice journals
/// no row, the journal stays empty, and `check` refuses `grey(cannot-assess)` on a
/// workspace where nothing ungoverned ever happened.
#[test]
fn governed_writes_leave_check_green() {
    let sb = sandbox();
    let ws = sb.corpus("acceptance");
    let root = root_of(&ws);

    assert!(
        rows(&root).is_empty(),
        "the corpus journals nothing before the writes under test"
    );

    // (1) `mrd pin` — the pin-only splice. FINDING 01's own write.
    let before = rows(&root);
    let pin = sb.run(&ws, &["pin", "claim.md", "source.md#Source/Guideline"]);
    assert_eq!(pin.status.code(), Some(0), "pin: {}", said(&pin));
    assert!(
        std::fs::read_to_string(ws.join("claim.md"))
            .expect("claim")
            .contains("meridian-lock"),
        "the governed write landed bytes"
    );
    let pin_row = assert_journaled(&root, &before, "splice", "mrd pin");

    let out = sb.run(&ws, &["check"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "a governed splice leaves check GREEN — row ^{} {} -> {}: {}",
        pin_row.anchor,
        pin_row.root_before,
        pin_row.root_after,
        said(&out)
    );
    assert_eq!(
        stdout(&out),
        format!(
            "check core {}\n  chain: green\n  foreign_edit: none\n",
            root.0.display()
        ),
        "and it is the honest green, earned against a current baseline"
    );

    // (2) `mrd put` — a batch splice, so the row is not a pin special case.
    let before = rows(&root);
    let put = sb.run_stdin(
        &ws,
        &["put", "plan.md", "--actor", "agent:alice"],
        &goals_match("alpha beta", "alpha beta gamma"),
    );
    assert_eq!(put.status.code(), Some(0), "put: {}", said(&put));
    assert!(
        std::fs::read_to_string(ws.join("plan.md"))
            .expect("plan")
            .contains("alpha beta gamma"),
        "the batch splice landed bytes"
    );
    let put_row = assert_journaled(&root, &before, "splice", "mrd put");
    assert_eq!(
        put_row.path, "plan.md",
        "the row names the file the splice landed on"
    );
    assert_eq!(
        put_row.actor.as_deref(),
        Some("agent:alice"),
        "and the actor exactly as given (§9)"
    );
    assert_eq!(put_row.edits, 1, "one edit rode this batch");

    let out = sb.run(&ws, &["check"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "still green after the second governed write — row ^{} {} -> {}: {}",
        put_row.anchor,
        put_row.root_before,
        put_row.root_after,
        said(&out)
    );
    assert!(
        stdout(&out).contains("chain: green") && stdout(&out).contains("foreign_edit: none"),
        "both detectors read a verdict, not a grey: {}",
        stdout(&out)
    );
}

// ── ARM 2 — the REFUSAL ──────────────────────────────────────────────────────

/// **An out-of-band edit leaves `mrd check` NON-ZERO.** Same corpus, same governed
/// pin — then a plain shell rewrite of the pinned section, through no meridian
/// writer at all (the human-in-Obsidian case the ratified design names).
///
/// The assert is the refusal (R26): no green anywhere, and a non-clean exit. The
/// engine still declines to NAME the cause — a stale baseline is also what a write
/// door that does not journal would leave, and one such door survives U32 by
/// charter (`mrd realise --truth file`'s bare `std::fs::write`, U31/U12). Refusing
/// with the evidence is what it can prove; accusing would be a claim wider than it.
#[test]
fn an_out_of_band_edit_leaves_check_non_zero() {
    let sb = sandbox();
    let ws = sb.corpus("refusal");
    let root = root_of(&ws);

    let before = rows(&root);
    let pin = sb.run(&ws, &["pin", "claim.md", "source.md#Source/Guideline"]);
    assert_eq!(pin.status.code(), Some(0), "pin: {}", said(&pin));
    let governed = assert_journaled(&root, &before, "splice", "mrd pin");

    // The control: at this instant the corpus is fully governed and green.
    let clean = sb.run(&ws, &["check"]);
    assert_eq!(
        clean.status.code(),
        Some(0),
        "the green-path control — the refusal below must be caused by the EDIT, \
         not by the corpus: {}",
        said(&clean)
    );

    // The out-of-band write.
    write(
        &ws,
        "source.md",
        "# Source\n\n## Guideline\n\nOUT OF BAND\n",
    );

    // R40: assert the state change, never a command's exit status.
    assert!(
        std::fs::read_to_string(ws.join("source.md"))
            .expect("source")
            .contains("OUT OF BAND"),
        "the out-of-band rewrite landed on disk"
    );
    assert_eq!(
        rows(&root).len(),
        before.len() + 1,
        "and it journaled nothing — that is what makes it out-of-band"
    );
    assert_ne!(
        live_root(&root),
        governed.root_after,
        "the live tree no longer folds to the root the last governed write recorded"
    );

    let out = sb.run(&ws, &["check"]);
    let text = said(&out);
    assert_ne!(
        out.status.code(),
        Some(0),
        "check may not exit clean on an out-of-band write: {text}"
    );
    assert!(
        !text.contains("green"),
        "nor print green ANYWHERE — not for chain, not for a second detector: {text}"
    );
    assert!(
        !text.contains("foreign_edit: none"),
        "`none` is the foreign_edit detector's green: {text}"
    );
    assert!(
        text.contains(&governed.root_after) && text.contains(&governed.anchor),
        "and the refusal cites its evidence — the last receipt and the root it \
         recorded: {text}"
    );

    // The cross-surface control: the same corpus, the same run. walk agrees.
    let walk = sb.run(&ws, &["walk", "claim.md"]);
    assert_eq!(
        walk.status.code(),
        Some(1),
        "walk reddens on this corpus too — the planes agree: {}",
        said(&walk)
    );
}

// ── THE PAIRING — the two arms must be DISTINGUISHABLE ───────────────────────

/// **The defect stated as one sentence:** before U32, a governed splice and an
/// out-of-writer edit left `mrd check` the *identical* answer — `grey(cannot-assess)`
/// on both detectors, exit 1 — because the splice advanced the tree root and
/// journaled nothing, so both regimes reached the verb as the same stale baseline.
///
/// A fix that only reddens harder is indistinguishable from one that refuses
/// everything (S3-R8(c)). So this leg runs BOTH corpora in one test and asserts
/// they now answer differently: exit 0 versus non-zero, on inputs that differ by
/// exactly one shell write.
#[test]
fn the_two_arms_are_distinguishable() {
    let sb = sandbox();

    let governed_ws = sb.corpus("pair-governed");
    let pin = sb.run(
        &governed_ws,
        &["pin", "claim.md", "source.md#Source/Guideline"],
    );
    assert_eq!(pin.status.code(), Some(0), "pin: {}", said(&pin));

    let foreign_ws = sb.corpus("pair-foreign");
    let pin = sb.run(
        &foreign_ws,
        &["pin", "claim.md", "source.md#Source/Guideline"],
    );
    assert_eq!(pin.status.code(), Some(0), "pin: {}", said(&pin));
    write(
        &foreign_ws,
        "source.md",
        "# Source\n\n## Guideline\n\nOUT OF BAND\n",
    );

    let governed = sb.run(&governed_ws, &["check"]);
    let foreign = sb.run(&foreign_ws, &["check"]);

    assert_eq!(
        governed.status.code(),
        Some(0),
        "the governed corpus is accepted: {}",
        said(&governed)
    );
    assert_ne!(
        foreign.status.code(),
        Some(0),
        "the out-of-band corpus is refused: {}",
        said(&foreign)
    );
    assert_ne!(
        governed.status.code(),
        foreign.status.code(),
        "the two corpora differ by ONE shell write and the verb must tell them \
         apart — before U32 both answered exit 1"
    );
}

// ── THE HEAL — a SEQUENCE, because the false red was PERMANENT ───────────────

/// **The false red never healed** — the next governed write's `root_before` no
/// longer continued the prior `root_after`, so once the baseline went stale the
/// chain reddened too and stayed that way. One governed write proving green does
/// not exhibit that; only a sequence does (S3-R8(d): *what corpus could have shown
/// this, and does mine?*).
///
/// Six governed writes of three shapes — pin, put, put, pin, put, put — each
/// followed by a full `mrd check`. Every step asserts the row it added continues
/// the chain and re-dates the tree, and every check exits 0.
#[test]
fn a_sequence_of_governed_writes_stays_green() {
    let sb = sandbox();
    let ws = sb.corpus("heal");
    let root = root_of(&ws);
    write(&ws, "second.md", "# Second\n\nanother claim.\n");
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-qm", "second claim"]);

    let mut body = "alpha beta".to_string();
    let mut trace = Vec::new();

    for step in 1..=6 {
        let before = rows(&root);
        match step {
            1 => {
                let out = sb.run(&ws, &["pin", "claim.md", "source.md#Source/Guideline"]);
                assert_eq!(
                    out.status.code(),
                    Some(0),
                    "step {step} pin: {}",
                    said(&out)
                );
            }
            4 => {
                let out = sb.run(&ws, &["pin", "second.md", "source.md#Source/Guideline"]);
                assert_eq!(
                    out.status.code(),
                    Some(0),
                    "step {step} pin: {}",
                    said(&out)
                );
            }
            _ => {
                let next = format!("{body} w{step}");
                let out = sb.run_stdin(
                    &ws,
                    &["put", "plan.md", "--actor", "agent:alice"],
                    &goals_match(&body, &next),
                );
                assert_eq!(
                    out.status.code(),
                    Some(0),
                    "step {step} put: {}",
                    said(&out)
                );
                body = next;
            }
        }

        let row = assert_journaled(&root, &before, "splice", &format!("step {step}"));
        trace.push(format!(
            "step {step}: ^{} {} -> {}",
            row.anchor, row.root_before, row.root_after
        ));

        let out = sb.run(&ws, &["check"]);
        assert_eq!(
            out.status.code(),
            Some(0),
            "step {step} — a sequence of governed writes STAYS green, it does not \
             go stale after the first one.\nchain so far:\n{}\ncheck said: {}",
            trace.join("\n"),
            said(&out)
        );
        assert!(
            stdout(&out).contains("chain: green") && stdout(&out).contains("foreign_edit: none"),
            "step {step}: both detectors keep reading a verdict: {}",
            stdout(&out)
        );
    }

    assert_eq!(
        rows(&root).len(),
        6,
        "six guarded writes, six journal rows:\n{}",
        trace.join("\n")
    );
}
