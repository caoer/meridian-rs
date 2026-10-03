//! `mrd pin --refresh PAGE` over the process boundary: every row of a page's lock re-pinned
//! through the real daemon, in a real git work tree (the R4 blob plane needs git).
//!
//! The gates: an unchanged page refreshes green; a drifted target refuses with the lock bytes
//! untouched until `--accept`; two rows into one target that both promote anchors end on the
//! target file's final blob; a lock that will not parse, or a row no door re-pins, refuses
//! with nothing written. The rehearsal's drift signal is checked at its source too: a dry pin
//! carrying a stale token refuses `pin_proof_required`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::Value;

mod common;

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
}

struct Sandbox {
    tmp: tempfile::TempDir,
    cache_home: PathBuf,
    home: PathBuf,
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        common::reap_daemon(&self.home, &self.cache_home);
    }
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

/// Two sections, so one page can draw from both.
const TARGET: &str =
    "# Guide\n\n## Note\n\nsome text worth attesting\n\n## Other\n\nmore text worth attesting\n";
const PAGE: &str = "# Plan\n\na claim drawn from the guide.\n";

impl Sandbox {
    /// Writes are IPC: this path auto-spawns the test binary as the daemon
    /// (`MERIDIAN_DAEMON_BIN` = this `mrd`).
    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        common::mrd_command(&self.home, &self.cache_home)
            .args(args)
            .current_dir(cwd)
            .env("MERIDIAN_DAEMON_BIN", mrd_bin())
            .env_remove("MERIDIAN_WORKSPACE")
            .output()
            .expect("spawn mrd")
    }

    /// A git-backed marked workspace holding the target and the pinning page.
    fn workspace(&self) -> PathBuf {
        let ws = self.tmp.path().join("project");
        std::fs::create_dir_all(&ws).expect("mkdir");
        git(&ws, &["init", "-q"]);
        git(&ws, &["config", "user.email", "refresh@example.invalid"]);
        git(&ws, &["config", "user.name", "refresh"]);
        std::fs::write(ws.join("guide.md"), TARGET).expect("guide");
        std::fs::write(ws.join("plan.md"), PAGE).expect("plan");
        let init = self.run(&ws, &["init"]);
        assert!(init.status.success(), "init: {}", stderr(&init));
        ws
    }

    /// The workspace with plan.md drawing from both sections of guide.md — two rows into one
    /// target, each of which promoted its anchor.
    fn pinned_workspace(&self) -> PathBuf {
        let ws = self.workspace();
        for spec in ["guide.md#Guide/Note", "guide.md#Guide/Other"] {
            let out = self.run(&ws, &["pin", "plan.md", spec]);
            assert_eq!(code(&out), 0, "pin {spec}: {}", stderr(&out));
        }
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

/// The blob git computes for the file as it stands — what a row's `hash` must name.
fn blob_of(ws: &Path, rel: &str) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(ws)
        .args(["hash-object", rel])
        .output()
        .expect("git hash-object");
    assert!(out.status.success(), "git hash-object {rel}");
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("exit code")
}

fn read_file(ws: &Path, rel: &str) -> String {
    std::fs::read_to_string(ws.join(rel)).expect("read file")
}

/// The state word the human face printed on the row labelled `label` (`state  TARGET#SEL  …`).
fn state_of(text: &str, label: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        let state = words.next()?;
        (words.next()? == label).then(|| state.to_owned())
    })
}

/// plan.md's lock rows, read by the one lock parser.
fn rows(ws: &Path) -> Vec<lock::PinEntry> {
    let raw = read_file(ws, "plan.md");
    let doc = model::build(raw.clone(), syntax::parse(&raw));
    lock::find(&doc)
        .expect("the lock parses")
        .expect("plan.md carries a lock")
        .lock
        .pins
}

/// Every row's `hash` names the target file's final blob.
fn assert_hashes_are_final(ws: &Path) {
    let blob = blob_of(ws, "guide.md");
    for row in rows(ws) {
        assert_eq!(
            row.hash, blob,
            "row {:?} names guide.md's final blob",
            row.selector
        );
    }
}

/// `mrd walk plan.md` reads both rows green.
fn assert_walks_green(sb: &Sandbox, ws: &Path) {
    let walk = sb.run(ws, &["walk", "plan.md"]);
    let out = stdout(&walk);
    assert_eq!(code(&walk), 0, "walk is clean: {out}{}", stderr(&walk));
    for section in ["Guide/Note", "Guide/Other"] {
        assert!(
            out.contains(&format!("green  guide.md §{section}")),
            "{section} walks green: {out}"
        );
    }
}

/// An unchanged page refreshes at exit 0: both rows rehearse `ok`, keep their fingerprints,
/// walk green — and the first row's hash, minted before the second row's anchor landed in the
/// same file, now names that file's final blob.
#[test]
fn an_unchanged_page_refreshes_and_stays_green() {
    let sb = sandbox();
    let ws = sb.pinned_workspace();
    let before = rows(&ws);
    let guide = read_file(&ws, "guide.md");

    let out = sb.run(&ws, &["pin", "--refresh", "plan.md"]);
    assert_eq!(code(&out), 0, "refresh: {}{}", stdout(&out), stderr(&out));
    let text = stdout(&out);
    for label in ["guide.md#Guide/Note", "guide.md#Guide/Other"] {
        assert_eq!(state_of(&text, label).as_deref(), Some("ok"), "{text}");
    }
    assert!(text.contains("refreshed 2 pin(s) in plan.md"), "{text}");

    let after = rows(&ws);
    assert_eq!(after.len(), 2, "a refresh adds no row");
    for (old, new) in before.iter().zip(&after) {
        assert_eq!(new.selector, old.selector, "lock order and keys kept");
        assert_eq!(new.fingerprint, old.fingerprint, "the claim did not move");
    }
    assert_eq!(
        read_file(&ws, "guide.md"),
        guide,
        "no anchor promoted again"
    );
    assert_hashes_are_final(&ws);
    assert_walks_green(&sb, &ws);
}

/// A drifted target refuses at exit 1 with both files byte-untouched and names the loop; the
/// rehearsal's signal is a dry pin with the stale token refusing `pin_proof_required`.
/// `--dry --accept` rehearses the accept; `--accept` lands it and the page walks green.
#[test]
fn a_drifted_target_refuses_until_accepted() {
    let sb = sandbox();
    let ws = sb.pinned_workspace();
    let recorded = rows(&ws)[0].fingerprint.clone();
    let moved = read_file(&ws, "guide.md").replace("some text worth", "some text no longer");
    std::fs::write(ws.join("guide.md"), &moved).expect("drift the target");

    // The drift signal at its source: the dry pin a rehearsal sends, with the stale token.
    let probe = sb.run(
        &ws,
        &[
            "pin",
            "plan.md",
            "guide.md#Guide/Note",
            "--fingerprint",
            &recorded,
            "--dry",
            "--json",
        ],
    );
    assert_eq!(
        code(&probe),
        1,
        "the stale token refuses: {}",
        stderr(&probe)
    );
    let envelope: Value = serde_json::from_str(&stdout(&probe)).expect("json envelope");
    assert_eq!(
        envelope["error"]["code"], "pin_proof_required",
        "{envelope}"
    );

    let page = read_file(&ws, "plan.md");
    let out = sb.run(&ws, &["pin", "--refresh", "plan.md"]);
    assert_eq!(code(&out), 1, "drift refuses: {}", stdout(&out));
    assert_eq!(
        state_of(&stdout(&out), "guide.md#Guide/Note").as_deref(),
        Some("drifted"),
        "the drifted row is listed: {}",
        stdout(&out)
    );
    let err = stderr(&out);
    assert!(
        err.contains("mrd walk plan.md") && err.contains("mrd pin --refresh plan.md --accept"),
        "the refusal names the loop: {err}"
    );
    assert_eq!(
        read_file(&ws, "plan.md"),
        page,
        "the lock is byte-untouched"
    );
    assert_eq!(read_file(&ws, "guide.md"), moved, "the target is untouched");

    let rehearsal = sb.run(&ws, &["pin", "--refresh", "plan.md", "--accept", "--dry"]);
    assert_eq!(code(&rehearsal), 0, "{}", stderr(&rehearsal));
    assert_eq!(
        state_of(&stdout(&rehearsal), "guide.md#Guide/Note").as_deref(),
        Some("would-accept"),
        "{}",
        stdout(&rehearsal)
    );
    assert_eq!(read_file(&ws, "plan.md"), page, "a dry run writes nothing");

    let accepted = sb.run(&ws, &["pin", "--refresh", "plan.md", "--accept"]);
    assert_eq!(code(&accepted), 0, "accept lands: {}", stderr(&accepted));
    assert_eq!(
        state_of(&stdout(&accepted), "guide.md#Guide/Note").as_deref(),
        Some("accepted"),
        "{}",
        stdout(&accepted)
    );
    assert_ne!(
        rows(&ws)[0].fingerprint,
        recorded,
        "the row now attests the live bytes"
    );
    assert_hashes_are_final(&ws);
    assert_walks_green(&sb, &ws);
}

/// Two rows into one target that both need anchors: pass 1 promotes `^note`, then `^other`,
/// which leaves the first row naming a blob of guide.md that never stands; pass 2 re-pins it,
/// and both rows end on the file's final blob.
#[test]
fn two_rows_promoting_into_one_target_end_on_its_final_blob() {
    let sb = sandbox();
    let ws = sb.pinned_workspace();
    // Strip the anchors the first pins promoted. A marker line is fingerprint-neutral, so both
    // rows still rehearse ok — and each now needs its anchor written again.
    std::fs::write(ws.join("guide.md"), TARGET).expect("strip anchors");

    let out = sb.run(&ws, &["pin", "--refresh", "plan.md", "--json"]);
    assert_eq!(code(&out), 0, "refresh: {}{}", stdout(&out), stderr(&out));
    let report: Value = serde_json::from_str(&stdout(&out)).expect("json report");
    let states: Vec<&str> = report["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| row["state"].as_str().expect("state"))
        .collect();
    assert_eq!(states, ["re-pinned", "ok"], "{report}");
    for row in report["rows"].as_array().expect("rows") {
        assert_eq!(row["promoted"], true, "each row wrote its anchor: {row}");
        assert_eq!(row["target"], "guide.md");
    }
    assert_eq!(report["error"], Value::Null);
    assert_eq!(report["rows"][0]["selector"]["path"][1], "Note");

    let guide = read_file(&ws, "guide.md");
    assert!(
        guide.contains("^note") && guide.contains("^other"),
        "both anchors written: {guide}"
    );
    assert_hashes_are_final(&ws);
    assert_eq!(
        report["rows"][0]["blob"],
        blob_of(&ws, "guide.md"),
        "the report names the blob the lock carries"
    );
    assert_walks_green(&sb, &ws);
}

/// A lock that refuses to parse refuses the refresh at exit 1, page byte-untouched.
#[test]
fn a_lock_that_will_not_parse_refuses_with_nothing_written() {
    let sb = sandbox();
    let ws = sb.workspace();
    let broken = format!("{PAGE}\n```meridian-lock\nversion: 2\ngarbage here\n```\n");
    std::fs::write(ws.join("plan.md"), &broken).expect("broken lock");

    let out = sb.run(&ws, &["pin", "--refresh", "plan.md"]);
    assert_eq!(code(&out), 1, "{}", stderr(&out));
    assert!(
        stderr(&out).contains("refuses to parse"),
        "{}",
        stderr(&out)
    );
    assert_eq!(read_file(&ws, "plan.md"), broken, "nothing was written");
}

/// A row no door re-pins (a `properties` row) refuses the whole refresh at exit 1, naming
/// why, while the re-pinnable row beside it stays as it was.
#[test]
fn a_row_no_door_repins_refuses_the_whole_refresh() {
    let sb = sandbox();
    let ws = sb.workspace();
    let pinned = sb.run(&ws, &["pin", "plan.md", "guide.md#Guide/Note"]);
    assert_eq!(code(&pinned), 0, "{}", stderr(&pinned));
    let page = read_file(&ws, "plan.md");
    let fence = page.rfind("```").expect("the lock's closing fence");
    let with_props = format!(
        "{}  - object: \"[[guide]]\"\n    hash: \"{}\"\n    properties: [\"title\"]\n    \
         fingerprint: \"fp1.props1.b3.00\"\n{}",
        &page[..fence],
        blob_of(&ws, "guide.md"),
        &page[fence..]
    );
    std::fs::write(ws.join("plan.md"), &with_props).expect("hand-authored row");
    assert_eq!(rows(&ws).len(), 2, "the hand-authored row parses");

    let out = sb.run(&ws, &["pin", "--refresh", "plan.md"]);
    assert_eq!(code(&out), 1, "{}", stdout(&out));
    assert!(
        stderr(&out).contains("properties"),
        "the refusal names the row: {}",
        stderr(&out)
    );
    assert_eq!(read_file(&ws, "plan.md"), with_props, "nothing was written");
}
