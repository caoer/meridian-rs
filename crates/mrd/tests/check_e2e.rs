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
use mrd::hook::FENCE_VERSION;
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

    /// A workspace with one page, declared a root by `mrd init`.
    fn workspace(&self) -> PathBuf {
        let ws = self.tmp.path().join("project");
        std::fs::create_dir_all(&ws).expect("mkdir");
        std::fs::write(ws.join("a.md"), "# A\n\nalpha\n").expect("a");
        let init = self.run(&ws, &["init"]);
        assert!(init.status.success(), "init: {}", stderr(&init));
        ws
    }

    /// A git-backed workspace, declared a root by `mrd init`. Git is real
    /// because `mrd pin` asks git real questions about the pinned blob — and it
    /// is what anchors the ladder here (`git-root`), which the declaration does
    /// not do.
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

/// The ruled reason word (S3-R6), spelled once here so a leg asserting its ABSENCE
/// cannot drift from the legs asserting its presence.
const GREY: &str = "grey(cannot-assess)";

/// **Row 21's fence block for a workspace that is no git repository**, spelled once
/// for the two key-for-key legs below.
///
/// The door-plane keys (`doors`, `fenced_doors`, `total_doors`) are **absent, not
/// null**: this face's law is that an absent field reads as *not checked*, and a
/// root with no hook directory has no door plane that could have been read. A
/// `null` would say the doors WERE read and came back as nothing — a different
/// fact, and a false one. Row 21's own suite (`s4r21_fence_line.rs`) holds that
/// absence against a fenced root where the same keys ARE present.
fn no_repo_fence(root: &Path) -> serde_json::Value {
    serde_json::json!({
        "state": "not-a-git-repo",
        "fenceable": false,
        "teaching": format!(
            "{} is not a git repository, so there is no hook directory to place a fence in. A \
             meridian workspace does not have to be a git repository — this is a supported \
             state, not a fault in the workspace.",
            root.display()
        ),
        "engine_version": FENCE_VERSION,
        "gates_the_exit": false,
    })
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
///
/// # The `interval:` line is an AMENDMENT with a named adjudicator (S3-R30)
/// This golden gained one line, and the artifact that adjudicates it is not this
/// unit's: **S3-R29 — "a byte check is only as wide as the INTERVAL IT SPANS, so
/// STATE THE INTERVAL WHENEVER YOU STATE THE CHECK"** — ruled before F1 was found
/// and unmoved by this change. The four journal/pin lines below are byte-identical;
/// what is added is the sentence the ruling requires, and it says plainly that a
/// bare `mrd check` did NOT read the index. *The pin caught the addition, which is
/// what it is for.*
///
/// # And the `fence:` line is a SECOND amendment, adjudicated by row 21
/// Its adjudicator is likewise not this unit's: **fence coverage is per-checkout
/// and opt-in, permanently, so a checkout that carries no fence must be TOLD so
/// unasked** — the silence, not the absence, was the defect. This workspace is not
/// a git repository, so the observed state is `not-a-git-repo` and there is no
/// door plane to print beside it. **Every verdict line above is byte-identical**,
/// and the exit code is unmoved — which is row 21's own central claim, held here
/// by `s4r21_fence_line.rs` over a checkout read fenced and unfenced.
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
            "check core {ws}\n  interval: worktree — the bytes on disk. The git INDEX was not \
             read, so this says nothing about what a commit would record: `mrd check --staged` \
             asks that question\n  chain: green\n  foreign_edit: none\n  pins: green\n  \
             anchoring: no pinned objects\n  fence: not-a-git-repo — {ws} is not a git \
             repository, so there is no hook directory to place a fence in. A meridian workspace \
             does not have to be a git repository — this is a supported state, not a fault in \
             the workspace · REPORTED, never gated on — fence coverage is a property of this \
             local checkout and not of the corpus, so this line does not move check's exit\n",
            ws = root.0.display()
        ),
        "the baseline-present render is pinned byte for byte — the two JOURNAL \
         lines are unchanged, U14's two PIN-PLANE lines and row 21's fence line \
         are the additions"
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
            },
            // U14: the pin plane, asserted key for key beside the untouched
            // `core` block. `asked: 0` is the POPULATION (S3-R23(5)) — this
            // workspace pins nothing, so the empty list is a reading of nothing
            // rather than a clean bill over something.
            "pins": {
                "red": [],
                "grey": [],
                "anchoring": {
                    "asked": 0,
                    "anchored": 0,
                    "pending_anchor": 0,
                    "never_anchored": 0,
                    "orphaned": [],
                },
                "anchoring_cannot_assess": null,
            },
            // F1 / S3-R29 — the interval, on the machine face. `not-asked` plus
            // `spans_the_commit: false` is the honest pair for a bare `mrd check`:
            // the answer is about the worktree, and the reader is told so rather
            // than left to assume the index was read.
            "interval": {
                "state": "not-asked",
                "spans_the_commit": false,
                "cannot_ask_detail": null,
                "diverged_paths": [],
                "staged": null,
            },
            // Row 21 — the CHECKOUT's fence coverage, reported beside the verdict
            // and reaching no exit. Top-level, never inside the interval objects:
            // it is a reading of the local checkout, not of any byte range.
            "fence": no_repo_fence(&root.0),
        }),
        "the baseline-present json: the `core` block is unchanged key for key, \
         the pin plane and row 21's fence block are the additions"
    );
}

// ── the vacuous-baseline path: grey, never green (S3-R5) ─────────────────────

/// **S3-R5, on finding-01's own corpus.** `mrd init` → `mrd pin` → git commit →
/// a plain shell rewrite of the PINNED section. The measured behaviour was
/// `chain: green · foreign_edit: none`, EXIT 0, on the exact out-of-band write the
/// fence exists to catch.
///
/// The assert is the REFUSAL (R26): check must not say green anywhere, and must
/// not exit clean. `mrd walk` reads the same corpus in the same run, holding the
/// cross-surface disagreement in view — it reddens, and check may not disagree by
/// claiming a clean bill it cannot support.
///
/// **U32 changed this leg's MECHANISM, not its verdict.** This fixture used to
/// assert that the pin journaled nothing — that WAS the vacuous baseline the grey
/// rested on. A splice now journals its row, so the baseline here is present and
/// the out-of-band rewrite makes it STALE; the refusal is now earned against
/// evidence instead of produced by its absence. Every refusal assert below is
/// unchanged; only the witness of the state the workspace is in was replaced.
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

    // U32: the governed pin journaled its row, and at THIS instant the workspace
    // is clean — the green-path control (S3-R8(c)), so the refusal below is caused
    // by the out-of-band write and not by the corpus.
    let root = WorkspaceRoot(workspace::canonicalize(&ws).expect("canonicalize"));
    assert_eq!(
        journal_rows(&root),
        1,
        "the pin is a guarded write and leaves a row"
    );
    let clean = sb.run(&ws, &["check"]);
    assert_eq!(
        clean.status.code(),
        Some(0),
        "the governed corpus is accepted: {}",
        said(&clean)
    );

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
    assert_eq!(
        journal_rows(&root),
        1,
        "and it journaled nothing — that is what makes it out-of-band, and it is \
         what leaves the present baseline STALE"
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

/// **The OTHER direction (S3-R8): the false RED, now closed at the root.** A
/// corpus in which every write was governed — a journaled `create`, then a
/// `mrd pin` through the CLI — and where **no out-of-band edit exists anywhere**.
///
/// The shipped verb answered `foreign_edit: RED`, exit 1 — *it accused a fully
/// governed workspace* — because the splice advanced the tree root and journaled
/// no row, so the recorded baseline went stale. u14i withdrew that accusation and
/// left a grey; **U32 removed the mechanism, so the honest answer here is now
/// GREEN, exit 0.** A fence built on this verb accepts a commit whose writes were
/// all governed, which is the green-path control criterion 5 lacked (S3-R8(c)).
///
/// This leg's assert INVERTED, and that is the unit's whole point: what changed is
/// the engine's behaviour, not the test's standard. The refusal direction is
/// asserted on its own corpus in `u32_governed_journal.rs`.
#[test]
fn check_accepts_a_fully_governed_corpus() {
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
        rows_before + 1,
        "THE MECHANISM U32 DELIVERS: the governed splice journaled its row, so the \
         baseline it advanced past is the baseline it refreshed"
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
        !text.contains(GREY),
        "nor may it refuse what it CAN now assess — every write here was governed \
         and every one of them is journaled: {text}"
    );
    assert!(
        text.contains("chain: green") && text.contains("foreign_edit: none"),
        "both detectors read a real verdict off a current baseline: {text}"
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "the green-path control (S3-R8(c)): a fully governed corpus is ACCEPTED: {text}"
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
    // NARROWED BY U14, and the narrowing is a sharpening. S3-R5's subject is the
    // two JOURNAL detectors — the assert's own message says so ("not for chain,
    // not for a second vacuous detector"). A bare `!contains("green")` now also
    // forbids the pin plane's green, which on this corpus is EARNED: nothing is
    // pinned, so nothing is owed. Naming the detectors asserts what the ruling
    // actually said, and it is strictly more precise than grepping a word with two
    // legitimate producers (S3-R23(1) — an instrument's precision buys its
    // survival). The pin plane's green is adjudicated by its own suite, which this
    // assert does not own: `u14_check_pin_plane.rs` proves it appears only when
    // earned and is replaced by a refusal when it is not.
    assert!(
        !text.contains("chain: green"),
        "no journal green without a baseline: {text}"
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
    // Narrowed to the journal detectors for the reason given on the sibling gate
    // above: this corpus pins nothing, so the pin plane's green is earned.
    assert!(
        !text.contains("chain: green"),
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
            },
            // U14: the pin plane rides beside the journal's `cannot_assess`, not
            // inside it. The two planes fail independently, so one refusing must
            // never be reported as the other refusing.
            "pins": {
                "red": [],
                "grey": [],
                "anchoring": {
                    "asked": 0,
                    "anchored": 0,
                    "pending_anchor": 0,
                    "never_anchored": 0,
                    "orphaned": [],
                },
                "anchoring_cannot_assess": null,
            },
            // F1 / S3-R29: the interval is STATED on the machine face too, and
            // `state: "not-asked"` is a fact — a bare `mrd check` did not read the
            // index, so `spans_the_commit` is FALSE and a reader cannot bank this
            // answer as being about their commit. The adjudicator for this addition
            // is S3-R29 itself (see the sibling render gate), not this unit.
            "interval": {
                "state": "not-asked",
                "spans_the_commit": false,
                "cannot_ask_detail": null,
                "diverged_paths": [],
                "staged": null,
            },
            // Row 21 — the fence block rides beside the journal's `cannot_assess`
            // for the same reason the pin plane does: it is a separate proposition
            // about a separate subject, and one refusing must never be reported as
            // the other refusing. `gates_the_exit: false` is the whole claim.
            "fence": no_repo_fence(&root.0),
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
