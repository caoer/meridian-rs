//! End-to-end gates for `mrd check` (U2.10), driving the REAL binary
//! (`CARGO_BIN_EXE_mrd`) over its process boundary. Three legs of one verb:
//!
//! - **green** — a journal carrying real `create` rows whose last receipt matches
//!   the live tree reads green and exits 0. Asserted render-for-render (the whole
//!   stdout, and the whole `--json` value), so the baseline-PRESENT path is pinned
//!   byte-for-byte and the grey below cannot leak into it.
//! - **red** — a spliced/forged journal row reddens with the row cited, exit 1.
//! - **grey** — S3-R5: with NO journal baseline BOTH layer-0 detectors are
//!   vacuous, so `check` reports that it CANNOT ASSESS and does not exit clean.
//!   The corpus is finding-01's own — `init` → `pin` → git commit → a plain shell
//!   rewrite of the pinned section, the exact out-of-band write the fence exists
//!   to catch — and `mrd walk` reads the SAME corpus in the SAME run, so the
//!   cross-surface disagreement that the finding measured stays visible here.
//!
//! The grey asserts are shaped by R26: they assert the **absence of green** and a
//! non-clean exit, never the presence of a new string. A gate that only greps for
//! `cannot assess` passes while a second vacuous detector still prints green.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use fs::WorkspaceRoot;
use fs::domain::RESERVED_JOURNAL_PATH;
use wire::Path as WirePath;
use wire_serve::write::{CreateArgs, create};

/// The binary every drive here goes through. `MRD_BIN` names another artifact —
/// the fixv convention (`crates/mrd/tests/s2fix_cross_surface.rs`), reused here so
/// the SAME asserts can run against a pre-change build: the baseline-present legs
/// must pass on both binaries (that is what "unchanged" means), and the grey legs
/// must fail on the old one (that is what "reddens" means).
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
    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        Command::new(mrd_bin())
            .args(args)
            .current_dir(cwd)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env_remove("MERIDIAN_WORKSPACE")
            .output()
            .expect("spawn mrd")
    }

    /// A workspace with one page + an `mrd init` marker.
    fn workspace(&self) -> PathBuf {
        let ws = self.tmp.path().join("project");
        std::fs::create_dir_all(&ws).expect("mkdir");
        std::fs::write(ws.join("a.md"), "# A\n\nalpha\n").expect("a");
        let init = self.run(&ws, &["init"]);
        assert!(init.status.success(), "init: {}", stderr(&init));
        ws
    }

    /// A git-backed workspace + an `mrd init` marker. Git is real because `mrd
    /// pin` asks git real questions about the pinned blob.
    fn git_workspace(&self, name: &str) -> PathBuf {
        let ws = self.tmp.path().join(name);
        std::fs::create_dir_all(&ws).expect("mkdir");
        git(&ws, &["init", "-q"]);
        git(&ws, &["config", "user.email", "check-e2e@example.invalid"]);
        git(&ws, &["config", "user.name", "check-e2e"]);
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

/// stdout+stderr together — the render rides stdout, the exit message rides
/// stderr, and an "is there a green anywhere" assert cares about what the
/// operator SEES.
fn said(out: &Output) -> String {
    format!("{}{}", stdout(out), stderr(out))
}

fn write(ws: &Path, rel: &str, body: &str) {
    std::fs::write(ws.join(rel), body).expect("write fixture");
}

/// One journal row line in the `render_row` grammar (`parse_rows` reads
/// `root_before=`, `root_after=`, and the trailing `^r-NNNNNN`).
fn row(anchor: &str, root_before: &str, root_after: &str) -> String {
    format!(
        "- op=splice path=a.md root_before={root_before} root_after={root_after} edits=0 ^{anchor}"
    )
}

/// How many rows the reserved journal carries right now — the baseline's size,
/// read the way the detectors read it.
fn journal_rows(root: &WorkspaceRoot) -> usize {
    let page = std::fs::read_to_string(root.0.join(RESERVED_JOURNAL_PATH)).unwrap_or_default();
    receipt::journal::parse_rows(&page).len()
}

/// Birth `path` through the PRODUCTION guarded-create write path — the real
/// journal-row-minting edge, so the baseline-present fixture below has a journal
/// nobody hand-wrote.
fn produce(root: &WorkspaceRoot, path: &str, body: &str) {
    let args = CreateArgs {
        id: None,
        path: WirePath(path.to_string()),
        body: body.to_string(),
        actor: Some("agent:alice".to_string()),
        now: None,
        if_root: None,
        dry: false,
    };
    create(root, 0, &args, &[])
        .unwrap_or_else(|e| panic!("production create {path} refused: {e:?}"));
}

// ── the baseline-PRESENT path: unchanged, pinned render-for-render ───────────

/// **The unchanged path (S3-R5 bound).** A workspace that journals `create` rows
/// has a real baseline: the chain recompute has rows to recompute and the last
/// receipt's `root_after` is the live tree root, so `check` reads GREEN and exits
/// 0 — the honest green, earned against evidence.
///
/// The whole stdout is asserted, not a substring: this is the render that must
/// stay byte-identical while the vacuous-baseline path turns grey.
#[test]
fn check_is_green_when_the_journal_carries_create_rows() {
    let sb = sandbox();
    let ws = sb.workspace();
    let root = WorkspaceRoot(workspace::canonicalize(&ws).expect("canonicalize"));

    produce(&root, "b.md", "# B\n\nbeta\n");
    produce(&root, "c.md", "# C\n\ngamma\n");
    let journal = std::fs::read_to_string(root.0.join(RESERVED_JOURNAL_PATH)).expect("journal");
    assert!(
        journal.contains("^r-000001") && journal.contains("^r-000002"),
        "two real create rows landed — the baseline this leg is about:\n{journal}"
    );

    let out = sb.run(&ws, &["check"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "a baseline-present clean tree exits 0: {}",
        said(&out)
    );
    assert_eq!(
        stdout(&out),
        format!(
            "check core {}\n  chain: green\n  foreign_edit: none\n",
            root.0.display()
        ),
        "the baseline-present render is unchanged, byte for byte"
    );

    let out = sb.run(&ws, &["check", "--json"]);
    assert_eq!(out.status.code(), Some(0), "json green exits 0");
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(
        value,
        serde_json::json!({
            "workspace": root.0.display().to_string(),
            "red": false,
            "core": {
                "chain": { "green": true, "breaks": [] },
                "foreign_edit": null,
                "drifted_claims": [],
            }
        }),
        "the baseline-present json is unchanged, key for key"
    );
}

// ── the vacuous-baseline path: grey, never green (S3-R5) ─────────────────────

/// **S3-R5, on finding-01's own corpus.** `mrd init` → `mrd pin` → git commit →
/// a plain shell rewrite of the PINNED section. That workspace journals nothing
/// (the pin rides `commit_batch`, D7), so both layer-0 detectors have no baseline
/// — and the measured behaviour was `chain: green · foreign_edit: none`, EXIT 0,
/// on the exact out-of-band write the fence exists to catch.
///
/// The assert is the REFUSAL (R26): check must not say green anywhere, and must
/// not exit clean. `mrd walk` reads the same corpus in the same run, holding the
/// cross-surface disagreement in view — it reddens, and check may not disagree by
/// claiming a clean bill it cannot support.
#[test]
fn check_cannot_assess_on_the_pin_only_corpus_that_walk_reddens() {
    let sb = sandbox();
    let ws = sb.git_workspace("pin-only");
    write(
        &ws,
        "source.md",
        "# Source\n\n## Guideline\n\nthe pinned body\n",
    );
    write(&ws, "claim.md", "# Claim\n\nwe rely on the guideline.\n");
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-qm", "init"]);

    let pin = sb.run(&ws, &["pin", "claim.md", "source.md#Source/Guideline"]);
    assert_eq!(pin.status.code(), Some(0), "pin: {}", said(&pin));
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-qm", "pin"]);

    // The out-of-band write: a plain rewrite of the pinned section through no
    // meridian writer at all — the human-in-Obsidian case the design names.
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
    assert!(
        !ws.join(RESERVED_JOURNAL_PATH).exists(),
        "a pin/put-only workspace journals nothing — this is the vacuous baseline \
         under test, and if it ever gains rows this fixture is measuring something else"
    );

    let out = sb.run(&ws, &["check"]);
    let text = said(&out);

    assert!(
        !text.contains("green"),
        "S3-R5: with no journal baseline check may not print green ANYWHERE — \
         not for chain, not for a second vacuous detector: {text}"
    );
    assert!(
        !text.contains("foreign_edit: none"),
        "`none` is the foreign_edit detector's green — it has no baseline to say it from: {text}"
    );
    assert_ne!(
        out.status.code(),
        Some(0),
        "unknown is not clean: check may not exit 0 on a baseline it never had: {text}"
    );
    // S3-R6: exit 1 (the closed triad, no fourth code) carrying the RULED reason
    // word — the exit says "do not proceed", the word says why.
    assert_eq!(
        out.status.code(),
        Some(1),
        "grey refuses on the finding leg, not on a code of its own: {text}"
    );

    // BOTH vacuous detectors, not just the first one touched.
    assert!(
        text.contains("chain: grey(cannot-assess)"),
        "the chain recompute names itself with the ruled reason word: {text}"
    );
    assert!(
        text.contains("foreign_edit: grey(cannot-assess)"),
        "the foreign_edit trace names itself with the ruled reason word: {text}"
    );

    // The cross-surface control: the same corpus, the same run. walk reddens.
    let walk = sb.run(&ws, &["walk", "claim.md"]);
    assert_eq!(
        walk.status.code(),
        Some(1),
        "walk reddens on this corpus — the disagreement the finding measured: {}",
        said(&walk)
    );
    assert!(
        stdout(&walk).contains("red content-drifted"),
        "walk sees the out-of-band edit per-pin: {}",
        stdout(&walk)
    );
}

/// **The OTHER direction (S3-R8): the false RED.** A corpus in which every write
/// was governed — a journaled `create`, then a `mrd pin` through the CLI — and
/// where **no out-of-band edit exists anywhere**. The splice advances the tree root
/// and journals no row, so the recorded baseline goes stale, and the shipped verb
/// answered `foreign_edit: RED`, exit 1: *it accused a fully governed workspace.*
/// The ratified fence built on that sentence would block every governed commit.
///
/// The assert is the refusal in this direction too: check must NOT say
/// red-foreign-edit, and must not say green either (the green half of that render
/// was equally unearned — the baseline it rested on was stale).
///
/// This does NOT make the fence usable: the honest answer is still a refusal
/// (grey, exit 1). It stops the verb from lying about WHY.
#[test]
fn check_does_not_accuse_a_fully_governed_corpus() {
    let sb = sandbox();
    let ws = sb.git_workspace("all-governed");
    write(&ws, "source.md", "# Source\n\n## Guideline\n\nthe body\n");
    write(&ws, "claim.md", "# Claim\n\nwe rely on the guideline.\n");
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-qm", "init"]);

    // A journaled write establishes a REAL baseline: `create` records the live
    // tree root in its row, so at this instant check has everything it needs.
    let root = WorkspaceRoot(workspace::canonicalize(&ws).expect("canonicalize"));
    produce(&root, "note.md", "# Note\n\ngoverned birth\n");
    let rows_before = journal_rows(&root);
    assert_eq!(rows_before, 1, "one journaled write — the baseline");
    let out = sb.run(&ws, &["check"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "control: with a CURRENT baseline this corpus is green: {}",
        said(&out)
    );

    // One ordinary governed write later — through the shipped CLI, no raw edit.
    let pin = sb.run(&ws, &["pin", "claim.md", "source.md#Source/Guideline"]);
    assert_eq!(pin.status.code(), Some(0), "pin: {}", said(&pin));

    // R40: assert the state change and the mechanism, never a command's exit.
    assert!(
        std::fs::read_to_string(ws.join("claim.md"))
            .expect("claim")
            .contains("meridian-lock"),
        "the governed write landed"
    );
    assert_eq!(
        journal_rows(&root),
        rows_before,
        "THE MECHANISM: a governed splice journals no row, so the baseline it \
         should have refreshed stays where it was"
    );

    let out = sb.run(&ws, &["check"]);
    let text = said(&out);
    assert!(
        !text.contains("foreign_edit: RED"),
        "S3-R8: no out-of-band edit exists in this corpus — accusing one is the \
         false red the fence would have blocked every governed commit on: {text}"
    );
    assert!(
        !text.contains("out-of-writer edit landed"),
        "and the accusation must not survive in the exit message either: {text}"
    );
    assert!(
        !text.contains("green"),
        "nor may the other detector claim a green off the same stale baseline: {text}"
    );
    assert!(
        text.contains("chain: grey(cannot-assess)")
            && text.contains("foreign_edit: grey(cannot-assess)"),
        "both detectors report what they can prove: {text}"
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "still a refusal (S3-R6) — honest, not permissive: {text}"
    );
}

/// The same vacuity with no pin plane in sight: a workspace whose journal page
/// does not exist has no baseline either, so `check` cannot assess it. (This
/// SUPERSEDES the former `check_green_on_fresh_workspace`, which asserted the
/// false green S3-R5 overrules: "a fresh workspace reads GREEN" was check
/// reporting a clean bill on evidence it never had.)
#[test]
fn check_cannot_assess_a_workspace_with_no_journal_page() {
    let sb = sandbox();
    let ws = sb.workspace();
    assert!(
        !ws.join(RESERVED_JOURNAL_PATH).exists(),
        "no journal page — the vacuous baseline"
    );

    let out = sb.run(&ws, &["check"]);
    let text = said(&out);
    assert!(
        !text.contains("green"),
        "no green without a baseline: {text}"
    );
    assert!(
        !text.contains("foreign_edit: none"),
        "no vacuous `none` either: {text}"
    );
    assert_ne!(out.status.code(), Some(0), "unknown is not clean: {text}");
}

/// A journal page that exists but parses to ZERO rows is the same vacuity by a
/// different route — the detectors read parsed rows, not bytes. Without this the
/// grey could be implemented as "the file is missing" and still ship a green on
/// an empty (or unparseable) journal.
#[test]
fn check_cannot_assess_an_empty_journal_page() {
    let sb = sandbox();
    let ws = sb.workspace();
    let root = WorkspaceRoot(workspace::canonicalize(&ws).expect("canonicalize"));
    std::fs::create_dir_all(root.0.join("meridian")).expect("meridian dir");
    std::fs::write(
        root.0.join(RESERVED_JOURNAL_PATH),
        "# Receipt journal\n\nno rows yet.\n",
    )
    .expect("write journal");

    let out = sb.run(&ws, &["check"]);
    let text = said(&out);
    assert!(
        !text.contains("green"),
        "zero parsed rows is zero baseline: {text}"
    );
    assert!(
        !text.contains("foreign_edit: none"),
        "zero parsed rows is zero baseline: {text}"
    );
    assert_ne!(out.status.code(), Some(0), "unknown is not clean: {text}");
}

/// The `--json` face of the grey: `red` stays honest (grey is not red), the two
/// vacuous detectors are `null` rather than a green object, and the
/// `cannot_assess` block carries the SAME ruled reason word the human line does
/// (S3-R6 — distinct on both faces), plus which detectors it covers. Exit 1.
#[test]
fn check_json_says_cannot_assess_and_names_both_detectors() {
    let sb = sandbox();
    let ws = sb.workspace();
    let root = WorkspaceRoot(workspace::canonicalize(&ws).expect("canonicalize"));

    let out = sb.run(&ws, &["check", "--json"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "unknown is not clean in json either — and it rides the finding leg: {}",
        said(&out)
    );
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(
        value,
        serde_json::json!({
            "workspace": root.0.display().to_string(),
            "red": false,
            "cannot_assess": {
                "reason": "grey(cannot-assess)",
                "detectors": ["chain", "foreign_edit"],
                "detail": "the receipt journal carries no row, so the chain recompute \
                           and the foreign_edit trace have nothing to compare against",
                "baseline": null,
            },
            "core": {
                "chain": null,
                "foreign_edit": null,
                "drifted_claims": [],
            }
        }),
        "the grey json: no `green: true` for a reader to mistake for assessed, and \
         the ruled reason word verbatim"
    );
}

// ── the RED path: unchanged ─────────────────────────────────────────────────

/// A spliced/forged journal row reddens `mrd check`: the chain recompute cites the
/// forged row and the verb exits 1 (a finding, never a door refusal). The last
/// honest row's `root_after` is pinned to the LIVE tree root, so `foreign_edit`
/// stays clear and the chain break is the isolated signal.
#[test]
fn check_reddens_and_cites_a_spliced_journal_row() {
    let sb = sandbox();
    let ws = sb.workspace();

    // The live tree root the binary will fold at check time (journal-excluded, so
    // writing the journal below does not move it).
    let root = WorkspaceRoot(workspace::canonicalize(&ws).expect("canonicalize"));
    let live = fs::domain_snapshot(&root).expect("snapshot").1.0;

    // Two honest rows chain (R0 -> R1 -> LIVE); a forged row is spliced BETWEEN
    // them. The forged row's roots continue nothing, breaking the chain; the last
    // honest row's root_after == LIVE, so there is no foreign_edit.
    let journal = format!(
        "# Receipt journal\n{}\n{}\n{}\n",
        row("r-000001", "b3:R0", "b3:R1"),
        row("r-000099", "b3:FORGED_BEFORE", "b3:FORGED_AFTER"),
        row("r-000002", "b3:R1", &live),
    );
    std::fs::create_dir_all(root.0.join("meridian")).expect("meridian dir");
    std::fs::write(root.0.join(RESERVED_JOURNAL_PATH), journal).expect("write journal");

    let out = sb.run(&ws, &["check"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "a spliced journal row is a finding (exit 1): {} / {}",
        stdout(&out),
        stderr(&out)
    );
    let text = stdout(&out);
    assert!(text.contains("chain: RED"), "the chain reddens: {text}");
    assert!(
        text.contains("r-000099"),
        "the render cites the forged row end-to-end: {text}"
    );
    assert!(
        text.contains("foreign_edit: none"),
        "the last honest row matches the live root — no foreign_edit noise: {text}"
    );
}

/// The `--json` face carries the chain break and the top-level `red` verdict, and
/// still exits 1.
#[test]
fn check_json_carries_the_break_and_reddens() {
    let sb = sandbox();
    let ws = sb.workspace();

    let root = WorkspaceRoot(workspace::canonicalize(&ws).expect("canonicalize"));
    let live = fs::domain_snapshot(&root).expect("snapshot").1.0;
    let journal = format!(
        "# Receipt journal\n{}\n{}\n{}\n",
        row("r-000001", "b3:R0", "b3:R1"),
        row("r-000099", "b3:FORGED_BEFORE", "b3:FORGED_AFTER"),
        row("r-000002", "b3:R1", &live),
    );
    std::fs::create_dir_all(root.0.join("meridian")).expect("meridian dir");
    std::fs::write(root.0.join(RESERVED_JOURNAL_PATH), journal).expect("write journal");

    let out = sb.run(&ws, &["check", "--json"]);
    assert_eq!(out.status.code(), Some(1), "json red still exits 1");
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(value["red"], serde_json::json!(true));
    assert_eq!(value["core"]["chain"]["green"], serde_json::json!(false));
    let breaks = value["core"]["chain"]["breaks"].as_array().expect("breaks");
    assert!(
        breaks.iter().any(|b| b["row_anchor"] == "r-000099"),
        "the forged row is cited in json: {breaks:?}"
    );
    assert_eq!(value["core"]["foreign_edit"], serde_json::Value::Null);
}
