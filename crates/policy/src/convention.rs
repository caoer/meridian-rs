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
//! The folder NAME is the subject slug, and it is an identifier, not free text: it
//! is stamped verbatim into the attested INDEX and into the reserved journal, so
//! [`load_convention`] admits only the convention-slug charset `[a-z][a-z0-9-]*` —
//! a strict subset of the one identifier charset (`[A-Za-z0-9-]`, contract §2.4 /
//! decision 011). See `validate_slug` § THE INTAKE CHARSET.
//!
//! # What loads (rulings § v1 ships CHECK only, § capability grammar)
//! CHECK (the law leg) and HOOK (the emit leg) load. A convention that declares a
//! FIX / VIEW file is refused with a named deferral
//! ([`LoadError::CapabilityDeferred`]) — those are named power ceilings deferred
//! until a real subject needs them, never silently ignored. The CHECK power ceiling
//! is enforced by the evaluator ([`crate::check_eval`]); HOOK's is enforced at LOAD
//! ([`crate::hook`]).
//!
//! HOOK's own deferral said *"until a real subject needs them"*, and a status
//! subscription is that subject. **A convention may carry HOOK without CHECK** — a
//! reaction is not a law, and HOOK exists as a distinct name precisely so the two
//! are never confused. A folder carrying neither is still refused: it declares no
//! capability at all.
//!
//! # Scope (rulings § scoping — the Claude-rules pattern)
//! `CHECK.md` frontmatter declares `paths:` — a flat glob list (obsidian-legal),
//! the convention's default scope. [`Convention::matches_path`] answers whether a
//! document path is in scope; the attested INDEX row may narrow it further at
//! arming (U1.4), never widen it.

use crate::change::Change;
use crate::check_eval::{self, CheckError, CheckLimits, CheckTelemetry};
use crate::hook::Hook;

/// The four capability files a convention folder may carry. Each earns a file iff
/// it needs a distinct power ceiling (rulings § capability grammar); this slice loads
/// [`Capability::Check`] and [`Capability::Hook`] and defers FIX/VIEW.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    /// `CHECK.md` — reads the change + pinned facts, produces findings / refusals.
    Check,
    /// `FIX.md` — mutates the change under caps (deferred).
    Fix,
    /// `HOOK.md` — reacts outward to the landed change under declared effect caps.
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

    /// The capabilities still deferred, in declaration order. HOOK left this list
    /// when its stated deferral condition was met (a status subscription is the real
    /// subject); FIX's mutate power and VIEW's read face have no such subject yet.
    const DEFERRED: [Capability; 2] = [Capability::Fix, Capability::View];
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
    /// The slug is empty, contains a path separator or `..`, begins with `.` (a
    /// dot-dir — outside the hash domain, cannot carry attested law), or falls
    /// outside the convention-slug charset `[a-z][a-z0-9-]*` (R45/R46 — the slug is
    /// stamped verbatim into the attested INDEX and the reserved journal).
    SlugInvalid { slug: String, reason: String },
    /// The folder declares NO capability the loader can load — no readable
    /// `CHECK.md` and no `HOOK.md`. A convention with neither a law nor a reaction
    /// is not a convention.
    CheckMissing { detail: String },
    /// The convention declares a deferred capability (FIX / VIEW). The refusal names
    /// the capability and the deferral.
    CapabilityDeferred { capability: Capability },
    /// `CHECK.md` is malformed: no frontmatter, no `paths:` scope, an empty scope,
    /// or no fenced `def check_change` predicate block.
    Malformed { reason: String },
    /// The CHECK predicate failed the load gate (over-long, over-nested, or
    /// unparseable starlark) under the full limits.
    CheckInvalid { source: CheckError },
    /// `HOOK.md` is malformed — the reason names the offending field.
    HookMalformed { reason: String },
    /// `HOOK.md` declares a real effect cap that slice 1 does not carry. A named
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
            LoadError::SlugInvalid { slug, reason } => {
                write!(f, "convention slug '{slug}' is invalid: {reason}")
            }
            LoadError::CheckMissing { detail } => write!(
                f,
                "convention declares no loadable capability — no readable CHECK.md \
                 ({detail}) and no HOOK.md. Add CHECK.md for a law, or HOOK.md for a \
                 reaction"
            ),
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
            LoadError::HookMalformed { reason } => write!(f, "HOOK.md is malformed: {reason}"),
            LoadError::HookCapDeferred { cap } => write!(
                f,
                "HOOK.md declares the cap `{cap}`, which slice 1 does not carry — slice 1 \
                 admits `proto.send` only. The cap is a named power ceiling deferred until a \
                 real subject needs it, never silently ignored"
            ),
            LoadError::HookPredicateInvalid { reason } => {
                write!(f, "HOOK.md predicate failed the load gate: {reason}")
            }
            LoadError::HookCeiling { reason } => {
                write!(
                    f,
                    "HOOK.md predicate is outside its capability ceiling: {reason}"
                )
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
    /// The CHECK predicate, when the convention carries a law. `None` for a
    /// HOOK-only convention — a reaction is not a law.
    check_source: Option<String>,
    /// The HOOK declaration, when the convention carries a reaction.
    hook: Option<Hook>,
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
        path_in_scope(&self.scope, path)
    }

    /// The HOOK declaration, when the convention carries one.
    #[must_use]
    pub fn hook(&self) -> Option<&Hook> {
        self.hook.as_ref()
    }

    /// Run the CHECK predicate over one [`Change`] under the convention's full
    /// limits, returning the [`CheckOutcome`] (the refusals it emitted).
    ///
    /// A convention with no CHECK **passes everything**: it emits no refusals because
    /// it declares no law. This is the HOOK-without-CHECK case, and it is silence by
    /// construction rather than by accident — a reaction never vetoes.
    ///
    /// The caller is responsible for scoping — `check_change` runs on the change it
    /// is handed. Pair with [`Convention::matches_path`] on the change's document
    /// path to skip out-of-scope documents.
    ///
    /// # Errors
    /// [`CheckError`] — a budget/parse/runtime fault or a missing `check_change`.
    pub fn check_change(&self, change: &Change) -> Result<CheckOutcome, CheckError> {
        let Some(source) = &self.check_source else {
            return Ok(CheckOutcome {
                refusals: Vec::new(),
            });
        };
        let refusals = check_eval::run_check_change(source, change, self.limits)?;
        Ok(CheckOutcome { refusals })
    }

    /// Run the CHECK over one [`Change`] and return the [`CheckTelemetry`] — the
    /// refusals AND the exact fuel + heap the evaluation spent. Same metered core
    /// as [`Convention::check_change`]; the `test --corpus` tier (U1.5) reads the
    /// telemetry for its fuel + heap p50/p99 budgets.
    ///
    /// # Errors
    /// [`CheckError`] — a budget/parse/runtime fault, a missing `check_change`, or
    /// [`CheckError::MissingEntry`] when the convention declares no CHECK at all
    /// (the metered path reports the absence rather than inventing a zero reading).
    pub fn check_change_metered(&self, change: &Change) -> Result<CheckTelemetry, CheckError> {
        let source = self.check_source.as_ref().ok_or(CheckError::MissingEntry)?;
        check_eval::run_check_change_metered(source, change, self.limits)
    }

    /// The CHECK source, when the convention carries a law (for tests and later units
    /// that re-run it).
    #[must_use]
    pub fn check_source(&self) -> Option<&str> {
        self.check_source.as_deref()
    }
}

/// A convention loaded only for the `test --corpus` counterfactual proof.
///
/// This wrapper is deliberately not [`Convention`]. Its widened HOOK cannot enter
/// ordinary policy evaluation, arming, or the production loader by type. The public
/// surface exposes only the facts the proof needs; the widened [`Hook`] stays behind
/// the policy-owned corpus evaluator.
///
/// ```compile_fail
/// use policy::{Convention, CounterfactualConvention};
///
/// fn cannot_enter_ordinary_policy(proof: CounterfactualConvention) {
///     let _ordinary: Convention = proof;
/// }
/// ```
#[derive(Debug, Clone)]
pub struct CounterfactualConvention {
    inner: Convention,
}

impl CounterfactualConvention {
    /// The convention's subject slug.
    #[must_use]
    pub fn slug(&self) -> &str {
        self.inner.slug()
    }

    /// The convention's declared default scope.
    #[must_use]
    pub fn scope(&self) -> &[String] {
        self.inner.scope()
    }

    /// Whether the CHECK leg matches `path`.
    #[must_use]
    pub fn matches_path(&self, path: &str) -> bool {
        self.inner.matches_path(path)
    }

    /// Whether the convention carries a CHECK leg.
    #[must_use]
    pub fn has_check(&self) -> bool {
        self.inner.check_source().is_some()
    }

    /// Run the CHECK leg through the ordinary metered evaluator.
    ///
    /// # Errors
    /// The same [`CheckError`] surface as [`Convention::check_change_metered`].
    pub fn check_change_metered(&self, change: &Change) -> Result<CheckTelemetry, CheckError> {
        self.inner.check_change_metered(change)
    }

    /// Whether the convention carries a counterfactual HOOK leg.
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

/// Load the convention `conventions/<slug>/` through the injected `files` accessor
/// under the given full limits.
///
/// Pipeline: validate the slug (never a dot-dir) → refuse any declared deferred
/// capability (FIX / VIEW) → require at least one of `CHECK.md` / `HOOK.md` → parse
/// each present capability and parse-gate its predicate under the full limits →
/// admit.
///
/// **Fail-closed, never partial:** a convention that declares two capabilities and
/// gets one of them wrong loads NEITHER. The whole folder is the unit.
///
/// The convention's scope is `CHECK.md`'s `paths:` when it carries a law, and
/// `HOOK.md`'s when it carries only a reaction. A HOOK always answers scope through
/// its own [`Hook::matches_path`], so a convention carrying both can scope its law
/// and its reaction differently.
///
/// # Errors
/// [`LoadError`] — see its variants.
pub fn load_convention(
    slug: &str,
    files: &dyn ConventionFiles,
    limits: CheckLimits,
) -> Result<Convention, LoadError> {
    load_convention_with_hook_loader(slug, files, limits, crate::hook::load_hook)
}

/// Load a convention for the `test --corpus` pre-arming proof.
///
/// This differs from [`load_convention`] only for HOOK capability admission:
/// counterfactual `md.*` descriptors may load so the tier can prove whether their
/// trigger graph is quiescent. The widened declaration is returned as the opaque
/// [`CounterfactualConvention`], so it cannot enter [`crate::evaluate_hooks`], an
/// armed set, or any API accepting an ordinary [`Convention`]. The production loader
/// remains pinned to [`crate::SLICE1_CAPS`].
///
/// # Errors
/// The same [`LoadError`] surface as [`load_convention`].
pub fn load_convention_for_corpus(
    slug: &str,
    files: &dyn ConventionFiles,
    limits: CheckLimits,
) -> Result<CounterfactualConvention, LoadError> {
    load_convention_with_hook_loader(slug, files, limits, crate::hook::load_hook_for_corpus)
        .map(|inner| CounterfactualConvention { inner })
}

fn load_convention_with_hook_loader(
    slug: &str,
    files: &dyn ConventionFiles,
    limits: CheckLimits,
    hook_loader: fn(&str, CheckLimits) -> Result<Hook, LoadError>,
) -> Result<Convention, LoadError> {
    validate_slug(slug)?;

    // 1. Capability ceiling — a declared FIX / VIEW is a named deferral, never a
    //    silent drop. Checked BEFORE anything is parsed: a folder reaching for a
    //    deferred power is refused for that reach, not for some later detail of a
    //    file it also happens to carry.
    for capability in Capability::DEFERRED {
        if files.exists(capability.filename()) {
            return Err(LoadError::CapabilityDeferred { capability });
        }
    }

    // 2. At least one loadable capability, and each present one fully parsed. CHECK
    //    is the law leg, HOOK the emit leg; either alone is a convention, neither is
    //    not. Matching on the pair keeps "one capability is present" a fact of the
    //    control flow rather than an invariant a later arm has to assume.
    let load_hook = |md: &str| hook_loader(md, limits);
    let (scope, check_source, hook) = match (
        files.read(Capability::Check.filename()),
        files.read(Capability::Hook.filename()),
    ) {
        (Err(check_err), Err(_)) => {
            return Err(LoadError::CheckMissing {
                detail: check_err.to_string(),
            });
        }
        (Ok(check_md), hook_md) => {
            let (scope, source) = parse_check(&check_md, limits)?;
            let hook = match hook_md {
                Ok(hook_md) => Some(load_hook(&hook_md)?),
                Err(_) => None,
            };
            (scope, Some(source), hook)
        }
        // HOOK-only: the reaction's scope IS the convention's scope.
        (Err(_), Ok(hook_md)) => {
            let hook = load_hook(&hook_md)?;
            (hook.scope().to_vec(), None, Some(hook))
        }
    };

    Ok(Convention {
        slug: slug.to_string(),
        scope,
        check_source,
        hook,
        limits,
    })
}

/// Parse `CHECK.md`: the `paths:` scope frontmatter and the fenced predicate,
/// parse-gated under the FULL limits (source-size + nesting + parse) so authoring
/// faults surface here, once, at load.
fn parse_check(check_md: &str, limits: CheckLimits) -> Result<(Vec<String>, String), LoadError> {
    let scope = parse_scope(check_md)?;
    let source =
        crate::pack::extract_fenced_starlark(check_md).ok_or_else(|| LoadError::Malformed {
            reason: "no fenced ```starlark block defining `def check_change(change)`".to_string(),
        })?;
    check_eval::validate_check_source(&source, limits)
        .map_err(|source| LoadError::CheckInvalid { source })?;
    Ok((scope, source))
}

/// Validate a convention slug: non-empty, no path separator, no `..`, never a
/// dot-dir (rulings § scoping — dot-dirs sit outside the hash domain), and inside
/// the one identifier charset (R45 — see THE INTAKE CHARSET below).
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
    // THE INTAKE CHARSET (R45/R46). The four guards above are path-traversal only:
    // a folder named `[[guide#^goal@green.b3af12cd|G]]` passes all of them. That
    // name is not decoration — it is stamped VERBATIM into two stored artifacts,
    // and a slug that can render as markdown forges the record of its own
    // enforcement:
    //
    // - the attested INDEX, twice per row (`index::IndexEntry::render` — the bold
    //   label and the wikilink to its `CHECK.md`), written by `mrd realise --truth
    //   file`; a claim token there sits in the ENFORCEMENT SUBSTRATE;
    // - the reserved journal's `forced_rule=` token (`gate::GateFinding::slug` →
    //   `wire-serve`'s `ForcedSkip` → `force_journal_write`); a claim token there
    //   forges the chain-continuity detector's own input.
    //
    // The class is STRUCTURAL, not a taste: the slug lands INSIDE a wikilink and
    // INSIDE a journal row, so `[`, `]`, `|`, `#`, `^` and `@` are unsafe there
    // whatever the escaping. Guarding the two RENDERERS would leave the third one
    // anyone adds later open — and this milestone met one-owner-many-doors seven
    // times, the seventh being a door nobody had enumerated. So the guard sits at
    // INTAKE, where the next renderer inherits it. Every INDEX row is an
    // `IndexEntry`, whose construction is sealed to `index::sweep` (which calls this
    // loader) and `index::arm` (which consumes a swept row); every journalled
    // `forced_rule=` is an `ArmedConvention::slug`, which `gate::resolve_one` loads
    // through this same function before the gate can name it. Neither renderer has a
    // path that skips this refusal, and the refusal makes the hostile bytes
    // unrepresentable rather than removable.
    if let Some((bad, rule)) = charset_fault(slug) {
        return Err(invalid(&format!(
            "the character {bad:?} (U+{code:04X}) is outside the convention-slug charset \
             [a-z][a-z0-9-]* — {rule}. The slug is stamped verbatim into the attested INDEX \
             (as the row label, and inside the wikilink to its CHECK.md) and into the reserved \
             journal's `forced_rule=` token, so a name that can render as markdown forges those \
             records. Rename the folder to `conventions/{suggestion}/`",
            code = bad as u32,
            suggestion = corrected_slug(slug),
        )));
    }
    Ok(())
}

/// The first character that puts `slug` outside the convention-slug charset
/// `[a-z][a-z0-9-]*`, paired with the rule it broke. `None` ⇔ the slug is a legal
/// convention identifier. (A non-empty `slug` is the caller's precondition.)
///
/// The charset is a STRICT SUBSET of the one identifier charset (`[A-Za-z0-9-]`,
/// contract §2.4 / decision 011) — never wider, so a convention slug is always a
/// legal identifier too. It is narrower on two axes, each earned: the slug is also
/// a DIRECTORY NAME on a case-insensitive filesystem, where `Foo` and `foo` are one
/// folder but two INDEX rows; and a leading digit or dash reads as list or numeric
/// markup in the row it renders into.
fn charset_fault(slug: &str) -> Option<(char, &'static str)> {
    let mut chars = slug.chars();
    let first = chars.next()?;
    if !first.is_ascii_lowercase() {
        return Some((first, "a slug STARTS with a lowercase letter"));
    }
    chars
        .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-'))
        .map(|c| (c, "a slug carries only lowercase letters, digits and `-`"))
}

/// The slug the author most likely meant: lowercased, every run of out-of-charset
/// bytes collapsed to one `-`, trimmed, and led by a letter. The refusal prints
/// this so someone meeting the guard in a year needs no archaeology (R24 — a
/// refusal teaches the legal path, it does not merely deny one). Total by
/// construction: an input with no usable byte falls back to a nameable default
/// rather than a suggestion the guard would refuse a second time.
fn corrected_slug(slug: &str) -> String {
    let mut out = String::with_capacity(slug.len());
    for c in slug.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let led = out.trim_start_matches(|c: char| !c.is_ascii_lowercase());
    let trimmed = led.trim_end_matches('-');
    if trimmed.is_empty() {
        "my-convention".to_string()
    } else {
        trimmed.to_string()
    }
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

/// Whether `path` matches any glob in `scope` — the one scope answer, shared by
/// [`Convention::matches_path`] and [`Hook::matches_path`] so a capability can never
/// drift into a second glob grammar.
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

    const VALID_HOOK: &str = "\
---
kind: hook
severity: info
paths: [\"tasks/*.md\"]
caps:  [proto.send]
budget: { steps: 10000, mem: 4194304 }
how:
  route:    { info: channel-review }
  wake_policy: never-cold
---

# task-status-notify

```starlark
def on_change(event):
    send(to = [\"reviewer\"], message = \"task → review\")
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

    /// The deferral NARROWED rather than disappeared: VIEW (and FIX, above) still
    /// refuse, and HOOK — whose deferral said *"until a real subject needs them"* —
    /// no longer does, because a status subscription is that subject.
    #[test]
    fn view_still_defers_and_hook_no_longer_does() {
        let files = MemFiles::new()
            .with("CHECK.md", VALID_CHECK)
            .with("VIEW.md", "# deferred\n");
        assert_eq!(
            load_convention("s", &files, CheckLimits::default()).unwrap_err(),
            LoadError::CapabilityDeferred {
                capability: Capability::View
            }
        );

        let files = MemFiles::new()
            .with("CHECK.md", VALID_CHECK)
            .with("HOOK.md", VALID_HOOK);
        let conv = load_convention("s", &files, CheckLimits::default())
            .expect("HOOK is no longer a deferred capability");
        assert!(conv.hook().is_some(), "the HOOK loaded alongside the CHECK");
        assert!(conv.check_source().is_some(), "the CHECK is still there");
    }

    /// A convention may carry HOOK WITHOUT CHECK — a reaction is not a law. With no
    /// law it refuses nothing, and the reaction's scope is the convention's scope.
    #[test]
    fn hook_without_check_loads_and_refuses_nothing() {
        let files = MemFiles::new().with("HOOK.md", VALID_HOOK);
        let conv = load_convention("task-status-notify", &files, CheckLimits::default())
            .expect("a reaction is not a law — HOOK alone is a convention");
        println!("POPULATION scope = {:?}", conv.scope());
        assert!(conv.hook().is_some());
        assert_eq!(conv.check_source(), None, "no law declared");
        assert_eq!(conv.scope(), &["tasks/*.md".to_string()]);
        assert!(conv.matches_path("tasks/x.md"));
    }

    /// A folder declaring NEITHER capability is refused — it is not a convention.
    #[test]
    fn neither_capability_is_refused() {
        let files = MemFiles::new().with("README.md", "# not a capability\n");
        let err = load_convention("s", &files, CheckLimits::default()).unwrap_err();
        println!("POPULATION no-capability -> {err}");
        assert!(matches!(err, LoadError::CheckMissing { .. }), "{err:?}");
        assert!(
            err.to_string().contains("HOOK.md"),
            "the refusal teaches both legal paths: {err}"
        );
    }

    /// FAIL-CLOSED, never partial: a folder whose CHECK is perfect and whose HOOK is
    /// malformed loads ZERO capabilities. The folder is the unit.
    ///
    /// Both directions, so the property is shown rather than assumed for one arm.
    #[test]
    fn one_bad_capability_loads_zero_capabilities() {
        let bad_hook = VALID_HOOK.replace("severity: info\n", "");
        let files = MemFiles::new()
            .with("CHECK.md", VALID_CHECK)
            .with("HOOK.md", &bad_hook);
        let err = load_convention("s", &files, CheckLimits::default())
            .expect_err("a bad HOOK sinks the whole folder, good CHECK or not");
        println!("POPULATION good-check-bad-hook -> {err}");
        assert!(matches!(err, LoadError::HookMalformed { .. }), "{err:?}");

        let bad_check =
            "---\ntitle: no scope\n---\n\n```starlark\ndef check_change(change):\n    pass\n```\n";
        let files = MemFiles::new()
            .with("CHECK.md", bad_check)
            .with("HOOK.md", VALID_HOOK);
        let err = load_convention("s", &files, CheckLimits::default())
            .expect_err("a bad CHECK sinks the whole folder, good HOOK or not");
        println!("POPULATION bad-check-good-hook -> {err}");
        assert!(matches!(err, LoadError::Malformed { .. }), "{err:?}");

        // The control: each file is individually fine, so the two refusals above are
        // caused by the single mutation in each arm.
        let files = MemFiles::new()
            .with("CHECK.md", VALID_CHECK)
            .with("HOOK.md", VALID_HOOK);
        assert!(load_convention("s", &files, CheckLimits::default()).is_ok());
    }

    /// A convention carrying BOTH may scope its law and its reaction differently —
    /// the CHECK answers `Convention::matches_path`, the HOOK answers its own.
    #[test]
    fn check_and_hook_carry_independent_scopes() {
        let files = MemFiles::new()
            .with("CHECK.md", VALID_CHECK) // paths: tasks/**
            .with("HOOK.md", VALID_HOOK); // paths: tasks/*.md
        let conv = load_convention("s", &files, CheckLimits::default()).unwrap();
        let hook = conv.hook().unwrap();
        println!(
            "POPULATION check scope = {:?} | hook scope = {:?}",
            conv.scope(),
            hook.scope()
        );
        assert!(
            conv.matches_path("tasks/deep/nested.md"),
            "CHECK spans depth"
        );
        assert!(
            !hook.matches_path("tasks/deep/nested.md"),
            "the HOOK declared a shallower scope and keeps it"
        );
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

    /// A folder carrying ONLY a deferred capability is refused for the DEFERRAL, not
    /// for the missing CHECK.
    ///
    /// This test asserted `CheckMissing` before HOOK was un-deferred. The variant
    /// changed because the capability ceiling now runs before any file is read, and
    /// that ordering is deliberate: once CHECK is optional, "no readable CHECK.md and
    /// no HOOK.md" would be a true but useless thing to tell someone whose folder
    /// plainly declares VIEW. The refusal should name the reach the author actually
    /// made. Still loud, still fail-closed — a more specific reason.
    /// `neither_capability_is_refused` covers the original intent (a folder that
    /// declares nothing at all).
    #[test]
    fn a_folder_with_only_a_deferred_capability_names_the_deferral() {
        let files = MemFiles::new().with("VIEW.md", "# no check\n");
        let err = load_convention("s", &files, CheckLimits::default()).unwrap_err();
        println!("POPULATION view-only -> {err}");
        assert_eq!(
            err,
            LoadError::CapabilityDeferred {
                capability: Capability::View
            }
        );
    }

    #[test]
    fn dot_dir_slug_is_refused() {
        let files = MemFiles::new().with("CHECK.md", VALID_CHECK);
        assert!(matches!(
            load_convention(".hidden", &files, CheckLimits::default()),
            Err(LoadError::SlugInvalid { .. })
        ));
    }

    /// THE INTAKE CHARSET (R45/R46). A folder name that can render as markdown is
    /// refused at load, so it never becomes an `IndexEntry` and never reaches the
    /// attested INDEX or the reserved journal.
    ///
    /// **The refusal must TEACH (R24, a hard requirement):** it names the charset,
    /// SHOWS the offending character, names both artifacts the slug is stamped into,
    /// and prints a CORRECTED FORM of the name the author actually typed — so
    /// meeting this guard in a year costs zero archaeology.
    #[test]
    fn out_of_charset_slug_is_refused_with_the_teaching() {
        let files = MemFiles::new().with("CHECK.md", VALID_CHECK);
        let err = load_convention(
            "[[guide#^goal@green.b3af12cd|G]]",
            &files,
            CheckLimits::default(),
        )
        .expect_err("a slug that can render as markdown is refused at intake");
        let text = err.to_string();
        for (named, why) in [
            ("[a-z][a-z0-9-]*", "the charset"),
            ("'['", "the offending character, shown"),
            ("U+005B", "the offending character's codepoint"),
            ("attested INDEX", "the first artifact it would forge"),
            ("reserved journal", "the second artifact it would forge"),
            (
                "conventions/guide-goal-green-b3af12cd-g/",
                "a corrected form of the slug the author typed",
            ),
        ] {
            assert!(
                text.contains(named),
                "the refusal must name {why} ({named}): {text}"
            );
        }
    }

    /// The correction is TOTAL: it never suggests a name the guard would refuse a
    /// second time — including for an input with no usable byte at all.
    #[test]
    fn the_suggested_correction_always_passes_the_guard() {
        let files = MemFiles::new().with("CHECK.md", VALID_CHECK);
        for typed in [
            "[[guide#^goal@green.b3af12cd|G]]",
            "Reviewer Not Owner",
            "2fa-policy",
            "---",
            "^^^",
            "CLAIM_CAS",
            "a b\tc\nd",
        ] {
            let suggestion = corrected_slug(typed);
            assert!(
                load_convention(&suggestion, &files, CheckLimits::default()).is_ok(),
                "the correction offered for {typed:?} was {suggestion:?}, which the guard \
                 itself refuses"
            );
        }
    }

    /// The NARROWING, stated rather than discovered: three shapes that loaded
    /// before this guard and do not now. None forges a claim; each is refused
    /// because a slug is an ADDRESS — uppercase collides with itself on a
    /// case-insensitive filesystem (one folder, two INDEX rows), and a leading digit
    /// or dash reads as markup in the row it renders into.
    #[test]
    fn the_narrowing_is_named_not_discovered() {
        let files = MemFiles::new().with("CHECK.md", VALID_CHECK);
        for (slug, why) in [
            ("Reviewer-Not-Owner", "uppercase — one folder, two rows"),
            ("2fa", "a leading digit reads as numeric markup"),
            ("-draft", "a leading dash reads as list markup"),
            (
                "claim_cas",
                "`_` — ruling 011 killed the underscore superset",
            ),
        ] {
            assert!(
                matches!(
                    load_convention(slug, &files, CheckLimits::default()),
                    Err(LoadError::SlugInvalid { .. })
                ),
                "{slug} is newly invalid ({why}) and must refuse"
            );
        }
    }

    /// Every forging SHAPE the slug could carry into either artifact — not just the
    /// `@fp` claim token that raised the finding. A charset closes the class; a
    /// per-shape strip would close one member of it (fix9's measured argument, and
    /// the reason R45 chose intake over per-renderer escaping).
    #[test]
    fn every_forging_slug_shape_is_refused_at_intake() {
        let files = MemFiles::new().with("CHECK.md", VALID_CHECK);
        for (slug, shape) in [
            ("[[guide#^goal@green.b3af12cd|G]]", "an `@fp` claim link"),
            ("![[guide#^goal@green.b3af12cd]]", "a claim EMBED"),
            ("a · block · `deadbeefdeadbeef", "the INDEX field separator"),
            ("a\nb", "a whole forged INDEX row"),
            ("a`b", "the pinned-rev code span"),
            ("a**b", "the row label's bold fence"),
            ("a b", "a second journal token"),
            ("a^r-000099", "a second journal block anchor"),
        ] {
            assert!(
                matches!(
                    load_convention(slug, &files, CheckLimits::default()),
                    Err(LoadError::SlugInvalid { .. })
                ),
                "slug {slug:?} forges {shape} — it must not load"
            );
        }
    }

    /// **The R45/R46 census, frozen as a test.** Every convention slug in use across
    /// both repos and the vault when the guard landed — directory names owning any
    /// capability file (`CHECK`/`FIX`/`HOOK`/`VIEW.md`), the strongest of the three
    /// markers the census was run with (a narrower marker missed `armed-index` and
    /// `no-draws-from`). The guard refuses NONE of them.
    ///
    /// The census is corroboration, not the argument: these are all fixtures, so
    /// "no live slug refused" would be weak alone. The ruling rests on the
    /// STRUCTURAL claim in `validate_slug`. What this test buys is the other
    /// direction — if a future charset change would break a name already in use, it
    /// fails HERE, and the migration is owed before the narrowing rather than after.
    #[test]
    fn every_slug_in_use_when_the_guard_landed_still_loads() {
        let files = MemFiles::new().with("CHECK.md", VALID_CHECK);
        for slug in [
            "armed-index",
            "candidate",
            "claim-cas",
            "close-verdict",
            "decoy-close",
            "meta-convention",
            "no-draws-from",
            "reviewer-and-priority",
            "reviewer-not-owner",
            "verdict-reviewer-bind",
        ] {
            assert!(
                load_convention(slug, &files, CheckLimits::default()).is_ok(),
                "the guard must refuse no convention that was in use: {slug}"
            );
        }
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
