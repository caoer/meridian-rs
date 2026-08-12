//! The mount table — where a declared mount entry becomes a bound root.
//!
//! Owns: canonicalization at bind, the `workspace::deny_reason` ceiling, the
//! equal-or-nested refusal, the three-way map's uniqueness invariants, the read
//! of each root's own self-declaration and the declared-vs-bound check, and
//! mount-as-claim — the pin a mount carries over the root it declares.
//!
//! Never does: resolve an address (`docs/address-grammar.md` §5, §6). This
//! module answers only *which roots does this machine bind, and what can it say
//! about each*.
//!
//! The table is the single authority for the three-way translation — canonical
//! root name ↔ Obsidian vault name ↔ local path. Each representation is a key
//! (INV-1 name, INV-2/INV-4 path, INV-3 vault); name ↔ path is total, the
//! vault axis is partial because a `git-folder` root carries no vault name.
//!
//! Every mount path is canonicalized and then passed through
//! [`workspace::deny_reason`] — the same predicate the workspace ladder uses,
//! reused, never re-implemented (`docs/address-grammar.md` §8 B-2). A refused
//! mount fails the whole parse: [`MountTable`]'s field is private and [`bind`]
//! is its only constructor, so a partially-bound table cannot exist.
//!
//! Canonicalize first: a symlinked spelling and a trailing-slash spelling are
//! one tree, and one tree bound twice under two names yields two canonical refs
//! the read-mint recheck cannot tell apart. Equal-or-nested is refused; the
//! prefix test is path-segment-boundary, so `/a/wiki` + `/a/wiki-two` stay
//! legal siblings.
//!
//! The root declares its own name in [`DECLARATION_FILENAME`]; `MERIDIAN.md`
//! binds. Agreement binds; disagreement fails the whole parse naming both
//! spellings; an absent declaration renders grey. The declaration shares the
//! config's reserved filename and is discriminated by its `type:` key.
//!
//! A mount may pin the root it declares (schema §5.3): the pin's target is the
//! declaration file, verified through [`model::fingerprint::verify_content`].
//! The pin does not protect the table's own membership — deleting a mount
//! block deletes its pin along with it.

use std::path::{Path, PathBuf};

use crate::{
    CONFIG_FILENAME, Config, MountEntry, MountKind, NO_PARTIAL_LOAD_CLAUSE, Resolution, VERSION,
    check_name, find_frontmatter, frontmatter_inner, key_line, scalar_text,
};

/// The reserved filename of a root's own self-declaration, at the root's top
/// level — the same reserved name the config plane uses, discriminated by the
/// `type:` key (schema §4, §2.4).
pub const DECLARATION_FILENAME: &str = CONFIG_FILENAME;

/// The `type:` discriminator a root's self-declaration carries.
pub const DECLARATION_TYPE: &str = "meridian-root";

/// The declaration's required frontmatter keys, in canonical order. `name` is
/// the canonical root name the root claims for itself.
pub const DECLARATION_KEYS: [&str; 3] = ["type", "version", "name"];

/// Why a mount refused to bind. The closed set of the mount-path law
/// (`docs/address-grammar.md` §3 rows T2-T5 and §8 rows M1-M4).
///
/// A separate closed set from [`crate::Reason`]: that one is schema §8.2's
/// in-file parse vocabulary; these are bind-time semantics with a different
/// spec and owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MountReason {
    /// The canonicalized mount path is one [`workspace::deny_reason`] refuses.
    MountPathDenied,
    /// Two entries canonicalize to the same tree (INV-2, row M3).
    DuplicateMountPath,
    /// One bound path contains another at a segment boundary (INV-4, row M4).
    NestedMount,
    /// Two entries name the same Obsidian vault (INV-3, row T3).
    DuplicateVaultName,
    /// A root declares a canonical name the table does not bind it to (INV-5,
    /// row T5).
    DeclaredBoundMismatch,
}

impl MountReason {
    /// Every reason word, in the order the law's tables state them.
    pub const ALL: [MountReason; 5] = [
        MountReason::MountPathDenied,
        MountReason::DuplicateMountPath,
        MountReason::NestedMount,
        MountReason::DuplicateVaultName,
        MountReason::DeclaredBoundMismatch,
    ];

    /// The reason word — the closed-set spelling.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            MountReason::MountPathDenied => "mount-path-denied",
            MountReason::DuplicateMountPath => "duplicate-mount-path",
            MountReason::NestedMount => "nested-mount",
            MountReason::DuplicateVaultName => "duplicate-vault-name",
            MountReason::DeclaredBoundMismatch => "declared-bound-mismatch",
        }
    }
}

/// A bind refusal — the same five facts and rendered shape as
/// [`crate::ConfigError`], including [`NO_PARTIAL_LOAD_CLAUSE`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountError {
    /// The closed-set reason word's variant.
    pub reason: MountReason,
    /// The config file the offending mount block lives in.
    pub path: PathBuf,
    /// The 1-based FILE line of the offending mount block's opening fence.
    pub line: usize,
    /// What was found and what is legal.
    pub detail: String,
    /// The `Fix:` clause.
    pub fix: String,
}

impl std::fmt::Display for MountError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "refused: {} line {}: {} {NO_PARTIAL_LOAD_CLAUSE} Fix: {}",
            self.path.display(),
            self.line,
            self.detail,
            self.fix
        )
    }
}

impl std::error::Error for MountError {}

impl MountError {
    fn new(
        reason: MountReason,
        path: &Path,
        line: usize,
        detail: impl Into<String>,
        fix: impl Into<String>,
    ) -> MountError {
        MountError {
            reason,
            path: path.to_path_buf(),
            line,
            detail: detail.into(),
            fix: fix.into(),
        }
    }
}

/// What this machine can say about one bound root.
///
/// Closed, and every arm but [`MountState::Bound`] refuses — grey and red
/// alike ride exit 1 with their own reason word. There is no arm for
/// "unmounted": a root absent from the table is not a state of the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MountState {
    /// The path canonicalized, passed the ceiling, is unique and unnested; the
    /// root's own declaration names the bound name; and the claim, if any,
    /// verified. The acceptance half.
    Bound,
    /// The mount path does not exist here, or cannot be read. Not a parse
    /// failure — one root being absent from one machine is the topology
    /// working as designed (row M6).
    PathUnseeable {
        /// The underlying filesystem reason, verbatim.
        detail: String,
    },
    /// The root holds no [`DECLARATION_FILENAME`]. Grey, with the missing file
    /// named — never red.
    Undeclared {
        /// The declaration file this bind looked for and did not find.
        declaration: PathBuf,
    },
    /// A declaration file is there but does not read as one. Present is not
    /// absent, and a foreign root's broken content must not fail this
    /// machine's whole parse.
    DeclarationUnreadable {
        /// The declaration file that would not read.
        declaration: PathBuf,
        /// What is wrong with it.
        detail: String,
    },
    /// A pin is carried but this build cannot decide it — an unimplemented
    /// triple member, or a declaration whose canonicalization is empty. Grey,
    /// never green.
    ClaimUnverifiable {
        /// Which member, or which condition, blocks the verdict.
        detail: String,
    },
    /// The claim was decided and the root's declaration drifted. Red: this is
    /// not the edge of sight, it is a measured disagreement.
    Drifted {
        /// The token the mount entry pinned.
        pinned: String,
        /// The declaration's live fingerprint — the re-pin candidate.
        live: String,
    },
}

/// This plane's wrapped spelling of the shared reason word. The bare word
/// lives in [`addr::PATH_UNSEEABLE_REASON_WORD`]; the `const` assertion below
/// fails the build if the two ever drift.
const PATH_UNSEEABLE_WRAPPED: &str = "grey(path-unseeable)";

/// Is `wrapped` exactly `grey(<bare>)`? A `const fn` so the check runs at
/// compile time; `&str` equality is not itself const-evaluable, so the bytes are
/// walked.
const fn wraps_bare_word(wrapped: &str, bare: &str) -> bool {
    let (w, b) = (wrapped.as_bytes(), bare.as_bytes());
    if w.len() != b.len() + 6 {
        return false;
    }
    // "grey(" … ")"
    if w[0] != b'g' || w[1] != b'r' || w[2] != b'e' || w[3] != b'y' || w[4] != b'(' {
        return false;
    }
    if w[w.len() - 1] != b')' {
        return false;
    }
    let mut i = 0;
    while i < b.len() {
        if w[5 + i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

const _: () = assert!(
    wraps_bare_word(PATH_UNSEEABLE_WRAPPED, addr::PATH_UNSEEABLE_REASON_WORD),
    "the mount plane's wrapped spelling must be exactly `grey(<the shared bare word>)` (S3-R49)"
);

impl MountState {
    /// The reason word: `bound`, a `grey(...)`, or a `red(...)`. One spelling,
    /// used in the human line and in `--json` alike.
    #[must_use]
    pub fn word(&self) -> &'static str {
        match self {
            MountState::Bound => "bound",
            MountState::PathUnseeable { .. } => PATH_UNSEEABLE_WRAPPED,
            MountState::Undeclared { .. } => "grey(undeclared)",
            MountState::DeclarationUnreadable { .. } => "grey(declaration-unreadable)",
            MountState::ClaimUnverifiable { .. } => "grey(claim-unverifiable)",
            MountState::Drifted { .. } => "red(content-drifted)",
        }
    }

    /// True when this state refuses — every state but [`MountState::Bound`].
    /// Grey refuses exactly as red does, on exit 1, each with its own reason
    /// word.
    #[must_use]
    pub fn refuses(&self) -> bool {
        !matches!(self, MountState::Bound)
    }

    /// The teaching sentence beside the reason word — what was looked for,
    /// where, and what to do. Empty for [`MountState::Bound`].
    #[must_use]
    pub fn detail(&self) -> String {
        match self {
            MountState::Bound => String::new(),
            MountState::PathUnseeable { detail } => {
                format!(
                    "the mount path cannot be read here ({detail}) — the table stays loaded and this one root is unseeable. Fix: check out the root at that path, or remove the mount."
                )
            }
            MountState::Undeclared { declaration } => format!(
                "the root binds, but declares no canonical name of its own: {} does not exist. The root declares and MERIDIAN.md binds, so an undeclared root cannot be checked. Fix: add {DECLARATION_FILENAME} at the root's top level with `type: {DECLARATION_TYPE}` and `name:`.",
                declaration.display()
            ),
            MountState::DeclarationUnreadable {
                declaration,
                detail,
            } => format!(
                "{} does not read as a root declaration: {detail}. Fix: give it `type: {DECLARATION_TYPE}`, `version: {VERSION}`, and a `name:` in the canonical charset.",
                declaration.display()
            ),
            MountState::ClaimUnverifiable { detail } => format!(
                "the mount pins this root, but the claim cannot be decided here: {detail}. Fix: upgrade the engine, or remove the `pin:` line."
            ),
            MountState::Drifted { pinned, live } => format!(
                "the mount pins this root at {pinned}, but its declaration now fingerprints {live} — the declared content drifted. Fix: re-read the root's declaration and re-pin, or restore the content."
            ),
        }
    }
}

/// One bound root: a declared [`MountEntry`] plus everything binding it
/// decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    name: String,
    declared_path: String,
    canonical: Option<PathBuf>,
    kind: MountKind,
    primary: bool,
    vault: Option<String>,
    pin: Option<String>,
    declared_name: Option<String>,
    fence_line: usize,
    state: MountState,
}

impl Mount {
    /// The canonical root name this table binds the root to.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The path exactly as the config file spells it.
    #[must_use]
    pub fn declared_path(&self) -> &str {
        &self.declared_path
    }

    /// The canonicalized path — `Some` iff the path was readable at bind. This
    /// is the only spelling any comparison uses; the declared one is for
    /// refusals a human reads.
    #[must_use]
    pub fn canonical_path(&self) -> Option<&Path> {
        self.canonical.as_deref()
    }

    /// The root's kind.
    #[must_use]
    pub fn kind(&self) -> MountKind {
        self.kind
    }

    /// The declared-primary designation, verbatim from the config (schema
    /// §5.1). A binding ROLE fleet hosts consume; the engine reports it and
    /// never acts on it.
    #[must_use]
    pub fn primary(&self) -> bool {
        self.primary
    }

    /// The Obsidian vault name — `Some` iff [`MountKind::Vault`]. This is the
    /// partial leg of the three-way map.
    #[must_use]
    pub fn vault(&self) -> Option<&str> {
        self.vault.as_deref()
    }

    /// The mount-as-claim token, verbatim.
    #[must_use]
    pub fn pin(&self) -> Option<&str> {
        self.pin.as_deref()
    }

    /// The name the ROOT declares for itself — `Some` only when its declaration
    /// was found and read. Equal to [`Mount::name`] whenever it is `Some`, since
    /// a disagreement fails the whole parse.
    #[must_use]
    pub fn declared_name(&self) -> Option<&str> {
        self.declared_name.as_deref()
    }

    /// The 1-based FILE line of this mount's opening fence in the config.
    #[must_use]
    pub fn fence_line(&self) -> usize {
        self.fence_line
    }

    /// What this machine can say about the root.
    #[must_use]
    pub fn state(&self) -> &MountState {
        &self.state
    }
}

/// The bound mount table.
///
/// The field is private and [`bind`] is the only constructor, so no partial
/// mount table can exist to be observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountTable {
    mounts: Vec<Mount>,
}

impl MountTable {
    /// Every bound root, in document order.
    #[must_use]
    pub fn mounts(&self) -> &[Mount] {
        &self.mounts
    }

    /// name → (vault, path). The mount bound to a canonical root name.
    #[must_use]
    pub fn by_name(&self, name: &str) -> Option<&Mount> {
        self.mounts.iter().find(|m| m.name == name)
    }

    /// path → (name, vault). The mount bound to a local path.
    ///
    /// The argument is canonicalized by the same rule [`bind`] used, so a
    /// symlinked or trailing-slash spelling finds the mount it names.
    #[must_use]
    pub fn by_path(&self, path: &Path) -> Option<&Mount> {
        let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.mounts
            .iter()
            .find(|m| m.canonical.as_deref() == Some(target.as_path()))
    }

    /// vault → (name, path). The mount naming an Obsidian vault. Partial by
    /// construction: a `git-folder` root has no vault name and is never found
    /// here.
    #[must_use]
    pub fn by_vault(&self, vault: &str) -> Option<&Mount> {
        self.mounts
            .iter()
            .find(|m| m.vault.as_deref() == Some(vault))
    }

    /// True when every mount is [`MountState::Bound`] — nothing grey, nothing
    /// red. The predicate a verb's exit code answers.
    #[must_use]
    pub fn is_clear(&self) -> bool {
        self.mounts.iter().all(|m| !m.state.refuses())
    }

    /// The table, projected for the planes that resolve and translate.
    ///
    /// Carries which names this machine binds, the vault name each bound vault
    /// root carries, and which declared names are unreachable here with the
    /// path to check. A refusing mount is recorded as unreachable rather than
    /// dropped, so "declared but unreadable" is not collapsed into "nobody
    /// declared it".
    ///
    /// Not `mrd walk`'s projection: `walk_cmd::load_mounts` also marks a root
    /// unreachable when its corpus will not build. The two agree wherever they
    /// overlap because both read `Mount::state`.
    #[must_use]
    pub fn projection(&self) -> addr::MountSet {
        let mut bound: Vec<addr::MountName> = Vec::new();
        let mut unreachable: Vec<(addr::MountName, &Mount)> = Vec::new();
        let mut vaults: Vec<(addr::MountName, &str)> = Vec::new();
        for mount in &self.mounts {
            // Not a canonical name — no address can reach it anyway.
            let Ok(name) = addr::MountName::parse(mount.name()) else {
                continue;
            };
            if mount.state().refuses() {
                unreachable.push((name, mount));
                continue;
            }
            if let Some(vault) = mount.vault() {
                vaults.push((name.clone(), vault));
            }
            bound.push(name);
        }
        let mut set = addr::MountSet::new(bound);
        for (name, vault) in vaults {
            set = set.with_vault(name, vault);
        }
        for (name, mount) in unreachable {
            let detail = match mount.state() {
                MountState::PathUnseeable { detail } => detail.clone(),
                other => other.detail(),
            };
            set = set.with_unreachable(name, mount.declared_path().to_owned(), detail);
        }
        set
    }
}

impl Resolution {
    /// Bind the resolved config's declared mounts.
    ///
    /// State A (absent) and state D (zero mounts) both produce the empty table
    /// through this one call.
    ///
    /// # Errors
    /// [`MountError`] when a mount may not be bound at all. The refusal fails
    /// the whole table: there is no partially-bound value to return.
    pub fn bind(&self) -> Result<MountTable, MountError> {
        match self {
            Resolution::Absent { .. } => Ok(MountTable { mounts: Vec::new() }),
            Resolution::Loaded(config) => bind(config),
        }
    }
}

/// Bind a parsed config's declared mounts into the mount table.
///
/// Entries are taken in document order, each carried through its own checks
/// before the next, so the first refusal in FILE order is the one reported
/// (schema §8.4). Within one entry: canonicalize, then the ceiling, then
/// uniqueness and nesting, then the declaration, then the claim — the ceiling
/// runs before anything reads inside the root.
///
/// # Errors
/// [`MountError`] — one of the closed set, naming the offending mount block's
/// line and stating that nothing was loaded.
pub fn bind(config: &Config) -> Result<MountTable, MountError> {
    let source = config.path();
    let mut mounts: Vec<Mount> = Vec::new();

    for entry in config.mounts() {
        let mount = bind_one(entry, source, &mounts)?;
        mounts.push(mount);
    }

    Ok(MountTable { mounts })
}

fn bind_one(entry: &MountEntry, source: &Path, bound: &[Mount]) -> Result<Mount, MountError> {
    let line = entry.fence_line;

    // Canonicalize first: `deny_reason` compares resolved paths, so checking an
    // uncanonicalized spelling would check a path that is not the one bound.
    let canonical = match std::fs::canonicalize(&entry.path) {
        Ok(canonical) => canonical,
        Err(e) => {
            // Unseeable is grey for this root, never a parse failure (row M6) —
            // failing would brick every machine not holding all declared roots.
            return Ok(mount_in_state(
                entry,
                None,
                None,
                MountState::PathUnseeable {
                    detail: e.to_string(),
                },
            ));
        }
    };

    // The ceiling, reused whole from `workspace` (row M1/M2).
    if let Some(reason) = workspace::deny_reason(&canonical) {
        return Err(MountError::new(
            MountReason::MountPathDenied,
            source,
            line,
            format!(
                "mount `{}` binds {}, which is the {reason} — a mount may not bind a path the workspace deny ceiling refuses, or the ceiling is bypassed by a config file that is itself ordinary editable content.",
                entry.name,
                canonical.display()
            ),
            "point `path:` at a directory the workspace ceiling admits, or remove the mount block.",
        ));
    }

    check_uniqueness(entry, source, bound, &canonical)?;

    // The root's own declaration: absent is grey, unreadable is grey; only a
    // read declaration that disagrees fails the parse (INV-5).
    let declaration_path = canonical.join(DECLARATION_FILENAME);
    let declaration = match read_declaration(&declaration_path) {
        Ok(declaration) => declaration,
        Err(DeclarationFault::Absent) => {
            return Ok(mount_in_state(
                entry,
                Some(canonical),
                None,
                MountState::Undeclared {
                    declaration: declaration_path,
                },
            ));
        }
        Err(DeclarationFault::Unreadable(detail)) => {
            return Ok(mount_in_state(
                entry,
                Some(canonical),
                None,
                MountState::DeclarationUnreadable {
                    declaration: declaration_path,
                    detail,
                },
            ));
        }
    };

    if declaration.name != entry.name {
        return Err(MountError::new(
            MountReason::DeclaredBoundMismatch,
            source,
            line,
            format!(
                "mount `{}` binds {}, but that root declares itself `{}` in {} — the root declares and MERIDIAN.md binds, so the two spellings must agree; a silent pick would make stored links mean different things on different machines.",
                entry.name,
                canonical.display(),
                declaration.name,
                declaration_path.display()
            ),
            format!(
                "rename the mount to `{}`, or change the root's own declaration to `{}`.",
                declaration.name, entry.name
            ),
        ));
    }

    // Mount-as-claim. Reuses `verify_content` whole; the pin's target is the
    // declaration file, so the claim protects exactly the artifact the check
    // above reads.
    let state = match entry.pin.as_deref() {
        None => MountState::Bound,
        Some(pin) => verdict_state(&declaration.document, pin),
    };

    Ok(mount_in_state(
        entry,
        Some(canonical),
        Some(declaration.name),
        state,
    ))
}

/// INV-2, INV-4 and INV-3, over canonicalized paths, against the entries
/// already bound — a refusal lands on the second occurrence and names the
/// first (§8.1a).
fn check_uniqueness(
    entry: &MountEntry,
    source: &Path,
    bound: &[Mount],
    canonical: &Path,
) -> Result<(), MountError> {
    let line = entry.fence_line;
    for prior in bound {
        // An unseeable prior has no canonical path, so it collides with
        // nothing.
        let Some(prior_path) = prior.canonical.as_deref() else {
            continue;
        };
        if prior_path == canonical {
            return Err(MountError::new(
                MountReason::DuplicateMountPath,
                source,
                line,
                format!(
                    "mount `{}` canonicalizes to {}, the same tree mount `{}` binds (declared at line {}) — a symlinked spelling and a trailing slash are two spellings of one tree, and one tree bound twice under two names yields two canonical refs over identical bytes, so a read receipt minted on one would gate a pin on the other.",
                    entry.name,
                    canonical.display(),
                    prior.name,
                    prior.fence_line
                ),
                "bind the tree under one name, and delete the other mount block.",
            ));
        }
        if let Some((outer, inner)) = containment(prior_path, canonical) {
            return Err(MountError::new(
                MountReason::NestedMount,
                source,
                line,
                format!(
                    "mount `{}` canonicalizes to {}, which lies inside {} — mount `{}` at line {} already binds the outer tree, and a document inside both would carry two canonical addresses.",
                    entry.name,
                    inner.display(),
                    outer.display(),
                    prior.name,
                    prior.fence_line
                ),
                "bind the outer root only, or move the inner root out from under it.",
            ));
        }
        if let (Some(vault), Some(prior_vault)) = (entry.vault.as_deref(), prior.vault.as_deref())
            && vault == prior_vault
        {
            return Err(MountError::new(
                MountReason::DuplicateVaultName,
                source,
                line,
                format!(
                    "mount `{}` names the Obsidian vault `{vault}`, already named by mount `{}` at line {} — the mount table is a three-way map, so a vault name is a key, and a stored `obsidian://` URI carrying it would name two roots.",
                    entry.name, prior.name, prior.fence_line
                ),
                "give the two roots distinct Obsidian vault names, or delete the duplicate mount block.",
            ));
        }
    }
    Ok(())
}

fn verdict_state(document: &model::Document, pin: &str) -> MountState {
    match model::fingerprint::verify_content(document, &document.root, pin) {
        model::fingerprint::ContentVerdict::Green => MountState::Bound,
        model::fingerprint::ContentVerdict::Red { actual } => MountState::Drifted {
            pinned: pin.to_string(),
            live: actual.into_string(),
        },
        verdict @ model::fingerprint::ContentVerdict::Unverifiable { .. } => {
            MountState::ClaimUnverifiable {
                detail: format!(
                    "the pinned token's {} is not implemented by this build",
                    verdict.unknown_members().join(" and ")
                ),
            }
        }
        model::fingerprint::ContentVerdict::EmptySpan => MountState::ClaimUnverifiable {
            detail: "the declaration canonicalizes to nothing, so there is no live fingerprint to compare".to_string(),
        },
        // Unreachable through `parse`, which already refuses a malformed
        // token; rendered rather than panicked so a hand-built `Config` still
        // gets a verdict.
        model::fingerprint::ContentVerdict::Malformed => MountState::ClaimUnverifiable {
            detail: "the pinned value is not a fingerprint token".to_string(),
        },
    }
}

fn mount_in_state(
    entry: &MountEntry,
    canonical: Option<PathBuf>,
    declared_name: Option<String>,
    state: MountState,
) -> Mount {
    Mount {
        name: entry.name.clone(),
        declared_path: entry.path.clone(),
        canonical,
        kind: entry.kind,
        primary: entry.primary,
        vault: entry.vault.clone(),
        pin: entry.pin.clone(),
        declared_name,
        fence_line: entry.fence_line,
        state,
    }
}

/// `Some((outer, inner))` when one path contains the other at a path-segment
/// boundary. `Path::starts_with` is component-wise, so `/a/wiki-two` is not
/// inside `/a/wiki` — the sibling case stays legal (row M5).
fn containment<'a>(first: &'a Path, second: &'a Path) -> Option<(&'a Path, &'a Path)> {
    if second.starts_with(first) {
        Some((first, second))
    } else if first.starts_with(second) {
        Some((second, first))
    } else {
        None
    }
}

/// A root's self-declaration, read.
///
/// Public because the run plane reads the same artifact for its own
/// convention table: what a valid root declaration is has one owner.
pub struct Declaration {
    /// The canonical root name the root claims for itself.
    pub name: String,
    /// The declaration parsed as content, for the consumer's own keys and for
    /// pin verification.
    pub document: model::Document,
}

/// Why a declaration did not produce a name. Absent and unreadable are kept
/// apart because they teach different fixes.
pub enum DeclarationFault {
    /// No [`DECLARATION_FILENAME`] at the root's top level.
    Absent,
    /// Present, but it does not read as a `meridian-root` declaration.
    Unreadable(String),
}

/// Read the self-declaration of the root at `root` — [`DECLARATION_FILENAME`]
/// at its top level.
///
/// # Errors
/// [`DeclarationFault::Absent`] when the root holds no declaration, or
/// [`DeclarationFault::Unreadable`] when one is present but does not read as
/// a `meridian-root` declaration.
pub fn read_root_declaration(root: &Path) -> Result<Declaration, DeclarationFault> {
    read_declaration(&root.join(DECLARATION_FILENAME))
}

/// Read a self-declaration from its exact path.
///
/// # Errors
/// [`DeclarationFault::Absent`] or [`DeclarationFault::Unreadable`].
pub fn read_declaration(path: &Path) -> Result<Declaration, DeclarationFault> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(DeclarationFault::Absent),
        Err(e) => return Err(DeclarationFault::Unreadable(e.to_string())),
    };

    let document = model::build(raw.clone(), syntax::parse(&raw));
    let fm = find_frontmatter(&document.root).ok_or_else(|| {
        DeclarationFault::Unreadable(
            "it does not open with a closed `---` frontmatter block".to_string(),
        )
    })?;
    let (inner, inner_first_line) = frontmatter_inner(&raw, fm);
    let value: serde_yaml::Value = serde_yaml::from_str(&inner)
        .map_err(|e| DeclarationFault::Unreadable(format!("its frontmatter is not YAML: {e}")))?;
    let map = value.as_mapping();
    let key = |name: &str| map.and_then(|m| m.get(serde_yaml::Value::from(name)));

    let declared_type = key("type").ok_or_else(|| {
        DeclarationFault::Unreadable(format!(
            "its frontmatter declares no `type: {DECLARATION_TYPE}`"
        ))
    })?;
    if declared_type.as_str() != Some(DECLARATION_TYPE) {
        return Err(DeclarationFault::Unreadable(format!(
            "its `type:` is `{}`, not `{DECLARATION_TYPE}`",
            scalar_text(declared_type)
        )));
    }

    let version = key("version")
        .ok_or_else(|| DeclarationFault::Unreadable("it declares no `version:`".to_string()))?;
    match version.as_u64() {
        Some(found) if found == VERSION => {}
        Some(found) => {
            return Err(DeclarationFault::Unreadable(format!(
                "it declares `version: {found}`, which this build does not implement"
            )));
        }
        None => {
            return Err(DeclarationFault::Unreadable(format!(
                "its `version:` is `{}`, which is not an integer",
                scalar_text(version)
            )));
        }
    }

    let name = key("name")
        .ok_or_else(|| DeclarationFault::Unreadable("it declares no `name:`".to_string()))?;
    let Some(name) = name.as_str() else {
        return Err(DeclarationFault::Unreadable(format!(
            "its `name:` is `{}`, which is not a name",
            scalar_text(name)
        )));
    };

    // The charset has one owner; its refusal becomes the grey's teaching
    // detail rather than a parse failure.
    check_name(
        name,
        path,
        key_line(&inner, inner_first_line, "name"),
        "a canonical root name",
    )
    .map_err(|e| DeclarationFault::Unreadable(e.detail))?;

    Ok(Declaration {
        name: name.to_string(),
        document,
    })
}

#[cfg(test)]
mod tests;
