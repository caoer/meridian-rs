//! Capability resolution — deny-by-default (verdict ruling 3, decision #15).
//! Undeclared block is read-only. Precedence: explicit `task.<name>.caps` >
//! root `MERIDIAN.md` `run.caps.<pattern>` > none. Conventions NARROW only
//! (intersection survives; narrowed remainder reported). Builtin `check-*` /
//! `verify-*` ceiling is empty and non-overridable; those names refuse bash
//! at load. `fix-*` does not — fix blocks declare writes.
//!
//! # Caps do not apply to bash (`docs/laws.md` § Amendment)
//! The ladder above is **starlark's**. Bash resolves [`Authority::Unsandboxed`]
//! — no cap set, no source; `task.<name>.caps` is not read. [`Authority`]
//! makes that structural: bash never holds the capability-carrying type.
//! Gate: `crates/mrd/tests/law_no_caps_on_bash.rs`.
//!
//! [`resolve_authority`] is the only language-aware entry (bash short-circuit).
//! [`resolve_caps`] takes no language.
//!
//! # Convention plane (marker-retirement ruling, 2026-07-26)
//! Table lives in `<root>/MERIDIAN.md` with `type: meridian-root`. Retired
//! marker files are not read and have no fallback. Grammar is flat dotted
//! keys (model's FM scanner skips indented lines):
//!
//! ```yaml
//! run.caps.fix-*: md.set_field, md.append_section
//! run.caps.fix-note: md.set_field:status
//! ```
//!
//! [`ConventionSource`] reports which root situation answered. Present-but-
//! invalid declaration REFUSES ([`CapsError::Declaration`]) — silent empty
//! table would delete a ceiling on typo. Target-scoped caps are narrower
//! than untargeted forms.
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use model::Document;

use crate::address::frontmatter;
use crate::fence::TaskLanguage;

/// The frontmatter key prefix carrying one convention entry. The pattern is
/// whatever follows it, taken by stripping this FIXED prefix rather than by
/// splitting on dots — so a pattern may itself contain `.`, which
/// [`Conventions::new`]'s charset allows.
pub const CAPS_KEY_PREFIX: &str = "run.caps.";

/// The reserved filename of the root's self-declaration. Re-exported from its
/// ONE owner rather than re-spelled here.
pub use config::mount::{DECLARATION_FILENAME, DECLARATION_TYPE};

/// How every surface spells a bash block's effect declaration: there is none.
/// One owner, so `--list`, `--dry` and the run report cannot drift into three
/// wordings of the same fact.
pub const UNDECLARED_EFFECTS: &str = "undeclared";

/// Name patterns that are read-only BY CONVENTION, builtin and non-overridable:
/// their ceiling is empty, and a bash fence under them refuses at load. `fix-*`
/// is deliberately absent (ruling 3: check-*/verify-* only).
pub const READ_ONLY_PATTERNS: [&str; 2] = ["check-*", "verify-*"];

/// One parsed capability: a namespaced kind (`md.set_field`) with an optional
/// scope target (`md.set_field:status`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cap {
    /// The namespaced kind string (`md.set_field`).
    pub kind: String,
    /// The optional scope target (`status` in `md.set_field:status`).
    pub target: Option<String>,
}

impl Cap {
    /// Parse one cap string: `ns.name` or `ns.name:target`.
    ///
    /// # Errors
    /// [`CapsError::BadCap`] when the string is not a namespaced cap.
    pub fn parse(raw: &str) -> Result<Self, CapsError> {
        let bad = || CapsError::BadCap {
            raw: raw.to_owned(),
        };
        let (kind, target) = match raw.split_once(':') {
            Some((k, t)) => (k, Some(t)),
            None => (raw, None),
        };
        let (ns, name) = kind.split_once('.').ok_or_else(bad)?;
        let word_ok = |s: &str| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        };
        if !word_ok(ns) || !word_ok(name) {
            return Err(bad());
        }
        if let Some(t) = target {
            let target_ok = !t.is_empty()
                && t.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
            if !target_ok {
                return Err(bad());
            }
        }
        Ok(Self {
            kind: kind.to_owned(),
            target: target.map(str::to_owned),
        })
    }

    /// The canonical string form.
    #[must_use]
    pub fn as_string(&self) -> String {
        match &self.target {
            Some(t) => format!("{}:{t}", self.kind),
            None => self.kind.clone(),
        }
    }

    /// Does this cap admit an effect of `kind` against `target`? An untargeted
    /// cap admits every target of its kind; a targeted cap admits only its own.
    #[must_use]
    pub fn admits(&self, kind: &str, target: Option<&str>) -> bool {
        self.kind == kind
            && match &self.target {
                None => true,
                Some(t) => target == Some(t.as_str()),
            }
    }

    /// The meet (narrower) of two caps of the same kind, if comparable:
    /// untargeted ∩ targeted = the targeted one; equal targets = that target;
    /// different targets = incomparable (`None`).
    fn meet(&self, other: &Cap) -> Option<Cap> {
        if self.kind != other.kind {
            return None;
        }
        match (&self.target, &other.target) {
            (None, t) | (t, None) => Some(Cap {
                kind: self.kind.clone(),
                target: t.clone(),
            }),
            (Some(a), Some(b)) if a == b => Some(self.clone()),
            _ => None,
        }
    }
}

/// An ordered set of capabilities.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapSet(pub BTreeSet<Cap>);

impl CapSet {
    /// The empty (read-only) set.
    #[must_use]
    pub fn none() -> Self {
        Self(BTreeSet::new())
    }

    /// Parse a comma/whitespace-separated cap list.
    ///
    /// # Errors
    /// [`CapsError::BadCap`] on the first malformed cap.
    pub fn parse(raw: &str) -> Result<Self, CapsError> {
        let mut set = BTreeSet::new();
        for word in raw
            .trim()
            .trim_matches(['"', '\''])
            .split([',', ' ', '\t'])
            .filter(|s| !s.is_empty())
        {
            set.insert(Cap::parse(word)?);
        }
        Ok(Self(set))
    }

    /// Does any cap in the set admit an effect of `kind` against `target`?
    #[must_use]
    pub fn admits(&self, kind: &str, target: Option<&str>) -> bool {
        self.0.iter().any(|c| c.admits(kind, target))
    }

    /// Narrow this set under a `ceiling` (conventions narrow only, never
    /// widen): each cap survives as its meet with the ceiling; a cap with no
    /// comparable ceiling cap is dropped. Returns `(effective, narrowed)` —
    /// `narrowed` lists the caps that did not survive INTACT (dropped or
    /// tightened), so narrowing is always visible, never silent.
    #[must_use]
    pub fn narrow(&self, ceiling: &CapSet) -> (CapSet, Vec<Cap>) {
        let mut effective = BTreeSet::new();
        let mut narrowed = Vec::new();
        for cap in &self.0 {
            let meets: BTreeSet<Cap> = ceiling.0.iter().filter_map(|c| cap.meet(c)).collect();
            if meets.iter().any(|m| m == cap) {
                effective.insert(cap.clone());
            } else {
                narrowed.push(cap.clone());
                effective.extend(meets);
            }
        }
        (CapSet(effective), narrowed)
    }
}

/// The `run.caps.<pattern>` name-convention table from the root's declaration:
/// pattern → grant/ceiling set, resolution-ordered (longest pattern first, then
/// lexicographic — deterministic regardless of file order).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Conventions(Vec<(String, CapSet)>);

impl Conventions {
    /// An empty table — deny-by-default only.
    #[must_use]
    pub fn none() -> Self {
        Self(Vec::new())
    }

    /// Build from `(pattern, caps)` pairs, validating each pattern (a literal
    /// name or a trailing-`*` prefix glob).
    ///
    /// # Errors
    /// [`CapsError::BadPattern`] on a malformed pattern.
    pub fn new(mut entries: Vec<(String, CapSet)>) -> Result<Self, CapsError> {
        for (pattern, _) in &entries {
            let body = pattern.strip_suffix('*').unwrap_or(pattern);
            let ok = !pattern.is_empty()
                && !body.contains('*')
                && body
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
            if !ok {
                return Err(CapsError::BadPattern {
                    pattern: pattern.clone(),
                });
            }
        }
        entries.sort_by(|(a, _), (b, _)| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        Ok(Self(entries))
    }

    /// The most specific entry matching `name` (longest pattern wins).
    #[must_use]
    pub fn matching(&self, name: &str) -> Option<(&str, &CapSet)> {
        self.0
            .iter()
            .find(|(p, _)| pattern_matches(p, name))
            .map(|(p, s)| (p.as_str(), s))
    }
}

/// `check-*` matches `check-links`; a pattern without `*` matches exactly.
fn pattern_matches(pattern: &str, name: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == pattern,
    }
}

/// Where a block's capability grant came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapSource {
    /// Explicit `task.<name>.caps` frontmatter.
    Explicit,
    /// A root-declared `run.caps.<pattern>` convention entry (the pattern).
    Convention(String),
    /// No declaration anywhere — deny-by-default, read-only.
    DenyDefault,
}

/// Which root situation produced a [`Conventions`] table. The ruling requires
/// every resolution to say which root answered rather than going silent, and
/// these three teach three different fixes.
///
/// Three states and not four: "the root declared a table" versus "the root
/// declared none" is read off [`Conventions`] being empty, so it needs no
/// variant of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConventionSource {
    /// `<root>/MERIDIAN.md` read as a `meridian-root` declaration. The table may
    /// still be empty — the root declares itself but states no `run.caps.*`.
    Declared(PathBuf),
    /// The root holds no `MERIDIAN.md`. Absent is not broken (the same stance
    /// `config`'s own D7 takes), so the table is empty and deny-by-default stands.
    Undeclared(PathBuf),
    /// No root resolved at all — the ladder's `CwdDefault`, where `root()` is
    /// `None`. There is no declaring root, so there is no ceiling to read and
    /// NO convention ceiling is in force. Not an error: refusing here would
    /// delete the convenience default the ruling deliberately kept.
    NoRoot,
}

impl ConventionSource {
    /// The root whose declaration was consulted, if there was one.
    #[must_use]
    pub fn root(&self) -> Option<&Path> {
        match self {
            Self::Declared(p) | Self::Undeclared(p) => Some(p.as_path()),
            Self::NoRoot => None,
        }
    }
}

impl std::fmt::Display for ConventionSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Declared(p) => write!(f, "declared by {}", p.display()),
            Self::Undeclared(p) => {
                write!(f, "{} declares no {}", p.display(), DECLARATION_FILENAME)
            }
            Self::NoRoot => f.write_str("no declaring root — no convention ceiling in force"),
        }
    }
}

/// A block's resolved capabilities: the effective set, where the grant came
/// from, and every granted cap a ceiling narrowed away (never silent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapResolution {
    /// The caps the block's effects are validated against at the choke point.
    pub effective: CapSet,
    /// The grant's origin.
    pub source: CapSource,
    /// Granted caps that did not survive the ceilings intact.
    pub narrowed: Vec<Cap>,
}

/// What the engine may claim about one block's effects — the ONE value
/// threaded from resolution to the executor's choke point.
///
/// Two variants, because there are two honest answers and no third. A block
/// either carries a capability contract the engine can KEEP, or it is an
/// unsandboxed shell the engine cannot bound at all. There is no `CapSet`
/// spelling of the second: cwd isolation and env scrubbing do not restrict
/// network, credentials, SSH or `rm -rf`, and the exec-window detector is
/// escaped by a `nohup` — so no value, including `none`, is true of bash.
///
/// The law is `docs/laws.md` § "Amendment — capabilities do not apply to
/// bash"; the executable gate is `crates/mrd/tests/law_no_caps_on_bash.rs`.
/// Structural, not cosmetic: the bash dispatcher never holds a
/// [`CapResolution`], so it cannot print, narrow, or half-enforce one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Authority {
    /// Starlark: a real, enforceable grant. The hermetic evaluator cannot
    /// reach past a descriptor, so the choke point validating against this set
    /// is a promise the engine keeps.
    Capabilities(CapResolution),
    /// Bash: an unsandboxed shell with undeclared effects. No set, no source,
    /// no `deny-default` — the engine states what it observes, never what the
    /// block may do.
    Unsandboxed,
}

impl Authority {
    /// A stated grant with no ceiling history — for call sites that name the
    /// caps directly (the proof corpus, executor tests) rather than resolving
    /// them off a page.
    #[must_use]
    pub fn granted(effective: CapSet) -> Self {
        Self::Capabilities(CapResolution {
            effective,
            source: CapSource::Explicit,
            narrowed: Vec::new(),
        })
    }

    /// The capability resolution, when this authority IS one.
    ///
    /// `None` for an unsandboxed shell, and every surface renders that as an
    /// ABSENT key rather than an empty one: `null` or `[]` would still be an
    /// answer to a question the engine cannot answer.
    #[must_use]
    pub fn capabilities(&self) -> Option<&CapResolution> {
        match self {
            Self::Capabilities(resolution) => Some(resolution),
            Self::Unsandboxed => None,
        }
    }

    /// Does this authority admit an effect of `kind` against `target`?
    ///
    /// `Unsandboxed` admits every descriptor — not a grant of everything, but
    /// the absence of a gate. Denying the shim here would only push the same
    /// write to `sed -i`, off the attested path, where the bracket at most
    /// detects it.
    #[must_use]
    pub fn admits(&self, kind: &str, target: Option<&str>) -> bool {
        match self {
            Self::Capabilities(resolution) => resolution.effective.admits(kind, target),
            Self::Unsandboxed => true,
        }
    }
}

/// Why capability handling refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapsError {
    /// A cap string is not `ns.name[:target]`.
    BadCap { raw: String },
    /// A convention pattern is not a literal or trailing-`*` prefix glob.
    BadPattern { pattern: String },
    /// `<root>/MERIDIAN.md` exists but does not read as a `meridian-root`
    /// declaration — an unreadable policy file never silently becomes "no
    /// policy". Silence here would delete a declared ceiling on one typo,
    /// which is a widening.
    Declaration { path: PathBuf, reason: String },
    /// A `check-*` / `verify-*` block carries a bash fence — refused at load
    /// (ruling 3): a read-only-by-convention name gets no exec.
    BashFenceRefused { task: String, pattern: String },
}

impl std::fmt::Display for CapsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CapsError::BadCap { raw } => {
                write!(f, "invalid capability '{raw}' (expected ns.name[:target])")
            }
            CapsError::BadPattern { pattern } => write!(
                f,
                "invalid caps pattern '{pattern}' (expected a name or prefix-*)"
            ),
            CapsError::Declaration { path, reason } => write!(
                f,
                "refused: {} does not read as a root declaration: {reason}. No convention table was loaded. Fix: give it `type: {DECLARATION_TYPE}`, `version: {}`, and a `name:`, or remove the file.",
                path.display(),
                config::VERSION,
            ),
            CapsError::BashFenceRefused { task, pattern } => write!(
                f,
                "task '{task}' matches read-only convention '{pattern}' but carries a bash fence — refused"
            ),
        }
    }
}

impl std::error::Error for CapsError {}

/// The explicit `task.<name>.caps` declaration, if present. A present-but-empty
/// declaration is an EXPLICIT read-only grant (distinct from undeclared).
///
/// # Errors
/// [`CapsError::BadCap`] on a malformed cap in the declaration.
pub fn explicit_caps(doc: &Document, task: &str) -> Result<Option<CapSet>, CapsError> {
    let Some(map) = frontmatter(doc) else {
        return Ok(None);
    };
    let key = format!("task.{task}.caps");
    match map.0.iter().find(|(k, _)| *k == key) {
        Some((_, value)) => CapSet::parse(value).map(Some),
        None => Ok(None),
    }
}

/// Read the `run.caps.<pattern>` conventions out of an already-read root
/// declaration. Pure: the caller owns the I/O and the validity check, this
/// owns only what the keys MEAN.
///
/// # Errors
/// [`CapsError::BadCap`] / [`CapsError::BadPattern`] on a malformed entry — a
/// mis-spelled ceiling is reported, never read as an absent one.
pub fn conventions_from_declaration(declaration: &Document) -> Result<Conventions, CapsError> {
    let Some(map) = frontmatter(declaration) else {
        return Ok(Conventions::none());
    };
    let mut entries = Vec::new();
    for (key, value) in &map.0 {
        let Some(pattern) = key.strip_prefix(CAPS_KEY_PREFIX) else {
            continue;
        };
        entries.push((pattern.to_owned(), CapSet::parse(value)?));
    }
    Conventions::new(entries)
}

/// Load the run-plane conventions declared by `root` — `<root>/MERIDIAN.md`
/// with `type: meridian-root`.
///
/// `root` is `None` when the ladder answered `CwdDefault`: there is no
/// declaring root, so the table is empty and [`ConventionSource::NoRoot`] says
/// so out loud rather than letting an absent ceiling pass for a satisfied one.
///
/// An absent declaration is the empty table (deny-by-default stands). A
/// PRESENT one that does not read as a root declaration is a loud refusal —
/// reading it as empty would silently drop a declared ceiling.
///
/// # Errors
/// [`CapsError::Declaration`] / [`CapsError::BadPattern`] / [`CapsError::BadCap`].
pub fn load_conventions(root: Option<&Path>) -> Result<(Conventions, ConventionSource), CapsError> {
    let Some(root) = root else {
        return Ok((Conventions::none(), ConventionSource::NoRoot));
    };
    match config::mount::read_root_declaration(root) {
        Ok(declaration) => Ok((
            conventions_from_declaration(&declaration.document)?,
            ConventionSource::Declared(root.to_path_buf()),
        )),
        Err(config::mount::DeclarationFault::Absent) => Ok((
            Conventions::none(),
            ConventionSource::Undeclared(root.to_path_buf()),
        )),
        Err(config::mount::DeclarationFault::Unreadable(reason)) => Err(CapsError::Declaration {
            path: root.join(DECLARATION_FILENAME),
            reason,
        }),
    }
}

/// Resolve what the engine may claim about one block — the ONE language-aware
/// entry, and the only place the bash law lives.
///
/// Order: (1) a `check-*` / `verify-*` bash fence refuses loudly — that is a
/// NAME law, not a capability, and it survives the amendment below; (2) any
/// other bash block is [`Authority::Unsandboxed`], resolved WITHOUT reading its
/// `task.<name>.caps` declaration, because that declaration governs nothing;
/// (3) starlark runs the full ladder ([`resolve_caps`]).
///
/// # Errors
/// [`CapsError::BashFenceRefused`] on a read-only-by-convention bash fence;
/// [`CapsError::BadCap`] on a malformed starlark cap declaration.
pub fn resolve_authority(
    doc: &Document,
    task: &str,
    lang: TaskLanguage,
    conventions: &Conventions,
) -> Result<Authority, CapsError> {
    if lang == TaskLanguage::Bash {
        if let Some(pattern) = READ_ONLY_PATTERNS.iter().find(|p| pattern_matches(p, task)) {
            return Err(CapsError::BashFenceRefused {
                task: task.to_owned(),
                pattern: (*pattern).to_owned(),
            });
        }
        return Ok(Authority::Unsandboxed);
    }
    let explicit = explicit_caps(doc, task)?;
    Ok(Authority::Capabilities(resolve_caps(
        task,
        explicit.as_ref(),
        conventions,
    )))
}

/// The capability ladder, over an already-read declaration: the grant resolves
/// explicit > convention > none, then ceilings narrow — a matching convention
/// over an EXPLICIT grant, then the builtin read-only ceiling over everything
/// it matches.
///
/// Takes no [`TaskLanguage`]: the ladder has no language axis to get wrong, and
/// the one caller that has a language ([`resolve_authority`]) reaches here only
/// for starlark.
#[must_use]
pub fn resolve_caps(
    task: &str,
    explicit: Option<&CapSet>,
    conventions: &Conventions,
) -> CapResolution {
    let read_only_match = READ_ONLY_PATTERNS.iter().find(|p| pattern_matches(p, task));
    let convention_match = conventions.matching(task);
    let (grant, source) = match (explicit, convention_match) {
        (Some(set), _) => (set.clone(), CapSource::Explicit),
        (None, Some((pattern, set))) => (set.clone(), CapSource::Convention(pattern.to_owned())),
        (None, None) => (CapSet::none(), CapSource::DenyDefault),
    };

    let mut effective = grant;
    let mut narrowed = Vec::new();
    // A convention over an explicit grant is a ceiling (narrow only, never
    // widen) — when the grant CAME from the convention this is a no-op.
    if source == CapSource::Explicit
        && let Some((_, ceiling)) = convention_match
    {
        let (e, n) = effective.narrow(ceiling);
        effective = e;
        narrowed.extend(n);
    }
    // The builtin read-only ceiling is absolute.
    if read_only_match.is_some() {
        let (e, n) = effective.narrow(&CapSet::none());
        effective = e;
        narrowed.extend(n);
    }
    CapResolution {
        effective,
        source,
        narrowed,
    }
}
