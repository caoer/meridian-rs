//! Pack loading for `compile` (§11.3): manifest parse, fixtures, the injected
//! fact surface, and metered predicate evaluation. Pin verify before run.
//! [`facts_from_document`] builds [`FactDoc`]s from a real AST so no
//! `syntax`/`model` type crosses the load-gate fence.

use std::cell::RefCell;

use model::{Document, Node, NodeKind, NodeRev};
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

/// Ruleset-level budget class = max class over used assertions, surfaced by
/// `compile` so corpus-class-ness is detectable at load (a corpus-class pack
/// needs the resident index; the `daemon_only` refusal that once enforced
/// this at an index-less host is RETIRED — §3.3/§8). A
/// conservative textual scan of rule-page sources for the §4 file/corpus
/// assertion names; over-classification is the safe direction for a load gate.
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
/// `None` when the content does not open with a frontmatter fence. Shared with the
/// convention loader (`CHECK.md` carries the same `---` frontmatter + body shape).
pub(crate) fn split_frontmatter(content: &str) -> Option<(&str, &str)> {
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

/// One world-model node exposed to a predicate as a Starlark struct — the
/// fact-injection surface. Production builds it from a real `model::Document`
/// AST via [`facts_from_document`]; the synthetic `facts_from_markdown` builder
/// is test-only.
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
///
/// Public but opaque (fields `pub(crate)`): the return type of the load gate's
/// injected fact builder and of [`crate::facts_from_document`], so the
/// composition layer hands policy facts through the same plane production
/// evaluation uses without the closure signature naming `syntax::`/`model::`
/// types. Consumers pass it; only policy reads it.
pub struct FactDoc {
    pub(crate) path: String,
    pub(crate) nodes: Vec<FactNode>,
}

/// Build the injected fact surface synthetically from a fixture's markdown body,
/// one node per non-blank line (headings carry level + hpath, else paragraph).
///
/// Test-only: retired from the production admission path — its per-line
/// paragraph fabrication demonstrates fixtures on a fact plane production never
/// reproduces (the real world model is paragraph-blind).
#[cfg(test)]
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

/// ATX heading level (1..=6) if `line` is a heading, else `None`. Test-only: the
/// only caller is the retired synthetic `facts_from_markdown` builder.
#[cfg(test)]
fn heading_level(line: &str) -> Option<u8> {
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ') {
        u8::try_from(hashes).ok()
    } else {
        None
    }
}

/// Derive the injected fact surface from a REAL `model::Document` AST — the
/// production fact plane. Public: the composition layer calls it inside the
/// load gate's injected builder so fixtures demonstrate over the same plane
/// `evaluate` runs.
///
/// The world-model tree flattens to `doc.nodes` in document order (preorder
/// walk, §1 total order). The root `Document` node is not emitted — it *is*
/// `doc`. A model `Section` injects as a `"heading"` fact carrying the
/// section's real coordinates (heading-through-subtree `span`,
/// content-addressed `node_rev`, `hpath`), so a §11.1 finding carries the wire
/// coordinates verbatim.
///
/// The model is paragraph-blind (no `Paragraph` node is minted from the dialect
/// stream today), so a section that opens directly onto a subsection has no
/// intervening content fact.
#[must_use]
pub fn facts_from_document(doc: &Document) -> FactDoc {
    let path = match &doc.root.kind {
        NodeKind::Document { path, .. } => path.clone(),
        _ => String::new(),
    };
    let mut nodes = Vec::new();
    for child in &doc.root.children {
        push_facts(&doc.raw, child, &mut nodes);
    }
    FactDoc { path, nodes }
}

/// Preorder emit: the node itself, then its children in span order.
fn push_facts(raw: &str, node: &Node, out: &mut Vec<FactNode>) {
    out.push(fact_node(raw, node));
    for child in &node.children {
        push_facts(raw, child, out);
    }
}

/// One model node → one injected [`FactNode`] (span/rev/hpath carried verbatim).
fn fact_node(raw: &str, node: &Node) -> FactNode {
    let (kind, level, text) = classify_fact(raw, node);
    FactNode {
        kind,
        level,
        text,
        span: (node.span.start, node.span.end),
        node_rev: node.node_rev.0.clone(),
        hpath: node.hpath.clone().unwrap_or_default(),
    }
}

/// Map a model [`NodeKind`] to the injected `(kind, level, text)` triple. Sections
/// and headings inject as `"heading"` (§11.2 node vocabulary); the rest carry their
/// world-model kind so a predicate can distinguish content from structure.
fn classify_fact(raw: &str, node: &Node) -> (&'static str, u32, String) {
    use NodeKind as K;
    match &node.kind {
        K::Document { .. } => ("document", 0, String::new()),
        K::Frontmatter { .. } => ("frontmatter", 0, String::new()),
        K::Section {
            heading_text,
            level,
        } => ("heading", u32::from(*level), heading_text.clone()),
        K::Heading { text, level } => ("heading", u32::from(*level), text.clone()),
        K::Paragraph => ("paragraph", 0, span_text(raw, node)),
        K::List => ("list", 0, String::new()),
        K::ListItem => ("list_item", 0, span_text(raw, node)),
        K::TaskItem { .. } => ("task", 0, span_text(raw, node)),
        K::CodeBlock { .. } => ("code_block", 0, span_text(raw, node)),
        K::Callout { .. } => ("callout", 0, span_text(raw, node)),
        K::Table => ("table", 0, span_text(raw, node)),
        K::Wikilink { target, .. } => ("wikilink", 0, target.clone()),
        K::Link { target } => ("link", 0, target.clone()),
        K::Embed { target, .. } => ("embed", 0, target.clone()),
        K::Anchor { name } => ("anchor", 0, name.clone()),
        K::Tag { name } => ("tag", 0, name.clone()),
        K::InlineCode => ("inline_code", 0, span_text(raw, node)),
        K::Comment => ("comment", 0, span_text(raw, node)),
    }
}

/// The node's raw bytes as trimmed text (a best-effort content fact for non-heading
/// nodes); an out-of-range or non-char-boundary span yields the empty string rather
/// than panicking.
fn span_text(raw: &str, node: &Node) -> String {
    raw.get(node.span.clone())
        .unwrap_or_default()
        .trim()
        .to_string()
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
/// rule page. Returns `None` when the page carries no such block. Shared with the
/// convention loader (`CHECK.md` carries its `def check_change` predicate the same
/// way).
pub(crate) fn extract_fenced_starlark(page: &str) -> Option<String> {
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
            && info.split_whitespace().next() == Some("starlark")
        {
            // The CommonMark info string's first token is the language; a trailing
            // string (```starlark title=…) still names a Starlark predicate block.
            collecting = true;
        }
    }
    None
}

// ── The closed WHEN vocabulary (§11.2, compile-enforced) ─────────────────────

/// The fact-vocabulary version this engine implements — tied to the `rulepack-api@1`
/// pin: version 1 injects the world-model node surface (`doc.path`, `doc.nodes`, and
/// each node's `kind`/`level`/`text`/`span`/`node_rev`/`hpath`) plus the `violation()`
/// builtin, over the Starlark standard library. A WHEN that references any name
/// outside that surface presumes a fact the engine does not provide — a *later*
/// vocabulary — and is refused at compile ([`check_when_vocab`]).
pub(crate) const FACT_VOCAB_VERSION: u32 = 1;

/// §11.2 WHEN/HOW partition, compile-enforced: a rule's WHEN sees world-model facts
/// ONLY. Statically resolve each predicate's free names against the closed injected
/// vocabulary (the `rulepack-api@1` globals — Starlark stdlib + `violation()`); a
/// reference to any *other* global (`now`, `actor`, `link_resolves`, a stray helper)
/// is a fact the engine does not inject, so the WHEN references outside the
/// vocabulary and is refused loud with [`CompileError::UnsupportedVocab`] — at COMPILE
/// time, before any fixture runs, never at eval time.
///
/// Node/doc FIELD access (`node.kind`, `doc.nodes`) is attribute access, not a
/// name, so it is out of this check's scope by construction — the boundary is
/// the set of *global names* a predicate may reach for. The check leans on
/// starlark's own name resolution (`AstModuleLint`), so it tracks the real
/// binding rules rather than a hand-rolled scan.
pub(crate) fn check_when_vocab(predicates: &[Predicate]) -> Result<(), CompileError> {
    use starlark::analysis::AstModuleLint;
    use std::collections::HashSet;

    let allowed: HashSet<String> = rulepack_globals()
        .names()
        .map(|n| n.as_str().to_owned())
        .collect();

    for predicate in predicates {
        let ast = AstModule::parse(
            &predicate.origin,
            predicate.source.clone(),
            &Dialect::Standard,
        )
        .map_err(|e| CompileError::Malformed {
            reason: format!("rule '{}' starlark parse error: {e}", predicate.origin),
        })?;
        // `using-undefined` = a name used that is neither a local binding nor one of
        // the injected globals: the WHEN reaches outside the fact vocabulary.
        if ast
            .lint(Some(&allowed))
            .iter()
            .any(|l| l.short_name == "using-undefined")
        {
            return Err(CompileError::UnsupportedVocab {
                requires: FACT_VOCAB_VERSION + 1,
                engine: FACT_VOCAB_VERSION,
            });
        }
    }
    Ok(())
}

// ── The sealed evaluation bridge ─────────────────────────────────────────────

/// Raised when a fixture evaluation exceeds the pack's per-eval `{steps, mem}`
/// budget. At the load gate this is a fixture failure (pack refused); on the
/// wire the same exhaustion surfaces as the `budget_exceeded` FINDING (never an
/// error frame, §8).
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

/// The evaluation core: run every predicate over one injected [`FactDoc`], metered
/// cumulatively under `budget`. Separated from fixture parsing so the wire path
/// can call it with facts built from real ASTs, and tests can inject facts.
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

/// Evaluate a ruleset over one REAL document's facts (P6-EVAL), metering the
/// per-eval `{steps, mem}` budget cumulatively across predicates. This is the wire
/// path (`policy::evaluate`): budget exhaustion becomes the `budget_exceeded`
/// FINDING appended to the returned verdicts — NEVER an error or panic (frozen §8:
/// `budget_exceeded` is a typed finding inside `verdicts`, not a wire error class).
///
/// Distinct from [`eval_over_facts`] (the load gate), which returns `Err(Budget)` so
/// a pack that cannot demonstrate itself under budget is refused at compile. Here the
/// pack is already admitted; exhaustion on a real doc is reported, not refused.
///
/// `doc_root_rev` anchors the budget finding to the document version it was metered
/// against (the root node's rev = the file rev).
pub(crate) fn eval_document(
    predicates: &[Predicate],
    facts: &FactDoc,
    budget: EvalBudget,
    doc_root_rev: &str,
) -> Vec<Violation> {
    let globals = rulepack_globals();
    let mut out = Vec::new();
    let mut total_steps: u64 = 0;
    let mut peak_mem: u64 = 0;
    for predicate in predicates {
        match run_predicate(&globals, predicate, facts, budget) {
            Ok(run) => {
                total_steps = total_steps.saturating_add(run.used_steps);
                peak_mem = peak_mem.max(run.used_mem);
                // Cumulative post-eval accounting — the exact bound (per-predicate
                // guards are best-effort, see `run_predicate`). Over budget ⇒ the
                // finding, and evaluation stops (the verdict set is truncated).
                if total_steps > budget.steps || peak_mem > budget.mem {
                    out.push(budget_finding(predicate, facts, doc_root_rev, budget));
                    break;
                }
                out.extend(run.violations);
            }
            Err(EvalError::Budget(_)) => {
                out.push(budget_finding(predicate, facts, doc_root_rev, budget));
                break;
            }
            Err(EvalError::Runtime(_)) => {
                // A post-admission runtime fault (the load gate already ran this
                // predicate over its fixtures): the rule contributes no findings for
                // this doc. Only budget exhaustion is a typed wire finding (§8) — a
                // runtime fault is not, so it is not fabricated into one here.
            }
        }
    }
    out
}

/// The `budget_exceeded` finding (frozen §8): rule = the busted rule PAGE (the
/// predicate origin — a predicate that exhausts its budget may never reach its own
/// `violation(rule=…)` call, so the page path is the only honest identifier of the
/// rule that overran); severity = `Error` (the doc could harbor violations this rule
/// never got to check — a truncated evaluation is a correctness gap Go should see,
/// though whether an `error` blocks stays Go's call, §11.1); span `0..0` and no hpath
/// (the finding is rule-level, not node-anchored), `node_rev` = the document version.
fn budget_finding(
    predicate: &Predicate,
    facts: &FactDoc,
    doc_root_rev: &str,
    budget: EvalBudget,
) -> Violation {
    Violation {
        rule: predicate.origin.clone(),
        severity: Severity::Error,
        path: facts.path.clone(),
        span: 0..0,
        node_rev: NodeRev(doc_root_rev.to_owned()),
        hpath: None,
        message: format!(
            "budget_exceeded: rule evaluation exceeded the per-eval budget \
             (steps>{} or mem>{} bytes) — evaluation truncated",
            budget.steps, budget.mem
        ),
    }
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

/// Peak heap-arena bytes as `u64` (saturating) — the memory meter reads. Peak
/// (not current) is monotonic and matches the quantity starlark's own
/// `set_max_heap_size` guard checks, so an abort by that guard and the post-eval
/// accounting agree, and a post-eval GC cannot deflate the reading.
fn heap_bytes(heap: Heap<'_>) -> u64 {
    u64::try_from(heap.peak_allocated_bytes()).unwrap_or(u64::MAX)
}

/// Evaluate one predicate over the injected facts, metered.
///
/// starlark-rust enforces its `set_max_*` limits only at coarse (~1000-event)
/// boundaries — best-effort runaway guards, not a hard wall-time/RSS bound: a
/// single non-looping allocating expression (`"x" * n`) runs to completion
/// before any guard fires. Exhaustion is therefore made loud and safe:
///
/// - The real verdict bound is the EXACT post-eval accounting
///   (`get_total_tick_count()` for steps, PEAK heap-arena bytes for mem)
///   against the declared budget — conservative, never falsely admitting an
///   over-budget pack.
/// - Mem counts the whole eval arena's high-water mark (injected facts + engine
///   working set + the arena's minimum chunk), so set realistic budgets — a
///   sub-chunk budget refuses even a no-op.
/// - A pathological overrun trips a `len overflow` assert inside the engine;
///   [`std::panic::catch_unwind`] converts that panic into a clean
///   [`EvalError::Runtime`] (⇒ `FixtureFailed`) so it never crashes the loader.
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

    // AssertUnwindSafe: on panic we discard `store` unread and return a fault, so
    // its interior mutability cannot leak an inconsistent value across the boundary.
    let evaluated = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Module::with_temp_heap(|module| {
            let heap = module.heap();
            let doc_value = alloc_doc(heap, facts);

            let mut eval = Evaluator::new(&module);
            // GC OFF, load-bearing (mw-splice-engine-eof, 2026-08-18):
            // `doc_value` lives in a Rust local across `eval_module` and is
            // NOT a GC root — a GC fired once the heap crosses starlark's
            // 100KB threshold (a large fact doc crosses it alone) collects it,
            // and `eval_function` then segfaults on the dangling Value.
            // `set_max_heap_size` still caps the arena; the temp heap drops
            // whole at scope end.
            eval.disable_gc();
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
    }));

    match evaluated {
        Ok(inner) => inner,
        Err(_panic) => Err(EvalError::Runtime(format!(
            "rule '{}' panicked inside the Starlark engine (resource overflow — an \
             allocation that outran the best-effort budget guards)",
            predicate.origin
        ))),
    }
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
        // A finding coordinate must be a well-formed byte range: reject start > end
        // loudly (⇒ FixtureFailed), consistent with how out-of-domain span ints
        // (negative / > usize) already fail at the (usize, usize) unpack.
        if span.0 > span.1 {
            return Err(anyhow::anyhow!(
                "rulepack-api: violation span start ({}) exceeds end ({})",
                span.0,
                span.1
            ));
        }
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

    /// Regression (mw-splice-engine-eof, 2026-08-18): a fact doc whose
    /// temp-heap allocation crosses starlark's GC threshold (100KB) must
    /// survive evaluation. Pre-fix, `eval_module` fired a GC, collected the
    /// doc Value held in a Rust local (not a GC root), and `eval_function`
    /// SEGFAULTED the process — this test dies with SIGSEGV on the unfixed
    /// evaluator.
    #[test]
    fn a_fact_doc_past_the_gc_threshold_survives_evaluation() {
        use std::fmt::Write as _;
        let mut md = String::new();
        for i in 0..500 {
            let _ = write!(
                md,
                "## Section {i:03}\nThe hook plane unifies the four trigger families under \
                 one armed row shape; each family keeps its own firing clock but shares the \
                 severity ladder and the receipts contract. Row {i} carries its own receipts \
                 line.\n"
            );
        }
        let facts = facts_from_markdown("results/big.md", &md);
        let violations = eval_over_facts(
            &blurb_predicates(),
            &facts,
            EvalBudget {
                steps: 10_000_000,
                mem: 256 << 20,
            },
        )
        .expect("a big fact doc evaluates cleanly");
        assert!(
            violations.is_empty(),
            "every section carries a blurb line: {violations:?}"
        );
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

    #[test]
    fn violation_with_reversed_span_is_loud() {
        // A finding whose span start > end is a malformed coordinate; the injected
        // builtin rejects it (⇒ Runtime ⇒ FixtureFailed), just like an out-of-range
        // int would — the injection surface is consistent.
        let preds = extract_predicates(
            &["# r\n\n```starlark\ndef check(doc):\n    violation(rule=\"r\", severity=\"warn\", span=(100, 3), node_rev=\"\", hpath=[], message=\"m\")\n```\n".to_string()],
            &["rules/r.md".to_string()],
        )
        .unwrap();
        let facts = facts_from_markdown("f.md", "# A\nb\n");
        assert!(matches!(
            eval_over_facts(&preds, &facts, budget()),
            Err(EvalError::Runtime(_))
        ));
    }

    #[test]
    fn single_expression_allocation_is_metered_not_admitted() {
        // A non-looping allocation defeats the coarse runaway guard (no backedge),
        // but the EXACT post-eval mem accounting still catches it: refused as
        // Budget, never falsely admitted.
        let preds = extract_predicates(
            &["# big\n\n```starlark\ndef check(doc):\n    s = \"x\" * 100000\n    _ = len(s)\n```\n".to_string()],
            &["rules/big.md".to_string()],
        )
        .unwrap();
        let facts = facts_from_markdown("f.md", "# A\nb\n");
        let out = eval_over_facts(
            &preds,
            &facts,
            EvalBudget {
                steps: 10_000,
                mem: 4096,
            },
        );
        assert!(matches!(out, Err(EvalError::Budget(_))), "got {out:?}");
    }

    #[test]
    fn extract_accepts_starlark_fence_with_trailing_info() {
        // A CommonMark info string beyond the `starlark` lang token is still a
        // predicate block (```starlark title=demo).
        let preds = extract_predicates(
            &["# r\n\n```starlark title=demo\ndef check(doc):\n    pass\n```\n".to_string()],
            &["rules/r.md".to_string()],
        )
        .expect("```starlark with trailing info extracts");
        assert_eq!(preds.len(), 1);
    }

    // ── P6-EVAL: real-AST facts + WHEN vocabulary + budget finding ───────────

    /// A real `model::Document` for the given markdown (the caller's build step).
    fn doc_of(md: &str) -> Document {
        model::build(md.to_string(), syntax::parse(md))
    }

    #[test]
    fn facts_from_document_injects_sections_as_heading_facts_with_real_revs() {
        // Sections inject as `"heading"` facts in document order, carrying the
        // section's real span/rev/hpath (the surface a §11.1 finding rides).
        let doc = doc_of("# A\n\nbody\n\n## B\n\nmore\n");
        let facts = facts_from_document(&doc);
        let headings: Vec<&FactNode> = facts.nodes.iter().filter(|n| n.kind == "heading").collect();
        assert_eq!(headings.len(), 2, "two sections → two heading facts");

        assert_eq!(headings[0].text, "A");
        assert_eq!(headings[0].level, 1);
        assert_eq!(headings[0].hpath, vec!["A".to_string()]);
        assert_eq!(headings[0].span.0, 0, "the A section opens at byte 0");
        assert_eq!(
            headings[0].node_rev.len(),
            16,
            "real 16-hex rev from the AST"
        );

        assert_eq!(headings[1].text, "B");
        assert_eq!(headings[1].level, 2);
        assert_eq!(headings[1].hpath, vec!["A".to_string(), "B".to_string()]);
    }

    #[test]
    fn check_when_vocab_refuses_free_global_reference() {
        // `actor` is not injected by rulepack-api@1 → the WHEN reaches outside the
        // fact vocabulary → UnsupportedVocab, at compile.
        let preds = extract_predicates(
            &[
                "# t\n\n```starlark\ndef check(doc):\n    if actor:\n        pass\n```\n"
                    .to_string(),
            ],
            &["rules/t.md".to_string()],
        )
        .unwrap();
        assert!(matches!(
            check_when_vocab(&preds),
            Err(CompileError::UnsupportedVocab {
                requires: 2,
                engine: 1
            })
        ));
    }

    #[test]
    fn check_when_vocab_accepts_in_vocab_predicate() {
        // The blurb predicate touches only doc/node facts + stdlib + violation().
        assert!(check_when_vocab(&blurb_predicates()).is_ok());
    }

    #[test]
    fn eval_document_budget_exhaustion_is_a_finding() {
        // A cheap predicate under a steps:1 budget over a real doc: exhaustion lands
        // as the budget_exceeded finding (rule = the busted page), never an error.
        let doc = doc_of("# A\n\nb\n\n# B\n\nc\n");
        let facts = facts_from_document(&doc);
        let out = eval_document(
            &blurb_predicates(),
            &facts,
            EvalBudget {
                steps: 1,
                mem: 1 << 20,
            },
            "rootrev",
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].rule, "rules/blurb-required.md");
        assert_eq!(out[0].severity, Severity::Error);
        assert!(out[0].message.starts_with("budget_exceeded:"));
        assert_eq!(out[0].node_rev, NodeRev("rootrev".into()));
    }

    #[test]
    fn eval_document_drops_runtime_faulting_predicate_without_panic() {
        // A predicate that faults at eval (index past the end) contributes no
        // findings and does not panic — only budget exhaustion is a typed finding.
        let preds = extract_predicates(
            &[
                "# bad\n\n```starlark\ndef check(doc):\n    x = doc.nodes[999]\n    _ = x\n```\n"
                    .to_string(),
            ],
            &["rules/bad.md".to_string()],
        )
        .unwrap();
        let doc = doc_of("# A\n\nb\n");
        let facts = facts_from_document(&doc);
        let out = eval_document(
            &preds,
            &facts,
            EvalBudget {
                steps: 10_000,
                mem: 1 << 20,
            },
            "rootrev",
        );
        assert!(
            out.is_empty(),
            "a runtime-faulting rule drops out; no finding fabricated: {out:?}"
        );
    }
}
