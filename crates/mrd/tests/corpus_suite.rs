//! End-to-end gates for `mrd test --corpus` — the U1.5 tier-2 corpus runner.
//!
//! Drives the REAL `mrd` binary over committed corpus-test specs and the 18-02
//! governed tree (`tests/corpus/tree/` — real 18-02 session task pages, verbatim)
//! and asserts the three signals the pre-arming gate rests on:
//!
//! - **fire-where-expected** — the fixture rule PAGE fires exactly where the
//!   expected-fire manifest declares (owner-self-close fires, reviewer-close /
//!   external-edit / out-of-scope pass); a WRONG `expect` is caught as a mismatch.
//! - **zero dead rules** — a healthy run reports none; a declared citation the
//!   corpus never fires is reported DEAD (over a one-citation page AND over a
//!   two-citation page with a genuinely-present-but-never-fired citation).
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

fn hook_spec(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/hook-tier/corpus/specs")
        .join(format!("{name}.md"))
}

/// Run `mrd test --corpus <spec>` (human report), returning `(exit_code, stdout)`.
fn run_human(name: &str) -> (i32, String) {
    run_human_path(spec(name))
}

fn run_hook_human(name: &str) -> (i32, String) {
    run_human_path(hook_spec(name))
}

fn run_human_path(path: PathBuf) -> (i32, String) {
    let out: Output = mrd()
        .arg("test")
        .arg("--corpus")
        .arg(path)
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
    run_json_path(spec(name))
}

fn run_hook_json(name: &str) -> (i32, Value) {
    run_json_path(hook_spec(name))
}

fn run_json_path(path: PathBuf) -> (i32, Value) {
    let out: Output = mrd()
        .arg("test")
        .arg("--corpus")
        .arg(path)
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
    assert_eq!(report["rule"], "reviewer-not-owner");
    assert_eq!(report["rule_source"], "../rules/reviewer-not-owner.md");
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
    assert_eq!(fire["fired"][0], "reviewer-close");
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
fn dead_citation_over_a_one_citation_page_is_reported() {
    // The literal plan Test: a corpus run over a one-citation law where that
    // declared citation never fires — every case still matches (all pass), yet the
    // dead rule is reported and the run exits 1.
    let (code, report) = run_json("dead-rule");
    assert_eq!(code, 1, "a dead rule is a findings exit (1): {report}");
    assert_eq!(report["rule"], "reviewer-not-owner");
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
        serde_json::json!(["check:reviewer-close"]),
        "the declared citation the corpus never fired is reported dead, in ITS namespace"
    );

    // The dead-rule report is SHOWN in the human render (task gate).
    let (hcode, human) = run_human("dead-rule");
    assert_eq!(hcode, 1);
    assert!(
        human.contains("Dead rules (declared, never fired)")
            && human.contains("check:reviewer-close"),
        "the human report shows the dead rule:\n{human}"
    );
}

#[test]
fn dead_citation_on_a_two_citation_page_is_reported() {
    // ONE page carrying TWO citations: the LIVE one fires where expected, the
    // present-but-never-fired priority citation is reported dead (the @2 twin of
    // the effect kernel's dead_priority replay rule). Liveness is per CITATION,
    // which is why one page can be half dead — the identity a report row is keyed
    // on is the citation, not the page.
    let (code, report) = run_json("dead-priority");
    assert_eq!(
        code, 1,
        "the dead priority citation is a findings exit (1): {report}"
    );
    assert_eq!(report["rule"], "reviewer-and-priority");
    assert_eq!(
        report["rule_source"], "../rules/reviewer-and-priority.md",
        "the report names the PAGE the law was loaded from"
    );
    assert_eq!(
        report["summary"]["mismatches"], 0,
        "the live citation fires where expected"
    );
    // The live citation fired; only the priority one is dead.
    assert_eq!(
        case(&report, "r3a-self-close")["fired"][0],
        "reviewer-close"
    );
    assert_eq!(
        report["dead_rules"],
        serde_json::json!(["check:lower-priority"]),
        "only the never-fired priority citation is dead; the reviewer one is live"
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
    // A citation the rule emitted that the manifest never declared is a surprise
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
        serde_json::json!(["check:reviewer-close"]),
        "the fired-but-undeclared citation is reported as a surprise"
    );
}

#[test]
fn proto_send_hook_reports_fire_dead_fuel_and_explicit_quiescence() {
    let (code, report) = run_hook_json("valid-hook");
    assert_eq!(code, 0, "the valid HOOK corpus must pass: {report}");
    assert_eq!(report["summary"]["mismatches"], 0);
    assert_eq!(report["summary"]["dead_rules"], 0);
    assert_eq!(report["summary"]["errors"], 0);
    assert_eq!(report["budgets"]["evals"], 2);
    assert_eq!(report["quiescence"]["passed"], true);
    assert_eq!(report["quiescence"]["verdict"], "acyclic");
    assert_eq!(
        report["quiescence"]["nodes"],
        serde_json::json!(["task-status-notify"])
    );
    assert_eq!(report["quiescence"]["edges"], serde_json::json!([]));
    assert_eq!(
        case(&report, "move-to-review")["fired"],
        serde_json::json!(["task-status-notify"])
    );
    assert_eq!(
        policy::SLICE1_CAPS,
        [effects::EffectKind::Send],
        "the proof did not widen the production cap ceiling"
    );
}

#[test]
fn hook_with_no_matching_case_is_dead() {
    let (code, report) = run_hook_json("dead-hook");
    assert_eq!(code, 1, "a dead HOOK is a finding: {report}");
    assert_eq!(report["summary"]["mismatches"], 0);
    assert_eq!(
        report["dead_rules"],
        serde_json::json!(["hook:task-status-notify"])
    );
    assert_eq!(report["quiescence"]["verdict"], "acyclic");
}

#[test]
fn hook_fuel_breach_is_named_and_fails() {
    let (code, report) = run_hook_json("fuel-breach");
    assert_eq!(code, 1, "a HOOK fuel breach is a finding: {report}");
    let row = case(&report, "matching-change-exhausts-budget");
    assert_eq!(row["outcome"], "error");
    let findings = row["budget_findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1);
    assert!(
        findings[0]
            .as_str()
            .unwrap_or_default()
            .contains("tiny-budget: budget_exceeded steps=1"),
        "the finding names the HOOK and its declared step ceiling: {findings:?}"
    );
}

/// The trigger graph the cyclic pair really has. Both rules react to the same field
/// and both write it, so each rule re-triggers itself as well as its peer. The
/// self-edges are real trigger edges — their work is idempotent and terminates, but
/// the edge exists and the proof does not hide it.
fn cyclic_edges() -> Value {
    serde_json::json!([
        {"from":"cycle-alpha","to":"cycle-alpha"},
        {"from":"cycle-alpha","to":"cycle-beta"},
        {"from":"cycle-beta","to":"cycle-alpha"},
        {"from":"cycle-beta","to":"cycle-beta"}
    ])
}

#[test]
fn deliberately_cyclic_hooks_fail_only_quiescence() {
    let (code, report) = run_hook_json("cyclic-hooks");
    assert_eq!(code, 1, "the cyclic fixture must fail: {report}");
    assert_eq!(report["summary"]["mismatches"], 0);
    assert_eq!(report["summary"]["dead_rules"], 0);
    assert_eq!(report["summary"]["surprise_rules"], 0);
    assert_eq!(report["summary"]["errors"], 0);
    assert_eq!(report["summary"]["findings"], 1);
    assert_eq!(report["quiescence"]["passed"], false);
    assert_eq!(report["quiescence"]["verdict"], "cycle");
    assert_eq!(
        report["quiescence"]["cycle"],
        serde_json::json!(["cycle-beta", "cycle-alpha", "cycle-beta"])
    );
    assert_eq!(report["quiescence"]["edges"], cyclic_edges());

    let (human_code, human) = run_hook_human("cyclic-hooks");
    assert_eq!(human_code, 1);
    assert!(
        human.contains("quiescence assertion failed: cycle-beta → cycle-alpha → cycle-beta"),
        "the negative receipt fails specifically at quiescence:\n{human}"
    );
}

#[test]
fn two_seeded_lineages_still_prove_the_cycle() {
    // The gate-2 reproducer: two initial cases seed DISTINCT lineages into the same
    // cyclic pair. A global work cache retires one lineage's item before the other
    // lineage's descendant can close its ancestry, and the run exits 0 claiming
    // `acyclic` with both edges present. Recurrence is per lineage, so it cannot.
    let (code, report) = run_hook_json("two-entry-cycle");
    assert_eq!(code, 1, "a reachable cycle must fail quiescence: {report}");
    assert_eq!(report["summary"]["cases"], 2);
    assert_eq!(report["summary"]["matched"], 2);
    assert_eq!(report["quiescence"]["passed"], false);
    assert_eq!(report["quiescence"]["verdict"], "cycle");
    assert_eq!(
        report["quiescence"]["cycle"],
        serde_json::json!(["cycle-beta", "cycle-alpha", "cycle-beta"])
    );
    assert_eq!(
        report["quiescence"]["edges"],
        cyclic_edges(),
        "both cross edges are present AND the cycle is reported"
    );
}

#[test]
fn converging_acyclic_branches_are_not_a_cycle() {
    // The opposite false-positive class: two peers emit the IDENTICAL generation from
    // the IDENTICAL state, so their branches converge on a byte-identical downstream
    // work item carried by two different lineages. No lineage returns to one of its
    // own ancestors, so the verdict is acyclic.
    let (code, report) = run_hook_json("acyclic-convergence");
    assert_eq!(code, 0, "converging branches are not a cycle: {report}");
    assert_eq!(report["quiescence"]["verdict"], "acyclic");
    assert_eq!(report["quiescence"]["cycle"], Value::Null);
    assert_eq!(
        report["quiescence"]["edges"],
        serde_json::json!([
            {"from":"fan-a","to":"converge-sink"},
            {"from":"fan-b","to":"converge-sink"}
        ]),
        "both branches really did reach the convergence point"
    );
    assert_eq!(report["summary"]["dead_rules"], 0);
}

#[test]
fn a_terminating_convergent_cascade_is_not_a_fuel_exhaustion() {
    // N1. Two peers per level emit the IDENTICAL generation, so the graph's branches
    // reconverge at every level: `2^(d+1) − 2` causal PATHS over `d` states. A proof
    // that enumerates paths pays the first number and reports `fuel_exhausted` on a
    // convention set with no cycle at all — measured at depth 8, where 510 paths breach
    // the 256 budget. A pre-arming gate that fails good conventions blocks arming.
    //
    // Both depths must certify, and the step counts must stay proportional to the
    // STATE space (2 per level), never to the path count.
    for (spec, steps) in [
        ("convergent-cascade-depth-8", 16),
        ("convergent-cascade-depth-12", 24),
    ] {
        let (code, report) = run_hook_json(spec);
        assert_eq!(code, 0, "{spec}: a terminating cascade certifies: {report}");
        assert_eq!(report["quiescence"]["verdict"], "acyclic", "{spec}");
        assert_eq!(report["quiescence"]["fuel_exhausted"], false, "{spec}");
        assert_eq!(report["quiescence"]["cycle"], Value::Null, "{spec}");
        assert_eq!(
            report["quiescence"]["steps"], steps,
            "{spec}: the walk costs the state space, not the path count: {report}"
        );
        // Not vacuous: the cascade really ran, and every peer edge is in the graph.
        assert_eq!(
            report["quiescence"]["edges"],
            serde_json::json!([
                {"from":"chain-a","to":"chain-a"},
                {"from":"chain-a","to":"chain-b"},
                {"from":"chain-b","to":"chain-a"},
                {"from":"chain-b","to":"chain-b"}
            ]),
            "{spec}: both peers really did re-trigger each other"
        );
        assert_eq!(report["summary"]["dead_rules"], 0, "{spec}");
    }
}

#[test]
fn a_divergent_cascade_still_exhausts_fuel() {
    // The direction-of-failure rail on the N1 repair (advisor requirement). The defect
    // it repairs false-FAILS good conventions, which is the SAFE direction for a
    // pre-arming gate; the repair must never buy that back by flipping the gate toward
    // false-ACCEPT.
    //
    // Here the two peers append DIFFERENT lines to one section, so no two causal paths
    // ever meet: the state space really is exponential and the cascade never settles.
    // Nothing recurs, so no ancestry check can fire — the graph fuel is the only thing
    // that stops it, and a completed-work memo must leave it stopping.
    let (code, report) = run_hook_json("divergent-cascade");
    assert_eq!(code, 1, "a never-settling cascade must fail: {report}");
    assert_eq!(report["quiescence"]["verdict"], "fuel_exhausted");
    assert_eq!(report["quiescence"]["passed"], false);
    assert_eq!(
        report["quiescence"]["cycle"],
        Value::Null,
        "divergence is not a cycle — there is no recurrence to report"
    );
    assert_eq!(
        report["quiescence"]["steps"], report["quiescence"]["fuel_limit"],
        "the budget is what stopped the walk: {report}"
    );
}

#[test]
fn a_live_hook_cannot_vouch_for_a_same_named_check_citation() {
    // The MIRROR of `a_check_citation_cannot_vouch_for_a_same_named_hook`, over the
    // same two pages and the same collided name, running the other way: here the HOOK
    // fires and the CHECK citation of that name never does. One merged `declared_rules`
    // list with a hook-wins rule closes only the named direction; two typed namespaces
    // close both.
    let (code, report) = run_hook_json("hook-liveness-mirror");
    assert_eq!(
        code, 1,
        "the same-named dead citation is a finding: {report}"
    );
    assert_eq!(
        report["declared_rules"],
        serde_json::json!(["check:task-status-notify", "hook:task-status-notify"]),
        "one collided name is TWO liveness subjects"
    );
    assert_eq!(
        report["dead_rules"],
        serde_json::json!(["check:task-status-notify"]),
        "the citation is dead in its own namespace, and says which namespace"
    );
    // The live HOOK is unaffected — it fired, and it is not reported dead.
    assert_eq!(
        case(&report, "mirror")["fired"],
        serde_json::json!(["task-status-notify"])
    );
    assert_eq!(case(&report, "mirror")["matched"], true);
    assert_eq!(report["surprise_rules"], serde_json::json!([]));
}

#[test]
fn a_later_silent_hook_is_dead_even_beside_a_live_one() {
    // Two loaded HOOKs, an EMPTY `rules` fence: the live one fires, and the later,
    // silent one is still a liveness subject.
    let (code, report) = run_hook_json("dead-hook-multi-omitted-from-rules");
    assert_eq!(code, 1, "the silent HOOK is a finding: {report}");
    assert_eq!(
        report["declared_rules"],
        serde_json::json!(["hook:task-status-notify", "hook:never-fires"]),
        "liveness subjects come from the loaded rule set, not the fence"
    );
    assert_eq!(
        report["dead_rules"],
        serde_json::json!(["hook:never-fires"])
    );
    assert_eq!(report["summary"]["mismatches"], 0);
    assert_eq!(
        case(&report, "move-to-review")["fired"],
        serde_json::json!(["task-status-notify"]),
        "the live sibling is unaffected"
    );
}

#[test]
fn a_check_citation_cannot_vouch_for_a_same_named_hook() {
    // The CHECK cites `task-status-notify`, which is also the sibling HOOK's slug.
    // One merged set makes the silent HOOK look live; two namespaces cannot.
    let (code, report) = run_hook_json("citation-collision");
    assert_eq!(code, 1, "the same-named silent HOOK is dead: {report}");
    let row = case(&report, "collide");
    assert_eq!(row["outcome"], "fired");
    assert_eq!(
        row["fired"],
        serde_json::json!(["task-status-notify"]),
        "the CHECK expectation still matches on the citation id"
    );
    assert_eq!(row["matched"], true);
    assert_eq!(
        report["dead_rules"],
        serde_json::json!(["hook:task-status-notify"]),
        "and the silent HOOK of the same name is still reported dead"
    );
    assert_eq!(
        report["surprise_rules"],
        serde_json::json!([]),
        "a declared name is never also a surprise"
    );
}

#[test]
fn one_generation_is_one_batch_that_names_no_identity() {
    // Production applies a mixed frontmatter+section emission as ONE atomic batch,
    // and such a batch has no addressable Delta container — the one synthesized event
    // names neither the field nor the section. A proof that split the generation into
    // independent single-edit writes would derive two rich events and fabricate the
    // downstream fire this watcher is waiting for.
    let (code, report) = run_hook_json("mixed-generation");
    assert_eq!(code, 1, "the watcher must go dead: {report}");
    assert_eq!(
        case(&report, "pull-the-trigger")["fired"],
        serde_json::json!(["mixed-emitter"])
    );
    assert_eq!(
        report["dead_rules"],
        serde_json::json!(["hook:mixed-watcher"])
    );
    assert_eq!(
        report["quiescence"]["edges"],
        serde_json::json!([]),
        "one batch, one event, no invented downstream edge"
    );
    assert_eq!(report["quiescence"]["verdict"], "acyclic");
}

#[test]
fn narrowed_reporting_is_byte_identical_in_both_corpus_modes() {
    // `counterfactual: true` widens which caps a DECLARATION may carry. It must not
    // change how a result is projected or reported by one byte, so the same denied
    // ordinary descriptor renders identically in both modes.
    let (production_code, production) = run_hook_json("narrowed-production");
    let (counterfactual_code, counterfactual) = run_hook_json("narrowed-counterfactual");
    assert_eq!(production_code, 0, "{production}");
    assert_eq!(counterfactual_code, 0, "{counterfactual}");
    let denied = serde_json::json!(["narrowed-md:md.set_field"]);
    assert_eq!(
        case(&production, "move-to-review")["narrowed_effects"],
        denied
    );
    assert_eq!(
        case(&production, "move-to-review"),
        case(&counterfactual, "move-to-review"),
        "every reported case fact is identical across the two modes"
    );
}

#[test]
fn a_forged_receipt_is_refused_in_both_corpus_modes() {
    // The ORDINARY production corpus branch is the one an armed HOOK will travel, so
    // it is the one that must carry the guard — and the counterfactual twin proves
    // widening the cap ceiling did not weaken it.
    for spec in ["forged-receipt", "forged-receipt-counterfactual"] {
        let out: Output = mrd()
            .arg("test")
            .arg("--corpus")
            .arg(hook_spec(spec))
            .output()
            .expect("run the forged-receipt control");
        assert_eq!(
            out.status.code(),
            Some(2),
            "{spec}: a forged receipt is a hard fault"
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("emitted a malformed intent")
                && stderr.contains("does not name this landed change"),
            "{spec}: the policy boundary names the fault: {stderr}"
        );
    }
}

#[test]
fn raw_md_descriptors_cannot_bypass_canonical_validation() {
    // R13's negative half: a direct `set_field` constructor carries no canonical
    // action and no receipt. Production HOOK projection rejects it, and the
    // counterfactual branch now rejects it identically — the proof validates the same
    // production-shaped intent a real armed hook would, or it proves nothing.
    let out: Output = mrd()
        .arg("test")
        .arg("--corpus")
        .arg(hook_spec("raw-md-descriptor"))
        .output()
        .expect("run the raw-md control");
    assert_eq!(
        out.status.code(),
        Some(2),
        "a raw md.* descriptor is a hard fault"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("raw-md") && stderr.contains("malformed intent"),
        "the fault names the rule and the canonical shape: {stderr}"
    );
}

#[test]
fn a_canonical_md_intent_reaches_the_production_executor() {
    // R13's positive half: a canonical `md.append_section` intent crosses the `run`
    // adapter into the production batch executor. The watcher fires on the EXECUTOR's
    // own synthesized event, which it can only do if the append really landed against
    // the isolated proof corpus with production section semantics.
    let (code, report) = run_hook_json("canonical-md-adapter");
    assert_eq!(
        code, 0,
        "the canonical adapter path must be clean: {report}"
    );
    assert_eq!(
        report["quiescence"]["edges"],
        serde_json::json!([{"from":"canonical-writer","to":"log-watcher"}]),
        "the executor's synthesized event named the section the adapter wrote"
    );
    assert_eq!(report["quiescence"]["verdict"], "acyclic");
    assert_eq!(report["summary"]["dead_rules"], 0);
    assert_eq!(
        policy::SLICE1_CAPS,
        [effects::EffectKind::Send],
        "the adapter is production code; the runtime cap ceiling is unchanged"
    );
}

#[test]
fn loaded_hook_omitted_from_rules_is_still_dead() {
    let (code, report) = run_hook_json("dead-hook-omitted-from-rules");
    assert_eq!(
        code, 1,
        "a loaded dead HOOK cannot hide outside `rules`: {report}"
    );
    assert_eq!(
        report["declared_rules"],
        serde_json::json!(["hook:task-status-notify"])
    );
    assert_eq!(
        report["dead_rules"],
        serde_json::json!(["hook:task-status-notify"])
    );
    assert_eq!(report["summary"]["mismatches"], 0);
}

#[test]
fn duplicate_identical_cases_are_acyclic_but_the_real_cycle_still_bites() {
    let (code, report) = run_hook_json("duplicate-identical-cases");
    assert_eq!(
        code, 0,
        "global duplicate work is not a causal cycle: {report}"
    );
    assert_eq!(report["summary"]["cases"], 2);
    assert_eq!(report["summary"]["matched"], 2);
    assert_eq!(report["quiescence"]["verdict"], "acyclic");
    assert_eq!(report["quiescence"]["cycle"], Value::Null);

    let (cycle_code, cycle) = run_hook_json("cyclic-hooks");
    assert_eq!(cycle_code, 1);
    assert_eq!(cycle["quiescence"]["verdict"], "cycle");
    assert_eq!(
        cycle["quiescence"]["cycle"],
        serde_json::json!(["cycle-beta", "cycle-alpha", "cycle-beta"])
    );
}

#[test]
fn a_case_can_drive_a_content_reading_check() {
    // The tier's CONTENT reach, and the reason this gate exists: a CHECK reading
    // `change.sections_changed` / `change.doc.nodes[].text` is a rule a markdown
    // engine must be able to test, and until a case could write the BODY it was
    // unreachable by construction — reported dead over every case form the grammar
    // had. The case's change half is the PRODUCTION edit grammar (the ratified
    // tier-1 split: JSON for the change, in the exact op format production uses),
    // so an `hpath` target and a `put at:end` reach the body the same way a real
    // write does.
    let (code, report) = run_json("body-content");
    assert_eq!(
        code, 0,
        "a content case that fires where declared is a clean run: {report}"
    );
    assert_eq!(report["rule"], "no-smuggled-heading");
    assert_eq!(report["summary"]["cases"], 2);
    assert_eq!(report["summary"]["matched"], 2);
    assert_eq!(report["summary"]["mismatches"], 0);
    assert_eq!(
        report["summary"]["dead_rules"], 0,
        "the content citation is LIVE — the whole point"
    );

    let fired = case(&report, "smuggle-a-heading");
    assert_eq!(fired["outcome"], "fired");
    assert_eq!(
        fired["fired"],
        serde_json::json!(["structural-write"]),
        "the body write moved the section diff and the node kinds the CHECK compares"
    );
    assert_eq!(
        case(&report, "plain-section-append")["outcome"],
        "pass",
        "the same body write without a TODO passes — the rule discriminates on CONTENT"
    );
}

#[test]
fn an_unknown_case_key_is_a_tool_failure() {
    // The silence that hid the content gap: an author reaching for a verb the
    // grammar does not carry used to get a green-looking run over a case that
    // mutated nothing. An unknown key is now the authoring fault it always was.
    let out = mrd()
        .arg("test")
        .arg("--corpus")
        .arg(spec("unknown-case-key"))
        .output()
        .expect("run mrd test --corpus");
    assert_eq!(
        out.status.code(),
        Some(2),
        "an unknown case key is exit 2, never a silently-dropped clause"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("append_section"),
        "the tool failure names the key the author wrote: {stderr}"
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
