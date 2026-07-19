//! Rung-6 policy engine stub: compile rulesets-as-data, evaluate assertions
//! under declared budgets, authorize I3-shaped writes.
//!
//! # Charter
//! **Owns:** executing rulesets — YAML parse/compile (content-hash cache, pin
//! verify), the 14-assertion vocabulary with declared `Budget { class, p99_us }`,
//! and the `policy` / `policy_compile` / `policy_vocab` evaluation entry points.
//! The complete contract — schema, semantics, error taxonomy, versioning axes —
//! is `policy-schema-design.md` (session `18-02-meridian-rs/results/`); it is
//! cited, never restated: this crate implements that document.
//!
//! **Never does:** decide what a violation *means* (block/annotate/page is Go's
//! action mapping), own a corpus index (borrows `model`'s, capability-gated:
//! `Option<&CorpusIndex>` — absent index + corpus-class ruleset = the loud
//! `daemon_only` error), contain rules (the vocabulary is engine, rules are
//! data — weekly-churn policy never couples to release-cadence binary).
//!
//! # Axis E — SETTLED Go-side by review C5; this stub's argument kept for the record
//! The frozen `splice` shape carries no actor field: authorization is
//! structurally Go's (position b). `authorize` below is **deferrable** — it is
//! NOT on the rung-2 splice path. If hpath-shaped authorization rules ever
//! need this crate's selector machinery, they arrive as ordinary rung-6
//! assertions evaluated *for* Go (verdict as data, decision and actor stay
//! Go's). The original position-(a) argument, superseded but kept:
//!
//! ## (superseded) position (a), argued
//! I3 splice-authorization rules are hpath/section-shaped (`$owner`, section +
//! path specificity) — which is *exactly* the selector machinery this crate
//! already owns for rung 6: file-glob, hpath-glob, node predicates. Position (b)
//! (authorization entirely Go-side) would force Go to answer a section-shaped
//! question, and law 1 says Go never parses markdown — so (b) either duplicates
//! the selector engine in Go or degrades I3 to path-only rules. Position (a)
//! keeps the split clean along the existing law line: Go pre-authorizes the
//! *actor* (identity is fleet state — the oracle sees states, not who changed
//! them, per the schema doc's T2 ruling), passes actor claims in as data, and
//! this engine evaluates the section-shaped *rules*, returning a verdict on the
//! wire. Rules stay data (I3 rulesets ship like any ruleset — no daemon
//! redeploy); the actor never enters the oracle. Cost accepted: `authorize`
//! sits on the splice path, so its budget is gate-critical — the declared-p99
//! machinery exists precisely to hold that line.
//!
//! # Rungs
//! Rung 6 lands the engine (`compile` / `evaluate` / `vocab`); `authorize` is
//! deferred per C5 (not on the splice path). The wire ops appear only in
//! `sidecar`.

use model::{CorpusIndex, Document};

mod pack;

/// The load gate's injected fact plane, in policy vocabulary: the composition
/// layer builds [`FactDoc`]s from fixture bytes through the real parse→facts path
/// ([`facts_from_document`] over a real AST) and hands them to [`compile`] — no
/// `syntax::`/`model::` type crosses the `build_facts` signature (the fence holds
/// at the type level, P6-VERDICTS load-gate unification).
pub use pack::{FactDoc, facts_from_document};

/// The rule-language / injected-fact-API pin this engine implements (§11.4,
/// ruling 008). A manifest whose `api` differs is a loud `PinMismatch` — an
/// evaluator/dialect change is a pack change, gated at load.
///
/// # `rulepack-api@1` — the pinned surface
/// Rule predicates are fenced ` ```starlark ` blocks in literate rule pages, each
/// defining `def check(doc)`, evaluated in-engine via starlark-rust under the
/// manifest's per-eval [`EvalBudget`]. The pin names the injected world-model fact
/// surface (§11.2 — nodes / revs / spans / hpaths only):
/// - `doc.path` (str), `doc.nodes` (list, document order);
/// - node fields `kind`, `level`, `text`, `span` (int tuple), `node_rev`, `hpath`;
/// - `violation(rule, severity, span, node_rev, hpath, message)` (named args) —
///   records one §11.1 finding; `severity` ∈ {error, warn, info}.
///
/// Changing this surface or the dialect bumps the pin to `rulepack-api@N` and is
/// gated at load — never a wire change (row-13 wire-invariance: no wire crate
/// names Starlark). The exact injected surface is documented on `pack`.
pub const RULEPACK_API: &str = "rulepack-api@1";

/// The §11.3 per-eval metering budget: `{steps, mem}` bounding one evaluation.
///
/// Distinct from [`Budget`] (`{class, p99_us}`, the per-assertion cost
/// declaration for `vocab`) — do not conflate. Exhaustion of *this* budget is
/// metered: at the load gate it refuses the pack; on the wire (P6-EVAL) it
/// surfaces as the `budget_exceeded` FINDING, never an error frame (§8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvalBudget {
    pub steps: u64,
    pub mem: u64,
}

/// Caller-provided resolver for manifest-relative pack files (fixtures, rules).
///
/// Policy stays I/O-free (as `model` is): the caller — `fs`/`sidecar` at the
/// §6.1 "reads path from disk" edge — injects file access, and tests inject an
/// in-memory map. Paths are exactly the manifest's relative strings.
pub trait PackFiles {
    /// Read a pack file's UTF-8 contents, or fail (missing/unreadable/non-UTF-8).
    ///
    /// # Errors
    /// Any I/O or decode failure from the underlying source.
    fn read(&self, rel_path: &str) -> std::io::Result<String>;
}

/// Per-assertion cost declaration, surfaced verbatim by `policy_vocab` and
/// enforced by the bench suite against the frozen GT corpus (a release
/// exceeding a declared p99 fails CI).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub class: BudgetClass,
    pub p99_us: u32,
}

/// Budget classes place the evaluator: node/file run sidecar-mode from rung 1
/// machinery; corpus requires the resident index — daemon-only, enforced at
/// compile time with a loud error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetClass {
    Node,
    File,
    Corpus,
}

/// A pinned ruleset reference: id label + path + optional sha256 (effect-style
/// pin; mismatch fails loud, never evaluates).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulesetPin {
    pub id: String,
    pub path: String,
    pub sha256: Option<String>,
}

/// A compiled ruleset — admitted only after its fixtures demonstrated
/// themselves under budget (§11.3 load gate). Private fields seal construction
/// to `policy::compile` (the capability seal); the eval bridge lives behind
/// this type, so P6-STARLARK/P6-EVAL add no public surface here.
#[derive(Debug)]
pub struct CompiledRuleset {
    id: String,
    content_hash: String,
    budget: EvalBudget,
    budget_class: BudgetClass,
    /// The extracted, parse-checked, vocabulary-validated predicates — held so
    /// `evaluate` reuses the compile-time work rather than re-parsing every rule
    /// page per document.
    predicates: Vec<pack::Predicate>,
}

impl CompiledRuleset {
    /// The pack's declared `id` (echoed into violations for provenance).
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// sha256 (hex) of the manifest source — the content-hash cache key. The
    /// cache itself is caller/daemon state (law 2, disposable), not held here.
    #[must_use]
    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }

    /// The §11.3 per-eval metering budget the fixtures passed under.
    #[must_use]
    pub fn budget(&self) -> EvalBudget {
        self.budget
    }

    /// Ruleset-level budget class = max class over used assertions (schema doc
    /// L188). The P6-VERDICTS seam: a `Corpus` result means the pack needs the
    /// resident corpus index, so loading it sidecar-mode is later refused
    /// `daemon_only` (that error is NOT defined in this unit).
    #[must_use]
    pub fn budget_class(&self) -> BudgetClass {
        self.budget_class
    }

    /// Number of rule pages the pack declares.
    #[must_use]
    pub fn rule_count(&self) -> usize {
        self.predicates.len()
    }
}

/// Compile errors per the schema doc's error taxonomy (`ruleset_not_found`,
/// `pin_mismatch`, `compile_error` + `unknown_assertions`, `unsupported_vocab`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompileError {
    NotFound,
    PinMismatch {
        expected: String,
        actual: String,
    },
    UnknownAssertions {
        names: Vec<String>,
        at_rules: Vec<String>,
    },
    UnsupportedVocab {
        requires: u32,
        engine: u32,
    },
    /// Manifest unparseable or failing schema validation (bad YAML, missing or
    /// unknown keys, an unreadable rule page). The taxonomy's `compile_error`
    /// class — fails loud, never evaluates.
    Malformed {
        reason: String,
    },
    /// The §11.3 load gate: a fixture failed to demonstrate the pack — its actual
    /// verdict disagreed with its declared `expect`, it exhausted the per-eval
    /// budget, or it was unreadable/undeclared. A pack that cannot demonstrate
    /// itself is never admitted.
    FixtureFailed {
        fixture: String,
        detail: String,
    },
}

/// One finding: rule id, severity, and the violating node's wire coordinates
/// (`path/span/node_rev/hpath`) — findings, never decisions.
#[derive(Debug, Clone, PartialEq)]
pub struct Violation {
    pub rule: String,
    pub severity: Severity,
    pub path: String,
    pub span: model::ByteSpan,
    pub node_rev: model::NodeRev,
    pub hpath: Option<Vec<String>>,
    pub message: String,
}

/// Descriptive policy data (how bad, per the convention) — what to *do* about
/// it is Go's action mapping. The T1 line, held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warn,
    Info,
}

/// Parse the §11.3 manifest, verify the pin, and RUN the pack's fixtures under
/// its declared budgets as the load gate — a pack whose fixtures fail is never
/// admitted. YAML enters the engine here and nowhere else.
///
/// `source` is the manifest text (the caller pre-reads it from `pin.path`);
/// `files` resolves manifest-relative fixture/rule paths to their contents, so
/// policy performs no I/O of its own. `build_facts` turns a fixture's `(path,
/// body)` into the injected fact plane — the load gate's demonstration runs over
/// the SAME facts production `evaluate` sees (the composition layer injects the
/// real parse→facts path; policy names no parser). Kept in policy vocabulary
/// (`&str`s in, [`FactDoc`] out) so the fence holds at the type level.
///
/// Pipeline: content-pin (sha256) → parse → api-pin → read+classify rules →
/// extract+parse fenced Starlark predicates → run fixtures over `build_facts`'s
/// facts through the metered Starlark core → admit / refuse.
///
/// # Errors
/// [`CompileError::PinMismatch`] (api or sha256), [`CompileError::Malformed`]
/// (bad manifest / unreadable rule), or [`CompileError::FixtureFailed`] (the
/// load gate).
pub fn compile(
    pin: &RulesetPin,
    source: &str,
    files: &dyn PackFiles,
    build_facts: &dyn Fn(&str, &str) -> FactDoc,
) -> Result<CompiledRuleset, CompileError> {
    // 1. Content pin — integrity before trust; fails loud, never evaluates.
    let content_hash = sha256_hex(source);
    if let Some(expected) = &pin.sha256 {
        let want = expected.strip_prefix("sha256:").unwrap_or(expected);
        if !content_hash.eq_ignore_ascii_case(want) {
            return Err(CompileError::PinMismatch {
                expected: expected.clone(),
                actual: format!("sha256:{content_hash}"),
            });
        }
    }

    // 2. Parse the generic manifest.
    let manifest = pack::parse_manifest(source)?;

    // 3. api pin — an evaluator/dialect mismatch is gated at load.
    if manifest.api != RULEPACK_API {
        return Err(CompileError::PinMismatch {
            expected: RULEPACK_API.to_string(),
            actual: manifest.api,
        });
    }

    // 4. Read rule pages (must resolve) and classify the ruleset budget class.
    let mut rule_sources = Vec::with_capacity(manifest.rules.len());
    for rule in &manifest.rules {
        let text = files.read(rule).map_err(|e| CompileError::Malformed {
            reason: format!("rule '{rule}' unreadable: {e}"),
        })?;
        rule_sources.push(text);
    }
    let budget_class = pack::classify_budget_class(&rule_sources);

    // 4b. Extract + parse-check each rule page's fenced Starlark predicate. A rule
    // that cannot be read (no ```starlark block, or unparseable) fails loud here,
    // before any fixture runs.
    let predicates = pack::extract_predicates(&rule_sources, &manifest.rules)?;

    // 4c. §11.2 WHEN/HOW partition: a WHEN referencing anything outside the closed
    // fact vocabulary is refused at COMPILE, before any fixture runs.
    pack::check_when_vocab(&predicates)?;

    // 5. Fixtures ARE the load gate: no fixtures = cannot demonstrate itself.
    if manifest.fixtures.is_empty() {
        return Err(CompileError::FixtureFailed {
            fixture: String::new(),
            detail: "pack declares no fixtures — a rule that cannot demonstrate itself \
                     does not run"
                .to_string(),
        });
    }
    for fixture in &manifest.fixtures {
        let content = files
            .read(fixture)
            .map_err(|e| CompileError::FixtureFailed {
                fixture: fixture.clone(),
                detail: format!("unreadable: {e}"),
            })?;
        let fx = pack::parse_fixture(fixture, &content)?;
        // Load-gate unification (P6-VERDICTS): the fixture demonstrates over the
        // SAME fact plane production `evaluate` uses — `build_facts` is the real
        // parse→facts path the composition layer injects (`facts_from_document`
        // over a real AST), so a fixture cannot pass on a plane the wire never
        // reproduces. The signature stays in POLICY vocabulary (`&str`s in,
        // `FactDoc` out) — no `syntax::`/`model::` type crosses it, the fence
        // holds at the type level. The retired synthetic per-line builder lives
        // test-only now.
        let facts = build_facts(&fx.path, &fx.body);
        match pack::eval_over_facts(&predicates, &facts, manifest.budgets) {
            Ok(violations) => {
                let actual = if violations.is_empty() {
                    pack::Expect::Pass
                } else {
                    pack::Expect::Fail
                };
                if actual != fx.expect {
                    return Err(CompileError::FixtureFailed {
                        fixture: fixture.clone(),
                        detail: format!(
                            "declared expect:{} but demonstrated:{}",
                            fx.expect, actual
                        ),
                    });
                }
            }
            Err(pack::EvalError::Budget(exhausted)) => {
                return Err(CompileError::FixtureFailed {
                    fixture: fixture.clone(),
                    detail: format!(
                        "exhausted per-eval budget (steps>{} or mem>{})",
                        exhausted.steps, exhausted.mem
                    ),
                });
            }
            Err(pack::EvalError::Runtime(msg)) => {
                return Err(CompileError::FixtureFailed {
                    fixture: fixture.clone(),
                    detail: format!("rule evaluation error: {msg}"),
                });
            }
        }
    }

    // 6. Admitted.
    Ok(CompiledRuleset {
        id: manifest.id,
        content_hash,
        budget: manifest.budgets,
        budget_class,
        predicates,
    })
}

/// sha256 of `s` as lowercase hex — the content-pin check and cache key.
fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(s.as_bytes());
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// Evaluate a ruleset over documents (gate and diagnostic modes share this
/// path; mode/limit shaping is `sidecar`'s wire concern) and return the §11.1
/// findings. Facts are derived from each real `Document` AST (`model::build` →
/// `FactDoc`); the WHEN vocabulary was closed at compile (§11.2), so every
/// predicate here reads world-model facts only.
///
/// Per-eval `{steps, mem}` budget is metered per document; exhaustion appends the
/// `budget_exceeded` FINDING to the returned verdicts and never surfaces as an
/// error or panic (frozen §8). The verdict order is document order, then
/// per-document rule order.
///
/// `corpus` is the capability parameter: `None` in sidecar mode. File/node-class
/// rules need no index; corpus-class refusal (`daemon_only`) is a load-time
/// concern owned by P6-VERDICTS, not evaluated here.
#[must_use]
pub fn evaluate(
    ruleset: &CompiledRuleset,
    docs: &[&Document],
    corpus: Option<&CorpusIndex>,
) -> Vec<Violation> {
    let _ = corpus;
    let mut out = Vec::new();
    for doc in docs {
        let facts = facts_from_document(doc);
        out.extend(pack::eval_document(
            &ruleset.predicates,
            &facts,
            ruleset.budget,
            &doc.root.node_rev.0,
        ));
    }
    out
}

/// Axis-E entry point: may this splice stand, per the I3-shaped rules in
/// `ruleset`? `actor` arrives as pre-authorized data from Go (never resolved
/// here); the verdict goes back on the wire. See crate doc for the position
/// argument.
#[must_use]
pub fn authorize(
    doc: &Document,
    target: &model::Target,
    actor: &str,
    ruleset: &CompiledRuleset,
) -> AuthorizeVerdict {
    let _ = (doc, target, actor, ruleset);
    todo!("rung 2: I3 rules-as-data evaluation (EPERM-first, $owner sentinel)")
}

/// EPERM-first: denial is the default shape, allowance is explicit.
#[derive(Debug, Clone, PartialEq)]
pub enum AuthorizeVerdict {
    Allow,
    Deny { rule: String, message: String },
}

/// The engine declares itself: vocabulary version + per-assertion budgets,
/// surfaced verbatim (the `policy_vocab` op body).
#[must_use]
pub fn vocab() -> Vec<(&'static str, Budget)> {
    todo!("rung 6: the 14-assertion vocabulary with declared budgets")
}
