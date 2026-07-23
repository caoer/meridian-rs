//! End-to-end gates for `mrd check` (U2.10), driving the REAL binary
//! (`CARGO_BIN_EXE_mrd`) over its process boundary. Two ends of the exit triad:
//! a fresh workspace reads GREEN, and a workspace whose receipt journal carries a
//! spliced/forged row reddens with the row cited — the U2.1 spliced-row fixture
//! caught end-to-end through the shipped verb, not just the library.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use fs::WorkspaceRoot;
use fs::domain::RESERVED_JOURNAL_PATH;

fn mrd_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mrd")
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
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// One journal row line in the `render_row` grammar (`parse_rows` reads
/// `root_before=`, `root_after=`, and the trailing `^r-NNNNNN`).
fn row(anchor: &str, root_before: &str, root_after: &str) -> String {
    format!(
        "- op=splice path=a.md root_before={root_before} root_after={root_after} edits=0 ^{anchor}"
    )
}

/// A fresh workspace has no journal — the chain is continuous and there is no
/// out-of-writer edit, so `mrd check` reads GREEN and exits 0.
#[test]
fn check_green_on_fresh_workspace() {
    let sb = sandbox();
    let ws = sb.workspace();

    let out = sb.run(&ws, &["check"]);
    assert!(
        out.status.success(),
        "clean check should exit 0: {}",
        stderr(&out)
    );
    let text = stdout(&out);
    assert!(text.contains("chain: green"), "chain green: {text}");
    assert!(
        text.contains("foreign_edit: none"),
        "no foreign edit: {text}"
    );
}

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
