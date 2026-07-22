//! Capability resolution — deny-by-default (verdict ruling 3, plan decision
//! #15). An undeclared block is read-only: it can compute, but no effect of
//! its executes. Resolution precedence: explicit frontmatter
//! (`task.<name>.caps`) > `.meridian.toml` `[run.caps]` name-convention >
//! none. Conventions NARROW only, never widen: a matching convention acts as a
//! ceiling over an explicit grant (the intersection survives, the narrowed
//! remainder is reported), and the builtin `check-*` / `verify-*` read-only
//! ceiling cannot be overridden at all. `check-*` / `verify-*` names refuse a
//! bash fence loudly at load; `fix-*` does not — fix blocks declare writes and
//! are exactly where bash is wanted.
//!
//! Caps are namespaced strings (`md.set_field`), forward-compatible to
//! target-scoped (`md.set_field:status`) — a target-scoped cap is strictly
//! narrower than its untargeted form.

use std::collections::BTreeSet;
use std::path::Path;

use model::Document;

use crate::address::frontmatter;
use crate::fence::TaskLanguage;

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

/// The `[run.caps]` name-convention table from `.meridian.toml`: pattern →
/// grant/ceiling set, resolution-ordered (longest pattern first, then
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
    /// A `.meridian.toml` `[run.caps]` convention entry (the pattern).
    Convention(String),
    /// No declaration anywhere — deny-by-default, read-only.
    DenyDefault,
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

/// Why capability handling refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapsError {
    /// A cap string is not `ns.name[:target]`.
    BadCap { raw: String },
    /// A convention pattern is not a literal or trailing-`*` prefix glob.
    BadPattern { pattern: String },
    /// `.meridian.toml` exists but does not parse, or `[run.caps]` is not a
    /// table of string arrays — an unreadable policy file never silently
    /// becomes "no policy".
    Toml { reason: String },
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
            CapsError::Toml { reason } => write!(f, ".meridian.toml [run.caps]: {reason}"),
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

/// Load the `[run.caps]` conventions from `.meridian.toml` at the workspace
/// root. An absent file or absent section is the empty table (deny-by-default
/// stands); a malformed file is a loud typed error.
///
/// # Errors
/// [`CapsError::Toml`] / [`CapsError::BadPattern`] / [`CapsError::BadCap`].
pub fn load_conventions(workspace_root: &Path) -> Result<Conventions, CapsError> {
    let path = workspace_root.join(".meridian.toml");
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Conventions::none()),
        Err(e) => {
            return Err(CapsError::Toml {
                reason: e.to_string(),
            });
        }
    };
    let table: toml::Table = raw.parse().map_err(|e: toml::de::Error| CapsError::Toml {
        reason: e.to_string(),
    })?;
    let Some(caps) = table
        .get("run")
        .and_then(toml::Value::as_table)
        .and_then(|run| run.get("caps"))
    else {
        return Ok(Conventions::none());
    };
    let caps = caps.as_table().ok_or_else(|| CapsError::Toml {
        reason: "[run.caps] must be a table".to_owned(),
    })?;
    let mut entries = Vec::new();
    for (pattern, value) in caps {
        let list = value.as_array().ok_or_else(|| CapsError::Toml {
            reason: format!("[run.caps] '{pattern}' must be an array of cap strings"),
        })?;
        let mut set = BTreeSet::new();
        for item in list {
            let raw_cap = item.as_str().ok_or_else(|| CapsError::Toml {
                reason: format!("[run.caps] '{pattern}' entries must be strings"),
            })?;
            set.insert(Cap::parse(raw_cap)?);
        }
        entries.push((pattern.clone(), CapSet(set)));
    }
    Conventions::new(entries)
}

/// Resolve a block's effective capabilities (see the module docs for the law).
///
/// Order: (1) the builtin read-only conventions refuse a bash fence loudly;
/// (2) the grant resolves explicit > convention > none; (3) ceilings narrow —
/// a matching convention over an EXPLICIT grant, then the builtin read-only
/// ceiling over everything it matches.
///
/// # Errors
/// [`CapsError::BashFenceRefused`].
pub fn resolve_caps(
    task: &str,
    lang: TaskLanguage,
    explicit: Option<&CapSet>,
    conventions: &Conventions,
) -> Result<CapResolution, CapsError> {
    let read_only_match = READ_ONLY_PATTERNS.iter().find(|p| pattern_matches(p, task));
    if lang == TaskLanguage::Bash
        && let Some(pattern) = read_only_match
    {
        return Err(CapsError::BashFenceRefused {
            task: task.to_owned(),
            pattern: (*pattern).to_owned(),
        });
    }

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
    Ok(CapResolution {
        effective,
        source,
        narrowed,
    })
}
