//! Query rules in `mrd check` and `mrd arm --remove` (`docs/armed-plane.md`
//! § Part A3), driven through the real binary.

use std::path::PathBuf;
use std::process::Output;

use serde_json::Value;

mod common;

fn mrd_bin() -> PathBuf {
    std::env::var_os("MRD_BIN")
        .map_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_mrd")), PathBuf::from)
}

struct Sandbox {
    #[allow(dead_code)]
    tmp: tempfile::TempDir,
    home: PathBuf,
    cache_home: PathBuf,
    ws: PathBuf,
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        common::reap_daemon(&self.home, &self.cache_home);
    }
}

impl Sandbox {
    fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path().join("home");
        let cache_home = tmp.path().join("xdg-cache");
        let ws = tmp.path().join("ws");
        for dir in [&home, &cache_home, &ws] {
            std::fs::create_dir_all(dir).expect("mkdir");
        }
        std::fs::write(
            home.join("MERIDIAN.md"),
            "---\ntype: meridian-config\nversion: 1\n---\n\n# This machine\n",
        )
        .expect("home anchor");
        let s = Sandbox {
            tmp,
            home,
            cache_home,
            ws,
        };
        s.write(
            "MERIDIAN.md",
            "---\ntype: meridian-root\nversion: 1\nname: ws\n---\n\n# The workspace\n",
        );
        s.write("notes/a.md", "---\ntags: [note]\n---\n\n# a\n");
        s.write("other/b.md", "---\ntags: [note]\n---\n\n# b\n");
        s
    }

    fn write(&self, rel: &str, bytes: &str) {
        let path = self.ws.join(rel);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
        std::fs::write(path, bytes).expect("write");
    }

    fn run(&self, args: &[&str]) -> Output {
        let mut child = common::mrd_command(&self.home, &self.cache_home)
            .args(args)
            .current_dir(&self.ws)
            .env_remove("MERIDIAN_CONFIG")
            .env_remove("MERIDIAN_WORKSPACE")
            .env("MERIDIAN_DAEMON_BIN", mrd_bin())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn mrd");
        common::feed_stdin(&mut child, b"");
        child.wait_with_output().expect("wait mrd")
    }

    /// The winner's rev as `mrd rules --json` prints it — what a reviewer reads.
    fn rev(&self, id: &str) -> String {
        let out = self.run(&["rules", ".", "--json"]);
        let json: Value = serde_json::from_slice(&out.stdout).expect("rules --json");
        json["rules"]["rules"]
            .as_array()
            .expect("rows")
            .iter()
            .find(|row| row["id"] == id)
            .and_then(|row| row["chain"][0]["rev"].as_str())
            .unwrap_or_else(|| panic!("{id} not registered: {json}"))
            .to_owned()
    }

    /// Author a rule page at `rules/<id>.md` and arm it in `mode`.
    fn armed(&self, id: &str, tags: &str, extra: &str, body: &str, mode: &str) {
        self.write(
            &format!("rules/{id}.md"),
            &format!("---\ntags: [{tags}]\nid: {id}\n{extra}---\n\n# {id}\n\n{body}"),
        );
        let rev = self.rev(id);
        let out = self.run(&["arm", id, "--mode", mode, "--rev", &rev]);
        assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    }

    fn query(&self, id: &str, sql: &str, mode: &str) {
        self.armed(
            id,
            "rules/query",
            "",
            &format!("```sql\n{sql}\n```\n"),
            mode,
        );
    }
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Every page under `notes/` breaks it.
const NOTES: &str = "SELECT path, 'a note' AS detail FROM doc WHERE path LIKE 'notes/%'";

// ── mrd check ─────────────────────────────────────────────────────────────────

#[test]
fn a_block_query_row_fails_the_check_and_a_warn_row_only_prints() {
    let s = Sandbox::new();
    s.query("no-notes", NOTES, "block");
    let out = s.run(&["check"]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    let said = text(&out);
    assert!(
        said.contains("query `no-notes` (block): 1 row(s)"),
        "{said}"
    );
    assert!(said.contains("notes/a.md — a note"), "{said}");

    let s = Sandbox::new();
    s.query("no-notes", NOTES, "warn");
    let out = s.run(&["check"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    assert!(text(&out).contains("notes/a.md — a note"), "{}", text(&out));
}

#[test]
fn a_control_row_is_never_a_finding_and_its_absence_is_one() {
    let s = Sandbox::new();
    s.armed(
        "no-notes",
        "rules/query",
        "control: notes/a.md\n",
        &format!("```sql\n{NOTES}\n```\n"),
        "block",
    );
    let out = s.run(&["check"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "the control alone: {}",
        text(&out)
    );
    assert!(
        text(&out).contains("control notes/a.md fired"),
        "{}",
        text(&out)
    );

    std::fs::remove_file(s.ws.join("notes/a.md")).expect("rm");
    let out = s.run(&["check"]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    assert!(
        text(&out).contains("did not report its control `notes/a.md`"),
        "{}",
        text(&out)
    );
}

#[test]
fn a_query_that_cannot_run_or_names_no_path_fails_the_check() {
    for (sql, says) in [
        ("SELECT nothing FROM nowhere", "could not run"),
        ("SELECT path AS page FROM doc", "no `path` column"),
    ] {
        let s = Sandbox::new();
        s.query("broken", sql, "warn");
        let out = s.run(&["check"]);
        assert_eq!(out.status.code(), Some(1), "{sql}: {}", text(&out));
        assert!(text(&out).contains(says), "{sql}: {}", text(&out));
    }
}

#[test]
fn paths_limit_query_rows_but_not_a_rule_that_cannot_run() {
    let s = Sandbox::new();
    s.query("no-notes", NOTES, "block");
    assert_eq!(s.run(&["check", "--", "other"]).status.code(), Some(0));
    assert_eq!(s.run(&["check", "--", "other/b.md"]).status.code(), Some(0));
    assert_eq!(s.run(&["check", "--", "notes"]).status.code(), Some(1));
    let absolute = s.ws.join("notes/a.md");
    assert_eq!(
        s.run(&["check", "--", absolute.to_str().unwrap()])
            .status
            .code(),
        Some(1)
    );

    s.query("broken", "SELECT nothing FROM nowhere", "warn");
    let out = s.run(&["check", "--", "other"]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    assert!(text(&out).contains("could not run"), "{}", text(&out));
}

#[test]
fn check_json_carries_each_rule_and_the_armed_rows() {
    let s = Sandbox::new();
    s.armed(
        "no-notes",
        "rules/query",
        "control: notes/a.md\n",
        &format!(
            "```sql\n{}\n```\n",
            NOTES.replace("'notes/%'", "'notes/%' OR path LIKE 'other/%'")
        ),
        "warn",
    );
    let out = s.run(&["check", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    let json: Value = serde_json::from_slice(&out.stdout).expect("check --json");
    let rule = &json["rules"][0];
    assert_eq!(rule["id"], "no-notes");
    assert_eq!(rule["page"], "rules/no-notes.md");
    assert_eq!(rule["mode"], "warn");
    assert_eq!(
        rule["rows"],
        serde_json::json!([{ "path": "other/b.md", "detail": "a note" }])
    );
    assert_eq!(
        rule["control"],
        serde_json::json!({ "path": "notes/a.md", "fired": true })
    );
    assert_eq!(rule["error"], Value::Null);
    assert_eq!(json["armed_rows"]["rows"], 1);
    assert_eq!(json["armed_rows"]["red"], serde_json::json!([]));
}

#[test]
fn a_drifted_armed_row_is_a_check_finding() {
    let s = Sandbox::new();
    s.query("no-notes", NOTES, "warn");
    s.write(
        "rules/no-notes.md",
        &format!(
            "---\ntags: [rules/query]\nid: no-notes\n---\n\n# edited\n\n```sql\n{NOTES}\n```\n"
        ),
    );
    let out = s.run(&["check"]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    assert!(text(&out).contains("edited after arming"), "{}", text(&out));
}

// ── mrd arm --remove ──────────────────────────────────────────────────────────

#[test]
fn remove_deletes_an_off_row_and_refuses_an_armed_absent_or_last_row() {
    let s = Sandbox::new();
    s.query("keep", NOTES, "warn");
    s.query("retire", NOTES, "block");

    let out = s.run(&["arm", "retire", "--remove"]);
    assert_eq!(out.status.code(), Some(1), "an armed row: {}", text(&out));
    assert!(text(&out).contains("--mode off"), "{}", text(&out));

    let rev = s.rev("retire");
    assert!(
        s.run(&["arm", "retire", "--mode", "off", "--rev", &rev])
            .status
            .success()
    );
    std::fs::remove_file(s.ws.join("rules/retire.md")).expect("rm page");
    let out = s.run(&["arm", "retire", "--remove"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    let artifact = std::fs::read_to_string(s.ws.join("meridian/armed-rules.md")).unwrap();
    assert!(
        !artifact.contains("retire") && artifact.contains("keep"),
        "{artifact}"
    );
    assert_eq!(s.run(&["rules", "--workspace"]).status.code(), Some(0));

    let out = s.run(&["arm", "retire", "--remove"]);
    assert_eq!(out.status.code(), Some(1), "an absent row: {}", text(&out));

    let rev = s.rev("keep");
    assert!(
        s.run(&["arm", "keep", "--mode", "off", "--rev", &rev])
            .status
            .success()
    );
    let out = s.run(&["arm", "keep", "--remove"]);
    assert_eq!(out.status.code(), Some(1), "the last row: {}", text(&out));
    assert!(text(&out).contains("last row"), "{}", text(&out));

    let out = s.run(&["arm", "keep", "--remove", "--mode", "off"]);
    assert_eq!(out.status.code(), Some(2), "{}", text(&out));
}
