//! The page-shaped rule load — what a tag-registered page becomes when it is
//! evaluated.
//!
//! # The layering (ruling §§ 1–2)
//! [`crate::registration`] answers IDENTITY: does this page carry a `rules/*` tag,
//! and what is its `id:`. This module answers EVALUABILITY: what does that page
//! DECLARE, and does the declaration load. The split is why [`load_rule`] takes a
//! [`Registration`] rather than re-deriving one — there is exactly one place a page
//! becomes a rule, and it is upstream of here.
//!
//! # What a rule page declares
//! One page, one id, up to two legs — the legs its registration tags name:
//!
//! - `rules/check` — the law leg: `paths:` frontmatter plus a fenced
//!   `def check_change(change)` predicate. May refuse a write.
//! - `rules/hook` — the reaction leg: the [`crate::hook`] declaration
//!   (`severity`/`paths`/`caps`/`budget`/`how` plus the reaction predicate). May
//!   never veto or mutate.
//!
//! A page carrying BOTH tags declares both legs and must satisfy both loads. It is
//! fail-closed and never partial, the same way a two-capability convention folder
//! is: a page that gets one leg wrong loads NEITHER, because half a rule is a rule
//! whose author does not know which half is running.
//!
//! # The filename is gone, and nothing replaced it
//! Nothing here reads a folder name, a `CHECK.md`/`HOOK.md` spelling, or a `kind:`
//! frontmatter key. The parsers are the SAME ones the folder loader uses
//! ([`crate::declaration::parse_check`], [`crate::hook::load_hook`]) — they were
//! always page-shaped, taking bytes rather than filenames. What dies with the folder
//! loader is the addressing above them, not the parsing.
//!
//! # Bytes must be the registered bytes
//! [`load_rule`] verifies `page_rev(bytes)` against the rev the registration pinned
//! and refuses on mismatch. A caller that walks, registers, then re-reads a page an
//! editor has moved under it would otherwise evaluate one page's law under another
//! page's identity — silently, and only sometimes.

use crate::check_eval::CheckLimits;
use crate::declaration::{CheckOutcome, LoadError};
use crate::hook::Hook;
use crate::registration::{Registration, RuleId, RuleKind, page_rev};

/// A loaded rule page: its identity, its declared legs, and its scope.
/// Construction is sealed to [`load_rule`], so a `Rule` in hand has passed the
/// registration gates AND its declarations' load gates.
#[derive(Debug, Clone)]
pub struct Rule {
    id: RuleId,
    kinds: Vec<RuleKind>,
    page: String,
    scope: Vec<String>,
    check_source: Option<String>,
    hook: Option<Hook>,
    limits: CheckLimits,
}

impl Rule {
    /// The rule's identity — its frontmatter `id:`, not a filename.
    #[must_use]
    pub fn id(&self) -> &RuleId {
        &self.id
    }

    /// The legs the page registers as, sorted and deduplicated — derived from the
    /// registration TAGS, never from a `kind:` key. The armed-set artifact reads
    /// this to know which mode vocabulary a row may carry.
    #[must_use]
    pub fn kinds(&self) -> &[RuleKind] {
        &self.kinds
    }

    /// The page this rule was loaded from, relative to its layer's root.
    #[must_use]
    pub fn page(&self) -> &str {
        &self.page
    }

    /// The `paths:` globs the rule declares. A page carrying both legs answers with
    /// the CHECK leg's scope; the hook always answers for itself through
    /// [`Hook::matches_path`], so the two legs may scope differently.
    #[must_use]
    pub fn scope(&self) -> &[String] {
        &self.scope
    }

    /// Whether `path` is inside the rule's declared scope.
    #[must_use]
    pub fn matches_path(&self, path: &str) -> bool {
        crate::declaration::path_in_scope(&self.scope, path)
    }

    /// The reaction leg, when the page carries `rules/hook`.
    #[must_use]
    pub fn hook(&self) -> Option<&Hook> {
        self.hook.as_ref()
    }

    /// The law leg's predicate source, when the page carries `rules/check`.
    #[must_use]
    pub fn check_source(&self) -> Option<&str> {
        self.check_source.as_deref()
    }

    /// Run the law leg over one [`crate::Change`] under the rule's full limits.
    ///
    /// A page with no CHECK leg **passes everything** — it emits no refusals because
    /// it declares no law. A reaction never vetoes, so this is silence by
    /// construction rather than by accident.
    ///
    /// Scoping is the caller's: pair with [`Rule::matches_path`] on the change's
    /// document path so an out-of-scope document is never evaluated.
    ///
    /// # Errors
    /// [`crate::CheckError`] — a budget/parse/runtime fault in the predicate.
    pub fn check_change(&self, change: &crate::Change) -> Result<CheckOutcome, crate::CheckError> {
        let Some(source) = &self.check_source else {
            return Ok(CheckOutcome {
                refusals: Vec::new(),
            });
        };
        let refusals = crate::check_eval::run_check_change(source, change, self.limits)?;
        Ok(CheckOutcome { refusals })
    }

    /// Run the law leg and return the [`crate::CheckTelemetry`] — the refusals AND
    /// the exact fuel/heap the evaluation spent.
    ///
    /// Same metered core as [`Rule::check_change`]; this variant keeps the reading
    /// the `test --corpus` tier's p50/p99 budget signal is computed from. The twin
    /// on [`CounterfactualRule`] exists for the same reason and shares this body's
    /// evaluator: the two corpus loader modes differ in which caps a HOOK
    /// declaration may CARRY, never in how a law is metered, so a profile taken
    /// through one must be the profile taken through the other.
    ///
    /// # Errors
    /// [`crate::CheckError`] — a budget/parse/runtime fault, or
    /// [`crate::CheckError::MissingEntry`] when the page declares no CHECK leg. A
    /// page with no law reports the ABSENCE rather than a zero reading, because an
    /// unmetered pass and a law that spent nothing are different facts — averaging
    /// the former into a budget profile would understate every law beside it. That
    /// is also why this is not [`Rule::check_change`]'s silence: there, silence is
    /// the honest verdict; here, there is no measurement to report.
    pub fn check_change_metered(
        &self,
        change: &crate::Change,
    ) -> Result<crate::CheckTelemetry, crate::CheckError> {
        let source = self
            .check_source
            .as_ref()
            .ok_or(crate::CheckError::MissingEntry)?;
        crate::check_eval::run_check_change_metered(source, change, self.limits)
    }
}

/// Why a registered page could not be loaded as a rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleLoadError {
    /// The bytes handed to the loader are not the bytes the registration pinned —
    /// the page changed between discovery and load. Fail-closed: evaluating these
    /// bytes under that identity is exactly the drift the rev law exists to catch.
    RevMismatch {
        /// The page the mismatch is about.
        page: String,
        /// The rev discovery pinned.
        registered: String,
        /// The rev the supplied bytes hash to.
        supplied: String,
    },
    /// The page carries a `kind:` key that contradicts its registration tags.
    ///
    /// `kind:` is legacy vocabulary the engine no longer reads for registration
    /// (ruling § 1), so it may RESTATE what the tag says and may be absent, but it
    /// may never say something else — the same law the block-declared id lives
    /// under. A page that arms as a hook while calling itself a check has two
    /// answers to one question, and the write path would find out first.
    KindDisagrees {
        /// The offending page.
        page: String,
        /// The `kind:` as written (empty when it was not even a string).
        declared: String,
        /// The kinds the page's tags register.
        tags: Vec<RuleKind>,
    },
    /// The page declares a leg whose entry point its block never defines.
    ///
    /// The folder loader could not meet this case: two legs meant two files, so a
    /// `CHECK.md` holding only `def on_change` was a file whose own name refuted it.
    /// One page carrying both tags shares ONE fenced block between both legs, so the
    /// entry point is the only thing that still distinguishes them — and a law that
    /// silently cannot fire is exactly what fail-closed exists to prevent.
    EntryMissing {
        /// The offending page.
        page: String,
        /// The leg whose entry point is absent.
        kind: RuleKind,
        /// The entry point the leg is evaluated through.
        entry: &'static str,
    },
    /// A declared leg did not load. The page and the kind name which leg.
    Leg {
        /// The page the fault is in.
        page: String,
        /// The leg that failed to load.
        kind: RuleKind,
        /// The parser's own refusal.
        source: LoadError,
    },
}

impl RuleLoadError {
    /// The page the refusal is about.
    #[must_use]
    pub fn page(&self) -> &str {
        match self {
            RuleLoadError::RevMismatch { page, .. }
            | RuleLoadError::KindDisagrees { page, .. }
            | RuleLoadError::EntryMissing { page, .. }
            | RuleLoadError::Leg { page, .. } => page,
        }
    }
}

impl std::fmt::Display for RuleLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuleLoadError::RevMismatch {
                page,
                registered,
                supplied,
            } => write!(
                f,
                "`{page}` registered at rev `{registered}` but the bytes supplied to the loader \
                 hash to `{supplied}` — the page moved between discovery and load; re-discover \
                 before evaluating it"
            ),
            RuleLoadError::KindDisagrees {
                page,
                declared,
                tags,
            } => write!(
                f,
                "`{page}` declares `kind: {declared}` but registers as {tags} — the tag names the \
                 leg, so a `kind:` key may restate it or be absent, never contradict it. Remove \
                 `kind:` or make it agree",
                tags = tags
                    .iter()
                    .map(|k| format!("`{}`", k.tag()))
                    .collect::<Vec<_>>()
                    .join(" + "),
            ),
            RuleLoadError::EntryMissing { page, kind, entry } => write!(
                f,
                "`{page}` declares `{tag}`, but its starlark block defines no `def {entry}(…)` — \
                 the tag names the leg, and the leg is evaluated through that entry point. A page \
                 carrying both tags defines both entry points in the one block",
                tag = kind.tag()
            ),
            RuleLoadError::Leg { page, kind, source } => write!(
                f,
                "`{page}` declares `{tag}` but its declaration does not load: {source}",
                tag = kind.tag()
            ),
        }
    }
}

impl std::error::Error for RuleLoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RuleLoadError::Leg { source, .. } => Some(source),
            RuleLoadError::RevMismatch { .. }
            | RuleLoadError::KindDisagrees { .. }
            | RuleLoadError::EntryMissing { .. } => None,
        }
    }
}

/// The one frontmatter key this module reads for itself: the legacy `kind:`, kept
/// honest rather than kept. Everything else a rule page declares is read by the leg
/// parsers or by registration.
#[derive(serde::Deserialize, Default)]
struct KindFrontmatter {
    kind: Option<serde_yaml::Value>,
}

/// The `kind:` a page declares, or `None` when it declares none (the form the
/// ruling prefers — the tag already said it).
///
/// A non-string `kind:` reads as the empty declaration `""`, which agrees with
/// nothing and so refuses: identity keys are frontmatter QUERIES, and a value that
/// is not text cannot be compared to a tag without evaluating something.
///
/// Unparseable frontmatter cannot arrive here — [`register_page`] refused it before
/// a `Registration` existed, and [`load_rule`] verified these bytes ARE those bytes.
fn declared_kind(bytes: &str) -> Option<String> {
    let (frontmatter, _body) = crate::pack::split_frontmatter(bytes)?;
    let parsed: KindFrontmatter = serde_yaml::from_str(frontmatter).unwrap_or_default();
    let value = parsed.kind.filter(|v| !v.is_null())?;
    Some(value.as_str().unwrap_or_default().to_string())
}

/// The entry point a leg is evaluated through — `rulepack-api@2`'s
/// `check_change(change)` for the law, the effect kernel's `on_change(event)` for
/// the reaction.
fn entry_point(kind: RuleKind) -> &'static str {
    match kind {
        RuleKind::Check => "check_change",
        RuleKind::Hook => "on_change",
    }
}

/// Whether `source` defines a TOP-LEVEL `def <name>`, read statically from the
/// parsed AST — never evaluated, the same layering [`register_page`] reads a
/// block-declared id under. A nested `def` is a local, not the leg's entry point.
///
/// An unparseable block answers `false` rather than raising: the leg's own load
/// gate (`validate_check_source` / `effects::validate`) owns the parse refusal, and
/// two refusals for one fault teach nothing the first did not.
fn declares_def(source: &str, name: &str) -> bool {
    use starlark::syntax::ast::{AstStmt, Stmt};
    use starlark::syntax::{AstModule, Dialect};

    fn scan(stmt: &AstStmt, name: &str) -> bool {
        match &**stmt {
            Stmt::Statements(statements) => statements.iter().any(|s| scan(s, name)),
            Stmt::Def(def) => def.name.ident == name,
            _ => false,
        }
    }

    AstModule::parse("rule", source.to_owned(), &Dialect::Standard)
        .is_ok_and(|ast| scan(ast.statement(), name))
}

/// Load a registered page into an evaluable [`Rule`] under `limits`.
///
/// `bytes` must be the bytes `registration` was discovered from — the rev is
/// verified, not trusted.
///
/// # Errors
/// [`RuleLoadError`] — a rev mismatch, or a declared leg that does not load.
pub fn load_rule(
    registration: &Registration,
    bytes: &str,
    limits: CheckLimits,
) -> Result<Rule, RuleLoadError> {
    load_rule_with_hook_loader(registration, bytes, limits, crate::hook::load_hook)
}

/// The one load pipeline, parameterized by which HOOK capability allowlist admits
/// the reaction leg. Production and the corpus proof share every other gate — the
/// rev check, the kind seam, both legs' parse, the entry points, and the scope
/// rule — so a second copy is how the two would drift into disagreeing about what
/// a rule page IS.
fn load_rule_with_hook_loader(
    registration: &Registration,
    bytes: &str,
    limits: CheckLimits,
    hook_loader: fn(&str, CheckLimits) -> Result<Hook, LoadError>,
) -> Result<Rule, RuleLoadError> {
    let supplied = page_rev(bytes);
    if supplied != registration.rev() {
        return Err(RuleLoadError::RevMismatch {
            page: registration.page().to_string(),
            registered: registration.rev().to_string(),
            supplied,
        });
    }

    // The kind seam (ruled 2026-08-01). `kind:` may restate the tag or be absent —
    // absent DERIVES from the tag, which is why nothing below reads it again. It may
    // not contradict, and this is the one place that is enforced: a page that armed
    // as one leg while calling itself another would otherwise fault on every write.
    // Checked before the legs, because a page that cannot say what it is has no
    // declaration worth parsing.
    if let Some(declared) = declared_kind(bytes)
        && !registration
            .kinds()
            .iter()
            .any(|kind| kind.as_str() == declared)
    {
        return Err(RuleLoadError::KindDisagrees {
            page: registration.page().to_string(),
            declared,
            tags: registration.kinds().to_vec(),
        });
    }

    let leg = |kind: RuleKind| {
        move |source: LoadError| RuleLoadError::Leg {
            page: registration.page().to_string(),
            kind,
            source,
        }
    };

    // Both legs parse before either is admitted: a page that declares two legs and
    // gets one wrong loads NEITHER (the fail-closed law the folder loader holds for
    // a two-capability folder).
    let check = registration
        .kinds()
        .contains(&RuleKind::Check)
        .then(|| crate::declaration::parse_check(bytes, limits).map_err(leg(RuleKind::Check)))
        .transpose()?;
    let hook = registration
        .kinds()
        .contains(&RuleKind::Hook)
        .then(|| hook_loader(bytes, limits).map_err(leg(RuleKind::Hook)))
        .transpose()?;

    // Each declared leg must define its own entry point. One page, one fenced block,
    // two possible legs — so the tag alone no longer says which code is the law and
    // which is the reaction.
    for (kind, source) in [
        (RuleKind::Check, check.as_ref().map(|(_, source)| &**source)),
        (RuleKind::Hook, hook.as_ref().map(Hook::source)),
    ] {
        let entry = entry_point(kind);
        if source.is_some_and(|source| !declares_def(source, entry)) {
            return Err(RuleLoadError::EntryMissing {
                page: registration.page().to_string(),
                kind,
                entry,
            });
        }
    }

    // Scope: the law's `paths:` when the page carries a law, the reaction's when it
    // carries only a reaction. A hook always answers scope through its own
    // `matches_path`, so a dual-leg page may scope its law and its reaction apart.
    let scope = match (&check, &hook) {
        (Some((scope, _)), _) => scope.clone(),
        (None, Some(hook)) => hook.scope().to_vec(),
        (None, None) => Vec::new(),
    };

    Ok(Rule {
        id: registration.id().clone(),
        kinds: registration.kinds().to_vec(),
        page: registration.page().to_string(),
        scope,
        check_source: check.map(|(_, source)| source),
        hook,
        limits,
    })
}

/// A rule page loaded only for the `test --corpus` counterfactual proof.
///
/// This wrapper is deliberately not [`Rule`]. Its widened HOOK cannot enter
/// ordinary policy evaluation, arming, or the production loader BY TYPE. The
/// public surface exposes only the facts the proof needs; the widened [`Hook`]
/// stays behind the policy-owned corpus evaluator.
///
/// ```compile_fail
/// use policy::{CounterfactualRule, Rule};
///
/// fn cannot_enter_ordinary_policy(proof: CounterfactualRule) {
///     let _ordinary: Rule = proof;
/// }
/// ```
#[derive(Debug, Clone)]
pub struct CounterfactualRule {
    inner: Rule,
}

impl CounterfactualRule {
    /// The rule's id — the name every corpus report row is labelled with.
    #[must_use]
    pub fn id(&self) -> &str {
        self.inner.id().as_str()
    }

    /// The page the rule was loaded from.
    #[must_use]
    pub fn page(&self) -> &str {
        self.inner.page()
    }

    /// The rule's declared default scope.
    #[must_use]
    pub fn scope(&self) -> &[String] {
        self.inner.scope()
    }

    /// Whether the CHECK leg matches `path`.
    #[must_use]
    pub fn matches_path(&self, path: &str) -> bool {
        self.inner.matches_path(path)
    }

    /// Whether the page carries a CHECK leg.
    #[must_use]
    pub fn has_check(&self) -> bool {
        self.inner.check_source().is_some()
    }

    /// Run the CHECK leg through the ordinary metered evaluator.
    ///
    /// # Errors
    /// [`crate::CheckError`] — a budget/parse/runtime fault, or
    /// [`crate::CheckError::MissingEntry`] when the page declares no CHECK leg
    /// (the metered path reports the absence rather than inventing a zero
    /// reading).
    pub fn check_change_metered(
        &self,
        change: &crate::Change,
    ) -> Result<crate::CheckTelemetry, crate::CheckError> {
        // Delegates rather than re-runs the evaluator: the widening this wrapper
        // carries is a HOOK-cap widening, and a second metered body here is how the
        // two corpus modes would come to profile the same law differently.
        self.inner.check_change_metered(change)
    }

    /// Whether the page carries a counterfactual HOOK leg.
    #[must_use]
    pub fn has_hook(&self) -> bool {
        self.inner.hook().is_some()
    }

    /// Whether the counterfactual HOOK leg matches `path`.
    #[must_use]
    pub fn hook_matches_path(&self, path: &str) -> bool {
        self.inner
            .hook()
            .is_some_and(|hook| hook.matches_path(path))
    }

    pub(crate) fn hook(&self) -> Option<&Hook> {
        self.inner.hook()
    }
}

/// Load a registered page for the `test --corpus` pre-arming proof.
///
/// This differs from [`load_rule`] only in HOOK capability admission:
/// counterfactual `md.*` descriptors may load so the tier can prove whether their
/// trigger graph is quiescent. The widened declaration is returned as the opaque
/// [`CounterfactualRule`], so it cannot enter [`crate::evaluate_hooks`], an armed
/// set, or any API accepting an ordinary [`Rule`]. The production loader stays
/// pinned to [`crate::SLICE1_CAPS`].
///
/// # The loader-to-evaluator boundary
/// The doc pair below is a mutation control over ONE edit — which loader minted
/// the value. The positive twin compiles, so the negative twin's failure is the
/// widened type being refused at an ordinary evaluator API, not some unrelated
/// breakage.
///
/// The ordinary loader's value is accepted:
///
/// ```
/// use policy::{CheckLimits, PageRef, Rule, ScopeLayer, evaluate_hooks_for_test,
///              load_rule, register_page};
///
/// fn ordinary_reaches_the_evaluator(page: &str, event: &effects::ChangeEvent) {
///     let registration = register_page(PageRef {
///         layer: ScopeLayer::Workspace,
///         page: "rules/notify.md",
///         bytes: page,
///     })
///     .expect("registers")
///     .expect("is a rule page");
///     let rule: Rule = load_rule(&registration, page, CheckLimits::default()).expect("loads");
///     let _ = evaluate_hooks_for_test(&[rule], event);
/// }
/// ```
///
/// The widened loader's value is not:
///
/// ```compile_fail
/// use policy::{CheckLimits, PageRef, ScopeLayer, evaluate_hooks_for_test,
///              load_rule_for_corpus, register_page};
///
/// fn widened_cannot_reach_the_evaluator(page: &str, event: &effects::ChangeEvent) {
///     let registration = register_page(PageRef {
///         layer: ScopeLayer::Workspace,
///         page: "rules/notify.md",
///         bytes: page,
///     })
///     .expect("registers")
///     .expect("is a rule page");
///     let proof = load_rule_for_corpus(&registration, page, CheckLimits::default())
///         .expect("loads");
///     let _ = evaluate_hooks_for_test(&[proof], event);
/// }
/// ```
///
/// # Errors
/// The same [`RuleLoadError`] surface as [`load_rule`].
pub fn load_rule_for_corpus(
    registration: &Registration,
    bytes: &str,
    limits: CheckLimits,
) -> Result<CounterfactualRule, RuleLoadError> {
    load_rule_with_hook_loader(
        registration,
        bytes,
        limits,
        crate::hook::load_hook_for_corpus,
    )
    .map(|inner| CounterfactualRule { inner })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registration::{PageRef, ScopeLayer, register_page};

    /// A loadable HOOK page — design § 5.2's form, tag-registered instead of
    /// filename-registered, and carrying NO `kind:` key.
    const HOOK_PAGE: &str = r#"---
tags: [type/rule, rules/hook]
id: task.review-notify
severity: info
paths: ["tasks/*.md"]
caps: [proto.send]
budget: { steps: 10000, mem: 4194304 }
how:
  route: { info: channel-review }
---

# task-review-notify

```starlark
def on_change(event):
    return []
```
"#;

    /// A loadable CHECK page: `paths:` plus the law predicate.
    const CHECK_PAGE: &str = r#"---
tags: [type/rule, rules/check]
id: reviewer-not-owner
paths: ["tasks/**"]
---

```starlark
def check_change(change):
    return []
```
"#;

    fn registered(page: &str, bytes: &str) -> Registration {
        register_page(PageRef {
            layer: ScopeLayer::Workspace,
            page,
            bytes,
        })
        .expect("the fixture registers")
        .expect("the fixture is a rule page")
    }

    fn load(page: &str, bytes: &str) -> Result<Rule, RuleLoadError> {
        load_rule(&registered(page, bytes), bytes, CheckLimits::default())
    }

    #[test]
    fn a_tagged_hook_page_loads_without_any_kind_frontmatter() {
        let rule = load("rules/notify.md", HOOK_PAGE).expect("a tagged hook page loads");
        assert_eq!(rule.id().as_str(), "task.review-notify");
        assert_eq!(rule.page(), "rules/notify.md");
        assert_eq!(rule.scope(), ["tasks/*.md"]);
        assert!(rule.matches_path("tasks/x.md"));
        assert!(!rule.matches_path("agents/x.md"));
        let hook = rule.hook().expect("the reaction leg loaded");
        assert_eq!(hook.severity(), "info");
        assert!(rule.check_source().is_none(), "no law leg was declared");
    }

    #[test]
    fn a_tagged_check_page_loads_its_law_leg() {
        let rule = load("rules/reviewer.md", CHECK_PAGE).expect("a tagged check page loads");
        assert_eq!(rule.scope(), ["tasks/**"]);
        assert!(rule.check_source().is_some());
        assert!(rule.hook().is_none());
    }

    #[test]
    fn a_page_with_no_law_leg_refuses_nothing() {
        let rule = load("rules/notify.md", HOOK_PAGE).unwrap();
        assert!(
            !rule
                .check_change(&a_change())
                .expect("a reaction-only page evaluates")
                .fired(),
            "a reaction never vetoes"
        );
    }

    /// A real splice change over one task document — the same construction the
    /// gate's fixtures use, so the reaction-only silence is measured against a
    /// change the evaluator would actually be handed.
    fn a_change() -> crate::Change {
        let doc = |md: &str| {
            let mut doc = model::build(md.to_string(), syntax::parse(md));
            if let model::NodeKind::Document { path, .. } = &mut doc.root.kind {
                *path = "tasks/x.md".to_string();
            }
            doc
        };
        crate::derive_change(
            &doc("---\nstatus: open\n---\n# x\n"),
            &doc("---\nstatus: review\n---\n# x\n"),
            &[],
            crate::Invocation {
                op: crate::ChangeOp::Splice,
                actor: Some("author"),
                force: false,
            },
            &[],
            &|_| None,
        )
    }

    /// **A defect the folder loader could not have.** Two tags on one page share
    /// ONE fenced block, so a page can claim the law leg while defining only the
    /// reaction's entry point. Under `CHECK.md` + `HOOK.md` the filenames kept the
    /// two apart; under tags only the entry point does, and the law's silence would
    /// otherwise surface as a runtime `MissingEntry` on the first real change —
    /// after the page had been registered, resolved, and possibly armed.
    #[test]
    fn a_declared_leg_without_its_entry_point_is_refused_at_load() {
        let both = HOOK_PAGE.replace("rules/hook]", "rules/hook, rules/check]");
        let err = load("rules/dual.md", &both).expect_err("a law that cannot fire never loads");
        assert_eq!(
            err,
            RuleLoadError::EntryMissing {
                page: "rules/dual.md".into(),
                kind: RuleKind::Check,
                entry: "check_change",
            }
        );
        let rendered = err.to_string();
        assert!(rendered.contains("rules/check"), "{rendered}");
        assert!(rendered.contains("check_change"), "{rendered}");
    }

    #[test]
    fn a_dual_leg_page_defining_both_entry_points_loads_both() {
        let both = HOOK_PAGE
            .replace("rules/hook]", "rules/hook, rules/check]")
            .replace(
                "def on_change(event):\n    return []",
                "def on_change(event):\n    return []\n\ndef check_change(change):\n    return []",
            );
        let rule = load("rules/dual.md", &both).expect("both legs load");
        assert!(rule.check_source().is_some(), "the law leg loaded");
        assert!(rule.hook().is_some(), "the reaction leg loaded");
        assert_eq!(
            rule.scope(),
            ["tasks/*.md"],
            "a dual page answers scope through its law's `paths:`"
        );
    }

    #[test]
    fn a_dual_leg_page_must_satisfy_both_loads() {
        // Declares both tags, and gets the LAW's declaration wrong: `paths:` is
        // absent, so neither leg admits — half a rule is a rule whose author does
        // not know which half is running.
        let both = HOOK_PAGE
            .replace("rules/hook]", "rules/hook, rules/check]")
            .replace("paths: [\"tasks/*.md\"]\n", "");
        let err = load("rules/dual.md", &both).expect_err("half a rule never loads");
        let RuleLoadError::Leg { kind, .. } = &err else {
            panic!("expected a leg fault, got {err:?}");
        };
        assert_eq!(*kind, RuleKind::Check);
        assert!(err.to_string().contains("rules/check"), "{err}");
    }

    // ── the kind seam (ruled 2026-08-01) ──────────────────────────────────────

    /// Arm 1: a `kind:` that RESTATES the tag is legal — the corpus is full of
    /// pages carrying it, and a migration that refused them would make the tag law
    /// a rewrite order rather than a registration change.
    #[test]
    fn a_kind_that_agrees_with_the_tag_loads() {
        let with_kind = HOOK_PAGE.replace("severity: info", "kind: hook\nseverity: info");
        let rule = load("rules/notify.md", &with_kind).expect("an agreeing kind is legal");
        assert_eq!(rule.kinds(), &[RuleKind::Hook]);
        assert!(rule.hook().is_some());
    }

    /// Arm 2: an ABSENT `kind:` derives from the tag — the preferred form, and the
    /// only one that keeps one name for one thing.
    #[test]
    fn an_absent_kind_derives_from_the_tag() {
        assert!(
            !HOOK_PAGE.contains("kind:"),
            "the fixture must carry no kind: for this to mean anything"
        );
        let rule = load("rules/notify.md", HOOK_PAGE).expect("the tag alone is enough");
        assert_eq!(
            rule.kinds(),
            &[RuleKind::Hook],
            "the leg is derived from the tag, not from a key"
        );
        assert!(rule.hook().is_some(), "and the derived leg is what loaded");
    }

    /// Arm 3: a CONTRADICTING `kind:` fails load loudly. Without this the page arms
    /// as a hook, calls itself a check, and the disagreement surfaces on the first
    /// write instead of at load.
    #[test]
    fn a_kind_that_contradicts_the_tag_fails_load_loudly() {
        for declared in ["check", "schedule", "hook-ish"] {
            let conflicting = HOOK_PAGE.replace(
                "severity: info",
                &format!("kind: {declared}\nseverity: info"),
            );
            let err = load("rules/notify.md", &conflicting).expect_err("a contradiction is loud");
            assert_eq!(
                err,
                RuleLoadError::KindDisagrees {
                    page: "rules/notify.md".into(),
                    declared: declared.to_string(),
                    tags: vec![RuleKind::Hook],
                },
                "kind: {declared}"
            );
            let rendered = err.to_string();
            assert!(rendered.contains("rules/notify.md"), "{rendered}");
            assert!(rendered.contains("rules/hook"), "{rendered}");
            assert!(rendered.contains(declared), "{rendered}");
        }
    }

    /// A `kind:` that is not text cannot agree with a tag, and deciding what it
    /// equals would mean evaluating it — the layering forbids that here exactly as
    /// it does for a block-declared id.
    #[test]
    fn a_non_string_kind_cannot_agree() {
        let listed = HOOK_PAGE.replace("severity: info", "kind: [hook]\nseverity: info");
        let err = load("rules/notify.md", &listed).expect_err("a list is not a kind");
        assert!(
            matches!(err, RuleLoadError::KindDisagrees { ref declared, .. } if declared.is_empty()),
            "{err:?}"
        );
    }

    /// A dual-leg page's `kind:` may name EITHER registered leg: it is not lying
    /// about the leg it names, and it cannot name both. The tags remain the whole
    /// truth — which is why they, not the key, decide what loads.
    #[test]
    fn a_dual_leg_page_admits_a_kind_naming_either_leg() {
        let dual = HOOK_PAGE
            .replace("rules/hook]", "rules/hook, rules/check]")
            .replace(
                "def on_change(event):\n    return []",
                "def on_change(event):\n    return []\n\ndef check_change(change):\n    return []",
            );
        for declared in ["hook", "check"] {
            let body = dual.replace(
                "severity: info",
                &format!("kind: {declared}\nseverity: info"),
            );
            let rule = load("rules/dual.md", &body).expect("either registered leg may be named");
            assert_eq!(rule.kinds(), &[RuleKind::Check, RuleKind::Hook]);
        }
    }

    #[test]
    fn bytes_that_are_not_the_registered_bytes_are_refused() {
        let registration = registered("rules/notify.md", HOOK_PAGE);
        let edited = HOOK_PAGE.replace("severity: info", "severity: error");
        let err = load_rule(&registration, &edited, CheckLimits::default())
            .expect_err("drift between discovery and load is loud");
        let RuleLoadError::RevMismatch {
            page,
            registered: pinned,
            supplied,
        } = &err
        else {
            panic!("expected a rev mismatch, got {err:?}");
        };
        assert_eq!(page, "rules/notify.md");
        assert_eq!(pinned, registration.rev());
        assert_eq!(*supplied, page_rev(&edited));
        assert_ne!(pinned, supplied);
    }

    #[test]
    fn a_declared_leg_that_does_not_load_names_the_page_and_the_leg() {
        // A hook page missing `caps:` — the declaration is incomplete, and the
        // refusal must say which page and which leg rather than which file.
        let no_caps = HOOK_PAGE.replace("caps: [proto.send]\n", "");
        let err = load("rules/notify.md", &no_caps).expect_err("an incomplete hook is loud");
        assert_eq!(err.page(), "rules/notify.md");
        let rendered = err.to_string();
        assert!(rendered.contains("rules/notify.md"), "{rendered}");
        assert!(rendered.contains("rules/hook"), "{rendered}");
        assert!(rendered.contains("caps:"), "{rendered}");
    }

    /// The founding demo's frontmatter, verbatim from
    /// `18-02-meridian-rs/…/demo/rules/task-review-notify.md` AFTER the registration
    /// migration — the tag and the id are added, the founding declaration is not
    /// rewritten. Registration and loadability are different questions, and this
    /// page is the case that proves it: it registers by tag, and it still does not
    /// load, for the three incompatibilities `25e6d3ac` pinned (`scope:` not
    /// `paths:`, string budgets, no `caps:`).
    const FOUNDING_DEMO: &str = r#"---
type: rule
rule: task-review-notify
pack: ccc-session@3
kind: hook
severity: info
scope: "tasks/*.md"
budget: { steps: 20k, mem: 2mb }
how:
  route: { info: channel-review, error: telegram }
  batching: 30s
  wake_policy: never-cold
tags: [type/rule, topic/ccc-sessions, topic/demo, meta/fixture, rules/hook]
id: task.review-notify
---

# task-review-notify

```star
def when(delta, facts, now):
    return None
```
"#;

    #[test]
    fn the_founding_demo_registers_by_tag_and_still_does_not_load() {
        let registration = registered("demo/rules/task-review-notify.md", FOUNDING_DEMO);
        assert_eq!(registration.id().as_str(), "task.review-notify");
        assert_eq!(registration.kinds(), &[RuleKind::Hook]);

        let err = load_rule(&registration, FOUNDING_DEMO, CheckLimits::default())
            .expect_err("the founding record is a record, not a loadable declaration");
        assert!(
            matches!(
                err,
                RuleLoadError::Leg {
                    kind: RuleKind::Hook,
                    ..
                }
            ),
            "{err:?}"
        );
    }
}
