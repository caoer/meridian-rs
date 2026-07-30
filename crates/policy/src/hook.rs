//! The HOOK capability declaration (`HOOK.md`) — the emit leg's load surface (U1.3).
//!
//! # What HOOK is (rulings § capability grammar)
//! HOOK **may read** the landed change, **may react outward** under DECLARED effect
//! caps, and **may never veto or mutate**. It is the emit leg, and its deferral
//! condition (*"until a real subject needs them"*) is met — a status subscription is
//! that subject. CHECK is the law leg and is unchanged by this module; FIX and VIEW
//! stay deferred.
//!
//! A convention may carry HOOK **without** CHECK: a reaction is not a law, and HOOK
//! exists as a distinct name so reactions are never confused with law.
//!
//! # The declaration
//! ```text
//! ---
//! kind: hook
//! severity: info
//! paths: ["tasks/*.md"]
//! caps:  [proto.send]
//! budget: { steps: 10000, mem: 4194304 }
//! how:
//!   route:    { info: channel-review, error: telegram }
//!   batching: 30s
//!   wake_policy: never-cold
//! ---
//! ```
//! plus one fenced Starlark block defining the reaction predicate.
//!
//! # `how:` is FROZEN DATA
//! The engine **checks that it is well-formed and carries it through byte for byte**;
//! it never interprets it. `ccc-statusd` is the only reader. That is why
//! [`Hook::how`] hands back the verbatim source slice rather than a re-serialized
//! structure, and why an unrecognized key under `how:` is **carried, not refused** —
//! refusing an unknown key would be the engine ruling on a vocabulary that is not
//! its own. Only the shapes of the keys slice 1 names are checked.
//!
//! # The ceiling is enforced at LOAD, not at eval
//! A HOOK predicate is written in the `effects` rule language, so its power ceiling
//! is the descriptor constructors that language registers. [`Hook`] resolves the
//! declared `caps:` to exactly those constructor names and statically resolves the
//! predicate's free names against them: a predicate that calls `set_field` under
//! `caps: [proto.send]` is refused when the convention loads, before any change ever
//! reaches it. This is the `check_when_vocab` precedent (§11.2) applied to the emit
//! leg — the same `using-undefined` name resolution, a different closed vocabulary.

use std::collections::HashSet;

use effects::{EffectKind, EvalLimits, Rule};
use starlark::analysis::AstModuleLint;
use starlark::environment::GlobalsBuilder;
use starlark::syntax::{AstModule, Dialect};

use crate::EvalBudget;
use crate::check_eval::CheckLimits;
use crate::convention::LoadError;

/// The caps slice 1 admits — `proto.send` and nothing else. Every other descriptor
/// kind is a NAMED deferral at load ([`LoadError::HookCapDeferred`]), never a silent
/// ignore: a convention that declares `md.set_field` is told the cap exists and that
/// this slice does not carry it.
pub const SLICE1_CAPS: [EffectKind; 1] = [EffectKind::Send];

/// A loaded `HOOK.md`: its declared scope, severity, caps, per-eval budget, the
/// verbatim `how:` block, and the parse- and ceiling-validated predicate.
/// Construction is sealed to [`load_hook`] — a `Hook` in hand has passed the
/// frontmatter grammar, the slice-1 cap allowlist, the full-limits load gate, and the
/// capability ceiling.
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
    /// [`crate::Convention::matches_path`] — the one matcher, reused.
    #[must_use]
    pub fn matches_path(&self, path: &str) -> bool {
        crate::convention::path_in_scope(&self.scope, path)
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

/// Parse and validate a `HOOK.md` page under `limits`.
///
/// Pipeline: frontmatter grammar (`kind`/`severity`/`paths`/`caps`/`budget`/`how`) →
/// `how:` shape → caps against the slice-1 allowlist → the fenced predicate →
/// `effects::validate` under the full limits → the capability ceiling.
///
/// # Errors
/// [`LoadError::HookMalformed`], [`LoadError::HookCapDeferred`],
/// [`LoadError::HookPredicateInvalid`], [`LoadError::HookCeiling`].
pub(crate) fn load_hook(hook_md: &str, limits: CheckLimits) -> Result<Hook, LoadError> {
    let malformed = |reason: String| LoadError::HookMalformed { reason };

    let (frontmatter, _body) = crate::pack::split_frontmatter(hook_md)
        .ok_or_else(|| malformed("no `---` frontmatter".to_string()))?;
    let parsed: HookFrontmatter = serde_yaml::from_str(frontmatter)
        .map_err(|e| malformed(format!("frontmatter parse: {e}")))?;

    let kind = parsed
        .kind
        .ok_or_else(|| malformed("frontmatter must declare `kind: hook`".to_string()))?;
    if kind != "hook" {
        return Err(malformed(format!(
            "`kind:` is {kind:?}, but this file is HOOK.md — the declared kind must be `hook`"
        )));
    }

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
    let caps = resolve_caps(&declared_caps)?;

    let budget = parsed
        .budget
        .ok_or_else(|| malformed("frontmatter must declare `budget: {steps, mem}`".to_string()))?;

    let how_value = parsed
        .how
        .ok_or_else(|| malformed("frontmatter must declare `how:`".to_string()))?;
    validate_how(&how_value).map_err(malformed)?;
    let how = extract_how_block(frontmatter)
        .ok_or_else(|| {
            malformed("`how:` is not a block this loader can carry verbatim".to_string())
        })?
        .to_string();

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
/// declaration may carry descriptive frontmatter — the founding pages carry `type:`,
/// `rule:`, `pack:`, `tags:`) and ignored.
#[derive(serde::Deserialize)]
struct HookFrontmatter {
    kind: Option<String>,
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
fn resolve_caps(declared: &[String]) -> Result<Vec<EffectKind>, LoadError> {
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
        if !SLICE1_CAPS.contains(&kind) {
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
/// names against the Starlark standard library plus exactly the constructors the
/// declared caps grant; any other global name is a reach the ceiling does not admit.
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

    /// Design §5.2's `HOOK.md` declaration, verbatim in its frontmatter — the form
    /// this card exists to load. The predicate is the SHIPPED reaction language
    /// (`on_change(event)` over the descriptor constructors); the design's own
    /// `when(delta, facts, now)` body needs `intent()` / `receipt_addr()`, which are
    /// C2's to register — see `the_designs_own_predicate_body_is_not_loadable_yet`.
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
        // Owned strings so each mutation is exactly one edit off the SAME baseline.
        let mutations: Vec<(&str, String, &str)> = vec![
            (
                "kind absent",
                FOUNDING_HOOK.replace("kind: hook\n", ""),
                "kind",
            ),
            (
                "kind is not hook",
                FOUNDING_HOOK.replace("kind: hook", "kind: check"),
                "kind",
            ),
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
            "proto.warn",
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

    /// **The founding DEMO page does not load as written, and this pins why.**
    ///
    /// `foundation-panel/round-2/demo/rules/task-review-notify.md` (2026-07-18) is
    /// slice 1's origin, and design §5.2 is its corrected form. The two frontmatters
    /// diverge in three ways, so anyone copying the demo page verbatim gets a
    /// refusal:
    ///
    /// | demo page | design §5.2 | consequence |
    /// |---|---|---|
    /// | `scope: "tasks/*.md"` | `paths: ["tasks/*.md"]` | refused — no `paths:` |
    /// | `budget: { steps: 20k, mem: 2mb }` | integers | `20k` is not a number |
    /// | no `caps:` | `caps: [proto.send]` | refused — the ceiling is undeclared |
    ///
    /// The design's form is the one that loads; the demo page is the older sketch.
    /// Pinned as a test rather than reported once, because the demo page is what a
    /// reader is pointed at first.
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

    /// The design's own §5.2 predicate BODY is not loadable yet, and that is the
    /// correct boundary rather than a defect: `intent()` and `receipt_addr()` are
    /// C2's to register, so today they are reaches outside the ceiling. C1 loads the
    /// DECLARATION; C2 gives the predicate its constructors.
    #[test]
    fn the_designs_own_predicate_body_is_not_loadable_yet() {
        let page = FOUNDING_HOOK.replace(
            "    send(to = [\"reviewer\"], message = \"task → review\")",
            "    return intent(action = \"notify\", target = \"r\", \
             receipt = receipt_addr(\"tasks/x.md\", \"abc\"))",
        );
        let err = load(&page).expect_err("intent()/receipt_addr() do not exist yet");
        let text = err.to_string();
        println!("POPULATION design predicate -> {text}");
        assert!(matches!(err, LoadError::HookCeiling { .. }), "{err:?}");
        assert!(text.contains("intent"), "names what is missing: {text}");
    }
}
