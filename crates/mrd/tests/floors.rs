//! U4.4 floor conventions — the full enumerated suite, gated across all three
//! `mrd test` tiers plus the two message-naming refusals that the corpus tier's
//! fire-where-expected cannot assert (it records the fired RULE, not the message).
//!
//! The six floor conventions live under `tests/floors/conventions/<slug>/` (real
//! convention folders: `CHECK.md` + `base/` + `scenarios/`). This file:
//!
//! - **corpus tier** — drives `mrd test --corpus` over the six committed specs
//!   (`tests/floors/corpus/specs/*.md`): fire-where-expected + zero dead rules,
//!   exit 0 each.
//! - **scenario tier** — drives `mrd test tests/floors/scenarios/`: the bounce
//!   create-OR-replace Verdict LANDS (gate 1) and the first-arming write is
//!   ungated-but-journaled-and-grey (gate 7).
//! - **history tier** — builds a fresh temp git workspace, seeds the
//!   `reviewer-not-owner` floor + a journal, and drives `mrd test --history`:
//!   the owner-self-close history row is a would-refuse item, declared in a
//!   GOLDEN list.
//! - **message-naming** — loads the `claim-cas` and `verdict-reviewer-bind`
//!   floors through the SAME `policy` loader/evaluator the door uses and asserts
//!   the refusal NAMES the winner (gate 2) and the bound reviewer (gate 6).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

// ── shared helpers ────────────────────────────────────────────────────────────

/// The `mrd` binary under test.
fn mrd() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mrd"))
}

/// `tests/floors/…` under the crate manifest dir.
fn floors(sub: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/floors")
        .join(sub)
}

/// Run `mrd <args…>`, returning `(exit_code, stdout)`.
fn run(args: &[&str]) -> (i32, String) {
    let out: Output = mrd().args(args).output().expect("spawn mrd");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

// ── tier 2: the corpus expected-fire manifest ─────────────────────────────────

/// Every floor convention's corpus spec is fire-where-expected with zero dead
/// rules — exit 0. This is the `--corpus` tier recording for all six conventions.
#[test]
fn corpus_specs_all_fire_where_expected() {
    for slug in [
        "reviewer-not-owner",
        "claim-cas",
        "close-verdict",
        "decoy-close",
        "verdict-reviewer-bind",
        "meta-convention",
    ] {
        let spec = floors(&format!("corpus/specs/{slug}.md"));
        let (code, stdout) = run(&["test", "--corpus", spec.to_str().unwrap(), "--json"]);
        let report: Value = serde_json::from_str(&stdout)
            .unwrap_or_else(|e| panic!("{slug}: corpus JSON did not parse ({e}); stdout={stdout}"));
        assert_eq!(code, 0, "{slug}: corpus spec must exit 0 (report={report})");
        assert_eq!(
            report["summary"]["mismatches"], 0,
            "{slug}: no fire mismatch"
        );
        assert_eq!(report["summary"]["dead_rules"], 0, "{slug}: no dead rule");
        assert_eq!(report["summary"]["errors"], 0, "{slug}: no eval error");
    }
}

// ── tier 1: the scenario suite (gate 1 bounce, gate 7 first-arm grey) ──────────

/// The scenario suite passes: the bounce approve LANDS (create-OR-replace
/// Verdict) and the first-arming write is ungated-but-journaled-and-grey.
#[test]
fn scenario_suite_bounce_and_genesis_grey() {
    let dir = floors("scenarios");
    let (code, stdout) = run(&["test", dir.to_str().unwrap(), "--json"]);
    let report: Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("scenario JSON did not parse ({e}); stdout={stdout}"));
    assert_eq!(code, 0, "the scenario suite must exit 0 (report={report})");
    assert_eq!(report["summary"]["failed"], 0);
    assert_eq!(report["summary"]["malformed"], 0);
    assert_eq!(report["summary"]["pairing_hard_errors"], 0);
    for name in ["bounce-approve-lands", "first-arm-genesis-grey"] {
        let s = report["scenarios"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == name)
            .unwrap_or_else(|| panic!("scenario `{name}` not in report"));
        assert_eq!(s["outcome"], "passed", "{name} must pass");
        assert_eq!(s["expect_ok"], true, "{name} ^expect holds");
    }
}

// ── message-naming: gates 2 and 6 (the corpus tier records the rule, not text) ─

/// A convention folder on disk, read through the SAME `policy` loader the door
/// uses.
struct DirFiles {
    dir: PathBuf,
}

impl policy::ConventionFiles for DirFiles {
    fn read(&self, rel: &str) -> std::io::Result<String> {
        std::fs::read_to_string(self.dir.join(rel))
    }
    fn exists(&self, rel: &str) -> bool {
        self.dir.join(rel).exists()
    }
}

/// Build a path-stamped `model::Document` (the fixture doc-building path).
fn doc(path: &str, md: &str) -> model::Document {
    let mut d = model::build(md.to_string(), syntax::parse(md));
    if let model::NodeKind::Document { path: p, .. } = &mut d.root.kind {
        *p = path.to_string();
    }
    d
}

/// Load a floor convention by slug and run its `check_change` over the change
/// derived from `before`→`after` as `actor` (splice, no evidence edges).
fn refusals(
    slug: &str,
    path: &str,
    before_md: &str,
    after_md: &str,
    actor: &str,
) -> Vec<policy::Refusal> {
    let files = DirFiles {
        dir: floors(&format!("conventions/{slug}")),
    };
    let conv = policy::load_convention(slug, &files, policy::CheckLimits::default())
        .unwrap_or_else(|e| panic!("{slug} must load: {e}"));
    let before = doc(path, before_md);
    let after = doc(path, after_md);
    let change = policy::derive_change(
        &before,
        &after,
        &[],
        policy::Invocation {
            op: policy::ChangeOp::Splice,
            actor: Some(actor),
            force: false,
        },
        &[],
        &|_: &str| None,
    );
    conv.check_change(&change)
        .expect("check_change evaluates")
        .refusals
}

/// Gate 2 — a contested claim's refusal NAMES the winner (the current holder).
#[test]
fn claim_cas_refusal_names_the_winner() {
    let before = "---\nstatus: open\nowner: worker-a\n---\n# T\n\nx\n";
    let after = "---\nstatus: open\nowner: worker-b\n---\n# T\n\nx\n";
    let r = refusals("claim-cas", "tasks/t.md", before, after, "worker-b");
    assert_eq!(r.len(), 1, "a contested claim fires exactly once");
    assert!(
        r[0].message.contains("worker-a"),
        "the loser's refusal must name the winner (worker-a): {}",
        r[0].message
    );
    assert_eq!(r[0].passing_scenario, "scenarios/uncontested-claim.md");
}

/// Gate 6 — a Verdict naming a reviewer other than the closing actor is refused,
/// and the refusal NAMES the unbound reviewer.
#[test]
fn verdict_bind_refusal_names_the_reviewer() {
    let before = "---\nreviewer: carol\noutcome: pending\n---\n# V\n\nx\n";
    let after = "---\nreviewer: carol\noutcome: approve\n---\n# V\n\nx\n";
    let r = refusals(
        "verdict-reviewer-bind",
        "verdicts/v.md",
        before,
        after,
        "dave",
    );
    assert_eq!(r.len(), 1, "an unbound Verdict fires exactly once");
    assert!(
        r[0].message.contains("carol") && r[0].message.contains("dave"),
        "the refusal must name the named reviewer (carol) and the closing actor (dave): {}",
        r[0].message
    );
    assert_eq!(r[0].passing_scenario, "scenarios/bound-verdict.md");
}

// ── tier 3: the history run over a real temp git workspace ─────────────────────

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

/// Write `body` to `dir/rel`, creating parents.
fn write(dir: &Path, rel: &str, body: &str) {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, body).unwrap();
}

/// Commit the whole working tree.
fn commit(dir: &Path, message: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", message]);
}

/// One journal row (matches `receipt::journal::parse_rows`: `- op= path= actor=
/// root_before= root_after= … ^r-NNNNNN`).
fn row(op: &str, path: &str, actor: &str, seq: u64, rb: &str, ra: &str, extra: &str) -> String {
    format!(
        "- op={op} path={path} actor={actor} root_before={rb} root_after={ra} {extra} ^r-{seq:06}"
    )
}

const FIX_OPEN: &str =
    "---\ntype: task\nstatus: open\nowner: worker-a\n---\n\n# Fix parser\n\nbody\n";
const FIX_CLOSED: &str =
    "---\ntype: task\nstatus: closed\nowner: worker-a\n---\n\n# Fix parser\n\nbody\n";
const FIX_NOTE: &str =
    "---\ntype: task\nstatus: closed\nowner: worker-a\n---\n\n# Fix parser\n\nbody\n\n- reviewed\n";

/// Seed a temp git workspace with the reviewer-not-owner FLOOR + a journal whose
/// C1 row is an owner-self-close (would-refuse), C2 a reviewer edit (passes).
fn seeded_workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tmpdir");
    let ws = dir.path();

    git(ws, &["init", "-q", "-b", "main"]);
    git(ws, &["config", "user.email", "test@meridian.local"]);
    git(ws, &["config", "user.name", "mrd-test"]);

    // The floor convention on disk (the real U4.4 law, not the seed).
    let check = std::fs::read_to_string(floors("conventions/reviewer-not-owner/CHECK.md")).unwrap();
    write(ws, "conventions/reviewer-not-owner/CHECK.md", &check);

    // C0 — bob creates fix-parser (owner worker-a) + r-000001 (create).
    write(ws, "tasks/fix-parser.md", FIX_OPEN);
    let j1 = format!(
        "# Receipt journal\n\n{}\n",
        row(
            "create",
            "tasks/fix-parser.md",
            "worker-b",
            1,
            "b3:0",
            "b3:1",
            "before=absent after=aaaa edits=0"
        )
    );
    write(ws, "meridian/journal.md", &j1);
    commit(ws, "C0 create fix-parser");

    // C1 — worker-a (the OWNER) closes her own task + r-000002 (splice) → would-refuse.
    write(ws, "tasks/fix-parser.md", FIX_CLOSED);
    let j2 = format!(
        "{j1}{}\n",
        row(
            "splice",
            "tasks/fix-parser.md",
            "worker-a",
            2,
            "b3:1",
            "b3:2",
            "edits=1 status:put:aaaa->bbbb"
        )
    );
    write(ws, "meridian/journal.md", &j2);
    commit(ws, "C1 owner self-close");

    // C2 — reviewer-b edits + r-000003 (splice) → passes (reviewer != owner).
    write(ws, "tasks/fix-parser.md", FIX_NOTE);
    let j3 = format!(
        "{j2}{}\n",
        row(
            "splice",
            "tasks/fix-parser.md",
            "reviewer-b",
            3,
            "b3:2",
            "b3:3",
            "edits=1 Fix parser:put:bbbb->cccc"
        )
    );
    write(ws, "meridian/journal.md", &j3);
    commit(ws, "C2 reviewer edit");

    dir
}

/// Run `mrd test --history <ws> --convention reviewer-not-owner --json`.
fn run_history(ws: &Path, extra: &[&str]) -> (i32, Value) {
    let mut args = vec![
        "test",
        "--history",
        ws.to_str().unwrap(),
        "--convention",
        "reviewer-not-owner",
        "--json",
    ];
    args.extend_from_slice(extra);
    let out = mrd().args(&args).output().expect("spawn mrd");
    let code = out.status.code().unwrap_or(-1);
    let report: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "history JSON did not parse ({e}); stdout={}",
            String::from_utf8_lossy(&out.stdout)
        )
    });
    (code, report)
}

/// The history tier reconstructs the journal, fires the owner-self-close row as
/// an UNDECLARED would-refuse (exit 1), then a GOLDEN declaration flips it to a
/// declared exception (exit 0). This is the `--history` tier recording.
#[test]
fn history_owner_self_close_is_a_would_refuse_then_declared() {
    let dir = seeded_workspace();
    let ws = dir.path();

    // Undeclared: the owner self-close row is a would-refuse finding (exit 1).
    let (code, report) = run_history(ws, &[]);
    assert_eq!(
        code, 1,
        "an undeclared would-refuse is exit 1 (report={report})"
    );
    assert_eq!(
        report["summary"]["undeclared"], 1,
        "one undeclared would-refuse"
    );
    assert_eq!(report["fidelity"]["full_bytes"], 2, "C1+C2 are full-bytes");
    assert_eq!(
        report["fidelity"]["structural"], 1,
        "C0 create is structural"
    );

    // Declare it in the GOLDEN list → exit 0, one declared exception.
    let golden = "---\nconvention: reviewer-not-owner\n---\n\n# Golden list\n\n\
        - item=r-000002 reason=\"legacy owner self-close predates the reviewer-not-owner floor\"\n";
    write(ws, "conventions/reviewer-not-owner/GOLDEN.md", golden);
    let (code, report) = run_history(ws, &[]);
    assert_eq!(
        code, 0,
        "a declared would-refuse is exit 0 (report={report})"
    );
    assert_eq!(report["summary"]["undeclared"], 0, "nothing undeclared");
    assert_eq!(report["summary"]["declared"], 1, "one declared exception");
}
