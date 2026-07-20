//! End-to-end `mrd` binary tests against temp vaults.
//!
//! The vault is the contract's §0.3 wsfix S0 byte-for-byte, so the frozen
//! worked values (R0, `file_rev`, section revs) pin the CLI's output to the
//! contract, not to this test's own expectations.

use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;

/// §0.3 `notes/plan.md` at S0, exact bytes (LF endings, trailing newline).
const PLAN: &str = "---\ntitle: Plan\n---\n# Goals\n\nShip the contract.\n\n## Q3\n\nship by August\n\n## Q4\n\n- item one\n- see [[2026-07-18]]\n- blocked on [[roadmap]]\n";
/// §0.3 `receipts/2026-07-18.md` at S0 (26 B — the `—` is 3-byte UTF-8).
const RECEIPTS: &str = "# Receipts — 2026-07-18\n";
/// The frozen S0 root R0.
const R0: &str = "b3:74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9";

fn s0_vault() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    for (rel, bytes) in [
        ("notes/plan.md", PLAN),
        ("receipts/2026-07-18.md", RECEIPTS),
    ] {
        let abs = dir.path().join(rel);
        std::fs::create_dir_all(abs.parent().expect("parent")).expect("mkdir");
        std::fs::write(&abs, bytes).expect("write fixture");
    }
    dir
}

fn mrd(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mrd"))
        .arg("--root")
        .arg(root)
        .args(args)
        .output()
        .expect("run mrd")
}

fn stdout(out: &Output) -> String {
    String::from_utf8(out.stdout.clone()).expect("stdout UTF-8")
}

fn stderr(out: &Output) -> String {
    String::from_utf8(out.stderr.clone()).expect("stderr UTF-8")
}

// ---------------------------------------------------------------------------
// happy paths: hello / toc / cat
// ---------------------------------------------------------------------------

#[test]
fn hello_reports_server_and_complete_caps() {
    let vault = s0_vault();
    let out = mrd(vault.path(), &["hello"]);
    assert!(out.status.success(), "exit 0: {}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("meridian-sidecar/2.0"), "server name: {text}");
    // caps discovery is COMPLETE (§3.2) — every armed op is announced.
    for cap in sidecar::CAPS {
        assert!(text.contains(cap), "cap {cap} missing: {text}");
    }
}

#[test]
fn toc_human_shows_hpath_rows_and_frozen_revs() {
    let vault = s0_vault();
    let out = mrd(vault.path(), &["toc", "notes/plan.md"]);
    assert!(out.status.success(), "exit 0: {}", stderr(&out));
    let text = stdout(&out);
    assert!(
        text.contains("file_rev=e3c4acaceb75b907"),
        "frozen file_rev: {text}"
    );
    assert!(text.contains(R0), "frozen ambient root: {text}");
    assert!(text.contains("Goals > Q3"), "hpath row: {text}");
    assert!(
        text.contains("rev=33d5b0e1b27cb48b"),
        "frozen Q3 rev: {text}"
    );
}

#[test]
fn cat_section_prints_exact_span_bytes() {
    let vault = s0_vault();
    let out = mrd(
        vault.path(),
        &["cat", "notes/plan.md", "--sec", "Goals", "--sec", "Q3"],
    );
    assert!(out.status.success(), "exit 0: {}", stderr(&out));
    // §4.2 worked value: the full span bytes, heading-inclusive.
    assert_eq!(stdout(&out), "## Q3\n\nship by August\n\n");
}

#[test]
fn cat_whole_file_roundtrips_the_bytes() {
    let vault = s0_vault();
    let out = mrd(vault.path(), &["cat", "notes/plan.md"]);
    assert!(out.status.success());
    assert_eq!(stdout(&out), PLAN);
}

// ---------------------------------------------------------------------------
// --json = the raw wire response frame
// ---------------------------------------------------------------------------

#[test]
fn json_toc_is_one_wire_frame_with_frozen_values() {
    let vault = s0_vault();
    let out = mrd(vault.path(), &["--json", "toc", "notes/plan.md"]);
    assert!(out.status.success());
    let text = stdout(&out);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines.len(),
        1,
        "one-shot toc emits exactly one frame: {text}"
    );
    let frame: Value = serde_json::from_str(lines[0]).expect("frame parses");
    assert_eq!(frame["ok"], Value::Bool(true));
    assert!(frame["id"].is_u64(), "response frame carries the id key");
    assert_eq!(frame["body"]["file_rev"], "e3c4acaceb75b907");
    assert_eq!(frame["body"]["root"], R0);
    assert!(
        frame["body"]["nodes"]
            .as_array()
            .is_some_and(|n| !n.is_empty())
    );
}

#[test]
fn json_error_frame_rides_stdout_with_exit_1() {
    let vault = s0_vault();
    let out = mrd(vault.path(), &["--json", "cat", "missing.md"]);
    assert_eq!(out.status.code(), Some(1), "wire error exits 1");
    let frame: Value = serde_json::from_str(stdout(&out).trim()).expect("frame parses");
    assert_eq!(frame["ok"], Value::Bool(false));
    assert_eq!(frame["error"]["code"], "file_not_found");
    assert_eq!(frame["error"]["recovery"], "env");
}

// ---------------------------------------------------------------------------
// error path + exit codes
// ---------------------------------------------------------------------------

#[test]
fn wire_error_is_human_on_stderr_with_exit_1() {
    let vault = s0_vault();
    let out = mrd(vault.path(), &["cat", "missing.md"]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(err.contains("file_not_found"), "code named: {err}");
    assert!(err.contains("recovery: env"), "recovery class named: {err}");
}

#[test]
fn usage_error_exits_2() {
    let vault = s0_vault();
    let out = mrd(vault.path(), &["no-such-op"]);
    assert_eq!(out.status.code(), Some(2), "clap usage error");
    // malformed --edits JSON is a usage error too, same class
    let out = mrd(
        vault.path(),
        &["splice", "notes/plan.md", "--edits", "not json"],
    );
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("--edits"), "{}", stderr(&out));
}

// ---------------------------------------------------------------------------
// the write path: splice (dry + wet) through one-shot invocations
// ---------------------------------------------------------------------------

const Q3_EDIT: &str = r#"[{"target":{"hpath":[{"h":"Goals"},{"h":"Q3"}]},
  "edit":{"match":{"old":"ship by August","new":"ship by September"}}}]"#;

#[test]
fn splice_dry_leaves_disk_untouched() {
    let vault = s0_vault();
    let out = mrd(
        vault.path(),
        &[
            "--json",
            "splice",
            "notes/plan.md",
            "--dry",
            "--edits",
            Q3_EDIT,
        ],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let frame: Value = serde_json::from_str(stdout(&out).trim()).expect("frame parses");
    assert_eq!(frame["ok"], Value::Bool(true));
    // §4.4 dry law: same response shape, root_after null.
    assert!(frame["body"]["root_after"].is_null(), "{frame}");
    let on_disk = std::fs::read_to_string(vault.path().join("notes/plan.md")).expect("read");
    assert_eq!(on_disk, PLAN, "dry run never touches disk");
}

#[test]
fn splice_then_cat_roundtrips_across_processes() {
    let vault = s0_vault();
    let out = mrd(
        vault.path(),
        &["splice", "notes/plan.md", "--edits", Q3_EDIT],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    // the armed rev transition, frozen §4.4 worked values
    assert!(
        text.contains("33d5b0e1b27cb48b -> 41f643f034e5681f"),
        "{text}"
    );
    // a SECOND process sees the committed write — one-shot persistence proof
    let out = mrd(
        vault.path(),
        &["cat", "notes/plan.md", "--sec", "Goals", "--sec", "Q3"],
    );
    assert!(out.status.success());
    assert_eq!(stdout(&out), "## Q3\n\nship by September\n\n");
}

#[test]
fn splice_edits_from_file_and_unknown_edit_field_refuses_loud() {
    let vault = s0_vault();
    // @FILE form works
    let edits_file = vault.path().join("edits.json");
    std::fs::write(&edits_file, Q3_EDIT).expect("write edits");
    let at = format!("@{}", edits_file.display());
    let out = mrd(
        vault.path(),
        &["splice", "notes/plan.md", "--dry", "--edits", &at],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    // a typo'd edit field reaches the STRICT server and refuses loud —
    // the raw-passthrough law (v2 §3.2), not silent client-side dropping
    let typo = r#"[{"target":{"hpath":[{"h":"Goals"}]},
      "edit":{"match":{"old":"a","new":"b"}},"if_node_rve":"beef"}]"#;
    let out = mrd(
        vault.path(),
        &["splice", "notes/plan.md", "--dry", "--edits", typo],
    );
    assert_eq!(out.status.code(), Some(1), "server-side bad_request");
    assert!(stderr(&out).contains("bad_request"), "{}", stderr(&out));
}

// ---------------------------------------------------------------------------
// the remaining read ops: extract / resolve / links / root / diff / sub
// ---------------------------------------------------------------------------

#[test]
fn extract_lists_nodes_and_unknown_kind_refuses_loud() {
    let vault = s0_vault();
    let out = mrd(
        vault.path(),
        &["extract", "notes/plan.md", "--kinds", "wikilink"],
    );
    assert!(out.status.success());
    assert!(stdout(&out).contains("wikilink"), "{}", stdout(&out));
    // §4.3 D-C5: unknown kinds refuse loud, never silently match nothing
    let out = mrd(
        vault.path(),
        &["extract", "notes/plan.md", "--kinds", "bogus"],
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("unknown_kinds"), "{}", stderr(&out));
}

#[test]
fn resolve_walks_and_dangling_ref_fails_with_stage() {
    let vault = s0_vault();
    let out = mrd(vault.path(), &["resolve", "notes/plan.md", "plan#Goals#Q3"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("notes/plan.md"), "{}", stdout(&out));
    // §4.5: dangling vault-namespace ref → ref_not_found stage 1
    let out = mrd(vault.path(), &["resolve", "notes/plan.md", "roadmap"]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(err.contains("ref_not_found"), "{err}");
    assert!(err.contains("stage"), "{err}");
}

#[test]
fn links_reports_resolved_and_unresolved_edges() {
    let vault = s0_vault();
    let out = mrd(vault.path(), &["links", "notes/plan.md"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(
        text.contains("receipts/2026-07-18.md"),
        "resolved edge: {text}"
    );
    assert!(text.contains("roadmap"), "unresolved edge: {text}");
}

#[test]
fn root_answers_the_frozen_ambient_root() {
    let vault = s0_vault();
    let out = mrd(vault.path(), &["root"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains(R0), "{}", stdout(&out));
}

#[test]
fn diff_identical_roots_replays_nothing() {
    let vault = s0_vault();
    let out = mrd(vault.path(), &["--json", "diff", R0, R0]);
    assert!(out.status.success(), "{}", stderr(&out));
    let frame: Value = serde_json::from_str(stdout(&out).trim()).expect("frame parses");
    assert_eq!(frame["body"]["batches"], serde_json::json!([]));
}

#[test]
fn sub_acks_at_seq_zero_and_refuses_beyond_the_epoch() {
    let vault = s0_vault();
    // one-shot honesty: the fresh epoch acks from_seq 0 with nothing to replay
    let out = mrd(vault.path(), &["sub"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("seq:  0"), "{}", stdout(&out));
    // beyond the epoch's retained history → root_unknown → diff-by-root
    let out = mrd(vault.path(), &["sub", "5"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("root_unknown"), "{}", stderr(&out));
}
