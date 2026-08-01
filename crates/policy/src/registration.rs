//! Tag-indexed rule registration — discovery, id grammar, override resolution.
//!
//! # What registers (ruling § 1)
//! A page registers as a rule by carrying a **registration tag** in its
//! frontmatter `tags:` — [`RuleKind::Hook`] is `rules/hook`, [`RuleKind::Check`] is
//! `rules/check`. `rules/` is the registration NAMESPACE: any page in the
//! workspace carrying a `rules/*` tag is discoverable, and a `rules/*` tag this
//! slice does not carry is a NAMED deferral ([`RegisterError::KindDeferred`]),
//! never a silent drop.
//!
//! Registration replaces the filename: nothing here reads a folder name, a
//! `CHECK.md`/`HOOK.md` spelling, or the corpus's legacy `kind:` frontmatter. Two
//! names for one thing is the defect being removed, so `kind:` is **not consulted
//! by this layer at all**.
//!
//! Tags are read from FRONTMATTER only — "frontmatter is for filtering, body is
//! for reading". An inline `#rules/hook` in prose does not register a page.
//!
//! # Identity is a frontmatter query (ruling § 2)
//! Identity lives in frontmatter `id:`, grammar
//! `^[a-z0-9][a-z0-9-]*(\.[a-z0-9][a-z0-9-]*)*$`, at most [`MAX_ID_LEN`] bytes. A
//! registration-tagged page without a valid `id:` fails loudly. The fenced
//! Starlark block MAY restate the id as a top-level `id = "…"` assignment; if it
//! does, it must agree with the frontmatter, and it is read STATICALLY from the
//! parsed AST — never evaluated. Needing to run a block to learn its identity
//! inverts the layering.
//!
//! # Registration is not arming (the cap-escape guardrail)
//! Nothing in this module arms anything, mints a receipt, spends a cap, or
//! touches disk. Discovery makes a page KNOWN; only the explicit attested ARM act
//! activates it. A tag that armed by itself would let any writer with put access
//! self-arm reactive code under caps. [`RuleIndex::resolve`] is a pure function of
//! its input, so a read-only consumer (`mrd rules`) and the arming path call the
//! SAME resolver — never a parallel computation.
//!
//! # Override resolution (ruling § 3)
//! Precedence is the scope ladder, outermost to innermost: **user space** →
//! **workspace root** → **folder/session tree**. Among pages sharing an id the
//! deepest mount governs — layer first, then directory depth, and NOTHING else.
//! There is no lexical and no mtime tiebreak; mtime is not even an input to this
//! module. Two pages tied at the winning scope are a loud refusal: that id
//! resolves to nothing, the refusal names both pages, and every other id is
//! unaffected.
//!
//! # The chain is retained, never collapsed (ruling § 7)
//! Resolution keeps EVERY candidate for an id, not just the winner, so the
//! override chain is printable winner-first in ladder order — the
//! `git config --show-origin` spirit. A resolver that discarded losers would make
//! the chain unreconstructable, so retention is a property of this layer rather
//! than of its consumer.
//!
//! # Where the walk lives
//! `policy` is the WHEN plane and is I/O-free (docs/laws.md), so this module never
//! enumerates a directory. The caller's walk offers [`PageRef`]s exactly as the
//! convention loader takes an injected [`crate::ConventionFiles`]; siting the disk
//! edge is the cutover card's job.

use std::borrow::Borrow;
use std::collections::BTreeMap;

use starlark::syntax::ast::{AssignTarget, AstLiteral, AstStmt, Expr, Stmt};
use starlark::syntax::{AstModule, Dialect};

/// The registration tag namespace. A page carrying `rules/<kind>` in its
/// frontmatter `tags:` offers itself to registration.
pub const REGISTRATION_NAMESPACE: &str = "rules/";

/// The frontmatter key identity lives under, and the Starlark name a block may
/// restate it with.
pub const ID_KEY: &str = "id";

/// The id length ceiling (ruling § 2) — bytes, over the whole dotted id.
pub const MAX_ID_LEN: usize = 64;

// ── Registration kind ─────────────────────────────────────────────────────────

/// Which rule plane a page registers into. The tag suffix after
/// [`REGISTRATION_NAMESPACE`] IS the kind, so the page's identity never couples to
/// the engine's name (`meridian/hook` was rejected for exactly that coupling).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RuleKind {
    /// `rules/check` — the law leg. A check may refuse a write.
    Check,
    /// `rules/hook` — the reaction leg. A hook may never veto or mutate.
    Hook,
}

impl RuleKind {
    /// The tag suffix this kind is spelled with.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            RuleKind::Check => "check",
            RuleKind::Hook => "hook",
        }
    }

    /// The full registration tag (`rules/check` / `rules/hook`).
    #[must_use]
    pub fn tag(self) -> String {
        format!("{REGISTRATION_NAMESPACE}{}", self.as_str())
    }

    /// The kind a tag suffix names, or `None` when this slice does not carry it.
    fn from_suffix(suffix: &str) -> Option<Self> {
        match suffix {
            "check" => Some(RuleKind::Check),
            "hook" => Some(RuleKind::Hook),
            _ => None,
        }
    }
}

// ── The id grammar ────────────────────────────────────────────────────────────

/// A frontmatter `id:` that passed the § 2 grammar. Construction is sealed to
/// [`RuleId::parse`], so an id in hand is a valid id.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuleId(String);

impl RuleId {
    /// Parse an id against the § 2 grammar:
    /// `^[a-z0-9][a-z0-9-]*(\.[a-z0-9][a-z0-9-]*)*$`, at most [`MAX_ID_LEN`] bytes.
    ///
    /// # Errors
    /// [`IdFault`] naming which rule broke and where.
    pub fn parse(id: &str) -> Result<Self, IdFault> {
        if id.is_empty() {
            return Err(IdFault::Empty);
        }
        if id.len() > MAX_ID_LEN {
            return Err(IdFault::TooLong { len: id.len() });
        }
        for (position, segment) in id.split('.').enumerate() {
            let mut chars = segment.chars();
            // The grammar's head class is narrower than its tail class: a segment
            // may not START with `-`, or `a.-b` would read as a dotted dash. An
            // absent head IS the empty segment, so one match answers both.
            let Some(head) = chars.next() else {
                return Err(IdFault::EmptySegment { position });
            };
            if !head.is_ascii_lowercase() && !head.is_ascii_digit() {
                return Err(IdFault::SegmentHead {
                    position,
                    found: head,
                });
            }
            if let Some(found) = chars.find(|c| !is_id_tail(*c)) {
                return Err(IdFault::SegmentChar { position, found });
            }
        }
        Ok(RuleId(id.to_string()))
    }

    /// The id as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The tail character class of one id segment: `[a-z0-9-]`.
fn is_id_tail(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'
}

impl std::fmt::Display for RuleId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Lets an [`EffectiveSet`] keyed by [`RuleId`] be looked up with a plain `&str`.
impl Borrow<str> for RuleId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

/// Why an `id:` is not a legal rule id. Every variant names the offending
/// position, so the refusal teaches the grammar rather than restating it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdFault {
    /// The id is the empty string.
    Empty,
    /// The id exceeds [`MAX_ID_LEN`] bytes.
    TooLong {
        /// The id's length in bytes.
        len: usize,
    },
    /// A dot-separated segment is empty (a leading, trailing, or doubled `.`).
    EmptySegment {
        /// 0-based segment index.
        position: usize,
    },
    /// A segment starts outside `[a-z0-9]`.
    SegmentHead {
        /// 0-based segment index.
        position: usize,
        /// The offending first character.
        found: char,
    },
    /// A segment carries a character outside `[a-z0-9-]`.
    SegmentChar {
        /// 0-based segment index.
        position: usize,
        /// The offending character.
        found: char,
    },
    /// The frontmatter `id:` is present but is not a string (a number, a list, a
    /// map). Identity is a frontmatter QUERY, so it must be readable as text.
    NotAString,
}

impl std::fmt::Display for IdFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let grammar =
            "dot-separated kebab segments, `^[a-z0-9][a-z0-9-]*(\\.[a-z0-9][a-z0-9-]*)*$`";
        match self {
            IdFault::Empty => write!(f, "the id is empty — {grammar}"),
            IdFault::TooLong { len } => write!(
                f,
                "the id is {len} bytes, over the {MAX_ID_LEN}-byte ceiling"
            ),
            IdFault::EmptySegment { position } => write!(
                f,
                "segment {position} is empty — a leading, trailing or doubled `.`; {grammar}"
            ),
            IdFault::SegmentHead { position, found } => write!(
                f,
                "segment {position} starts with {found:?} (U+{code:04X}) — a segment starts with \
                 a lowercase letter or a digit; {grammar}",
                code = *found as u32
            ),
            IdFault::SegmentChar { position, found } => write!(
                f,
                "segment {position} carries {found:?} (U+{code:04X}) — outside `[a-z0-9-]`; \
                 {grammar}",
                code = *found as u32
            ),
            IdFault::NotAString => write!(
                f,
                "`{ID_KEY}:` is not a string — identity is a frontmatter query, so it must be \
                 readable as text; {grammar}"
            ),
        }
    }
}

impl std::error::Error for IdFault {}

// ── The scope ladder ──────────────────────────────────────────────────────────

/// A rung of the scope ladder, outermost first. Declaration order IS the
/// precedence order (`User` < `Workspace`), so ANY workspace page outranks ANY
/// user-space page regardless of how deep the user-space page sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ScopeLayer {
    /// User space — rules under the user scope, sibling of `~/MERIDIAN.md`.
    User,
    /// The workspace. Its root and its folder/session tree are ONE layer,
    /// separated by [`Scope::depth`]: a session-tree page is simply deeper.
    Workspace,
}

impl ScopeLayer {
    /// The layer's name, for refusals and the print verb's scope column.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ScopeLayer::User => "user",
            ScopeLayer::Workspace => "workspace",
        }
    }
}

/// Where a page is mounted on the ladder: its layer and its directory depth
/// within that layer's root. **Ordering is the override law** — layer first, then
/// depth, and no third axis exists. Two pages that compare equal here are tied,
/// and a tie at the winning scope is [`Collision`], never a lexical or mtime
/// coin-flip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Scope {
    layer: ScopeLayer,
    depth: usize,
}

impl Scope {
    /// The scope of a page mounted at `page` (a path relative to `layer`'s root).
    /// Depth counts the directory segments above the file, so a root-level page is
    /// depth 0 and `sessions/s1/rules.md` is depth 2.
    #[must_use]
    pub fn of(layer: ScopeLayer, page: &str) -> Self {
        Scope {
            layer,
            depth: page.split('/').count() - 1,
        }
    }

    /// The ladder rung.
    #[must_use]
    pub fn layer(self) -> ScopeLayer {
        self.layer
    }

    /// Directory depth within the layer's root.
    #[must_use]
    pub fn depth(self) -> usize {
        self.depth
    }
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.layer.as_str(), self.depth)
    }
}

// ── A registered page ─────────────────────────────────────────────────────────

/// One page that carries a registration tag and a valid id. Construction is
/// sealed to [`register_page`] — a `Registration` in hand has passed the tag,
/// grammar and block-agreement gates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registration {
    id: RuleId,
    kinds: Vec<RuleKind>,
    page: String,
    rev: String,
    scope: Scope,
}

impl Registration {
    /// The page's identity.
    #[must_use]
    pub fn id(&self) -> &RuleId {
        &self.id
    }

    /// The registration kinds the page carries, sorted and deduplicated. Never
    /// empty: an untagged page is not a registration.
    #[must_use]
    pub fn kinds(&self) -> &[RuleKind] {
        &self.kinds
    }

    /// The page's path, relative to its layer's root.
    #[must_use]
    pub fn page(&self) -> &str {
        &self.page
    }

    /// The page rev — the uniform fingerprint the ARM act pins ([`page_rev`]).
    #[must_use]
    pub fn rev(&self) -> &str {
        &self.rev
    }

    /// Where the page is mounted on the ladder.
    #[must_use]
    pub fn scope(&self) -> Scope {
        self.scope
    }

    /// The directory the page is mounted in (`""` at a layer root). Consumers that
    /// narrow to an evaluation path read this; the resolver itself never does.
    #[must_use]
    pub fn mount_dir(&self) -> &str {
        match self.page.rfind('/') {
            Some(cut) => &self.page[..cut],
            None => "",
        }
    }
}

/// Why a page that offered itself to registration was refused. Every variant
/// names the page: a rule that silently failed to register is a rule that
/// silently stopped being enforced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterError {
    /// The page has a frontmatter block that is not parseable YAML, so whether it
    /// carries a registration tag cannot be answered. Fail-closed and per-page:
    /// every other page in the same run is unaffected.
    FrontmatterUnparsed {
        /// The offending page.
        page: String,
        /// The YAML parser's own message.
        detail: String,
    },
    /// The page carries a `rules/*` tag this slice does not carry. A named
    /// deferral, never a silent drop.
    KindDeferred {
        /// The offending page.
        page: String,
        /// The tag as written.
        tag: String,
    },
    /// The page carries a registration tag but declares no `id:`.
    IdAbsent {
        /// The offending page.
        page: String,
    },
    /// The page's `id:` is outside the § 2 grammar.
    IdInvalid {
        /// The offending page.
        page: String,
        /// The id as written (empty when it was not even a string).
        id: String,
        /// Which rule broke.
        fault: IdFault,
    },
    /// The fenced Starlark block restates the id, and disagrees with frontmatter.
    IdDisagrees {
        /// The offending page.
        page: String,
        /// What the frontmatter says.
        frontmatter: String,
        /// What the block says.
        block: String,
    },
    /// The block assigns `id` to something that is not a string literal, so the
    /// agreement question is unanswerable without evaluating it — which the
    /// layering forbids.
    BlockIdNotLiteral {
        /// The offending page.
        page: String,
    },
    /// The page's fenced Starlark block does not parse, so a block-declared id
    /// cannot be read. Fail-closed: a page whose block might contradict its
    /// frontmatter never registers on the assumption that it does not.
    BlockUnparsed {
        /// The offending page.
        page: String,
        /// The Starlark parser's own message.
        detail: String,
    },
}

impl RegisterError {
    /// The page the refusal is about.
    #[must_use]
    pub fn page(&self) -> &str {
        match self {
            RegisterError::FrontmatterUnparsed { page, .. }
            | RegisterError::KindDeferred { page, .. }
            | RegisterError::IdAbsent { page }
            | RegisterError::IdInvalid { page, .. }
            | RegisterError::IdDisagrees { page, .. }
            | RegisterError::BlockIdNotLiteral { page }
            | RegisterError::BlockUnparsed { page, .. } => page,
        }
    }
}

impl std::fmt::Display for RegisterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegisterError::FrontmatterUnparsed { page, detail } => write!(
                f,
                "`{page}` has frontmatter that does not parse, so its registration tags cannot \
                 be read: {detail}"
            ),
            RegisterError::KindDeferred { page, tag } => write!(
                f,
                "`{page}` carries `{tag}`, but `{REGISTRATION_NAMESPACE}` carries \
                 `{check}` and `{hook}` only — the tag is reserved, not ignored",
                check = RuleKind::Check.tag(),
                hook = RuleKind::Hook.tag(),
            ),
            RegisterError::IdAbsent { page } => write!(
                f,
                "`{page}` carries a `{REGISTRATION_NAMESPACE}*` registration tag but declares no \
                 `{ID_KEY}:` — a rule page is identified by its frontmatter id, not by its filename"
            ),
            RegisterError::IdInvalid { page, id, fault } => {
                write!(f, "`{page}` declares `{ID_KEY}: {id}` — {fault}")
            }
            RegisterError::IdDisagrees {
                page,
                frontmatter,
                block,
            } => write!(
                f,
                "`{page}` declares `{ID_KEY}: {frontmatter}` in frontmatter but `{ID_KEY} = \
                 {block:?}` in its Starlark block — the block binds to the page's frontmatter id, \
                 it does not name a second one"
            ),
            RegisterError::BlockIdNotLiteral { page } => write!(
                f,
                "`{page}` assigns `{ID_KEY}` in its Starlark block to something other than a \
                 string literal — identity is a frontmatter query, so a block may only restate \
                 the id verbatim"
            ),
            RegisterError::BlockUnparsed { page, detail } => write!(
                f,
                "`{page}` has a fenced starlark block that does not parse, so a block-declared \
                 `{ID_KEY}` cannot be read: {detail}"
            ),
        }
    }
}

impl std::error::Error for RegisterError {}

// ── The page fingerprint ──────────────────────────────────────────────────────

/// The page rev: `blake3(page bytes)[:16]`, 16 lowercase hex.
///
/// This is the node-rev-merkle-spec law applied at the document root, whose span
/// is the whole file (§ 2 hashes the node's span bytes; § 3's file leaf is the
/// same bytes). ONE fingerprint law for check pages and hook pages alike — the
/// grain that dissolves the `blake3(CHECK.md)` special-casing, which is why
/// [`crate::evidence_rev`] delegates here rather than computing a second one.
#[must_use]
pub fn page_rev(bytes: &str) -> String {
    blake3::hash(bytes.as_bytes()).to_hex().as_str()[..16].to_string()
}

// ── Discovery ─────────────────────────────────────────────────────────────────

/// Only the frontmatter keys registration reads. Every other key is permitted and
/// ignored — a rule page carries ordinary page frontmatter too. Note what is NOT
/// here: `kind:`. The engine consults it nowhere.
#[derive(serde::Deserialize)]
struct RegistrationFrontmatter {
    tags: Option<Tags>,
    id: Option<serde_yaml::Value>,
}

/// `tags:` as either a flow/block sequence or a single scalar. Both spellings are
/// legal Obsidian frontmatter, so a page written the second way must not fail as
/// unparseable YAML when it is merely terse.
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum Tags {
    One(String),
    Many(Vec<String>),
}

impl Tags {
    fn iter(&self) -> std::slice::Iter<'_, String> {
        match self {
            Tags::One(one) => std::slice::from_ref(one).iter(),
            Tags::Many(many) => many.iter(),
        }
    }
}

/// One page the caller's walk offers to discovery. `page` is relative to `layer`'s
/// root; `bytes` are the page's raw bytes, which the rev hashes verbatim.
#[derive(Debug, Clone, Copy)]
pub struct PageRef<'a> {
    /// Which ladder rung the page was found on.
    pub layer: ScopeLayer,
    /// The page's path, relative to that layer's root.
    pub page: &'a str,
    /// The page's raw bytes.
    pub bytes: &'a str,
}

/// Read one page's registration.
///
/// Returns `Ok(None)` when the page carries no `rules/*` tag — the overwhelmingly
/// common case, and silent by design: an ordinary page is not a failed rule page.
///
/// Faults are ordered frontmatter-first: an id fault is reported before the block
/// is even looked at, because identity is a frontmatter query and a page with no
/// valid id has no identity for a block to agree with.
///
/// # Errors
/// [`RegisterError`] — the page offered itself to registration and was refused.
pub fn register_page(offered: PageRef<'_>) -> Result<Option<Registration>, RegisterError> {
    let PageRef { layer, page, bytes } = offered;

    let Some((frontmatter, _body)) = crate::pack::split_frontmatter(bytes) else {
        return Ok(None); // no frontmatter block ⇒ no tags ⇒ not a rule page
    };
    let parsed: RegistrationFrontmatter =
        serde_yaml::from_str(frontmatter).map_err(|e| RegisterError::FrontmatterUnparsed {
            page: page.to_string(),
            detail: e.to_string(),
        })?;

    let kinds = registration_kinds(page, parsed.tags.as_ref())?;
    if kinds.is_empty() {
        return Ok(None); // tagged, but not in the registration namespace
    }

    let id = frontmatter_id(page, parsed.id.as_ref())?;

    if let Some(declared) = block_declared_id(page, bytes)?
        && declared != id.as_str()
    {
        return Err(RegisterError::IdDisagrees {
            page: page.to_string(),
            frontmatter: id.as_str().to_string(),
            block: declared,
        });
    }

    Ok(Some(Registration {
        id,
        kinds,
        page: page.to_string(),
        rev: page_rev(bytes),
        scope: Scope::of(layer, page),
    }))
}

/// The registration kinds `tags:` declares, sorted and deduplicated. A `rules/*`
/// tag outside this slice's vocabulary is refused by name.
fn registration_kinds(page: &str, tags: Option<&Tags>) -> Result<Vec<RuleKind>, RegisterError> {
    let Some(tags) = tags else {
        return Ok(Vec::new());
    };
    let mut kinds = Vec::new();
    for tag in tags.iter() {
        let Some(suffix) = tag.strip_prefix(REGISTRATION_NAMESPACE) else {
            continue;
        };
        let kind = RuleKind::from_suffix(suffix).ok_or_else(|| RegisterError::KindDeferred {
            page: page.to_string(),
            tag: tag.clone(),
        })?;
        kinds.push(kind);
    }
    kinds.sort_unstable();
    kinds.dedup();
    Ok(kinds)
}

/// The page's frontmatter `id:`, validated to the § 2 grammar.
fn frontmatter_id(page: &str, id: Option<&serde_yaml::Value>) -> Result<RuleId, RegisterError> {
    let absent = || RegisterError::IdAbsent {
        page: page.to_string(),
    };
    let value = id.filter(|v| !v.is_null()).ok_or_else(absent)?;
    let Some(text) = value.as_str() else {
        return Err(RegisterError::IdInvalid {
            page: page.to_string(),
            id: String::new(),
            fault: IdFault::NotAString,
        });
    };
    RuleId::parse(text).map_err(|fault| RegisterError::IdInvalid {
        page: page.to_string(),
        id: text.to_string(),
        fault,
    })
}

/// The id a page's fenced Starlark block restates, read STATICALLY from the
/// parsed AST. `Ok(None)` when the page carries no block, or a block that binds no
/// top-level `id`.
fn block_declared_id(page: &str, bytes: &str) -> Result<Option<String>, RegisterError> {
    let Some(source) = crate::pack::extract_fenced_starlark(bytes) else {
        return Ok(None);
    };
    let ast = AstModule::parse(page, source, &Dialect::Standard).map_err(|e| {
        RegisterError::BlockUnparsed {
            page: page.to_string(),
            detail: e.to_string(),
        }
    })?;
    match top_level_id(ast.statement()) {
        Some(BlockId::Text(id)) => Ok(Some(id)),
        Some(BlockId::NotALiteral) => Err(RegisterError::BlockIdNotLiteral {
            page: page.to_string(),
        }),
        None => Ok(None),
    }
}

/// What a top-level `id` binding was assigned.
enum BlockId {
    /// A string literal — the only spelling that can restate a frontmatter id.
    Text(String),
    /// Anything else. Deciding what it equals would require evaluating the block.
    NotALiteral,
}

/// The first TOP-LEVEL `id = …` binding in a module. Recursion enters
/// [`Stmt::Statements`] (the module's own statement list) and nothing else — an
/// `id` bound inside a `def` or an `if` is a local, not a declaration.
fn top_level_id(stmt: &AstStmt) -> Option<BlockId> {
    match &**stmt {
        Stmt::Statements(statements) => statements.iter().find_map(top_level_id),
        Stmt::Assign(assign) => {
            let AssignTarget::Identifier(target) = &*assign.lhs else {
                return None;
            };
            if target.ident != ID_KEY {
                return None;
            }
            Some(match &*assign.rhs {
                Expr::Literal(AstLiteral::String(text)) => BlockId::Text(text.node.clone()),
                _ => BlockId::NotALiteral,
            })
        }
        _ => None,
    }
}

// ── The discovery index ───────────────────────────────────────────────────────

/// Every rule page a walk found, plus every page it refused. This is the retained
/// index: it keeps ALL candidates for an id, so [`RuleIndex::resolve`] can hand
/// back the full override chain rather than only its winner.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuleIndex {
    registered: Vec<Registration>,
    refused: Vec<RegisterError>,
}

impl RuleIndex {
    /// Discover over the pages a caller's walk offers. Refusals are collected
    /// per-page and never abort the sweep — one malformed rule page must not
    /// un-register the rest of the workspace.
    #[must_use]
    pub fn discover<'a>(pages: impl IntoIterator<Item = PageRef<'a>>) -> Self {
        let mut index = RuleIndex::default();
        for offered in pages {
            match register_page(offered) {
                Ok(Some(registration)) => index.registered.push(registration),
                Ok(None) => {}
                Err(refusal) => index.refused.push(refusal),
            }
        }
        index
    }

    /// Every page that registered, in discovery order.
    #[must_use]
    pub fn registered(&self) -> &[Registration] {
        &self.registered
    }

    /// Every page that offered itself and was refused, in discovery order.
    #[must_use]
    pub fn refused(&self) -> &[RegisterError] {
        &self.refused
    }

    /// Resolve the override law over the index (§ 3).
    ///
    /// Pure: no I/O, no arming, no receipt, no cap. Calling it twice on the same
    /// index yields the same answer, which is what lets a read-only consumer and
    /// the arming path share ONE resolver.
    ///
    /// Per id: the candidate with the greatest [`Scope`] wins and the rest are
    /// retained as its shadowed chain, nearest-first. A tie at the greatest scope
    /// is a [`Collision`] — that id resolves to nothing while every other id
    /// resolves normally.
    #[must_use]
    pub fn resolve(&self) -> EffectiveSet {
        let mut grouped: BTreeMap<&RuleId, Vec<&Registration>> = BTreeMap::new();
        for registration in &self.registered {
            grouped
                .entry(&registration.id)
                .or_default()
                .push(registration);
        }

        let mut resolved = BTreeMap::new();
        let mut collisions = Vec::new();
        for (id, mut candidates) in grouped {
            // Ladder order: nearest scope first. `page` breaks ties only so the
            // RENDER is deterministic — it never decides precedence, which is why
            // a tie at the top is a collision instead of a lexical win.
            candidates.sort_by(|a, b| b.scope.cmp(&a.scope).then_with(|| a.page.cmp(&b.page)));
            let top = candidates[0].scope;
            let tied = candidates.iter().filter(|c| c.scope == top).count();
            if tied > 1 {
                collisions.push(Collision {
                    id: id.clone(),
                    scope: top,
                    tied: candidates[..tied].iter().map(|c| (*c).clone()).collect(),
                    shadowed: candidates[tied..].iter().map(|c| (*c).clone()).collect(),
                });
            } else {
                resolved.insert(
                    id.clone(),
                    Effective {
                        winner: candidates[0].clone(),
                        shadowed: candidates[1..].iter().map(|c| (*c).clone()).collect(),
                    },
                );
            }
        }
        EffectiveSet {
            resolved,
            collisions,
        }
    }
}

/// The resolved effective set: which page governs each id, what it shadows, and
/// which ids resolve to nothing because they collided.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EffectiveSet {
    resolved: BTreeMap<RuleId, Effective>,
    collisions: Vec<Collision>,
}

impl EffectiveSet {
    /// Every id that resolves, id-ascending.
    #[must_use]
    pub fn resolved(&self) -> &BTreeMap<RuleId, Effective> {
        &self.resolved
    }

    /// The resolution for one id, or `None` when it is unregistered OR collided.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Effective> {
        self.resolved.get(id)
    }

    /// Every id refused for a same-scope tie, id-ascending.
    #[must_use]
    pub fn collisions(&self) -> &[Collision] {
        &self.collisions
    }
}

/// One id's resolution: the page that governs, plus every page it shadows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effective {
    winner: Registration,
    shadowed: Vec<Registration>,
}

impl Effective {
    /// The page that governs this id.
    #[must_use]
    pub fn winner(&self) -> &Registration {
        &self.winner
    }

    /// The pages this id's winner shadows, nearest scope first.
    #[must_use]
    pub fn shadowed(&self) -> &[Registration] {
        &self.shadowed
    }

    /// The whole override chain, winner first then outward — what the print verb
    /// renders so the chain is never silently collapsed.
    pub fn chain(&self) -> impl Iterator<Item = &Registration> {
        std::iter::once(&self.winner).chain(self.shadowed.iter())
    }
}

/// Two or more pages tied at an id's winning scope. The id resolves to NOTHING —
/// fail-closed, because picking one would be a coin-flip dressed as a law. The
/// shadowed remainder is retained anyway so the chain stays printable under the
/// refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collision {
    id: RuleId,
    scope: Scope,
    tied: Vec<Registration>,
    shadowed: Vec<Registration>,
}

impl Collision {
    /// The id that resolves to nothing.
    #[must_use]
    pub fn id(&self) -> &RuleId {
        &self.id
    }

    /// The scope both pages are mounted at.
    #[must_use]
    pub fn scope(&self) -> Scope {
        self.scope
    }

    /// The tied pages, page-ascending. Always at least two.
    #[must_use]
    pub fn tied(&self) -> &[Registration] {
        &self.tied
    }

    /// The pages the tie shadows, nearest scope first.
    #[must_use]
    pub fn shadowed(&self) -> &[Registration] {
        &self.shadowed
    }
}

impl std::fmt::Display for Collision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let pages: Vec<&str> = self.tied.iter().map(Registration::page).collect();
        write!(
            f,
            "id `{id}` resolves to nothing: {n} pages are mounted at the same scope ({scope}) — \
             {pages}. Mount depth is the only precedence axis, so a tie has no winner; move one \
             page or rename its id. Every other id is unaffected.",
            id = self.id,
            n = self.tied.len(),
            scope = self.scope,
            pages = pages.join(", "),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal rule page: registration tag + id, no block.
    fn page(id: &str) -> String {
        format!("---\ntags: [type/rule, rules/hook]\nid: {id}\n---\n\n# rule\n")
    }

    fn offer<'a>(layer: ScopeLayer, path: &'a str, bytes: &'a str) -> PageRef<'a> {
        PageRef {
            layer,
            page: path,
            bytes,
        }
    }

    fn register(path: &str, bytes: &str) -> Result<Option<Registration>, RegisterError> {
        register_page(offer(ScopeLayer::Workspace, path, bytes))
    }

    // ── discovery ─────────────────────────────────────────────────────────────

    #[test]
    fn a_tagged_page_with_a_valid_id_registers() {
        let body = page("task.review-notify");
        let got = register("rules/notify.md", &body)
            .expect("a well-formed rule page registers")
            .expect("and it is a rule page");
        assert_eq!(got.id().as_str(), "task.review-notify");
        assert_eq!(got.kinds(), &[RuleKind::Hook]);
        assert_eq!(got.page(), "rules/notify.md");
        assert_eq!(got.rev(), page_rev(&body));
        assert_eq!(
            got.scope(),
            Scope::of(ScopeLayer::Workspace, "rules/notify.md")
        );
        assert_eq!(got.mount_dir(), "rules");
    }

    #[test]
    fn an_untagged_page_is_not_discovered() {
        let ordinary = "---\ntype: note\ntags: [type/note]\nid: looks-like-an-id\n---\n\n# note\n";
        assert_eq!(register("notes/x.md", ordinary), Ok(None));
        // …and neither is a page with no frontmatter at all.
        assert_eq!(register("notes/y.md", "# just a heading\n"), Ok(None));
    }

    #[test]
    fn the_check_tag_registers_the_law_leg() {
        let body = "---\ntags: [rules/check]\nid: reviewer-not-owner\n---\n";
        let got = register("CHECK-ish.md", body).unwrap().unwrap();
        assert_eq!(got.kinds(), &[RuleKind::Check]);
    }

    /// **A named gap, pinned rather than hidden.** A page may carry BOTH
    /// registration tags. Discovery records what the page declares — it does not
    /// mint a "one tag per page" refusal the ruling never stated. What a dual-kind
    /// page ARMS is the arming card's question (§4 splits mode vocabulary by kind:
    /// `off|warn|block` for checks, `off|armed` for hooks), and answering it here
    /// would be minting law inside the loader.
    #[test]
    fn a_page_may_carry_both_registration_tags() {
        let body = "---\ntags: [rules/check, rules/hook]\nid: dual\n---\n";
        let got = register("dual.md", body).unwrap().unwrap();
        assert_eq!(got.kinds(), &[RuleKind::Check, RuleKind::Hook]);
    }

    #[test]
    fn the_engine_consults_no_kind_frontmatter() {
        // `kind: hook` WITHOUT a registration tag registers nothing…
        let legacy = "---\nkind: hook\nid: legacy\n---\n";
        assert_eq!(register("legacy.md", legacy), Ok(None));
        // …and a contradictory `kind:` beside a real tag changes nothing.
        let mixed = "---\nkind: check\ntags: [rules/hook]\nid: mixed\n---\n";
        let got = register("mixed.md", mixed).unwrap().unwrap();
        assert_eq!(
            got.kinds(),
            &[RuleKind::Hook],
            "the TAG decides, not `kind:`"
        );
    }

    #[test]
    fn a_scalar_tags_spelling_registers() {
        let body = "---\ntags: rules/hook\nid: terse\n---\n";
        assert_eq!(
            register("t.md", body).unwrap().unwrap().kinds(),
            &[RuleKind::Hook]
        );
    }

    #[test]
    fn an_unknown_rules_tag_is_a_named_deferral() {
        let body = "---\ntags: [rules/fix]\nid: someday\n---\n";
        let err = register("f.md", body).expect_err("the namespace is reserved");
        assert!(matches!(err, RegisterError::KindDeferred { .. }), "{err:?}");
        assert_eq!(err.page(), "f.md");
        assert!(err.to_string().contains("rules/fix"), "{err}");
    }

    // ── id grammar ────────────────────────────────────────────────────────────

    #[test]
    fn a_tagged_page_without_an_id_fails_loudly() {
        let body = "---\ntags: [rules/hook]\n---\n\n# no id\n";
        let err = register("rules/anon.md", body).expect_err("no id is loud");
        assert_eq!(
            err,
            RegisterError::IdAbsent {
                page: "rules/anon.md".into()
            }
        );
        let rendered = err.to_string();
        assert!(
            rendered.contains("rules/anon.md"),
            "names the page: {rendered}"
        );
        assert!(rendered.contains("id:"), "names the reason: {rendered}");
    }

    #[test]
    fn a_null_id_reads_as_absent() {
        let body = "---\ntags: [rules/hook]\nid:\n---\n";
        assert!(matches!(
            register("n.md", body),
            Err(RegisterError::IdAbsent { .. })
        ));
    }

    #[test]
    fn a_malformed_id_fails_loudly_naming_page_and_reason() {
        for (id, expect) in [
            (
                "Task_Review",
                IdFault::SegmentHead {
                    position: 0,
                    found: 'T',
                },
            ),
            (
                "task_review",
                IdFault::SegmentChar {
                    position: 0,
                    found: '_',
                },
            ),
            (
                "-leading",
                IdFault::SegmentHead {
                    position: 0,
                    found: '-',
                },
            ),
            (".leading-dot", IdFault::EmptySegment { position: 0 }),
            ("trailing.", IdFault::EmptySegment { position: 1 }),
            ("double..dot", IdFault::EmptySegment { position: 1 }),
            (
                "a.-b",
                IdFault::SegmentHead {
                    position: 1,
                    found: '-',
                },
            ),
        ] {
            let body = format!("---\ntags: [rules/hook]\nid: \"{id}\"\n---\n");
            let err = register("rules/bad.md", &body).expect_err("malformed id is loud");
            let RegisterError::IdInvalid {
                page,
                id: got,
                fault,
            } = err.clone()
            else {
                panic!("expected IdInvalid for {id:?}, got {err:?}");
            };
            assert_eq!(page, "rules/bad.md");
            assert_eq!(got, id);
            assert_eq!(fault, expect, "id {id:?}");
            assert!(err.to_string().contains("rules/bad.md"), "{err}");
        }
    }

    #[test]
    fn the_id_ceiling_is_64_bytes() {
        assert!(RuleId::parse(&"a".repeat(MAX_ID_LEN)).is_ok());
        assert_eq!(
            RuleId::parse(&"a".repeat(MAX_ID_LEN + 1)),
            Err(IdFault::TooLong {
                len: MAX_ID_LEN + 1
            })
        );
    }

    #[test]
    fn the_grammar_admits_dotted_kebab_segments() {
        for ok in [
            "a",
            "0",
            "task-review",
            "task.review-notify",
            "a.b.c",
            "x9.y-0",
        ] {
            assert!(RuleId::parse(ok).is_ok(), "{ok:?} is legal");
        }
    }

    #[test]
    fn a_non_string_id_is_not_an_identity() {
        let body = "---\ntags: [rules/hook]\nid: 42\n---\n";
        let err = register("num.md", body).expect_err("a number is not an id");
        assert!(
            matches!(
                err,
                RegisterError::IdInvalid {
                    fault: IdFault::NotAString,
                    ..
                }
            ),
            "{err:?}"
        );
    }

    // ── block-declared id ─────────────────────────────────────────────────────

    fn page_with_block(id: &str, block: &str) -> String {
        format!("---\ntags: [rules/hook]\nid: {id}\n---\n\n```starlark\n{block}\n```\n")
    }

    #[test]
    fn a_block_id_that_agrees_registers() {
        let body = page_with_block(
            "task.review",
            "id = \"task.review\"\n\ndef on_change(e):\n    pass",
        );
        let got = register("r.md", &body).unwrap().unwrap();
        assert_eq!(got.id().as_str(), "task.review");
    }

    #[test]
    fn a_block_id_that_disagrees_fails_loudly() {
        let body = page_with_block("task.review", "id = \"other.rule\"");
        let err = register("r.md", &body).expect_err("disagreement is loud");
        assert_eq!(
            err,
            RegisterError::IdDisagrees {
                page: "r.md".into(),
                frontmatter: "task.review".into(),
                block: "other.rule".into(),
            }
        );
        let rendered = err.to_string();
        assert!(
            rendered.contains("task.review") && rendered.contains("other.rule"),
            "{rendered}"
        );
    }

    #[test]
    fn a_block_that_declares_no_id_is_silent() {
        let body = page_with_block("task.review", "def on_change(e):\n    pass");
        assert!(register("r.md", &body).unwrap().is_some());
    }

    #[test]
    fn an_id_bound_inside_a_def_is_a_local_not_a_declaration() {
        let body = page_with_block(
            "task.review",
            "def on_change(e):\n    id = \"shadow\"\n    return id",
        );
        assert!(
            register("r.md", &body).unwrap().is_some(),
            "only TOP-LEVEL bindings declare"
        );
    }

    #[test]
    fn a_non_literal_block_id_is_refused_rather_than_evaluated() {
        let body = page_with_block("task.review", "id = 1 + 1");
        assert!(matches!(
            register("r.md", &body),
            Err(RegisterError::BlockIdNotLiteral { .. })
        ));
    }

    #[test]
    fn an_unparseable_block_fails_closed() {
        let body = page_with_block("task.review", "def (((");
        let err = register("r.md", &body).expect_err("an unreadable block is loud");
        assert!(
            matches!(err, RegisterError::BlockUnparsed { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn unparseable_frontmatter_is_refused_per_page() {
        let body = "---\ntags: [rules/hook\nid: broken\n---\n";
        let err = register("b.md", body).expect_err("unreadable frontmatter is loud");
        assert!(
            matches!(err, RegisterError::FrontmatterUnparsed { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn frontmatter_faults_are_reported_before_block_faults() {
        // Both wrong: no id AND an unparseable block. Identity is a frontmatter
        // query, so the frontmatter fault is the one reported.
        let body = "---\ntags: [rules/hook]\n---\n\n```starlark\ndef (((\n```\n";
        assert!(matches!(
            register("r.md", body),
            Err(RegisterError::IdAbsent { .. })
        ));
    }

    // ── override resolution ───────────────────────────────────────────────────

    /// Build an index from `(layer, path)` pairs sharing generated bodies.
    fn index_of(pages: &[(ScopeLayer, &str, &str)]) -> RuleIndex {
        let bodies: Vec<String> = pages.iter().map(|(_, _, id)| page(id)).collect();
        RuleIndex::discover(
            pages
                .iter()
                .zip(&bodies)
                .map(|((layer, path, _), body)| offer(*layer, path, body)),
        )
    }

    #[test]
    fn the_deeper_workspace_page_wins() {
        use ScopeLayer::Workspace;
        let index = index_of(&[
            (Workspace, "rules.md", "shared"),
            (Workspace, "sessions/s1/rules.md", "shared"),
        ]);
        let set = index.resolve();
        let effective = set.get("shared").expect("resolves");
        assert_eq!(effective.winner().page(), "sessions/s1/rules.md");
        assert_eq!(effective.winner().scope().depth(), 2);
        assert_eq!(effective.shadowed().len(), 1);
        assert_eq!(effective.shadowed()[0].page(), "rules.md");
        assert!(set.collisions().is_empty());
    }

    #[test]
    fn depth_alone_decides_not_lexical_order_or_mtime() {
        use ScopeLayer::Workspace;
        // `zzz.md` sorts LAST lexically and is the shallower page; the deeper
        // `aaa/aaa.md` still wins. mtime is not an input to this module at all —
        // `PageRef` carries no timestamp, so there is nothing to tiebreak on.
        let index = index_of(&[
            (Workspace, "zzz.md", "shared"),
            (Workspace, "aaa/aaa.md", "shared"),
        ]);
        let effective = index.resolve();
        let got = effective.get("shared").unwrap();
        assert_eq!(got.winner().page(), "aaa/aaa.md");
        assert_eq!(got.shadowed()[0].page(), "zzz.md");
    }

    #[test]
    fn the_three_ladder_layers_order_outermost_to_innermost() {
        use ScopeLayer::{User, Workspace};
        // session tree beats workspace root
        let a = index_of(&[
            (Workspace, "rules.md", "x"),
            (Workspace, "sessions/s1/rules.md", "x"),
        ]);
        assert_eq!(
            a.resolve().get("x").unwrap().winner().page(),
            "sessions/s1/rules.md"
        );

        // workspace root beats user space — even a DEEP user page.
        let b = index_of(&[
            (User, "a/b/c/d/e/f/g/h/i/rules.md", "x"),
            (Workspace, "rules.md", "x"),
        ]);
        let b = b.resolve();
        let b = b.get("x").unwrap();
        assert_eq!(b.winner().page(), "rules.md");
        assert_eq!(b.winner().scope().layer(), Workspace);

        // session tree beats user space
        let c = index_of(&[
            (User, "rules.md", "x"),
            (Workspace, "sessions/s1/rules.md", "x"),
        ]);
        assert_eq!(
            c.resolve().get("x").unwrap().winner().scope().layer(),
            Workspace
        );
    }

    #[test]
    fn a_tie_at_the_winning_scope_refuses_that_id_alone() {
        use ScopeLayer::Workspace;
        let index = index_of(&[
            (Workspace, "rules/a.md", "shared"),
            (Workspace, "rules/b.md", "shared"),
            (Workspace, "rules/c.md", "untouched"),
        ]);
        let set = index.resolve();

        assert!(
            set.get("shared").is_none(),
            "a collided id resolves to nothing"
        );
        assert_eq!(set.collisions().len(), 1);
        let collision = &set.collisions()[0];
        assert_eq!(collision.id().as_str(), "shared");
        assert_eq!(
            collision
                .tied()
                .iter()
                .map(Registration::page)
                .collect::<Vec<_>>(),
            vec!["rules/a.md", "rules/b.md"],
        );
        let rendered = collision.to_string();
        assert!(
            rendered.contains("rules/a.md") && rendered.contains("rules/b.md"),
            "{rendered}"
        );

        // Every other id is unaffected.
        assert_eq!(
            set.get("untouched").expect("resolves").winner().page(),
            "rules/c.md"
        );
    }

    #[test]
    fn a_collision_still_retains_the_chain_it_shadows() {
        use ScopeLayer::Workspace;
        let index = index_of(&[
            (Workspace, "rules.md", "shared"),
            (Workspace, "s/a.md", "shared"),
            (Workspace, "s/b.md", "shared"),
        ]);
        let set = index.resolve();
        let collision = &set.collisions()[0];
        assert_eq!(collision.tied().len(), 2, "the two deep pages tie");
        assert_eq!(collision.shadowed().len(), 1);
        assert_eq!(collision.shadowed()[0].page(), "rules.md");
    }

    #[test]
    fn the_chain_is_recoverable_winner_first_in_ladder_order() {
        use ScopeLayer::{User, Workspace};
        let index = index_of(&[
            (User, "rules.md", "x"),
            (Workspace, "rules.md", "x"),
            (Workspace, "a/rules.md", "x"),
            (Workspace, "a/b/rules.md", "x"),
        ]);
        let set = index.resolve();
        let chain: Vec<(&str, ScopeLayer, usize)> = set
            .get("x")
            .unwrap()
            .chain()
            .map(|r| (r.page(), r.scope().layer(), r.scope().depth()))
            .collect();
        assert_eq!(
            chain,
            vec![
                ("a/b/rules.md", Workspace, 2),
                ("a/rules.md", Workspace, 1),
                ("rules.md", Workspace, 0),
                ("rules.md", User, 0),
            ],
            "winner first, then outward to user space"
        );
    }

    #[test]
    fn resolution_is_pure_and_repeatable() {
        use ScopeLayer::Workspace;
        let index = index_of(&[
            (Workspace, "a.md", "one"),
            (Workspace, "s/a.md", "one"),
            (Workspace, "b.md", "two"),
        ]);
        assert_eq!(index.resolve(), index.resolve());
    }

    #[test]
    fn refusals_never_abort_the_sweep() {
        let good = page("good");
        let bad = "---\ntags: [rules/hook]\n---\n";
        let ordinary = "---\ntype: note\n---\n";
        let index = RuleIndex::discover([
            offer(ScopeLayer::Workspace, "good.md", &good),
            offer(ScopeLayer::Workspace, "bad.md", bad),
            offer(ScopeLayer::Workspace, "note.md", ordinary),
        ]);
        assert_eq!(index.registered().len(), 1);
        assert_eq!(index.refused().len(), 1);
        assert_eq!(index.refused()[0].page(), "bad.md");
        assert_eq!(
            index.resolve().get("good").unwrap().winner().page(),
            "good.md"
        );
    }

    #[test]
    fn page_rev_is_the_one_page_fingerprint_law() {
        let rev = page_rev("hello");
        assert_eq!(rev.len(), 16);
        assert!(
            rev.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
        assert_ne!(page_rev("a"), page_rev("b"));
        assert_eq!(
            rev,
            crate::evidence_rev("hello"),
            "the CHECK.md evidence rev IS the page rev — one law, not two"
        );
    }

    #[test]
    fn scope_depth_counts_directory_segments() {
        assert_eq!(Scope::of(ScopeLayer::Workspace, "a.md").depth(), 0);
        assert_eq!(Scope::of(ScopeLayer::Workspace, "a/b.md").depth(), 1);
        assert_eq!(Scope::of(ScopeLayer::Workspace, "a/b/c.md").depth(), 2);
    }

    #[test]
    fn the_ladder_orders_user_below_workspace_at_every_depth() {
        let deep_user = Scope::of(ScopeLayer::User, "a/b/c/d/e.md");
        let root_workspace = Scope::of(ScopeLayer::Workspace, "e.md");
        assert!(root_workspace > deep_user, "layer dominates depth");
    }
}
