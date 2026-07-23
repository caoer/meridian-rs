//! End-to-end gates for `mrd test` — the U1.2 tier-1 scenario runner.
//!
//! Drives the REAL `mrd` binary over the committed self-hosting scenario suite
//! (`tests/scenarios/`) and asserts the observable contract: every `^expect`
//! holds, the mount-escape scenarios REFUSE `bad_path`, the CAS-declared write
//! commits while a stale CAS root fires `root_mismatch`, an unknown `t.` attribute
//! fails LOUD, and a firing scenario with no passing sibling is a pairing HARD
//! error. The exit-code convention (0 clean / 1 findings / 2 tool failure) is
//! pinned here too.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// The `mrd` binary under test.
fn mrd() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mrd"))
}

/// A scenario directory under `tests/scenarios/`.
fn scenarios(sub: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scenarios")
        .join(sub)
}

/// Run `mrd test <path> --json`, returning `(exit_code, parsed_report)`.
fn run_json(path: &Path) -> (i32, Value) {
    let out = mrd()
        .arg("test")
        .arg(path)
        .arg("--json")
        .output()
        .expect("run mrd test");
    let code = out.status.code().unwrap_or(-1);
    let report: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "report JSON did not parse ({e}); stdout={}",
            String::from_utf8_lossy(&out.stdout)
        )
    });
    (code, report)
}

/// The report row for scenario `name`.
fn scenario<'a>(report: &'a Value, name: &str) -> &'a Value {
    report["scenarios"]
        .as_array()
        .expect("scenarios array")
        .iter()
        .find(|s| s["name"] == name)
        .unwrap_or_else(|| panic!("scenario `{name}` not in the report"))
}

#[test]
fn self_hosting_suite_is_all_green() {
    let (code, report) = run_json(&scenarios("suite"));
    assert_eq!(
        code, 0,
        "the self-hosting suite must exit 0 (report={report})"
    );
    assert_eq!(report["summary"]["failed"], 0);
    assert_eq!(report["summary"]["malformed"], 0);
    assert_eq!(report["summary"]["pairing_hard_errors"], 0);
    assert_eq!(report["summary"]["passed"], 6);
    // The report records the convention rev tested.
    assert_eq!(
        report["summary"]["convention_revs"],
        serde_json::json!(["seed@1"]),
    );
    for s in report["scenarios"].as_array().unwrap() {
        assert_eq!(s["expect_ok"], true, "every ^expect must hold: {s}");
    }
}

#[test]
fn mount_escape_absolute_path_refuses_bad_path() {
    let (_code, report) = run_json(&scenarios("suite"));
    let s = scenario(&report, "put-escape-abs");
    assert_eq!(s["outcome"], "fired", "an absolute-path put must fire");
    assert_eq!(s["result_code"], "bad_path", "the refusal is bad_path");
    assert_eq!(
        s["expect_ok"], true,
        "its ^expect (asserting the refusal) holds"
    );
}

#[test]
fn mount_escape_dotdot_traversal_refuses_bad_path() {
    let (_code, report) = run_json(&scenarios("suite"));
    let s = scenario(&report, "put-escape-dotdot");
    assert_eq!(s["outcome"], "fired", "a `..` traversal put must fire");
    assert_eq!(s["result_code"], "bad_path", "the refusal is bad_path");
    assert_eq!(s["expect_ok"], true);
}

#[test]
fn cas_declared_write_commits() {
    let (_code, report) = run_json(&scenarios("suite"));
    let s = scenario(&report, "cas-declared");
    assert_eq!(
        s["outcome"], "passed",
        "a guarded write against the live root commits"
    );
    assert_eq!(s["expect_ok"], true);
}

#[test]
fn cas_stale_root_fires_root_mismatch() {
    let (_code, report) = run_json(&scenarios("suite"));
    let s = scenario(&report, "cas-stale-fires");
    assert_eq!(s["outcome"], "fired", "a stale world guard must fire");
    assert_eq!(
        s["result_code"], "root_mismatch",
        "the refusal is root_mismatch"
    );
    assert_eq!(s["expect_ok"], true);
}

#[test]
fn create_scenario_journals_the_birth() {
    let (_code, report) = run_json(&scenarios("suite"));
    let s = scenario(&report, "create-file-pass");
    assert_eq!(s["outcome"], "passed");
    assert_eq!(
        s["expect_ok"], true,
        "t.journal carries the create row, t.doc the bytes"
    );
}

#[test]
fn unknown_expect_attribute_fails_loud() {
    let (code, report) = run_json(&scenarios("unknown-attr"));
    assert_eq!(code, 1, "an ^expect that faults is a findings exit (1)");
    let s = scenario(&report, "unknown-attr");
    assert_eq!(
        s["expect_ok"], false,
        "the unknown-attribute ^expect must FAIL"
    );
    let err = s["expect_error"].as_str().unwrap_or("");
    assert!(
        err.contains("bogus") && err.contains("attribute"),
        "the loud failure names the missing attribute: {err}"
    );
}

#[test]
fn pairing_lint_missing_sibling_is_a_hard_error() {
    let (code, report) = run_json(&scenarios("pairing-missing"));
    assert_eq!(code, 2, "a broken pairing is a tool-failure exit (2)");
    let hard = report["hard_errors"].as_array().expect("hard_errors array");
    assert_eq!(hard.len(), 1, "exactly the one broken pairing");
    let msg = hard[0].as_str().unwrap_or("");
    assert!(
        msg.contains("orphan-fire") && msg.contains("passing sibling"),
        "the hard error names the orphan firing scenario: {msg}"
    );
}
