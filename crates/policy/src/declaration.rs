//! The DECLARATION layer — what a rule page's legs say, and the one scope
//! grammar every leg answers through.
//!
//! # Two layers, one page (ruling § 1, § 5)
//! Registration answers IDENTITY: a page carries `rules/check` / `rules/hook` in
//! its `tags:` and an `id:`, and that is the whole of what makes it a known rule
//! ([`crate::registration`]). Declaration answers EVALUABILITY: the `paths:` scope
//! and the fenced predicate that make the named leg runnable. This module is the
//! second half, and it reads BYTES — never a filename.
//!
//! # Why nothing here names a file
//! The generation this replaces keyed a rule on its FOLDER: `conventions/<slug>/`
//! held `CHECK.md` and `HOOK.md`, the slug was the identity, and the capability
//! was the filename. Under tag registration the page IS the rule, so the leg is
//! named by a tag and the identity by an `id:`. What survived the cut is exactly
//! what was never about filenames: the declaration parser ([`parse_check`]) and
//! the `paths:` glob grammar ([`path_in_scope`]) that both legs share, so a
//! capability can never drift into a second glob dialect.
//!
//! A [`LoadError`] here therefore names the LEG, never a file. The caller
//! ([`crate::rule::RuleLoadError`]) supplies which page it was reading.

use crate::check_eval::{self, CheckError, CheckLimits};

/// Why a declared leg did not load. Every malformed / over-reaching declaration
/// lands as one of these typed errors; the loader fails loud, never admits a
/// half-read rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// The check declaration is malformed: no frontmatter, no `paths:` scope, an
    /// empty scope, or no fenced `def check_change` predicate block.
    Malformed { reason: String },
    /// The CHECK predicate failed the load gate (over-long, over-nested, or
    /// unparseable starlark) under the full limits.
    CheckInvalid { source: CheckError },
    /// The hook declaration is malformed — the reason names the offending field.
    HookMalformed { reason: String },
    /// The hook declares a real effect cap that slice 1 does not carry. A named
    /// ceiling, never a silent ignore.
    HookCapDeferred { cap: String },
    /// The HOOK predicate failed the load gate (over-long, over-nested, or
    /// unparseable starlark) under the full limits.
    HookPredicateInvalid { reason: String },
    /// The HOOK predicate reaches for a constructor its declared `caps:` do not
    /// grant — refused at LOAD, before any change reaches it.
    HookCeiling { reason: String },
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Malformed { reason } => {
                write!(f, "the check declaration is malformed: {reason}")
            }
            LoadError::CheckInvalid { source } => {
                write!(f, "the check predicate failed the load gate: {source}")
            }
            LoadError::HookMalformed { reason } => {
                write!(f, "the hook declaration is malformed: {reason}")
            }
            LoadError::HookCapDeferred { cap } => write!(
                f,
                "the hook declares the cap `{cap}`, which slice 1 does not carry — slice 1 \
                 admits `proto.send` only. The cap is a named power ceiling deferred until a \
                 real subject needs it, never silently ignored"
            ),
            LoadError::HookPredicateInvalid { reason } => {
                write!(f, "the hook predicate failed the load gate: {reason}")
            }
            LoadError::HookCeiling { reason } => {
                write!(
                    f,
                    "the hook predicate is outside its capability ceiling: {reason}"
                )
            }
        }
    }
}

impl std::error::Error for LoadError {}

/// One recorded refusal — the teaching finding a CHECK emits. `message` says what
/// is wrong; `passing_scenario` cites the legal path, so every refusal points at
/// the way to do it right (rulings § refusals cite the passing scenario).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// What is wrong with the change (the teaching message).
    pub message: String,
    /// The passing case the refusal cites — the legal path.
    pub passing_scenario: String,
}

/// The outcome of running a rule's CHECK leg over one change: the refusals it
/// emitted. A check **fires** when it emitted at least one refusal; it **passes**
/// when it emitted none.
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

/// Parse the CHECK leg from a rule page's bytes: the `paths:` scope frontmatter
/// and the fenced predicate, parse-gated under the FULL limits so authoring faults
/// surface here, once, at load.
///
/// # Errors
/// [`LoadError`] — see its variants.
pub(crate) fn parse_check(
    page: &str,
    limits: CheckLimits,
) -> Result<(Vec<String>, String), LoadError> {
    let scope = parse_scope(page)?;
    let source =
        crate::pack::extract_fenced_starlark(page).ok_or_else(|| LoadError::Malformed {
            reason: "no fenced ```starlark block defining `def check_change(change)`".to_string(),
        })?;
    check_eval::validate_check_source(&source, limits)
        .map_err(|source| LoadError::CheckInvalid { source })?;
    Ok((scope, source))
}

#[derive(serde::Deserialize)]
struct ScopeFrontmatter {
    paths: Option<Vec<String>>,
}

/// Parse a check leg's `paths:` scope. A check MUST declare a non-empty scope — a
/// law that applies to nothing is fail-closed nonsense, so a missing or empty
/// `paths:` is loud.
fn parse_scope(page: &str) -> Result<Vec<String>, LoadError> {
    let (frontmatter, _body) =
        crate::pack::split_frontmatter(page).ok_or_else(|| LoadError::Malformed {
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

/// Whether `path` matches any glob in `scope` — the one scope answer, shared by
/// [`crate::Rule::matches_path`] and [`crate::Hook::matches_path`] so a leg can
/// never drift into a second glob grammar.
pub(crate) fn path_in_scope(scope: &[String], path: &str) -> bool {
    scope.iter().any(|glob| glob_match(glob, path))
}

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

    const PREDICATE: &str = "```starlark\ndef check_change(change):\n    pass\n```\n";

    fn check_page(frontmatter: &str) -> String {
        format!("---\n{frontmatter}---\n\n# rule\n\n{PREDICATE}")
    }

    // ── the check declaration ────────────────────────────────────────────────

    #[test]
    fn a_well_formed_check_declaration_yields_its_scope_and_predicate() {
        let page = check_page("paths:\n  - tasks/**\n");
        let (scope, source) = parse_check(&page, CheckLimits::default()).expect("loads");
        assert_eq!(scope, vec!["tasks/**".to_string()]);
        assert!(source.contains("def check_change"));
    }

    #[test]
    fn a_check_declaration_without_frontmatter_is_malformed() {
        let err = parse_check(PREDICATE, CheckLimits::default()).expect_err("no frontmatter");
        assert!(
            matches!(&err, LoadError::Malformed { reason } if reason.contains("frontmatter")),
            "{err}"
        );
    }

    /// Fail-closed: a law with no declared scope applies to nothing, so the
    /// absence is loud rather than an empty-scope no-op.
    #[test]
    fn an_empty_or_missing_paths_scope_is_loud() {
        for frontmatter in ["paths: []\n", "id: a.check\n"] {
            let err = parse_check(&check_page(frontmatter), CheckLimits::default())
                .expect_err("an empty scope must be refused");
            assert!(
                matches!(&err, LoadError::Malformed { reason } if reason.contains("paths")),
                "{frontmatter:?}: {err}"
            );
        }
    }

    #[test]
    fn a_check_declaration_without_a_fenced_block_is_malformed() {
        let page = "---\npaths:\n  - tasks/**\n---\n\n# rule\n\nno predicate here\n";
        let err = parse_check(page, CheckLimits::default()).expect_err("no fenced block");
        assert!(
            matches!(&err, LoadError::Malformed { reason } if reason.contains("starlark")),
            "{err}"
        );
    }

    /// The refusal names the LEG, never a file — the page-shaped generation has no
    /// `CHECK.md` to send a reader looking for.
    #[test]
    fn a_declaration_refusal_never_names_a_file() {
        let err = parse_check(PREDICATE, CheckLimits::default()).unwrap_err();
        let rendered = err.to_string();
        assert!(
            !rendered.contains("CHECK.md") && !rendered.contains("HOOK.md"),
            "the declaration layer names legs, not filenames: {rendered}"
        );
        assert!(rendered.contains("check declaration"), "{rendered}");
    }

    // ── the shared glob grammar ──────────────────────────────────────────────

    #[test]
    fn double_star_spans_zero_or_more_segments() {
        let scope = vec!["tasks/**".to_string()];
        assert!(path_in_scope(&scope, "tasks/a.md"));
        assert!(path_in_scope(&scope, "tasks/deep/nested/a.md"));
        assert!(!path_in_scope(&scope, "notes/a.md"));
    }

    #[test]
    fn single_star_stays_inside_one_segment() {
        let scope = vec!["tasks/*.md".to_string()];
        assert!(path_in_scope(&scope, "tasks/a.md"));
        assert!(
            !path_in_scope(&scope, "tasks/deep/a.md"),
            "`*` never crosses a `/`"
        );
    }

    #[test]
    fn a_path_matching_any_glob_is_in_scope() {
        let scope = vec!["tasks/**".to_string(), "notes/*.md".to_string()];
        assert!(path_in_scope(&scope, "notes/plan.md"));
        assert!(path_in_scope(&scope, "tasks/x/y.md"));
        assert!(!path_in_scope(&scope, "other/plan.md"));
    }
}
