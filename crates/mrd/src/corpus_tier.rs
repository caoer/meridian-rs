//! `mrd test --corpus` — the tier-2 corpus runner (U1.5): drive a convention's
//! `check_change` over SYNTHETIC changes derived from the 18-02 corpus, and
//! report the three signals the pre-arming gate needs — fire-where-expected,
//! zero dead rules, and the fuel + heap p50/p99 budgets.
//!
//! # What a corpus-test spec is
//! A spec is a markdown file (house grammar): YAML frontmatter (`convention`,
//! `corpus`, optional `corpus_test`) plus fenced blocks —
//!
//! - ` ```rules ` — the DECLARED rule set: one rule id per line (a `passing`
//!   citation the convention's `check_change` can emit). A declared rule that no
//!   synthetic change fires is a DEAD rule ([the headline signal](Report)).
//! - ` ```case ` — one synthetic-change case as JSON: `{doc, actor?, force?,
//!   set?, remove?, expect}`. `doc` names a corpus file (mount-confined within
//!   the corpus root); the mutation (`set`/`remove` frontmatter edits) produces
//!   the AFTER state over the corpus doc's real BEFORE bytes; `expect` is the
//!   rule id the change must fire, or `"pass"`.
//!
//! # The three signals (rulings § test tiers — `test --corpus`)
//! For each case the tier reads the corpus doc, applies the mutation, derives the
//! [`rulepack-api@2`](policy) [`Change`] from the before/after STATES, and runs
//! the convention's `check_change` METERED under FULL `EvalLimits`:
//!
//! - **fire-where-expected** — the set of rules a case fired must EQUAL its
//!   `expect` (a single rule id, or `"pass"` for no fire). A doc outside the
//!   convention's `paths:` scope is never run — it can only pass (scope gating is
//!   observable here). A mismatch is a finding.
//! - **zero dead rules** — every DECLARED rule must fire at least once over the
//!   corpus; a declared rule with zero fires is reported DEAD (a finding). A rule
//!   the convention fired that the spec never declared is a `surprise` finding.
//! - **fuel + heap budgets** — the exact fuel (ticks) and peak heap each eval
//!   spent, reduced to p50/p99/max (nearest-rank) over the in-scope evals.
//!
//! # Output + exit codes (§4 preamble law, `docs/status.md`)
//! JSON under `--json`, a human report otherwise. Exit 0 (manifest matched, zero
//! dead rules), 1 (a fire mismatch, a dead/surprise rule, or a convention eval
//! fault — findings), 2 (a malformed spec, an unreadable corpus doc, or a
//! convention that would not load — tool failure).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use model::{Document, NodeKind};
use policy::ConventionFiles;
use policy::{
    Change, ChangeOp, CheckError, CheckLimits, Convention, Invocation, derive_change,
    load_convention, load_seed_convention,
};
use serde::Deserialize;
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

    let convention = load_spec_convention(&spec)?;
    let report = run_corpus(&convention, &spec)?;

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
    /// `seed` (the embedded seed convention) or a resolved folder path.
    convention: ConventionRef,
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
    /// The rule id the change must fire, or `"pass"` for no fire.
    expect: String,
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

        let convention = match fm.get("convention").map(String::as_str) {
            Some("seed") => ConventionRef::Seed,
            Some(path) if !path.is_empty() => {
                let dir = resolve_rel(spec_dir, path);
                let slug = dir
                    .file_name()
                    .and_then(|s| s.to_str())
                    .ok_or_else(|| {
                        Fail::tool(format!("convention folder {path} has no slug component"))
                    })?
                    .to_owned();
                ConventionRef::Folder { slug, dir }
            }
            _ => {
                return Err(Fail::tool(
                    "corpus spec frontmatter needs `convention: seed` or a folder path".to_owned(),
                ));
            }
        };

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
        if cases.is_empty() {
            return Err(Fail::tool(
                "corpus spec declares no ```case blocks".to_owned(),
            ));
        }
        Ok(Spec {
            name,
            convention,
            corpus_root,
            declared_rules,
            cases,
        })
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
fn load_spec_convention(spec: &Spec) -> Result<Convention, Fail> {
    let limits = CheckLimits::default();
    match &spec.convention {
        ConventionRef::Seed => load_seed_convention(limits)
            .map_err(|e| Fail::tool(format!("seed convention failed to load: {e}"))),
        ConventionRef::Folder { slug, dir } => {
            let files = DirConventionFiles { dir: dir.clone() };
            load_convention(slug, &files, limits)
                .map_err(|e| Fail::tool(format!("convention `{slug}` failed to load: {e}")))
        }
    }
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
    expect: String,
    observed: Observed,
    /// Fuel + heap the eval spent (`None` when the doc was out of scope or the
    /// eval faulted — those spend no metered budget the profile should count).
    fuel: Option<u64>,
    mem: Option<u64>,
}

impl CaseResult {
    /// Whether the observed outcome matched `expect`. An out-of-scope doc can
    /// only pass; a fired case must fire EXACTLY the expected rule.
    fn matched(&self) -> bool {
        match &self.observed {
            Observed::Pass { .. } => self.expect == "pass",
            Observed::Fired { rules } => {
                self.expect != "pass" && rules.len() == 1 && rules.contains(&self.expect)
            }
            Observed::Error { .. } => false,
        }
    }
}

/// Run every case over the corpus and fold into a [`Report`].
///
/// # Errors
/// [`Fail::tool`] — a case names an unreadable / non-UTF-8 corpus doc, or a path
/// that escapes the corpus mount.
fn run_corpus(convention: &Convention, spec: &Spec) -> Result<Report, Fail> {
    let mut results = Vec::with_capacity(spec.cases.len());
    for (i, case) in spec.cases.iter().enumerate() {
        results.push(run_case(convention, spec, i, case)?);
    }

    // Fired-rule tally → dead + surprise sets.
    let mut fired: BTreeSet<String> = BTreeSet::new();
    for r in &results {
        if let Observed::Fired { rules } = &r.observed {
            fired.extend(rules.iter().cloned());
        }
    }
    let declared: BTreeSet<String> = spec.declared_rules.iter().cloned().collect();
    let dead_rules: Vec<String> = spec
        .declared_rules
        .iter()
        .filter(|r| !fired.contains(*r))
        .cloned()
        .collect();
    let surprise_rules: Vec<String> = fired.difference(&declared).cloned().collect();

    // Fuel + heap p50/p99/max over the in-scope, non-faulted evals.
    let mut fuel: Vec<u64> = results.iter().filter_map(|r| r.fuel).collect();
    let mut mem: Vec<u64> = results.iter().filter_map(|r| r.mem).collect();
    fuel.sort_unstable();
    mem.sort_unstable();

    Ok(Report {
        name: spec.name.clone(),
        convention_slug: convention.slug().to_owned(),
        convention_source: convention_source_label(&spec.convention),
        corpus_root: spec.corpus_root.display().to_string(),
        scope: convention.scope().to_vec(),
        declared_rules: spec.declared_rules.clone(),
        results,
        dead_rules,
        surprise_rules,
        fuel_budget: Budget::of(&fuel),
        mem_budget: Budget::of(&mem),
    })
}

/// Run one case: read the corpus doc, apply the mutation, derive the change, and
/// run the convention's metered `check_change`.
fn run_case(
    convention: &Convention,
    spec: &Spec,
    index: usize,
    case: &CaseSpec,
) -> Result<CaseResult, Fail> {
    let rel = confine(&case.doc)
        .map_err(|m| Fail::tool(format!("case {index} doc {:?}: {m}", case.doc)))?;
    let on_disk = spec.corpus_root.join(&rel);
    let before_md = std::fs::read_to_string(&on_disk).map_err(|e| {
        Fail::tool(format!(
            "case {index}: cannot read corpus doc {}: {e}",
            on_disk.display()
        ))
    })?;
    let after_md = apply_mutation(&before_md, &case.set, &case.remove);

    let doc_path = rel.to_string_lossy().replace('\\', "/");
    let before = build_doc(&doc_path, &before_md);
    let after = build_doc(&doc_path, &after_md);
    let in_scope = convention.matches_path(&doc_path);

    let name = case.name.clone().unwrap_or_else(|| doc_path.clone());
    if !in_scope {
        // Out of scope: the convention is never run — the change can only pass.
        return Ok(CaseResult {
            name,
            doc: doc_path,
            actor: case.actor.clone(),
            in_scope,
            expect: case.expect.clone(),
            observed: Observed::Pass { in_scope },
            fuel: None,
            mem: None,
        });
    }

    let change = synth_change(&before, &after, case);
    let (observed, fuel, mem) = match convention.check_change_metered(&change) {
        Ok(tel) => {
            let fuel = Some(tel.fuel_used);
            let mem = Some(tel.mem_used);
            if tel.refusals.is_empty() {
                (Observed::Pass { in_scope }, fuel, mem)
            } else {
                let rules = tel
                    .refusals
                    .iter()
                    .map(|r| r.passing_scenario.clone())
                    .collect();
                (Observed::Fired { rules }, fuel, mem)
            }
        }
        Err(e) => (
            Observed::Error {
                detail: check_error_label(&e),
            },
            None,
            None,
        ),
    };

    Ok(CaseResult {
        name,
        doc: doc_path,
        actor: case.actor.clone(),
        in_scope,
        expect: case.expect.clone(),
        observed,
        fuel,
        mem,
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
    convention_slug: String,
    convention_source: String,
    corpus_root: String,
    scope: Vec<String>,
    declared_rules: Vec<String>,
    results: Vec<CaseResult>,
    dead_rules: Vec<String>,
    surprise_rules: Vec<String>,
    fuel_budget: Budget,
    mem_budget: Budget,
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
        self.mismatches() + self.dead_rules.len() + self.surprise_rules.len()
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
        let mut s = String::new();
        let _ = writeln!(s, "# mrd test --corpus — {}\n", self.name);
        let _ = writeln!(
            s,
            "convention: `{}` ({})",
            self.convention_slug, self.convention_source
        );
        let _ = writeln!(s, "corpus: `{}`", self.corpus_root);
        let _ = writeln!(s, "scope: `{}`", self.scope.join("`, `"));
        let in_scope = self.results.iter().filter(|r| r.in_scope).count();
        let _ = writeln!(
            s,
            "cases: {} ({in_scope} in-scope eval(s))\n",
            self.results.len()
        );

        // Fire-where-expected table.
        s.push_str("## Fire-where-expected\n\n");
        s.push_str("| case | doc | actor | expected | observed | ok |\n");
        s.push_str("|------|-----|-------|----------|----------|:--:|\n");
        for r in &self.results {
            let actor = r.actor.as_deref().unwrap_or("(external)");
            let ok = if r.matched() { "ok" } else { "**MISMATCH**" };
            let _ = writeln!(
                s,
                "| {} | `{}` | `{actor}` | {} | {} | {ok} |",
                r.name,
                r.doc,
                r.expect,
                Self::observed_cell(r),
            );
        }
        s.push('\n');

        // Dead rules — the headline signal.
        s.push_str("## Dead rules (declared, never fired)\n\n");
        if self.dead_rules.is_empty() {
            s.push_str("_none — every declared rule fired at least once._\n\n");
        } else {
            for id in &self.dead_rules {
                let _ = writeln!(s, "- `{id}`");
            }
            s.push('\n');
        }

        // Surprise rules (fired but never declared).
        if !self.surprise_rules.is_empty() {
            s.push_str("## Surprise rules (fired, never declared)\n\n");
            for id in &self.surprise_rules {
                let _ = writeln!(s, "- `{id}`");
            }
            s.push('\n');
        }

        // Fuel + heap budgets.
        s.push_str("## Fuel + heap budgets\n\n");
        let _ = writeln!(s, "over {} in-scope eval(s):\n", self.fuel_budget.n);
        s.push_str("| metric | p50 | p99 | max |\n");
        s.push_str("|--------|----:|----:|----:|\n");
        let _ = writeln!(
            s,
            "| fuel (ticks) | {} | {} | {} |",
            self.fuel_budget.p50, self.fuel_budget.p99, self.fuel_budget.max
        );
        let _ = writeln!(
            s,
            "| heap (bytes) | {} | {} | {} |",
            self.mem_budget.p50, self.mem_budget.p99, self.mem_budget.max
        );
        s.push('\n');

        let matched = self.results.len() - self.mismatches();
        let _ = writeln!(
            s,
            "{} case(s): {matched} matched, {} mismatch(es), {} dead rule(s), {} error(s).",
            self.results.len(),
            self.mismatches(),
            self.dead_rules.len(),
            self.errored(),
        );
        s
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
                    "fuel_used": r.fuel,
                    "mem_used": r.mem,
                })
            })
            .collect();
        let value = json!({
            "corpus_test": self.name,
            "convention": self.convention_slug,
            "convention_source": self.convention_source,
            "corpus_root": self.corpus_root,
            "scope": self.scope,
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
            "summary": {
                "cases": self.results.len(),
                "matched": self.results.len() - self.mismatches(),
                "mismatches": self.mismatches(),
                "dead_rules": self.dead_rules.len(),
                "surprise_rules": self.surprise_rules.len(),
                "errors": self.errored(),
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
