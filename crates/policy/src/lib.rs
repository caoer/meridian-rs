//! Rung-6 policy engine: compile rulesets-as-data, evaluate assertions under
//! declared budgets, and gate writes at the armed change plane.
//!
//! # Charter
//! **Owns:** executing rulesets — YAML parse/compile (content-hash cache, pin
//! verify), the 14-assertion vocabulary with declared `Budget { class, p99_us }`,
//! and the `policy` / `policy_compile` / `policy_vocab` evaluation entry points.
//! Since U4.2 it ALSO owns the **blocking gate** at the armed change plane (see
//! [`gate`] / [`resolve_armed_set`]; laws.md § the policy gate): the pure
//! decision that refuses a write when the workspace's own attested law says so.
//! The complete contract — schema, semantics, error taxonomy, versioning —
//! is implemented here; this crate is that contract's implementation.
//!
//! **Never does:** own a corpus index (borrows `model`'s, capability-gated:
//! `Option<&CorpusIndex>` — absent index + corpus-class ruleset = the loud
//! `daemon_only` error), contain rules (the vocabulary is engine, rules are
//! data — weekly-churn policy never couples to release-cadence binary), reach
//! disk (stays I/O-free like `model`: the gate's armed-set load takes injected
//! file access — the trusted write path in `wire-serve`/`run` does the reads).
//!
//! # The gate at the armed change plane (U4.2)
//! When a workspace is armed (an attested INDEX plus the once-armed marker),
//! [`gate`] evaluates a [`Change`] through the workspace's OWN armed law after
//! CAS and before bytes land, in both writer paths — a `block`-severity firing,
//! a drifted law, or an un-loadable/corrupt/missing-once-armed INDEX REFUSES the
//! write with a `{code, recovery}` pair from the closed §8 taxonomy. A
//! never-armed workspace is a bit-for-bit no-op. This is the seam the deferred
//! Go-era `authorize` stub reserved; the caller-supplies shape died with Go —
//! the armed set is loaded and verified from the workspace path, never supplied
//! by the caller.
//!
//! # Rungs
//! Rung 6 lands the engine (`compile` / `evaluate` / `vocab`) and the U4.2 gate;
//! the wire ops appear only in `sidecar`.

use model::{CorpusIndex, Document};

/// The attested armed-set artifact — the INDEX successor (registration ruling § 4,
/// amended 2026-08-01). [`armed::arm`] is the ONE act that turns a discovered page
/// into an armed one: it narrows to an [`armed::ArmRoot`], resolves through the
/// landed resolver, and pins each winner's page + [`page_rev`] into an
/// [`armed::ArmedArtifact`] row keyed by (id, arm root).
///
/// It is a MODULE, not a flat re-export, because the folder loader's `index::arm`
/// still lives until the loader cutover retires it — two acts named `arm` in one
/// namespace would be two names for one thing at exactly the seam where the
/// difference matters. When `index::arm` dies, nothing here has to be renamed.
pub mod armed;
pub mod armed_law;

mod binding;
mod change;
mod check_eval;
mod convention;
/// I4 def-conformance (U8c): the engine-side port of meridian-go's write-time
/// def validator — the pure verdict over (prev, candidate) documents the put
/// path consults before any splice. Byte-exact against the U0 defs goldens.
pub mod defs;
mod gate;
mod hook;
mod index;
mod pack;
mod reaction;
mod registration;
mod rule;
mod seed;

/// The `rulepack-api@2` change surface (U1.1): the `Change` struct a
/// `check_change(change)` predicate reads, its derivation from before/after
/// states, the closed 14-key fact vocabulary, and the purity guard's classifier.
/// Shared by the `mrd test` harness (U1.2) and the door's `gate()` (U4.2).
pub use change::{
    CHANGE_FACT_VOCAB, Change, ChangeOp, DocFacts, Edge, EdgeDecl, EditFact, Invocation, NodeFact,
    RULEPACK_API_V2, TargetFact, assert_vocab_pure, derive_change, impure_source, vocab_keys,
};

/// The load gate's injected fact plane, in policy vocabulary: the composition
/// layer builds [`FactDoc`]s from fixture bytes through the real parse→facts path
/// ([`facts_from_document`] over a real AST) and hands them to [`compile`] — no
/// `syntax::`/`model::` type crosses the `build_facts` signature (the fence holds
/// at the type level, P6-VERDICTS load-gate unification).
pub use pack::{FactDoc, facts_from_document};

/// The `rulepack-api@2` CHECK evaluation surface (U1.3): the full-`EvalLimits`
/// `check_change(change)` evaluator behind the convention loader. [`CheckLimits`]
/// meters all five guards (tick + heap + call-depth + source-size + nesting);
/// [`CheckError`] is its typed failure surface.
pub use check_eval::{CheckError, CheckLimits, CheckTelemetry};

/// The convention loader (U1.3): `conventions/<slug>/` folder grammar, the CHECK
/// and HOOK capability ceilings (FIX/VIEW deferred), and `paths:` scope. A loaded
/// [`Convention`] runs its `check_change` over a [`Change`], returning the
/// [`CheckOutcome`]'s [`Refusal`]s; a refusal always cites its passing scenario.
pub use convention::{
    Capability, CheckOutcome, Convention, ConventionFiles, CounterfactualConvention, LoadError,
    Refusal, load_convention, load_convention_for_corpus,
};

/// The HOOK capability (U1.3): the emit leg's declaration. A [`Hook`] carries the
/// declared severity, scope, effect caps, per-eval budget, the VERBATIM `how:` block
/// (frozen data the engine never interprets), and a predicate whose capability
/// ceiling was enforced at load. [`evaluate_hooks`] runs armed, in-scope HOOKs and
/// returns advisory-only [`HookOutcome`]s; [`SLICE1_CAPS`] is what slice 1 admits.
pub use hook::{
    Hook, HookEvalError, HookFinding, HookOutcome, HookTestTelemetry, Intent, SLICE1_CAPS,
    evaluate_counterfactual_hooks_for_corpus_metered, evaluate_hooks, evaluate_hooks_for_test,
    evaluate_hooks_for_test_metered, evaluate_loaded_hooks, intent_from_effect, load_hook,
};

/// The reaction-plane payload (C1a): [`derive_event`] turns a landed [`Change`]
/// into the `on_change(event)` argument a HOOK predicate reads. It attaches the
/// values behind `fields_changed` and the changed document's frontmatter, and it
/// drops `actor` — the engine must not observe the observer.
pub use reaction::derive_event;

/// The throwaway seed convention (U1.3): a real `reviewer-not-owner` `check_change`
/// the harness (U1.2) and the door (U4.2) pre-test against before U4.4's floor lands.
pub use seed::{SEED_CONVENTION_SLUG, SeedFiles, load_seed_convention, seed_convention_files};

/// The attested INDEX generator (U1.4): sweep `conventions/` into a living
/// checklist page (row = checkbox · slug · severity · pinned rev · evidence link ·
/// scope; sorted severity desc, then slug), the O(armed) single-file read
/// ([`armed_from_index`]), and the arming gate ([`arm`]) that refuses on evidence
/// drift ([`ArmError::Drift`], `report-rev == armed-rev`).
pub use index::{
    ArmError, ArmedRef, Enforcement, IndexCorrupt, IndexEntry, arm, armed_from_index,
    generate_index, parse_index_strict, render_rows, sweep,
};

/// Tag-indexed rule registration — the registration rework's discovery layer.
/// A page registers by carrying `rules/hook` / `rules/check` in its frontmatter
/// `tags:` ([`RuleKind`]), is identified by its frontmatter `id:` ([`RuleId`],
/// § 2 grammar), and resolves against same-id pages by mount depth along the
/// three-rung scope ladder ([`ScopeLayer`] → [`Scope`]). [`RuleIndex::discover`]
/// reads a caller-supplied page feed ([`PageRef`] — `policy` stays I/O-free);
/// [`RuleIndex::resolve`] is the ONE pure resolver both the arming path and the
/// read-only print verb call, and it RETAINS every shadowed candidate so the
/// override chain stays printable. Nothing here arms: discovery makes a page
/// known, only the explicit attested ARM act activates it.
pub use registration::{
    Collision, Effective, EffectiveSet, ID_KEY, IdFault, MAX_ID_LEN, PageRef,
    REGISTRATION_NAMESPACE, RegisterError, RegisterFault, Registration, RuleId, RuleIndex,
    RuleKind, Scope, ScopeLayer, page_rev, register_page,
};

/// The page-shaped rule load — what a registered page becomes when it is
/// evaluated. [`register_page`] answers identity (the tag and the `id:`);
/// [`load_rule`] answers evaluability, parsing the legs the registration tags
/// declare through the SAME parsers the convention folder loader uses. It takes the
/// [`Registration`] rather than re-deriving one, so a page becomes a rule in exactly
/// one place, and it verifies the supplied bytes against the registered rev rather
/// than trusting the caller not to have re-read a moved page.
pub use rule::{Rule, RuleLoadError, load_rule};

/// The blocking gate at the armed change plane (U4.2): the pure decision
/// ([`gate`]) plus the load-and-verify half ([`resolve_armed_set`]) that reads a
/// workspace's attested INDEX (U1.4) + once-armed marker (U2.5) through an
/// injected [`ConventionSource`], failing CLOSED on missing-once-armed, corrupt,
/// un-loadable, or DRIFTED armed law. Fills the retired `authorize` seam.
pub use gate::{
    ArmedConvention, ArmedSet, ConventionSource, GateFault, GateFinding, GateOutcome, GateRefusal,
    GateViolation, gate, resolve_armed_set,
};

/// The binding law + `--truth` convergence (U4.3): the door law that refuses a
/// one-sided file↔index change ([`classify_door_law`], taxonomy row 9) and the
/// INDEX-integrity floor that refuses deletion/rename of the INDEX or the
/// once-armed marker (row 10, not force-escapable). [`converge`] deploys a
/// divergence in an explicit direction ([`Truth`]).
pub use binding::{
    ATTESTED_MARKER_PATH, BindingSide, ConvergeError, Convergence, DoorLaw, FileAction,
    RESERVED_INDEX_PATH, Truth, classify_door_law, converge,
};

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

/// The engine declares itself: the `rulepack-api@2` change surface's 14-key fact
/// vocabulary, each key paired with the [`Budget`] of the plane it reads (the
/// `policy_vocab` op body).
///
/// The `class` is the true cost tier of the fact (change-local / whole-document /
/// one-hop cross-document); the `p99_us` is a monotonic class DEFAULT
/// ([`change::class_default_p99`]), not a per-key measured SLA — per-key
/// calibration against the frozen corpus is U1.5's (`test --corpus`). The keys
/// are exactly the [purity-guarded vocabulary](change::CHANGE_FACT_VOCAB): every
/// one a pure function of the before/after states and pinned evidence, no
/// git/clock/random/io fact among them.
#[must_use]
pub fn vocab() -> Vec<(&'static str, Budget)> {
    CHANGE_FACT_VOCAB
        .iter()
        .map(|(key, class)| {
            (
                *key,
                Budget {
                    class: *class,
                    p99_us: change::class_default_p99(*class),
                },
            )
        })
        .collect()
}
