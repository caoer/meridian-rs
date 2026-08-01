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
//! ([`crate::convention::parse_check`], [`crate::hook::load_hook_page`]) — they were
//! always page-shaped, taking bytes rather than filenames. What dies with the folder
//! loader is the addressing above them, not the parsing.
//!
//! # Bytes must be the registered bytes
//! [`load_rule`] verifies `page_rev(bytes)` against the rev the registration pinned
//! and refuses on mismatch. A caller that walks, registers, then re-reads a page an
//! editor has moved under it would otherwise evaluate one page's law under another
//! page's identity — silently, and only sometimes.

use crate::check_eval::CheckLimits;
use crate::convention::{CheckOutcome, LoadError};
use crate::hook::Hook;
use crate::registration::{Registration, RuleId, RuleKind, page_rev};

/// A loaded rule page: its identity, its declared legs, and its scope.
/// Construction is sealed to [`load_rule`], so a `Rule` in hand has passed the
/// registration gates AND its declarations' load gates.
#[derive(Debug, Clone)]
pub struct Rule {
    id: RuleId,
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
        crate::convention::path_in_scope(&self.scope, path)
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
            RuleLoadError::RevMismatch { .. } | RuleLoadError::EntryMissing { .. } => None,
        }
    }
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
    let supplied = page_rev(bytes);
    if supplied != registration.rev() {
        return Err(RuleLoadError::RevMismatch {
            page: registration.page().to_string(),
            registered: registration.rev().to_string(),
            supplied,
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
        .then(|| crate::convention::parse_check(bytes, limits).map_err(leg(RuleKind::Check)))
        .transpose()?;
    let hook = registration
        .kinds()
        .contains(&RuleKind::Hook)
        .then(|| crate::hook::load_hook_page(bytes, limits).map_err(leg(RuleKind::Hook)))
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
        page: registration.page().to_string(),
        scope,
        check_source: check.map(|(_, source)| source),
        hook,
        limits,
    })
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
