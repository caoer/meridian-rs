//! Rung-6 policy engine: compile rulesets-as-data, evaluate assertions under
//! declared budgets, and gate writes at the armed change plane.
//!
//! # Charter
//! **Owns:** ruleset compile/eval (`policy` / `policy_compile` / `policy_vocab`),
//! the 14-assertion vocabulary with declared budgets, and the blocking gate
//! at the armed change plane ([`gate`] / [`resolve_armed_law`]; laws.md).
//!
//! **Never does:** own a corpus index (borrows `model`'s, capability-gated),
//! contain rules (vocabulary is engine; rules are data), or reach disk (I/O-
//! free like `model` — callers inject file access for armed-set load).
//!
//! # Gate at the armed change plane (U4.2)
//! When armed, [`gate`] evaluates a [`Change`] through the workspace's own
//! law after CAS and before bytes land. Block-severity, drifted law, or
//! unloadable INDEX refuses with `{code, recovery}` from the §8 taxonomy.
//! Never-armed is a bit-for-bit no-op. Armed set is loaded from the workspace
//! path, never supplied by the caller.

use model::{CorpusIndex, Document};

/// The attested armed-set artifact — the INDEX successor (registration ruling § 4).
/// [`armed::arm`] turns a discovered page into an armed one: it narrows to an
/// [`armed::ArmRoot`], resolves through the landed resolver, and pins each
/// winner's page + [`page_rev`] into an [`armed::ArmedArtifact`] row keyed by
/// (id, arm root).
///
/// A module (not a flat re-export) so it does not collide with the folder
/// loader's `index::arm`.
pub mod armed;
pub mod armed_law;

mod binding;
mod change;
mod check_eval;
mod declaration;
/// I4 def-conformance (U8c): the write-time def validator — the pure verdict
/// over (prev, candidate) documents the put path consults before any splice.
/// Byte-exact against the U0 defs goldens.
pub mod defs;
mod gate;
mod hook;
mod middleware_eval;
mod pack;
mod reaction;
mod registration;
mod rule;

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
/// `syntax::`/`model::` type crosses the `build_facts` signature.
pub use pack::{FactDoc, facts_from_document};

/// The `rulepack-api@2` CHECK evaluation surface (U1.3): the full-`EvalLimits`
/// `check_change(change)` evaluator behind the convention loader. [`CheckLimits`]
/// meters all five guards (tick + heap + call-depth + source-size + nesting);
/// [`CheckError`] is its typed failure surface.
pub use check_eval::{CheckError, CheckLimits, CheckTelemetry};

/// The DOOR leg's evaluation surface (armed-plane Part A2): one metered
/// `middleware(ctx)` run per armed in-scope middleware page, its typed
/// emissions ([`MwEmit`]), and the injected overlay-world seam ([`MwWorld`]).
pub use middleware_eval::{
    MwCtxInput, MwEmit, MwOutcome, MwWorld, SqlRow, SqlValue, run_middleware,
};

/// The DECLARATION layer: what a rule page's legs say. [`LoadError`] is its typed
/// failure surface — it names the LEG, never a file, because a page has no
/// filename to name. A loaded [`Rule`] runs its check leg over a [`Change`],
/// returning the [`CheckOutcome`]'s [`Refusal`]s; a refusal always cites its
/// passing case.
pub use declaration::{
    CheckOutcome, LoadError, Refusal, expand_globs, first_member_order_fault, glob_match,
    glob_subsumes, is_glob_pattern,
};

/// The HOOK capability (U1.3): the emit leg's declaration. A [`Hook`] carries the
/// declared severity, scope, effect caps, per-eval budget, the VERBATIM `how:` block
/// (frozen data the engine never interprets), and a predicate whose capability
/// ceiling was enforced at load. [`evaluate_hooks`] runs armed, in-scope HOOKs and
/// returns advisory-only [`HookOutcome`]s; [`SLICE1_CAPS`] is what slice 1 admits.
pub use hook::{
    Hook, HookEvalError, HookFinding, HookOutcome, HookTestTelemetry, Intent, SLICE1_CAPS,
    evaluate_counterfactual_hooks_for_corpus_metered, evaluate_hooks, evaluate_hooks_for_test,
    evaluate_hooks_for_test_metered, evaluate_loaded_hooks, intent_from_effect,
};

/// The reaction-plane payload (C1a): [`derive_event`] turns a landed [`Change`]
/// into the `on_change(event)` argument a HOOK predicate reads. It attaches the
/// values behind `fields_changed` and the changed document's frontmatter, and it
/// drops `actor` — the engine must not observe the observer.
pub use reaction::derive_event;

/// Tag-indexed rule registration. A page registers by carrying `rules/hook` /
/// `rules/check` in its frontmatter `tags:` ([`RuleKind`]), is identified by its
/// frontmatter `id:` ([`RuleId`], § 2 grammar), and resolves against same-id
/// pages by mount depth along the three-rung scope ladder ([`ScopeLayer`] →
/// [`Scope`]). [`RuleIndex::discover`] reads a caller-supplied page feed
/// ([`PageRef`] — `policy` stays I/O-free); [`RuleIndex::resolve`] is the one
/// pure resolver both the arming path and the print verb call, and it retains
/// every shadowed candidate so the override chain stays printable. Nothing here
/// arms: discovery makes a page known, only the attested ARM act activates it.
pub use registration::{
    Collision, Effective, EffectiveSet, ID_KEY, IdFault, MAX_ID_LEN, PageRef,
    REGISTRATION_NAMESPACE, RegisterError, RegisterFault, Registration, RuleId, RuleIndex,
    RuleKind, Scope, ScopeLayer, governing_dirs, page_rev, register_page,
};

/// The page-shaped rule load — what a registered page becomes when it is
/// evaluated. [`register_page`] answers identity (the tag and the `id:`);
/// [`load_rule`] answers evaluability, parsing the legs the registration tags
/// declare through the same parsers the convention folder loader uses. It takes
/// the [`Registration`] rather than re-deriving one, and verifies the supplied
/// bytes against the registered rev.
pub use rule::{CounterfactualRule, Rule, RuleLoadError, load_rule, load_rule_for_corpus};

/// The blocking gate at the armed change plane (U4.2): the pure decision
/// ([`gate`]), whose input is the [`ArmedLaw`] that [`resolve_armed_law`]
/// resolved at the write's own path.
pub use gate::{GateFinding, GateOutcome, GateRefusal, GateViolation, gate};

/// A workspace's attested armed law at ONE path, and the ONE surface reporting
/// every way it could not be honored ([`ArmedFault`] — absent, corrupt, attesting
/// zero rows, a red row, a row that will not load, a law that will not evaluate).
/// [`resolve_armed_law`] pivots on the once-armed marker, so a workspace that has
/// ever been armed fails CLOSED rather than reading as "nothing armed".
pub use armed_law::{ArmedFault, ArmedLaw, ArmedRule, resolve_armed_law};

/// The binding law (U4.3): the door law that refuses a one-sided artifact↔page
/// change ([`classify_door_law`], taxonomy row 9) and the integrity floor that
/// refuses deletion/rename of the armed-rules artifact or the once-armed marker
/// (row 10, not force-escapable).
pub use binding::{ATTESTED_MARKER_PATH, BindingSide, DoorLaw, classify_door_law};

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
/// gated at load — never a wire change. The exact injected surface is documented
/// on `pack`.
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
/// Policy stays I/O-free (as `model` is): the caller — `fs`/the serving host
/// at the §6.1 "reads path from disk" edge — injects file access, and tests inject an
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

/// Budget classes place the evaluator: node/file run on an index-less engine
/// from rung 1 machinery; corpus requires the resident index (§11.3 — since
/// the sidecar host's DROP, §3.3, every wire door is daemon-backed, so the
/// class gates nothing at the wire; the law stands).
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
/// to `policy::compile`.
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

    /// Ruleset-level budget class = max class over used assertions. A `Corpus`
    /// result means the pack needs the resident corpus index (the `daemon_only`
    /// refusal that once enforced this at an index-less host is RETIRED — §8;
    /// every wire door is daemon-backed).
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

/// Parse the §11.3 manifest, verify the pin, and run the pack's fixtures under
/// its declared budgets as the load gate — a pack whose fixtures fail is never
/// admitted. YAML enters the engine here and nowhere else.
///
/// `source` is the manifest text (the caller pre-reads it from `pin.path`);
/// `files` resolves manifest-relative fixture/rule paths to their contents, so
/// policy performs no I/O of its own. `build_facts` turns a fixture's `(path,
/// body)` into the injected fact plane — the same parse→facts path production
/// `evaluate` sees, kept in policy vocabulary (`&str`s in, [`FactDoc`] out).
///
/// Pipeline: content-pin (sha256) → parse → api-pin → read+classify rules →
/// extract+parse fenced Starlark predicates → run fixtures → admit / refuse.
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

    // 4b. Extract + parse-check each rule page's fenced Starlark predicate.
    let predicates = pack::extract_predicates(&rule_sources, &manifest.rules)?;

    // 4c. §11.2 WHEN/HOW partition: a WHEN outside the closed fact vocabulary is
    // refused at compile, before any fixture runs.
    pack::check_when_vocab(&predicates)?;

    // 5. Fixtures are the load gate: no fixtures = cannot demonstrate itself.
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
/// path; mode/limit shaping is the serving host's wire concern) and return the §11.1
/// findings. Facts are derived from each real `Document` AST (`model::build` →
/// `FactDoc`); the WHEN vocabulary was closed at compile (§11.2), so every
/// predicate here reads world-model facts only.
///
/// Per-eval `{steps, mem}` budget is metered per document; exhaustion appends the
/// `budget_exceeded` FINDING to the returned verdicts and never surfaces as an
/// error or panic (frozen §8). The verdict order is document order, then
/// per-document rule order.
///
/// `corpus` is the capability parameter: `None` for an engine holding no
/// resident index. File/node-class rules need no index; corpus-class
/// admission is a load-time concern owned by P6-VERDICTS, not evaluated here.
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
/// one-hop cross-document); the `p99_us` is a monotonic class default
/// ([`change::class_default_p99`]), not a per-key measured SLA. The keys are
/// exactly the [purity-guarded vocabulary](change::CHANGE_FACT_VOCAB): every one
/// a pure function of the before/after states and pinned evidence, no
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
