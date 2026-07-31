//! `mrd test --corpus` — the tier-2 pre-arming runner (U1.5): drive CHECK and
//! HOOK conventions over SYNTHETIC changes derived from a governed corpus.
//!
//! # What a corpus-test spec is
//! A spec is a markdown file with `corpus:` plus one `convention:` or a
//! ` ```conventions ` list. `counterfactual: true` admits `md.*` descriptors in
//! this tier only so quiescence can be falsified without widening runtime caps.
//!
//! - ` ```rules ` — the DECLARED CHECK citations and HOOK slugs. Any one with no
//!   observed emission is a dead rule.
//! - ` ```case ` — one synthetic change as JSON: `{doc, actor?, force?, set?,
//!   remove?, expect}`. `expect` is one rule id, a list of ids, or `"pass"`.
//!
//! # The four signals (rulings § test tiers — `test --corpus`)
//! For each case the tier derives the shared [`Change`] from before/after states,
//! then runs every in-scope CHECK and HOOK under its declared budget:
//!
//! - **fire-where-expected** — the set of rules a case fired must EQUAL its
//!   `expect` (a single rule id, or `"pass"` for no fire). A doc outside the
//!   convention's `paths:` scope is never run — it can only pass (scope gating is
//!   observable here). A mismatch is a finding.
//! - **zero dead rules** — every DECLARED rule must fire at least once over the
//!   corpus; a declared rule with zero fires is reported DEAD (a finding). A rule
//!   the convention fired that the spec never declared is a `surprise` finding.
//! - **fuel + heap budgets** — exact ticks and peak heap, reduced to
//!   p50/p99/max over all in-scope evaluations.
//! - **FIX/HOOK quiescence** — follow reachable `md.*` descriptors through a
//!   trigger graph. A repeated `(state, pending descriptor)` is a cycle that can
//!   keep firing. The proof has its own fuel and disables runtime depth suppression.
//!
//! # Output + exit codes (§4 preamble law, `docs/status.md`)
//! JSON under `--json`, a human report otherwise. Exit 0 when all four signals are
//! clean; 1 for a mismatch, dead/surprise rule, budget/eval finding, or failed
//! quiescence; 2 for malformed input or unreadable state.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use effects::{ArgValue, CapabilitySet, Domain, Effect, EffectKind, EvalError, EvalLimits, Rule};
use model::{Document, NodeKind};
use policy::ConventionFiles;
use policy::{
    Change, ChangeOp, CheckError, CheckLimits, Convention, Invocation, derive_change, derive_event,
    load_convention, load_convention_for_corpus, load_seed_convention,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::test_cmd::{confine, parse_frontmatter, scan_blocks};
use crate::{Fail, Format};

/// Run `mrd test --corpus <SPEC> [--json]`: load the spec, load its convention,
/// run every case over the corpus, and render the report.
///
/// # Errors
/// [`Fail`] — exit 2 (a malformed spec, an unreadable corpus/convention, or a
/// per-case authoring fault) or exit 1 (a fire mismatch, a dead/surprise rule,
/// or a convention eval fault).
pub(crate) fn run(spec_path: &str, format: Format) -> Result<(), Fail> {
    let spec_file = Path::new(spec_path);
    let text = std::fs::read_to_string(spec_file)
        .map_err(|e| Fail::tool(format!("cannot read corpus spec {spec_path}: {e}")))?;
    let spec_dir = spec_file.parent().unwrap_or_else(|| Path::new("."));
    let spec = Spec::parse(&text, spec_dir)?;

    let conventions = load_spec_conventions(&spec)?;
    let report = run_corpus(&conventions, &spec)?;

    match format {
        Format::Json => println!("{}", report.to_json()),
        Format::Human => print!("{}", report.to_human()),
    }

    if report.findings() > 0 {
        return Err(Fail::findings(report.findings_summary()));
    }
    Ok(())
}

// ── spec model + parse ────────────────────────────────────────────────────────

/// One parsed corpus-test spec.
struct Spec {
    name: String,
    /// One or more conventions. Existing specs use the singular frontmatter key;
    /// a `conventions` fence adds peers for trigger-graph proofs.
    conventions: Vec<ConventionRef>,
    /// Whether `md.*` capabilities are admitted only for counterfactual chaining.
    counterfactual: bool,
    /// The corpus root (the 18-02 corpus / governed tree), resolved absolute.
    corpus_root: PathBuf,
    /// The declared rule set — the citations dead-rule detection is measured
    /// against (in declaration order, deduplicated).
    declared_rules: Vec<String>,
    /// The synthetic-change cases, in file order.
    cases: Vec<CaseSpec>,
}

/// Where a spec's convention comes from.
enum ConventionRef {
    /// The embedded throwaway seed convention (`reviewer-not-owner`).
    Seed,
    /// A `conventions/<slug>/` folder on disk, resolved from the spec dir.
    Folder { slug: String, dir: PathBuf },
}

/// One synthetic-change case: a mutation applied to a corpus doc, plus the
/// outcome the run must observe.
#[derive(Deserialize)]
struct CaseSpec {
    /// A label for the report row (defaults to `doc`).
    #[serde(default)]
    name: Option<String>,
    /// The corpus file the change is derived over (mount-confined).
    doc: String,
    /// The acting writer, or absent for an external (out-of-engine) edit.
    #[serde(default)]
    actor: Option<String>,
    /// Whether the write was forced past the checks.
    #[serde(default)]
    force: bool,
    /// Frontmatter keys to set in the AFTER state (scalar values).
    #[serde(default)]
    set: BTreeMap<String, String>,
    /// Frontmatter keys to remove in the AFTER state.
    #[serde(default)]
    remove: Vec<String>,
    /// The rule id(s) the change must fire, or `"pass"` for no fire.
    expect: Expected,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(untagged)]
enum Expected {
    One(String),
    Many(Vec<String>),
}

impl Expected {
    fn rules(&self) -> BTreeSet<String> {
        match self {
            Self::One(value) if value == "pass" => BTreeSet::new(),
            Self::One(value) => BTreeSet::from([value.clone()]),
            Self::Many(values) => values.iter().cloned().collect(),
        }
    }

    fn label(&self) -> String {
        let rules = self.rules();
        if rules.is_empty() {
            "pass".to_owned()
        } else {
            rules.into_iter().collect::<Vec<_>>().join(", ")
        }
    }
}

impl Spec {
    /// Parse a spec's frontmatter + fenced blocks. A missing `convention` /
    /// `corpus`, an unparseable `case` JSON, or a case that declares no `expect`
    /// is a malformed spec (exit 2).
    fn parse(text: &str, spec_dir: &Path) -> Result<Self, Fail> {
        let fm = parse_frontmatter(text);
        let name = fm
            .get("corpus_test")
            .cloned()
            .unwrap_or_else(|| "corpus-test".to_owned());

        let mut conventions = Vec::new();
        if let Some(path) = fm.get("convention").filter(|path| !path.is_empty()) {
            conventions.push(parse_convention_ref(spec_dir, path)?);
        }
        let counterfactual = fm
            .get("counterfactual")
            .is_some_and(|value| value == "true");

        let corpus = fm.get("corpus").filter(|c| !c.is_empty()).ok_or_else(|| {
            Fail::tool("corpus spec frontmatter needs a `corpus:` directory".to_owned())
        })?;
        let corpus_root = resolve_rel(spec_dir, corpus);
        if !corpus_root.is_dir() {
            return Err(Fail::tool(format!(
                "corpus root {} is not a directory",
                corpus_root.display()
            )));
        }

        let mut declared_rules: Vec<String> = Vec::new();
        let mut cases = Vec::new();
        for (info, body) in scan_blocks(text) {
            match info.split_whitespace().next() {
                Some("conventions") => {
                    for line in body.lines() {
                        let path = line.trim();
                        if path.is_empty() || path.starts_with('#') {
                            continue;
                        }
                        conventions.push(parse_convention_ref(spec_dir, path)?);
                    }
                }
                Some("rules") => {
                    for line in body.lines() {
                        let rule = line.trim();
                        if rule.is_empty() || rule.starts_with('#') {
                            continue;
                        }
                        if !declared_rules.iter().any(|r| r == rule) {
                            declared_rules.push(rule.to_owned());
                        }
                    }
                }
                Some("case") => {
                    let case: CaseSpec = serde_json::from_str(&body)
                        .map_err(|e| Fail::tool(format!("```case JSON did not parse: {e}")))?;
                    cases.push(case);
                }
                _ => {}
            }
        }
        if conventions.is_empty() {
            return Err(Fail::tool(
                "corpus spec needs `convention:` or a ```conventions block".to_owned(),
            ));
        }
        if cases.is_empty() {
            return Err(Fail::tool(
                "corpus spec declares no ```case blocks".to_owned(),
            ));
        }
        Ok(Spec {
            name,
            conventions,
            counterfactual,
            corpus_root,
            declared_rules,
            cases,
        })
    }
}

fn parse_convention_ref(base: &Path, value: &str) -> Result<ConventionRef, Fail> {
    if value == "seed" {
        return Ok(ConventionRef::Seed);
    }
    let dir = resolve_rel(base, value);
    let slug = dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| Fail::tool(format!("convention folder {value} has no slug component")))?
        .to_owned();
    Ok(ConventionRef::Folder { slug, dir })
}

/// Resolve `rel` against `base` (absolute `rel` is taken as-is).
fn resolve_rel(base: &Path, rel: &str) -> PathBuf {
    let p = Path::new(rel);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        base.join(p)
    }
}

/// A convention folder on disk — the [`ConventionFiles`] the loader reads through
/// for a `convention: <folder>` spec (the seed uses its own embedded accessor).
struct DirConventionFiles {
    dir: PathBuf,
}

impl ConventionFiles for DirConventionFiles {
    fn read(&self, rel_path: &str) -> std::io::Result<String> {
        std::fs::read_to_string(self.dir.join(rel_path))
    }

    fn exists(&self, rel_path: &str) -> bool {
        self.dir.join(rel_path).exists()
    }
}

/// Load a spec's convention through the loader under the default full limits.
///
/// # Errors
/// [`Fail::tool`] — a convention folder that will not load (a `LoadError`), or a
/// seed convention that no longer satisfies the loader.
fn load_spec_conventions(spec: &Spec) -> Result<Vec<Convention>, Fail> {
    let limits = CheckLimits::default();
    let mut loaded = Vec::with_capacity(spec.conventions.len());
    for convention in &spec.conventions {
        let convention = match convention {
            ConventionRef::Seed => load_seed_convention(limits)
                .map_err(|error| Fail::tool(format!("seed convention failed to load: {error}")))?,
            ConventionRef::Folder { slug, dir } => {
                let files = DirConventionFiles { dir: dir.clone() };
                let result = if spec.counterfactual {
                    load_convention_for_corpus(slug, &files, limits)
                } else {
                    load_convention(slug, &files, limits)
                };
                result.map_err(|error| {
                    Fail::tool(format!("convention `{slug}` failed to load: {error}"))
                })?
            }
        };
        if loaded
            .iter()
            .any(|existing: &Convention| existing.slug() == convention.slug())
        {
            return Err(Fail::tool(format!(
                "corpus spec declares convention `{}` more than once",
                convention.slug()
            )));
        }
        loaded.push(convention);
    }
    Ok(loaded)
}

// ── running the corpus ────────────────────────────────────────────────────────

/// What a case's `check_change` observed.
enum Observed {
    /// The change committed with no refusal (or the doc was out of scope).
    Pass { in_scope: bool },
    /// The change fired: the set of rule ids (`passing` citations) it emitted.
    Fired { rules: BTreeSet<String> },
    /// The convention faulted at eval (budget / runtime / parse) — a finding.
    Error { detail: String },
}

/// One case's finished result.
struct CaseResult {
    name: String,
    doc: String,
    actor: Option<String>,
    in_scope: bool,
    expect: Expected,
    observed: Observed,
    /// One exact metering sample per in-scope capability evaluation.
    fuel: Vec<u64>,
    mem: Vec<u64>,
    /// Admitted HOOK descriptors emitted by the initial synthetic change.
    effects: Vec<Effect>,
    /// The initial case's post-mutation bytes, used only by counterfactual chaining.
    after_md: String,
    /// Named budget findings, kept distinct from structural evaluation faults.
    budget_findings: Vec<String>,
}

impl CaseResult {
    /// Whether the observed outcome exactly matched the declared rule set.
    fn matched(&self) -> bool {
        match &self.observed {
            Observed::Pass { .. } => self.expect.rules().is_empty(),
            Observed::Fired { rules } => *rules == self.expect.rules(),
            Observed::Error { .. } => false,
        }
    }
}

/// Run every case over the corpus and fold into a [`Report`].
///
/// # Errors
/// [`Fail::tool`] — a case names an unreadable / non-UTF-8 corpus doc, or a path
/// that escapes the corpus mount.
fn run_corpus(conventions: &[Convention], spec: &Spec) -> Result<Report, Fail> {
    let mut results = Vec::with_capacity(spec.cases.len());
    for (index, case) in spec.cases.iter().enumerate() {
        results.push(run_case(conventions, spec, index, case)?);
    }

    let quiescence = prove_quiescence(conventions, &results)?;

    let mut fired: BTreeSet<String> = quiescence.fired_rules.clone();
    for result in &results {
        if let Observed::Fired { rules } = &result.observed {
            fired.extend(rules.iter().cloned());
        }
    }
    let declared: BTreeSet<String> = spec.declared_rules.iter().cloned().collect();
    let dead_rules: Vec<String> = spec
        .declared_rules
        .iter()
        .filter(|rule| !fired.contains(*rule))
        .cloned()
        .collect();
    let surprise_rules: Vec<String> = fired.difference(&declared).cloned().collect();

    let mut fuel: Vec<u64> = results
        .iter()
        .flat_map(|result| result.fuel.iter().copied())
        .chain(quiescence.fuel_samples.iter().copied())
        .collect();
    let mut mem: Vec<u64> = results
        .iter()
        .flat_map(|result| result.mem.iter().copied())
        .chain(quiescence.mem_samples.iter().copied())
        .collect();
    fuel.sort_unstable();
    mem.sort_unstable();

    Ok(Report {
        name: spec.name.clone(),
        convention_slugs: conventions
            .iter()
            .map(|convention| convention.slug().to_owned())
            .collect(),
        convention_sources: spec
            .conventions
            .iter()
            .map(convention_source_label)
            .collect(),
        corpus_root: spec.corpus_root.display().to_string(),
        scopes: conventions
            .iter()
            .map(|convention| convention.scope().to_vec())
            .collect(),
        declared_rules: spec.declared_rules.clone(),
        results,
        dead_rules,
        surprise_rules,
        fuel_budget: Budget::of(&fuel),
        mem_budget: Budget::of(&mem),
        quiescence,
    })
}

/// Run one initial synthetic change through every declared CHECK and HOOK.
fn run_case(
    conventions: &[Convention],
    spec: &Spec,
    index: usize,
    case: &CaseSpec,
) -> Result<CaseResult, Fail> {
    let rel = confine(&case.doc)
        .map_err(|message| Fail::tool(format!("case {index} doc {:?}: {message}", case.doc)))?;
    let on_disk = spec.corpus_root.join(&rel);
    let before_md = std::fs::read_to_string(&on_disk).map_err(|error| {
        Fail::tool(format!(
            "case {index}: cannot read corpus doc {}: {error}",
            on_disk.display()
        ))
    })?;
    let after_md = apply_mutation(&before_md, &case.set, &case.remove);
    let doc_path = rel.to_string_lossy().replace('\\', "/");
    let before = build_doc(&doc_path, &before_md);
    let after = build_doc(&doc_path, &after_md);
    let change = synth_change(&before, &after, case);
    let event = derive_event(&change, &before.root.node_rev.0, &after.root.node_rev.0, 0);

    let mut in_scope = false;
    let mut fired = BTreeSet::new();
    let mut effects = Vec::new();
    let mut fuel = Vec::new();
    let mut mem = Vec::new();
    let mut errors = Vec::new();
    let mut budget_findings = Vec::new();

    for convention in conventions {
        if convention.check_source().is_some() && convention.matches_path(&doc_path) {
            in_scope = true;
            match convention.check_change_metered(&change) {
                Ok(telemetry) => {
                    fuel.push(telemetry.fuel_used);
                    mem.push(telemetry.mem_used);
                    fired.extend(
                        telemetry
                            .refusals
                            .into_iter()
                            .map(|refusal| refusal.passing_scenario),
                    );
                }
                Err(error) => errors.push(check_error_label(&error)),
            }
        }

        let Some(hook) = convention.hook() else {
            continue;
        };
        if !hook.matches_path(&doc_path) {
            continue;
        }
        in_scope = true;
        let telemetry = evaluate_hook(convention, &event);
        fuel.push(telemetry.fuel);
        mem.push(telemetry.mem);
        match telemetry.outcome {
            Ok(emitted) => {
                if !emitted.is_empty() {
                    fired.insert(convention.slug().to_owned());
                }
                effects.extend(emitted);
            }
            Err(HookRunError::Budget { steps, mem }) => {
                let finding = format!(
                    "{}: budget_exceeded steps={steps} mem={mem}",
                    convention.slug()
                );
                budget_findings.push(finding.clone());
                errors.push(finding);
            }
            Err(HookRunError::Eval(detail)) => errors.push(format!(
                "{}: HOOK evaluation failed: {detail}",
                convention.slug()
            )),
        }
    }

    let observed = if !errors.is_empty() {
        Observed::Error {
            detail: errors.join("; "),
        }
    } else if fired.is_empty() {
        Observed::Pass { in_scope }
    } else {
        Observed::Fired { rules: fired }
    };

    Ok(CaseResult {
        name: case.name.clone().unwrap_or_else(|| doc_path.clone()),
        doc: doc_path,
        actor: case.actor.clone(),
        in_scope,
        expect: case.expect.clone(),
        observed,
        fuel,
        mem,
        effects,
        after_md,
        budget_findings,
    })
}

struct HookTelemetry {
    fuel: u64,
    mem: u64,
    outcome: Result<Vec<Effect>, HookRunError>,
}

enum HookRunError {
    Budget { steps: u64, mem: u64 },
    Eval(String),
}

fn evaluate_hook(convention: &Convention, event: &effects::ChangeEvent) -> HookTelemetry {
    let hook = convention.hook().expect("caller selected a HOOK");
    let budget = hook.budget();
    let telemetry = effects::eval_telemetry(
        &[Rule::new(convention.slug(), hook.source())],
        event,
        EvalLimits {
            fuel: budget.steps,
            mem: budget.mem,
            // Corpus quiescence must not borrow the runtime cascade suppression as
            // its proof. Counterfactual search has its own explicit fuel below.
            max_depth: u32::MAX,
            ..EvalLimits::default()
        },
    )
    .pop()
    .expect("one rule produces one telemetry row");
    let outcome = match telemetry.outcome {
        Ok(effects) => {
            let caps: CapabilitySet = hook.caps().iter().copied().collect();
            let (admitted, _narrowed) = caps.route(effects);
            Ok(admitted)
        }
        Err(EvalError::Budget { fuel, mem }) => Err(HookRunError::Budget { steps: fuel, mem }),
        Err(error) => Err(HookRunError::Eval(error.to_string())),
    };
    HookTelemetry {
        fuel: telemetry.fuel_used,
        mem: telemetry.mem_used,
        outcome,
    }
}

const QUIESCENCE_FUEL: usize = 256;

struct Quiescence {
    nodes: Vec<String>,
    edges: BTreeSet<(String, String)>,
    steps: usize,
    fuel_limit: usize,
    fuel_exhausted: bool,
    cycle: Option<Vec<String>>,
    fault: Option<String>,
    fired_rules: BTreeSet<String>,
    fuel_samples: Vec<u64>,
    mem_samples: Vec<u64>,
}

impl Quiescence {
    fn passed(&self) -> bool {
        self.cycle.is_none() && !self.fuel_exhausted && self.fault.is_none()
    }

    fn verdict(&self) -> &'static str {
        if self.cycle.is_some() {
            "cycle"
        } else if self.fuel_exhausted {
            "fuel_exhausted"
        } else if self.fault.is_some() {
            "evaluation_fault"
        } else {
            "acyclic"
        }
    }
}

struct PendingEffect {
    emitter: String,
    path: String,
    markdown: String,
    effect: Effect,
    chain: Vec<String>,
}

/// Follow only reachable `md.*` descriptors from the declared synthetic cases.
/// A repeated `(state, pending descriptor)` proves a deterministic cycle that can
/// keep firing. Terminal `proto.*` descriptors add no graph edge, which makes the
/// slice-1 `[proto.send]` verdict explicitly acyclic rather than implicitly skipped.
fn prove_quiescence(
    conventions: &[Convention],
    results: &[CaseResult],
) -> Result<Quiescence, Fail> {
    let nodes = conventions
        .iter()
        .filter(|convention| convention.hook().is_some())
        .map(|convention| convention.slug().to_owned())
        .collect();
    let mut proof = Quiescence {
        nodes,
        edges: BTreeSet::new(),
        steps: 0,
        fuel_limit: QUIESCENCE_FUEL,
        fuel_exhausted: false,
        cycle: None,
        fault: None,
        fired_rules: BTreeSet::new(),
        fuel_samples: Vec::new(),
        mem_samples: Vec::new(),
    };
    let mut queue = initial_counterfactuals(results);
    let mut seen = BTreeSet::new();

    while let Some(pending) = queue.pop_front() {
        if !seen.insert(pending_signature(&pending)) {
            proof.cycle = Some(repeated_cycle(&pending.chain));
            break;
        }
        if proof.steps >= proof.fuel_limit {
            proof.fuel_exhausted = true;
            break;
        }
        proof.steps += 1;
        advance_counterfactual(&pending, conventions, &mut queue, &mut proof)?;
        if proof.fault.is_some() {
            break;
        }
    }
    Ok(proof)
}

fn initial_counterfactuals(results: &[CaseResult]) -> VecDeque<PendingEffect> {
    results
        .iter()
        .flat_map(|result| {
            result
                .effects
                .iter()
                .filter(|effect| effect.kind.domain() == Domain::Md)
                .map(|effect| PendingEffect {
                    emitter: effect.rule_id.clone(),
                    path: result.doc.clone(),
                    markdown: result.after_md.clone(),
                    effect: effect.clone(),
                    chain: vec![effect.rule_id.clone()],
                })
        })
        .collect()
}

fn pending_signature(pending: &PendingEffect) -> String {
    serde_json::to_string(&json!({
        "emitter": pending.emitter,
        "path": pending.path,
        "markdown": pending.markdown,
        "kind": pending.effect.kind.as_str(),
        "args": pending.effect.args,
    }))
    .expect("counterfactual signature serializes")
}

fn advance_counterfactual(
    pending: &PendingEffect,
    conventions: &[Convention],
    queue: &mut VecDeque<PendingEffect>,
    proof: &mut Quiescence,
) -> Result<(), Fail> {
    let Some(after_md) = apply_md_effect(&pending.markdown, &pending.effect)? else {
        return Ok(());
    };
    let before = build_doc(&pending.path, &pending.markdown);
    let after = build_doc(&pending.path, &after_md);
    let synthetic = CaseSpec {
        name: None,
        doc: pending.path.clone(),
        actor: Some(format!("rule:{}", pending.emitter)),
        force: false,
        set: BTreeMap::new(),
        remove: Vec::new(),
        expect: Expected::One("pass".to_owned()),
    };
    let change = synth_change(&before, &after, &synthetic);
    let event = derive_event(
        &change,
        &before.root.node_rev.0,
        &after.root.node_rev.0,
        u32::try_from(pending.chain.len()).unwrap_or(u32::MAX),
    );

    for convention in conventions {
        let Some(hook) = convention.hook() else {
            continue;
        };
        if !hook.matches_path(&pending.path) {
            continue;
        }
        let telemetry = evaluate_hook(convention, &event);
        proof.fuel_samples.push(telemetry.fuel);
        proof.mem_samples.push(telemetry.mem);
        let emitted = match telemetry.outcome {
            Ok(effects) => effects,
            Err(HookRunError::Budget { steps, mem }) => {
                proof.fault = Some(format!(
                    "{}: budget_exceeded steps={steps} mem={mem}",
                    convention.slug()
                ));
                return Ok(());
            }
            Err(HookRunError::Eval(detail)) => {
                proof.fault = Some(format!(
                    "{}: HOOK evaluation failed: {detail}",
                    convention.slug()
                ));
                return Ok(());
            }
        };
        enqueue_emitted(pending, convention, &after_md, emitted, queue, proof);
    }
    Ok(())
}

fn enqueue_emitted(
    pending: &PendingEffect,
    convention: &Convention,
    after_md: &str,
    emitted: Vec<Effect>,
    queue: &mut VecDeque<PendingEffect>,
    proof: &mut Quiescence,
) {
    if emitted.is_empty() {
        return;
    }
    proof
        .edges
        .insert((pending.emitter.clone(), convention.slug().to_owned()));
    proof.fired_rules.insert(convention.slug().to_owned());
    let mut chain = pending.chain.clone();
    chain.push(convention.slug().to_owned());
    for effect in emitted {
        if effect.kind.domain() == Domain::Md {
            queue.push_back(PendingEffect {
                emitter: convention.slug().to_owned(),
                path: pending.path.clone(),
                markdown: after_md.to_owned(),
                effect,
                chain: chain.clone(),
            });
        }
    }
}

fn repeated_cycle(chain: &[String]) -> Vec<String> {
    let Some(last) = chain.last() else {
        return Vec::new();
    };
    let start = chain[..chain.len().saturating_sub(1)]
        .iter()
        .position(|rule| rule == last)
        .unwrap_or(0);
    chain[start..].to_vec()
}

fn apply_md_effect(markdown: &str, effect: &Effect) -> Result<Option<String>, Fail> {
    match effect.kind {
        EffectKind::SetField => {
            let field = effect_arg(effect, "field")?;
            let value = effect_arg(effect, "value")?;
            let after = apply_mutation(
                markdown,
                &BTreeMap::from([(field.to_owned(), value.to_owned())]),
                &[],
            );
            Ok((after != markdown).then_some(after))
        }
        EffectKind::AppendSection => {
            let section = effect_arg(effect, "section")?;
            let content = effect_arg(effect, "content")?;
            let after = append_to_section(markdown, section, content).ok_or_else(|| {
                Fail::tool(format!(
                    "counterfactual effect from `{}` names missing section {section:?}",
                    effect.rule_id
                ))
            })?;
            Ok((after != markdown).then_some(after))
        }
        _ => Ok(None),
    }
}

fn effect_arg<'a>(effect: &'a Effect, name: &str) -> Result<&'a str, Fail> {
    match effect.args.get(name) {
        Some(ArgValue::Str(value)) => Ok(value),
        Some(ArgValue::List(_)) => Err(Fail::tool(format!(
            "counterfactual `{}` argument {name:?} must be a string",
            effect.kind.as_str()
        ))),
        None => Err(Fail::tool(format!(
            "counterfactual `{}` descriptor is missing {name:?}",
            effect.kind.as_str()
        ))),
    }
}

fn append_to_section(markdown: &str, section: &str, content: &str) -> Option<String> {
    let wanted = section.rsplit('/').next().unwrap_or(section);
    let mut start = None;
    let mut end = markdown.len();
    let mut offset = 0usize;
    let mut level = 0usize;
    for line in markdown.split_inclusive('\n') {
        let trimmed = line.trim_end_matches('\n');
        let hashes = trimmed.chars().take_while(|ch| *ch == '#').count();
        let is_heading = hashes > 0 && trimmed.as_bytes().get(hashes) == Some(&b' ');
        if is_heading {
            let heading = trimmed[hashes + 1..].trim();
            if start.is_none() && heading == wanted {
                start = Some(offset + line.len());
                level = hashes;
            } else if start.is_some() && hashes <= level {
                end = offset;
                break;
            }
        }
        offset += line.len();
    }
    start?;
    let mut out = markdown.to_owned();
    let mut insertion = String::new();
    if end > 0 && !out[..end].ends_with('\n') {
        insertion.push('\n');
    }
    insertion.push_str(content);
    if !insertion.ends_with('\n') {
        insertion.push('\n');
    }
    out.insert_str(end, &insertion);
    Some(out)
}

/// Derive the `rulepack-api@2` change for a synthetic frontmatter mutation over
/// the corpus doc — op `splice`, no edits (the change facts come from the
/// before/after STATES), no declared evidence.
fn synth_change(before: &Document, after: &Document, case: &CaseSpec) -> Change {
    derive_change(
        before,
        after,
        &[],
        Invocation {
            op: ChangeOp::Splice,
            actor: case.actor.as_deref(),
            force: case.force,
        },
        &[],
        &|_: &str| None,
    )
}

/// Build a real `model::Document` from markdown, stamping its corpus path (what
/// the disk edge does; `model::build` leaves the path empty).
fn build_doc(path: &str, md: &str) -> Document {
    let nodes = syntax::parse(md);
    let mut doc = model::build(md.to_string(), nodes);
    if let NodeKind::Document { path: p, .. } = &mut doc.root.kind {
        path.clone_into(p);
    }
    doc
}

/// Apply a synthetic frontmatter mutation to `before` — set/replace scalar keys,
/// remove keys — producing the AFTER markdown. A synthetic change, NOT a
/// production write (write fidelity is the tier-1 scenario runner's concern); it
/// only needs a valid AFTER state for the `@2` change derivation. A doc with no
/// leading `---` frontmatter grows one when keys are set.
fn apply_mutation(before: &str, set: &BTreeMap<String, String>, remove: &[String]) -> String {
    let (inner, body) = split_frontmatter(before);
    let mut lines = inner.unwrap_or_default();

    // Replace / drop existing keys.
    let mut handled: BTreeSet<String> = BTreeSet::new();
    lines.retain_mut(|line| {
        let Some((k, _)) = line.split_once(':') else {
            return true; // a continuation / list line — never a scalar key
        };
        let key = k.trim().to_owned();
        if remove.contains(&key) {
            return false;
        }
        if let Some(v) = set.get(&key) {
            *line = format!("{key}: {v}");
            handled.insert(key);
        }
        true
    });
    // Append set keys the block did not already carry (declaration order).
    for (k, v) in set {
        if !handled.contains(k) {
            lines.push(format!("{k}: {v}"));
        }
    }

    if lines.is_empty() {
        // No frontmatter and nothing to set → the body is unchanged.
        return body;
    }
    let mut out = String::from("---\n");
    for line in &lines {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str("---\n");
    out.push_str(&body);
    out
}

/// Split a leading `---\n … \n---` frontmatter block into its inner lines (each
/// stripped of its trailing newline) and the remaining body. Returns `(None,
/// whole)` when the text has no terminated frontmatter block (the ground-truth
/// rule: frontmatter only when bytes 0..3 are `---\n`).
fn split_frontmatter(text: &str) -> (Option<Vec<String>>, String) {
    let Some(after) = text.strip_prefix("---\n") else {
        return (None, text.to_owned());
    };
    let mut inner = Vec::new();
    let mut consumed = 0usize;
    for line in after.split_inclusive('\n') {
        let trimmed = line.strip_suffix('\n').unwrap_or(line);
        if trimmed == "---" {
            let body_start = consumed + line.len();
            return (Some(inner), after[body_start..].to_owned());
        }
        inner.push(trimmed.to_owned());
        consumed += line.len();
    }
    // Unterminated frontmatter — treat the whole text as body.
    (None, text.to_owned())
}

/// A short label for a `check_change` eval fault (for the report).
fn check_error_label(e: &CheckError) -> String {
    e.to_string()
}

/// A label for where the convention came from.
fn convention_source_label(c: &ConventionRef) -> String {
    match c {
        ConventionRef::Seed => "seed".to_owned(),
        ConventionRef::Folder { dir, .. } => dir.display().to_string(),
    }
}

// ── budgets ───────────────────────────────────────────────────────────────────

/// A p50/p99/max budget summary over a metered quantity (fuel ticks or heap
/// bytes) across the in-scope evals.
struct Budget {
    n: usize,
    p50: u64,
    p99: u64,
    max: u64,
}

impl Budget {
    /// Reduce a SORTED (ascending) sample to its p50/p99/max. An empty sample is
    /// all-zero (no in-scope eval spent anything to profile).
    fn of(sorted: &[u64]) -> Self {
        Budget {
            n: sorted.len(),
            p50: percentile(sorted, 50),
            p99: percentile(sorted, 99),
            max: sorted.last().copied().unwrap_or(0),
        }
    }
}

/// Nearest-rank percentile of a SORTED (ascending) sample: `rank = ceil(p * n /
/// 100)` (1-based), clamped into range. Integer arithmetic — no float rounding —
/// so the corpus profile is a deterministic, pure function of `(convention,
/// corpus, spec)`.
fn percentile(sorted: &[u64], p: usize) -> u64 {
    let n = sorted.len();
    if n == 0 {
        return 0;
    }
    let rank = (p * n).div_ceil(100);
    let idx = rank.saturating_sub(1).min(n - 1);
    sorted[idx]
}

// ── report ────────────────────────────────────────────────────────────────────

/// The finished corpus-run report — a pure function of `(convention, corpus,
/// spec)`, so re-running is byte-identical (no wall-clock stamp).
struct Report {
    name: String,
    convention_slugs: Vec<String>,
    convention_sources: Vec<String>,
    corpus_root: String,
    scopes: Vec<Vec<String>>,
    declared_rules: Vec<String>,
    results: Vec<CaseResult>,
    dead_rules: Vec<String>,
    surprise_rules: Vec<String>,
    fuel_budget: Budget,
    mem_budget: Budget,
    quiescence: Quiescence,
}

impl Report {
    /// Cases whose observed outcome did not match `expect`.
    fn mismatches(&self) -> usize {
        self.results.iter().filter(|r| !r.matched()).count()
    }

    /// Cases whose convention faulted at eval.
    fn errored(&self) -> usize {
        self.results
            .iter()
            .filter(|r| matches!(r.observed, Observed::Error { .. }))
            .count()
    }

    /// The number of findings — the exit-1 signal. A run is clean (exit 0) iff
    /// every case matched, no declared rule is dead, and no surprise rule fired.
    fn findings(&self) -> usize {
        self.mismatches()
            + self.dead_rules.len()
            + self.surprise_rules.len()
            + usize::from(!self.quiescence.passed())
    }

    /// A one-line findings summary for the exit-1 diagnostic.
    fn findings_summary(&self) -> String {
        let mut parts = Vec::new();
        let mism = self.mismatches();
        if mism > 0 {
            parts.push(format!("{mism} fire mismatch(es)"));
        }
        if !self.dead_rules.is_empty() {
            parts.push(format!("{} dead rule(s)", self.dead_rules.len()));
        }
        if !self.surprise_rules.is_empty() {
            parts.push(format!("{} surprise rule(s)", self.surprise_rules.len()));
        }
        if !self.quiescence.passed() {
            parts.push(format!("quiescence {}", self.quiescence.verdict()));
        }
        parts.join(", ")
    }

    /// The observed-outcome cell for the report.
    fn observed_cell(r: &CaseResult) -> String {
        match &r.observed {
            Observed::Pass { in_scope: true } => "pass".to_owned(),
            Observed::Pass { in_scope: false } => "pass (out-of-scope)".to_owned(),
            Observed::Fired { rules } => {
                format!(
                    "fired: {}",
                    rules.iter().cloned().collect::<Vec<_>>().join(", ")
                )
            }
            Observed::Error { detail } => format!("ERROR: {detail}"),
        }
    }

    fn to_human(&self) -> String {
        let mut out = String::new();
        self.write_overview(&mut out);
        self.write_cases(&mut out);
        self.write_rule_liveness(&mut out);
        self.write_budgets(&mut out);
        self.write_quiescence(&mut out);
        let matched = self.results.len() - self.mismatches();
        let _ = writeln!(
            out,
            "{} case(s): {matched} matched, {} mismatch(es), {} dead rule(s), {} error(s).",
            self.results.len(),
            self.mismatches(),
            self.dead_rules.len(),
            self.errored(),
        );
        out
    }

    fn write_overview(&self, out: &mut String) {
        let _ = writeln!(out, "# mrd test --corpus — {}\n", self.name);
        for ((slug, source), scope) in self
            .convention_slugs
            .iter()
            .zip(&self.convention_sources)
            .zip(&self.scopes)
        {
            let _ = writeln!(
                out,
                "convention: `{slug}` ({source}) · scope `{}`",
                scope.join("`, `")
            );
        }
        let _ = writeln!(out, "corpus: `{}`", self.corpus_root);
        let in_scope = self.results.iter().filter(|result| result.in_scope).count();
        let _ = writeln!(
            out,
            "cases: {} ({in_scope} in-scope case(s))\n",
            self.results.len()
        );
    }

    fn write_cases(&self, out: &mut String) {
        out.push_str("## Fire-where-expected\n\n");
        out.push_str("| case | doc | actor | expected | observed | ok |\n");
        out.push_str("|------|-----|-------|----------|----------|:--:|\n");
        for result in &self.results {
            let actor = result.actor.as_deref().unwrap_or("(external)");
            let ok = if result.matched() {
                "ok"
            } else {
                "**MISMATCH**"
            };
            let _ = writeln!(
                out,
                "| {} | `{}` | `{actor}` | {} | {} | {ok} |",
                result.name,
                result.doc,
                result.expect.label(),
                Self::observed_cell(result),
            );
        }
        out.push('\n');
    }

    fn write_rule_liveness(&self, out: &mut String) {
        out.push_str("## Dead rules (declared, never fired)\n\n");
        if self.dead_rules.is_empty() {
            out.push_str("_none — every declared rule fired at least once._\n\n");
        } else {
            for id in &self.dead_rules {
                let _ = writeln!(out, "- `{id}`");
            }
            out.push('\n');
        }
        if !self.surprise_rules.is_empty() {
            out.push_str("## Surprise rules (fired, never declared)\n\n");
            for id in &self.surprise_rules {
                let _ = writeln!(out, "- `{id}`");
            }
            out.push('\n');
        }
    }

    fn write_budgets(&self, out: &mut String) {
        out.push_str("## Fuel + heap budgets\n\n");
        let _ = writeln!(out, "over {} in-scope eval(s):\n", self.fuel_budget.n);
        out.push_str("| metric | p50 | p99 | max |\n");
        out.push_str("|--------|----:|----:|----:|\n");
        let _ = writeln!(
            out,
            "| fuel (ticks) | {} | {} | {} |",
            self.fuel_budget.p50, self.fuel_budget.p99, self.fuel_budget.max
        );
        let _ = writeln!(
            out,
            "| heap (bytes) | {} | {} | {} |",
            self.mem_budget.p50, self.mem_budget.p99, self.mem_budget.max
        );
        out.push('\n');
        for finding in self
            .results
            .iter()
            .flat_map(|result| &result.budget_findings)
        {
            let _ = writeln!(out, "- budget finding: `{finding}`");
        }
        out.push('\n');
    }

    fn write_quiescence(&self, out: &mut String) {
        out.push_str("## FIX/HOOK quiescence\n\n");
        let _ = writeln!(
            out,
            "verdict: **{}** · graph nodes={} edges={} · counterfactual steps={}/{}",
            self.quiescence.verdict(),
            self.quiescence.nodes.len(),
            self.quiescence.edges.len(),
            self.quiescence.steps,
            self.quiescence.fuel_limit,
        );
        if self.quiescence.edges.is_empty() {
            out.push_str("\n_none — emitted effects mutate no corpus state._\n");
        } else {
            out.push('\n');
            for (from, to) in &self.quiescence.edges {
                let _ = writeln!(out, "- `{from}` → `{to}`");
            }
        }
        if let Some(cycle) = &self.quiescence.cycle {
            let _ = writeln!(out, "\nquiescence assertion failed: {}", cycle.join(" → "));
        }
        if let Some(fault) = &self.quiescence.fault {
            let _ = writeln!(out, "\nquiescence assertion failed: {fault}");
        }
        if self.quiescence.fuel_exhausted {
            out.push_str("\nquiescence assertion failed: counterfactual fuel exhausted\n");
        }
        out.push('\n');
    }

    fn to_json(&self) -> String {
        let cases: Vec<Value> = self
            .results
            .iter()
            .map(|r| {
                let (outcome, fired, error): (&str, Vec<String>, Option<String>) = match &r.observed
                {
                    Observed::Pass { .. } => ("pass", Vec::new(), None),
                    Observed::Fired { rules } => ("fired", rules.iter().cloned().collect(), None),
                    Observed::Error { detail } => ("error", Vec::new(), Some(detail.clone())),
                };
                json!({
                    "name": r.name,
                    "doc": r.doc,
                    "actor": r.actor,
                    "in_scope": r.in_scope,
                    "expect": r.expect,
                    "outcome": outcome,
                    "fired": fired,
                    "error": error,
                    "matched": r.matched(),
                    "fuel_used": r.fuel.iter().sum::<u64>(),
                    "mem_used": r.mem.iter().max().copied().unwrap_or(0),
                    "budget_findings": r.budget_findings,
                })
            })
            .collect();
        let value = json!({
            "corpus_test": self.name,
            "convention": self.convention_slugs.first(),
            "conventions": self.convention_slugs,
            "convention_source": self.convention_sources.first(),
            "convention_sources": self.convention_sources,
            "corpus_root": self.corpus_root,
            "scope": self.scopes.first(),
            "scopes": self.scopes,
            "declared_rules": self.declared_rules,
            "cases": cases,
            "dead_rules": self.dead_rules,
            "surprise_rules": self.surprise_rules,
            "budgets": {
                "evals": self.fuel_budget.n,
                "fuel": {
                    "p50": self.fuel_budget.p50,
                    "p99": self.fuel_budget.p99,
                    "max": self.fuel_budget.max,
                },
                "heap": {
                    "p50": self.mem_budget.p50,
                    "p99": self.mem_budget.p99,
                    "max": self.mem_budget.max,
                },
            },
            "quiescence": {
                "passed": self.quiescence.passed(),
                "verdict": self.quiescence.verdict(),
                "nodes": self.quiescence.nodes,
                "edges": self.quiescence.edges.iter().map(|(from, to)| json!({
                    "from": from,
                    "to": to,
                })).collect::<Vec<_>>(),
                "steps": self.quiescence.steps,
                "fuel_limit": self.quiescence.fuel_limit,
                "fuel_exhausted": self.quiescence.fuel_exhausted,
                "cycle": self.quiescence.cycle,
                "fault": self.quiescence.fault,
            },
            "summary": {
                "cases": self.results.len(),
                "matched": self.results.len() - self.mismatches(),
                "mismatches": self.mismatches(),
                "dead_rules": self.dead_rules.len(),
                "surprise_rules": self.surprise_rules.len(),
                "errors": self.errored(),
                "quiescence": self.quiescence.verdict(),
                "findings": self.findings(),
            },
        });
        serde_json::to_string_pretty(&value).expect("json")
    }
}

#[cfg(test)]
mod tests {
    use super::{apply_mutation, percentile, split_frontmatter};
    use std::collections::BTreeMap;

    fn set(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn percentile_is_nearest_rank() {
        let s = [10u64, 20, 30, 40, 50];
        assert_eq!(percentile(&s, 50), 30, "median");
        assert_eq!(percentile(&s, 99), 50, "top of the sample");
        assert_eq!(percentile(&[], 50), 0, "empty is zero");
        assert_eq!(percentile(&[7], 50), 7);
        assert_eq!(percentile(&[7], 99), 7);
    }

    #[test]
    fn mutation_replaces_an_existing_scalar_key() {
        let before = "---\nowner: alice\nstatus: open\n---\n# T\n\nbody\n";
        let after = apply_mutation(before, &set(&[("status", "closed")]), &[]);
        assert!(after.contains("status: closed"), "status replaced: {after}");
        assert!(after.contains("owner: alice"), "owner preserved");
        assert!(after.ends_with("# T\n\nbody\n"), "body byte-preserved");
    }

    #[test]
    fn mutation_appends_a_missing_key_and_removes_one() {
        let before = "---\nstatus: open\ndraft: true\n---\nbody\n";
        let after = apply_mutation(before, &set(&[("owner", "agent:x")]), &["draft".to_owned()]);
        assert!(after.contains("owner: agent:x"), "owner appended");
        assert!(!after.contains("draft"), "draft removed: {after}");
        assert!(after.contains("status: open"), "status preserved");
    }

    #[test]
    fn mutation_grows_frontmatter_when_absent() {
        let before = "# Just a heading\n\nbody\n";
        let after = apply_mutation(before, &set(&[("owner", "agent:x")]), &[]);
        assert!(
            after.starts_with("---\nowner: agent:x\n---\n"),
            "fm grown: {after}"
        );
        assert!(
            after.ends_with("# Just a heading\n\nbody\n"),
            "body preserved"
        );
    }

    #[test]
    fn mutation_without_frontmatter_or_sets_is_identity() {
        let before = "# heading\n\nbody\n";
        assert_eq!(apply_mutation(before, &BTreeMap::new(), &[]), before);
    }

    #[test]
    fn split_frontmatter_finds_the_block() {
        let (inner, body) = split_frontmatter("---\na: 1\nb: 2\n---\nrest\n");
        assert_eq!(inner.unwrap(), vec!["a: 1".to_owned(), "b: 2".to_owned()]);
        assert_eq!(body, "rest\n");
        let (none, whole) = split_frontmatter("no frontmatter\n");
        assert!(none.is_none());
        assert_eq!(whole, "no frontmatter\n");
    }
}
