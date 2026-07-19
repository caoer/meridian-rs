//! Pack loading internals for `compile` (§11.3): manifest parse, fixtures, the
//! sealed Starlark evaluation bridge, the injected `rulepack-api@1` fact API, and
//! ruleset budget-class classification.
//!
//! Everything here is `pub(crate)` — the public surface is `compile` +
//! `CompiledRuleset` in `lib.rs`. The evaluation bridge (`RuleEvaluator`) is the
//! sealed seam: `StarlarkEvaluator` (ruling 008) runs fenced Starlark predicates
//! over injected world-model facts, metered under the manifest's `EvalBudget`.
//! P6-EVAL feeds the same [`FactDoc`] surface from real `model` ASTs — without
//! reshaping it — and neither the public `compile` signature nor `CompiledRuleset`
//! changes.
//!
//! # `rulepack-api@1` — the injected fact API (the pin's meaning)
//! A rule page is literate markdown whose predicate is a fenced ` ```starlark `
//! block defining `def check(doc)`. The engine calls it once per document,
//! injecting exactly the §11.2 world-model fact surface — nodes, revs, spans,
//! links, hpaths — and nothing else:
//!
//! - `doc.path` (str) — the document path.
//! - `doc.nodes` (list) — world-model nodes in document order. Each node exposes
//!   `kind` (str: `"heading"` / `"paragraph"` / …), `level` (int; heading level or
//!   `0`), `text` (str), `span` (int tuple `(start, end)`), `node_rev` (str),
//!   `hpath` (list[str]).
//! - `violation(rule, severity, span, node_rev, hpath, message)` (builtin, all
//!   named) — records one §11.1 finding; `severity` ∈ {`error`, `warn`, `info`}.
//!
//! Changing this surface or the dialect is an evaluator change ⇒ a
//! `rulepack-api@N` bump, gated at load — never a wire amendment (row-13
//! wire-invariance: no wire crate names Starlark).

use std::cell::RefCell;

use model::NodeRev;
use starlark::environment::{Globals, GlobalsBuilder, Module};
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::syntax::{AstModule, Dialect};
use starlark::values::Heap;
use starlark::values::Value;
use starlark::values::list::UnpackList;
use starlark::values::none::NoneType;
use starlark::values::structs::AllocStruct;

use crate::{BudgetClass, CompileError, EvalBudget, Severity, Violation};

/// The §11.3 pack manifest — generic and evaluator-free. `deny_unknown_fields`
/// makes a typo (or a field from a newer `rulepack-api@N`) a loud compile
/// failure rather than a silent drop; api evolution rides the `api` pin, not
/// lenient parsing.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    pub id: String,
    pub api: String,
    pub budgets: EvalBudget,
    pub fixtures: Vec<String>,
    pub rules: Vec<String>,
}

/// Parse the manifest YAML. Any parse or schema-validation failure is the loud
/// `Malformed` path (taxonomy `compile_error` class).
pub(crate) fn parse_manifest(source: &str) -> Result<Manifest, CompileError> {
    serde_yaml::from_str::<Manifest>(source).map_err(|e| CompileError::Malformed {
        reason: format!("manifest parse: {e}"),
    })
}

/// Ruleset-level budget class = max class over used assertions (schema doc L188),
/// surfaced by `compile` so P6-VERDICTS can detect corpus-class-ness at load
/// (a corpus-class pack loaded sidecar-mode is later refused `daemon_only` —
/// NOT defined here). Stand-in: a conservative textual scan of rule-page sources
/// for the §4 vocabulary's file/corpus assertion names. P6-EVAL replaces this
/// with precise per-assertion analysis once real rule parsing lands; until then
/// over-classification (flag Corpus/File when unsure) is the safe direction for
/// a load gate.
pub(crate) fn classify_budget_class(rule_sources: &[String]) -> BudgetClass {
    let mut max = BudgetClass::Node;
    for src in rule_sources {
        // corpus-class (§4 #13) is the ceiling — short-circuit.
        if src.contains("link_resolves") {
            return BudgetClass::Corpus;
        }
        // file-class (§4 #11, #12).
        if src.contains("sibling_exists") || src.contains("child_exists") {
            max = BudgetClass::File;
        }
    }
    max
}

/// The demonstration a fixture asserts under its declared budgets. Authoritative
/// via frontmatter `expect:`; the `-pass`/`-fail` filename convention is
/// descriptive only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Expect {
    Pass,
    Fail,
}

impl std::fmt::Display for Expect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Expect::Pass => "pass",
            Expect::Fail => "fail",
        })
    }
}

/// Only the fixture frontmatter key the load gate reads (other keys ignored).
#[derive(serde::Deserialize)]
struct FixtureFrontmatter {
    expect: String,
}

/// One parsed fixture: its declared expectation plus the markdown body the
/// evaluator runs over.
pub(crate) struct FixtureDoc {
    pub path: String,
    pub expect: Expect,
    pub body: String,
}

/// Parse a fixture file: frontmatter declaring `expect: pass|fail`, then body.
/// A fixture with no such declaration cannot serve as a load-gate demonstration,
/// so it is a loud `FixtureFailed`.
pub(crate) fn parse_fixture(name: &str, content: &str) -> Result<FixtureDoc, CompileError> {
    let fail = |detail: String| CompileError::FixtureFailed {
        fixture: name.to_string(),
        detail,
    };
    let (frontmatter, body) = split_frontmatter(content)
        .ok_or_else(|| fail("no frontmatter declaring `expect: pass|fail`".into()))?;

    // Fixtures may carry arbitrary other frontmatter; only `expect` is read.
    let parsed: FixtureFrontmatter =
        serde_yaml::from_str(frontmatter).map_err(|e| fail(format!("frontmatter parse: {e}")))?;
    let expect = match parsed.expect.as_str() {
        "pass" => Expect::Pass,
        "fail" => Expect::Fail,
        other => return Err(fail(format!("`expect` must be pass|fail, got '{other}'"))),
    };
    Ok(FixtureDoc {
        path: name.to_string(),
        expect,
        body: body.to_string(),
    })
}

/// Split a leading `---` fenced YAML frontmatter block from the body. Returns
/// `None` when the content does not open with a frontmatter fence.
fn split_frontmatter(content: &str) -> Option<(&str, &str)> {
    let mut lines = content.split_inclusive('\n');
    let first = lines.next()?;
    if first.trim_end() != "---" {
        return None;
    }
    let fm_start = first.len();
    let mut idx = fm_start;
    for line in lines {
        if line.trim_end() == "---" {
            return Some((&content[fm_start..idx], &content[idx + line.len()..]));
        }
        idx += line.len();
    }
    None
}

// ── Injected world-model facts (rulepack-api@1) ──────────────────────────────

/// One world-model node exposed to a predicate as a Starlark struct. This is the
/// fact-injection surface: the load gate builds it synthetically from fixture
/// markdown ([`facts_from_markdown`]); P6-EVAL builds it from a real
/// `model::Document` AST — the *source* of the facts changes, this shape does not.
pub(crate) struct FactNode {
    /// `"heading"` / `"paragraph"` / … — the world-model node kind.
    pub kind: &'static str,
    /// Heading level (1..=6), or `0` for non-headings.
    pub level: u32,
    pub text: String,
    /// Byte span `(start, end)` into the document — the §11.1 finding coordinate.
    pub span: (usize, usize),
    /// The node's content-addressed rev (§11.1). Empty for markdown-derived facts
    /// (the load gate); P6-EVAL injects the real rev from the AST.
    pub node_rev: String,
    /// Heading path to the node (world-model fact, §11.2).
    pub hpath: Vec<String>,
}

/// The document a predicate evaluates over: path + nodes in document order. The
/// injected `doc` value (see module docs) is this, allocated as a Starlark struct.
pub(crate) struct FactDoc {
    pub path: String,
    pub nodes: Vec<FactNode>,
}

/// Build the injected fact surface synthetically from a fixture's markdown body —
/// the load-gate stand-in for a real `model::Document` (`model::build` is still a
/// `todo!()`). Each non-blank line becomes one node: headings carry their level +
/// hpath, everything else is a paragraph. P6-EVAL replaces this builder with real
/// AST facts without reshaping [`FactDoc`] / [`FactNode`].
pub(crate) fn facts_from_markdown(path: &str, body: &str) -> FactDoc {
    let mut nodes = Vec::new();
    let mut offset = 0usize;
    for line in body.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        let text = line.trim_end_matches('\n');
        if text.trim().is_empty() {
            continue;
        }
        let end = start + text.len();
        if let Some(level) = heading_level(text) {
            let title = text.trim_start_matches('#').trim().to_string();
            nodes.push(FactNode {
                kind: "heading",
                level: u32::from(level),
                text: title.clone(),
                span: (start, end),
                node_rev: String::new(),
                hpath: vec![title],
            });
        } else {
            nodes.push(FactNode {
                kind: "paragraph",
                level: 0,
                text: text.to_string(),
                span: (start, end),
                node_rev: String::new(),
                hpath: Vec::new(),
            });
        }
    }
    FactDoc {
        path: path.to_string(),
        nodes,
    }
}

/// ATX heading level (1..=6) if `line` is a heading, else `None`.
fn heading_level(line: &str) -> Option<u8> {
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ') {
        u8::try_from(hashes).ok()
    } else {
        None
    }
}

// ── Fenced-predicate extraction ──────────────────────────────────────────────

/// One rule page's extracted, parse-checked Starlark predicate.
#[derive(Debug)]
pub(crate) struct Predicate {
    /// The rule page path — carried into parse/runtime error messages.
    pub origin: String,
    /// The fenced Starlark source (a `def check(doc)` definition).
    pub source: String,
}

/// Extract and parse-check the fenced Starlark predicate of every rule page. A
/// page with no ` ```starlark ` block, or one whose Starlark will not parse, fails
/// compile loudly (taxonomy `compile_error` → [`CompileError::Malformed`]) BEFORE
/// any fixture runs — a rule that cannot even be read does not gate on fixtures.
pub(crate) fn extract_predicates(
    rule_sources: &[String],
    rule_paths: &[String],
) -> Result<Vec<Predicate>, CompileError> {
    let mut out = Vec::with_capacity(rule_sources.len());
    for (source, path) in rule_sources.iter().zip(rule_paths) {
        let code = extract_fenced_starlark(source).ok_or_else(|| CompileError::Malformed {
            reason: format!("rule '{path}' has no fenced ```starlark predicate block"),
        })?;
        AstModule::parse(path, code.clone(), &Dialect::Standard).map_err(|e| {
            CompileError::Malformed {
                reason: format!("rule '{path}' starlark parse error: {e}"),
            }
        })?;
        out.push(Predicate {
            origin: path.clone(),
            source: code,
        });
    }
    Ok(out)
}

/// Pull the first ` ```starlark … ``` ` fenced block's inner source from a literate
/// rule page. Returns `None` when the page carries no such block.
fn extract_fenced_starlark(page: &str) -> Option<String> {
    let mut collecting = false;
    let mut buf = String::new();
    for line in page.lines() {
        let trimmed = line.trim_start();
        if collecting {
            if trimmed.starts_with("```") {
                return Some(buf);
            }
            buf.push_str(line);
            buf.push('\n');
        } else if let Some(info) = trimmed.strip_prefix("```")
            && info.trim() == "starlark"
        {
            collecting = true;
        }
    }
    None
}

// ── The sealed evaluation bridge ─────────────────────────────────────────────

/// Raised when a fixture evaluation exceeds the pack's per-eval `{steps, mem}`
/// budget. At the load gate this is a fixture failure (pack refused); once
/// P6-EVAL wires real evaluation, the same exhaustion surfaces on the wire as
/// the `budget_exceeded` FINDING (never an error frame, §8).
#[derive(Debug)]
pub(crate) struct BudgetExhausted {
    pub steps: u64,
    pub mem: u64,
}

/// Why an evaluation did not produce a verdict: it exhausted its per-eval budget,
/// or the predicate faulted (parse error at runtime, missing `check`, a raised
/// error, a bad `violation()` argument). `compile` maps both to `FixtureFailed`.
#[derive(Debug)]
pub(crate) enum EvalError {
    Budget(BudgetExhausted),
    Runtime(String),
}

/// The sealed evaluation bridge. `StarlarkEvaluator` is the real impl; the load
/// gate (and, later, `evaluate`) call through this trait, so the evaluator never
/// touches the public API. P6-EVAL grows the fact surface / WHEN-vocabulary
/// enforcement behind this same seam.
pub(crate) trait RuleEvaluator {
    /// Evaluate the pack's predicates over one fixture's body, metered under
    /// `budget`. `Ok(violations)` (empty = the fixture passes) or [`EvalError`].
    fn eval_fixture(
        &self,
        predicates: &[Predicate],
        fixture: &FixtureDoc,
        budget: EvalBudget,
    ) -> Result<Vec<Violation>, EvalError>;
}

/// The ruling-008 evaluator: fenced Starlark predicates over injected world-model
/// facts, in-engine via starlark-rust, metered per-eval by `{steps, mem}`.
pub(crate) struct StarlarkEvaluator;

impl RuleEvaluator for StarlarkEvaluator {
    fn eval_fixture(
        &self,
        predicates: &[Predicate],
        fixture: &FixtureDoc,
        budget: EvalBudget,
    ) -> Result<Vec<Violation>, EvalError> {
        let facts = facts_from_markdown(&fixture.path, &fixture.body);
        eval_over_facts(predicates, &facts, budget)
    }
}

/// The evaluation core: run every predicate over one injected [`FactDoc`], metered
/// cumulatively under `budget`. Separated from fixture parsing so P6-EVAL can call
/// it with facts built from real ASTs, and so tests can inject facts directly.
pub(crate) fn eval_over_facts(
    predicates: &[Predicate],
    facts: &FactDoc,
    budget: EvalBudget,
) -> Result<Vec<Violation>, EvalError> {
    let globals = rulepack_globals();
    let mut all = Vec::new();
    let mut total_steps: u64 = 0;
    let mut peak_mem: u64 = 0;
    for predicate in predicates {
        let run = run_predicate(&globals, predicate, facts, budget)?;
        total_steps = total_steps.saturating_add(run.used_steps);
        peak_mem = peak_mem.max(run.used_mem);
        if total_steps > budget.steps || peak_mem > budget.mem {
            return Err(EvalError::Budget(BudgetExhausted {
                steps: budget.steps,
                mem: budget.mem,
            }));
        }
        all.extend(run.violations);
    }
    Ok(all)
}

/// One predicate's evaluation outcome: findings plus the exact resources it drew.
struct PredicateRun {
    violations: Vec<Violation>,
    used_steps: u64,
    used_mem: u64,
}

/// Build the `rulepack-api@1` globals: the Starlark standard library plus the
/// injected `violation()` builtin.
fn rulepack_globals() -> Globals {
    GlobalsBuilder::standard().with(rulepack_api).build()
}

/// Heap arena bytes as `u64` (saturating) — the memory meter reads.
fn heap_bytes(heap: Heap<'_>) -> u64 {
    u64::try_from(heap.allocated_bytes()).unwrap_or(u64::MAX)
}

/// Evaluate one predicate over the injected facts, metered.
///
/// starlark-rust enforces its `set_max_*` limits only at coarse (~1000-tick)
/// boundaries, so those are best-effort *runaway* guards (they stop an infinite
/// loop at the load gate); the load gate's real bound is the EXACT post-eval
/// accounting — `get_total_tick_count()` for steps, total heap-arena bytes for
/// mem (the arena grows in chunks, so a delta rounds to zero; the total is the
/// honest coarse bound, and counts the injected facts as working set) — which the
/// caller compares against the declared budget.
fn run_predicate(
    globals: &Globals,
    predicate: &Predicate,
    facts: &FactDoc,
    budget: EvalBudget,
) -> Result<PredicateRun, EvalError> {
    let store = EmitStore {
        path: facts.path.clone(),
        out: RefCell::new(Vec::new()),
    };
    let step_guard = budget.steps.max(1);
    let mem_guard = usize::try_from(budget.mem).unwrap_or(usize::MAX).max(1);

    Module::with_temp_heap(|module| {
        let heap = module.heap();
        let doc_value = alloc_doc(heap, facts);

        let mut eval = Evaluator::new(&module);
        eval.set_max_tick_count(step_guard)
            .map_err(|e| EvalError::Runtime(e.to_string()))?;
        eval.set_max_heap_size(mem_guard)
            .map_err(|e| EvalError::Runtime(e.to_string()))?;
        eval.extra = Some(&store);

        let ast = AstModule::parse(
            &predicate.origin,
            predicate.source.clone(),
            &Dialect::Standard,
        )
        .map_err(|e| EvalError::Runtime(format!("{}: {e}", predicate.origin)))?;

        // Define `check`, fetch it, call it with the injected doc.
        let mut aborted = false;
        let mut fault: Option<String> = None;
        match eval.eval_module(ast, globals) {
            Ok(_) => match module.get("check") {
                Some(check) => {
                    if let Err(e) = eval.eval_function(check, &[doc_value], &[]) {
                        aborted = true;
                        fault = Some(e.to_string());
                    }
                }
                None => {
                    fault = Some(format!(
                        "rule '{}' defines no `check(doc)` predicate",
                        predicate.origin
                    ));
                }
            },
            Err(e) => {
                aborted = true;
                fault = Some(e.to_string());
            }
        }

        let used_steps = eval.get_total_tick_count();
        let used_mem = heap_bytes(module.heap());
        drop(eval);
        let violations = store.out.take();

        if aborted {
            // The eval errored: was it the runaway guard tripping (⇒ budget) or a
            // genuine fault? Exact accounting decides.
            if used_steps > budget.steps || used_mem > budget.mem {
                return Err(EvalError::Budget(BudgetExhausted {
                    steps: budget.steps,
                    mem: budget.mem,
                }));
            }
            return Err(EvalError::Runtime(fault.unwrap_or_default()));
        }
        if let Some(msg) = fault {
            return Err(EvalError::Runtime(msg));
        }
        Ok(PredicateRun {
            violations,
            used_steps,
            used_mem,
        })
    })
}

/// The store the injected `violation()` builtin records findings into, reached via
/// `Evaluator::extra`. `path` is the current document's path (a doc-level fact the
/// builtin stamps onto each finding).
#[derive(starlark::any::ProvidesStaticType)]
struct EmitStore {
    path: String,
    out: RefCell<Vec<Violation>>,
}

/// The injected `rulepack-api@1` builtins.
#[starlark_module]
fn rulepack_api(builder: &mut GlobalsBuilder) {
    /// Record one §11.1 finding. All arguments are named; `severity` ∈
    /// {`error`, `warn`, `info`}; `span` is an `(start, end)` int tuple; `hpath`
    /// is a list of heading-path strings.
    fn violation(
        #[starlark(require = named)] rule: String,
        #[starlark(require = named)] severity: String,
        #[starlark(require = named)] span: (usize, usize),
        #[starlark(require = named)] node_rev: String,
        #[starlark(require = named)] hpath: UnpackList<String>,
        #[starlark(require = named)] message: String,
        eval: &mut Evaluator<'_, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let store = eval
            .extra
            .and_then(|e| e.downcast_ref::<EmitStore>())
            .ok_or_else(|| {
                anyhow::anyhow!("rulepack-api: violation() invoked without an emit store")
            })?;
        let severity = severity_from_str(&severity).ok_or_else(|| {
            anyhow::anyhow!("rulepack-api: severity must be error|warn|info, got '{severity}'")
        })?;
        store.out.borrow_mut().push(Violation {
            rule,
            severity,
            path: store.path.clone(),
            span: span.0..span.1,
            node_rev: NodeRev(node_rev),
            hpath: Some(hpath.items),
            message,
        });
        Ok(NoneType)
    }
}

/// Map the injected `severity` string to the typed [`Severity`].
fn severity_from_str(s: &str) -> Option<Severity> {
    match s {
        "error" => Some(Severity::Error),
        "warn" => Some(Severity::Warn),
        "info" => Some(Severity::Info),
        _ => None,
    }
}

/// Allocate the injected `doc` value: a struct `{path, nodes}` where `nodes` is a
/// list of node structs (see module docs for the field surface).
fn alloc_doc<'v>(heap: Heap<'v>, facts: &FactDoc) -> Value<'v> {
    let nodes: Vec<Value<'v>> = facts.nodes.iter().map(|n| alloc_node(heap, n)).collect();
    heap.alloc(AllocStruct([
        ("path", heap.alloc(facts.path.as_str())),
        ("nodes", heap.alloc(nodes)),
    ]))
}

/// Allocate one injected node struct.
fn alloc_node<'v>(heap: Heap<'v>, node: &FactNode) -> Value<'v> {
    let hpath: Vec<Value<'v>> = node.hpath.iter().map(|s| heap.alloc(s.as_str())).collect();
    heap.alloc(AllocStruct([
        ("kind", heap.alloc(node.kind)),
        ("level", heap.alloc(node.level)),
        ("text", heap.alloc(node.text.as_str())),
        ("span", heap.alloc((node.span.0, node.span.1))),
        ("node_rev", heap.alloc(node.node_rev.as_str())),
        ("hpath", heap.alloc(hpath)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The canonical `blurb-required` rule page (a literate page with a fenced
    /// Starlark predicate) the unit tests evaluate.
    const BLURB_RULE_PAGE: &str = "\
# blurb-required

Every section must open with a blurb line: a heading immediately followed by
another heading (or the end of the document) has no blurb and is a violation.

```starlark
def check(doc):
    nodes = doc.nodes
    count = len(nodes)
    for i in range(count):
        node = nodes[i]
        if node.kind != \"heading\":
            continue
        has_blurb = i + 1 < count and nodes[i + 1].kind != \"heading\"
        if not has_blurb:
            violation(
                rule = \"blurb-required\",
                severity = \"warn\",
                span = node.span,
                node_rev = node.node_rev,
                hpath = node.hpath,
                message = \"section has no blurb line\",
            )
```
";

    fn blurb_predicates() -> Vec<Predicate> {
        extract_predicates(
            &[BLURB_RULE_PAGE.to_string()],
            &["rules/blurb-required.md".to_string()],
        )
        .expect("blurb rule page extracts + parses")
    }

    fn budget() -> EvalBudget {
        EvalBudget {
            steps: 10_000,
            mem: 1 << 20,
        }
    }

    #[test]
    fn classify_picks_max_class() {
        assert_eq!(classify_budget_class(&[]), BudgetClass::Node);
        assert_eq!(
            classify_budget_class(&["assert:\n  - keys_include: [a]".into()]),
            BudgetClass::Node
        );
        assert_eq!(
            classify_budget_class(&["assert:\n  - sibling_exists: {}".into()]),
            BudgetClass::File
        );
        assert_eq!(
            classify_budget_class(&[
                "assert:\n  - sibling_exists: {}".into(),
                "assert:\n  - link_resolves".into(),
            ]),
            BudgetClass::Corpus
        );
    }

    #[test]
    fn fixture_expect_parsed_from_frontmatter() {
        let fx = parse_fixture("f.md", "---\nexpect: fail\n---\n# H\n").unwrap();
        assert_eq!(fx.expect, Expect::Fail);
        assert_eq!(fx.body, "# H\n");
    }

    #[test]
    fn fixture_without_expect_declaration_is_loud() {
        assert!(matches!(
            parse_fixture("f.md", "# no frontmatter\n"),
            Err(CompileError::FixtureFailed { .. })
        ));
    }

    #[test]
    fn extract_rejects_page_without_starlark_fence() {
        // A plain ``` fence (no `starlark` info string) is not a predicate.
        let err = extract_predicates(
            &["# r\n\n```\nassert: x\n```\n".to_string()],
            &["rules/r.md".to_string()],
        )
        .unwrap_err();
        assert!(matches!(err, CompileError::Malformed { .. }));
    }

    #[test]
    fn extract_rejects_unparseable_predicate() {
        let err = extract_predicates(
            &["# r\n\n```starlark\ndef check(doc:\n```\n".to_string()],
            &["rules/r.md".to_string()],
        )
        .unwrap_err();
        assert!(matches!(err, CompileError::Malformed { .. }));
    }

    /// Gate 1 (unit) — the injected fact surface flows through the fenced Starlark
    /// predicate into the §11.1 finding: a hand-built `FactDoc` (the surface
    /// P6-EVAL feeds from real ASTs) with an explicit `node_rev`/`span`/`hpath`
    /// yields exactly one violation carrying those injected coordinates.
    #[test]
    fn gate1_injected_facts_flow_through_to_violation() {
        let facts = FactDoc {
            path: "notes/plan.md".into(),
            nodes: vec![
                FactNode {
                    kind: "heading",
                    level: 1,
                    text: "Goals".into(),
                    span: (20, 27),
                    node_rev: "5a8faa717fbcdb04".into(),
                    hpath: vec!["Goals".into()],
                },
                FactNode {
                    kind: "heading",
                    level: 2,
                    text: "Sub".into(),
                    span: (28, 34),
                    node_rev: "deadbeefdeadbeef".into(),
                    hpath: vec!["Sub".into()],
                },
                FactNode {
                    kind: "paragraph",
                    level: 0,
                    text: "body".into(),
                    span: (35, 39),
                    node_rev: String::new(),
                    hpath: Vec::new(),
                },
            ],
        };
        let v = eval_over_facts(&blurb_predicates(), &facts, budget()).unwrap();
        assert_eq!(v.len(), 1, "only the blurbless `Goals` heading fires");
        let f = &v[0];
        assert_eq!(f.rule, "blurb-required");
        assert_eq!(f.severity, Severity::Warn);
        assert_eq!(f.path, "notes/plan.md"); // doc-level fact
        assert_eq!(f.span, 20..27); // injected node.span
        assert_eq!(f.node_rev, NodeRev("5a8faa717fbcdb04".into())); // injected node.node_rev
        assert_eq!(f.hpath, Some(vec!["Goals".into()])); // injected node.hpath
        assert_eq!(f.message, "section has no blurb line");
    }

    #[test]
    fn blurb_pass_has_no_violations() {
        let facts = facts_from_markdown("pass.md", "# Goals\nA blurb line.\n");
        let v = eval_over_facts(&blurb_predicates(), &facts, budget()).unwrap();
        assert!(
            v.is_empty(),
            "conforming fixture must produce no violations"
        );
    }

    #[test]
    fn blurb_fail_flags_the_blurbless_heading() {
        let facts =
            facts_from_markdown("fail.md", "# Goals\n## Immediately another heading\nbody\n");
        let v = eval_over_facts(&blurb_predicates(), &facts, budget()).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].message, "section has no blurb line");
    }

    #[test]
    fn eval_over_step_budget_is_exhaustion() {
        let facts = facts_from_markdown("f.md", "# A\nx\n# B\ny\n");
        let out = eval_over_facts(
            &blurb_predicates(),
            &facts,
            EvalBudget {
                steps: 1,
                mem: 1 << 20,
            },
        );
        assert!(matches!(out, Err(EvalError::Budget(_))));
    }

    #[test]
    fn eval_over_mem_budget_is_exhaustion() {
        let facts = facts_from_markdown("f.md", "# A\nblurb\n");
        let out = eval_over_facts(
            &blurb_predicates(),
            &facts,
            EvalBudget {
                steps: 10_000,
                mem: 2,
            },
        );
        assert!(matches!(out, Err(EvalError::Budget(_))));
    }

    #[test]
    fn missing_check_predicate_is_runtime_error() {
        // Parses fine, but defines no `check` — a rule defect, surfaced loud.
        let preds = extract_predicates(
            &["# r\n\n```starlark\nx = 1\n```\n".to_string()],
            &["rules/r.md".to_string()],
        )
        .unwrap();
        let facts = facts_from_markdown("f.md", "# A\nb\n");
        assert!(matches!(
            eval_over_facts(&preds, &facts, budget()),
            Err(EvalError::Runtime(_))
        ));
    }
}
