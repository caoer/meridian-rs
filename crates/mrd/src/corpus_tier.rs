//! `mrd test --corpus` — the tier-2 pre-arming runner (U1.5): drive CHECK and
//! HOOK conventions over SYNTHETIC changes derived from a governed corpus.
//!
//! # What a corpus-test spec is
//! A spec is a markdown file with `corpus:` plus one `convention:` or a
//! ` ```conventions ` list. `counterfactual: true` admits `md.*` descriptors in
//! this tier only so quiescence can be falsified without widening runtime caps.
//!
//! - ` ```rules ` — the DECLARED CHECK citations. Every loaded HOOK slug joins the
//!   liveness universe automatically; omitting it from this fence cannot hide a dead HOOK.
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
//! - **zero dead rules** — every CHECK citation in `rules` and every loaded HOOK
//!   must fire at least once over the corpus. A non-firing member is DEAD; a CHECK
//!   citation that fires without declaration is a `surprise` finding.
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

use effects::{ArgValue, Domain, Effect, EffectKind};
use fs::WorkspaceRoot;
use model::{Document, NodeKind};
use policy::ConventionFiles;
use policy::{
    Change, ChangeOp, CheckError, CheckLimits, Convention, CounterfactualConvention, HookEvalError,
    HookFinding, Invocation, derive_change, derive_event,
    evaluate_counterfactual_hooks_for_corpus_metered, evaluate_hooks_for_test_metered,
    load_convention, load_convention_for_corpus, load_seed_convention, seed_convention_files,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wire::{Edit, EditShape, HpathSeg, Path as WirePath, PutAt, SecRef};
use wire_serve::write::{SpliceArgs, splice};

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
    /// Declared CHECK citation ids (in declaration order, deduplicated). Loaded
    /// HOOK slugs enter liveness from the convention set, never from this fence.
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

enum LoadedConventions {
    Production(Vec<Convention>),
    Counterfactual(Vec<CounterfactualConvention>),
}

#[derive(Clone, Copy)]
enum ConventionView<'a> {
    Production(&'a Convention),
    Counterfactual(&'a CounterfactualConvention),
}

impl<'a> ConventionView<'a> {
    fn slug(self) -> &'a str {
        match self {
            Self::Production(convention) => convention.slug(),
            Self::Counterfactual(convention) => convention.slug(),
        }
    }

    fn scope(self) -> &'a [String] {
        match self {
            Self::Production(convention) => convention.scope(),
            Self::Counterfactual(convention) => convention.scope(),
        }
    }

    fn has_check(self) -> bool {
        match self {
            Self::Production(convention) => convention.check_source().is_some(),
            Self::Counterfactual(convention) => convention.has_check(),
        }
    }

    fn matches_path(self, path: &str) -> bool {
        match self {
            Self::Production(convention) => convention.matches_path(path),
            Self::Counterfactual(convention) => convention.matches_path(path),
        }
    }

    fn check_change_metered(self, change: &Change) -> Result<policy::CheckTelemetry, CheckError> {
        match self {
            Self::Production(convention) => convention.check_change_metered(change),
            Self::Counterfactual(convention) => convention.check_change_metered(change),
        }
    }

    fn has_hook(self) -> bool {
        match self {
            Self::Production(convention) => convention.hook().is_some(),
            Self::Counterfactual(convention) => convention.has_hook(),
        }
    }

    fn hook_matches_path(self, path: &str) -> bool {
        match self {
            Self::Production(convention) => convention
                .hook()
                .is_some_and(|hook| hook.matches_path(path)),
            Self::Counterfactual(convention) => convention.hook_matches_path(path),
        }
    }
}

struct HookRun {
    rule_id: String,
    fired: bool,
    effects: Vec<Effect>,
    narrowed: Vec<String>,
    findings: Vec<HookFinding>,
    fuel: u64,
    mem: u64,
}

impl LoadedConventions {
    fn views(&self) -> Vec<ConventionView<'_>> {
        match self {
            Self::Production(conventions) => {
                conventions.iter().map(ConventionView::Production).collect()
            }
            Self::Counterfactual(conventions) => conventions
                .iter()
                .map(ConventionView::Counterfactual)
                .collect(),
        }
    }

    fn evaluate_hooks(&self, event: &effects::ChangeEvent) -> Result<Vec<HookRun>, HookEvalError> {
        match self {
            Self::Production(conventions) => {
                Ok(evaluate_hooks_for_test_metered(conventions, event)?
                    .into_iter()
                    .map(|row| HookRun {
                        rule_id: row.rule_id,
                        fired: !row.outcome.intents.is_empty(),
                        effects: Vec::new(),
                        narrowed: row
                            .outcome
                            .narrowed
                            .iter()
                            .map(|intent| format!("{}:{}", intent.rule_id, intent.action))
                            .collect(),
                        findings: row.outcome.findings,
                        fuel: row.fuel_used,
                        mem: row.mem_used,
                    })
                    .collect())
            }
            Self::Counterfactual(conventions) => Ok(
                evaluate_counterfactual_hooks_for_corpus_metered(conventions, event)?
                    .into_iter()
                    .map(|row| {
                        let fired = !row.effects.is_empty();
                        HookRun {
                            rule_id: row.rule_id,
                            fired,
                            effects: row.effects,
                            narrowed: row
                                .narrowed
                                .iter()
                                .map(|effect| {
                                    format!("{}:{}", effect.rule_id, effect.kind.as_str())
                                })
                                .collect(),
                            findings: row.findings,
                            fuel: row.fuel_used,
                            mem: row.mem_used,
                        }
                    })
                    .collect(),
            ),
        }
    }
}

/// Load a spec's conventions through either the production loader or the opaque
/// counterfactual proof loader. A widened declaration never becomes `Convention`.
///
/// # Errors
/// [`Fail::tool`] — a convention folder that will not load (a `LoadError`), or a
/// seed convention that no longer satisfies the loader.
fn load_spec_conventions(spec: &Spec) -> Result<LoadedConventions, Fail> {
    let limits = CheckLimits::default();
    if spec.counterfactual {
        let mut loaded = Vec::with_capacity(spec.conventions.len());
        for convention in &spec.conventions {
            let convention = match convention {
                ConventionRef::Seed => load_convention_for_corpus(
                    policy::SEED_CONVENTION_SLUG,
                    &seed_convention_files(),
                    limits,
                )
                .map_err(|error| Fail::tool(format!("seed convention failed to load: {error}")))?,
                ConventionRef::Folder { slug, dir } => {
                    let files = DirConventionFiles { dir: dir.clone() };
                    load_convention_for_corpus(slug, &files, limits).map_err(|error| {
                        Fail::tool(format!("convention `{slug}` failed to load: {error}"))
                    })?
                }
            };
            if loaded
                .iter()
                .any(|existing: &CounterfactualConvention| existing.slug() == convention.slug())
            {
                return Err(Fail::tool(format!(
                    "corpus spec declares convention `{}` more than once",
                    convention.slug()
                )));
            }
            loaded.push(convention);
        }
        return Ok(LoadedConventions::Counterfactual(loaded));
    }

    let mut loaded = Vec::with_capacity(spec.conventions.len());
    for convention in &spec.conventions {
        let convention = match convention {
            ConventionRef::Seed => load_seed_convention(limits)
                .map_err(|error| Fail::tool(format!("seed convention failed to load: {error}")))?,
            ConventionRef::Folder { slug, dir } => {
                let files = DirConventionFiles { dir: dir.clone() };
                load_convention(slug, &files, limits).map_err(|error| {
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
    Ok(LoadedConventions::Production(loaded))
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
    /// Complete descriptors denied by the declared capability ceiling.
    narrowed_effects: Vec<String>,
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
fn run_corpus(conventions: &LoadedConventions, spec: &Spec) -> Result<Report, Fail> {
    let views = conventions.views();
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

    // The `rules` fence declares CHECK citation ids only. Every loaded HOOK is a
    // liveness subject whether or not an author remembered to repeat its slug there.
    let mut declared_rules = spec.declared_rules.clone();
    for hook_slug in views
        .iter()
        .copied()
        .filter(|convention| convention.has_hook())
        .map(ConventionView::slug)
    {
        if !declared_rules.iter().any(|rule| rule == hook_slug) {
            declared_rules.push(hook_slug.to_owned());
        }
    }
    let declared: BTreeSet<String> = declared_rules.iter().cloned().collect();
    let dead_rules: Vec<String> = declared_rules
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
        convention_slugs: views
            .iter()
            .copied()
            .map(|convention| convention.slug().to_owned())
            .collect(),
        convention_sources: spec
            .conventions
            .iter()
            .map(convention_source_label)
            .collect(),
        corpus_root: spec.corpus_root.display().to_string(),
        scopes: views
            .iter()
            .copied()
            .map(|convention| convention.scope().to_vec())
            .collect(),
        declared_rules,
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
    conventions: &LoadedConventions,
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
    let doc_path = rel.to_string_lossy().replace('\\', "/");
    let after_md = apply_case_mutation(&doc_path, &before_md, case)?;
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
    let mut narrowed_effects = Vec::new();
    let mut budget_findings = Vec::new();

    for convention in conventions.views() {
        if convention.has_check() && convention.matches_path(&doc_path) {
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
        if convention.has_hook() && convention.hook_matches_path(&doc_path) {
            in_scope = true;
        }
    }

    let hook_rows = conventions
        .evaluate_hooks(&event)
        .map_err(|error| Fail::tool(format!("HOOK evaluation failed: {error}")))?;
    for row in hook_rows {
        fuel.push(row.fuel);
        mem.push(row.mem);
        if row.fired {
            fired.insert(row.rule_id.clone());
        }
        effects.extend(row.effects);
        narrowed_effects.extend(row.narrowed);
        for finding in row.findings {
            match finding {
                HookFinding::BudgetExceeded {
                    rule_id,
                    steps,
                    mem,
                } => {
                    let finding = format!("{rule_id}: budget_exceeded steps={steps} mem={mem}");
                    budget_findings.push(finding.clone());
                    errors.push(finding);
                }
            }
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
        narrowed_effects,
        budget_findings,
    })
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
    /// Signatures on this causal lineage only. A global duplicate is not a cycle.
    ancestors: BTreeSet<String>,
}

/// Follow only reachable `md.*` descriptors from the declared synthetic cases.
/// A signature recurring in its own causal ancestry proves a deterministic cycle.
/// Identical work from another case or a converging branch may be deduplicated after
/// this ancestry check, but it never manufactures a cycle. Terminal `proto.*`
/// descriptors add no graph edge, which keeps slice 1 explicitly acyclic.
fn prove_quiescence(
    conventions: &LoadedConventions,
    results: &[CaseResult],
) -> Result<Quiescence, Fail> {
    let nodes = conventions
        .views()
        .into_iter()
        .filter(|convention| convention.has_hook())
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
    let mut completed = BTreeSet::new();

    while let Some(mut pending) = queue.pop_front() {
        let signature = pending_signature(&pending);
        if pending.ancestors.contains(&signature) {
            proof.cycle = Some(repeated_cycle(&pending.chain));
            break;
        }
        if !completed.insert(signature.clone()) {
            continue;
        }
        if proof.steps >= proof.fuel_limit {
            proof.fuel_exhausted = true;
            break;
        }
        pending.ancestors.insert(signature);
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
                    ancestors: BTreeSet::new(),
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
    conventions: &LoadedConventions,
    queue: &mut VecDeque<PendingEffect>,
    proof: &mut Quiescence,
) -> Result<(), Fail> {
    let Some(after_md) =
        apply_counterfactual_effect(&pending.path, &pending.markdown, &pending.effect)?
    else {
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

    let rows = match conventions.evaluate_hooks(&event) {
        Ok(rows) => rows,
        Err(error) => {
            proof.fault = Some(format!("HOOK evaluation failed: {error}"));
            return Ok(());
        }
    };
    for row in rows {
        proof.fuel_samples.push(row.fuel);
        proof.mem_samples.push(row.mem);
        if let Some(HookFinding::BudgetExceeded {
            rule_id,
            steps,
            mem,
        }) = row.findings.into_iter().next()
        {
            proof.fault = Some(format!(
                "{rule_id}: budget_exceeded steps={steps} mem={mem}"
            ));
            return Ok(());
        }
        enqueue_emitted(pending, &row.rule_id, &after_md, row.effects, queue, proof);
    }
    Ok(())
}

fn enqueue_emitted(
    pending: &PendingEffect,
    rule_id: &str,
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
        .insert((pending.emitter.clone(), rule_id.to_owned()));
    proof.fired_rules.insert(rule_id.to_owned());
    let mut chain = pending.chain.clone();
    chain.push(rule_id.to_owned());
    for effect in emitted {
        if effect.kind.domain() == Domain::Md {
            queue.push_back(PendingEffect {
                emitter: rule_id.to_owned(),
                path: pending.path.clone(),
                markdown: after_md.to_owned(),
                effect,
                chain: chain.clone(),
                ancestors: pending.ancestors.clone(),
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

fn apply_counterfactual_effect(
    path: &str,
    markdown: &str,
    effect: &Effect,
) -> Result<Option<String>, Fail> {
    let edit = match effect.kind {
        EffectKind::SetField => Edit {
            target: SecRef::FmKey {
                fm_key: effect_arg(effect, "field")?.to_owned(),
            },
            edit: EditShape::Put {
                at: PutAt::Upsert,
                text: effect_arg(effect, "value")?.to_owned(),
            },
            if_node_rev: None,
        },
        EffectKind::AppendSection => {
            let section = effect_arg(effect, "section")?;
            let hpath = section
                .split('/')
                .map(|heading| {
                    if heading.is_empty() {
                        return Err(Fail::tool(format!(
                            "counterfactual section path {section:?} contains an empty segment"
                        )));
                    }
                    Ok(HpathSeg {
                        h: heading.to_owned(),
                        n: None,
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            Edit {
                target: SecRef::Hpath { hpath },
                edit: EditShape::Put {
                    at: PutAt::End,
                    text: effect_arg(effect, "content")?.to_owned(),
                },
                if_node_rev: None,
            }
        }
        _ => return Ok(None),
    };
    let actor = format!("rule:{}", effect.rule_id);
    let after = apply_production_edit(path, markdown, Some(&actor), false, edit)?;
    Ok((after != markdown).then_some(after))
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

/// Build a synthetic before/after pair through production semantics. Sets use the
/// live splice writer. Removals use the production model's exact fm-key grain because
/// the wire has no delete-property verb; no Markdown or YAML parser lives here.
fn apply_case_mutation(path: &str, before: &str, case: &CaseSpec) -> Result<String, Fail> {
    let mut after = before.to_owned();
    for field in &case.remove {
        let doc = build_doc(path, &after);
        let target = match model::resolve(&doc, &model::Ref::FmKey(field.clone())) {
            Ok(target) => target,
            Err(model::ResolveError::NotFound) => continue,
            Err(model::ResolveError::Ambiguous(_)) => {
                return Err(Fail::tool(format!(
                    "frontmatter key {field:?} resolved ambiguously"
                )));
            }
        };
        // `remove` describes the synthetic AFTER state, not an effect or a wire
        // operation. The current writer intentionally has no delete-property verb.
        // Use the production model's full fm-key grain (including multiline YAML
        // continuations) and remove exactly that span; no second parser is involved.
        after.replace_range(target.span, "");
    }
    for (field, value) in &case.set {
        after = apply_production_edit(
            path,
            &after,
            case.actor.as_deref(),
            case.force,
            Edit {
                target: SecRef::FmKey {
                    fm_key: field.clone(),
                },
                edit: EditShape::Put {
                    at: PutAt::Upsert,
                    text: value.clone(),
                },
                if_node_rev: None,
            },
        )?;
    }
    Ok(after)
}

fn apply_production_edit(
    path: &str,
    before: &str,
    actor: Option<&str>,
    force: bool,
    edit: Edit,
) -> Result<String, Fail> {
    let dir = tempfile::tempdir()
        .map_err(|error| Fail::tool(format!("cannot create corpus proof tmpdir: {error}")))?;
    let root = WorkspaceRoot(dir.path().to_path_buf());
    let rel = confine(path).map_err(Fail::tool)?;
    let full = dir.path().join(&rel);
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            Fail::tool(format!(
                "cannot create corpus proof parent {}: {error}",
                parent.display()
            ))
        })?;
    }
    std::fs::write(&full, before).map_err(|error| {
        Fail::tool(format!(
            "cannot mount corpus proof document {}: {error}",
            full.display()
        ))
    })?;

    let args = SpliceArgs {
        id: None,
        path: WirePath(path.to_owned()),
        actor: actor.map(str::to_owned),
        now: None,
        receipt: None,
        if_root: None,
        dry: false,
        force,
        edits: vec![edit],
        plan_edits: Vec::new(),
        pin: None,
    };
    splice(&root, 0, &args, &[], None).map_err(|error| {
        Fail::tool(format!(
            "production splice refused counterfactual write to {path}: {:?}: {}",
            error.code,
            error.message.as_deref().unwrap_or_default()
        ))
    })?;
    std::fs::read_to_string(&full).map_err(|error| {
        Fail::tool(format!(
            "cannot read production splice result {}: {error}",
            full.display()
        ))
    })
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
        for narrowed in self
            .results
            .iter()
            .flat_map(|result| &result.narrowed_effects)
        {
            let _ = writeln!(out, "- narrowed effect: `{narrowed}`");
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
                    "narrowed_effects": r.narrowed_effects,
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
    use std::collections::BTreeMap;

    use effects::{ArgValue, Effect, EffectKind, Provenance};

    use super::{apply_counterfactual_effect, percentile};

    fn effect(kind: EffectKind, args: &[(&str, &str)]) -> Effect {
        Effect {
            kind,
            rule_id: "proof-control".to_owned(),
            seq: 0,
            depth: 0,
            provenance: Provenance::Change {
                fingerprint_before: "before".to_owned(),
                fingerprint_after: "after".to_owned(),
            },
            args: args
                .iter()
                .map(|(name, value)| ((*name).to_owned(), ArgValue::Str((*value).to_owned())))
                .collect::<BTreeMap<_, _>>(),
        }
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
    fn counterfactual_set_field_replaces_the_complete_multiline_yaml_node() {
        let before = "---\nlabels:\n  - alpha\n  - beta\nstatus: open\n---\n# Card\n";
        let after = apply_counterfactual_effect(
            "tasks/card.md",
            before,
            &effect(
                EffectKind::SetField,
                &[("field", "labels"), ("value", "done")],
            ),
        )
        .expect("production writer accepts the effect")
        .expect("the effect changes bytes");
        assert_eq!(
            after, "---\nlabels: done\nstatus: open\n---\n# Card\n",
            "production frontmatter node semantics remove continuation lines"
        );
    }

    #[test]
    fn counterfactual_append_uses_the_full_hpath_and_ignores_fenced_headings() {
        let before = "# A\n\n## Notes\n\nleft\n\n```text\n# B\n## Notes\nfake\n```\n\n# B\n\n## Notes\n\nright\n";
        let after = apply_counterfactual_effect(
            "tasks/card.md",
            before,
            &effect(
                EffectKind::AppendSection,
                &[("section", "B/Notes"), ("content", "\nadded\n")],
            ),
        )
        .expect("production writer accepts the effect")
        .expect("the effect changes bytes");
        assert_eq!(
            after,
            "# A\n\n## Notes\n\nleft\n\n```text\n# B\n## Notes\nfake\n```\n\n# B\n\n## Notes\n\nright\n\nadded\n",
            "only the real B/Notes section receives the append"
        );
    }
}
