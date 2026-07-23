//! End-to-end gates for `mrd test --corpus` — the U1.5 tier-2 corpus runner.
//!
//! Drives the REAL `mrd` binary over committed corpus-test specs and the 18-02
//! governed tree (`tests/corpus/tree/` — real 18-02 session task pages, verbatim)
//! and asserts the three signals the pre-arming gate rests on:
//!
//! - **fire-where-expected** — the seed convention fires exactly where the
//!   expected-fire manifest declares (owner-self-close fires, reviewer-close /
//!   external-edit / out-of-scope pass); a WRONG `expect` is caught as a mismatch.
//! - **zero dead rules** — a healthy run reports none; a declared rule the corpus
//!   never fires is reported DEAD (over the seed convention AND over a two-rule
//!   folder convention with a genuinely-present-but-never-fired rule).
//! - **fuel + heap budgets** — every run reports p50/p99/max over its in-scope
//!   evals.
//!
//! The exit-code convention (0 clean / 1 findings / 2 tool failure) is pinned too.

use std::path::PathBuf;
use std::process::{Command, Output};

use serde_json::Value;

/// The `mrd` binary under test.
fn mrd() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mrd"))
}

/// The committed corpus-test spec `<name>.md` under `tests/corpus/specs/`.
fn spec(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/corpus/specs")
        .join(format!("{name}.md"))
}

/// Run `mrd test --corpus <spec>` (human report), returning `(exit_code, stdout)`.
fn run_human(name: &str) -> (i32, String) {
    let out: Output = mrd()
        .arg("test")
        .arg("--corpus")
        .arg(spec(name))
        .output()
        .expect("run mrd test --corpus");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// Run `mrd test --corpus <spec> --json`, returning `(exit_code, parsed_report)`.
/// A malformed spec emits no JSON, so this is for the specs that produce a report.
fn run_json(name: &str) -> (i32, Value) {
    let out: Output = mrd()
        .arg("test")
        .arg("--corpus")
        .arg(spec(name))
        .arg("--json")
        .output()
        .expect("run mrd test --corpus --json");
    let code = out.status.code().unwrap_or(-1);
    let report: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "report JSON did not parse ({e}); stdout={}",
            String::from_utf8_lossy(&out.stdout)
        )
    });
    (code, report)
}

/// The report row for case `name`.
fn case<'a>(report: &'a Value, name: &str) -> &'a Value {
    report["cases"]
        .as_array()
        .expect("cases array")
        .iter()
        .find(|c| c["name"] == name)
        .unwrap_or_else(|| panic!("case `{name}` not in the report"))
}

#[test]
fn fire_where_expected_matches_and_exits_zero() {
    let (code, report) = run_json("fire-where-expected");
    assert_eq!(
        code, 0,
        "a matched manifest with no dead rule exits 0: {report}"
    );
    assert_eq!(report["convention"], "reviewer-not-owner");
    assert_eq!(report["convention_source"], "seed");
    let summary = &report["summary"];
    assert_eq!(summary["cases"], 7);
    assert_eq!(summary["matched"], 7, "every case matched its expect");
    assert_eq!(summary["mismatches"], 0);
    assert_eq!(
        summary["dead_rules"], 0,
        "the rule fired, so it is not dead"
    );
    assert_eq!(summary["findings"], 0);
    assert_eq!(report["dead_rules"].as_array().unwrap().len(), 0);

    // The owner-self-close FIRES the reviewer-not-owner rule; the reviewer-close
    // and the external edit PASS.
    let fire = case(&report, "r3a-self-close");
    assert_eq!(fire["outcome"], "fired");
    assert_eq!(fire["fired"][0], "scenarios/reviewer-close.md");
    assert_eq!(fire["matched"], true);
    assert_eq!(case(&report, "r3a-reviewer-close")["outcome"], "pass");
    assert_eq!(case(&report, "gatecheck-external-edit")["outcome"], "pass");

    // Scope gating is observable: an out-of-scope doc is never run.
    let oos = case(&report, "decision-out-of-scope");
    assert_eq!(oos["in_scope"], false, "decisions/ is outside tasks/**");
    assert_eq!(oos["outcome"], "pass");

    // Budgets: p50/p99/max over the 6 in-scope evals are present and ordered.
    let budgets = &report["budgets"];
    assert_eq!(
        budgets["evals"], 6,
        "6 in-scope evals (the out-of-scope doc is skipped)"
    );
    for metric in ["fuel", "heap"] {
        let m = &budgets[metric];
        let (p50, p99, max) = (
            m["p50"].as_u64().unwrap(),
            m["p99"].as_u64().unwrap(),
            m["max"].as_u64().unwrap(),
        );
        assert!(p50 <= p99 && p99 <= max, "{metric} p50<=p99<=max: {m}");
    }
}

#[test]
fn dead_rule_over_the_seed_convention_is_reported() {
    // The literal plan Test: a corpus run over the seed convention where the one
    // declared rule never fires — every case still matches (all pass), yet the
    // dead rule is reported and the run exits 1.
    let (code, report) = run_json("dead-rule");
    assert_eq!(code, 1, "a dead rule is a findings exit (1): {report}");
    assert_eq!(report["convention"], "reviewer-not-owner");
    assert_eq!(
        report["summary"]["mismatches"], 0,
        "fire-where-expected still holds"
    );
    assert_eq!(
        report["summary"]["matched"], 4,
        "every case matched (all pass)"
    );
    assert_eq!(
        report["dead_rules"],
        serde_json::json!(["scenarios/reviewer-close.md"]),
        "the declared rule the corpus never fired is reported dead"
    );

    // The dead-rule report is SHOWN in the human render (task gate).
    let (hcode, human) = run_human("dead-rule");
    assert_eq!(hcode, 1);
    assert!(
        human.contains("Dead rules (declared, never fired)")
            && human.contains("scenarios/reviewer-close.md"),
        "the human report shows the dead rule:\n{human}"
    );
}

#[test]
fn dead_rule_in_a_folder_convention_is_reported() {
    // A two-rule convention loaded from a folder on disk: the LIVE rule fires
    // where expected, the present-but-never-fired priority rule is reported dead
    // (the @2 twin of the effect kernel's dead_priority replay rule).
    let (code, report) = run_json("dead-priority");
    assert_eq!(
        code, 1,
        "the dead priority rule is a findings exit (1): {report}"
    );
    assert_eq!(report["convention"], "reviewer-and-priority");
    assert_ne!(
        report["convention_source"], "seed",
        "loaded from a folder, not the embedded seed"
    );
    assert_eq!(
        report["summary"]["mismatches"], 0,
        "the live rule fires where expected"
    );
    // The live rule fired; only the priority rule is dead.
    assert_eq!(
        case(&report, "r3a-self-close")["fired"][0],
        "scenarios/reviewer-close.md"
    );
    assert_eq!(
        report["dead_rules"],
        serde_json::json!(["scenarios/lower-priority.md"]),
        "only the never-fired priority rule is dead; the reviewer rule is live"
    );
}

#[test]
fn fire_mismatch_is_caught_and_exits_one() {
    // The load-bearing negative: a case declaring `expect: pass` over a change
    // that actually FIRES is reported as a mismatch — fire-where-expected is
    // enforced, not vacuous.
    let (code, report) = run_json("fire-mismatch");
    assert_eq!(code, 1, "a fire mismatch is a findings exit (1): {report}");
    assert_eq!(report["summary"]["mismatches"], 1);
    assert_eq!(
        report["summary"]["dead_rules"], 0,
        "the rule fires, so it is not dead"
    );
    let wrong = case(&report, "wrong-expect-pass");
    assert_eq!(wrong["expect"], "pass");
    assert_eq!(wrong["outcome"], "fired", "it actually fired");
    assert_eq!(
        wrong["matched"], false,
        "observed disagrees with the manifest"
    );
    // The correctly-declared sibling still matches.
    assert_eq!(case(&report, "correct-self-close")["matched"], true);
}

#[test]
fn surprise_rule_fired_but_undeclared_is_reported() {
    // A rule the convention fired that the manifest never declared is a surprise
    // finding — an under-declared expected-fire manifest cannot pass silently.
    let (code, report) = run_json("surprise-rule");
    assert_eq!(code, 1, "a surprise rule is a findings exit (1): {report}");
    assert_eq!(
        report["summary"]["mismatches"], 0,
        "every case matched its expect"
    );
    assert_eq!(
        report["summary"]["dead_rules"], 0,
        "nothing was declared, so nothing is dead"
    );
    assert_eq!(
        report["surprise_rules"],
        serde_json::json!(["scenarios/reviewer-close.md"]),
        "the fired-but-undeclared rule is reported as a surprise"
    );
}

#[test]
fn malformed_spec_is_a_tool_failure() {
    // A ```case block whose JSON will not parse is exit 2 (tool failure), not a
    // findings run — it emits no report on stdout.
    let out = mrd()
        .arg("test")
        .arg("--corpus")
        .arg(spec("malformed"))
        .output()
        .expect("run mrd test --corpus");
    assert_eq!(out.status.code(), Some(2), "a malformed spec is exit 2");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("case") && stderr.contains("did not parse"),
        "the tool failure names the malformed case: {stderr}"
    );
}
