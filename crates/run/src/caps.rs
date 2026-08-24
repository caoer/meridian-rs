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
//! # Cap grammar (caps-redesign ruling, 2026-08-19)
//! The cap plane speaks THREE VERBS — [`VERB_CREATE`] / [`VERB_EDIT`] /
//! [`VERB_DELETE`] — answering one question: may this block touch files
//! there. HOW it touches them is the descriptor plane's (executor ops),
//! extensible without growing this grammar. A verb is optionally scoped by a
//! PATH GLOB in the system's one glob grammar ([`policy::glob_match`] —
//! caps are its fourth consumer), matched at the choke point against the
//! block's DECLARED coordinates (ruling #2: the boundary is DATA — a birth's
//! `path` argument verbatim, its resolution base a separate axis; a page
//! edit in the coordinates the page was addressed by — the engine holds no
//! layout pattern). Several scopes = several cap entries; no list syntax.
//!
//! Migration (the ruled split): the retired per-op spellings `md.set_field`
//! and `md.append_section` FOLD into `md.edit` when untargeted; their
//! targeted forms REFUSE with the retirement teaching ([`CapsError::
//! RetiredTarget`]) — the old target was a field/section name, the new
//! target position is a path, and neither silent reinterpretation is legal.
//! Partition grain (parent-dir-name match) is retired with them.
//!
//! # One grammar, both planes (caps-one-parser ruling, 2026-08-23)
//! `caps:` is read by this plane AND by the policy plane's HOOK loader
//! (`crates/policy/src/hook.rs`) with different VOCABULARIES — verbs with an
//! optional glob scope here, descriptor kinds there. The SPELLING is shared and
//! has exactly one owner: [`model::parse_caps_list`] tokenizes, and
//! [`model::fm_doc_caps`] / `model::fm_caps` read the key off the frontmatter
//! block. Bare scalar, flow sequence and block sequence therefore mean the same
//! thing on both planes; `caps: []` is a DECLARED empty grant and a bare
//! `caps:` is not a declaration at all, on both. Gate:
//! `crates/testsuite/tests/caps_one_grammar.rs`.
//!
//! Before that, each plane had its own reader and they disagreed in opposite
//! directions — the run plane faulted `invalid capability '[md.create'` on a
//! flow sequence the policy plane read fine, and the policy plane refused
//! `invalid type: string` on a plain scalar the run plane read fine (card
//! `caps-flow-sequence-faults-at-run-plane` and its sibling).
//!
//! # Convention plane (marker-retirement ruling)
//! Table lives in `<root>/MERIDIAN.md` with `type: meridian-root`. Retired
//! marker files are not read and have no fallback. Grammar is dotted keys,
//! whose VALUE takes any of the three spellings above:
//!
//! ```yaml
//! run.caps.fix-*: md.edit
//! run.caps.fix-note: md.edit:tasks/*.md
//! ```
//!
//! [`ConventionSource`] reports which root situation answered. Present-but-
//! invalid declaration REFUSES ([`CapsError::Declaration`]) — silent empty
//! table would delete a ceiling on typo. Scoped caps are narrower than
//! untargeted forms.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use model::Document;

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

/// The birth verb: may this block CREATE files there.
pub const VERB_CREATE: &str = "md.create";
/// The edit verb: may this block CHANGE pages there (every page-mutating
/// descriptor — `md.set_field`, `md.append_section` — authorizes under it).
pub const VERB_EDIT: &str = "md.edit";
/// The delete verb — reserved with the grammar (caps-redesign ruling): it
/// parses and resolves today so grants can be written ahead, but no
/// descriptor maps to it until a retire descriptor exists.
pub const VERB_DELETE: &str = "md.delete";

/// One parsed capability: a verb (`md.create`) with an optional path-glob
/// scope (`md.create:tasks/*.md`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cap {
    /// The verb string ([`VERB_CREATE`] / [`VERB_EDIT`] / [`VERB_DELETE`]).
    pub kind: String,
    /// The optional path-glob scope (`tasks/*.md` in `md.create:tasks/*.md`),
    /// in the one glob grammar, matched against declared coordinates.
    pub target: Option<String>,
}

/// Why `glob` is not a legal cap scope, or `None` when it is. A scope is a
/// path glob in the system's ONE glob grammar ([`policy::glob_match`]),
/// shaped like the workspace-relative paths it matches: relative, no
/// `.`/`..`/empty segment.
fn bad_glob(glob: &str) -> Option<String> {
    if glob.is_empty() {
        return Some("the scope is empty".to_owned());
    }
    for segment in glob.split('/') {
        if segment.is_empty() {
            return Some("empty segment (a leading, trailing or doubled `/`)".to_owned());
        }
        if segment == "." || segment == ".." {
            return Some(format!(
                "`{segment}` segment — scopes are workspace-shaped, never navigated"
            ));
        }
        if let Some(c) = segment
            .chars()
            .find(|c| !c.is_ascii_alphanumeric() && !matches!(c, '_' | '-' | '.' | '*' | '='))
        {
            return Some(format!(
                "segment `{segment}` carries `{c}` (outside letters, digits, `_ - . * =`)"
            ));
        }
    }
    None
}

impl Cap {
    /// Parse one cap string: `verb` or `verb:path/glob`.
    ///
    /// The verb fold (caps-redesign ruling, 2026-08-19): the retired per-op
    /// spellings `md.set_field` / `md.append_section` FOLD into [`VERB_EDIT`]
    /// when untargeted — live grants keep working across the cutover, and
    /// every surface reports the canonical verb. Their TARGETED forms refuse
    /// ([`CapsError::RetiredTarget`]): the old target named a field or
    /// section, the new target position is a path glob, and both silent
    /// reinterpretations are wrong — dropping the target widens the grant,
    /// reading it as a path kills it.
    ///
    /// # Errors
    /// [`CapsError::BadCap`] on an unknown verb; [`CapsError::RetiredTarget`]
    /// on a targeted retired spelling; [`CapsError::BadGlob`] on a malformed
    /// scope.
    pub fn parse(raw: &str) -> Result<Self, CapsError> {
        let (kind, target) = match raw.split_once(':') {
            Some((k, t)) => (k, Some(t)),
            None => (raw, None),
        };
        let kind = match kind {
            VERB_CREATE | VERB_EDIT | VERB_DELETE => kind,
            "md.set_field" | "md.append_section" => {
                if target.is_some() {
                    return Err(CapsError::RetiredTarget {
                        raw: raw.to_owned(),
                    });
                }
                VERB_EDIT
            }
            _ => {
                return Err(CapsError::BadCap {
                    raw: raw.to_owned(),
                });
            }
        };
        if let Some(glob) = target
            && let Some(reason) = bad_glob(glob)
        {
            return Err(CapsError::BadGlob {
                raw: raw.to_owned(),
                reason,
            });
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

    /// Does this cap admit an effect of verb `kind` against the path
    /// `target`? An unscoped cap admits every path of its verb; a scoped cap
    /// admits the paths its glob matches. The path the choke point asks with
    /// is the block's DECLARED coordinate (executor `descriptor_surface`) —
    /// the coordinate is the caller's, matching is [`policy::glob_match`]'s,
    /// and this method adds no third opinion.
    #[must_use]
    pub fn admits(&self, kind: &str, target: Option<&str>) -> bool {
        self.kind == kind
            && match &self.target {
                None => true,
                Some(glob) => target.is_some_and(|path| policy::glob_match(glob, path)),
            }
    }

    /// The meet (narrower) of two caps of the same kind, if comparable:
    /// unscoped ∩ scoped = the scoped one; NESTED scopes = the inner one,
    /// decided by [`policy::glob_subsumes`] — segment-wise `**`/`*`/literal
    /// containment in the one glob grammar (cap-meet-subsumption ruling,
    /// 2026-08-20; string equality before that). Scopes neither containing
    /// the other are incomparable (`None`): overlap is not nesting (glob
    /// intersection has no simple normal form), and the subsumption check is
    /// itself conservative (a containment it cannot prove segment-wise reads
    /// as incomparable), so narrowing can only drop, never widen.
    fn meet(&self, other: &Cap) -> Option<Cap> {
        if self.kind != other.kind {
            return None;
        }
        match (&self.target, &other.target) {
            (None, t) | (t, None) => Some(Cap {
                kind: self.kind.clone(),
                target: t.clone(),
            }),
            (Some(a), Some(b)) if policy::glob_subsumes(b, a) => Some(self.clone()),
            (Some(a), Some(b)) if policy::glob_subsumes(a, b) => Some(other.clone()),
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

    /// Parse a cap list VALUE in the one capability grammar
    /// ([`model::parse_caps_list`]): items separate on comma, space or tab, a
    /// surrounding bracket pair is stripped, quotes come off the value and off
    /// each item.
    ///
    /// The splitting is the POLICY PLANE's, literally — the same function
    /// `crates/policy/src/hook.rs` tokenizes a HOOK's `caps:` with. This door
    /// used to trim quotes off the whole string and never brackets, so a flow
    /// sequence (`[md.create, md.edit]`) that the policy plane read fine
    /// reached [`Cap::parse`] as `[md.create` and faulted (card
    /// `caps-flow-sequence-faults-at-run-plane`).
    ///
    /// # Errors
    /// [`CapsError::BadCap`] on the first malformed cap.
    pub fn parse(raw: &str) -> Result<Self, CapsError> {
        let mut set = BTreeSet::new();
        for word in model::parse_caps_list(raw) {
            set.insert(Cap::parse(&word)?);
        }
        Ok(Self(set))
    }

    /// A set from tokens the one grammar already split ([`model::fm_doc_caps`]).
    ///
    /// # Errors
    /// [`CapsError::BadCap`] on the first malformed cap.
    fn from_tokens(tokens: &[String]) -> Result<Self, CapsError> {
        let mut set = BTreeSet::new();
        for token in tokens {
            set.insert(Cap::parse(token)?);
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

/// Which root situation produced a [`Conventions`] table — every resolution
/// reports which root answered rather than going silent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConventionSource {
    /// `<root>/MERIDIAN.md` read as a `meridian-root` declaration. The table may
    /// still be empty — the root declares itself but states no `run.caps.*`.
    Declared(PathBuf),
    /// The root holds no `MERIDIAN.md`. Absent is not broken: the table is
    /// empty and deny-by-default stands.
    Undeclared(PathBuf),
    /// No root resolved at all — the ladder's `CwdDefault`, where `root()` is
    /// `None`. No declaring root, so no convention ceiling is in force. Not an
    /// error.
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

/// Which ceiling narrowed a grant. Named so a denial can say what ate the cap
/// the caller can see on their own page (run-plane § capabilities) — the
/// identity is carried from the resolution, never re-derived at the refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ceiling {
    /// The root's `run.caps.<pattern>` entry that won the match.
    Convention(String),
    /// The builtin `check-*` / `verify-*` read-only ceiling, and the pattern
    /// the task name matched.
    BuiltinReadOnly(String),
}

impl std::fmt::Display for Ceiling {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Convention(pattern) => write!(f, "the root convention `run.caps.{pattern}`"),
            Self::BuiltinReadOnly(pattern) => {
                write!(f, "the builtin read-only ceiling on `{pattern}` names")
            }
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
    /// Each ceiling that narrowed, WITH the caps it took — the pairing is what
    /// lets a denial name the ceiling responsible for THIS descriptor instead
    /// of the last one that happened to run.
    pub ceilings: Vec<(Ceiling, Vec<Cap>)>,
}

impl CapResolution {
    /// The ceiling that took a cap which would have admitted `kind`/`target` —
    /// `None` when no ceiling is the reason this descriptor is denied (a
    /// deny-default block, or an explicit grant that never held the cap).
    ///
    /// The denial teaches a ceiling remedy ONLY on `Some`: a remedy derived
    /// from a cause the engine did not measure is worse than no remedy.
    #[must_use]
    pub fn ceiling_denying(&self, kind: &str, target: Option<&str>) -> Option<&Ceiling> {
        self.ceilings
            .iter()
            .find(|(_, taken)| taken.iter().any(|c| c.admits(kind, target)))
            .map(|(ceiling, _)| ceiling)
    }
}

/// What the engine may claim about one block's effects — the one value
/// threaded from resolution to the executor's choke point.
///
/// A block either carries a capability contract the engine can keep, or it is
/// an unsandboxed shell it cannot bound — no `CapSet` value, including `none`,
/// is true of bash. Law: `docs/laws.md` § "Amendment — capabilities do not
/// apply to bash"; gate: `crates/mrd/tests/law_no_caps_on_bash.rs`.
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
            ceilings: Vec::new(),
        })
    }

    /// The capability resolution, when this authority is one.
    ///
    /// `None` for an unsandboxed shell — every surface renders that as an
    /// absent key, never an empty one.
    #[must_use]
    pub fn capabilities(&self) -> Option<&CapResolution> {
        match self {
            Self::Capabilities(resolution) => Some(resolution),
            Self::Unsandboxed => None,
        }
    }

    /// Does this authority admit an effect of `kind` against `target`?
    ///
    /// `Unsandboxed` admits everything — the absence of a gate, not a
    /// grant: denying it would only push the same write off the attested
    /// path.
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
    /// A cap string names no known verb.
    BadCap { raw: String },
    /// A cap scope is not a legal path glob.
    BadGlob { raw: String, reason: String },
    /// A retired field-grain spelling (`md.set_field:status`) — the target
    /// position is a path glob now, and no silent reinterpretation is legal.
    RetiredTarget { raw: String },
    /// A convention pattern is not a literal or trailing-`*` prefix glob.
    BadPattern { pattern: String },
    /// `<root>/MERIDIAN.md` exists but does not read as a `meridian-root`
    /// declaration — an unreadable policy file never silently becomes "no
    /// policy".
    Declaration { path: PathBuf, reason: String },
    /// A `check-*` / `verify-*` block carries a bash fence — refused at load
    /// (ruling 3): a read-only-by-convention name gets no exec.
    BashFenceRefused { task: String, pattern: String },
    /// A `run.caps.<pattern>` entry in a root declaration did not parse. The
    /// `source` fault says WHAT is wrong; this variant says WHERE — the
    /// declaration path and the key whose value failed.
    ///
    /// It exists because one bad entry refuses the WHOLE table by design
    /// (a mis-spelled ceiling is never read as an absent one), so the blast
    /// radius is every task on the root — and the bare inner refusal named
    /// neither the file nor the key, leaving an operator with a bricked root
    /// and nothing to grep for (round-2 review of caps-redesign-docs,
    /// defect b). `path` is `None` only for a caller that parsed a
    /// declaration it never read from disk.
    TableEntry {
        path: Option<PathBuf>,
        key: String,
        source: Box<CapsError>,
    },
}

impl std::fmt::Display for CapsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CapsError::BadCap { raw } => {
                write!(
                    f,
                    "invalid capability '{raw}' — a cap is one of the verbs `md.create`, \
                     `md.edit`, `md.delete`, optionally scoped by a path glob \
                     (`md.create:tasks/*.md`) matched against the block's declared \
                     relative paths."
                )
            }
            CapsError::BadGlob { raw, reason } => {
                write!(
                    f,
                    "invalid capability '{raw}': {reason}. A scope is a path glob in the \
                     one glob grammar — segments split on `/`, `**` spans whole segments, \
                     `*` matches within a segment — shaped workspace-relative (no absolute \
                     path, no `.`/`..`/empty segment)."
                )
            }
            CapsError::RetiredTarget { raw } => {
                write!(
                    f,
                    "capability '{raw}' uses the retired field-grain form — the cap target \
                     position is a PATH GLOB matched against the block's declared landing \
                     path, never a field or section name. Bare `md.set_field` / \
                     `md.append_section` fold into `md.edit`; a field-grain guard belongs \
                     inside the block. Scope by path instead, e.g. `md.edit:agents/*/CARD.md`."
                )
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
            CapsError::TableEntry { path, key, source } => {
                match path {
                    Some(path) => write!(f, "refused: {}: `{key}`: ", path.display())?,
                    None => write!(f, "refused: the root declaration's `{key}`: ")?,
                }
                write!(f, "{source}")?;
                write!(
                    f,
                    " One bad entry refuses the WHOLE convention table — a mis-spelled \
                     ceiling is never read as an absent one — so every task on this root \
                     refuses until `{key}` is fixed, read-only tasks and `--list` included. \
                     A frequent cause is a TRAILING YAML COMMENT: the frontmatter scanner \
                     strips none, so `{key}: md.edit # longest wins` parses `#` as a \
                     capability."
                )
            }
        }
    }
}

impl std::error::Error for CapsError {}

/// The explicit `task.<name>.caps` declaration, if present. A `caps: []`
/// declaration is an EXPLICIT read-only grant (distinct from undeclared).
///
/// Read through [`model::fm_doc_caps`], the one capability grammar, so every
/// YAML spelling of the list means the same thing here as it does on the
/// policy plane — and so a BLOCK sequence is seen at all: `frontmatter(doc)`
/// keeps only the key LINE's remainder, which for a block list is the empty
/// string, and an empty string parses to an explicit read-only grant. That is
/// a page's declared caps silently replaced, not a fault (same defect class as
/// card `fm-block-list-sql-empty`).
///
/// # Errors
/// [`CapsError::BadCap`] on a malformed cap in the declaration.
pub fn explicit_caps(doc: &Document, task: &str) -> Result<Option<CapSet>, CapsError> {
    let key = format!("task.{task}.caps");
    match model::fm_doc_caps(doc, &key) {
        Some(tokens) => CapSet::from_tokens(&tokens).map(Some),
        None => Ok(None),
    }
}

/// The frontmatter key a declaring page's own capability ceiling lives under.
///
/// New grammar (hook-support design § 2.2, § 2.5). It is deliberately NOT
/// `task.<name>.caps`: that key scopes to one task, and a declaring page's
/// blocks have no task names — the ceiling is the PAGE's, one for every block
/// it declares, which is what makes a page's authority readable in one line
/// by whoever reviews the page.
pub const PAGE_CAPS_KEY: &str = "caps";

/// The page-level `caps:` declaration, if present (§ 2.2 *Evaluation — fire*,
/// step 2).
///
/// A `caps: []` declaration is an EXPLICIT read-only grant, exactly as on the
/// task plane — the distinction between "declared nothing" and "did not
/// declare" is preserved because they are different statements by the author.
/// Absent is deny-by-default: a fire on a page with no `caps:` may call any
/// constructor and every one of them is refused at [`crate::executor::admit`]
/// with `cap_denied`, which is A1's shape — callable and refused, never absent
/// and mysterious.
///
/// This is the key the daemon's declaring pages carry, so it is the key both
/// planes read: [`model::fm_doc_caps`] is the same grammar
/// `crates/policy/src/hook.rs` loads a HOOK's `caps:` through. A bare `caps:`
/// with nothing after it is NOT a declaration on either plane — the engine
/// never invents `[]` for it.
///
/// # Errors
/// [`CapsError::BadCap`] on a malformed cap in the declaration.
pub fn page_caps(doc: &Document) -> Result<Option<CapSet>, CapsError> {
    match model::fm_doc_caps(doc, PAGE_CAPS_KEY) {
        Some(tokens) => CapSet::from_tokens(&tokens).map(Some),
        None => Ok(None),
    }
}

/// The authority one declared block fires under: the page's `caps:`, narrowed
/// by the declaring root's conventions exactly as the task ladder narrows an
/// explicit grant.
///
/// Takes no language axis, unlike [`resolve_authority`]: caps do not apply to
/// a process (law, § 1.4), so an exec'd entry never reaches here — the caller
/// dispatches on the entry kind before asking about authority.
#[must_use]
pub fn resolve_page_authority(page_caps: Option<&CapSet>, conventions: &Conventions) -> Authority {
    // The conventions table is keyed by TASK NAME patterns. A declaring block
    // has no task name, so only a table entry that matches everything can
    // bind it; `resolve_caps` handles that through the same matcher, with the
    // page's grant standing in for the explicit one.
    Authority::Capabilities(resolve_caps(PAGE_CAPS_KEY, page_caps, conventions))
}

/// Read the `run.caps.<pattern>` conventions out of an already-read root
/// declaration. Pure: the caller owns the I/O and the validity check, this
/// owns only what the keys MEAN.
///
/// `source` is the declaration's own path, carried for REFUSALS only — never
/// read, never opened. A caller parsing a declaration it did not load from
/// disk (a doc-fence extraction gate) passes `None` and gets the same fault
/// minus the path. It is a parameter rather than a field on the document
/// because one bad entry bricks the whole root, and a refusal that cannot
/// name the file leaves nothing to grep for.
///
/// # Errors
/// [`CapsError::TableEntry`] wrapping the entry's own fault — the inner
/// [`CapsError::BadCap`] / [`CapsError::BadGlob`] / [`CapsError::RetiredTarget`]
/// says what is wrong, the wrapper says which file and key — or
/// [`CapsError::BadPattern`] from the pattern grammar. A mis-spelled ceiling is
/// reported, never read as an absent one.
pub fn conventions_from_declaration(
    declaration: &Document,
    source: Option<&Path>,
) -> Result<Conventions, CapsError> {
    let mut entries = Vec::new();
    // Key NAMES from the frontmatter map (the pattern is whatever follows the
    // fixed prefix), VALUES through the one capability grammar — so a
    // convention entry spelled as a block sequence is read, not silently
    // emptied, and every spelling means here what it means on the policy plane.
    for key in model::fm_doc_keys(declaration) {
        let Some(pattern) = key.strip_prefix(CAPS_KEY_PREFIX) else {
            continue;
        };
        // A bare `run.caps.<pattern>:` declares no cap, and a ceiling that
        // declares no cap is the EMPTY one — never an absent entry. Fail
        // closed, the same direction an absent `caps:` fails on the grant
        // side: a mis-spelled ceiling is never read as no ceiling (see
        // [`CapsError::TableEntry`]).
        let tokens = model::fm_doc_caps(declaration, &key).unwrap_or_default();
        let caps = CapSet::from_tokens(&tokens).map_err(|fault| CapsError::TableEntry {
            path: source.map(Path::to_path_buf),
            key: key.clone(),
            source: Box::new(fault),
        })?;
        entries.push((pattern.to_owned(), caps));
    }
    Conventions::new(entries)
}

/// Load the run-plane conventions declared by `root` — `<root>/MERIDIAN.md`
/// with `type: meridian-root`.
///
/// `root` is `None` when the ladder answered `CwdDefault`: no declaring root,
/// so the table is empty and [`ConventionSource::NoRoot`] says so.
///
/// An absent declaration is the empty table (deny-by-default stands). A
/// present one that does not read as a root declaration is a loud refusal —
/// reading it as empty would silently drop a declared ceiling.
///
/// # Errors
/// [`CapsError::Declaration`] / [`CapsError::BadPattern`] /
/// [`CapsError::TableEntry`] (which names the declaration path and the failing
/// key — this is the only caller that knows the path).
pub fn load_conventions(root: Option<&Path>) -> Result<(Conventions, ConventionSource), CapsError> {
    let Some(root) = root else {
        return Ok((Conventions::none(), ConventionSource::NoRoot));
    };
    match config::mount::read_root_declaration(root) {
        Ok(declaration) => Ok((
            conventions_from_declaration(
                &declaration.document,
                Some(&root.join(DECLARATION_FILENAME)),
            )?,
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
/// Order: (1) a `check-*` / `verify-*` bash fence refuses loudly — a name
/// law, not a capability; (2) any other bash block is
/// [`Authority::Unsandboxed`], resolved without reading its
/// `task.<name>.caps` declaration, which governs nothing; (3) starlark runs
/// the full ladder ([`resolve_caps`]).
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
    let mut ceilings: Vec<(Ceiling, Vec<Cap>)> = Vec::new();
    // A convention over an explicit grant is a ceiling (narrow only, never
    // widen) — when the grant CAME from the convention this is a no-op.
    if source == CapSource::Explicit
        && let Some((pattern, ceiling)) = convention_match
    {
        let (e, n) = effective.narrow(ceiling);
        effective = e;
        if !n.is_empty() {
            ceilings.push((Ceiling::Convention(pattern.to_owned()), n));
        }
    }
    // The builtin read-only ceiling is absolute.
    if let Some(pattern) = read_only_match {
        let (e, n) = effective.narrow(&CapSet::none());
        effective = e;
        if !n.is_empty() {
            ceilings.push((Ceiling::BuiltinReadOnly((*pattern).to_owned()), n));
        }
    }
    CapResolution {
        effective,
        source,
        narrowed: ceilings.iter().flat_map(|(_, n)| n.clone()).collect(),
        ceilings,
    }
}
