//! The agent-plane address, as a fallible type.
//!
//! `[root:]path[#selector]` — parsed once, here, so no plane downstream re-splits a
//! string. A `std`-only leaf with zero dependencies, upstream of `syntax`
//! (`docs/address-grammar.md` § 7.1).
//!
//! [`Addr::parse`] is the sole constructor: `Addr.path` never carries a root prefix,
//! which is what makes every downstream guard checkable.
//!
//! Parse is not resolve: [`Addr::parse`] never touches the mount table. Whether the
//! named root is bound is the resolver's question, answered grey, not a parse error
//! (§ 2.2, § 6).

use core::fmt;

pub mod stored;

/// A canonical root NAME — the mount-table key a cross-root address carries
/// (`sessions`, `assets`).
///
/// Named `MountName`, never `Root`: this is neither a Merkle cursor
/// (`wire::Root`), a directory (`fs::WorkspaceRoot`), nor the `root:`
/// frontmatter key (`docs/address-grammar.md` § 1).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MountName(String);

impl MountName {
    /// The one charset: `[a-z0-9-]`, non-empty (`docs/address-grammar.md`
    /// § 4.3).
    ///
    /// An uppercase byte refuses rather than normalizing — one name per root,
    /// used everywhere.
    ///
    /// # Errors
    ///
    /// [`AddrError::EmptyMountName`] when `name` is empty;
    /// [`AddrError::BadMountName`] when any byte is outside the charset.
    pub fn parse(name: &str) -> Result<Self, AddrError> {
        if name.is_empty() {
            return Err(AddrError::EmptyMountName);
        }
        if !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(AddrError::BadMountName {
                found: name.to_string(),
            });
        }
        Ok(MountName(name.to_string()))
    }

    /// The canonical name, borrowed.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MountName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why an address refused to parse — the closed set of `docs/address-grammar.md`
/// § 4.5.
///
/// Every variant carries the offending text, so a refusal can name what it
/// refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddrError {
    /// The head carried a root separator but the name before it is outside the
    /// `[a-z0-9-]` charset — including any uppercase byte (§ 4.3).
    BadMountName {
        /// The rejected name, verbatim.
        found: String,
    },
    /// The head opened with the root separator (`:notes.md`) — a separator with
    /// no name before it.
    EmptyMountName,
    /// Nothing follows the root separator (`sessions:`), or the address is
    /// empty. An address always names a path.
    EmptyPath,
    /// Two or more colons in the head (`a:b:c.md`) — exactly one colon may act
    /// as a separator.
    AmbiguousColon {
        /// The offending head, verbatim.
        found: String,
    },
}

impl fmt::Display for AddrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AddrError::BadMountName { found } => write!(
                f,
                "refused: '{found}' is not a canonical root name — root names are `[a-z0-9-]`. \
                 Fix: quote the path differently or rename the root; see [[address-grammar]]."
            ),
            AddrError::EmptyMountName => f.write_str(
                "refused: the address opens with a root separator and names no root. \
                 Fix: write `root:path`, or drop the leading `:`; see [[address-grammar]].",
            ),
            AddrError::EmptyPath => f.write_str(
                "refused: the address names no path. \
                 Fix: write `root:path` — a root alone addresses nothing; see [[address-grammar]].",
            ),
            AddrError::AmbiguousColon { found } => write!(
                f,
                "refused: '{found}' carries more than one `:` before the first `/` — \
                 exactly one colon may separate a root from its path. \
                 Fix: rename the root or move the path under a directory; see [[address-grammar]]."
            ),
        }
    }
}

impl std::error::Error for AddrError {}

/// The agent-plane address `[root:]path[#selector]`, parsed.
///
/// Construct with [`Addr::parse`] — the sole constructor. Round-trips
/// byte-identically through [`Display`](fmt::Display).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Addr {
    root: Option<MountName>,
    path: String,
    selector: Option<String>,
}

impl Addr {
    /// Parse an agent-plane address. The sole constructor.
    ///
    /// The colon law (`docs/address-grammar.md` § 4.1), over the head — the
    /// text before the first `/` and before the first `#`:
    ///
    /// - zero `:` — no root; the address resolves in the ambient root;
    /// - exactly one `:` — the root separator: a well-formed [`MountName`]
    ///   before it, a non-empty path after it, else REFUSED — never
    ///   reinterpreted as a literal path (a typo'd root must refuse, not
    ///   degrade into a literal-path lookup);
    /// - two or more `:` — refused.
    ///
    /// After the first `/`, a `:` is an ordinary path byte.
    ///
    /// A fragment runs from the first `#` to the end of the spelling, and every
    /// byte of it is selector bytes — `@` included (§ 4.4, Law A-2).
    /// Fingerprint pinning is its own field on whatever surface carries one,
    /// never an in-band suffix parsed out of the name lane.
    ///
    /// # Errors
    ///
    /// The closed set of [`AddrError`].
    pub fn parse(spelling: &str) -> Result<Self, AddrError> {
        let (before_frag, frag) = match spelling.split_once('#') {
            Some((left, frag)) => (left, Some(frag)),
            None => (spelling, None),
        };

        // A `:` after the first `/` is an ordinary path byte (§ 4.1).
        let head = before_frag.split('/').next().unwrap_or(before_frag);
        let (root, path) = match head.match_indices(':').count() {
            0 => (None, before_frag.to_string()),
            1 => {
                let colon = head.find(':').unwrap_or(0);
                let name = MountName::parse(&head[..colon])?;
                let rest = &before_frag[colon + 1..];
                if rest.is_empty() {
                    return Err(AddrError::EmptyPath);
                }
                (Some(name), rest.to_string())
            }
            _ => {
                return Err(AddrError::AmbiguousColon {
                    found: head.to_string(),
                });
            }
        };
        if path.is_empty() {
            return Err(AddrError::EmptyPath);
        }

        Ok(Addr {
            root,
            path,
            selector: frag.map(str::to_string),
        })
    }

    /// The canonical root name this address names, or `None` for the ambient
    /// root.
    #[must_use]
    pub fn root(&self) -> Option<&MountName> {
        self.root.as_ref()
    }

    /// The path portion — **never carries a root prefix, by construction.**
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The selector — every byte after the first `#`, to the END of the
    /// spelling, `@` included (Law A-2). `""` means the page/doc root,
    /// matching the shipped `node.selector` column.
    #[must_use]
    pub fn selector(&self) -> &str {
        self.selector.as_deref().unwrap_or("")
    }

    /// Did the spelling carry a `#` at all? Distinguishes `page` (page-grain)
    /// from `page#` (an empty selector written out) — the two are different
    /// spellings and this type round-trips both.
    #[must_use]
    pub fn has_selector(&self) -> bool {
        self.selector.is_some()
    }

    /// The address minus its selector: `[root:]path` — the identity a corpus
    /// lookup addresses. The root stays on the spelling; the root-keyed
    /// lookup belongs to the resolver.
    #[must_use]
    pub fn target(&self) -> String {
        match &self.root {
            Some(root) => format!("{root}:{}", self.path),
            None => self.path.clone(),
        }
    }

    /// This address with its path replaced — the resolution seam, fallible on
    /// purpose: a resolved corpus path must itself carry no root separator, or
    /// the [`Addr::parse`] invariant could be smuggled around. Root and
    /// selector ride through unchanged.
    ///
    /// # Errors
    ///
    /// [`AddrError`] when `path` is empty or its head carries a `:` — the
    /// `grey(unaddressable-path)` corpus key of § 4.2.
    pub fn with_path(&self, path: &str) -> Result<Self, AddrError> {
        let reparsed = Addr::parse(path)?;
        if reparsed.root.is_some() || reparsed.selector.is_some() {
            return Err(AddrError::AmbiguousColon {
                found: path.to_string(),
            });
        }
        Ok(Addr {
            root: self.root.clone(),
            path: reparsed.path,
            selector: self.selector.clone(),
        })
    }
}

impl fmt::Display for Addr {
    /// The canonical spelling — byte-identical to what was parsed. The `lock`
    /// byte-round-trip seam and the verbatim `declared_ref` column both
    /// require lossless rendering.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(root) = &self.root {
            write!(f, "{root}:")?;
        }
        f.write_str(&self.path)?;
        if let Some(sel) = &self.selector {
            write!(f, "#{sel}")?;
        }
        Ok(())
    }
}

/// The reason word for a root that is DECLARED but cannot be read here — the
/// one source, in the bare form.
///
/// Two planes print it: the mount plane wraps it as `grey(path-unseeable)`,
/// the address plane takes it bare and lets its own renderer wrap. Each
/// plane's wrapped spelling is checked against this const at compile time.
pub const PATH_UNSEEABLE_REASON_WORD: &str = "path-unseeable";

/// Does this spelling's HEAD carry a root separator? — the one lexical
/// question "could the address plane have something to say about this?"
///
/// The head is the text before the first `/` and before the first `#` (§ 4.1).
/// Deliberately weaker than "parses as a rooted [`Addr`]": a malformed rooted
/// spelling does not parse, and its address-plane answer is a REFUSAL — still
/// an answer. A parse-success predicate would drop every refusal silently.
///
/// Lives in this leaf because the resolver's C-3 guard and the link plane's
/// degrade must be the same test, not two that agree today. The degrade's
/// SECOND, table-narrowed question is [`head_names_declared_root`] below.
#[must_use]
pub fn head_carries_root_separator(spelling: &str) -> bool {
    let trimmed = spelling.trim();
    let before_frag = trimmed.split_once('#').map_or(trimmed, |(left, _)| left);
    before_frag
        .split('/')
        .next()
        .unwrap_or(before_frag)
        .contains(':')
}

/// Does this spelling's head name a root `set` DECLARES — the link plane's
/// NARROWED degrade question: "would the address plane's answer differ from
/// `unresolved` on a table THIS machine holds?".
///
/// [`head_carries_root_separator`] answers the wider lexical question, and an
/// external URI trips it (`https://…` has `https:` in its head) — one
/// `[[https://…]]` wikilink then costs a whole-corpus ephemeral rebuild for a
/// spelling no mount table binds. This predicate takes the text before the
/// head's FIRST `:` and asks [`MountSet::is_declared`]: declared-but-malformed
/// remainders (`sessions:`, `sessions:a:b`) still answer `true` — the refusal
/// is the address plane's to speak — while an undeclared or uncharsetted name
/// (`https`, `MixedCase`) answers `false` and the ambient `unresolved` stands.
///
/// Lives in this leaf beside the wide predicate so the head-peeling stays one
/// spelling of the colon law (§ 4.1), not two that agree today.
#[must_use]
pub fn head_names_declared_root(spelling: &str, set: &MountSet) -> bool {
    let trimmed = spelling.trim();
    let before_frag = trimmed.split_once('#').map_or(trimmed, |(left, _)| left);
    let head = before_frag.split('/').next().unwrap_or(before_frag);
    let Some((name, _)) = head.split_once(':') else {
        return false;
    };
    MountName::parse(name).is_ok_and(|n| set.is_declared(&n))
}

/// The one lexical confinement predicate for a corpus-relative path.
///
/// A path is confined when it is non-empty, does not start with `/`, has no
/// empty / `.` / `..` segment, and carries no root separator in its head — a
/// `root:`-bearing spelling is an ADDRESS, never a corpus path (§ 4.2, D11).
///
/// Lives in this leaf because the write doors in `wire-serve` and the resolver
/// in `model` must share one implementation. Lexical only: it cannot see a
/// symlink escape; canonicalize-at-bind (§ 8 B-1) covers that.
#[must_use]
pub fn confined(path: &str) -> bool {
    if path.is_empty() || path.starts_with('/') {
        return false;
    }
    if path
        .split('/')
        .any(|seg| seg.is_empty() || seg == "." || seg == "..")
    {
        return false;
    }
    // A `:` after the first `/` is an ordinary path byte (§ 4.1).
    !path.split('/').next().unwrap_or(path).contains(':')
}

/// The resolution-facing projection of the mount table: which canonical names
/// this machine binds.
///
/// Constructed by `config`, consumed by `model` — it lives in this upstream
/// leaf so the resolver can name it without depending on `config`
/// (`docs/address-grammar.md` § 7.2). A concrete type, not a trait: a second
/// implementation of an address fact is one question with two answers.
///
/// Answers three questions: is this name bound; which names are bound (so a
/// refusal can teach); and is this name DECLARED but unreadable here — a
/// distinct fact, or its refusal would prescribe a fix that is already done.
///
/// The vault axis maps name ↔ vault name ↔ path; the stored plane is spelled
/// in vault names. Partial by construction: a mount without a `vault:` leg has no
/// Obsidian vault, so [`MountSet::vault_of`] is `None` there and the stored
/// form refuses rather than inventing one.
///
/// The ALIAS axis maps a second lookup spelling onto a declared name
/// (`meridian-md-schema.md` § 5.1b), so a skill can spell ONE constant —
/// `sessions:` — that any machine's table maps to whatever it calls that root.
/// Lookup is **name first, then alias** ([`MountSet::canonical`]), which is what
/// makes "a name is its own alias" true with no special case. An alias is never a
/// STORED spelling: [`MountSet::vault_of`] and every canonical echo stay keyed by
/// name, so a caller canonicalizes through [`MountSet::canonical`] before asking
/// for one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MountSet {
    bound: Vec<MountName>,
    unreachable: Vec<UnreachableRoot>,
    vaults: Vec<(MountName, String)>,
    aliases: Vec<(MountName, MountName)>,
}

/// A root the mount table DECLARES but this machine cannot read — the path is
/// absent, unreadable, or holds no corpus. Carries the path: that is what its
/// refusal must name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreachableRoot {
    /// The canonical name the file declares.
    pub name: MountName,
    /// The path it is declared at — what a reader must go and check.
    pub path: String,
    /// The underlying reason, verbatim from the filesystem or the corpus build.
    pub detail: String,
}

impl MountSet {
    /// The set of bound names. Duplicates collapse; order is the caller's.
    #[must_use]
    pub fn new(bound: impl IntoIterator<Item = MountName>) -> Self {
        let mut names: Vec<MountName> = Vec::new();
        for name in bound {
            if !names.contains(&name) {
                names.push(name);
            }
        }
        MountSet {
            bound: names,
            unreachable: Vec::new(),
            aliases: Vec::new(),
            vaults: Vec::new(),
        }
    }

    /// Record the Obsidian vault name a bound root carries. Chainable.
    ///
    /// A later record for the same name REPLACES the earlier one (INV-3: a
    /// vault name is a key).
    #[must_use]
    pub fn with_vault(mut self, name: MountName, vault: impl Into<String>) -> Self {
        let vault = vault.into();
        self.vaults.retain(|(n, _)| n != &name);
        self.vaults.push((name, vault));
        self
    }

    /// The Obsidian vault name bound to `name`, for the STORED spelling.
    ///
    /// `None` when the root carries no vault name —
    /// and the stored form then refuses rather than inventing one.
    #[must_use]
    pub fn vault_of(&self, name: &MountName) -> Option<&str> {
        self.vaults
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    /// The canonical root name a stored form's vault name belongs to — the
    /// reverse of [`MountSet::vault_of`], and the read plane's whole question.
    ///
    /// `None` means this machine binds no root under that vault name, which is
    /// how the read plane tells an engine-minted stored form from an ordinary
    /// `obsidian://` link a human wrote to a vault meridian does not govern.
    #[must_use]
    pub fn name_of_vault(&self, vault: &str) -> Option<&MountName> {
        self.vaults.iter().find(|(_, v)| v == vault).map(|(n, _)| n)
    }

    /// Record a DECLARED root this machine cannot read, with the path to check
    /// and the underlying reason. Chainable.
    ///
    /// Such a name is not bound, but not undeclared either;
    /// [`MountSet::unreachable`] is how a resolver tells the two apart.
    #[must_use]
    pub fn with_unreachable(
        mut self,
        name: MountName,
        path: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        self.bound.retain(|n| n != &name);
        self.vaults.retain(|(n, _)| n != &name);
        self.unreachable.retain(|u| u.name != name);
        self.unreachable.push(UnreachableRoot {
            name,
            path: path.into(),
            detail: detail.into(),
        });
        self
    }

    /// Record the second spelling a declared root answers to. Chainable.
    ///
    /// A later record for the same alias REPLACES the earlier one — an alias is
    /// a key. The config plane refuses a table where one is ambiguous
    /// (`alias-shadows-name`) before this is ever reached, so the replacement is
    /// a type-level invariant, never a tie-break.
    #[must_use]
    pub fn with_alias(mut self, alias: MountName, name: MountName) -> Self {
        self.aliases.retain(|(a, _)| a != &alias);
        self.aliases.push((alias, name));
        self
    }

    /// The DECLARED name a lookup spelling reaches: **the spelling itself when
    /// some mount is named that, else the name an alias points at** (§ 5.1b).
    ///
    /// `None` when neither — which is what "this machine does not know that
    /// root" means, and the answer every refusal is built on. Name-first is not
    /// a preference between two candidates: `alias-shadows-name` means a table
    /// where both could answer cannot load.
    ///
    /// Declared, not bound: an alias onto an unreadable root answers here, so
    /// the refusal that follows can carry THAT root's state instead of claiming
    /// nobody declares the name. [`MountSet::is_bound`] is the readable half.
    #[must_use]
    pub fn canonical<'a>(&'a self, spelling: &'a MountName) -> Option<&'a MountName> {
        if self.bound.contains(spelling) || self.unreachable.iter().any(|u| &u.name == spelling) {
            return Some(spelling);
        }
        self.aliases
            .iter()
            .find(|(a, _)| a == spelling)
            .map(|(_, name)| name)
    }

    /// The alias bound to a canonical name — the reverse of the alias leg, for
    /// the faces that RENDER the table (`mounts` rows, `mrd config`).
    #[must_use]
    pub fn alias_of(&self, name: &MountName) -> Option<&MountName> {
        self.aliases.iter().find(|(_, n)| n == name).map(|(a, _)| a)
    }

    /// Does this machine bind `name` — under that name or an alias for it?
    ///
    /// Alias-aware by the § 5.1b lookup order. With no alias declared the
    /// answer is the plain membership test it has always been: the alias leg is
    /// empty, so nothing can be reached through it.
    #[must_use]
    pub fn is_bound(&self, name: &MountName) -> bool {
        self.bound.contains(name)
            || self
                .aliases
                .iter()
                .any(|(a, n)| a == name && self.bound.contains(n))
    }

    /// Every bound name, so a refusal can name what IS available.
    #[must_use]
    pub fn bound_names(&self) -> &[MountName] {
        &self.bound
    }

    /// Is `name` DECLARED but unreadable here? `None` when it is bound, or when
    /// nothing declares it at all.
    ///
    /// Alias-aware, like [`MountSet::is_bound`]: a `sessions:` whose alias
    /// points at a root this machine cannot read must reach that root's own
    /// state, whose prescribed fix is already done — not the undeclared refusal,
    /// which would prescribe declaring a mount that is already declared.
    #[must_use]
    pub fn unreachable(&self, name: &MountName) -> Option<&UnreachableRoot> {
        if let Some(direct) = self.unreachable.iter().find(|u| &u.name == name) {
            return Some(direct);
        }
        let target = self
            .aliases
            .iter()
            .find(|(a, _)| a == name)
            .map(|(_, n)| n)?;
        self.unreachable.iter().find(|u| &u.name == target)
    }

    /// Every declared-but-unreadable root.
    #[must_use]
    pub fn unreachable_roots(&self) -> &[UnreachableRoot] {
        &self.unreachable
    }

    /// Does the mount table DECLARE `name` at all — bound or not?
    ///
    /// The union of [`MountSet::is_bound`] and [`MountSet::unreachable`]: the
    /// question "is this name MINE to judge?" rather than "can I read it?".
    ///
    /// Not interchangeable with `is_bound`: external URIs parse as roots
    /// (`https://…` → root `https`), and `is_bound` is false for both an
    /// external scheme (leave verbatim) and a declared-but-unreadable root
    /// (must refuse). This question keeps the two apart.
    #[must_use]
    pub fn is_declared(&self, name: &MountName) -> bool {
        self.is_bound(name) || self.unreachable(name).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Acceptance rows D1-D4 of `docs/address-grammar.md` § 4.5.
    #[test]
    fn the_colon_law_accepts_rows_d1_through_d4() {
        // D1 — a rooted address.
        let d1 = Addr::parse("sessions:notes.md").expect("D1 parses");
        assert_eq!(d1.root().map(MountName::as_str), Some("sessions"));
        assert_eq!(d1.path(), "notes.md");

        // D2 — the ambient majority case, unchanged.
        let d2 = Addr::parse("notes.md").expect("D2 parses");
        assert_eq!(d2.root(), None);
        assert_eq!(d2.path(), "notes.md");

        // D3 — the colon FOLLOWS the first `/`, so it is an ordinary path byte.
        let d3 = Addr::parse("dir/a:b.md").expect("D3 parses");
        assert_eq!(d3.root(), None);
        assert_eq!(d3.path(), "dir/a:b.md");

        // D4 — root, subdirs and a selector together.
        let d4 = Addr::parse("sessions:24-01/notes.md#Design").expect("D4 parses");
        assert_eq!(d4.root().map(MountName::as_str), Some("sessions"));
        assert_eq!(d4.path(), "24-01/notes.md");
        assert_eq!(d4.selector(), "Design");
    }

    /// Rows D5-D9 — every refusal of § 4.5, each with its named class.
    #[test]
    fn the_colon_law_refuses_rows_d5_through_d9() {
        assert_eq!(
            Addr::parse("My Notes:draft.md"),
            Err(AddrError::BadMountName {
                found: "My Notes".to_string()
            }),
            "D5 — a space is outside `[a-z0-9-]`",
        );
        assert_eq!(
            Addr::parse("Sessions:notes.md"),
            Err(AddrError::BadMountName {
                found: "Sessions".to_string()
            }),
            "D6 — uppercase REFUSES; it is never silently lowercased (§ 4.3)",
        );
        assert_eq!(
            Addr::parse(":notes.md"),
            Err(AddrError::EmptyMountName),
            "D7 — a separator with no name before it",
        );
        assert_eq!(
            Addr::parse("sessions:"),
            Err(AddrError::EmptyPath),
            "D8 — a root alone addresses nothing",
        );
        assert_eq!(
            Addr::parse("a:b:c.md"),
            Err(AddrError::AmbiguousColon {
                found: "a:b:c.md".to_string()
            }),
            "D9 — exactly one colon may act as a separator",
        );
        assert_eq!(
            Addr::parse(""),
            Err(AddrError::EmptyPath),
            "the empty spelling addresses nothing",
        );
    }

    /// No fallback to the literal reading (§ 4.1): a typo'd root refuses
    /// rather than degrading into a literal-path lookup.
    #[test]
    fn a_typod_root_refuses_and_never_falls_back_to_a_literal_path() {
        // A well-formed name parses AS a root; the mount lookup decides the rest.
        let typo = Addr::parse("session:notes.md").expect("a well-formed name parses");
        assert_eq!(typo.root().map(MountName::as_str), Some("session"));
        assert_eq!(
            typo.path(),
            "notes.md",
            "the head is a root, never a literal path segment",
        );
        // A malformed name refuses outright rather than falling back.
        assert!(
            Addr::parse("Session:notes.md").is_err(),
            "no fallback reading exists for a malformed root name",
        );
    }

    /// `Addr.path` carries no root prefix, and no constructor can smuggle one in.
    #[test]
    fn the_path_field_never_carries_a_root_prefix() {
        for spelling in [
            "sessions:notes.md",
            "sessions:a/b.md#Design",
            "sessions:a/b.md#^claim@fp1.span2.b3.dead",
        ] {
            let addr = Addr::parse(spelling).expect("parses");
            let head = addr.path().split('/').next().unwrap_or(addr.path());
            assert!(
                !head.contains(':'),
                "{spelling}: the path head must carry no separator, got {head}",
            );
        }
        // `with_path` is the only other way to reach the field, and it re-checks.
        let addr = Addr::parse("sessions:notes.md").expect("parses");
        assert!(
            addr.with_path("other:notes.md").is_err(),
            "with_path must refuse a path that carries its own root",
        );
        assert_eq!(
            addr.with_path("deep/notes.md")
                .expect("a root-free path is accepted")
                .to_string(),
            "sessions:deep/notes.md",
            "root and selector ride through a path replacement",
        );
    }

    /// Lossless round-trip: every parsed spelling renders back byte-identically.
    #[test]
    fn every_parsed_spelling_renders_back_byte_identically() {
        for spelling in [
            "notes.md",
            "wiki/page.md",
            "wiki/page.md#Design",
            "wiki/page.md#^claim-1",
            "wiki/page.md#Design/Sub",
            "sessions:notes.md",
            "sessions:24-01-retro/notes.md#Design",
            "dir/a:b.md",
            "a@b.md",
            "page.md#^claim@fp1.span2.b3.dead",
            "page.md#",
            "a\"b",
        ] {
            let addr = Addr::parse(spelling).expect("parses");
            assert_eq!(
                addr.to_string(),
                spelling,
                "the canonical spelling must reproduce its input byte for byte",
            );
        }
    }

    /// Law A-2 (§ 4.4): the fragment is selector bytes to its end — `@`
    /// included. No `@fp` is split out of the name lane; a decorated spelling
    /// pasted back in keeps its whole fragment and misses byte-exact.
    #[test]
    fn the_fragment_is_selector_bytes_to_its_end() {
        let decorated =
            Addr::parse("guide.md#^claim@fp1.span2.b3.beef").expect("a decorated address parses");
        assert_eq!(
            decorated.selector(),
            "^claim@fp1.span2.b3.beef",
            "every fragment byte is selector bytes — nothing is peeled",
        );
        assert_eq!(
            decorated.target(),
            "guide.md",
            "the identity a corpus lookup addresses is `[root:]path`",
        );

        // The corpus motive: an `@`-bearing heading is addressable by its own
        // spelling, and never resolves as its `@`-truncated prefix.
        let real = Addr::parse("page.md#Deploy @ prod").expect("parses");
        assert_eq!(real.selector(), "Deploy @ prod");
    }

    /// S3-R38 — an `@` is an ordinary byte everywhere: in a bare path, in a
    /// rooted path, and (Law A-2) in a fragment.
    #[test]
    fn an_at_is_an_ordinary_byte_in_path_and_fragment_alike() {
        let bare = Addr::parse("mail@host.md").expect("a bare path carrying `@` parses");
        assert_eq!(bare.path(), "mail@host.md", "the `@` is an ordinary byte");
        assert_eq!(
            bare.to_string(),
            "mail@host.md",
            "and it renders back byte-identically — the file stays addressable",
        );
        assert_eq!(
            bare.target(),
            "mail@host.md",
            "the corpus lookup addresses the whole filename, `@` included",
        );

        // In a fragment the same text is selector bytes — there is no fp slot.
        let slotted = Addr::parse("mail.md#^claim@host").expect("parses");
        assert_eq!(
            slotted.selector(),
            "^claim@host",
            "the fragment runs to the end of the spelling",
        );

        // And a ROOTED bare path keeps the same reading.
        let rooted = Addr::parse("sessions:mail@host.md").expect("rooted, still a path byte");
        assert_eq!(rooted.path(), "mail@host.md");
        assert_eq!(rooted.to_string(), "sessions:mail@host.md");
    }

    /// `target()` is byte-identical to the pre-type `split_once('#')` result.
    #[test]
    fn target_matches_the_pre_type_split_at_every_seam() {
        for spelling in [
            "wiki/page.md",
            "wiki/page.md#Design",
            "sessions:notes.md#Design",
            "sessions:24-01-retro/notes.md#Design",
        ] {
            let addr = Addr::parse(spelling).expect("parses");
            let legacy = spelling.split_once('#').map_or(spelling, |(left, _)| left);
            assert_eq!(
                addr.target(),
                legacy,
                "{spelling}: the retype must not move a single byte at the projection",
            );
        }
    }

    /// `MountName` is the mount-table key, and its charset is the one rule.
    #[test]
    fn mount_name_charset_is_lowercase_alnum_and_dash() {
        assert!(MountName::parse("sessions").is_ok());
        assert!(MountName::parse("field-notes").is_ok());
        assert!(MountName::parse("wiki2").is_ok());
        assert_eq!(MountName::parse(""), Err(AddrError::EmptyMountName));
        for bad in ["Sessions", "home_wiki", "home wiki", "home.wiki", "wiki/"] {
            assert_eq!(
                MountName::parse(bad),
                Err(AddrError::BadMountName {
                    found: bad.to_string()
                }),
                "{bad} is outside `[a-z0-9-]`",
            );
        }
    }

    /// `MountSet` answers exactly the two questions a resolver asks.
    #[test]
    fn mount_set_answers_is_bound_and_names_what_is_available() {
        let sessions = MountName::parse("sessions").expect("a name");
        let assets = MountName::parse("assets").expect("a name");
        let set = MountSet::new([sessions.clone(), sessions.clone()]);
        assert!(set.is_bound(&sessions));
        assert!(
            !set.is_bound(&assets),
            "an unbound name is grey at resolution, never bound here",
        );
        assert_eq!(
            set.bound_names(),
            &[sessions],
            "duplicates collapse — the table is a set of names",
        );
        assert!(MountSet::default().bound_names().is_empty());
    }

    /// The vault axis, both directions; a root with no vault answers `None`.
    #[test]
    fn the_vault_axis_answers_both_directions_and_stays_partial() {
        let sessions = MountName::parse("sessions").expect("a name");
        let assets = MountName::parse("assets").expect("a name");
        let set = MountSet::new([sessions.clone(), assets.clone()])
            .with_vault(sessions.clone(), "field-notes-sessions");

        assert_eq!(set.vault_of(&sessions), Some("field-notes-sessions"));
        assert_eq!(set.name_of_vault("field-notes-sessions"), Some(&sessions));
        assert_eq!(
            set.vault_of(&assets),
            None,
            "a plain-folder root has no vault name, and the axis says so",
        );
        assert_eq!(
            set.name_of_vault("someone-elses-vault"),
            None,
            "a vault this machine does not bind is not the engine's to translate",
        );
        // INV-3: one root, one vault name — a second record replaces.
        let set = set.with_vault(sessions.clone(), "renamed-vault");
        assert_eq!(set.vault_of(&sessions), Some("renamed-vault"));
        assert_eq!(set.name_of_vault("field-notes-sessions"), None);
        // An unreachable root keeps no vault name.
        let set = set.with_unreachable(sessions.clone(), "/gone", "unreadable");
        assert_eq!(set.vault_of(&sessions), None);
    }

    /// [`head_carries_root_separator`] admits refusing spellings too — a
    /// parse-gated predicate would drop them.
    #[test]
    fn the_head_separator_test_admits_refusals_and_leaves_the_ambient_corpus_out() {
        for rooted in [
            "sessions:notes.md",       // well-formed
            "sessions:24-01/notes.md", // well-formed, subdirs
            "sessions:a/b.md#Design",  // well-formed, fragment
            "Sessions:notes.md",       // REFUSES — BadMountName
            "My Notes:draft.md",       // REFUSES — BadMountName
            "a:b:c.md",                // REFUSES — AmbiguousColon
            ":notes.md",               // REFUSES — EmptyMountName
            "sessions:",               // REFUSES — EmptyPath
            "  sessions:notes.md",     // leading space — the resolver trims
        ] {
            assert!(
                head_carries_root_separator(rooted),
                "{rooted}: the address plane has an answer here, so the head test must admit it",
            );
        }
        // Acceptance half — a test that admits everything gates nothing.
        for ambient in [
            "notes.md",
            "a/b/c.md",
            "dir/a:b.md",   // a `:` AFTER the first `/` is a path byte (§ 4.1)
            "page.md#a:b",  // a `:` inside the FRAGMENT is not a root separator
            "mail@host.md", // S3-R38
            "",
        ] {
            assert!(
                !head_carries_root_separator(ambient),
                "{ambient}: ambient spellings must stay out",
            );
        }
    }

    /// [`head_names_declared_root`] narrows the wide head test to the table's
    /// own names: declared heads answer `true` — a malformed remainder after a
    /// declared name included, that refusal is the address plane's — while
    /// external schemes, undeclared roots, uncharsetted names, and the whole
    /// ambient corpus stay out.
    #[test]
    fn the_declared_root_test_admits_the_tables_names_and_nothing_else() {
        let sessions = MountName::parse("sessions").unwrap();
        let vault = MountName::parse("vault").unwrap();
        let set = MountSet::new([sessions]).with_unreachable(vault, "/gone", "unreadable");
        for declared in [
            "sessions:notes.md",       // bound, well-formed
            "sessions:24-01/notes.md", // bound, subdirs
            "sessions:a/b.md#Design",  // bound, fragment
            "sessions:",               // bound name, EmptyPath — the refusal is the plane's
            "sessions:a:b.md",         // bound name, AmbiguousColon — likewise
            "vault:notes.md",          // declared-but-unreachable MUST refuse, never dangle
            "  sessions:notes.md",     // leading space — the resolver trims
        ] {
            assert!(
                head_names_declared_root(declared, &set),
                "{declared}: the table declares this head, the address plane owns the answer",
            );
        }
        for outside in [
            "https://example.com", // external scheme — the URL-shaped wikilink
            "meridian://x/y",      // external scheme, engine-flavored
            "mailto:someone@host", // external scheme, no `//`
            "elsewhere:notes.md",  // rooted spelling, nobody declares it
            "Sessions:notes.md",   // uncharsetted name can never be declared
            ":notes.md",           // EmptyMountName
            "notes.md",            // ambient
            "dir/a:b.md",          // a `:` AFTER the first `/` is a path byte (§ 4.1)
            "page.md#a:b",         // a `:` inside the FRAGMENT is not a root separator
            "",
        ] {
            assert!(
                !head_names_declared_root(outside, &set),
                "{outside}: no declared root in the head, the ambient answer stands",
            );
        }
    }

    /// Every spelling the head test admits has an address-plane answer: a
    /// well-formed one parses ROOTED, a refused one is an `AddrError`.
    #[test]
    fn everything_the_head_test_admits_has_an_address_plane_answer() {
        for spelling in ["sessions:notes.md", "Sessions:notes.md", "a:b:c.md"] {
            assert!(head_carries_root_separator(spelling));
            // An Err arm needs no assertion: a refusal IS an address-plane answer.
            if let Ok(addr) = Addr::parse(spelling) {
                assert!(
                    addr.root().is_some(),
                    "{spelling}: admitted and parsed, so it must be ROOTED",
                );
            }
        }
    }

    /// [`confined`] rejects escapes and admits the ordinary corpus.
    #[test]
    fn confinement_rejects_escapes_and_admits_the_ordinary_corpus() {
        for ok in [
            "notes.md",
            "a/b/c.md",
            "dir/a:b.md",        // a `:` AFTER the first `/` is a path byte (§ 4.1)
            "mail@host.md",      // S3-R38 — an `@` in a bare path is a path byte
            ".github/README.md", // dot-DIRS are addressable; only `.` alone is not
        ] {
            assert!(confined(ok), "{ok} is an ordinary corpus path");
        }
        for bad in [
            "",
            "/etc/passwd",
            "../victim.md",
            "a/../../b.md",
            "./a.md",
            "a//b.md",
            "sessions:notes.md", // an ADDRESS, never a path (§ 4.2, D11)
            "sessions:",
        ] {
            assert!(!confined(bad), "{bad} must not be admitted as a path");
        }
    }

    /// Every refusal names what it refused and points at the law.
    #[test]
    fn every_refusal_names_the_offending_text_and_teaches_the_fix() {
        for (spelling, needle) in [
            ("My Notes:draft.md", "My Notes"),
            ("a:b:c.md", "a:b:c.md"),
            (":notes.md", "root separator"),
            ("sessions:", "names no path"),
        ] {
            let err = Addr::parse(spelling).expect_err("refuses");
            let text = err.to_string();
            assert!(
                text.contains(needle),
                "{spelling}: the refusal must name '{needle}', got: {text}",
            );
            assert!(
                text.contains("[[address-grammar]]"),
                "{spelling}: every refusal points at the law it enforces",
            );
        }
    }

    fn name(s: &str) -> MountName {
        MountName::parse(s).expect("a canonical name")
    }

    /// § 5.1b's lookup order, both halves: a bound name answers under its own
    /// spelling AND under an alias declared for it, and the canonical answer is
    /// the NAME whichever spelling asked — which is what keeps aliases out of
    /// every stored form.
    #[test]
    fn a_root_answers_under_its_name_and_under_its_alias() {
        let real = name("field-notes-sessions");
        let spelled = name("sessions");
        let set = MountSet::new([real.clone()])
            .with_alias(spelled.clone(), real.clone())
            .with_vault(real.clone(), "field-notes-sessions");

        assert!(set.is_bound(&real), "the name binds");
        assert!(set.is_bound(&spelled), "the alias binds the same root");
        assert_eq!(
            set.canonical(&spelled),
            Some(&real),
            "the alias canonicalizes to the name"
        );
        assert_eq!(set.canonical(&real), Some(&real), "a name is its own alias");
        assert_eq!(set.alias_of(&real), Some(&spelled));

        // The stored plane is keyed by NAME: asking with the alias directly
        // finds nothing, which is why callers canonicalize first.
        assert_eq!(set.vault_of(&real), Some("field-notes-sessions"));
        assert_eq!(set.vault_of(&spelled), None);
    }

    /// A name is its own alias with NO special case: a root actually NAMED
    /// `sessions` answers `sessions:` on a table that declares no aliases at
    /// all, and the alias leg being empty leaves every answer as it was.
    #[test]
    fn a_name_needs_no_alias_line() {
        let set = MountSet::new([name("sessions")]);
        assert!(set.is_bound(&name("sessions")));
        assert_eq!(set.canonical(&name("sessions")), Some(&name("sessions")));
        assert_eq!(set.alias_of(&name("sessions")), None);
        assert_eq!(
            set.canonical(&name("nothing")),
            None,
            "an undeclared name is undeclared"
        );
        assert!(!set.is_bound(&name("nothing")));
    }

    /// An alias onto a DECLARED-but-unreadable root reaches that root's own
    /// state, not the undeclared answer — the two causes take opposite remedies,
    /// and the unreachable one's prescribed fix ("declare the mount") is already
    /// done.
    #[test]
    fn an_alias_onto_an_unreachable_root_carries_that_roots_state() {
        let real = name("field-notes-sessions");
        let spelled = name("sessions");
        let set = MountSet::new([])
            .with_alias(spelled.clone(), real.clone())
            .with_unreachable(real.clone(), "/gone", "the path is absent");

        assert!(!set.is_bound(&spelled), "unreadable is not bound");
        assert!(set.is_declared(&spelled), "but it IS declared");
        assert_eq!(
            set.unreachable(&spelled).map(|u| u.path.as_str()),
            Some("/gone"),
            "the alias reaches the declaring root's own state",
        );
        assert_eq!(set.canonical(&spelled), Some(&real));
    }
}
