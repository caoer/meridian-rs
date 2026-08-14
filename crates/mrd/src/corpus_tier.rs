//! `mrd test --corpus` — the tier-2 pre-arming runner: drive CHECK and HOOK rule
//! pages over synthetic changes derived from a governed corpus.
//!
//! # Spec format
//! A markdown file with `corpus:` plus one `rule:` or a ` ```rule-pages ` list.
//! `counterfactual: true` admits `md.*` descriptors in this tier only, so
//! quiescence can be falsified without widening runtime caps.
//!
//! - ` ```rules ` — the declared CHECK citations. Every loaded HOOK's id joins the
//!   liveness universe automatically.
//! - ` ```case ` — one synthetic change as JSON: `{doc, actor?, force?, set?,
//!   remove?, edits?, expect}`. `expect` is one rule id, a list of ids, or `"pass"`.
//!   An unknown key is a malformed spec (exit 2).
//!
//! `edits` is a `wire::Edit` array applied as one batch through the real writer;
//! `set` / `remove` are frontmatter shorthand. Rule pages resolve against the
//! spec's own directory, and a page's frontmatter `id:` is its identity in every
//! row of the report.
//!
//! # The four signals
//! For each case the tier derives the shared [`Change`] from before/after states,
//! then runs every in-scope CHECK and HOOK under its declared budget:
//!
//! - **fire-where-expected** — the set of rules a case fired must equal its
//!   `expect`. A doc outside the rule's `paths:` scope is never run.
//! - **zero dead rules** — every CHECK citation in `rules` and every loaded HOOK
//!   must fire at least once over the corpus; a CHECK citation that fires without
//!   declaration is a `surprise` finding. Liveness is answered per namespace and
//!   reported as `check:<id>` / `hook:<id>`, so a name that is both a citation and
//!   a HOOK slug is two subjects.
//! - **fuel + heap budgets** — exact ticks and peak heap, reduced to
//!   p50/p99/max over all in-scope evaluations.
//! - **FIX/HOOK quiescence** — follow reachable `md.*` descriptors through a
//!   trigger graph. A repeated `(state, pending descriptor)` is a cycle. The proof
//!   has its own fuel and disables runtime depth suppression.
//!
//! A CHECK refusal cites a corpus CASE id, never a page path.
//!
//! # Output + exit codes
//! JSON under `--json`, a human report otherwise. Exit 0 when all four signals are
//! clean; 1 for a mismatch, dead/surprise rule, budget/eval finding, or failed
//! quiescence; 2 for malformed input or unreadable state.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use effects::{ChangeEvent, Domain, Provenance, action_kind};
use fs::WorkspaceRoot;
use model::{Document, MerkleRoot, NodeKind};
use policy::{
    Change, ChangeOp, CheckError, CheckLimits, CounterfactualRule, HookEvalError, HookFinding,
    Intent, Invocation, PageRef, Rule, ScopeLayer, derive_change, derive_event,
    evaluate_counterfactual_hooks_for_corpus_metered, evaluate_hooks_for_test_metered, load_rule,
    load_rule_for_corpus, register_page,
};
use run::caps::{Authority, CapSet};
use run::executor::{ApplyRequest, IntentApply, ReceiptAddr};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wire::{Edit, EditShape, Path as WirePath, PutAt, SecRef};
use wire_serve::write::{SpliceArgs, splice};

use crate::test_cmd::{confine, parse_frontmatter, scan_blocks};
use crate::{Fail, Format};

/// Run `mrd test --corpus <SPEC> [--json]`: load the spec, load its rule pages, run every case
/// over the corpus, and render the report. Errors [`Fail`] — exit 2 (a malformed spec, an
/// unreadable corpus/rule page, or a per-case authoring fault) or exit 1 (a fire mismatch, a
/// dead/surprise rule, or a rule eval fault).
pub(crate) fn run(spec_path: &str, format: Format) -> Result<(), Fail> {
    let spec_file = Path::new(spec_path);
    let text = std::fs::read_to_string(spec_file)
        .map_err(|e| Fail::tool(format!("cannot read corpus spec {spec_path}: {e}")))?;
    let spec_dir = spec_file.parent().unwrap_or_else(|| Path::new("."));
    let spec = Spec::parse(&text, spec_dir)?;

    let rules = load_spec_rules(&spec)?;
    let report = run_corpus(&rules, &spec)?;

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
    /// One or more rule pages: the singular `rule:` key, plus any `rule-pages` peers.
    rules: Vec<RulePageRef>,
    /// Whether `md.*` capabilities are admitted only for counterfactual chaining.
    counterfactual: bool,
    /// The corpus root (the governed tree), resolved absolute.
    corpus_root: PathBuf,
    /// Declared CHECK citation ids (in declaration order, deduplicated). Loaded
    /// HOOK ids enter liveness from the rule set, never from this fence.
    declared_rules: Vec<String>,
    /// The synthetic-change cases, in file order.
    cases: Vec<CaseSpec>,
}

/// Where a spec's rule comes from: one markdown page, resolved from the spec dir.
struct RulePageRef {
    /// The path exactly as the spec spelled it, relative to the spec dir.
    spelled: String,
    /// The resolved on-disk path the bytes are read from.
    path: PathBuf,
}

/// One synthetic-change case: a mutation applied to a corpus doc, plus the outcome the run must
/// observe. Unknown keys are refused rather than dropped.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
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
    /// Frontmatter keys to set in the AFTER state (scalar values) — shorthand for
    /// an `fm_key` upsert in [`CaseSpec::edits`].
    #[serde(default)]
    set: BTreeMap<String, String>,
    /// Frontmatter keys to remove in the AFTER state.
    #[serde(default)]
    remove: Vec<String>,
    /// The write's edits in the production op format, applied as one batch through the real
    /// writer — the half of the change surface `set`/`remove` cannot reach.
    #[serde(default)]
    edits: Vec<Edit>,
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
    /// Parse a spec's frontmatter + fenced blocks. A missing `rule` / `corpus`, an unparseable
    /// `case` JSON, or a case that declares no `expect` is a malformed spec (exit 2).
    fn parse(text: &str, spec_dir: &Path) -> Result<Self, Fail> {
        let fm = parse_frontmatter(text);
        let name = fm
            .get("corpus_test")
            .cloned()
            .unwrap_or_else(|| "corpus-test".to_owned());

        let mut rules = Vec::new();
        if let Some(path) = fm.get("rule").filter(|path| !path.is_empty()) {
            rules.push(rule_page_ref(spec_dir, path));
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
                Some("rule-pages") => {
                    for line in body.lines() {
                        let path = line.trim();
                        if path.is_empty() || path.starts_with('#') {
                            continue;
                        }
                        rules.push(rule_page_ref(spec_dir, path));
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
        if rules.is_empty() {
            return Err(Fail::tool(
                "corpus spec needs `rule:` or a ```rule-pages block".to_owned(),
            ));
        }
        if cases.is_empty() {
            return Err(Fail::tool(
                "corpus spec declares no ```case blocks".to_owned(),
            ));
        }
        Ok(Spec {
            name,
            rules,
            counterfactual,
            corpus_root,
            declared_rules,
            cases,
        })
    }
}

/// One spec-relative rule page reference. Nothing is validated here — identity is the page's own
/// `id:`, which only registration can read.
fn rule_page_ref(base: &Path, value: &str) -> RulePageRef {
    RulePageRef {
        spelled: value.to_owned(),
        path: resolve_rel(base, value),
    }
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

/// A spec's loaded rules, in the one mode the spec declared. The two arms are two loaders, not
/// two evaluators: `counterfactual: true` widens which caps a HOOK declaration may carry.
enum LoadedRules {
    Production(Vec<Rule>),
    Counterfactual(Vec<CounterfactualRule>),
}

#[derive(Clone, Copy)]
enum RuleView<'a> {
    Production(&'a Rule),
    Counterfactual(&'a CounterfactualRule),
}

impl<'a> RuleView<'a> {
    /// The rule's identity — its frontmatter `id:`, the key for report rows,
    /// liveness sets and quiescence nodes.
    fn id(self) -> &'a str {
        match self {
            Self::Production(rule) => rule.id().as_str(),
            Self::Counterfactual(rule) => rule.id(),
        }
    }

    fn scope(self) -> &'a [String] {
        match self {
            Self::Production(rule) => rule.scope(),
            Self::Counterfactual(rule) => rule.scope(),
        }
    }

    fn has_check(self) -> bool {
        match self {
            Self::Production(rule) => rule.check_source().is_some(),
            Self::Counterfactual(rule) => rule.has_check(),
        }
    }

    fn matches_path(self, path: &str) -> bool {
        match self {
            Self::Production(rule) => rule.matches_path(path),
            Self::Counterfactual(rule) => rule.matches_path(path),
        }
    }

    fn check_change_metered(self, change: &Change) -> Result<policy::CheckTelemetry, CheckError> {
        match self {
            Self::Production(rule) => rule.check_change_metered(change),
            Self::Counterfactual(rule) => rule.check_change_metered(change),
        }
    }

    fn has_hook(self) -> bool {
        match self {
            Self::Production(rule) => rule.hook().is_some(),
            Self::Counterfactual(rule) => rule.has_hook(),
        }
    }

    fn hook_matches_path(self, path: &str) -> bool {
        match self {
            Self::Production(rule) => rule.hook().is_some_and(|hook| hook.matches_path(path)),
            Self::Counterfactual(rule) => rule.hook_matches_path(path),
        }
    }
}

/// One HOOK's evaluation over one event, in the same projection for both loader modes: admitted
/// intents, narrowed intents rendered `rule:action`, typed findings, exact meters.
struct HookRun {
    rule_id: String,
    fired: bool,
    intents: Vec<Intent>,
    narrowed: Vec<String>,
    findings: Vec<HookFinding>,
    fuel: u64,
    mem: u64,
}

/// The one narrowed-descriptor spelling, shared by both corpus modes.
fn narrowed_label(intent: &Intent) -> String {
    format!("{}:{}", intent.rule_id, intent.action)
}

/// Project one policy telemetry row into the tier's row shape.
fn hook_run(row: policy::HookTestTelemetry) -> HookRun {
    HookRun {
        rule_id: row.rule_id,
        fired: !row.outcome.intents.is_empty(),
        intents: row.outcome.intents,
        narrowed: row.outcome.narrowed.iter().map(narrowed_label).collect(),
        findings: row.outcome.findings,
        fuel: row.fuel_used,
        mem: row.mem_used,
    }
}

impl LoadedRules {
    fn views(&self) -> Vec<RuleView<'_>> {
        match self {
            Self::Production(rules) => rules.iter().map(RuleView::Production).collect(),
            Self::Counterfactual(rules) => rules.iter().map(RuleView::Counterfactual).collect(),
        }
    }

    fn evaluate_hooks(&self, event: &ChangeEvent) -> Result<Vec<HookRun>, HookEvalError> {
        let rows = match self {
            Self::Production(rules) => evaluate_hooks_for_test_metered(rules, event)?,
            Self::Counterfactual(rules) => {
                evaluate_counterfactual_hooks_for_corpus_metered(rules, event)?
            }
        };
        Ok(rows.into_iter().map(hook_run).collect())
    }
}

/// Read one rule page's bytes, naming the path the spec author wrote.
fn read_rule_page(reference: &RulePageRef) -> Result<String, Fail> {
    std::fs::read_to_string(&reference.path).map_err(|error| {
        Fail::tool(format!(
            "cannot read rule page `{}` ({}): {error}",
            reference.spelled,
            reference.path.display()
        ))
    })
}

/// Register one rule page: does it carry a `rules/*` tag and a legal `id:`.
fn register_rule_page(reference: &RulePageRef, bytes: &str) -> Result<policy::Registration, Fail> {
    register_page(PageRef {
        layer: ScopeLayer::Workspace,
        page: &reference.spelled,
        bytes,
    })
    .map_err(|error| {
        Fail::tool(format!(
            "rule page `{}` is refused: {error}",
            reference.spelled
        ))
    })?
    .ok_or_else(|| {
        Fail::tool(format!(
            "`{}` carries no `rules/*` registration tag — a corpus spec names rule PAGES, \
             and a page registers by tag",
            reference.spelled
        ))
    })
}

/// Load a spec's rule pages through either the production loader or the counterfactual proof
/// loader. A widened declaration never becomes a [`Rule`]. Errors [`Fail::tool`] — an unreadable
/// page, a page that does not register, or a registered page whose declaration does not load.
fn load_spec_rules(spec: &Spec) -> Result<LoadedRules, Fail> {
    let limits = CheckLimits::default();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    // Duplicate check is on the resolved `id:`, not the path: one law must not run — and meter
    // — twice.
    let mut guard = |id: &str, spelled: &str| -> Result<(), Fail> {
        if !seen.insert(id.to_owned()) {
            return Err(Fail::tool(format!(
                "corpus spec declares rule `{id}` more than once (`{spelled}`)"
            )));
        }
        Ok(())
    };

    if spec.counterfactual {
        let mut loaded = Vec::with_capacity(spec.rules.len());
        for reference in &spec.rules {
            let bytes = read_rule_page(reference)?;
            let registration = register_rule_page(reference, &bytes)?;
            let rule = load_rule_for_corpus(&registration, &bytes, limits).map_err(|error| {
                Fail::tool(format!(
                    "rule page `{}` failed to load: {error}",
                    reference.spelled
                ))
            })?;
            guard(rule.id(), &reference.spelled)?;
            loaded.push(rule);
        }
        return Ok(LoadedRules::Counterfactual(loaded));
    }

    let mut loaded = Vec::with_capacity(spec.rules.len());
    for reference in &spec.rules {
        let bytes = read_rule_page(reference)?;
        let registration = register_rule_page(reference, &bytes)?;
        let rule = load_rule(&registration, &bytes, limits).map_err(|error| {
            Fail::tool(format!(
                "rule page `{}` failed to load: {error}",
                reference.spelled
            ))
        })?;
        guard(rule.id().as_str(), &reference.spelled)?;
        loaded.push(rule);
    }
    Ok(LoadedRules::Production(loaded))
}

// ── running the corpus ────────────────────────────────────────────────────────

/// What a case's `check_change` observed.
enum Observed {
    /// The change committed with no refusal (or the doc was out of scope).
    Pass { in_scope: bool },
    /// The change fired: the set of rule ids (`passing` citations) it emitted.
    Fired { rules: BTreeSet<String> },
    /// The rule faulted at eval (budget / runtime / parse) — a finding.
    Error { detail: String },
}

/// One HOOK's `md.*` emission over one event — the generation the production
/// executor applies as one atomic batch, synthesizing exactly one follow-on event.
struct Emission {
    emitter: String,
    intents: Vec<Intent>,
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
    /// CHECK citation ids this case fired. Kept in its own namespace: a citation id equal to a
    /// HOOK's id must never make that HOOK look live.
    fired_checks: BTreeSet<String>,
    /// HOOK ids this case fired.
    fired_hooks: BTreeSet<String>,
    /// The `md.*` generations the initial synthetic change armed, per emitter.
    emissions: Vec<Emission>,
    /// The initial case's post-mutation bytes, used only by counterfactual chaining.
    after_md: String,
    /// The landed change's plane facts, carried through to the adapted descriptors — a
    /// `policy::Intent` records no provenance of its own.
    provenance: Provenance,
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

/// One liveness subject: a declared name in one namespace. A name that exists in both is two
/// subjects, each answered only by its own leg firing.
struct Subject<'a> {
    namespace: Namespace,
    id: &'a str,
}

/// The two liveness namespaces.
#[derive(Clone, Copy)]
enum Namespace {
    /// Declared by the ```rules``` fence; answered only by a CHECK refusal citing it.
    Check,
    /// Declared by being loaded; answered only by that HOOK emitting.
    Hook,
}

impl<'a> Subject<'a> {
    fn check(id: &'a str) -> Self {
        Self {
            namespace: Namespace::Check,
            id,
        }
    }

    fn hook(id: &'a str) -> Self {
        Self {
            namespace: Namespace::Hook,
            id,
        }
    }

    /// The `kind:id` spelling the report uses for a liveness subject.
    fn label(&self) -> String {
        let namespace = match self.namespace {
            Namespace::Check => "check",
            Namespace::Hook => "hook",
        };
        format!("{namespace}:{}", self.id)
    }

    /// Whether this subject was answered, by its own namespace's fired set.
    fn fired(&self, checks: &BTreeSet<String>, hooks: &BTreeSet<String>) -> bool {
        match self.namespace {
            Namespace::Check => checks.contains(self.id),
            Namespace::Hook => hooks.contains(self.id),
        }
    }
}

/// Run every case over the corpus and fold into a [`Report`]. Errors [`Fail::tool`] — a case
/// names an unreadable / non-UTF-8 corpus doc, a path that escapes the corpus mount, or a
/// quiescence proof whose own workspace failed.
fn run_corpus(rules: &LoadedRules, spec: &Spec) -> Result<Report, Fail> {
    let views = rules.views();
    let mut results = Vec::with_capacity(spec.cases.len());
    for (index, case) in spec.cases.iter().enumerate() {
        results.push(run_case(rules, spec, index, case)?);
    }

    let quiescence = prove_quiescence(rules, &results)?;

    // Two namespaces, never one set: a merged set would let whichever leg is live vouch for the
    // other's silence.
    let mut fired_checks: BTreeSet<String> = BTreeSet::new();
    let mut fired_hooks: BTreeSet<String> = quiescence.fired_hooks.clone();
    for result in &results {
        fired_checks.extend(result.fired_checks.iter().cloned());
        fired_hooks.extend(result.fired_hooks.iter().cloned());
    }

    // Every loaded HOOK is a liveness subject, whether or not the fence repeats its id.
    let declared_hooks: Vec<String> = views
        .iter()
        .copied()
        .filter(|rule| rule.has_hook())
        .map(|rule| rule.id().to_owned())
        .collect();
    // A fence name is a CHECK citation subject — except in a spec that loads no CHECK, where a
    // fence entry naming a loaded HOOK repeats that HOOK's declaration.
    let citable = views.iter().copied().any(RuleView::has_check);
    let declared_checks: Vec<&String> = spec
        .declared_rules
        .iter()
        .filter(|id| citable || !declared_hooks.iter().any(|hook| hook == *id))
        .collect();
    let subjects: Vec<Subject<'_>> = declared_checks
        .iter()
        .map(|id| Subject::check(id))
        .chain(declared_hooks.iter().map(|id| Subject::hook(id)))
        .collect();
    let declared_rules: Vec<String> = subjects.iter().map(Subject::label).collect();
    let dead_rules: Vec<String> = subjects
        .iter()
        .filter(|subject| !subject.fired(&fired_checks, &fired_hooks))
        .map(Subject::label)
        .collect();
    // Surprise is a CHECK-only signal: a HOOK is declared by being loaded, so it can never fire
    // undeclared.
    let surprise_rules: Vec<String> = fired_checks
        .iter()
        .filter(|id| !declared_checks.contains(id))
        .map(|id| Subject::check(id).label())
        .collect();

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
        rule_ids: views
            .iter()
            .copied()
            .map(|rule| rule.id().to_owned())
            .collect(),
        rule_sources: spec
            .rules
            .iter()
            .map(|reference| reference.spelled.clone())
            .collect(),
        corpus_root: spec.corpus_root.display().to_string(),
        scopes: views
            .iter()
            .copied()
            .map(|rule| rule.scope().to_vec())
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
    rules: &LoadedRules,
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

    let mut fold = CaseFold::default();
    fold.run_checks(rules, &doc_path, &change);
    fold.run_hooks(
        rules
            .evaluate_hooks(&event)
            .map_err(|error| Fail::tool(format!("HOOK evaluation failed: {error}")))?,
    );

    Ok(CaseResult {
        name: case.name.clone().unwrap_or_else(|| doc_path.clone()),
        doc: doc_path,
        actor: case.actor.clone(),
        in_scope: fold.in_scope,
        expect: case.expect.clone(),
        observed: fold.observed(),
        fuel: fold.fuel,
        mem: fold.mem,
        fired_checks: fold.fired_checks,
        fired_hooks: fold.fired_hooks,
        emissions: fold.emissions,
        after_md,
        provenance: Provenance::Change {
            fingerprint_before: event.fingerprint_before.clone(),
            fingerprint_after: event.fingerprint_after.clone(),
        },
        narrowed_effects: fold.narrowed_effects,
        budget_findings: fold.budget_findings,
    })
}

/// What one case accumulates across its two legs. The two fired sets stay separate
/// for liveness; only the observed-outcome signal reads their union.
#[derive(Default)]
struct CaseFold {
    in_scope: bool,
    fired_checks: BTreeSet<String>,
    fired_hooks: BTreeSet<String>,
    emissions: Vec<Emission>,
    fuel: Vec<u64>,
    mem: Vec<u64>,
    errors: Vec<String>,
    narrowed_effects: Vec<String>,
    budget_findings: Vec<String>,
}

impl CaseFold {
    /// The law leg: every in-scope CHECK over this change, metered.
    fn run_checks(&mut self, rules: &LoadedRules, doc_path: &str, change: &Change) {
        for rule in rules.views() {
            if rule.has_check() && rule.matches_path(doc_path) {
                self.in_scope = true;
                match rule.check_change_metered(change) {
                    Ok(telemetry) => {
                        self.fuel.push(telemetry.fuel_used);
                        self.mem.push(telemetry.mem_used);
                        self.fired_checks.extend(
                            telemetry
                                .refusals
                                .into_iter()
                                .map(|refusal| refusal.passing_scenario),
                        );
                    }
                    Err(error) => self.errors.push(check_error_label(&error)),
                }
            }
            if rule.has_hook() && rule.hook_matches_path(doc_path) {
                self.in_scope = true;
            }
        }
    }

    /// The emit leg: one row per in-scope HOOK, each row's `md.*` intents kept whole
    /// as one generation.
    fn run_hooks(&mut self, rows: Vec<HookRun>) {
        for row in rows {
            self.fuel.push(row.fuel);
            self.mem.push(row.mem);
            if row.fired {
                self.fired_hooks.insert(row.rule_id.clone());
            }
            let generation = md_intents(row.intents);
            if !generation.is_empty() {
                self.emissions.push(Emission {
                    emitter: row.rule_id.clone(),
                    intents: generation,
                });
            }
            self.narrowed_effects.extend(row.narrowed);
            for finding in row.findings {
                let HookFinding::BudgetExceeded {
                    rule_id,
                    steps,
                    mem,
                } = finding;
                let finding = format!("{rule_id}: budget_exceeded steps={steps} mem={mem}");
                self.budget_findings.push(finding.clone());
                self.errors.push(finding);
            }
        }
    }

    /// `expect` names a rule by id whichever leg raised it, so the fire signal reads the union;
    /// liveness does not.
    fn observed(&self) -> Observed {
        if !self.errors.is_empty() {
            return Observed::Error {
                detail: self.errors.join("; "),
            };
        }
        let fired: BTreeSet<String> = self
            .fired_checks
            .union(&self.fired_hooks)
            .cloned()
            .collect();
        if fired.is_empty() {
            Observed::Pass {
                in_scope: self.in_scope,
            }
        } else {
            Observed::Fired { rules: fired }
        }
    }
}

/// The `md.*` half of one emission — the only intents the Markdown adapter carries. `proto.*`
/// intents mutate no corpus state, so they add no trigger-graph edge.
fn md_intents(intents: Vec<Intent>) -> Vec<Intent> {
    intents
        .into_iter()
        .filter(|intent| action_kind(&intent.action).is_some_and(|k| k.domain() == Domain::Md))
        .collect()
}

const QUIESCENCE_FUEL: usize = 256;

/// Where the isolated proof corpus lands its executor receipts: never the live tree, and never
/// the triggering write's own page.
const PROOF_RECEIPT_PATH: &str = "receipts/corpus-proof.md";

/// The task name the proof's receipts are actored by (`run:<task>`).
const PROOF_TASK: &str = "corpus-proof";

/// The procedure-hash the proof's receipts attest. Fixed: a wall-clock or random value would
/// break the report's byte-identical re-run law.
const PROOF_TASK_REV: &str = "b3:corpus-proof";

struct Quiescence {
    nodes: Vec<String>,
    edges: BTreeSet<(String, String)>,
    steps: usize,
    fuel_limit: usize,
    fuel_exhausted: bool,
    cycle: Option<Vec<String>>,
    fault: Option<String>,
    fired_hooks: BTreeSet<String>,
    fuel_samples: Vec<u64>,
    mem_samples: Vec<u64>,
    /// Complete descriptors the declared capability ceiling denied during the cascade; these
    /// belong to no case.
    narrowed_effects: Vec<String>,
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

/// One `md.*` generation waiting to be executed against the state it reacts to.
///
/// The unit is the generation, never a single descriptor: production applies one
/// emission as one atomic batch and synthesizes exactly one follow-on event.
struct PendingGeneration {
    emitter: String,
    path: String,
    markdown: String,
    /// The emission's `md.*` intents, in emission order.
    intents: Vec<Intent>,
    /// The emitting event's plane facts, carried through to the descriptors.
    provenance: Provenance,
    /// The applied generation's cascade depth.
    depth: u32,
    chain: Vec<String>,
}

/// One frame of the proof's depth-first walk: the generation being explored, and the
/// successors it has not been walked into yet.
struct Frame {
    signature: String,
    successors: VecDeque<PendingGeneration>,
}

/// Follow only reachable `md.*` generations from the declared synthetic cases. Each generation
/// is executed through the PRODUCTION batch executor against an isolated proof corpus, and the
/// executor's own single synthesized event is what the next round of HOOKs reads.
///
/// The walk is depth-first over the signature graph — a generation's successors are a pure
/// function of its signature — never a fan-out over causal paths, which is `O(b^d)` over an
/// `O(d)` state space and false-fails strictly terminating convention sets.
fn prove_quiescence(rules: &LoadedRules, results: &[CaseResult]) -> Result<Quiescence, Fail> {
    let nodes = rules
        .views()
        .into_iter()
        .filter(|rule| rule.has_hook())
        .map(|rule| rule.id().to_owned())
        .collect();
    let mut proof = Quiescence {
        nodes,
        edges: BTreeSet::new(),
        steps: 0,
        fuel_limit: QUIESCENCE_FUEL,
        fuel_exhausted: false,
        cycle: None,
        fault: None,
        fired_hooks: BTreeSet::new(),
        fuel_samples: Vec::new(),
        mem_samples: Vec::new(),
        narrowed_effects: Vec::new(),
    };
    let mut roots = initial_generations(results);
    let mut on_path: BTreeSet<String> = BTreeSet::new();
    let mut settled: BTreeSet<String> = BTreeSet::new();
    let mut stack: Vec<Frame> = Vec::new();

    loop {
        let next = match stack.last_mut() {
            Some(frame) => frame.successors.pop_front(),
            None => roots.pop_front(),
        };
        let Some(pending) = next else {
            // The top frame is exhausted, so its whole reachable subgraph is: settle it and leave
            // the path. An empty stack with no roots left ends the walk.
            let Some(done) = stack.pop() else { break };
            on_path.remove(&done.signature);
            settled.insert(done.signature);
            continue;
        };
        if !descend(
            &pending,
            rules,
            &mut stack,
            &mut on_path,
            &settled,
            &mut proof,
        )? {
            break;
        }
    }
    Ok(proof)
}

/// Walk into one generation, or decline to. Returns whether the walk continues: `false` ends it
/// with a cycle, an exhausted budget, or an evaluation fault already recorded on `proof`.
/// Errors [`Fail::tool`] — the proof workspace failed, which is not a fact about the
/// conventions under test.
fn descend(
    pending: &PendingGeneration,
    rules: &LoadedRules,
    stack: &mut Vec<Frame>,
    on_path: &mut BTreeSet<String>,
    settled: &BTreeSet<String>,
    proof: &mut Quiescence,
) -> Result<bool, Fail> {
    let signature = pending_signature(pending);
    if on_path.contains(&signature) {
        proof.cycle = Some(repeated_cycle(&pending.chain));
        return Ok(false);
    }
    if settled.contains(&signature) {
        return Ok(true);
    }
    if proof.steps >= proof.fuel_limit {
        proof.fuel_exhausted = true;
        return Ok(false);
    }
    proof.steps += 1;
    let successors = advance_generation(pending, rules, proof)?;
    if proof.fault.is_some() {
        return Ok(false);
    }
    on_path.insert(signature.clone());
    stack.push(Frame {
        signature,
        successors,
    });
    Ok(true)
}

fn initial_generations(results: &[CaseResult]) -> VecDeque<PendingGeneration> {
    results
        .iter()
        .flat_map(|result| {
            result.emissions.iter().map(|emission| PendingGeneration {
                emitter: emission.emitter.clone(),
                path: result.doc.clone(),
                markdown: result.after_md.clone(),
                intents: emission.intents.clone(),
                provenance: result.provenance.clone(),
                depth: 0,
                chain: vec![emission.emitter.clone()],
            })
        })
        .collect()
}

fn pending_signature(pending: &PendingGeneration) -> String {
    serde_json::to_string(&json!({
        "emitter": pending.emitter,
        "path": pending.path,
        "markdown": pending.markdown,
        "intents": pending.intents,
    }))
    .expect("counterfactual signature serializes")
}

/// Apply one generation and evaluate the reactions to it, returning the successors it armed.
/// Edges, fired HOOKs and meters are recorded here whether or not the caller goes on to walk
/// into any of them, so the reported graph is the whole reachable one.
fn advance_generation(
    pending: &PendingGeneration,
    rules: &LoadedRules,
    proof: &mut Quiescence,
) -> Result<VecDeque<PendingGeneration>, Fail> {
    let mut successors = VecDeque::new();
    let applied = match apply_generation(pending) {
        Ok(applied) => applied,
        Err(ProofFault::Refused(refusal)) => {
            // Production refuses this reaction: a fact about the proof's subject, not a harness
            // crash.
            proof.fault = Some(refusal);
            return Ok(successors);
        }
        Err(ProofFault::Workspace(fault)) => return Err(Fail::tool(fault)),
    };
    // A generation that changed no bytes synthesizes no event: the branch is terminal.
    let Some((after_md, event)) = applied else {
        return Ok(successors);
    };

    let rows = match rules.evaluate_hooks(&event) {
        Ok(rows) => rows,
        Err(error) => {
            proof.fault = Some(format!("HOOK evaluation failed: {error}"));
            return Ok(successors);
        }
    };
    for row in rows {
        proof.fuel_samples.push(row.fuel);
        proof.mem_samples.push(row.mem);
        proof.narrowed_effects.extend(row.narrowed);
        if let Some(HookFinding::BudgetExceeded {
            rule_id,
            steps,
            mem,
        }) = row.findings.into_iter().next()
        {
            proof.fault = Some(format!(
                "{rule_id}: budget_exceeded steps={steps} mem={mem}"
            ));
            return Ok(successors);
        }
        if let Some(next) =
            emitted_generation(pending, &row.rule_id, &after_md, &event, row.intents, proof)
        {
            successors.push_back(next);
        }
    }
    Ok(successors)
}

/// Record the trigger edge one reaction produced, and return the generation it armed — `None`
/// when the reaction emitted nothing, or nothing that mutates corpus state.
fn emitted_generation(
    pending: &PendingGeneration,
    rule_id: &str,
    after_md: &str,
    event: &ChangeEvent,
    emitted: Vec<Intent>,
    proof: &mut Quiescence,
) -> Option<PendingGeneration> {
    if emitted.is_empty() {
        return None;
    }
    proof
        .edges
        .insert((pending.emitter.clone(), rule_id.to_owned()));
    proof.fired_hooks.insert(rule_id.to_owned());
    let intents = md_intents(emitted);
    if intents.is_empty() {
        return None;
    }
    let mut chain = pending.chain.clone();
    chain.push(rule_id.to_owned());
    Some(PendingGeneration {
        emitter: rule_id.to_owned(),
        path: pending.path.clone(),
        markdown: after_md.to_owned(),
        intents,
        provenance: Provenance::Change {
            fingerprint_before: event.fingerprint_before.clone(),
            fingerprint_after: event.fingerprint_after.clone(),
        },
        depth: event.depth,
        chain,
    })
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

/// Why one emitted generation did not apply. The split is the module's exit law: exit 1 says
/// the conventions under test have a quiescence defect, exit 2 says the tool could not read or
/// write its own state.
#[derive(Debug)]
enum ProofFault {
    /// The production executor refused the reaction — an adapter fault, a cap denial,
    /// or any [`run::executor::ExecError`]. A fact about the subject: exit 1.
    Refused(String),
    /// The proof's own throwaway workspace failed: exit 2.
    Workspace(String),
}

/// Execute one emitted generation against an isolated proof corpus through the
/// production path: `policy::Intent` → the `run` executor adapter → the atomic batch
/// executor.
///
/// The workspace is a throwaway tmpdir carrying only the reacted-to page, so the
/// governed corpus tree is read-only. Receipts land in the isolated corpus — never
/// the triggering write's own page.
fn apply_generation(
    pending: &PendingGeneration,
) -> Result<Option<(String, ChangeEvent)>, ProofFault> {
    let dir = tempfile::tempdir()
        .map_err(|error| ProofFault::Workspace(format!("cannot create proof tmpdir: {error}")))?;
    let root = WorkspaceRoot(dir.path().to_path_buf());
    let rel = confine(&pending.path).map_err(ProofFault::Refused)?;
    let full = dir.path().join(&rel);
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            ProofFault::Workspace(format!("cannot create proof corpus parent: {error}"))
        })?;
    }
    std::fs::write(&full, &pending.markdown).map_err(|error| {
        ProofFault::Workspace(format!("cannot mount proof corpus document: {error}"))
    })?;

    let receipt = proof_receipt_addr(&pending.intents).map_err(ProofFault::Refused)?;
    let adapted = IntentApply::from_intents(
        &pending.intents,
        receipt,
        &pending.provenance,
        pending.depth,
    )
    .map_err(|error| ProofFault::Refused(error.to_string()))?;
    // A fixed allowlist of the two `md.*` actions the adapter carries: the executor's ceiling,
    // not the declaration's (`caps.route` already narrowed the emission on the policy side).
    let authority = Authority::granted(
        CapSet::parse("md.set_field md.append_section")
            .map_err(|error| ProofFault::Workspace(format!("proof corpus caps: {error}")))?,
    );
    let live: MerkleRoot = fs::domain_snapshot(&root)
        .map_err(|error| ProofFault::Workspace(format!("proof workspace fold: {error}")))?
        .1;
    let invocation = format!("corpus-proof-{}", pending.chain.join("-"));
    let request = adapted.request(&ApplyRequest {
        page: &pending.path,
        task: PROOF_TASK,
        task_rev: PROOF_TASK_REV,
        invocation_id: &invocation,
        now: None,
        effects: &[],
        authority: &authority,
        pin_root: &live,
        live_root: &live,
        receipt: None,
        takeover: false,
        exec: None,
        actor: None,
        depth: pending.depth,
        delta: None, // CLI host: no ring in reach (§18 row 12)
    });
    let applied = run::executor::apply(&root, &request).map_err(|error| {
        ProofFault::Refused(format!(
            "production executor refused the emitted generation from `{}`: {error}",
            pending.emitter
        ))
    })?;
    let Some(event) = applied.event else {
        return Ok(None);
    };
    let after_md = std::fs::read_to_string(&full).map_err(|error| {
        ProofFault::Workspace(format!("cannot read the applied proof corpus: {error}"))
    })?;
    Ok(Some((after_md, event)))
}

/// The receipt address the proof's batch rides with: the canonical anchor the intents already
/// carry, landed in the isolated corpus's own receipt file.
fn proof_receipt_addr(intents: &[Intent]) -> Result<ReceiptAddr, String> {
    let canonical = intents
        .first()
        .ok_or_else(|| "an emitted generation carries no intent".to_owned())?
        .receipt
        .as_str();
    let anchor = canonical
        .rsplit_once("#^")
        .map(|(_, anchor)| anchor.to_owned())
        .ok_or_else(|| {
            format!("intent receipt {canonical:?} is not a canonical `path#^anchor` address")
        })?;
    Ok(ReceiptAddr {
        path: PROOF_RECEIPT_PATH.to_owned(),
        anchor,
    })
}

/// Build a synthetic before/after pair through production semantics, in the one order a case is
/// read in: `remove`, then `set`, then `edits`. Sets use the live splice writer, one key at a
/// time.
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
        // `remove` describes the synthetic AFTER state — the writer has no delete-property verb.
        // The production model's fm-key grain removes the whole multiline YAML value.
        after.replace_range(target.span, "");
    }
    for (field, value) in &case.set {
        after = apply_production_edit(
            path,
            &after,
            case.actor.as_deref(),
            case.force,
            vec![Edit {
                target: SecRef::FmKey {
                    fm_key: field.clone(),
                },
                edit: EditShape::Put {
                    at: PutAt::Upsert,
                    text: value.clone(),
                },
                if_node_rev: None,
            }],
        )?;
    }
    if !case.edits.is_empty() {
        after = apply_production_edit(
            path,
            &after,
            case.actor.as_deref(),
            case.force,
            case.edits.clone(),
        )?;
    }
    Ok(after)
}

fn apply_production_edit(
    path: &str,
    before: &str,
    actor: Option<&str>,
    force: bool,
    edits: Vec<Edit>,
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
        origin: wire_serve::guard::Origin::InProcess,
        path: WirePath(path.to_owned()),
        actor: actor.map(str::to_owned),
        now: None,
        receipt: None,
        if_root: None,
        dry: false,
        force,
        edits,
        plan_edits: Vec::new(),
        pin: None,
    };
    // Exit 2, not the findings leg: the SPEC declared an edit the engine will not perform, which
    // is a bad input to this harness rather than the engine refusing a caller's request. The
    // sentence still comes from the one owner — rendering the body here spelled the code
    // `Debug`-style and dropped every message-less refusal to an empty string.
    splice(&root, None, &args, &[], None).map_err(|error| {
        Fail::tool(format!(
            "production splice refused counterfactual write to {path}: {}",
            crate::engine::refusal_text(&error)
        ))
    })?;
    std::fs::read_to_string(&full).map_err(|error| {
        Fail::tool(format!(
            "cannot read production splice result {}: {error}",
            full.display()
        ))
    })
}

/// Derive the `rulepack-api@2` change for a synthetic frontmatter mutation over the corpus doc
/// — op `splice`, no edits (the change facts come from the before/after states), no declared
/// evidence.
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

/// Nearest-rank percentile of a sorted (ascending) sample: `rank = ceil(p * n / 100)`
/// (1-based), clamped into range. Integer arithmetic — no float rounding — keeps the profile a
/// deterministic function of its inputs.
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

/// The finished corpus-run report — a pure function of `(rule pages, corpus,
/// spec)`, so re-running is byte-identical (no wall-clock stamp).
struct Report {
    name: String,
    /// Each loaded rule's `id:`, in spec order.
    rule_ids: Vec<String>,
    /// Each rule page as the spec spelled it. Kept spec-relative rather than absolute so the
    /// report stays a pure function of its inputs on any machine.
    rule_sources: Vec<String>,
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

    /// Cases whose rule faulted at eval.
    fn errored(&self) -> usize {
        self.results
            .iter()
            .filter(|r| matches!(r.observed, Observed::Error { .. }))
            .count()
    }

    /// The number of findings — the exit-1 signal. A run is clean (exit 0) iff every case matched,
    /// no declared rule is dead, and no surprise rule fired.
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
        for ((id, source), scope) in self
            .rule_ids
            .iter()
            .zip(&self.rule_sources)
            .zip(&self.scopes)
        {
            let _ = writeln!(
                out,
                "rule: `{id}` ({source}) · scope `{}`",
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
        // Every denial, whichever phase raised it: a case's own, then the cascade's.
        for narrowed in self
            .results
            .iter()
            .flat_map(|result| &result.narrowed_effects)
            .chain(&self.quiescence.narrowed_effects)
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
            "rule": self.rule_ids.first(),
            "rules": self.rule_ids,
            "rule_source": self.rule_sources.first(),
            "rule_sources": self.rule_sources,
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
                "narrowed_effects": self.quiescence.narrowed_effects,
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
    use super::*;

    const CARD: &str = "tasks/card.md";

    /// Two sections named `Notes`, one of them inside a fence. The fenced pair is
    /// not a heading, so the real ambiguity is exactly two.
    const TWO_NOTES: &str = "# A\n\n## Notes\n\nleft\n\n```text\n# B\n## Notes\nfake\n```\n\n# B\n\n## Notes\n\nright\n";

    fn control_intent(seq: u32, action: &str, target: &str, payload: &str) -> Intent {
        Intent {
            rule_id: "proof-control".to_owned(),
            seq,
            action: action.to_owned(),
            target: Some(target.to_owned()),
            severity: None,
            payload: Some(payload.to_owned()),
            receipt: effects::receipt_address(CARD, "rev-after"),
        }
    }

    /// Execute one generation exactly as the quiescence proof does.
    fn apply_control(
        markdown: &str,
        intents: Vec<Intent>,
    ) -> Result<Option<(String, ChangeEvent)>, ProofFault> {
        apply_generation(&PendingGeneration {
            emitter: "proof-control".to_owned(),
            path: CARD.to_owned(),
            markdown: markdown.to_owned(),
            intents,
            provenance: Provenance::Change {
                fingerprint_before: "before".to_owned(),
                fingerprint_after: "after".to_owned(),
            },
            depth: 0,
            chain: vec!["proof-control".to_owned()],
        })
    }

    /// A production refusal, asserted to be one: a workspace failure is a different fault with a
    /// different exit.
    fn refusal_of(fault: ProofFault) -> String {
        match fault {
            ProofFault::Refused(message) => message,
            ProofFault::Workspace(message) => {
                panic!("a production refusal, not a proof workspace fault: {message}")
            }
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

    /// The proof writes through the production model grain: a frontmatter key's node
    /// is its whole multiline YAML value, continuation lines included.
    #[test]
    fn proof_set_field_replaces_the_complete_multiline_yaml_node() {
        let before = "---\nlabels:\n  - alpha\n  - beta\nstatus: open\n---\n# Card\n";
        let (after, _event) = apply_control(
            before,
            vec![control_intent(0, "md.set_field", "labels", "done")],
        )
        .expect("the production executor accepts the generation")
        .expect("the generation changes bytes");
        assert_eq!(
            after, "---\nlabels: done\nstatus: open\n---\n# Card\n",
            "production frontmatter node semantics remove continuation lines"
        );
    }

    /// `md.append_section` names one exact heading text; a name appearing twice is ambiguous and
    /// production refuses rather than silently picking.
    #[test]
    fn proof_append_section_refuses_an_ambiguous_heading() {
        let refusal = refusal_of(
            apply_control(
                TWO_NOTES,
                vec![control_intent(0, "md.append_section", "Notes", "added")],
            )
            .expect_err("two sections named Notes are ambiguous"),
        );
        println!("POPULATION ambiguous -> {refusal}");
        assert!(
            refusal.contains("section 'Notes' appears 2 times (ambiguous)"),
            "the proof reports the production ambiguity refusal: {refusal}"
        );
    }

    /// The descriptor's `section` is one exact heading text, never a `/`-joined heading path.
    #[test]
    fn proof_append_section_refuses_a_slash_spelled_heading_path() {
        let refusal = refusal_of(
            apply_control(
                TWO_NOTES,
                vec![control_intent(0, "md.append_section", "B/Notes", "added")],
            )
            .expect_err("`B/Notes` is not a heading production can resolve"),
        );
        println!("POPULATION slash path -> {refusal}");
        assert!(
            refusal.contains("no section 'B/Notes'"),
            "the proof reports the production not-found refusal: {refusal}"
        );
    }

    /// Production normalizes an append to exactly one trailing newline.
    #[test]
    fn proof_append_section_normalizes_to_one_trailing_newline() {
        let before = "# Card\n\n## Log\n\nfirst\n";
        let (after, _event) = apply_control(
            before,
            vec![control_intent(0, "md.append_section", "Log", "added\n\n\n")],
        )
        .expect("the production executor accepts the generation")
        .expect("the generation changes bytes");
        assert_eq!(
            after, "# Card\n\n## Log\n\nfirst\nadded\n",
            "the caller's trailing newlines are normalized to exactly one"
        );
    }

    /// One emitted generation is one atomic batch synthesizing one event. A mixed
    /// frontmatter+section batch has no addressable Delta container, so the event names no field
    /// and no section, and a downstream HOOK cannot fire on it.
    #[test]
    fn proof_mixed_generation_is_one_batch_with_no_addressable_identities() {
        let before = "---\nstatus: open\n---\n\n# Card\n\n## Log\n\nfirst\n";
        let (after, event) = apply_control(
            before,
            vec![
                control_intent(0, "md.set_field", "status", "mixed"),
                control_intent(1, "md.append_section", "Log", "entry"),
            ],
        )
        .expect("the production executor accepts the mixed generation")
        .expect("the generation changes bytes");
        assert!(
            after.contains("status: mixed") && after.contains("entry"),
            "both descriptors landed in the SAME batch: {after:?}"
        );
        println!(
            "POPULATION mixed event fields={:?} sections={:?}",
            event.fields_changed, event.sections_changed
        );
        assert!(
            event.fields_changed.is_empty() && event.sections_changed.is_empty(),
            "a mixed batch has no addressable identities — the proof observes exactly \
             what production emits: {event:?}"
        );
        assert!(
            event.changes.is_empty(),
            "the synthesized event carries no value deltas — fail-closed by construction"
        );
    }

    /// A Verdict is recorded create-OR-REPLACE: `put at:upsert` on the `verdict` key. A bounce —
    /// reject, rework, re-approve — must land its second decision through that same upsert.
    #[test]
    fn a_bounce_re_upsert_replaces_the_earlier_verdict() {
        let before =
            "---\ntype: task\nstatus: open\nowner: worker-a\nverdict: reject\n---\n\n# Ship it\n";
        let case = CaseSpec {
            name: None,
            doc: CARD.to_owned(),
            actor: Some("reviewer-b".to_owned()),
            force: false,
            set: BTreeMap::from([("verdict".to_owned(), "approve".to_owned())]),
            remove: Vec::new(),
            edits: Vec::new(),
            expect: Expected::One("pass".to_owned()),
        };
        let after = apply_case_mutation(CARD, before, &case).expect("the bounce lands");
        println!("POPULATION bounced -> {after:?}");
        assert!(
            after.contains("verdict: approve"),
            "the second decision landed through the same upsert: {after:?}"
        );
        assert!(
            !after.contains("reject"),
            "the earlier reject was REPLACED, not appended beside it: {after:?}"
        );
        assert_eq!(
            after.matches("verdict:").count(),
            1,
            "create-OR-replace leaves exactly one Verdict key: {after:?}"
        );
    }

    /// `remove` uses the production model's fm-key grain, so a multiline YAML value is removed
    /// whole rather than leaving orphaned continuation lines.
    #[test]
    fn case_remove_uses_the_production_model_grain_for_multiline_frontmatter() {
        let before = "---\nlabels:\n  - alpha\n  - beta\nstatus: open\n---\n# Card\n";
        let case = CaseSpec {
            name: None,
            doc: CARD.to_owned(),
            actor: None,
            force: false,
            set: BTreeMap::new(),
            remove: vec!["labels".to_owned()],
            edits: Vec::new(),
            expect: Expected::One("pass".to_owned()),
        };
        let after = apply_case_mutation(CARD, before, &case).expect("the removal applies");
        println!("POPULATION removed -> {after:?}");
        // The blank line left behind is the span law (a block leaf's span excludes its final
        // terminator), not a stranded continuation.
        assert_eq!(
            after, "---\n\nstatus: open\n---\n# Card\n",
            "the whole multiline node goes, continuation lines included"
        );
        assert!(
            !after.contains("alpha") && !after.contains("beta") && !after.contains("labels"),
            "no continuation line survives the removal: {after:?}"
        );
    }
}
