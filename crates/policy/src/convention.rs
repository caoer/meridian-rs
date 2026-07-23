//! The convention loader (U1.3) — `conventions/<slug>/` folder grammar, the CHECK
//! capability ceiling, and `paths:` scope.
//!
//! # What a convention is (rulings § the unit — scenario spec)
//! A convention is an IN-TREE folder named for its subject slug:
//!
//! ```text
//! conventions/reviewer-not-owner/
//!   CHECK.md          # the law: `paths:` frontmatter + a fenced
//!                     # `def check_change(change)` starlark predicate
//!   base/             # the before-world (fixture space, U1.2's mount map)
//!   scenarios/        # teaching + test pages (fixture space)
//! ```
//!
//! It is `conventions/` IN-TREE, never a dot-dir — dot-dirs sit outside the hash
//! domain and cannot carry attested law (rulings § scoping).
//!
//! # What v1 loads (rulings § v1 ships CHECK only)
//! v1 loads the CHECK capability ONLY. A convention that declares a FIX / HOOK /
//! VIEW file is refused with a named deferral ([`LoadError::CapabilityDeferred`]) —
//! those are named power ceilings deferred until a real subject needs them, never
//! silently ignored. The CHECK power ceiling itself is enforced by the evaluator
//! ([`crate::check_eval`]).
//!
//! # Scope (rulings § scoping — the Claude-rules pattern)
//! `CHECK.md` frontmatter declares `paths:` — a flat glob list (obsidian-legal),
//! the convention's default scope. [`Convention::matches_path`] answers whether a
//! document path is in scope; the attested INDEX row may narrow it further at
//! arming (U1.4), never widen it.

use crate::change::Change;
use crate::check_eval::{self, CheckError, CheckLimits, CheckTelemetry};

/// The four capability files a convention folder may carry. Each earns a file iff
/// it needs a distinct power ceiling (rulings § capability grammar); v1 loads
/// [`Capability::Check`] and defers the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    /// `CHECK.md` — reads the change + pinned facts, produces findings / refusals.
    Check,
    /// `FIX.md` — mutates the change under caps (deferred).
    Fix,
    /// `HOOK.md` — reacts outward to the landed change under effect caps (deferred).
    Hook,
    /// `VIEW.md` — the capability-locked read face (deferred).
    View,
}

impl Capability {
    /// The capability name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Capability::Check => "CHECK",
            Capability::Fix => "FIX",
            Capability::Hook => "HOOK",
            Capability::View => "VIEW",
        }
    }

    /// The file that declares this capability inside a convention folder.
    #[must_use]
    pub fn filename(self) -> &'static str {
        match self {
            Capability::Check => "CHECK.md",
            Capability::Fix => "FIX.md",
            Capability::Hook => "HOOK.md",
            Capability::View => "VIEW.md",
        }
    }

    /// The capabilities deferred in v1, in declaration order.
    const DEFERRED: [Capability; 3] = [Capability::Fix, Capability::Hook, Capability::View];
}

/// Caller-provided access to a convention folder's files — the loader stays
/// I/O-free (as `model` is), so the caller (`fs`/`sidecar` at the disk edge) injects
/// file access and tests inject an in-memory or embedded map. Paths are relative to
/// the convention root (`CHECK.md`, `FIX.md`, …).
pub trait ConventionFiles {
    /// Read a file's UTF-8 contents relative to the convention root, or fail
    /// (missing / unreadable / non-UTF-8).
    ///
    /// # Errors
    /// Any I/O or decode failure from the underlying source.
    fn read(&self, rel_path: &str) -> std::io::Result<String>;

    /// Whether a file exists relative to the convention root (the capability-ceiling
    /// probe — does the folder declare FIX / HOOK / VIEW).
    fn exists(&self, rel_path: &str) -> bool;
}

/// Why a convention did not load. Every malformed / over-reaching input lands as one
/// of these typed errors; the loader fails loud, never admits a half-read
/// convention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// The slug is empty, contains a path separator or `..`, or begins with `.`
    /// (a dot-dir — outside the hash domain, cannot carry attested law).
    SlugInvalid { slug: String, reason: String },
    /// `CHECK.md` is absent or unreadable — a convention with no CHECK has no law.
    CheckMissing { detail: String },
    /// The convention declares a deferred capability (FIX / HOOK / VIEW). v1 loads
    /// CHECK only; the refusal names the capability and the deferral.
    CapabilityDeferred { capability: Capability },
    /// `CHECK.md` is malformed: no frontmatter, no `paths:` scope, an empty scope,
    /// or no fenced `def check_change` predicate block.
    Malformed { reason: String },
    /// The CHECK predicate failed the load gate (over-long, over-nested, or
    /// unparseable starlark) under the full limits.
    CheckInvalid { source: CheckError },
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::SlugInvalid { slug, reason } => {
                write!(f, "convention slug '{slug}' is invalid: {reason}")
            }
            LoadError::CheckMissing { detail } => {
                write!(f, "convention has no readable CHECK.md: {detail}")
            }
            LoadError::CapabilityDeferred { capability } => write!(
                f,
                "convention declares a {cap} file, but v1 ships CHECK only — the {cap} \
                 capability is a named power ceiling deferred until a real subject \
                 needs it (rulings § v1 ships CHECK only)",
                cap = capability.as_str()
            ),
            LoadError::Malformed { reason } => write!(f, "CHECK.md is malformed: {reason}"),
            LoadError::CheckInvalid { source } => {
                write!(f, "CHECK.md predicate failed the load gate: {source}")
            }
        }
    }
}

impl std::error::Error for LoadError {}

/// One recorded refusal — the teaching finding a CHECK emits. `message` says what is
/// wrong; `passing_scenario` cites the legal path (the passing scenario), so every
/// refusal points at the way to do it right (rulings § refusals cite the passing
/// scenario).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// What is wrong with the change (the teaching message).
    pub message: String,
    /// The passing scenario the refusal cites — the legal path.
    pub passing_scenario: String,
}

/// The outcome of running a convention's CHECK over one change: the refusals it
/// emitted. A convention **fires** when it emitted at least one refusal; it
/// **passes** when it emitted none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckOutcome {
    /// The refusals the CHECK emitted, in emission order.
    pub refusals: Vec<Refusal>,
}

impl CheckOutcome {
    /// Whether the CHECK fired (emitted at least one refusal).
    #[must_use]
    pub fn fired(&self) -> bool {
        !self.refusals.is_empty()
    }
}

/// A loaded convention: its slug, its `paths:` scope, and its parse-validated CHECK
/// predicate. Construction is sealed to [`load_convention`] (the capability seal) —
/// a `Convention` in hand has passed the folder grammar, the capability ceiling, and
/// the full-limits load gate.
#[derive(Debug, Clone)]
pub struct Convention {
    slug: String,
    scope: Vec<String>,
    check_source: String,
    limits: CheckLimits,
}

impl Convention {
    /// The convention's subject slug (its folder name).
    #[must_use]
    pub fn slug(&self) -> &str {
        &self.slug
    }

    /// The `paths:` scope globs the convention declared.
    #[must_use]
    pub fn scope(&self) -> &[String] {
        &self.scope
    }

    /// Whether `path` is in the convention's declared scope — true iff it matches
    /// any `paths:` glob. A document outside the scope is not the convention's
    /// concern (the CHECK is never run against it).
    #[must_use]
    pub fn matches_path(&self, path: &str) -> bool {
        self.scope.iter().any(|glob| glob_match(glob, path))
    }

    /// Run the CHECK predicate over one [`Change`] under the convention's full
    /// limits, returning the [`CheckOutcome`] (the refusals it emitted).
    ///
    /// The caller is responsible for scoping — `check_change` runs on the change it
    /// is handed. Pair with [`Convention::matches_path`] on the change's document
    /// path to skip out-of-scope documents.
    ///
    /// # Errors
    /// [`CheckError`] — a budget/parse/runtime fault or a missing `check_change`.
    pub fn check_change(&self, change: &Change) -> Result<CheckOutcome, CheckError> {
        let refusals = check_eval::run_check_change(&self.check_source, change, self.limits)?;
        Ok(CheckOutcome { refusals })
    }

    /// Run the CHECK over one [`Change`] and return the [`CheckTelemetry`] — the
    /// refusals AND the exact fuel + heap the evaluation spent. Same metered core
    /// as [`Convention::check_change`]; the `test --corpus` tier (U1.5) reads the
    /// telemetry for its fuel + heap p50/p99 budgets.
    ///
    /// # Errors
    /// [`CheckError`] — a budget/parse/runtime fault or a missing `check_change`.
    pub fn check_change_metered(&self, change: &Change) -> Result<CheckTelemetry, CheckError> {
        check_eval::run_check_change_metered(&self.check_source, change, self.limits)
    }

    /// The CHECK source (for tests and later units that re-run it).
    #[must_use]
    pub fn check_source(&self) -> &str {
        &self.check_source
    }
}

/// Load the convention `conventions/<slug>/` through the injected `files` accessor
/// under the given full limits.
///
/// Pipeline: validate the slug (never a dot-dir) → require `CHECK.md` → refuse any
/// declared deferred capability (FIX / HOOK / VIEW) → parse `CHECK.md`'s `paths:`
/// scope + fenced `def check_change` predicate → parse-gate the predicate under the
/// full limits → admit.
///
/// # Errors
/// [`LoadError`] — see its variants.
pub fn load_convention(
    slug: &str,
    files: &dyn ConventionFiles,
    limits: CheckLimits,
) -> Result<Convention, LoadError> {
    validate_slug(slug)?;

    // 1. CHECK.md is required — a convention with no CHECK has no law.
    let check_md =
        files
            .read(Capability::Check.filename())
            .map_err(|e| LoadError::CheckMissing {
                detail: e.to_string(),
            })?;

    // 2. Capability ceiling — v1 ships CHECK only. A declared FIX / HOOK / VIEW is a
    //    named deferral, never a silent drop.
    for capability in Capability::DEFERRED {
        if files.exists(capability.filename()) {
            return Err(LoadError::CapabilityDeferred { capability });
        }
    }

    // 3. Parse CHECK.md: `paths:` scope frontmatter + the fenced predicate.
    let scope = parse_scope(&check_md)?;
    let check_source =
        crate::pack::extract_fenced_starlark(&check_md).ok_or_else(|| LoadError::Malformed {
            reason: "no fenced ```starlark block defining `def check_change(change)`".to_string(),
        })?;

    // 4. Parse-gate the predicate under the FULL limits (source-size + nesting +
    //    parse) — authoring faults surface here, once, at load.
    check_eval::validate_check_source(&check_source, limits)
        .map_err(|source| LoadError::CheckInvalid { source })?;

    Ok(Convention {
        slug: slug.to_string(),
        scope,
        check_source,
        limits,
    })
}

/// Validate a convention slug: non-empty, no path separator, no `..`, and never a
/// dot-dir (rulings § scoping — dot-dirs sit outside the hash domain).
fn validate_slug(slug: &str) -> Result<(), LoadError> {
    let invalid = |reason: &str| LoadError::SlugInvalid {
        slug: slug.to_string(),
        reason: reason.to_string(),
    };
    if slug.is_empty() {
        return Err(invalid("empty"));
    }
    if slug.starts_with('.') {
        return Err(invalid(
            "a dot-dir — conventions live in-tree, never a dot-dir",
        ));
    }
    if slug.contains('/') || slug.contains('\\') {
        return Err(invalid("contains a path separator"));
    }
    if slug.split('/').any(|seg| seg == "..") || slug == ".." {
        return Err(invalid("contains `..`"));
    }
    Ok(())
}

/// Only the CHECK.md frontmatter key the loader reads — the `paths:` scope. Other
/// keys are permitted (a convention may carry descriptive frontmatter) and ignored.
#[derive(serde::Deserialize)]
struct ScopeFrontmatter {
    paths: Option<Vec<String>>,
}

/// Parse `CHECK.md`'s `paths:` scope. A convention MUST declare a non-empty scope —
/// a convention with no declared scope applies to nothing (fail-closed), so a
/// missing or empty `paths:` is loud.
fn parse_scope(check_md: &str) -> Result<Vec<String>, LoadError> {
    let (frontmatter, _body) =
        crate::pack::split_frontmatter(check_md).ok_or_else(|| LoadError::Malformed {
            reason: "no `---` frontmatter declaring `paths:`".to_string(),
        })?;
    let parsed: ScopeFrontmatter =
        serde_yaml::from_str(frontmatter).map_err(|e| LoadError::Malformed {
            reason: format!("frontmatter parse: {e}"),
        })?;
    let paths = parsed.paths.unwrap_or_default();
    if paths.is_empty() {
        return Err(LoadError::Malformed {
            reason: "frontmatter must declare a non-empty `paths:` scope".to_string(),
        });
    }
    Ok(paths)
}

// ── obsidian-legal glob matching ──────────────────────────────────────────────

/// Match a `path` against one obsidian-legal glob. Segments split on `/`; `**`
/// matches zero or more whole segments; within a segment `*` matches any run of
/// non-`/` characters and every other character is literal. This is the flat glob
/// grammar `paths:` declares (rulings § scoping — the Claude-rules pattern).
fn glob_match(pattern: &str, path: &str) -> bool {
    let pat: Vec<&str> = pattern.split('/').collect();
    let txt: Vec<&str> = path.split('/').collect();
    seg_match(&pat, &txt)
}

/// Segment-list match with `**` spanning zero or more segments.
fn seg_match(pat: &[&str], txt: &[&str]) -> bool {
    match pat.split_first() {
        None => txt.is_empty(),
        Some((&"**", rest)) => {
            // `**` matches zero segments here, or one segment then `**` again.
            if seg_match(rest, txt) {
                return true;
            }
            !txt.is_empty() && seg_match(pat, &txt[1..])
        }
        Some((&seg, rest)) => match txt.split_first() {
            Some((&head, txt_rest)) if segment_match(seg, head) => seg_match(rest, txt_rest),
            _ => false,
        },
    }
}

/// Within-segment match: `*` matches any run of non-`/` characters, every other
/// character is literal.
fn segment_match(pat: &str, txt: &str) -> bool {
    let p: Vec<char> = pat.chars().collect();
    let t: Vec<char> = txt.chars().collect();
    star_match(&p, &t)
}

/// Classic `*`-glob match over char slices (`*` = zero or more of anything).
fn star_match(pat: &[char], txt: &[char]) -> bool {
    match pat.split_first() {
        None => txt.is_empty(),
        Some(('*', rest)) => {
            if star_match(rest, txt) {
                return true;
            }
            !txt.is_empty() && star_match(pat, &txt[1..])
        }
        Some((&c, rest)) => match txt.split_first() {
            Some((&h, txt_rest)) if h == c => star_match(rest, txt_rest),
            _ => false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// An in-memory convention folder for the loader fixtures.
    struct MemFiles(BTreeMap<String, String>);

    impl MemFiles {
        fn new() -> Self {
            Self(BTreeMap::new())
        }
        fn with(mut self, rel: &str, body: &str) -> Self {
            self.0.insert(rel.to_string(), body.to_string());
            self
        }
    }

    impl ConventionFiles for MemFiles {
        fn read(&self, rel_path: &str) -> std::io::Result<String> {
            self.0.get(rel_path).cloned().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, format!("no {rel_path}"))
            })
        }
        fn exists(&self, rel_path: &str) -> bool {
            self.0.contains_key(rel_path)
        }
    }

    const VALID_CHECK: &str = "\
---
paths:
  - tasks/**
---

# reviewer-not-owner

```starlark
def check_change(change):
    pass
```
";

    #[test]
    fn valid_convention_loads() {
        let files = MemFiles::new().with("CHECK.md", VALID_CHECK);
        let conv = load_convention("reviewer-not-owner", &files, CheckLimits::default())
            .expect("a well-formed convention loads");
        assert_eq!(conv.slug(), "reviewer-not-owner");
        assert_eq!(conv.scope(), &["tasks/**".to_string()]);
    }

    #[test]
    fn fix_file_refuses_with_deferral_text() {
        let files = MemFiles::new()
            .with("CHECK.md", VALID_CHECK)
            .with("FIX.md", "# a deferred fix\n");
        let err = load_convention("reviewer-not-owner", &files, CheckLimits::default())
            .expect_err("a convention declaring FIX is refused");
        assert_eq!(
            err,
            LoadError::CapabilityDeferred {
                capability: Capability::Fix
            }
        );
        let text = err.to_string();
        assert!(text.contains("FIX"), "names the capability: {text}");
        assert!(
            text.contains("v1 ships CHECK only"),
            "names the deferral: {text}"
        );
    }

    #[test]
    fn hook_and_view_also_defer() {
        for (rel, capability) in [("HOOK.md", Capability::Hook), ("VIEW.md", Capability::View)] {
            let files = MemFiles::new()
                .with("CHECK.md", VALID_CHECK)
                .with(rel, "# deferred\n");
            let err = load_convention("s", &files, CheckLimits::default()).unwrap_err();
            assert_eq!(err, LoadError::CapabilityDeferred { capability });
        }
    }

    #[test]
    fn out_of_scope_path_not_matched() {
        let files = MemFiles::new().with("CHECK.md", VALID_CHECK);
        let conv = load_convention("reviewer-not-owner", &files, CheckLimits::default()).unwrap();
        assert!(conv.matches_path("tasks/fix-parser.md"), "in scope");
        assert!(conv.matches_path("tasks/deep/nested.md"), "** spans depth");
        assert!(
            !conv.matches_path("notes/plan.md"),
            "a document outside tasks/** is not the convention's concern"
        );
        assert!(
            !conv.matches_path("tasksed/x.md"),
            "a sibling directory sharing the prefix is not in scope"
        );
    }

    #[test]
    fn missing_check_is_loud() {
        let files = MemFiles::new().with("VIEW.md", "# no check\n");
        assert!(matches!(
            load_convention("s", &files, CheckLimits::default()),
            Err(LoadError::CheckMissing { .. })
        ));
    }

    #[test]
    fn dot_dir_slug_is_refused() {
        let files = MemFiles::new().with("CHECK.md", VALID_CHECK);
        assert!(matches!(
            load_convention(".hidden", &files, CheckLimits::default()),
            Err(LoadError::SlugInvalid { .. })
        ));
    }

    #[test]
    fn missing_paths_scope_is_malformed() {
        let no_scope =
            "---\ntitle: x\n---\n\n```starlark\ndef check_change(change):\n    pass\n```\n";
        let files = MemFiles::new().with("CHECK.md", no_scope);
        assert!(matches!(
            load_convention("s", &files, CheckLimits::default()),
            Err(LoadError::Malformed { .. })
        ));
    }

    #[test]
    fn no_predicate_block_is_malformed() {
        let no_pred = "---\npaths:\n  - tasks/**\n---\n\n# prose only, no fenced starlark\n";
        let files = MemFiles::new().with("CHECK.md", no_pred);
        assert!(matches!(
            load_convention("s", &files, CheckLimits::default()),
            Err(LoadError::Malformed { .. })
        ));
    }

    #[test]
    fn unparseable_predicate_fails_the_load_gate() {
        let bad = "---\npaths:\n  - tasks/**\n---\n\n```starlark\ndef check_change(:\n```\n";
        let files = MemFiles::new().with("CHECK.md", bad);
        assert!(matches!(
            load_convention("s", &files, CheckLimits::default()),
            Err(LoadError::CheckInvalid { .. })
        ));
    }

    #[test]
    fn glob_grammar_matches_obsidian_shapes() {
        assert!(glob_match("tasks/**", "tasks/a.md"));
        assert!(glob_match("tasks/**", "tasks/a/b/c.md"));
        assert!(
            glob_match("tasks/**", "tasks"),
            "** matches zero segments too"
        );
        assert!(!glob_match("tasks/**", "notes/a.md"));
        assert!(glob_match("*.md", "plan.md"));
        assert!(!glob_match("*.md", "plan.txt"));
        assert!(glob_match("notes/*.md", "notes/plan.md"));
        assert!(
            !glob_match("notes/*.md", "notes/deep/plan.md"),
            "* stays within a segment"
        );
        assert!(glob_match("**/verdict.md", "a/b/verdict.md"));
        assert!(glob_match("**/verdict.md", "verdict.md"));
    }
}
