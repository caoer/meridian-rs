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

/// A compiled ruleset, cached by content hash — the one in-memory cache the
/// laws permit (content-addressed, disposable, re-derived from disk).
#[derive(Debug)]
pub struct CompiledRuleset {
    _sealed: (),
}

/// Compile errors per the schema doc's error taxonomy (`ruleset_not_found`,
/// `pin_mismatch`, `compile_error` + unknown_assertions, `unsupported_vocab`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompileError {
    NotFound,
    PinMismatch { expected: String, actual: String },
    UnknownAssertions { names: Vec<String>, at_rules: Vec<String> },
    UnsupportedVocab { requires: u32, engine: u32 },
}

/// One finding: rule id, severity, and the violating node's wire coordinates
/// (path/span/node_rev/hpath) — findings, never decisions.
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

/// Read + verify pin + compile + cache. YAML enters the engine here and
/// nowhere else.
pub fn compile(pin: &RulesetPin, source: &str) -> Result<CompiledRuleset, CompileError> {
    let _ = (pin, source);
    todo!("rung 6: parse → validate schema → compile; cache by content sha256")
}

/// Evaluate a ruleset over documents (gate and diagnostic modes share this
/// path; mode/limit shaping is `sidecar`'s wire concern). `corpus` is the
/// capability parameter: `None` in sidecar mode.
pub fn evaluate(
    ruleset: &CompiledRuleset,
    docs: &[&Document],
    corpus: Option<&CorpusIndex>,
) -> Vec<Violation> {
    let _ = (ruleset, docs, corpus);
    todo!("rung 6: selector engine + assertion vocabulary over resident ASTs")
}

/// Axis-E entry point: may this splice stand, per the I3-shaped rules in
/// `ruleset`? `actor` arrives as pre-authorized data from Go (never resolved
/// here); the verdict goes back on the wire. See crate doc for the position
/// argument.
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
pub fn vocab() -> Vec<(&'static str, Budget)> {
    todo!("rung 6: the 14-assertion vocabulary with declared budgets")
}
