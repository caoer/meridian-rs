//! The agent-plane address, as a fallible TYPE.
//!
//! `[root:]path[#selector][@fp]` — parsed once, here, so no plane downstream
//! re-splits a string. A `std`-only leaf with zero dependencies, placed
//! UPSTREAM of `syntax` (`docs/address-grammar.md` § 7.1): `model` depends on
//! `syntax`, so an address type living beside the markdown parsing would be
//! architecturally unreachable from the wikilink ingress where a cross-root
//! address actually arrives.
//!
//! **Construction is the only way in.** [`Addr::parse`] is the sole
//! constructor; there is no `from_parts` a caller can use to smuggle an
//! unparsed root prefix into the [`Addr::path`] field. That invariant —
//! *`Addr.path` carries no root prefix* — is what makes every downstream guard
//! checkable, and it is why the type is fallible rather than a boolean helper a
//! caller may ignore (R5).
//!
//! **Parse is not resolve.** [`Addr::parse`] answers *"is this a well-formed
//! address?"*; it never touches the mount table. Whether the named root is
//! BOUND is the resolver's question and its answer is grey, not a parse error
//! (`docs/address-grammar.md` § 2.2, § 6). Conflating the two would make a
//! well-formed address to an unmounted root indistinguishable from a malformed
//! one.

use core::fmt;

pub mod stored;

/// A canonical root NAME — the mount-table key a cross-root address carries
/// (`sessions`, `assets`).
///
/// Deliberately named `MountName`, never `Root`: three senses of "root" already
/// meet in this engine and a fourth shares the spelling
/// (`docs/address-grammar.md` § 1). This is neither a Merkle cursor
/// (`wire::Root`) nor a directory (`fs::WorkspaceRoot`) nor the `root:`
/// frontmatter key of the preset grammar — and the word **Name** says so at
/// every call site.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MountName(String);

impl MountName {
    /// The one charset: `[a-z0-9-]`, non-empty (`docs/address-grammar.md`
    /// § 4.3).
    ///
    /// **An uppercase byte REFUSES rather than being silently normalized.** A
    /// silent normalization would make two spellings one name, and the ratified
    /// law is one name per root used everywhere. The corpus index already
    /// lowercases its own keys, so a normalizing `MountName` would be a second,
    /// invisible case rule on the same address.
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
/// refused rather than reporting that something, somewhere, was malformed.
///
/// One variant here is NOT reachable from [`Addr::parse`]:
/// [`AddrError::SelectorOnOpaqueRoot`] is a RESOLUTION-time refusal, because a
/// root's *kind* is a mount-table fact and `parse` does not read the mount table
/// (§ 2.2). Its constructor lives with the resolver — `model`'s
/// [`resolve_ref`](../model/struct.CorpusIndex.html) — and U11 ships the two
/// together. A variant with no constructor would be S3-R23(4)'s forbidden
/// weakened middle: a claim nothing checks.
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
    /// Two or more colons in the head (`a:b:c.md`). **Exactly one colon may act
    /// as a separator**, and a second is refused rather than resolved by a
    /// precedence rule no reader could recover from the spelling.
    AmbiguousColon {
        /// The offending head, verbatim.
        found: String,
    },
    /// A `#selector` on an address naming an OPAQUE root — one whose kind has no
    /// parse and no sections (`git-folder`). `assets:media/logo.png` is legal;
    /// `assets:media/logo.png#Design` is refused (§ 10.1, G-1).
    ///
    /// **Resolution-time, never parse-time**: the root's kind is a mount-table
    /// fact, so only the resolver can raise this. Pin grain for such a root is
    /// the file, and the fingerprint is a raw CID of the bytes.
    SelectorOnOpaqueRoot {
        /// The opaque root the address named.
        root: MountName,
        /// That root's kind word, as the mount table spells it (`git-folder`).
        kind: String,
        /// The refused selector, verbatim.
        selector: String,
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
            AddrError::SelectorOnOpaqueRoot {
                root,
                kind,
                selector,
            } => write!(
                f,
                "refused: root '{root}' is a {kind} root — it has no parse and no sections, \
                 so the selector '{selector}' addresses nothing. \
                 Fix: address the file itself (`{root}:path`) — pin grain for a {kind} root is \
                 the file; see [[address-grammar]]."
            ),
        }
    }
}

impl std::error::Error for AddrError {}

/// The agent-plane address `[root:]path[#selector][@fp]`, parsed.
///
/// Construct with [`Addr::parse`] — the sole constructor. Round-trips
/// byte-identically through [`Display`](fmt::Display), which is what lets an
/// address be a type at a seam that must still write the spelling it read.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Addr {
    root: Option<MountName>,
    path: String,
    selector: Option<String>,
    fp: Option<String>,
}

impl Addr {
    /// Parse an agent-plane address. **The sole constructor.**
    ///
    /// The colon law (`docs/address-grammar.md` § 4.1), over the **head** — the
    /// text before the first `/` and before the first `#`:
    ///
    /// - **zero `:`** → no root; the address resolves in the ambient root. The
    ///   overwhelming majority of refs, unchanged;
    /// - **exactly one `:`** → that colon IS the root separator. The text
    ///   before it must be a well-formed [`MountName`], the text after it
    ///   non-empty. If either fails the address is REFUSED — **never**
    ///   reinterpreted as a literal path;
    /// - **two or more `:`** → refused.
    ///
    /// After the first `/`, a `:` is an ordinary path byte.
    ///
    /// **Why no fallback to the literal reading.** A fallback would mean a
    /// typo'd root (`session:` for `sessions:`) silently degrades into a lookup
    /// for a file literally named `session:notes.md` — a wrong SUCCESS of
    /// exactly FINDING 03's shape. Refusing is the only outcome that cannot be
    /// mistaken for working.
    ///
    /// The `@fp` suffix is recognized **inside the fragment only** (after the
    /// first `#`), which is the position the render face mints it in and the
    /// only position where it could otherwise be absorbed into the selector
    /// (§ 4.4). An `@` in a bare path stays a path byte, so a filename carrying
    /// one is addressable.
    ///
    /// # Errors
    ///
    /// The closed set of [`AddrError`].
    pub fn parse(spelling: &str) -> Result<Self, AddrError> {
        let (before_frag, frag) = match spelling.split_once('#') {
            Some((left, frag)) => (left, Some(frag)),
            None => (spelling, None),
        };

        // The head is the first path segment of the pre-fragment text: a `:`
        // after the first `/` is an ordinary path byte and carries no meaning.
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

        let (selector, fp) = match frag {
            None => (None, None),
            Some(frag) => match frag.split_once('@') {
                Some((sel, fp)) => (Some(sel.to_string()), Some(fp.to_string())),
                None => (Some(frag.to_string()), None),
            },
        };

        Ok(Addr {
            root,
            path,
            selector,
            fp,
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

    /// The selector, verbatim after the first `#` with the caret kept. `""`
    /// means the page/doc root, matching the shipped `node.selector` column.
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

    /// The `@fp` render-face decoration, if the spelling carried one.
    ///
    /// **Never part of the identity a corpus lookup uses** — resolution reads
    /// the fp-free projection ([`Addr::target`]). The fp is minted on read and
    /// never stored (§ 4.4).
    #[must_use]
    pub fn fp(&self) -> Option<&str> {
        self.fp.as_deref()
    }

    /// The address minus its selector and fp: `[root:]path`.
    ///
    /// This is the identity a corpus lookup addresses, and it is byte-identical
    /// to what a pre-type `split_once('#')` produced at every seam — the root
    /// stays ON the spelling here rather than being peeled into a field nothing
    /// downstream reads yet. Peeling it before the resolver is root-aware is
    /// Story A's discard defect (FINDING 02); U11 owns the root-keyed lookup.
    #[must_use]
    pub fn target(&self) -> String {
        match &self.root {
            Some(root) => format!("{root}:{}", self.path),
            None => self.path.clone(),
        }
    }

    /// This address with its path replaced — the resolution seam, and it is
    /// **fallible on purpose**.
    ///
    /// A resolved corpus path must itself carry no root separator in its head,
    /// or the invariant [`Addr::parse`] establishes would be smuggled around by
    /// a caller holding an already-valid `Addr`. Root, selector and fp ride
    /// through unchanged.
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
            fp: self.fp.clone(),
        })
    }
}

impl fmt::Display for Addr {
    /// The canonical spelling — **byte-identical to what was parsed.**
    ///
    /// Lossless rendering is not a convenience: `lock` proves byte round-trip
    /// of its fenced block and the view projection writes `declared_ref`
    /// verbatim, so a type that could not reproduce its input would not be
    /// admissible at either seam.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(root) = &self.root {
            write!(f, "{root}:")?;
        }
        f.write_str(&self.path)?;
        if let Some(sel) = &self.selector {
            write!(f, "#{sel}")?;
        }
        if let Some(fp) = &self.fp {
            write!(f, "@{fp}")?;
        }
        Ok(())
    }
}

/// **THE reason word for a root that is DECLARED but cannot be read here — the
/// ONE source, in the BARE form (S3-R49).**
///
/// It lives in this leaf because TWO planes print it and a second literal is how
/// one state grows two spellings:
///
/// - the **mount** plane (`config::mount::MountState::PathUnseeable`) wraps it as
///   `grey(path-unseeable)` — the form `mrd config` prints;
/// - the **address** plane (`model::selector` / `view::walk`) takes it BARE, the
///   form `color_reason` returns, and lets its own renderer wrap.
///
/// **Reused, never minted.** U11 round 2 was ruled a NEW word
/// (`root-unreachable`) and the enumeration this unit was obliged to run found
/// that U7 already shipped this one for the same observed state, citing the same
/// M6 row, with a teaching that already names the path. Minting a synonym would
/// have been exactly the cross-crate re-spelling S3-R6 forbids.
///
/// **The wrapping is derived, not duplicated:** each plane's wrapped spelling is
/// checked against this const at COMPILE TIME, so the two agree by construction
/// rather than by two literals that happen to match today.
pub const PATH_UNSEEABLE_REASON_WORD: &str = "path-unseeable";

/// **The ONE lexical confinement predicate for a corpus-relative path.**
///
/// A path is confined when it is non-empty, does not start with `/`, has no
/// empty / `.` / `..` segment, and **carries no root separator in its head**.
///
/// Confinement used to be an ambient property: every path was joined onto the
/// ONE `fs::WorkspaceRoot`, so a lexical check was the whole story. **Multi-root
/// removes that ambient guarantee** — a `root:` prefix selects WHICH tree a path
/// is joined onto — so the head-colon arm is part of the predicate rather than a
/// separate address check: a `root:`-bearing spelling is an ADDRESS, and an
/// address is never a corpus path (§ 4.2, D11).
///
/// It lives here, in the `std`-only leaf, because two planes ask it — the write
/// doors in `wire-serve` and the resolver in `model` — and a second
/// implementation of a confinement fact is how one question grows two answers.
///
/// **Lexical only, by design.** It does not touch the filesystem, so it cannot
/// see a symlink that escapes; canonicalize-at-bind (§ 8 B-1) is what covers
/// that, and the two are complementary rather than alternatives.
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
    // The head is the first segment: a `:` after the first `/` is an ordinary
    // path byte (§ 4.1) and must not make an otherwise-fine path unwritable.
    !path.split('/').next().unwrap_or(path).contains(':')
}

/// The resolution-facing projection of the mount table: which canonical names
/// this machine binds.
///
/// Constructed by `config` (which owns the `MERIDIAN.md` parse and the
/// name ↔ vault-name ↔ path table) and consumed by `model` — it lives here, in
/// the upstream leaf, so `model::CorpusIndex::resolve_ref` can name it without
/// depending on `config`, which is downstream of `model`
/// (`docs/address-grammar.md` § 7.2).
///
/// **A concrete type, not a trait.** A trait invites a second implementation,
/// and a second implementation of an address fact is the exact defect D12 keeps
/// producing: one question, two answers.
///
/// The surface answers the three questions a resolver asks — *is this name
/// bound*, *which names are bound* (so a refusal can teach), and — S3-R43 —
/// *is this name DECLARED but unreadable here*, which is a different fact from
/// both and needs its own answer.
///
/// **Why the third question exists (S3-R43).** Round 1 carried only "bound or
/// not", so a root the file DECLARES but this machine cannot read was
/// indistinguishable from one nobody ever declared. The refusal for the second
/// says *"declare it in `~/MERIDIAN.md`"* — on the first that sentence is FALSE
/// and the prescribed fix is ALREADY DONE. A teaching refusal that prescribes a
/// completed action is worse than a bare class: it spends the user's trust and
/// their time and leaves no signal pointing at the real cause.
///
/// **The vault axis (U12).** The three-way map is *name ↔ vault name ↔ path*,
/// and the STORED plane is spelled in vault names
/// (`2026-07-24-cross-root-addressing.md` §2). So the same projection answers
/// the translation's two questions — *which vault name does this root have* and
/// *which root does this vault name belong to* — rather than a second
/// projection type growing beside this one for one axis of one map. The axis is
/// **partial by construction**: a `git-folder` root has no Obsidian vault, so
/// [`MountSet::vault_of`] is `None` there and the stored form refuses rather
/// than inventing a vault name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MountSet {
    bound: Vec<MountName>,
    unreachable: Vec<UnreachableRoot>,
    vaults: Vec<(MountName, String)>,
}

/// A root the mount table DECLARES but this machine cannot read — the path is
/// absent, unreadable, or holds no corpus.
///
/// Carries the PATH, because that is what its refusal must name: the mount entry
/// is already correct, so naming the entry teaches nothing.
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
            vaults: Vec::new(),
        }
    }

    /// Record the Obsidian vault name a bound root carries — the vault axis of
    /// the three-way map. Chainable.
    ///
    /// A later record for the same name REPLACES the earlier one rather than
    /// shadowing it: the mount table enforces INV-3 (a vault name is a key), so
    /// two answers for one root cannot both be right and keeping both would let
    /// a lookup order decide which stored form a link gets.
    #[must_use]
    pub fn with_vault(mut self, name: MountName, vault: impl Into<String>) -> Self {
        let vault = vault.into();
        self.vaults.retain(|(n, _)| n != &name);
        self.vaults.push((name, vault));
        self
    }

    /// The Obsidian vault name bound to `name`, for the STORED spelling.
    ///
    /// `None` when the root carries no vault — a `git-folder` root has none —
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
    /// Such a name is **not** bound — [`MountSet::is_bound`] stays false — but it
    /// is not undeclared either, and [`MountSet::unreachable`] is how a resolver
    /// tells the two apart.
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

    /// Does this machine bind `name`?
    #[must_use]
    pub fn is_bound(&self, name: &MountName) -> bool {
        self.bound.contains(name)
    }

    /// Every bound name, so a refusal can name what IS available.
    #[must_use]
    pub fn bound_names(&self) -> &[MountName] {
        &self.bound
    }

    /// Is `name` DECLARED but unreadable here? `None` when it is bound, or when
    /// nothing declares it at all.
    #[must_use]
    pub fn unreachable(&self, name: &MountName) -> Option<&UnreachableRoot> {
        self.unreachable.iter().find(|u| &u.name == name)
    }

    /// Every declared-but-unreadable root.
    #[must_use]
    pub fn unreachable_roots(&self) -> &[UnreachableRoot] {
        &self.unreachable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The acceptance half, asserted in the same breath as the refusals
    /// (S3-R8(c)): a grammar proven only by what it refuses is
    /// indistinguishable from one that refuses everything. Rows D1-D4 of
    /// `docs/address-grammar.md` § 4.5 — and D2/D3 are the ones that keep this
    /// law from swallowing the ordinary corpus.
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

    /// There is NO fallback to the literal reading — the property § 4.1 exists
    /// to establish. A typo'd root must refuse rather than degrade into a
    /// lookup for a file literally named `session:notes.md`.
    #[test]
    fn a_typod_root_refuses_and_never_falls_back_to_a_literal_path() {
        // Well-formed name, so it parses AS a root — the mount lookup (U11)
        // decides the rest. It must NOT become a literal path.
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

    /// The invariant every downstream guard rests on: `Addr.path` carries no
    /// root prefix, and there is no constructor that can smuggle one in.
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

    /// Lossless round-trip — the property that makes the type admissible at the
    /// `lock` byte-round-trip seam and the view projection's verbatim column.
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

    /// `@fp` is recognized in the fragment and kept off the identity, and an
    /// `@` in a bare path stays a path byte (§ 4.4).
    #[test]
    fn the_fp_decoration_is_recorded_and_never_part_of_the_identity() {
        let decorated =
            Addr::parse("guide.md#^claim@fp1.span2.b3.beef").expect("a decorated address parses");
        assert_eq!(decorated.selector(), "^claim", "the fp never bleeds in");
        assert_eq!(decorated.fp(), Some("fp1.span2.b3.beef"));
        assert_eq!(
            decorated.target(),
            "guide.md",
            "resolution reads the fp-free projection",
        );

        let at_in_path = Addr::parse("mail@host.md").expect("an `@` in a bare path parses");
        assert_eq!(at_in_path.path(), "mail@host.md");
        assert_eq!(at_in_path.fp(), None, "a bare path's `@` is a path byte");
    }

    /// **S3-R38 — an `@` in a BARE path is a PATH BYTE, so `mail@host.md` stays
    /// addressable.** Ratified, and pinned HERE rather than remembered: before
    /// this test the ruling lived in prose on [`Addr::parse`], and a grammar
    /// decision living only in a doc comment is one refactor from being reversed
    /// by someone who never read it.
    ///
    /// **Why it holds rather than merely being reasonable.** The `@fp` token's
    /// slot is defined by the BLOCK-REF POSITION, and a block ref requires a
    /// `#`. **A bare path has no such slot, so an `@` there cannot be an fp BY
    /// CONSTRUCTION** — not by convention, not by a rule someone must remember.
    /// The constraint is SLOT plus SHAPE, and a bare path fails the slot test
    /// structurally. The `#`-bearing sibling below is what makes that concrete:
    /// the SAME `@host` text is an fp once a fragment opens the slot.
    #[test]
    fn an_at_in_a_bare_path_is_a_path_byte_and_round_trips() {
        let bare = Addr::parse("mail@host.md").expect("a bare path carrying `@` parses");
        assert_eq!(bare.path(), "mail@host.md", "the `@` is an ordinary byte");
        assert_eq!(bare.fp(), None, "no `#`, no slot, therefore no fp");
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

        // The SLOT, opened: the same `@host` text after a `#` IS an fp. Without
        // this half the assertion above is equally satisfied by a parser that
        // never recognizes an fp anywhere.
        let slotted = Addr::parse("mail.md#^claim@host").expect("a block ref opens the fp slot");
        assert_eq!(slotted.selector(), "^claim");
        assert_eq!(
            slotted.fp(),
            Some("host"),
            "the block-ref position is what defines the fp slot",
        );

        // And a ROOTED bare path keeps the same reading.
        let rooted = Addr::parse("sessions:mail@host.md").expect("rooted, still a path byte");
        assert_eq!(rooted.path(), "mail@host.md");
        assert_eq!(rooted.fp(), None);
        assert_eq!(rooted.to_string(), "sessions:mail@host.md");
    }

    /// `target()` is byte-identical to the pre-type `split_once('#')` result —
    /// the property that keeps U10 behaviour-neutral at every projection seam
    /// and leaves the root-aware lookup to U11.
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

    /// The vault axis, both directions, with its PARTIALITY asserted: a root
    /// carrying no vault name answers `None` rather than a guess, which is what
    /// makes the stored form refuse instead of inventing one.
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
            "a git-folder root has no vault name, and the axis says so",
        );
        assert_eq!(
            set.name_of_vault("someone-elses-vault"),
            None,
            "a vault this machine does not bind is not the engine's to translate",
        );
        // INV-3 read from this side: one root, one vault name — a second record
        // REPLACES rather than shadowing.
        let set = set.with_vault(sessions.clone(), "renamed-vault");
        assert_eq!(set.vault_of(&sessions), Some("renamed-vault"));
        assert_eq!(set.name_of_vault("field-notes-sessions"), None);
        // An unreachable root keeps no vault name: it is not bound, so it has
        // no stored spelling either.
        let set = set.with_unreachable(sessions.clone(), "/gone", "unreadable");
        assert_eq!(set.vault_of(&sessions), None);
    }

    /// [`confined`] — the one lexical confinement law, with its ACCEPTANCE half
    /// asserted in the same breath (S3-R8(c)): a predicate proven only by what
    /// it rejects is indistinguishable from one that rejects everything.
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

    /// The resolution-time refusal U11 constructs (§ 10.1, G-1): a `#selector`
    /// on an address into an OPAQUE root, which has no parse and no sections.
    /// The variant ships WITH its constructor — an unconstructed variant would
    /// be S3-R23(4)'s forbidden weakened middle.
    #[test]
    fn the_opaque_root_refusal_names_the_root_its_kind_and_the_selector() {
        let err = AddrError::SelectorOnOpaqueRoot {
            root: MountName::parse("assets").expect("a name"),
            kind: "git-folder".to_string(),
            selector: "Design".to_string(),
        };
        let text = err.to_string();
        for needle in ["assets", "git-folder", "Design", "[[address-grammar]]"] {
            assert!(
                text.contains(needle),
                "the refusal must name {needle}: {text}"
            );
        }
    }

    /// Every refusal names what it refused — a refusal that cannot teach is the
    /// defect D8 exists to prevent.
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
}
