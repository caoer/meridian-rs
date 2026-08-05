//! **U11 — criterion 3, BOTH HALVES, driven through the REAL `mrd walk`.** `mrd read` may NOT
//! be cited for a per-item verdict (plan §9.1: its only colour channel is daemon-only, carries
//! tone but no reason, and is skipped exactly in the unresolvable-target case).
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

/// The live document-root rev of `raw` — the same parse `fs::build_corpus` runs
/// The live fingerprint token of a page's document root — what a CORRECT R4 pin
/// holds, minted through the engine's own mint over the same parse the corpus
/// builder runs.
fn live_fingerprint(raw: &str) -> String {
    let doc = model::build(raw.to_string(), syntax::parse(raw));
    model::fingerprint::fingerprint(&doc, &doc.root)
        .expect("the fixture page has content")
        .into_string()
}

/// One whole-body R4 pin, hand-written in the exact bytes `lock::render` emits — so these gates
/// depend on the CLI's own READER, never on the writer that made them. `object` keeps its
/// `root:` prefix: the canonical agent-plane spelling. NOTE FOR REVIEWERS: `version: 2` is the
/// LOCK FILE schema version, not the wire protocol version.
///
///
fn lock_block(object: &str, fingerprint: &str) -> String {
    format!(
        "```meridian-lock\nversion: 2\npins:\n  - object: \"[[{object}]]\"\n    \
         hash: \"9ae3f1deadbeef\"\n    path: []\n    fingerprint: \"{fingerprint}\"\n```"
    )
}

/// The TRUE target, in the `sessions` root.
const TARGET: &str = "# Notes\n\n## Design\n\nthe sessions root's real design note.\n";
/// The DECOY: same basename, ambient root, DIFFERENT bytes. This is the file the
/// shipped engine resolves `sessions:…/notes.md` onto (FINDING 03).
const DECOY: &str = "# Notes\n\n## Design\n\nAMBIENT ROOT FILE — the wrong document.\n";

struct Sandbox {
    /// Load-bearing: the sandbox tree is deleted when this drops, so it is held
    /// for its Drop and read by nothing.
    #[allow(dead_code)]
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
    /// The ambient workspace.
    ws: PathBuf,
    /// The mounted root's directory.
    sessions: PathBuf,
    /// `MERIDIAN.md` — the mount table.
    config: PathBuf,
}

/// A `MERIDIAN.md` mount table binding `sessions` to `dir`, or binding nothing.
fn config_raw(sessions: Option<&Path>) -> String {
    use std::fmt::Write as _;
    let mut raw = "---\ntype: meridian-config\nversion: 1\n---\n\n# Test roots\n\n".to_string();
    if let Some(dir) = sessions {
        let _ = write!(
            raw,
            "```meridian-mount\nname: sessions\npath: {}\nkind: vault\nvault: sessions\n```\n",
            dir.display()
        );
    }
    raw
}

fn sandbox() -> Sandbox {
    let tmp = tempfile::tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let ws = tmp.path().join("project");
    let sessions = tmp.path().join("sessions");
    for d in [&home, &ws, &sessions] {
        std::fs::create_dir_all(d).expect("mkdir");
    }

    // The mounted root declares its own canonical name (INV-5) — without this
    // the bind renders grey(undeclared) and every acceptance below is vacuous.
    std::fs::write(
        sessions.join("MERIDIAN.md"),
        "---\ntype: meridian-root\nversion: 1\nname: sessions\n---\n\n# Sessions root\n",
    )
    .expect("root declaration");
    std::fs::write(sessions.join("notes.md"), TARGET).expect("target");
    std::fs::write(ws.join("notes.md"), DECOY).expect("decoy");

    let config = home.join("MERIDIAN.md");
    std::fs::write(&config, config_raw(Some(&sessions))).expect("config");

    let cache_home = tmp.path().join("xdg-cache");
    Sandbox {
        tmp,
        cache_home,
        home,
        ws,
        sessions,
        config,
    }
}

impl Sandbox {
    fn run(&self, args: &[&str]) -> Output {
        Command::new(mrd_bin())
            .args(args)
            .current_dir(&self.ws)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("HOME", &self.home)
            .env("MERIDIAN_CONFIG", &self.config)
            .env_remove("MERIDIAN_WORKSPACE")
            .output()
            .expect("spawn mrd")
    }

    /// Write `claim.md` pinning `ref` at the fingerprint `rev`, then `mrd init`.
    fn claim(&self, r#ref: &str, rev: &str) {
        let claim = format!(
            "# Claim\n\ndraws from the sessions root.\n\n{}\n",
            lock_block(r#ref, rev)
        );
        std::fs::write(self.ws.join("claim.md"), claim).expect("claim");
        let init = self.run(&["init"]);
        assert!(init.status.success(), "init: {}", stderr(&init));
    }

    /// Unmount `sessions` by rewriting the table with no mount entry.
    fn unmount(&self) {
        std::fs::write(&self.config, config_raw(None)).expect("rewrite config");
    }

    /// The one walk entry, as `(color, reason, detail, selector)`.
    fn walk_entry(&self) -> (Output, String, String, String, String) {
        let out = self.run(&["walk", "claim.md", "--json"]);
        let v: Value = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("walk json ({e}): {}", stdout(&out)));
        let entries = v["walk"]["entries"].as_array().expect("entries").to_owned();
        assert_eq!(entries.len(), 1, "one pin, one entry: {entries:?}");
        let e = &entries[0];
        (
            out.clone(),
            e["color"].as_str().unwrap_or_default().to_string(),
            e["reason"].as_str().unwrap_or_default().to_string(),
            e["detail"].as_str().unwrap_or_default().to_string(),
            e["selector"].as_str().unwrap_or_default().to_string(),
        )
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// **CRITERION 3, HALF ONE — the resolved BYTES come from the TARGET ROOT**, proven by editing
/// the file in that root and observing the change, and by editing the ambient decoy and
/// observing NO change.
///
///
///
///
///
///
#[test]
fn the_resolved_bytes_come_from_the_target_root_not_the_ambient_decoy() {
    let sb = sandbox();
    sb.claim("sessions:notes", &live_fingerprint(TARGET));

    // (a) GREEN — the pin matches the TARGET root's bytes.
    let (out, color, reason, _detail, selector) = sb.walk_entry();
    assert_eq!(
        color,
        "green",
        "a cross-root ref to a MOUNTED root renders its true verdict: {} / {}",
        stdout(&out),
        stderr(&out),
    );
    assert!(reason.is_empty(), "green carries no reason word: {reason}");
    assert_eq!(
        selector, "sessions:notes.md",
        "the listing names the ROOT-QUALIFIED address, never the bare in-root path",
    );
    assert!(out.status.success(), "a green walk exits 0");

    // (b) THE CONTROL, FIRST — edit the AMBIENT DECOY. If the engine were resolving onto the
    // ambient file (FINDING 03), THIS is the edit that would move the verdict. It must not.
    //
    std::fs::write(
        sb.ws.join("notes.md"),
        "# Notes\n\n## Design\n\nDECOY EDITED.\n",
    )
    .expect("edit the decoy");
    let (_out, color_after_decoy, _r, _d, _s) = sb.walk_entry();
    assert_eq!(
        color_after_decoy, "green",
        "editing the AMBIENT root's same-basename file must not move a cross-root \
         verdict — if it does, the bytes are coming from the wrong root (FINDING 03)",
    );

    // (c) NOW edit the file IN THE TARGET ROOT. This is the edit that must move
    //     the verdict — and it moves it to RED content-drifted, not to grey.
    std::fs::write(
        sb.sessions.join("notes.md"),
        "# Notes\n\n## Design\n\nthe sessions root's design note, EDITED.\n",
    )
    .expect("edit the target");
    let (out, color, reason, _detail, _s) = sb.walk_entry();
    assert_eq!(
        color, "red",
        "editing the TARGET root's file moves the verdict — the bytes came from there",
    );
    assert_eq!(
        reason,
        "content-drifted",
        "a mounted root's drifted pin is RED content-drifted — a measured \
         disagreement, never grey: {}",
        stdout(&out),
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "a red edge is a finding — exit 1",
    );
}

/// **CRITERION 3, HALF TWO — an UNMOUNTED root renders GREY with a teaching refusal naming the
/// missing mount**, never red and never `file_not_found`. The pin is otherwise IDENTICAL to the
/// green case above — same ref, same rev, same bytes on disk. Only the mount table changed.
/// That isolation is the point (S3-R28(c): when two facts are cited as one risk, measure which
/// one carries it) — here the mount table alone carries the verdict.
///
///
#[test]
fn an_unmounted_root_renders_grey_with_a_teaching_refusal_naming_the_mount() {
    let sb = sandbox();
    sb.claim("sessions:notes", &live_fingerprint(TARGET));

    // It is GREEN while mounted — the acceptance half, asserted in the same breath (S3-R8(c)). A
    // build that renders EVERYTHING grey passes the rest of this test and ships nothing.
    //
    let (_out, color, _r, _d, _s) = sb.walk_entry();
    assert_eq!(color, "green", "mounted and matching — green");

    sb.unmount();

    let (out, color, reason, detail, selector) = sb.walk_entry();
    assert_eq!(
        color,
        "grey",
        "an unmounted root is GREY — nothing drifted, the ledger cannot see from \
         here (R-3: grey outranks red): {}",
        stdout(&out),
    );
    assert_eq!(
        reason,
        "unmounted",
        "S3-R6's reason word, distinct in `--json`: {}",
        stdout(&out),
    );
    assert!(
        detail.contains("sessions"),
        "the refusal NAMES the missing mount (D8): {detail}",
    );
    assert_eq!(
        selector, "sessions:notes.md",
        "an unmounted edge is named by the address the page declared",
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "grey REFUSES on exit 1 — no fourth exit code (S3-R6): {}",
        stderr(&out),
    );

    // The human line carries the same distinction — not only `--json`.
    let human = sb.run(&["walk", "claim.md"]);
    let text = stdout(&human);
    assert!(
        text.contains("grey unmounted"),
        "the human line carries the reason word too: {text}",
    );
    assert!(
        text.contains("sessions"),
        "and it names the missing mount: {text}",
    );
    // The TONE is what must not be red — asserted on the entry row's colour label, not by hunting
    // the substring "red" in the whole output. The crude form of this assertion FAILED once the
    // teaching refusal gained its output path, because the refusal's own wording carries
    //
    //
    //
    //
    //
    //
    let entry_row = text
        .lines()
        .find(|l| l.contains("sessions:notes.md"))
        .unwrap_or_else(|| panic!("no entry row: {text}"));
    assert!(
        entry_row.contains("grey unmounted"),
        "the row's colour label is the grey: {entry_row}",
    );
    assert!(
        !entry_row.contains(" red "),
        "and it is never red — nothing drifted: {entry_row}",
    );
}

/// **GATE 2 — unmounted and missing-file are DISTINCT CLASSES.** A mounted-root-missing-file
/// must NOT render grey `unmounted`: it is a measured absence inside a root this machine CAN
/// see. Conflating the two is the false negative the grey class exists to prevent.
///
#[test]
fn a_missing_file_in_a_mounted_root_is_not_the_unmounted_grey() {
    let sb = sandbox();
    // `sessions` IS mounted; the file it names is not there.
    sb.claim("sessions:absent", &live_fingerprint(TARGET));

    let (out, color, reason, _detail, _s) = sb.walk_entry();
    assert_ne!(
        reason,
        "unmounted",
        "a MOUNTED root whose file is missing is not the unmounted grey — the \
         root is visible, the file is absent: {}",
        stdout(&out),
    );
    assert_eq!(
        color,
        "red",
        "an address that resolves to nothing inside a visible root is a measured \
         absence, rendered red: {}",
        stdout(&out),
    );
    // **U21 CHANGED THIS WORD, and the change is the unit's whole point.** U11 asserted
    // `selector-unresolved` here because that was the only red word the type could reach:
    // `RefResolution::NotFound` was a unit variant, so no caller could say WHICH root missed, and
    // the row fell through to the arm that classifies an absent target document.
    //
    //
    //
    //
    //
    //
    //
    //
    //
    //
    assert_eq!(
        reason,
        "file-not-found",
        "and it keeps its own reason word, never borrowing grey's — nor the \
         selector plane's, which would claim the page resolved: {}",
        stdout(&out),
    );
    // The half U11 could not assert: the refusal is SCOPED to the root that missed. Its grey
    // sibling has named its root since U11; this one could not until the type carried it.
    //
    let (_out, _c, _r, detail, _s) = sb.walk_entry();
    assert!(
        detail.contains("sessions"),
        "the refusal names the root the file is missing INSIDE: {detail}",
    );
}

/// **GATE 4 — the basename fallback PEELS AND REFUSES, on FINDING 03's input VERBATIM.**
/// `[[sessions:24-01-retro/notes.mdDesign]]` must never answer the ambient root's `notes.md`,
/// with `sessions` unbound. This is the shipped defect's exact spelling, including the slash
/// the defect needs — the control in FINDING 03 is why that matters: the shortest test case is
/// the one that behaves correctly.
///
///
#[test]
fn finding_03s_verbatim_input_peels_and_refuses() {
    let sb = sandbox();
    sb.unmount(); // `sessions` unbound — FINDING 03's exact condition
    sb.claim("sessions:24-01-retro/notes", &live_fingerprint(DECOY));

    let (out, color, reason, detail, _s) = sb.walk_entry();
    assert_eq!(
        color,
        "grey",
        "FINDING 03's input must not resolve to the ambient root's notes.md — \
         it names an unbound root and is grey: {}",
        stdout(&out),
    );
    assert_eq!(reason, "unmounted");
    assert!(
        detail.contains("sessions"),
        "naming the missing mount: {detail}"
    );
    assert_eq!(out.status.code(), Some(1), "grey refuses on exit 1");

    // **C-4 — one address, one answer.** The no-slash spelling is the control that survived the
    // shipped defect by behaving correctly; after U11 the two spellings must CONVERGE, and that
    // convergence is the assert.
    sb.claim("sessions:notes", &live_fingerprint(DECOY));
    let (_out, color2, reason2, _d, _s) = sb.walk_entry();
    assert_eq!(
        (color.as_str(), reason.as_str()),
        (color2.as_str(), reason2.as_str()),
        "the slashed and unslashed spellings of one address must give ONE answer",
    );
}
