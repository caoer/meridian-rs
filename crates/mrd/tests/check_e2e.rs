//! End-to-end gates for `mrd check` (U2.10), driving the real binary
//! (`CARGO_BIN_EXE_mrd`) over its process boundary.
//!
//! Three legs:
//!
//! - **green** — a governed corpus reads green and exits 0, asserted render-for-render
//!   and key-for-key so the mandatory `write_history: not-assessed` disclosure cannot
//!   be dropped without this file failing.
//! - **red** — an out-of-band rewrite of pinned content reddens, exit 1, cited per-pin.
//! - **blind** — a forged journal-shaped page moves no verdict, asserted rather than
//!   left unsaid ([`check_is_blind_to_a_forged_journal_page`]).
//!
//! `check` reads no write history: the engine keeps no memory — history is pinned to
//! git at lock. Grey asserts follow R26: absence of green and a non-clean exit, never
//! merely the presence of a new string.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use fs::WorkspaceRoot;
use mrd::hook::FENCE_VERSION;
use std::collections::BTreeMap;
use wire::Path as WirePath;
use wire_serve::write::{CreateArgs, create};

/// The binary every drive here goes through. `MRD_BIN` names another artifact — the fixv
/// convention (`crates/mrd/tests/s2fix_cross_surface.rs`), reused here so the SAME asserts can
/// run against a pre-change build.
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

    /// A git-backed workspace, declared a root by `mrd init`. Git is real because `mrd pin` asks
    /// git real questions about the pinned blob — and it is what anchors the ladder here
    /// (`git-root`), which the declaration does not do.
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

/// stdout+stderr together — the render rides stdout, the exit message rides stderr, and an "is
/// there a green anywhere" assert cares about what the operator sees.
fn said(out: &Output) -> String {
    format!("{}{}", stdout(out), stderr(out))
}

fn write(ws: &Path, rel: &str, body: &str) {
    std::fs::write(ws.join(rel), body).expect("write fixture");
}

/// The ruled reason word (S3-R6), spelled once here so a leg asserting its ABSENCE
/// cannot drift from the legs asserting its presence.
const GREY: &str = "grey(cannot-assess)";

/// The retired reserved path, spelled as a literal on purpose: the constant is deleted and
/// this path reserves nothing, so re-importing one would imply a reservation that no longer
/// exists. It must be this path, not a merely-unused one.
const FORMER_JOURNAL_PATH: &str = "meridian/journal.md";

/// Row 21s fence block for a workspace that is no git repository, spelled once for the two
/// key-for-key legs below. The door-plane keys (`doors`, `fenced_doors`, `total_doors`) are
/// absent, not null: an absent field reads as *not checked*, and a root with no hook
/// directory has no door plane that could have been read.
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

/// The MANDATORY disclosure line, verbatim, spelled once (U5 § 6; advisor gate 1 §2). Every
/// render assert below carries it, so deleting the line fails this file rather than silently
/// returning a reader to the old, wider green. The line states the narrowing and the pointer
/// only; the mechanism WHY is docs/`--help` teaching (report-voice pass, ZT rulings 3–5).
fn write_history_line() -> String {
    format!(
        "  write_history: {} — chain continuity and last-receipt-vs-live are not checked; \
         history is in git\n",
        check::WRITE_HISTORY_NOT_ASSESSED
    )
}

/// Birth `path` through the production guarded-create write path — the real governed-write
/// edge, so a "governed corpus" fixture is one the engine wrote rather than one the test
/// hand-built.
fn produce(root: &WorkspaceRoot, path: &str, body: &str) {
    let args = CreateArgs {
        id: None,
        path: WirePath(path.to_string()),
        body: body.to_string(),
        actor: Some("agent:alice".to_string()),
        now: None,
        if_root: None,
        dry: false,
        fields: BTreeMap::default(),
    };
    create(root, None, &args, &[])
        .unwrap_or_else(|e| panic!("production create {path} refused: {e:?}"));
}

// ── the GREEN path: pinned render-for-render ─────────────────────────────────

/// The green: a workspace whose writes all went through the door has nothing drifted and
/// nothing unanchored, so `check` reads green and exits 0. The whole stdout is asserted, not
/// a substring — that is what makes the disclosure line load-bearing rather than decorative.
#[test]
fn check_is_green_on_a_governed_corpus_and_the_render_is_pinned() {
    let sb = sandbox();
    let ws = sb.workspace();
    let root = WorkspaceRoot(workspace::canonicalize(&ws).expect("canonicalize"));

    produce(&root, "b.md", "# B\n\nbeta\n");
    produce(&root, "c.md", "# C\n\ngamma\n");

    let out = sb.run(&ws, &["check"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "a governed clean tree exits 0: {}",
        said(&out)
    );
    assert_eq!(
        stdout(&out),
        format!(
            "check core {ws}\n  interval: worktree — the bytes on disk. The git INDEX was not \
             read, so this says nothing about what a commit would record: `mrd check --staged` \
             asks that question\n{disclosure}  pins: green\n  anchoring: no pinned objects\n  \
             fence: not-a-git-repo — {ws} is not a git repository, so there is no hook directory \
             to place a fence in. A meridian workspace does not have to be a git repository — \
             this is a supported state, not a fault in the workspace · REPORTED, never gated on \
             — fence coverage is a property of this local checkout and not of the corpus, so \
             this line does not move check's exit\n",
            ws = root.0.display(),
            disclosure = write_history_line()
        ),
        "the green render is pinned byte for byte — the two JOURNAL lines are GONE \
         and the write_history disclosure stands where they stood"
    );

    let out = sb.run(&ws, &["check", "--json"]);
    assert_eq!(out.status.code(), Some(0), "json green exits 0");
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(
        value,
        serde_json::json!({
            "workspace": root.0.display().to_string(),
            "red": false,
            // `write_history` replaces `core.chain` / `core.foreign_edit`; the key-for-key
            // compare holds the deleted keys deleted.
            "write_history": check::WRITE_HISTORY_NOT_ASSESSED,
            "core": {
                "drifted_claims": [],
            },
            // The pin plane, key for key. `asked: 0` is the population (S3-R23(5)) — this
            // workspace pins nothing, so the empty list is a reading of nothing.
            "pins": {
                "red": [],
                "grey": [],
                // Pins whose target the hash domain excludes — reported, never
                // gated (`wire-contract.md` §12.1 verdict-plane clause). Empty
                // here and PINNED empty: this shape gate is what forces the key
                // to be declared rather than to appear, and a caller gating on
                // `grey` must keep seeing an exclusion in its own key.
                "unattested": [],
                "anchoring": {
                    "asked": 0,
                    "anchored": 0,
                    "pending_anchor": 0,
                    "never_anchored": 0,
                    "orphaned": [],
                },
                "anchoring_cannot_assess": null,
                // Always present, even at count 0: an environment-varying key set would blur
                // "no cross-root pins" with "an engine that does not report them".
                "anchoring_out_of_jurisdiction": {
                    "count": 0,
                    "refs": [],
                    "owner": "u13_per_root_anchoring",
                },
            },
            // F1 / S3-R29 — the interval names which bytes an answer covers.
            "interval": {
                "state": "not-asked",
                "spans_the_commit": false,
                "cannot_ask_detail": null,
                "diverged_paths": [],
                "staged": null,
            },
            // Row 21 — the CHECKOUT's fence coverage, reported beside the verdict
            // and reaching no exit.
            "fence": no_repo_fence(&root.0),
        }),
        "the green json: `write_history` present, `core.chain` / `core.foreign_edit` \
         / `cannot_assess` absent"
    );
}

/// The green survives a fully governed corpus with a real pin in it — the green-path
/// control (S3-R8(c)).
#[test]
fn check_accepts_a_fully_governed_corpus() {
    let sb = sandbox();
    let ws = sb.git_workspace("all-governed");
    write(&ws, "source.md", "# Source\n\n## Guideline\n\nthe body\n");
    write(&ws, "claim.md", "# Claim\n\nwe rely on the guideline.\n");
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-qm", "init"]);

    let root = WorkspaceRoot(workspace::canonicalize(&ws).expect("canonicalize"));
    produce(&root, "note.md", "# Note\n\ngoverned birth\n");
    let out = sb.run(&ws, &["check"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "control: a governed corpus is green before the pin too: {}",
        said(&out)
    );

    // One ordinary governed write later — through the shipped CLI, no raw edit.
    let pin = sb.run(&ws, &["pin", "claim.md", "source.md#Source/Guideline"]);
    assert_eq!(pin.status.code(), Some(0), "pin: {}", said(&pin));

    // R40: assert the state change, never a command's exit status.
    assert!(
        std::fs::read_to_string(ws.join("claim.md"))
            .expect("claim")
            .contains("meridian-lock"),
        "the governed write landed"
    );

    let out = sb.run(&ws, &["check"]);
    let text = said(&out);
    assert!(
        !text.contains(GREY),
        "nothing here is unassessable — every plane the verb reads answered: {text}"
    );
    assert!(
        text.contains("pins: green"),
        "the pin plane reads a real verdict off the live content: {text}"
    );
    assert!(
        text.contains(check::WRITE_HISTORY_NOT_ASSESSED),
        "and the green is DISCLOSED as the narrower claim it is: {text}"
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "the green-path control (S3-R8(c)): a fully governed corpus is ACCEPTED: {text}"
    );
}

// ── the RED path: the surviving detector, and both surfaces now AGREE ────────

/// The pin plane reddens on an out-of-band rewrite of pinned content, and `mrd walk`
/// reddens on the same corpus in the same run: `red content-drifted`, citing the pin,
/// exit 1 — the same reason word on both surfaces, because both read
/// `view::walk::lock_pin_colors`. An out-of-band edit to content nothing pins is invisible
/// here; that case is asserted at [`check_is_blind_to_a_forged_journal_page`].
#[test]
fn check_reddens_on_the_pin_only_corpus_that_walk_also_reddens() {
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

    // Green-path control: at this instant the workspace is clean, so the refusal below is
    // caused by the out-of-band write and not by the corpus (S3-R8(c)).
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

    let out = sb.run(&ws, &["check"]);
    let text = said(&out);

    assert_ne!(
        out.status.code(),
        Some(0),
        "an out-of-band rewrite of PINNED content is a lie about the corpus, and \
         check may not exit clean over it: {text}"
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "a finding rides exit 1 — the triad stays CLOSED, no fourth code (S3-R6): {text}"
    );
    assert!(
        text.contains("pins: red content-drifted"),
        "the PIN PLANE is the detector that survived, and it names its colour and \
         its reason: {text}"
    );
    assert!(
        text.contains("claim.md → source.md#Source/Guideline"),
        "citing the pin per-pin, so the operator can locate it: {text}"
    );
    assert!(
        !text.contains(GREY),
        "and it is a VERDICT, not an unanswerable question — the grey this leg used \
         to assert had a journal baseline behind it and there is none: {text}"
    );

    // The cross-surface control: the same corpus, the same run. Both surfaces
    // redden, and with the SAME reason word — they share one computer.
    let walk = sb.run(&ws, &["walk", "claim.md"]);
    assert_eq!(
        walk.status.code(),
        Some(1),
        "walk reddens on this corpus: {}",
        said(&walk)
    );
    assert!(
        stdout(&walk).contains("red content-drifted"),
        "and it is the same reason word check printed — the disagreement finding-01 \
         measured is now an agreement by construction: {}",
        stdout(&walk)
    );
}

// ── the [LAW] moves: green where a grey used to stand ────────────────────────

/// U5 corpus row 01, tagged [LAW] — an empty workspace is green: the planes this verb reads
/// are clean, and dating write history is no longer asked.
#[test]
fn check_is_green_on_an_empty_workspace() {
    let sb = sandbox();
    let ws = sb.workspace();
    assert!(
        !ws.join(FORMER_JOURNAL_PATH).exists(),
        "nothing has written that path — and nothing would give it meaning if it had"
    );

    let out = sb.run(&ws, &["check"]);
    let text = said(&out);
    assert_eq!(
        out.status.code(),
        Some(0),
        "[LAW] corpus row 01: the planes this verb reads are clean, so it is green — \
         the grey it used to answer was about a plane it no longer has: {text}"
    );
    assert!(
        !text.contains(GREY),
        "there is nothing left here that could be unassessable: {text}"
    );
    assert!(
        text.contains(&write_history_line()),
        "AND THE DISCLOSURE IS MANDATORY: without this line a reader banks the old, \
         wider green, which is the false green this whole unit exists to prevent: {text}"
    );
}

/// The former reserved path is an ordinary page and moves no verdict: the path is reserved
/// by nothing, and a page sitting there is a page.
#[test]
fn the_former_journal_path_is_an_ordinary_page_and_moves_no_verdict() {
    let sb = sandbox();
    let ws = sb.workspace();
    let root = WorkspaceRoot(workspace::canonicalize(&ws).expect("canonicalize"));

    let before = sb.run(&ws, &["check"]);
    assert_eq!(before.status.code(), Some(0), "control: green before");

    std::fs::create_dir_all(root.0.join("meridian")).expect("meridian dir");
    std::fs::write(
        root.0.join(FORMER_JOURNAL_PATH),
        "# Receipt journal\n\nno rows yet.\n",
    )
    .expect("write the page");

    let after = sb.run(&ws, &["check"]);
    assert_eq!(
        after.status.code(),
        Some(0),
        "the page moved no verdict: {}",
        said(&after)
    );
    assert!(
        !said(&after).contains(GREY),
        "and produced no unassessable plane — there is no reader left to be confused \
         by it: {}",
        said(&after)
    );
}

/// The `--json` face of the green, asserted for what is absent as much as for what is present
/// (U5 § 6): `core.chain`, `core.foreign_edit` and the whole `cannot_assess` block are
/// removed as keys, not set to null.
#[test]
fn check_json_discloses_write_history_and_carries_no_chain_keys() {
    let sb = sandbox();
    let ws = sb.workspace();
    let root = WorkspaceRoot(workspace::canonicalize(&ws).expect("canonicalize"));

    let out = sb.run(&ws, &["check", "--json"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "[LAW] corpus row 01 on the machine face too: {}",
        said(&out)
    );
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");

    assert_eq!(
        value["write_history"],
        serde_json::json!(check::WRITE_HISTORY_NOT_ASSESSED),
        "the disclosure is on BOTH faces (S3-R6 — distinct on each): {value}"
    );
    assert!(
        value.get("cannot_assess").is_none(),
        "the block named `chain` and `foreign_edit` as its detectors; there are no \
         such detectors, so there is no block: {value}"
    );
    assert!(
        value["core"].get("chain").is_none(),
        "REMOVED, not nulled — a null would assert a read that never happened: {value}"
    );
    assert!(
        value["core"].get("foreign_edit").is_none(),
        "REMOVED, not nulled — this was the JSON twin of `foreign_edit: none`: {value}"
    );

    assert_eq!(
        value,
        serde_json::json!({
            "workspace": root.0.display().to_string(),
            "red": false,
            "write_history": check::WRITE_HISTORY_NOT_ASSESSED,
            "core": { "drifted_claims": [] },
            "pins": {
                "red": [],
                "grey": [],
                // Pins whose target the hash domain excludes — reported, never
                // gated (`wire-contract.md` §12.1 verdict-plane clause). Empty
                // here and PINNED empty: this shape gate is what forces the key
                // to be declared rather than to appear, and a caller gating on
                // `grey` must keep seeing an exclusion in its own key.
                "unattested": [],
                "anchoring": {
                    "asked": 0,
                    "anchored": 0,
                    "pending_anchor": 0,
                    "never_anchored": 0,
                    "orphaned": [],
                },
                "anchoring_cannot_assess": null,
                // Always present, even at count 0: an environment-varying key set would blur
                // "no cross-root pins" with "an engine that does not report them".
                "anchoring_out_of_jurisdiction": {
                    "count": 0,
                    "refs": [],
                    "owner": "u13_per_root_anchoring",
                },
            },
            "interval": {
                "state": "not-asked",
                "spans_the_commit": false,
                "cannot_ask_detail": null,
                "diverged_paths": [],
                "staged": null,
            },
            "fence": no_repo_fence(&root.0),
        }),
        "and the whole shape key for key, so a key cannot return unnoticed"
    );
}

// ── the BLIND arm: the deleted detector, asserted as deleted ─────────────────

/// The lost detector, pinned as an executable fact: a forged, chain-broken, journal-shaped
/// page is written into the workspace and `mrd check` answers green, exit 0, on both faces.
/// This asserts a reduction — `chain: RED` was `check`s only write-history red, and it is
/// gone.
#[test]
fn check_is_blind_to_a_forged_journal_page() {
    let sb = sandbox();
    let ws = sb.workspace();
    let root = WorkspaceRoot(workspace::canonicalize(&ws).expect("canonicalize"));

    let forged = "# Receipt journal\n\
                  - op=splice path=a.md root_before=b3:R0 root_after=b3:R1 edits=0 ^r-000001\n\
                  - op=splice path=a.md root_before=b3:FORGED_BEFORE root_after=b3:FORGED_AFTER \
                  edits=0 ^r-000099\n\
                  - op=splice path=a.md root_before=b3:R1 root_after=b3:LIVE edits=0 ^r-000002\n";
    std::fs::create_dir_all(root.0.join("meridian")).expect("meridian dir");
    std::fs::write(root.0.join(FORMER_JOURNAL_PATH), forged).expect("write the forged page");

    // R40: the state change, asserted before any verdict is read.
    assert!(
        std::fs::read_to_string(root.0.join(FORMER_JOURNAL_PATH))
            .expect("read back")
            .contains("r-000099"),
        "the forged row is on disk — the fixture is a fixture"
    );

    let out = sb.run(&ws, &["check"]);
    let text = said(&out);
    assert_eq!(
        out.status.code(),
        Some(0),
        "THE REDUCTION: a spliced row used to exit 1 citing r-000099. `check` does \
         not read write history at all, so the forged page is just a page: {text}"
    );
    assert!(
        !text.contains("RED"),
        "nothing reddens — there is no chain to break: {text}"
    );
    assert!(
        !text.contains("r-000099"),
        "and nothing cites the forged row, because nothing parsed it: {text}"
    );
    assert!(
        text.contains(check::WRITE_HISTORY_NOT_ASSESSED),
        "what the operator IS told is that write history was not assessed — the \
         disclosure is the whole mitigation, and it is the only one: {text}"
    );

    // The same blindness on the machine face, where a fence reads it.
    let js = sb.run(&ws, &["check", "--json"]);
    assert_eq!(
        js.status.code(),
        Some(0),
        "json is green too: {}",
        said(&js)
    );
    let value: serde_json::Value = serde_json::from_slice(&js.stdout).expect("json");
    assert_eq!(
        value["red"],
        serde_json::json!(false),
        "a machine consumer is told green, and told why the green is narrow: {value}"
    );
    assert_eq!(
        value["write_history"],
        serde_json::json!(check::WRITE_HISTORY_NOT_ASSESSED),
        "which is the disclosure doing the only job left to do: {value}"
    );
}
