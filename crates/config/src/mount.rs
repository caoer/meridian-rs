//! The mount table — where a declared mount entry becomes a **bound** root.
//!
//! # Charter
//! **Owns:** canonicalization at bind, the `workspace::deny_reason` ceiling,
//! the equal-or-nested refusal, the three-way map's uniqueness invariants, the
//! read of each root's own self-declaration and the declared-vs-bound check,
//! and mount-as-claim — the pin a mount carries over the root it declares.
//!
//! **Never does:** resolve an address. Which corpus a `root:` prefix selects,
//! and what an unmounted root renders, are U11's (`docs/address-grammar.md`
//! §5, §6). This module answers only *which roots does this machine bind, and
//! what can it say about each*.
//!
//! # The three-way map, and the arithmetic that closes it (R32)
//!
//! The mount table is the single authority for the three-way translation —
//! **canonical root name ↔ Obsidian vault name ↔ local path**
//! (`2026-07-24-cross-root-addressing.md` §2). Three representations give a
//! **3 × 3 = 9-cell** matrix, and every cell is accounted for exactly once: the
//! **diagonal** is "this representation is a key" (the three uniqueness
//! invariants), the **off-diagonal** is the six directed translations. Nothing
//! is counted twice, because the nine cells are the whole product set.
//!
//! | from ↓ / to → | **name** | **vault** | **path** |
//! |---|---|---|---|
//! | **name** | INV-1 — no two entries share a name. Enforced at PARSE ([`crate::Reason::DuplicateMountName`], schema §5.1) | name → vault: [`Mount::vault`]. **Partial** — `None` on a `git-folder` entry | name → path: [`MountTable::by_name`] then [`Mount::canonical_path`] |
//! | **vault** | vault → name: [`MountTable::by_vault`] then [`Mount::name`]. **Partial domain** — `vault`-kind entries only | INV-3 — no two entries share a vault name. [`MountReason::DuplicateVaultName`] | vault → path: [`MountTable::by_vault`] then [`Mount::canonical_path`]. **Partial domain** |
//! | **path** | path → name: [`MountTable::by_path`] then [`Mount::name`] | path → vault: [`MountTable::by_path`] then [`Mount::vault`]. **Partial** | INV-2 — no two entries share a canonicalized path, and INV-4 — none contains another. [`MountReason::DuplicateMountPath`], [`MountReason::NestedMount`] |
//!
//! **The count, stated:** 9 cells = 3 diagonal (INV-1, INV-3, INV-2+INV-4) + 6
//! off-diagonal translations. Of the six, **two are total** (name ↔ path) and
//! **four are partial** (every cell on the vault row or column), because a
//! `git-folder` root has no Obsidian vault. That partiality is the one fact a
//! reader would otherwise assume away: **the map is a bijection on the
//! name↔path axis and only an injection on the vault axis**, since INV-3 holds
//! vacuously over the `git-folder` entries that carry no vault name at all.
//!
//! `is_bound_by_this_machine` is deliberately absent from the matrix: it is a
//! predicate over one entry, not a translation between two representations.
//!
//! # The ceiling — why a mount is not just a path
//!
//! `MERIDIAN.md` is ordinary editable content. Without a ceiling at bind, a
//! mount binding `$HOME` or `/` would hand the whole filesystem to every plane
//! that resolves through the table — **the workspace deny ceiling bypassed by a
//! file that is itself ordinary editable content**. So every mount path is
//! canonicalized and then passed through [`workspace::deny_reason`], the SAME
//! predicate the workspace ladder uses; it is reused here, never re-implemented
//! (`docs/address-grammar.md` §8 B-2).
//!
//! A refused mount **fails the whole parse**. Like [`crate::Config`],
//! [`MountTable`]'s field is private and [`bind`] is its only constructor, so a
//! partially-bound table cannot exist to be observed.
//!
//! # Canonicalize first, and refuse equal-or-nested (S3-R7)
//!
//! Measured on this machine, before the code existed:
//! `/Users/Shared/repos/field-notes` is a **symlink** to
//! `/Users/Shared/projects/field-notes`, while `CCC_LLM_WIKI_PATH` carries the
//! real path **with a trailing slash**. A literal env-var inversion therefore
//! binds **one tree twice under two names** — two canonical refs over identical
//! bytes, with identical `sec_rev`, which **the read-mint recheck cannot tell
//! apart: a receipt minted on ref A would gate a pin on ref B.** That is a
//! read-mint bypass, and only canonicalization collapses both spellings.
//!
//! Nesting is refused on the same argument one level down: `/a/wiki` and
//! `/a/wiki/sub` bound under two names give one document two canonical
//! addresses. The prefix test is **path-segment-boundary** (`Path::starts_with`
//! is component-wise), so the sibling case `/a/wiki` + `/a/wiki-two` stays
//! legal — a naive string prefix would refuse it, and a mount law that refuses
//! legitimate siblings is a guard that blocks everything.
//!
//! # The root declares; `MERIDIAN.md` binds (D7)
//!
//! *"MERIDIAN.md binds, it doesn't baptize"* (`2026-07-24-cross-root-addressing.md`
//! §1a). A root's canonical name belongs to the root, because root names travel
//! inside stored, shared content — a name defined only in one user's config
//! would make links valid on exactly one machine. So a root declares its own
//! name in [`DECLARATION_FILENAME`] at its top level, and this module **checks**
//! the binding against it:
//!
//! - the two agree → the mount **binds** (the acceptance half, S3-R8(c));
//! - the two disagree → the **whole parse fails loud**, naming both spellings;
//! - the declaration is **absent** → the mount renders **grey**, naming the
//!   file it looked for. Not a mismatch, not a refusal of the table.
//!
//! The declaration is spelled in the **same reserved filename** as the config
//! ([`DECLARATION_FILENAME`] is [`crate::CONFIG_FILENAME`], one constant) and
//! discriminated by its `type:` key. That is not a collision, it is the
//! discriminator doing its job: `MERIDIAN_CONFIG` aimed at a root's declaration
//! refuses with [`crate::Reason::WrongTypeValue`] rather than half-loading an
//! unrelated page — which is precisely why schema §4 required the key. The two
//! can never be one file, because `$HOME` is a denied mount path
//! ([`workspace::DenyReason::HomeDir`]) and a root is never `$HOME`.
//!
//! # Mount-as-claim, and the residual it does NOT close
//!
//! A mount may **pin the root it declares** (schema §5.3). The pin's target is
//! the declaration file, and the two jobs reinforce each other: the pin
//! protects exactly the artifact the declared-vs-bound check reads, so tampering
//! with a root's declaration reddens the mount that trusts it. Verification
//! reuses [`model::fingerprint::verify_content`] whole — no new codec, no new
//! hash law, and an unimplemented triple member renders grey rather than green
//! (R26: outside sight never renders as verified).
//!
//! **What it does not close, stated rather than implied (S3-R10(b)):** a mount
//! pin protects the root a mount declares; it does **not** protect the table's
//! own *membership*, because deleting a mount block deletes its pin along with
//! it. What prevents silent passage there is S3-R6 — grey refuses on exit 1 —
//! not this mechanism.

use std::path::{Path, PathBuf};

use crate::{
    CONFIG_FILENAME, Config, MountEntry, MountKind, NO_PARTIAL_LOAD_CLAUSE, Resolution, VERSION,
    check_name, find_frontmatter, frontmatter_inner, key_line, scalar_text,
};

/// The reserved filename of a root's own self-declaration, at the root's top
/// level. Deliberately the **same** reserved name the config plane uses — one
/// constant, not two spellings — because the `type:` key is what tells them
/// apart, and a mis-aimed `MERIDIAN_CONFIG` must refuse loudly rather than
/// half-load (schema §4, §2.4).
pub const DECLARATION_FILENAME: &str = CONFIG_FILENAME;

/// The `type:` discriminator a root's self-declaration carries.
pub const DECLARATION_TYPE: &str = "meridian-root";

/// The declaration's required frontmatter keys, in canonical order. `name` is
/// the canonical root name the root claims for itself.
pub const DECLARATION_KEYS: [&str; 3] = ["type", "version", "name"];

/// Why a mount refused to bind. The closed set of the mount-path law
/// (`docs/address-grammar.md` §3 rows T2-T5 and §8 rows M1-M4).
///
/// A **separate** closed set from [`crate::Reason`], deliberately: that one is
/// schema §8.2's in-file parse vocabulary and is pinned word-for-word against
/// its own table. These are bind-time semantics with a different spec and a
/// different owner. One shared spelling would put two specs in one array and
/// let a change to either silently rewrite the other.
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

/// A bind refusal. The same five facts and the same rendered shape as
/// [`crate::ConfigError`] — reason word, config path, 1-based FILE line, what
/// was found, what is legal — because an operator reading the refusal is
/// looking at the same file either way, and [`NO_PARTIAL_LOAD_CLAUSE`] is the
/// one sentence both must carry.
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
/// Closed, and every arm but [`MountState::Bound`] **refuses** — grey and red
/// alike ride exit 1 with their own reason word (S3-R6). There is no arm for
/// "unmounted": a root absent from the table is not a state of the table, it is
/// U11's answer to an address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MountState {
    /// The path canonicalized, passed the ceiling, is unique and unnested; the
    /// root's own declaration names the bound name; and the claim, if any,
    /// verified. The acceptance half.
    Bound,
    /// The mount path does not exist here, or cannot be read. **Not a parse
    /// failure** — one root being absent from one machine is the topology
    /// working as designed (row M6).
    PathUnseeable {
        /// The underlying filesystem reason, verbatim.
        detail: String,
    },
    /// The root holds no [`DECLARATION_FILENAME`]. D7's absent case: grey, with
    /// the file that is missing named — never red, and never `file_not_found`.
    Undeclared {
        /// The declaration file this bind looked for and did not find.
        declaration: PathBuf,
    },
    /// A declaration file is there but does not read as one. The honest third
    /// arm: present is not absent, and a foreign root's broken content must not
    /// fail this machine's whole parse.
    DeclarationUnreadable {
        /// The declaration file that would not read.
        declaration: PathBuf,
        /// What is wrong with it.
        detail: String,
    },
    /// A pin is carried but this build cannot decide it — an unimplemented
    /// triple member, or a declaration whose canonicalization is empty. Grey,
    /// never green (R26).
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

/// This plane's WRAPPED spelling of the shared reason word (S3-R49).
///
/// **One source, checked at COMPILE TIME.** The bare word lives in
/// [`addr::PATH_UNSEEABLE_REASON_WORD`]; the address plane takes it bare and this
/// plane wraps it. The `const` assertion below fails the BUILD if the two ever
/// drift, so the planes agree by construction rather than by two literals that
/// happen to match today — which is what the ruling asked for and what a
/// string-equality test would only have detected after the fact.
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
    /// The reason word, in S3-R6's vocabulary: `bound`, a `grey(...)`, or a
    /// `red(...)`. One spelling, used in the human line and in `--json` alike.
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
    /// Grey refuses exactly as red does, on **exit 1**, each with its own
    /// reason word; there is no fourth exit code and no state that passes
    /// quietly (S3-R6).
    #[must_use]
    pub fn refuses(&self) -> bool {
        !matches!(self, MountState::Bound)
    }

    /// The teaching sentence beside the reason word — what was looked for,
    /// where, and what to do. Empty for [`MountState::Bound`], which teaches
    /// nothing because nothing is wrong.
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

/// One **bound** root: a declared [`MountEntry`] plus everything binding it
/// decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    name: String,
    declared_path: String,
    canonical: Option<PathBuf>,
    kind: MountKind,
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
/// The field is private and [`bind`] is the only constructor, so **no partial
/// mount table can exist to be observed** — the same property [`crate::Config`]
/// makes of a clean parse, carried one stage further.
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

    /// **name → (vault, path).** The mount bound to a canonical root name.
    #[must_use]
    pub fn by_name(&self, name: &str) -> Option<&Mount> {
        self.mounts.iter().find(|m| m.name == name)
    }

    /// **path → (name, vault).** The mount bound to a local path.
    ///
    /// The argument is canonicalized by the same rule [`bind`] used, because
    /// that is the whole measured point: a symlinked spelling and a
    /// trailing-slash spelling are the same tree, and a lookup that compared
    /// them literally would answer `None` for a path this table binds.
    #[must_use]
    pub fn by_path(&self, path: &Path) -> Option<&Mount> {
        let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.mounts
            .iter()
            .find(|m| m.canonical.as_deref() == Some(target.as_path()))
    }

    /// **vault → (name, path).** The mount naming an Obsidian vault. Partial by
    /// construction: a `git-folder` root has no vault name and is never found
    /// here.
    #[must_use]
    pub fn by_vault(&self, vault: &str) -> Option<&Mount> {
        self.mounts
            .iter()
            .find(|m| m.vault.as_deref() == Some(vault))
    }

    /// True when every mount is [`MountState::Bound`] — nothing grey, nothing
    /// red. This is the predicate a verb's exit code answers (S3-R6).
    #[must_use]
    pub fn is_clear(&self) -> bool {
        self.mounts.iter().all(|m| !m.state.refuses())
    }

    /// **The table, projected for the planes that resolve and translate** — the
    /// half `docs/laws.md` reserved for this crate (*"Still NOT here: projecting
    /// the bound names into `addr::MountSet`"*).
    ///
    /// Carries three facts and no paths beyond the ones a refusal must name:
    /// which names this machine BINDS, the **vault name** each bound vault root
    /// carries (the stored plane is spelled in vault names — U12), and which
    /// declared names are **unreachable here**, with the path to check.
    ///
    /// A refusing mount is recorded as unreachable rather than dropped —
    /// S3-R50: dropping it collapses *"declared but unreadable"* into *"nobody
    /// declared it"* one frame upstream of the refusal, and the refusal then
    /// prescribes a declaration that already exists.
    ///
    /// **This is not `mrd walk`'s projection and the difference is a FACT, not
    /// a second spelling.** `walk_cmd::load_mounts` also marks a root
    /// unreachable when its CORPUS will not build — a fact only a caller
    /// holding corpora can know. This projection answers what the TABLE knows.
    /// The two agree wherever they overlap because both read `Mount::state`.
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
    /// through this one call — the nil-vs-empty identity the config plane draws,
    /// carried into binding rather than re-decided here.
    ///
    /// # Errors
    /// [`MountError`] when a mount may not be bound at all. The refusal fails
    /// the WHOLE table: there is no partially-bound value to return.
    pub fn bind(&self) -> Result<MountTable, MountError> {
        match self {
            Resolution::Absent { .. } => Ok(MountTable { mounts: Vec::new() }),
            Resolution::Loaded(config) => bind(config),
        }
    }
}

/// Bind a parsed config's declared mounts into the mount table.
///
/// Each entry is taken in document order and carried through its own checks
/// before the next entry is looked at, so the first refusal in FILE order is
/// the one reported — schema §8.4's rule, applied to binding. **One consequence
/// is stated rather than discovered:** an entry's declared-vs-bound mismatch
/// masks a later entry's path collision. Both are fatal and neither is more
/// urgent, so file order stays the single rule; ranking the classes against
/// each other would be a second rule to get wrong.
///
/// The order within one entry is not arbitrary: **canonicalize, then the
/// ceiling, then uniqueness and nesting, then the declaration, then the claim.**
/// The ceiling runs before anything reads *inside* the root, so a mount that
/// may not be bound never causes a read at the path it names.
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

    // Canonicalize FIRST. `deny_reason` compares resolved paths, so checking an
    // uncanonicalized spelling would check a path that is not the one bound.
    let canonical = match std::fs::canonicalize(&entry.path) {
        Ok(canonical) => canonical,
        Err(e) => {
            // Row M6: unseeable is grey for this root, never a parse failure —
            // failing here would brick the CLI on every machine that does not
            // hold all of the declared roots.
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

    // The root's own declaration. Absent is grey, unreadable is grey, and only
    // a READ declaration that disagrees fails the parse (D7, INV-5).
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

/// INV-2, INV-4 and INV-3, over CANONICALIZED paths, against the entries already
/// bound — so a refusal lands on the **second** occurrence and names the first,
/// which is §8.1a's rule for a duplicate.
fn check_uniqueness(
    entry: &MountEntry,
    source: &Path,
    bound: &[Mount],
    canonical: &Path,
) -> Result<(), MountError> {
    let line = entry.fence_line;
    for prior in bound {
        // An unseeable prior has no canonical path, so it collides with
        // nothing. That is honest rather than lenient: a tree this machine
        // cannot resolve cannot be shown to be the same tree as another.
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
        // Unreachable through `parse`, which already refuses a malformed token
        // with `bad-value`. Rendered rather than panicked: a caller holding a
        // hand-built `Config` must get a verdict, not an abort.
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
        vault: entry.vault.clone(),
        pin: entry.pin.clone(),
        declared_name,
        fence_line: entry.fence_line,
        state,
    }
}

/// `Some((outer, inner))` when one path contains the other at a **path-segment
/// boundary**. `Path::starts_with` is component-wise, so `/a/wiki-two` is not
/// inside `/a/wiki` — the sibling case row M5 requires to stay legal.
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
struct Declaration {
    name: String,
    document: model::Document,
}

/// Why a declaration did not produce a name. Absent and unreadable are kept
/// apart because they teach different fixes — and because collapsing them would
/// report a broken declaration as no declaration.
enum DeclarationFault {
    Absent,
    Unreadable(String),
}

fn read_declaration(path: &Path) -> Result<Declaration, DeclarationFault> {
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

    // The charset has ONE owner. Reused rather than re-spelled, and its refusal
    // becomes the grey's teaching detail instead of a parse failure — a root's
    // broken content must not fail this machine's whole parse.
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
