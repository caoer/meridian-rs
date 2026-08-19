//! HOOK capability declaration (`HOOK.md`) — the emit leg's load surface (U1.3).
//! A [`Hook`] carries severity, scope, effect caps, per-eval budget, the
//! VERBATIM `how:` block (frozen data; engine never interprets it), and a
//! predicate whose capability ceiling was enforced at load.
//!
//! [`evaluate_hooks`] runs armed, in-scope HOOKs and returns advisory-only
//! [`HookOutcome`]s. [`SLICE1_CAPS`] is what slice 1 admits. Hooks never
//! block a write; CHECKs do.

use std::collections::HashSet;

use effects::{
    ArgValue, CapabilitySet, ChangeEvent, Effect, EffectKind, EvalError, EvalLimits,
    REACTION_VOCAB, Rule,
};
use starlark::analysis::AstModuleLint;
use starlark::environment::GlobalsBuilder;
use starlark::syntax::{AstModule, Dialect};

use crate::EvalBudget;
use crate::check_eval::CheckLimits;
use crate::declaration::LoadError;

/// The caps slice 1 admits — `proto.send` and nothing else. Every other descriptor
/// kind is a NAMED deferral at load ([`LoadError::HookCapDeferred`]), never a silent
/// ignore: a convention that declares `md.set_field` is told the cap exists and that
/// this slice does not carry it.
pub const SLICE1_CAPS: [EffectKind; 1] = [EffectKind::Send];

/// Extra descriptor kinds the corpus tier may evaluate while proving quiescence.
/// This is not an arming allowlist: only the explicitly named corpus loader can use
/// it, and the ordinary loader remains pinned to [`SLICE1_CAPS`].
const CORPUS_COUNTERFACTUAL_CAPS: [EffectKind; 3] = [
    EffectKind::SetField,
    EffectKind::AppendSection,
    EffectKind::Send,
];

/// The per-eval budget a `caps: []` declaration gets when it declares none. A
/// zero-cap predicate still evaluates (and can exhaust fuel), so an absent
/// `budget:` must not silently become unlimited — it becomes this named ceiling,
/// the fleet's conventional `{ steps: 10000, mem: 4194304 }`, so an explicit
/// declaration and an omitted one bound the predicate identically.
const ZERO_CAP_DEFAULT_BUDGET: EvalBudget = EvalBudget {
    steps: 10_000,
    mem: 4_194_304,
};

/// A loaded `HOOK.md`: its declared scope, severity, caps, per-eval budget, the
/// verbatim `how:` block, and the parse- and ceiling-validated predicate.
/// Public construction is sealed to [`load_hook`], so a publicly reachable `Hook`
/// has passed the slice-1 cap gate. The corpus-only widened constructor remains
/// private and its value is sealed inside [`crate::CounterfactualRule`]; no
/// widened `Hook` crosses this crate's ordinary policy surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hook {
    severity: String,
    scope: Vec<String>,
    caps: Vec<EffectKind>,
    budget: EvalBudget,
    how: String,
    source: String,
}

impl Hook {
    /// The declared severity (`info` / `warn` / `error` — routed by `how:`, which the
    /// engine does not read).
    #[must_use]
    pub fn severity(&self) -> &str {
        &self.severity
    }

    /// The `paths:` scope globs the HOOK declared.
    #[must_use]
    pub fn scope(&self) -> &[String] {
        &self.scope
    }

    /// Whether `path` is in the HOOK's declared scope. Same glob grammar as
    /// [`crate::Rule::matches_path`] — the one matcher, reused.
    #[must_use]
    pub fn matches_path(&self, path: &str) -> bool {
        crate::declaration::path_in_scope(&self.scope, path)
    }

    /// The declared effect caps, resolved to the closed descriptor surface.
    #[must_use]
    pub fn caps(&self) -> &[EffectKind] {
        &self.caps
    }

    /// The declared per-eval `{steps, mem}` budget (wire §11.3's pack-layer budget —
    /// distinct from the kernel's outer containment ceiling).
    #[must_use]
    pub fn budget(&self) -> EvalBudget {
        self.budget
    }

    /// The `how:` block **verbatim**, exactly the bytes the page carried. Frozen
    /// data: the engine validated its shape and never read its meaning.
    #[must_use]
    pub fn how(&self) -> &str {
        &self.how
    }

    /// The reaction predicate's Starlark source.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }
}

/// One armed reaction descriptor after the HOOK's capability ceiling has admitted
/// it. The descriptor says only what was armed. Delivery state has no place in this
/// type because delivery has not happened yet.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Intent {
    /// The armed convention that emitted this intent.
    pub rule_id: String,
    /// Outcome-wide emission order for this change.
    pub seq: u32,
    /// The action exactly as the predicate wrote it (`notify` is not rewritten).
    pub action: String,
    /// The opaque target, carried without resolution.
    pub target: Option<String>,
    /// The opaque classification, carried without ranking.
    pub severity: Option<String>,
    /// The opaque payload, carried without rendering.
    pub payload: Option<String>,
    /// The receipt address minted before delivery.
    pub receipt: String,
}

/// A named advisory finding from one HOOK evaluation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HookFinding {
    /// The predicate exhausted its declared per-eval budget. Its partial effects are
    /// discarded, but later HOOKs still evaluate over the same landed change.
    BudgetExceeded {
        /// The convention whose predicate exhausted its budget.
        rule_id: String,
        /// The declared Starlark step ceiling.
        steps: u64,
        /// The declared eval-heap byte ceiling.
        mem: u64,
    },
}

/// One in-scope HOOK's advisory result. This type has no refusal arm and no
/// conversion into the blocking gate: a reaction cannot veto or mutate the landed
/// write. `narrowed` names complete descriptors the declared caps dropped, while
/// `findings` names evaluation conditions that produced no descriptor.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct HookOutcome {
    /// Intents admitted by the HOOK's declared caps.
    pub intents: Vec<Intent>,
    /// Intents dropped by the HOOK's declared caps, retained as report data.
    pub narrowed: Vec<Intent>,
    /// Advisory evaluation findings, such as `budget_exceeded`.
    pub findings: Vec<HookFinding>,
    /// The declaration's `how:` block, byte-for-byte and uninterpreted.
    pub how: String,
}

/// A fault while evaluating an already-loaded HOOK.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookEvalError {
    /// The sealed effects evaluator rejected or exhausted the predicate.
    Eval(EvalError),
    /// A descriptor did not have the reaction-plane intent shape. In particular,
    /// every armed intent must name its action and pre-delivery receipt.
    MalformedIntent {
        /// The emitting convention.
        rule_id: String,
        /// The shape fault.
        reason: String,
    },
}

impl std::fmt::Display for HookEvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Eval(error) => error.fmt(f),
            Self::MalformedIntent { rule_id, reason } => {
                write!(f, "HOOK `{rule_id}` emitted a malformed intent: {reason}")
            }
        }
    }
}

impl std::error::Error for HookEvalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Eval(error) => Some(error),
            Self::MalformedIntent { .. } => None,
        }
    }
}

impl From<EvalError> for HookEvalError {
    fn from(error: EvalError) -> Self {
        Self::Eval(error)
    }
}

/// Evaluate every armed, in-scope HOOK over one landed change.
///
/// The input is [`crate::ArmedRule`], not a bare [`Hook`], so an unarmed
/// declaration cannot enter by accident. Each predicate runs through
/// [`effects::eval_with_limits`] under its declared [`EvalBudget`]. The declared
/// caps then partition the result through [`CapabilitySet::route`]; denied intents
/// remain in `narrowed` as named report data. Sequence numbers are reassigned across
/// the complete outcome, so the first intent from two different HOOKs cannot both
/// be `0`.
///
/// One [`HookOutcome`] is returned per armed, in-scope HOOK in input order. A
/// check-only convention and an out-of-scope HOOK are silent.
///
/// Budget exhaustion is returned as a named [`HookFinding::BudgetExceeded`] and
/// evaluation continues with later HOOKs.
///
/// # Errors
/// A non-budget predicate fault returns [`HookEvalError::Eval`]. A descriptor that
/// does not carry the canonical receipt for the landed change returns
/// [`HookEvalError::MalformedIntent`]; it is never silently discarded.
pub fn evaluate_hooks(
    armed: &[crate::ArmedRule],
    event: &ChangeEvent,
) -> Result<Vec<HookOutcome>, HookEvalError> {
    evaluate_loaded_hooks(
        armed
            .iter()
            .filter_map(|armed| armed.rule().hook().map(|hook| (armed.id().as_str(), hook))),
        event,
    )
}

/// One production-shaped pre-arming HOOK outcome with its exact evaluator meter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookTestTelemetry {
    /// The rule evaluated for this row, even when it emitted nothing.
    pub rule_id: String,
    /// The canonical intent projection, including `narrowed` and typed findings.
    pub outcome: HookOutcome,
    /// Starlark ticks spent by this one HOOK evaluation.
    pub fuel_used: u64,
    /// Peak evaluator heap for this one HOOK evaluation.
    pub mem_used: u64,
}

/// Evaluate loaded conventions before arming for the tier-1 scenario proof.
///
/// This is the same policy-owned descriptor evaluator and intent projection as
/// [`evaluate_hooks`], but its input is a loaded [`crate::Rule`] rather than an
/// attested artifact row. It grants no arming state and applies no effect. The scenario
/// runner uses it only after routing the declared `^put` through the production write
/// path, so `t.result.effects` observes what that landed change would arm.
///
/// # Errors
/// The same [`HookEvalError`] surface as [`evaluate_hooks`].
pub fn evaluate_hooks_for_test(
    rules: &[crate::Rule],
    event: &ChangeEvent,
) -> Result<Vec<HookOutcome>, HookEvalError> {
    evaluate_hooks_for_test_metered(rules, event)
        .map(|rows| rows.into_iter().map(|row| row.outcome).collect())
}

/// The tier-1 evaluator plus exact meters for `test --corpus`.
///
/// # Errors
/// The same [`HookEvalError`] surface as [`evaluate_hooks`].
pub fn evaluate_hooks_for_test_metered(
    rules: &[crate::Rule],
    event: &ChangeEvent,
) -> Result<Vec<HookTestTelemetry>, HookEvalError> {
    evaluate_loaded_hooks_metered(
        rules
            .iter()
            .filter_map(|rule| rule.hook().map(|hook| (rule.id().as_str(), hook))),
        event,
        EvalLimits::default().max_depth,
    )?
    .into_iter()
    .map(project_hook_telemetry)
    .collect()
}

/// Evaluate widened declarations only inside the corpus proof.
///
/// This API accepts only [`crate::CounterfactualRule`], so widened caps cannot
/// cross into ordinary policy code. Everything else is the production evaluator:
/// metering, capability routing, global sequencing, typed findings, and the same
/// canonical [`intent_from_effect`] projection for every domain. A raw `md.*`
/// descriptor carrying no canonical action or receipt is
/// [`HookEvalError::MalformedIntent`] here exactly as it is in production; the
/// proof executes the projected [`Intent`] through the `run` adapter and the
/// production batch executor. Explicit graph fuel replaces runtime depth
/// suppression.
///
/// # Errors
/// A non-budget predicate fault has the same [`HookEvalError::Eval`] surface as the
/// production evaluator, and a descriptor outside the canonical intent shape the same
/// [`HookEvalError::MalformedIntent`].
pub fn evaluate_counterfactual_hooks_for_corpus_metered(
    rules: &[crate::CounterfactualRule],
    event: &ChangeEvent,
) -> Result<Vec<HookTestTelemetry>, HookEvalError> {
    evaluate_loaded_hooks_metered(
        rules
            .iter()
            .filter_map(|rule| rule.hook().map(|hook| (rule.id(), hook))),
        event,
        u32::MAX,
    )?
    .into_iter()
    .map(project_hook_telemetry)
    .collect()
}

struct RawHookTelemetry {
    rule_id: String,
    effects: Vec<Effect>,
    narrowed: Vec<Effect>,
    findings: Vec<HookFinding>,
    how: String,
    expected_receipt: String,
    fuel_used: u64,
    mem_used: u64,
}

/// Evaluate already-loaded HOOKs against one landed change — the ARM-artifact
/// consumer's evaluator.
///
/// [`evaluate_hooks`] takes an already-resolved armed set, whose rules the law
/// resolver loaded. A caller holding bare pages loads them itself ([`load_hook`],
/// which is the capability gate) and hands the pairs here. Same evaluator, same
/// sequencing, same budget metering — the only difference is who resolved the
/// armed set.
///
/// Each hook still answers scope through its own `paths:` declaration, so an armed
/// row whose page does not match the changed file emits nothing.
///
/// # Errors
/// [`HookEvalError`] when an already-loaded predicate faults at evaluation.
pub fn evaluate_loaded_hooks<'a>(
    hooks: impl IntoIterator<Item = (&'a str, &'a Hook)>,
    event: &ChangeEvent,
) -> Result<Vec<HookOutcome>, HookEvalError> {
    evaluate_loaded_hooks_metered(hooks, event, EvalLimits::default().max_depth)?
        .into_iter()
        .map(project_hook_telemetry)
        .map(|row| row.map(|row| row.outcome))
        .collect()
}

fn evaluate_loaded_hooks_metered<'a>(
    hooks: impl IntoIterator<Item = (&'a str, &'a Hook)>,
    event: &ChangeEvent,
    max_depth: u32,
) -> Result<Vec<RawHookTelemetry>, HookEvalError> {
    let mut outcomes = Vec::new();
    let mut next_seq = 0_u32;

    for (rule_id, hook) in hooks {
        if !hook.matches_path(&event.file) {
            continue;
        }

        let limits = EvalLimits {
            fuel: hook.budget.steps,
            mem: hook.budget.mem,
            max_depth,
            ..EvalLimits::default()
        };
        let telemetry =
            effects::eval_telemetry(&[Rule::new(rule_id, hook.source())], event, limits)
                .pop()
                .expect("one HOOK produces one telemetry row");
        let (mut effects, findings) = match telemetry.outcome {
            Ok(effects) => (effects, Vec::new()),
            Err(EvalError::Budget { fuel, mem }) => (
                Vec::new(),
                vec![HookFinding::BudgetExceeded {
                    rule_id: rule_id.to_string(),
                    steps: fuel,
                    mem,
                }],
            ),
            Err(error) => return Err(error.into()),
        };
        for effect in &mut effects {
            effect.seq = next_seq;
            next_seq = next_seq
                .checked_add(1)
                .ok_or_else(|| HookEvalError::MalformedIntent {
                    rule_id: rule_id.to_string(),
                    reason: "outcome sequence exceeds u32".to_string(),
                })?;
        }

        let caps: CapabilitySet = hook.caps.iter().copied().collect();
        let (effects, narrowed) = caps.route(effects);
        outcomes.push(RawHookTelemetry {
            rule_id: rule_id.to_string(),
            effects,
            narrowed,
            findings,
            how: hook.how.clone(),
            expected_receipt: effects::receipt_address(&event.file, &event.fingerprint_after),
            fuel_used: telemetry.fuel_used,
            mem_used: telemetry.mem_used,
        });
    }

    Ok(outcomes)
}

fn project_hook_telemetry(row: RawHookTelemetry) -> Result<HookTestTelemetry, HookEvalError> {
    Ok(HookTestTelemetry {
        rule_id: row.rule_id,
        outcome: HookOutcome {
            intents: row
                .effects
                .into_iter()
                .map(|effect| intent_from_effect(effect, &row.expected_receipt))
                .collect::<Result<_, _>>()?,
            narrowed: row
                .narrowed
                .into_iter()
                .map(|effect| intent_from_effect(effect, &row.expected_receipt))
                .collect::<Result<_, _>>()?,
            findings: row.findings,
            how: row.how,
        },
        fuel_used: row.fuel_used,
        mem_used: row.mem_used,
    })
}

/// The production Effect→Intent conversion — the ONE canonical projection every
/// emitted descriptor passes before it can be called armed.
///
/// It validates the receipt against the landed change's canonical address, the
/// declared `action` against the emitted kind, and the argument surface against
/// the closed intent shape. Public because the pre-arming corpus proof needs the
/// production seam rather than a proof-only copy: raw `md.*` descriptors must
/// fault here exactly as they do in production, and the canonical `Intent` this
/// returns is what the `run` adapter turns into the executor's `ApplyRequest`.
///
/// # Errors
/// [`HookEvalError::MalformedIntent`] — a missing/forged receipt, an `action` that
/// does not name the emitted kind, a non-string argument, or an argument outside the
/// closed intent surface.
pub fn intent_from_effect(effect: Effect, expected_receipt: &str) -> Result<Intent, HookEvalError> {
    let rule_id = effect.rule_id;
    let mut args = effect.args;
    let action = take_required(&mut args, &rule_id, "action")?;
    let receipt = take_required(&mut args, &rule_id, "receipt")?;
    if receipt != expected_receipt {
        return Err(HookEvalError::MalformedIntent {
            rule_id,
            reason: format!(
                "receipt {receipt:?} does not name this landed change; expected {expected_receipt:?}"
            ),
        });
    }
    if effects::action_kind(&action) != Some(effect.kind) {
        return Err(HookEvalError::MalformedIntent {
            rule_id,
            reason: format!(
                "action {action:?} does not name the emitted kind `{}`",
                effect.kind.as_str()
            ),
        });
    }
    let target = take_string(&mut args, &rule_id, "target")?;
    let severity = take_string(&mut args, &rule_id, "severity")?;
    let payload = take_string(&mut args, &rule_id, "payload")?;
    if let Some((name, _)) = args.into_iter().next() {
        return Err(HookEvalError::MalformedIntent {
            rule_id,
            reason: format!("unexpected argument {name:?}"),
        });
    }

    Ok(Intent {
        rule_id,
        seq: effect.seq,
        action,
        target,
        severity,
        payload,
        receipt,
    })
}

/// An OPTIONAL scalar string argument: absent stays absent, a list is a fault.
fn take_string(
    args: &mut std::collections::BTreeMap<String, ArgValue>,
    rule_id: &str,
    name: &str,
) -> Result<Option<String>, HookEvalError> {
    match args.remove(name) {
        Some(ArgValue::Str(value)) => Ok(Some(value)),
        Some(ArgValue::List(_)) => Err(HookEvalError::MalformedIntent {
            rule_id: rule_id.to_string(),
            reason: format!("argument {name:?} must be a string"),
        }),
        None => Ok(None),
    }
}

/// A REQUIRED scalar string argument. Typed as `String` rather than an unwrapped
/// `Option`, so "this one is required" is the signature rather than a flag whose
/// contract a caller has to re-establish with an `expect`.
fn take_required(
    args: &mut std::collections::BTreeMap<String, ArgValue>,
    rule_id: &str,
    name: &str,
) -> Result<String, HookEvalError> {
    take_string(args, rule_id, name)?.ok_or_else(|| HookEvalError::MalformedIntent {
        rule_id: rule_id.to_string(),
        reason: format!("missing required argument {name:?}"),
    })
}

/// Parse and validate a rule page's HOOK leg under `limits`.
///
/// Pipeline: frontmatter grammar (`severity`/`paths`/`caps`/`budget`/`how` —
/// `budget:`/`how:` required only when `caps:` is non-empty; a `caps: []` page
/// can emit nothing, so both are unreachable and may be omitted) → `how:`
/// shape → caps against the slice-1 allowlist → the fenced predicate →
/// `effects::validate` under the full limits → the capability ceiling.
///
/// Crate-private on purpose: the kind seam has ONE enforcement point,
/// [`crate::load_rule`] (and its corpus twin) — a leg loader reachable from
/// outside `policy` would be a second way to turn a page into a rule that skips
/// it. No `kind:` assertion here: the tag `rules/hook` IS the kind (ruling § 1),
/// checked only at the kind seam.
///
/// # Errors
/// [`LoadError::HookMalformed`], [`LoadError::HookCapDeferred`],
/// [`LoadError::HookPredicateInvalid`], [`LoadError::HookCeiling`].
pub(crate) fn load_hook(hook_md: &str, limits: CheckLimits) -> Result<Hook, LoadError> {
    load_hook_with_caps(hook_md, limits, &SLICE1_CAPS)
}

/// Load a HOOK leg for `mrd test --corpus` counterfactual chaining. The ordinary
/// loader never reaches this allowlist, so admitting `md.*` here cannot widen the
/// armed runtime's [`SLICE1_CAPS`].
pub(crate) fn load_hook_for_corpus(hook_md: &str, limits: CheckLimits) -> Result<Hook, LoadError> {
    load_hook_with_caps(hook_md, limits, &CORPUS_COUNTERFACTUAL_CAPS)
}

fn load_hook_with_caps(
    hook_md: &str,
    limits: CheckLimits,
    allowed_caps: &[EffectKind],
) -> Result<Hook, LoadError> {
    let malformed = |reason: String| LoadError::HookMalformed { reason };

    let (frontmatter, _body) = crate::pack::split_frontmatter(hook_md)
        .ok_or_else(|| malformed("no `---` frontmatter".to_string()))?;
    let parsed: HookFrontmatter = serde_yaml::from_str(frontmatter)
        .map_err(|e| malformed(format!("frontmatter parse: {e}")))?;

    let severity = parsed
        .severity
        .ok_or_else(|| malformed("frontmatter must declare `severity:`".to_string()))?;

    let scope = parsed
        .paths
        .ok_or_else(|| malformed("frontmatter must declare a `paths:` scope".to_string()))?;
    if scope.is_empty() {
        return Err(malformed(
            "`paths:` is empty — a HOOK with no scope reacts to nothing".to_string(),
        ));
    }

    let declared_caps = parsed
        .caps
        .ok_or_else(|| malformed("frontmatter must declare `caps:`".to_string()))?;
    let caps = resolve_caps(&declared_caps, allowed_caps)?;

    // `budget:` and `how:` are required exactly when the declaration can DO
    // something: a `caps: []` page emits no intent, so its delivery config and
    // per-eval ceiling are unreachable by construction and may be omitted.
    // When declared anyway, both are honored as before.
    let budget = match parsed.budget {
        Some(budget) => budget,
        None if caps.is_empty() => ZERO_CAP_DEFAULT_BUDGET,
        None => {
            return Err(malformed(
                "frontmatter must declare `budget: {steps, mem}` when `caps:` is non-empty"
                    .to_string(),
            ));
        }
    };

    let how = match parsed.how {
        Some(how_value) => {
            validate_how(&how_value).map_err(malformed)?;
            extract_how_block(frontmatter)
                .ok_or_else(|| {
                    malformed("`how:` is not a block this loader can carry verbatim".to_string())
                })?
                .to_string()
        }
        None if caps.is_empty() => String::new(),
        None => {
            return Err(malformed(
                "frontmatter must declare `how:` when `caps:` is non-empty".to_string(),
            ));
        }
    };

    let source = crate::pack::extract_fenced_starlark(hook_md).ok_or_else(|| {
        malformed("no fenced ```starlark block defining the reaction predicate".to_string())
    })?;

    // The load gate, in the predicate's OWN language: `effects::validate` applies the
    // source-size, nesting and parse guards the effect kernel applies, under the full
    // limits — the same separation of authoring faults from per-change faults that
    // `check_eval::validate_check_source` gives CHECK.
    effects::validate(
        &[Rule::new("HOOK.md", source.clone())],
        eval_limits_from(limits),
    )
    .map_err(|e| LoadError::HookPredicateInvalid {
        reason: e.to_string(),
    })?;

    check_ceiling(&source, &caps)?;

    Ok(Hook {
        severity,
        scope,
        caps,
        budget,
        how,
        source,
    })
}

/// Only the HOOK frontmatter keys the loader reads. Other keys are permitted (a
/// declaration may carry descriptive frontmatter) and ignored — `kind:` among
/// them, deliberately: it is checked against the tag at exactly one place,
/// [`crate::load_rule`]'s kind seam, and a field here would be a second reader.
#[derive(serde::Deserialize)]
struct HookFrontmatter {
    severity: Option<String>,
    paths: Option<Vec<String>>,
    caps: Option<Vec<String>>,
    budget: Option<EvalBudget>,
    how: Option<serde_yaml::Value>,
}

/// Resolve declared cap strings against the closed descriptor surface, then against
/// the slice-1 allowlist. An unknown name is malformed (the surface is closed, so a
/// name outside it is a typo or a later vocabulary, never a guess); a known cap
/// outside slice 1 is a named deferral.
fn resolve_caps(
    declared: &[String],
    allowed_caps: &[EffectKind],
) -> Result<Vec<EffectKind>, LoadError> {
    let mut out = Vec::with_capacity(declared.len());
    for name in declared {
        let kind = EffectKind::from_wire_name(name).ok_or_else(|| LoadError::HookMalformed {
            reason: format!(
                "`caps:` names {name:?}, which is not an effect kind — the closed surface is [{}]",
                EffectKind::ALL
                    .iter()
                    .map(|k| k.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        })?;
        if !allowed_caps.contains(&kind) {
            return Err(LoadError::HookCapDeferred {
                cap: kind.as_str().to_string(),
            });
        }
        if !out.contains(&kind) {
            out.push(kind);
        }
    }
    Ok(out)
}

/// Validate the SHAPE of `how:` without reading its meaning. `how:` must be a
/// mapping; the three keys slice 1 names must have the shapes slice 1 names, IF they
/// are present. Every other key passes through untouched — see the module header:
/// ruling on an unknown key would be interpreting data this engine does not own.
fn validate_how(how: &serde_yaml::Value) -> Result<(), String> {
    let map = how
        .as_mapping()
        .ok_or_else(|| "`how:` must be a mapping of delivery data".to_string())?;

    if let Some(route) = map.get(serde_yaml::Value::from("route")) {
        let route = route
            .as_mapping()
            .ok_or_else(|| "`how: route:` must be a mapping of severity to channel".to_string())?;
        for (severity, channel) in route {
            if !severity.is_string() || !channel.is_string() {
                return Err(
                    "`how: route:` maps a severity NAME to a channel NAME — both are strings"
                        .to_string(),
                );
            }
        }
    }
    for key in ["batching", "wake_policy"] {
        if let Some(value) = map.get(serde_yaml::Value::from(key))
            && !value.is_string()
        {
            return Err(format!("`how: {key}:` must be a string"));
        }
    }
    Ok(())
}

/// The verbatim `how:` block from the raw frontmatter text — the `how:` line through
/// the last line indented under it. Byte-exact by construction: this returns a SLICE
/// of the page, never a re-serialization, which is what makes "carried, not
/// interpreted" checkable rather than merely claimed.
fn extract_how_block(frontmatter: &str) -> Option<&str> {
    let mut start = None;
    let mut end = frontmatter.len();
    let mut at = 0;
    for line in frontmatter.split_inclusive('\n') {
        let is_top_level_key = !line.starts_with([' ', '\t']) && !line.trim().is_empty();
        match start {
            None => {
                if is_top_level_key && line.trim_end().starts_with("how:") {
                    start = Some(at);
                }
            }
            Some(_) => {
                if is_top_level_key {
                    end = at;
                    break;
                }
            }
        }
        at += line.len();
    }
    start.map(|s| &frontmatter[s..end])
}

/// The capability ceiling, enforced at LOAD. Statically resolve the predicate's free
/// names against the Starlark standard library, the non-capability reaction vocabulary,
/// and exactly the constructors the declared caps grant. Any other global name is a
/// reach the ceiling does not admit.
///
/// `using-undefined` is starlark's own name resolution, so locals, parameters and
/// comprehension bindings are tracked properly and attribute access (`event.path`) is
/// out of scope by construction — the boundary this enforces is the set of GLOBAL
/// names a predicate may reach for, which is exactly where the capability line lives.
fn check_ceiling(source: &str, caps: &[EffectKind]) -> Result<(), LoadError> {
    let mut allowed: HashSet<String> = GlobalsBuilder::standard()
        .build()
        .names()
        .map(|n| n.as_str().to_owned())
        .collect();
    allowed.extend(REACTION_VOCAB.iter().map(|name| (*name).to_owned()));
    for cap in caps {
        allowed.insert(cap.constructor().to_owned());
    }

    let ast = AstModule::parse("HOOK.md", source.to_owned(), &Dialect::Standard).map_err(|e| {
        LoadError::HookPredicateInvalid {
            reason: e.to_string(),
        }
    })?;
    let reaches: Vec<String> = ast
        .lint(Some(&allowed))
        .iter()
        .filter(|l| l.short_name == "using-undefined")
        .map(|l| undefined_name(&l.problem))
        .collect();
    if reaches.is_empty() {
        return Ok(());
    }
    Err(LoadError::HookCeiling {
        reason: format!(
            "the predicate reaches for {} outside its declared caps [{}]. Declare the cap that \
             grants it, or drop the call — an undeclared capability is denied, never ignored",
            reaches.join(", "),
            caps.iter()
                .map(|c| c.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    })
}

/// The bare name out of starlark's "Use of undefined variable" lint text, so the
/// refusal reads as a list of names rather than a list of sentences. Falls back to
/// the whole message if the shape ever changes — a refusal that degrades to verbose
/// beats one that silently drops the name it exists to report.
fn undefined_name(problem: &str) -> String {
    problem
        .split_once('`')
        .and_then(|(_, rest)| rest.rsplit_once('`').map(|(name, _)| format!("`{name}`")))
        .unwrap_or_else(|| problem.to_string())
}

/// The effect kernel's limit shape from the loader's. One knob at the call site: a
/// HOOK's source is bounded by the same size/nesting/fuel ceiling a CHECK's is.
fn eval_limits_from(limits: CheckLimits) -> EvalLimits {
    EvalLimits {
        fuel: limits.fuel,
        mem: limits.mem,
        max_call_depth: limits.max_call_depth,
        max_source_bytes: limits.max_source_bytes,
        ..EvalLimits::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use effects::{ChangeFact, ChangeFactKind, EventFacts};

    /// The founding `HOOK.md` declaration (design §5.2), verbatim in its
    /// frontmatter; its tiny direct-`send` predicate is the loader baseline.
    const FOUNDING_HOOK: &str = "\
---
kind: hook
severity: info
paths: [\"tasks/*.md\"]
caps:  [proto.send]
budget: { steps: 10000, mem: 4194304 }
how:
  route:    { info: channel-review, error: telegram }
  batching: 30s
  wake_policy: never-cold
---

# task-status-notify — a task moves to review → the card's reviewer reacts

```starlark
def on_change(event):
    send(to = [\"reviewer\"], message = \"task → review\")
```
";

    /// The `how:` block exactly as the page carries it — the byte-identical
    /// round-trip target. Written out separately ON PURPOSE: comparing the loaded
    /// value against a slice computed from the fixture would let both drift
    /// together, so the expectation is spelled here independently.
    const FOUNDING_HOW: &str = "\
how:
  route:    { info: channel-review, error: telegram }
  batching: 30s
  wake_policy: never-cold
";

    fn load(md: &str) -> Result<Hook, LoadError> {
        load_hook(md, CheckLimits::default())
    }

    /// A `caps: []` declaration in the fleet's production shape, with `how:` and
    /// `budget:` OMITTED — the keys are unreachable by construction on a page
    /// that can emit nothing, so the loader must not demand them.
    const ZERO_CAP_HOOK: &str = "\
---
kind: hook
severity: info
paths: [\"tasks/*.md\"]
caps: []
---

# stop-check-probe — observes, emits nothing

```starlark
def on_change(event):
    pass
```
";

    /// Direction one: a zero-cap page without `how:`/`budget:` LOADS. The absent
    /// budget becomes the named default ceiling (never unlimited) and the absent
    /// `how:` is carried as the empty block.
    #[test]
    fn a_zero_cap_page_loads_without_how_or_budget() {
        let hook = load(ZERO_CAP_HOOK).expect("a `caps: []` page loads without `how:`/`budget:`");
        assert_eq!(hook.caps(), &[] as &[EffectKind]);
        assert_eq!(
            hook.budget(),
            ZERO_CAP_DEFAULT_BUDGET,
            "an omitted `budget:` on a zero-cap page is the named default, not unlimited"
        );
        assert_eq!(hook.how(), "", "no `how:` bytes on the page, none carried");
    }

    /// A zero-cap page that DOES declare `how:`/`budget:` keeps today's behavior:
    /// both are honored, and `how:` still round-trips byte-identically.
    #[test]
    fn a_zero_cap_page_declaring_the_keys_is_honored_unchanged() {
        let page = ZERO_CAP_HOOK.replace(
            "caps: []\n",
            "caps: []\nbudget: { steps: 77, mem: 4096 }\nhow:\n  batching: 30s\n",
        );
        let hook = load(&page).expect("declared keys still load on a zero-cap page");
        assert_eq!(
            hook.budget(),
            EvalBudget {
                steps: 77,
                mem: 4096
            }
        );
        assert_eq!(hook.how(), "how:\n  batching: 30s\n");
    }

    /// Direction two: a page whose caps are NON-empty still refuses loudly when
    /// `how:` or `budget:` is missing, and the refusal names the field and the
    /// condition that makes it required.
    #[test]
    fn a_page_with_caps_still_refuses_without_how_or_budget() {
        let missing_how = FOUNDING_HOOK.replace(FOUNDING_HOW, "");
        let err = load(&missing_how).expect_err("caps non-empty, `how:` absent → refusal");
        let LoadError::HookMalformed { reason } = &err else {
            panic!("expected HookMalformed, got {err:?}");
        };
        println!("POPULATION refusal (how)    = {reason}");
        assert!(
            reason.contains("`how:`") && reason.contains("`caps:` is non-empty"),
            "the refusal names the field and its condition: {reason}"
        );

        let missing_budget = FOUNDING_HOOK.replace("budget: { steps: 10000, mem: 4194304 }\n", "");
        let err = load(&missing_budget).expect_err("caps non-empty, `budget:` absent → refusal");
        let LoadError::HookMalformed { reason } = &err else {
            panic!("expected HookMalformed, got {err:?}");
        };
        println!("POPULATION refusal (budget) = {reason}");
        assert!(
            reason.contains("`budget:") && reason.contains("`caps:` is non-empty"),
            "the refusal names the field and its condition: {reason}"
        );
    }

    /// The card's first gate: the founding declaration's frontmatter loads —
    /// `kind`/`severity`/`paths`/`caps`/`budget`/`how` all present and readable —
    /// and `how:` round-trips BYTE-IDENTICALLY, which is the proof it was carried
    /// rather than interpreted.
    #[test]
    fn founding_declaration_loads_with_how_byte_identical() {
        let hook = load(FOUNDING_HOOK).expect("the founding declaration loads");
        println!("POPULATION severity = {:?}", hook.severity());
        println!("POPULATION scope    = {:?}", hook.scope());
        println!("POPULATION caps     = {:?}", hook.caps());
        println!("POPULATION budget   = {:?}", hook.budget());
        println!("POPULATION how      = {:?}", hook.how());

        assert_eq!(hook.severity(), "info");
        assert_eq!(hook.scope(), &["tasks/*.md".to_string()]);
        assert_eq!(hook.caps(), &[EffectKind::Send]);
        assert_eq!(
            hook.budget(),
            EvalBudget {
                steps: 10000,
                mem: 4_194_304
            }
        );
        assert_eq!(
            hook.how(),
            FOUNDING_HOW,
            "`how:` must survive byte for byte — it is frozen data, not a re-serialization"
        );
        assert!(
            FOUNDING_HOOK.contains(hook.how()),
            "the carried `how:` must be a literal slice of the page"
        );
        assert!(hook.matches_path("tasks/x.md"), "declared scope matches");
        assert!(
            !hook.matches_path("notes/x.md"),
            "outside the declared scope is not the HOOK's concern"
        );
    }

    /// A MUTATION CONTROL PER CONJUNCT. Loading is a conjunction over six
    /// frontmatter keys plus the fenced predicate; each row removes or corrupts
    /// exactly ONE of them and nothing else, so every conjunct is shown to be
    /// load-bearing on its own. A refusal must name the field it is about.
    #[test]
    fn each_conjunct_is_load_bearing_and_names_its_field() {
        // Owned strings so each mutation is exactly one edit off the same baseline.
        // `kind:` is not among the conjuncts: it is checked once, at `load_rule`'s
        // kind seam, and asserting it here would rebuild a second enforcement point.
        let mutations: Vec<(&str, String, &str)> = vec![
            (
                "severity absent",
                FOUNDING_HOOK.replace("severity: info\n", ""),
                "severity",
            ),
            (
                "paths absent",
                FOUNDING_HOOK.replace("paths: [\"tasks/*.md\"]\n", ""),
                "paths",
            ),
            (
                "paths empty",
                FOUNDING_HOOK.replace("paths: [\"tasks/*.md\"]", "paths: []"),
                "paths",
            ),
            (
                "caps absent",
                FOUNDING_HOOK.replace("caps:  [proto.send]\n", ""),
                "caps",
            ),
            (
                "budget absent",
                FOUNDING_HOOK.replace("budget: { steps: 10000, mem: 4194304 }\n", ""),
                "budget",
            ),
            ("how absent", FOUNDING_HOOK.replace(FOUNDING_HOW, ""), "how"),
            (
                "how is a scalar, not a mapping",
                FOUNDING_HOOK.replace(FOUNDING_HOW, "how: telegram\n"),
                "how",
            ),
            (
                "how: route maps to a non-string",
                FOUNDING_HOOK.replace(
                    "route:    { info: channel-review, error: telegram }",
                    "route:    { info: [a, b] }",
                ),
                "route",
            ),
            (
                "how: batching is not a string",
                FOUNDING_HOOK.replace("batching: 30s", "batching: [30]"),
                "batching",
            ),
            (
                "predicate block absent",
                FOUNDING_HOOK.replace("```starlark", "```text"),
                "starlark",
            ),
        ];

        for (name, page, field) in &mutations {
            let err = load(page)
                .err()
                .unwrap_or_else(|| panic!("{name}: the mutation must refuse, but it loaded"));
            let text = err.to_string();
            println!("POPULATION {name} -> {text}");
            assert!(
                matches!(err, LoadError::HookMalformed { .. }),
                "{name}: must be typed HookMalformed, got {err:?}"
            );
            assert!(
                text.contains(field),
                "{name}: the refusal must name the offending field `{field}`: {text}"
            );
        }

        // The other arm of the control: the UNMUTATED baseline still loads, so the
        // refusals above are caused by the single edit, not by a broken fixture.
        assert!(load(FOUNDING_HOOK).is_ok(), "the baseline must still load");
    }

    /// An unparseable predicate fails the load gate in the predicate's own language
    /// (`effects::validate`), typed separately from a frontmatter fault.
    #[test]
    fn unparseable_predicate_fails_the_load_gate() {
        let page = FOUNDING_HOOK.replace("def on_change(event):", "def on_change(:");
        let err = load(&page).expect_err("unparseable starlark is refused");
        println!("POPULATION unparseable -> {err}");
        assert!(
            matches!(err, LoadError::HookPredicateInvalid { .. }),
            "{err:?}"
        );
    }

    /// The card's third gate: a HOOK declaring `md.set_field` is refused, and the
    /// refusal names BOTH the cap and the deferral.
    #[test]
    fn a_cap_outside_slice_one_is_a_named_deferral() {
        for cap in [
            "md.set_field",
            "md.append_section",
            "daemon.refresh_view",
            // in-domain but still outside slice 1 — the allowlist is `proto.send`,
            // not `proto.*`, so the rest of the proto family defers too.
            "proto.remind",
            "proto.ask",
            "proto.notice",
        ] {
            let page = FOUNDING_HOOK.replace("[proto.send]", &format!("[{cap}]"));
            let err = load(&page).expect_err("a cap outside slice 1 must refuse");
            let text = err.to_string();
            println!("POPULATION {cap} -> {text}");
            assert_eq!(
                err,
                LoadError::HookCapDeferred {
                    cap: cap.to_string()
                },
                "the refusal is typed and names the cap"
            );
            assert!(text.contains(cap), "names the cap: {text}");
            assert!(
                text.contains("deferred") && text.contains("slice 1"),
                "names the deferral: {text}"
            );
        }

        // The control: the one cap slice 1 DOES carry still loads, so the refusals
        // above are about the allowlist, not about `caps:` parsing being broken.
        assert!(load(FOUNDING_HOOK).is_ok(), "proto.send is admitted");
    }

    #[test]
    fn corpus_counterfactual_caps_do_not_widen_the_production_loader() {
        let page = FOUNDING_HOOK
            .replace("[proto.send]", "[md.set_field]")
            .replace(
                "send(to = [\"reviewer\"], message = \"task → review\")",
                "set_field(field = \"status\", value = \"review\")",
            );
        let counterfactual = load_hook_for_corpus(&page, CheckLimits::default())
            .expect("the corpus tier may evaluate an md.* counterfactual");
        assert_eq!(counterfactual.caps(), &[EffectKind::SetField]);
        assert_eq!(SLICE1_CAPS, [EffectKind::Send]);
        assert!(
            matches!(
                load(&page),
                Err(LoadError::HookCapDeferred { cap }) if cap == "md.set_field"
            ),
            "the exact same declaration is still refused by the production loader"
        );
    }

    /// An unknown cap name is malformed — the descriptor surface is closed, so a
    /// name outside it is a typo or a later vocabulary, never a guess.
    #[test]
    fn an_unknown_cap_name_is_malformed_and_prints_the_closed_surface() {
        let page = FOUNDING_HOOK.replace("[proto.send]", "[proto.telepathy]");
        let err = load(&page).expect_err("an unknown cap is refused");
        let text = err.to_string();
        println!("POPULATION unknown cap -> {text}");
        assert!(matches!(err, LoadError::HookMalformed { .. }), "{err:?}");
        assert!(
            text.contains("proto.telepathy"),
            "names the offender: {text}"
        );
        assert!(
            text.contains("proto.send"),
            "prints the closed surface: {text}"
        );
    }

    /// The card's fifth gate: a predicate calling a non-`proto` constructor faults at
    /// the CEILING — at load — not at runtime.
    ///
    /// The mutation control runs on BOTH conjuncts of the ceiling, because the
    /// ceiling is `declared caps` × `names the predicate reaches`:
    /// - hold the caps, change the call → refused;
    /// - hold the call, empty the caps → refused.
    ///
    /// The second is the biting one: it proves the ceiling is computed FROM THE
    /// DECLARATION rather than hardcoded to a blessed list.
    #[test]
    fn the_ceiling_is_enforced_at_load_from_the_declaration() {
        // Conjunct 1 — same declared caps, a call outside them.
        let calls_md = FOUNDING_HOOK.replace(
            "send(to = [\"reviewer\"], message = \"task → review\")",
            "set_field(path = \"p\", key = \"k\", value = \"v\")",
        );
        let err = load(&calls_md).expect_err("a non-proto constructor is refused at load");
        let text = err.to_string();
        println!("POPULATION out-of-ceiling call -> {text}");
        assert!(matches!(err, LoadError::HookCeiling { .. }), "{err:?}");
        assert!(text.contains("set_field"), "names the reach: {text}");
        assert!(
            text.contains("proto.send"),
            "names the declared caps: {text}"
        );

        // Conjunct 2 — same call, no declared caps. The ceiling must now refuse the
        // very call it admitted above.
        let no_caps = FOUNDING_HOOK.replace("caps:  [proto.send]", "caps:  []");
        let err = load(&no_caps).expect_err("an undeclared cap is denied");
        println!("POPULATION send-with-no-caps -> {err}");
        assert!(matches!(err, LoadError::HookCeiling { .. }), "{err:?}");

        // And the baseline still loads — the refusals are the mutations' doing.
        assert!(load(FOUNDING_HOOK).is_ok(), "baseline loads");
    }

    /// The `how:` block is carried verbatim even when it is not the last key — the
    /// extractor must stop at the next top-level key, not run to the end.
    #[test]
    fn how_is_carried_verbatim_when_other_keys_follow() {
        let page = "\
---
kind: hook
how:
  route: { info: channel-review }
  batching: 30s
severity: info
paths: [\"tasks/*.md\"]
caps: [proto.send]
budget: { steps: 10, mem: 10 }
---

```starlark
def on_change(event):
    send(to = [\"r\"], message = \"m\")
```
";
        let hook = load(page).expect("loads with how: in the middle");
        println!("POPULATION how (mid-frontmatter) = {:?}", hook.how());
        assert_eq!(
            hook.how(),
            "how:\n  route: { info: channel-review }\n  batching: 30s\n"
        );
    }

    /// An unrecognized key under `how:` is CARRIED, not refused. Refusing it would be
    /// the engine ruling on a vocabulary it does not own — `how:` is opaque data and
    /// `ccc-statusd` is its only reader.
    #[test]
    fn an_unknown_how_key_is_carried_not_refused() {
        let page = FOUNDING_HOOK.replace(
            "  wake_policy: never-cold\n",
            "  wake_policy: never-cold\n  quiet_hours: [22, 7]\n",
        );
        let hook = load(&page).expect("an unknown `how:` key is opaque data, not a fault");
        println!("POPULATION how with unknown key = {:?}", hook.how());
        assert!(
            hook.how().contains("quiet_hours: [22, 7]"),
            "the unknown key is carried through: {:?}",
            hook.how()
        );
    }

    /// The founding demo page's frontmatter is an older sketch and does not
    /// load: `scope:` instead of `paths:`, `20k`/`2mb` budget units instead of
    /// integers, and no `caps:`. Pinned as a test because the demo page is what
    /// a reader is pointed at first.
    #[test]
    fn the_founding_demo_pages_frontmatter_does_not_load_as_written() {
        let demo = "\
---
type: rule
rule: task-review-notify
pack: ccc-session@3
kind: hook
severity: info
scope: \"tasks/*.md\"
budget: { steps: 20k, mem: 2mb }
how:
  route: { info: channel-review, error: telegram }
  batching: 30s
  wake_policy: never-cold
---

```starlark
def on_change(event):
    send(to = [\"r\"], message = \"m\")
```
";
        // The divergences surface in THIS order, which is not the order they are
        // listed above: the whole frontmatter is deserialized before any field check
        // runs, so the `20k` budget — a type error — is what serde reports first.
        let err = load(demo).expect_err("the demo page's frontmatter is the older sketch");
        let text = err.to_string();
        println!("POPULATION demo page -> {text}");
        assert!(matches!(err, LoadError::HookMalformed { .. }), "{err:?}");
        assert!(
            text.contains("budget") && text.contains("20k"),
            "the first divergence is the budget's units: {text}"
        );

        // Second, isolated: with an integer budget, the missing `paths:` remains.
        let with_budget = demo.replace(
            "budget: { steps: 20k, mem: 2mb }",
            "budget: { steps: 20000, mem: 2097152 }",
        );
        let err = load(&with_budget).expect_err("`scope:` is not `paths:`");
        println!("POPULATION demo page + budget -> {err}");
        assert!(err.to_string().contains("paths"), "{err}");

        // Third, isolated: with both fixed, the undeclared ceiling is what remains.
        let with_paths = with_budget.replace("scope: \"tasks/*.md\"", "paths: [\"tasks/*.md\"]");
        let err = load(&with_paths).expect_err("the ceiling must be declared");
        println!("POPULATION demo page + budget + paths -> {err}");
        assert!(err.to_string().contains("caps"), "{err}");

        // All three corrected — it loads. The control proving these three refusals
        // are the divergences and not some fourth thing.
        let corrected = with_paths.replace("kind: hook\n", "kind: hook\ncaps: [proto.send]\n");
        let hook = load(&corrected).expect("the corrected demo page loads");
        assert_eq!(hook.caps(), &[EffectKind::Send]);
    }

    /// The design's own §5.2 predicate BODY loads under exactly `[proto.send]`.
    /// The two reaction vocabulary names grant no capability: misspelling either
    /// faults at load, and a direct `md.*` constructor remains outside the ceiling.
    #[test]
    fn the_designs_own_predicate_body_loads_with_each_boundary_load_bearing() {
        let page = FOUNDING_HOOK.replace(
            "    send(to = [\"reviewer\"], message = \"task → review\")",
            "    return intent(action = \"notify\", target = \"r\", \
             receipt = receipt_addr(\"tasks/x.md\", \"abc\"))",
        );
        let hook = load(&page).expect("reaction vocabulary is admitted under proto.send");
        assert_eq!(hook.caps(), &[EffectKind::Send]);

        for (vocab, denied_name, mutation) in [
            ("intent", "intnt", page.replace("intent(", "intnt(")),
            (
                "receipt_addr",
                "receipt_adrr",
                page.replace("receipt_addr(", "receipt_adrr("),
            ),
        ] {
            let err = load(&mutation).expect_err("a misspelled reaction name is refused at load");
            let text = err.to_string();
            println!("POPULATION missing {vocab} -> {text}");
            assert!(matches!(err, LoadError::HookCeiling { .. }), "{err:?}");
            assert!(
                text.contains(denied_name),
                "the refusal names the misspelled {vocab} reach: {text}"
            );
        }

        let direct_md = page.replace(
            "    return intent(action = \"notify\", target = \"r\", \
             receipt = receipt_addr(\"tasks/x.md\", \"abc\"))",
            "    set_field(field = \"status\", value = \"done\")",
        );
        let err = load(&direct_md).expect_err("reaction vocabulary does not grant md.set_field");
        let text = err.to_string();
        println!("POPULATION direct md constructor -> {text}");
        assert!(matches!(err, LoadError::HookCeiling { .. }), "{err:?}");
        assert!(
            text.contains("set_field"),
            "names the denied constructor: {text}"
        );
        assert!(
            text.contains("proto.send"),
            "names the exact cap ceiling: {text}"
        );
    }

    const BASE_SOURCE: &str = "\
def on_change(event):
    send(to = [\"reviewer\"], message = \"task → review\")
";

    const DESIGN_SOURCE: &str = "\
def on_change(event):
    for delta in event.changes:
        if delta.kind != \"frontmatter\" or delta.key != \"status\":
            continue
        if delta.new != \"review\":
            continue
        return intent(
            action = \"notify\",
            target = event.facts.fm.get(\"reviewer\"),
            severity = \"info\",
            payload = event.facts.fm.get(\"hostile\"),
            receipt = receipt_addr(event.file, event.fingerprint_after),
        )
";

    fn page_with_source(source: &str) -> String {
        let page = FOUNDING_HOOK.replace(BASE_SOURCE, source);
        assert_ne!(page, FOUNDING_HOOK, "test source replacement must bite");
        page
    }

    fn review_event() -> ChangeEvent {
        ChangeEvent {
            file: "tasks/x.md".to_string(),
            sections_changed: Vec::new(),
            fields_changed: vec!["status".to_string()],
            changes: vec![ChangeFact {
                kind: ChangeFactKind::Frontmatter,
                key: "status".to_string(),
                old: Some("in-progress".to_string()),
                new: Some("review".to_string()),
                hpath: Vec::new(),
            }],
            facts: EventFacts {
                path: "tasks/x.md".to_string(),
                frontmatter: vec![
                    ("reviewer".to_string(), "e4201e72".to_string()),
                    (
                        "hostile".to_string(),
                        "<@&review> {{not_rendered}}\n\"quoted\" \\ trailing".to_string(),
                    ),
                ],
            },
            fingerprint_before: "rev-before".to_string(),
            fingerprint_after: "rev-after".to_string(),
            depth: 0,
        }
    }

    fn evaluate_named<'a>(
        hooks: impl IntoIterator<Item = (&'a str, &'a Hook)>,
        event: &ChangeEvent,
    ) -> Result<Vec<HookOutcome>, HookEvalError> {
        evaluate_loaded_hooks(hooks, event)
    }

    /// The same `(HOOK, event)` produces the same intent bytes twice. The golden
    /// pins the complete armed-not-delivered shape, including the receipt and the
    /// byte-identical opaque payload and `how:` block.
    #[test]
    fn the_same_hook_event_is_deterministic_and_golden() {
        let hook = load(&page_with_source(DESIGN_SOURCE)).expect("design HOOK loads");
        let event = review_event();
        let first =
            evaluate_named([("task-review-notify", &hook)], &event).expect("design HOOK evaluates");
        let second =
            evaluate_named([("task-review-notify", &hook)], &event).expect("the replay evaluates");
        assert_eq!(
            first, second,
            "same input must yield byte-identical intents"
        );
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].intents.len(), 1);
        assert!(first[0].narrowed.is_empty());

        let intent = &first[0].intents[0];
        assert_eq!(intent.action, "notify", "the alias is carried verbatim");
        assert_eq!(intent.target.as_deref(), Some("e4201e72"));
        assert_eq!(
            intent.payload.as_deref(),
            Some("<@&review> {{not_rendered}}\n\"quoted\" \\ trailing"),
            "Rust neither renders nor rewrites the hostile payload"
        );
        assert_eq!(
            intent.receipt,
            effects::receipt_address("tasks/x.md", "rev-after"),
            "receipt is pure in path and the landed revision"
        );
        assert_eq!(first[0].how, FOUNDING_HOW, "HOW is byte-identical");
        insta::assert_json_snapshot!("hook_outcome_armed_not_delivered", first);
    }

    /// `intent(action = …)` is vocabulary, not authority. A runtime-selected
    /// `md.*` kind passes the load lint, then the exact `[proto.send]` cap drops it
    /// and retains the complete descriptor in the named `narrowed` report.
    #[test]
    fn an_out_of_caps_intent_is_dropped_and_named() {
        let source = DESIGN_SOURCE.replace("action = \"notify\"", "action = \"md.set_field\"");
        let hook = load(&page_with_source(&source)).expect("runtime action strings load");
        assert_eq!(hook.caps(), &[EffectKind::Send], "allowlist did not widen");

        let outcomes = evaluate_named([("runtime-md", &hook)], &review_event())
            .expect("out-of-cap routing is a report, not an eval fault");
        assert!(outcomes[0].intents.is_empty(), "md.* cannot survive");
        assert_eq!(outcomes[0].narrowed.len(), 1);
        assert_eq!(outcomes[0].narrowed[0].action, "md.set_field");
        assert_eq!(outcomes[0].narrowed[0].rule_id, "runtime-md");
    }

    /// Kernel sequence numbers are per rule, so both descriptors arrive as zero.
    /// The HOOK evaluator assigns one outcome-wide ordinal and preserves rule ids.
    #[test]
    fn two_hooks_on_one_change_receive_distinct_sequence_numbers() {
        let first = load(&page_with_source(DESIGN_SOURCE)).expect("first HOOK loads");
        let second = load(&page_with_source(DESIGN_SOURCE)).expect("second HOOK loads");
        let outcomes = evaluate_named(
            [("first-hook", &first), ("second-hook", &second)],
            &review_event(),
        )
        .expect("both HOOKs evaluate");

        assert_eq!(outcomes.len(), 2);
        assert_eq!(outcomes[0].intents[0].seq, 0);
        assert_eq!(outcomes[1].intents[0].seq, 1);
        assert_eq!(outcomes[0].intents[0].rule_id, "first-hook");
        assert_eq!(outcomes[1].intents[0].rule_id, "second-hook");
        assert_eq!(
            outcomes[0].intents[0].receipt, outcomes[1].intents[0].receipt,
            "the same (path, new_rev) always names the same receipt"
        );
    }

    /// A predicate may spell any receipt string, but only the canonical arm-time
    /// address for this landed `(path, new_rev)` may cross the evaluator boundary.
    #[test]
    fn every_intent_receipt_must_name_the_exact_landed_change() {
        let receipt_line =
            "            receipt = receipt_addr(event.file, event.fingerprint_after),\n";
        let mutations = [
            ("missing", DESIGN_SOURCE.replace(receipt_line, "")),
            (
                "empty",
                DESIGN_SOURCE.replace(receipt_line, "            receipt = \"\",\n"),
            ),
            (
                "arbitrary",
                DESIGN_SOURCE.replace(receipt_line, "            receipt = \"forged\",\n"),
            ),
            (
                "wrong path",
                DESIGN_SOURCE.replace(
                    receipt_line,
                    "            receipt = receipt_addr(\"tasks/other.md\", event.fingerprint_after),\n",
                ),
            ),
            (
                "wrong revision",
                DESIGN_SOURCE.replace(
                    receipt_line,
                    "            receipt = receipt_addr(event.file, \"not-the-landed-rev\"),\n",
                ),
            ),
        ];

        for (name, source) in mutations {
            assert_ne!(source, DESIGN_SOURCE, "{name}: mutation must bite");
            let hook = load(&page_with_source(&source)).expect("receipt mutations load");
            let error = evaluate_named([(name, &hook)], &review_event())
                .expect_err("a forged receipt must not arm");
            let text = error.to_string();
            assert!(
                matches!(error, HookEvalError::MalformedIntent { .. }),
                "{name}: {error:?}"
            );
            assert!(
                text.contains("receipt"),
                "{name}: fault names receipt: {text}"
            );
        }

        let hook = load(&page_with_source(DESIGN_SOURCE)).expect("canonical receipt loads");
        let event = review_event();
        let first = evaluate_named([("canonical", &hook)], &event).expect("canonical receipt arms");
        let second = evaluate_named([("canonical", &hook)], &event).expect("replay arms");
        let expected = effects::receipt_address(&event.file, &event.fingerprint_after);
        assert_eq!(first[0].intents[0].receipt, expected);
        assert_eq!(first[0].intents[0].receipt, second[0].intents[0].receipt);
    }

    /// Budget exhaustion is advisory report data, not an error frame. One exhausted
    /// HOOK must not suppress a later valid HOOK over the same landed change.
    #[test]
    fn budget_exhaustion_is_named_and_does_not_abort_later_hooks() {
        let exhausted_page = page_with_source(DESIGN_SOURCE).replace(
            "budget: { steps: 10000, mem: 4194304 }",
            "budget: { steps: 1, mem: 4194304 }",
        );
        let exhausted = load(&exhausted_page).expect("the tiny budget is declaration data");
        let valid = load(&page_with_source(DESIGN_SOURCE)).expect("the valid HOOK loads");

        let outcomes = evaluate_named(
            [("tiny-budget", &exhausted), ("valid-after-budget", &valid)],
            &review_event(),
        )
        .unwrap_or_else(|error| panic!("budget exhaustion must be a finding, not {error:?}"));

        assert_eq!(outcomes.len(), 2, "both in-scope HOOKs report an outcome");
        assert_eq!(
            outcomes[0].findings,
            vec![HookFinding::BudgetExceeded {
                rule_id: "tiny-budget".to_string(),
                steps: 1,
                mem: 4_194_304,
            }],
            "the exhausted HOOK names its budget_exceeded finding"
        );
        assert!(outcomes[0].intents.is_empty());
        assert_eq!(outcomes[1].intents.len(), 1, "the later HOOK still arms");
        assert_eq!(outcomes[1].intents[0].rule_id, "valid-after-budget");
        assert!(outcomes[1].findings.is_empty());
    }

    /// The load lint sees bare globals but deliberately does not inspect attribute
    /// names. Therefore bare `actor` faults at LOAD, while `event.actor` loads and
    /// faults at EVAL because the payload struct has no such attribute.
    #[test]
    fn bare_actor_faults_at_load_but_event_actor_faults_at_eval() {
        let bare = page_with_source("def on_change(event):\n    x = actor\n");
        let error = load(&bare).expect_err("a bare actor name is outside the load ceiling");
        let text = error.to_string();
        println!("POPULATION bare actor load fault -> {text}");
        assert!(matches!(error, LoadError::HookCeiling { .. }), "{error:?}");
        assert!(
            text.contains("actor"),
            "load fault names the bare reach: {text}"
        );

        let attribute = page_with_source("def on_change(event):\n    x = event.actor\n");
        let hook = load(&attribute).expect("attribute names are not global reaches");
        let error = evaluate_named([("attribute-smuggler", &hook)], &review_event())
            .expect_err("the event carries no actor fact");
        let text = error.to_string();
        println!("POPULATION event.actor eval fault -> {text}");
        assert!(matches!(
            error,
            HookEvalError::Eval(EvalError::Runtime { .. })
        ));
        assert!(
            text.contains("actor"),
            "eval fault names the absent attribute: {text}"
        );
    }

    /// A reaction can return refusal-looking data, but that value is not a gate
    /// outcome and emits no refusal. Reaching for the CHECK-only `refuse` constructor
    /// is itself denied at load.
    #[test]
    fn a_hook_outcome_cannot_express_a_refusal() {
        let hook = load(&page_with_source(
            "def on_change(event):\n    return \"refuse the write\"\n",
        ))
        .expect("plain return data has no blocking power");
        let outcomes = evaluate_named([("looks-refusing", &hook)], &review_event())
            .expect("refusal-looking return data cannot fault the landed write");
        assert_eq!(
            outcomes,
            vec![HookOutcome {
                intents: Vec::new(),
                narrowed: Vec::new(),
                findings: Vec::new(),
                how: FOUNDING_HOW.to_string(),
            }]
        );

        let calls_refuse = page_with_source(
            "def on_change(event):\n    refuse(message = \"no\", passing_scenario = \"yes\")\n",
        );
        let error = load(&calls_refuse).expect_err("HOOK cannot name CHECK's refusal constructor");
        assert!(matches!(error, LoadError::HookCeiling { .. }), "{error:?}");
        assert!(error.to_string().contains("refuse"));
    }

    /// Scope is part of the evaluator boundary: an armed HOOK outside the changed
    /// path is not run and produces no outcome.
    #[test]
    fn an_out_of_scope_hook_is_silent() {
        let hook = load(&page_with_source(DESIGN_SOURCE)).expect("HOOK loads");
        let mut event = review_event();
        event.file = "notes/x.md".to_string();
        assert!(
            evaluate_named([("task-only", &hook)], &event)
                .expect("scope filtering does not evaluate")
                .is_empty()
        );
    }
}
