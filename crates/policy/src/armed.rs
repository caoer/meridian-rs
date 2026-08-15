//! Attested armed-set artifact (INDEX successor) — the ARM act, the § 4 row
//! grammar, and selection of what governs a path.
//!
//! [`arm`] is the ONE act that turns a discovered page into an armed one: it
//! narrows to an [`ArmRoot`], resolves through the landed resolver, and pins
//! each winner's page + [`page_rev`] into an [`ArmedArtifact`] row keyed by
//! (id, arm root). Discovery makes a page known; only ARM activates it.
//!
//! # Row law
//! - Key is (id, arm root); mode must be one the page kind admits.
//! - Selection for a write path is nearest-wins under § 3 narrowing.
//! - Fail-closed on corrupt/missing artifact once the workspace has been armed.
//! - Sibling subtrees do not couple: a CHECK armed at `sessions/a` must not
//!   refuse a write under `sessions/b`.

use std::collections::{BTreeMap, BTreeSet};

use crate::registration::{RuleId, RuleIndex, RuleKind, ScopeLayer, page_rev};

/// The workspace path the attested armed-set artifact lives at — sibling of the
/// once-armed marker (`meridian/attested`), under the engine-managed `meridian/`
/// directory.
///
/// Naming it here does not protect it: mirroring it into `fs::domain` and the
/// binding law's reserved set is the door's wiring — a named gap.
pub const ARMED_RULES_PATH: &str = "meridian/armed-rules.md";

/// The artifact page's title. A strict parse requires it — an armed-set page whose
/// title is gone is refused, never read as "nothing armed".
const ARTIFACT_TITLE: &str = "# Attested armed rules";

/// The § 4 column header, byte-exact. A page whose header differs is not this
/// artifact, and reading it as one would silently reinterpret its columns.
const ARTIFACT_HEADER: &str = "| id | page | rev | scope | mode |";

/// The header underline markdown requires between a table head and its body.
const ARTIFACT_RULE: &str = "| --- | --- | --- | --- | --- |";

/// The artifact's fixed preamble, teaching the row grammar and the arming law so
/// the page is self-describing where it is read.
const ARTIFACT_PREAMBLE: &str = "\
One row per (id, arm root). `scope` is the ARM ROOT — the root the resolution was \
narrowed to — and is part of the row key, so sibling scopes may arm the same id. At \
a path, per id, the deepest row whose arm root contains it governs. `rev` is the \
page rev the row was attested at: if the page's bytes move, the row reddens and \
does not fire. Arming freezes resolution — a page that appears later does not enter \
this set until a re-arm, and nothing arms by tag alone.";

// ── mode ──────────────────────────────────────────────────────────────────────

/// How an armed row acts, in the vocabulary its kind admits (§ 4).
///
/// `warn`/`block` is check-enforcement vocabulary; a hook can never veto or
/// mutate, so hook activation is binary ([`ArmFault::ModeKind`]). A row's kind
/// is recoverable from its mode (`warn`/`block` ⇒ check, `armed` ⇒ hook), so
/// the artifact needs no sixth column; `off` alone is shared and never fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Mode {
    /// Attested-off: the reviewer read the page at this rev and chose not to
    /// activate it. Legal for both kinds; never fires.
    Off,
    /// Check-only — the door annotates the write but lands it.
    Warn,
    /// Check-only — the door refuses the write.
    Block,
    /// Hook-only — the reaction fires. Binary by law: a hook has no severity axis.
    Armed,
}

impl Mode {
    /// The mode word the artifact renders and a strict parse reads back.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Off => "off",
            Mode::Warn => "warn",
            Mode::Block => "block",
            Mode::Armed => "armed",
        }
    }

    /// The mode a word names, or `None` when it is outside the closed vocabulary.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "off" => Some(Mode::Off),
            "warn" => Some(Mode::Warn),
            "block" => Some(Mode::Block),
            "armed" => Some(Mode::Armed),
            _ => None,
        }
    }

    /// Whether `kind` admits this mode — the § 4 vocabulary split.
    ///
    /// Matched on the mode so the match stays exhaustive: adding a mode without
    /// deciding which kind admits it is a compile error, not a silent `false`.
    #[must_use]
    pub fn admits(self, kind: RuleKind) -> bool {
        match self {
            Mode::Off => true,
            Mode::Warn | Mode::Block => kind == RuleKind::Check,
            Mode::Armed => kind == RuleKind::Hook,
        }
    }

    /// Whether this mode acts at all. `off` is attested but inert.
    #[must_use]
    pub fn fires(self) -> bool {
        self != Mode::Off
    }

    /// Whether this mode speaks check-ENFORCEMENT vocabulary — the modes whose row,
    /// when red, must refuse the write rather than fall silent.
    #[must_use]
    pub fn enforces(self) -> bool {
        matches!(self, Mode::Warn | Mode::Block)
    }

    /// The vocabulary `kind` admits, spelled for a refusal that teaches it.
    #[must_use]
    pub fn vocabulary(kind: RuleKind) -> &'static str {
        match kind {
            RuleKind::Check => "off|warn|block",
            RuleKind::Hook => "off|armed",
        }
    }
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ── the arm root ──────────────────────────────────────────────────────────────

/// The root an ARM act narrowed its resolution to — the `scope` column, and half of
/// the row key. A workspace-relative DIRECTORY path; the workspace root is the empty
/// path, rendered `.`.
///
/// Construction is sealed to [`ArmRoot::parse`], so a root in hand is a legal,
/// renderable, non-escaping directory.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArmRoot(String);

impl ArmRoot {
    /// The workspace root — the outermost arm root, containing every path.
    #[must_use]
    pub fn workspace() -> Self {
        ArmRoot(String::new())
    }

    /// Parse an arm root from a workspace-relative directory path. `""` and `"."`
    /// both name the workspace root.
    ///
    /// # Errors
    /// [`PathFault`] — the path escapes the workspace, is absolute, is spelled two
    /// ways, or carries a character that would forge the artifact's row grammar.
    pub fn parse(dir: &str) -> Result<Self, PathFault> {
        let dir = if dir == "." { "" } else { dir };
        if dir.is_empty() {
            return Ok(ArmRoot::workspace());
        }
        validate_workspace_path(dir)?;
        Ok(ArmRoot(dir.to_string()))
    }

    /// The root as a workspace-relative directory (`""` at the workspace root).
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// How deep the root sits — the workspace root is 0. Among the roots that
    /// contain one path, deeper is nearer, which IS the selection law's order.
    #[must_use]
    pub fn depth(&self) -> usize {
        if self.0.is_empty() {
            0
        } else {
            self.0.split('/').count()
        }
    }

    /// Whether a write at `path` falls under this root — the root is the path's
    /// ancestor, or the path itself.
    #[must_use]
    pub fn contains(&self, path: &str) -> bool {
        is_ancestor_or_self(&self.0, path)
    }
}

impl std::fmt::Display for ArmRoot {
    /// Renders the workspace root as `.` — an empty table cell would be ambiguous
    /// with a missing one.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.is_empty() {
            f.write_str(".")
        } else {
            f.write_str(&self.0)
        }
    }
}

/// Whether the directory `dir` is an ancestor of `path`, or names it exactly. The
/// empty `dir` is the workspace root and contains everything.
///
/// The separator is compared explicitly: a bare `starts_with` would read `a/bc.md`
/// as living under `a/b`.
fn is_ancestor_or_self(dir: &str, path: &str) -> bool {
    if dir.is_empty() {
        return true;
    }
    path == dir || path.starts_with(&format!("{dir}/"))
}

/// Why a workspace path may not enter the artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathFault {
    /// The path is absolute — the artifact records workspace-relative paths only.
    Absolute,
    /// The path carries a `..` segment and could escape the workspace.
    Escapes,
    /// The path carries a `.` segment, so one directory has two spellings.
    DotSegment,
    /// The path carries an empty segment — a doubled `/`, or a trailing one, either
    /// of which would give one directory two spellings.
    EmptySegment,
    /// The path carries a character that would forge the artifact's row grammar.
    Unrenderable {
        /// The offending character.
        found: char,
    },
    /// The first path segment carries `:` — under the address grammar a
    /// `root:`-bearing head is an ADDRESS qualifier (§ 4.2 D11), never a
    /// workspace path. The whole value rides along so the refusal can recognize
    /// the resolver's `layer:depth` scope spelling (`workspace:0` on a
    /// `mrd rules` chain line) and teach that specific confusion — the measured
    /// copy-paste trap was a scope cell that parsed clean and governed nothing.
    RootSeparator {
        /// The offered value, whole.
        value: String,
    },
}

/// Whether `value` is spelled like the resolver's `layer:depth` scope — one
/// word, a colon, digits. Recognized by SHAPE, not by today's layer names, so a
/// layer added later still earns the fitted teaching.
fn is_resolver_scope_spelling(value: &str) -> bool {
    value.split_once(':').is_some_and(|(layer, depth)| {
        !layer.is_empty()
            && layer.chars().all(|c| c.is_ascii_alphabetic())
            && !depth.is_empty()
            && depth.bytes().all(|b| b.is_ascii_digit())
    })
}

impl std::fmt::Display for PathFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PathFault::Absolute => {
                write!(
                    f,
                    "the path is absolute — the artifact records workspace-relative paths"
                )
            }
            PathFault::Escapes => write!(
                f,
                "the path carries a `..` segment and escapes the workspace"
            ),
            PathFault::DotSegment => write!(
                f,
                "the path carries a `.` segment — one directory, one spelling"
            ),
            PathFault::EmptySegment => write!(
                f,
                "the path carries an empty segment — a doubled or trailing `/`; one directory has \
                 one spelling"
            ),
            PathFault::Unrenderable { found } => write!(
                f,
                "the path carries {found:?} (U+{code:04X}), which would forge the artifact's own \
                 row grammar — a `|` opens a column, a backtick closes the cell's quoting, and a \
                 newline opens a whole row. The refusal makes the hostile bytes unrepresentable \
                 rather than escaped, so the next renderer inherits the guard",
                code = *found as u32
            ),
            PathFault::RootSeparator { value } => {
                if is_resolver_scope_spelling(value) {
                    write!(
                        f,
                        "`{value}` is the resolver's `layer:depth` scope spelling (the resolution \
                         chain of `mrd rules`), not a directory — the armed artifact's `scope` \
                         column takes the ARM ROOT the resolution was narrowed to: a \
                         workspace-relative directory path, `.` for the workspace root"
                    )
                } else {
                    write!(
                        f,
                        "the first path segment carries `:`, which the address grammar reserves \
                         for a `root:` qualifier — that spelling is an address, never a \
                         workspace path"
                    )
                }
            }
        }
    }
}

impl std::error::Error for PathFault {}

/// Validate a workspace path bound for a rendered cell.
///
/// The guard sits at intake so every renderer inherits it, making hostile bytes
/// unrepresentable rather than escaped. It refuses exactly what breaks a
/// backtick-quoted table cell: `|`, a backtick, and any control character. The
/// page renders as a plain path, never a wikilink, so `[`/`]`/`#`/`^` never
/// arise here. It also holds the address grammar's confinement line (§ 4.2
/// D11): a head segment carrying `:` is a `root:` qualifier — an address, never
/// a workspace path — which is what makes the resolver's `workspace:0` scope
/// spelling unrepresentable in a cell instead of silently inert.
fn validate_workspace_path(path: &str) -> Result<(), PathFault> {
    if path.starts_with('/') {
        return Err(PathFault::Absolute);
    }
    if let Some(found) = path
        .chars()
        .find(|c| *c == '|' || *c == '`' || c.is_control())
    {
        return Err(PathFault::Unrenderable { found });
    }
    // D11 exactly: only the HEAD segment's `:` is a root qualifier; a colon
    // after the first `/` is an ordinary path byte.
    if path.split('/').next().unwrap_or(path).contains(':') {
        return Err(PathFault::RootSeparator {
            value: path.to_string(),
        });
    }
    for segment in path.split('/') {
        match segment {
            ".." => return Err(PathFault::Escapes),
            "." => return Err(PathFault::DotSegment),
            "" => return Err(PathFault::EmptySegment),
            _ => {}
        }
    }
    Ok(())
}

// ── the row ───────────────────────────────────────────────────────────────────

/// One armed rule: exactly the § 4 columns, no more. Construction is sealed to
/// [`arm`] (the attested act) and [`parse_artifact`] (reading an attested page back),
/// so a row cannot be hand-built past the mode-vocabulary and renderability gates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArmedRow {
    id: RuleId,
    page: String,
    rev: String,
    scope: ArmRoot,
    mode: Mode,
}

impl ArmedRow {
    /// The armed rule's id.
    #[must_use]
    pub fn id(&self) -> &RuleId {
        &self.id
    }

    /// The workspace path of the page this id RESOLVED to at arm time.
    #[must_use]
    pub fn page(&self) -> &str {
        &self.page
    }

    /// The page rev the row is attested at. A page whose bytes no longer hash here
    /// has drifted, and the row reddens.
    #[must_use]
    pub fn rev(&self) -> &str {
        &self.rev
    }

    /// The arm root the resolution was narrowed to — half the row key.
    #[must_use]
    pub fn scope(&self) -> &ArmRoot {
        &self.scope
    }

    /// How the row acts.
    #[must_use]
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// The row key: (id, arm root). Two rows sharing it are a defect.
    fn key(&self) -> (&RuleId, &ArmRoot) {
        (&self.id, &self.scope)
    }

    /// Render one table row (no trailing newline).
    fn render(&self) -> String {
        format!(
            "| `{id}` | `{page}` | `{rev}` | `{scope}` | `{mode}` |",
            id = self.id,
            page = self.page,
            rev = self.rev,
            scope = self.scope,
            mode = self.mode,
        )
    }
}

// ── the ARM act ───────────────────────────────────────────────────────────────

/// One id a reviewer asks to arm, at the rev they read.
///
/// `attested_rev` is the attestation: the reviewer approved the page AT that rev. If
/// resolution now yields different bytes, the approval no longer covers the live law
/// and arming is refused ([`ArmFault::Drift`]) — never silently re-pinned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArmRequest {
    /// The id to arm.
    pub id: RuleId,
    /// The mode to arm it in, in the vocabulary its kind admits.
    pub mode: Mode,
    /// The page rev the reviewer read and approved.
    pub attested_rev: String,
}

/// Why an ARM act was refused. Every variant names the id, so a refused arming
/// always says which rule did not arm — a rule that silently failed to arm is a rule
/// that silently is not enforced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArmFault {
    /// The id resolves to nothing at this arm root — unregistered here, or collided.
    /// Arming cannot invent an effective set the resolver did not produce.
    Unresolved {
        /// The id that was asked for.
        id: RuleId,
        /// The root the resolution was narrowed to.
        root: ArmRoot,
    },
    /// The resolved page carries BOTH registration tags, so its row's `mode` column
    /// has no single vocabulary. Discovery admits such a page deliberately; arming
    /// is where the ambiguity has to be answered, and it is answered fail-closed.
    DualKind {
        /// The id that was asked for.
        id: RuleId,
        /// The dual-kind page.
        page: String,
    },
    /// The requested mode is outside the vocabulary the page's kind admits — a hook
    /// asked to `block`, or a check asked to be `armed`.
    ModeKind {
        /// The id that was asked for.
        id: RuleId,
        /// The resolved page.
        page: String,
        /// The kind the page registered as.
        kind: RuleKind,
        /// The mode that was asked for.
        mode: Mode,
    },
    /// The reviewer's approved rev is not the rev the page resolves to now — the law
    /// drifted between approval and arming.
    Drift {
        /// The id that was asked for.
        id: RuleId,
        /// The resolved page.
        page: String,
        /// The rev the reviewer approved.
        attested_rev: String,
        /// The rev the page resolves to now.
        resolved_rev: String,
    },
    /// The same id was requested twice in one act, so the act does not say what it
    /// attests.
    Duplicate {
        /// The doubly-requested id.
        id: RuleId,
    },
    /// The resolved page's path cannot be rendered into a row without forging it.
    Unrenderable {
        /// The id that was asked for.
        id: RuleId,
        /// The offending page path.
        page: String,
        /// Why it cannot be rendered.
        fault: PathFault,
    },
    /// **A named deferral, not a silent drop.** The id resolves to a USER-space page.
    /// § 4 defines the `page` column as the workspace path of the resolved page, so a
    /// user-space winner has no unambiguous spelling in a per-workspace artifact.
    /// Choosing one is a ruling question, not a loader's — so it is refused by name.
    UserLayerDeferred {
        /// The id that was asked for.
        id: RuleId,
        /// The user-space page that won.
        page: String,
    },
}

impl ArmFault {
    /// The id this refusal is about.
    #[must_use]
    pub fn id(&self) -> &RuleId {
        match self {
            ArmFault::Unresolved { id, .. }
            | ArmFault::DualKind { id, .. }
            | ArmFault::ModeKind { id, .. }
            | ArmFault::Drift { id, .. }
            | ArmFault::Duplicate { id }
            | ArmFault::Unrenderable { id, .. }
            | ArmFault::UserLayerDeferred { id, .. } => id,
        }
    }
}

impl std::fmt::Display for ArmFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArmFault::Unresolved { id, root } => write!(
                f,
                "id `{id}` resolves to nothing at arm root `{root}` — it is not registered on \
                 this chain, or it collided. Arming pins a RESOLVED set; it cannot arm an id \
                 the resolver refused"
            ),
            ArmFault::DualKind { id, page } => write!(
                f,
                "id `{id}` resolves to `{page}`, which carries BOTH `{check}` and `{hook}` — its \
                 row's mode has no single vocabulary ({check_vocab} for a check, {hook_vocab} for \
                 a hook). Split it into two pages with distinct ids, one per plane",
                check = RuleKind::Check.tag(),
                hook = RuleKind::Hook.tag(),
                check_vocab = Mode::vocabulary(RuleKind::Check),
                hook_vocab = Mode::vocabulary(RuleKind::Hook),
            ),
            ArmFault::ModeKind {
                id,
                page,
                kind,
                mode,
            } => write!(
                f,
                "id `{id}` (`{page}`) registered as `{tag}`, which arms in `{vocab}` — `{mode}` is \
                 not in that vocabulary. {why}",
                tag = kind.tag(),
                vocab = Mode::vocabulary(*kind),
                why = match kind {
                    RuleKind::Hook =>
                        "A hook can never veto or mutate a write, so it has no severity axis: it \
                         is off, or it fires",
                    RuleKind::Check =>
                        "`armed` is hook vocabulary; a check declares how hard it acts",
                },
            ),
            ArmFault::Drift {
                id,
                page,
                attested_rev,
                resolved_rev,
            } => write!(
                f,
                "id `{id}` was approved at rev `{attested_rev}` but `{page}` resolves to \
                 `{resolved_rev}` now — the law drifted between approval and arming. Re-read the \
                 page and arm at the live rev, or revert it"
            ),
            ArmFault::Duplicate { id } => write!(
                f,
                "id `{id}` was requested twice in one arm act — one act attests one mode per id, \
                 so a doubled request does not say what it attests"
            ),
            ArmFault::Unrenderable { id, page, fault } => write!(
                f,
                "id `{id}` resolves to `{page}`, which cannot be attested: {fault}"
            ),
            ArmFault::UserLayerDeferred { id, page } => write!(
                f,
                "id `{id}` resolves to the USER-space page `{page}`. The artifact's `page` column \
                 is a WORKSPACE path, so a user-space winner has no unambiguous spelling here — \
                 arming it is deferred by name rather than spelled ambiguously. Shadow the id \
                 with a workspace page to arm it in this workspace"
            ),
        }
    }
}

impl std::error::Error for ArmFault {}

/// **The ARM act.** Narrow to `root`, resolve, and pin the winners the requests name.
///
/// The one act that turns a discovered page into an armed one, as a single
/// indivisible step: narrow to `root`'s chain (§ 3), resolve through the landed
/// resolver ([`RuleIndex::resolve`]), pin the winner's page and rev. The caller
/// cannot interpose between narrowing and resolution, which keeps `scope`
/// truthful.
///
/// All-or-nothing, reporting every fault: a partially-landed artifact would
/// silently drop a rule the reviewer meant to arm.
///
/// # Errors
/// Every [`ArmFault`] the requests hit, in request order.
pub fn arm(
    index: &RuleIndex,
    root: &ArmRoot,
    requests: impl IntoIterator<Item = ArmRequest>,
) -> Result<ArmedArtifact, Vec<ArmFault>> {
    let effective = index.narrowed_to(root.as_str()).resolve();

    let mut rows: Vec<ArmedRow> = Vec::new();
    let mut faults = Vec::new();
    let mut seen: BTreeSet<RuleId> = BTreeSet::new();

    for request in requests {
        let ArmRequest {
            id,
            mode,
            attested_rev,
        } = request;

        if !seen.insert(id.clone()) {
            faults.push(ArmFault::Duplicate { id });
            continue;
        }

        let Some(effective_id) = effective.get(id.as_str()) else {
            faults.push(ArmFault::Unresolved {
                id,
                root: root.clone(),
            });
            continue;
        };
        let winner = effective_id.winner();

        if winner.scope().layer() == ScopeLayer::User {
            faults.push(ArmFault::UserLayerDeferred {
                id,
                page: winner.page().to_string(),
            });
            continue;
        }

        let [kind] = winner.kinds() else {
            faults.push(ArmFault::DualKind {
                id,
                page: winner.page().to_string(),
            });
            continue;
        };
        if !mode.admits(*kind) {
            faults.push(ArmFault::ModeKind {
                id,
                page: winner.page().to_string(),
                kind: *kind,
                mode,
            });
            continue;
        }

        if winner.rev() != attested_rev {
            faults.push(ArmFault::Drift {
                id,
                page: winner.page().to_string(),
                attested_rev,
                resolved_rev: winner.rev().to_string(),
            });
            continue;
        }

        if let Err(fault) = validate_workspace_path(winner.page()) {
            faults.push(ArmFault::Unrenderable {
                id,
                page: winner.page().to_string(),
                fault,
            });
            continue;
        }

        rows.push(ArmedRow {
            id,
            page: winner.page().to_string(),
            rev: winner.rev().to_string(),
            scope: root.clone(),
            mode,
        });
    }

    if faults.is_empty() {
        Ok(ArmedArtifact { rows })
    } else {
        Err(faults)
    }
}

// ── the artifact ──────────────────────────────────────────────────────────────

/// The attested armed-set artifact: the rows one or more ARM acts pinned.
///
/// Construction is sealed to [`arm`] and [`parse_artifact`]. The rendered page IS the
/// attestation — reading it back carries no gate, exactly as the INDEX it succeeds,
/// which is why [`parse_artifact`] is strict and fails closed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArmedArtifact {
    rows: Vec<ArmedRow>,
}

impl ArmedArtifact {
    /// Every armed row, in render order (arm root ascending, then id).
    #[must_use]
    pub fn rows(&self) -> &[ArmedRow] {
        &self.rows
    }

    /// Fold another act's rows in — how a per-workspace artifact accumulates arms at
    /// different roots.
    ///
    /// # Errors
    /// [`ArmFault::Duplicate`] per id already armed at the same root: (id, arm root)
    /// is the row key, so a repeat is a defect rather than an overwrite.
    pub fn merge(&mut self, other: ArmedArtifact) -> Result<(), Vec<ArmFault>> {
        let faults: Vec<ArmFault> = other
            .rows
            .iter()
            .filter(|row| self.rows.iter().any(|held| held.key() == row.key()))
            .map(|row| ArmFault::Duplicate { id: row.id.clone() })
            .collect();
        if !faults.is_empty() {
            return Err(faults);
        }
        self.rows.extend(other.rows);
        Ok(())
    }

    /// **The selection law.** The rows governing a write at `path`: per id, the
    /// deepest armed row whose arm root contains `path`.
    ///
    /// An inner arm shadows an outer arm on one chain; sibling scopes never
    /// interact. Selection orders already-attested rows only — it is not a
    /// re-resolution, so it cannot undo the freeze.
    #[must_use]
    pub fn select_at(&self, path: &str) -> Vec<&ArmedRow> {
        let mut nearest: BTreeMap<&RuleId, &ArmedRow> = BTreeMap::new();
        for row in &self.rows {
            if !row.scope.contains(path) {
                continue;
            }
            nearest
                .entry(&row.id)
                .and_modify(|held| {
                    if row.scope.depth() > held.scope.depth() {
                        *held = row;
                    }
                })
                .or_insert(row);
        }
        nearest.into_values().collect()
    }

    /// What governs a write at `path` — the one call a door or a feeder makes.
    ///
    /// Selects first over every attested row (a red row still shadows, which is
    /// what makes it fail closed), then verifies exactly the selected rows.
    /// Never compose these by hand: verify-then-select fails OPEN (a red inner
    /// row stops shadowing, so a stale outer row fires on its path), and
    /// verifying the whole artifact refuses TOO WIDE (a sibling scope's drift
    /// refuses this write).
    #[must_use]
    pub fn verify_at(&self, path: &str, pages: &dyn PageSource) -> ArmedVerdict {
        verify_rows(self.select_at(path).into_iter(), pages)
    }

    /// Re-hash every pinned page through the injected source and split the rows into
    /// those that still stand and those that reddened. A red row never fires on
    /// its new bytes.
    ///
    /// A whole-artifact HEALTH report, not a gate: it applies no selection, so
    /// neither `firing()` nor `refusing()` answers "what governs this write" —
    /// use [`ArmedArtifact::verify_at`] to decide anything.
    #[must_use]
    pub fn verify(&self, pages: &dyn PageSource) -> ArmedVerdict {
        verify_rows(self.rows.iter(), pages)
    }

    /// Render the full artifact page: title, preamble, and the § 4 table, terminated
    /// by a trailing newline. Rows sort by arm root then id — structural and
    /// deterministic, with no severity order invented across two kinds whose mode
    /// vocabularies do not share one.
    #[must_use]
    pub fn render(&self) -> String {
        let mut sorted: Vec<&ArmedRow> = self.rows.iter().collect();
        sorted.sort_by(|a, b| a.scope.cmp(&b.scope).then_with(|| a.id.cmp(&b.id)));
        let mut page = format!(
            "{ARTIFACT_TITLE}\n\n{ARTIFACT_PREAMBLE}\n\n{ARTIFACT_HEADER}\n{ARTIFACT_RULE}\n"
        );
        for row in sorted {
            page.push_str(&row.render());
            page.push('\n');
        }
        page
    }
}

/// Re-hash the given rows and split them into those that still stand and those that
/// reddened. The one implementation behind both [`ArmedArtifact::verify`] and
/// [`ArmedArtifact::verify_at`] — they differ only in which rows they hand it.
fn verify_rows<'a>(
    rows: impl Iterator<Item = &'a ArmedRow>,
    pages: &dyn PageSource,
) -> ArmedVerdict {
    let mut firing = Vec::new();
    let mut red = Vec::new();
    for row in rows {
        match pages.read(&row.page) {
            Err(e) => red.push(RedRow {
                row: row.clone(),
                why: Redness::Missing {
                    detail: e.to_string(),
                },
            }),
            Ok(bytes) => {
                let report_rev = page_rev(&bytes);
                if report_rev != row.rev {
                    red.push(RedRow {
                        row: row.clone(),
                        why: Redness::Drifted { report_rev },
                    });
                } else if let Some(why) = mode_outside_its_kind(row, &bytes) {
                    red.push(RedRow {
                        row: row.clone(),
                        why,
                    });
                } else if row.mode.fires() {
                    firing.push(row.clone());
                }
            }
        }
    }
    ArmedVerdict { firing, red }
}

/// Whether a row's mode is one its PAGE's kind admits, answered from the pinned
/// bytes rather than from the row.
///
/// The row cannot attest its own kind: the § 4 table has no `kind` column, so a
/// hand-edited hook row reading `block` parses clean — and would hand a hook the
/// veto the ruling denies it. Re-deriving the kind from the pinned page (already
/// read for the rev check, which proves these are the attested bytes) closes
/// that without a sixth column. A page that no longer registers at all reddens
/// too.
fn mode_outside_its_kind(row: &ArmedRow, bytes: &str) -> Option<Redness> {
    let registration = crate::registration::register_page(crate::registration::PageRef {
        layer: ScopeLayer::Workspace,
        page: &row.page,
        bytes,
    });
    let kinds = match registration {
        Ok(Some(registration)) => registration.kinds().to_vec(),
        // Refused, or no longer a rule page: either way it cannot say which
        // vocabulary its mode belongs to.
        _ => Vec::new(),
    };
    if kinds.iter().any(|kind| row.mode.admits(*kind)) {
        return None;
    }
    Some(Redness::mode_outside_kind(kinds))
}

/// The live bytes of a pinned page, injected — `policy` performs no I/O.
pub trait PageSource {
    /// Read the page at a workspace-relative path.
    ///
    /// # Errors
    /// Any I/O or decode failure. A page that cannot be read reddens its row; it
    /// never reads as unchanged.
    fn read(&self, page: &str) -> std::io::Result<String>;
}

/// Why a row reddened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Redness {
    /// The page is still there, but its bytes moved off the attested rev.
    Drifted {
        /// The rev the page reads NOW.
        report_rev: String,
    },
    /// The pinned page could not be read at all.
    Missing {
        /// The reader's own message.
        detail: String,
    },
    /// The row's mode is not in the vocabulary its PAGE's kind admits — the shape
    /// a hand-edited row takes when it reaches for a power its kind does not have
    /// (a hook row spelled `block`). Fails closed: the row does not fire.
    ModeOutsideKind {
        /// The kinds the pinned page registers (empty when it registers none).
        kinds: Vec<RuleKind>,
        /// The vocabulary those kinds admit, for the refusal's teaching half.
        vocabulary: &'static str,
    },
}

impl Redness {
    /// The mode-outside-kind redness for a page registering `kinds` — the one
    /// place the teaching vocabulary is derived, so verification and the law
    /// resolver ([`crate::armed_law`]) cannot drift into two spellings of it.
    pub(crate) fn mode_outside_kind(kinds: Vec<RuleKind>) -> Self {
        let vocabulary = kinds
            .first()
            .map_or("the page registers no rule kind", |kind| {
                Mode::vocabulary(*kind)
            });
        Redness::ModeOutsideKind { kinds, vocabulary }
    }
}

/// One red row: the attested row plus why it no longer stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedRow {
    row: ArmedRow,
    why: Redness,
}

impl RedRow {
    /// Mint a red row. Sealed to the crate: reddening is [`verify_rows`]' verdict
    /// and [`crate::armed_law`]'s when a pinned page moves between verification and
    /// load — never a caller's assertion.
    pub(crate) fn new(row: ArmedRow, why: Redness) -> Self {
        RedRow { row, why }
    }

    /// The row as attested.
    #[must_use]
    pub fn row(&self) -> &ArmedRow {
        &self.row
    }

    /// Why it reddened.
    #[must_use]
    pub fn why(&self) -> &Redness {
        &self.why
    }
}

impl std::fmt::Display for RedRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.why {
            Redness::Drifted { report_rev } => write!(
                f,
                "`{id}` is attested at rev `{rev}` but `{page}` reads `{report_rev}` now — the \
                 page was edited after arming, so the row does not fire on its new bytes. Re-arm \
                 at the live rev, or revert the page",
                id = self.row.id,
                rev = self.row.rev,
                page = self.row.page,
            ),
            Redness::Missing { detail } => write!(
                f,
                "`{id}` is attested against `{page}`, which cannot be read: {detail}",
                id = self.row.id,
                page = self.row.page,
            ),
            Redness::ModeOutsideKind { kinds, vocabulary } => write!(
                f,
                "`{id}` is armed `{mode}`, but `{page}` registers as {registers} — that mode is \
                 not in its vocabulary ({vocabulary}). A row reaching for a power its kind does \
                 not have does not fire; re-arm the page in a mode its kind admits",
                id = self.row.id,
                mode = self.row.mode.as_str(),
                page = self.row.page,
                registers = if kinds.is_empty() {
                    "no rule kind at all".to_string()
                } else {
                    kinds
                        .iter()
                        .map(|kind| format!("`{}`", kind.tag()))
                        .collect::<Vec<_>>()
                        .join(" + ")
                },
            ),
        }
    }
}

/// The result of verifying an artifact against live pages.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArmedVerdict {
    firing: Vec<ArmedRow>,
    red: Vec<RedRow>,
}

impl ArmedVerdict {
    /// The rows that stand and act: pinned rev intact, mode not `off`.
    #[must_use]
    pub fn firing(&self) -> &[ArmedRow] {
        &self.firing
    }

    /// Every row that reddened. None of them fire.
    #[must_use]
    pub fn red(&self) -> &[RedRow] {
        &self.red
    }

    /// The red rows that must REFUSE the write.
    ///
    /// The split is forced by the ruling: a red CHECK row is a law that cannot
    /// be evaluated, so it refuses; a red HOOK row falls silent — a hook may
    /// never veto — but stays red and reported.
    pub fn refusing(&self) -> impl Iterator<Item = &RedRow> {
        self.red.iter().filter(|r| r.row.mode.enforces())
    }
}

// ── reading an attested page back ─────────────────────────────────────────────

/// Why an artifact page is not a trustworthy attested armed set.
///
/// A corrupt artifact must never read as "nothing armed" — that would be a
/// gate-disabling edit dressed as a parse. Every fault is loud.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactCorrupt {
    /// What is structurally wrong with the page.
    pub detail: String,
}

impl std::fmt::Display for ArtifactCorrupt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "attested armed-set artifact is corrupt: {}", self.detail)
    }
}

impl std::error::Error for ArtifactCorrupt {}

/// Strictly parse an attested armed-set page — the fail-closed read path.
///
/// The page MUST open with the artifact title and MUST carry the § 4 header
/// byte-exactly; EVERY table row must parse into all five columns, inside the closed
/// mode vocabulary, with a legal id, a well-formed page rev, and a renderable path;
/// and no two rows may share (id, arm root). Anything else is refused, so a truncated
/// or tampered page never reads as an empty armed set.
///
/// # Errors
/// [`ArtifactCorrupt`], naming the fault.
pub fn parse_artifact(page: &str) -> Result<ArmedArtifact, ArtifactCorrupt> {
    let corrupt = |detail: String| ArtifactCorrupt { detail };

    let mut lines = page.lines();
    match lines.next() {
        Some(first) if first.trim_end() == ARTIFACT_TITLE => {}
        _ => {
            return Err(corrupt(format!(
                "missing the `{ARTIFACT_TITLE}` title header"
            )));
        }
    }
    if !page.lines().any(|l| l.trim_end() == ARTIFACT_HEADER) {
        return Err(corrupt(format!(
            "missing the column header `{ARTIFACT_HEADER}` — the page's columns cannot be trusted \
             to be the attested ones"
        )));
    }

    let mut rows: Vec<ArmedRow> = Vec::new();
    for line in page.lines() {
        let line = line.trim_end();
        if !line.starts_with('|') || line == ARTIFACT_HEADER || line == ARTIFACT_RULE {
            continue; // prose, the header, or its underline — never a data row
        }
        rows.push(parse_row(line).map_err(corrupt)?);
    }

    for (position, row) in rows.iter().enumerate() {
        if rows[..position].iter().any(|held| held.key() == row.key()) {
            return Err(corrupt(format!(
                "id `{id}` is armed twice at arm root `{scope}` — (id, arm root) is the row key, \
                 so the page does not say which mode governs",
                id = row.id,
                scope = row.scope,
            )));
        }
    }

    Ok(ArmedArtifact { rows })
}

/// Parse one table row into its five § 4 columns.
fn parse_row(line: &str) -> Result<ArmedRow, String> {
    let inner = line
        .strip_prefix('|')
        .and_then(|l| l.strip_suffix('|'))
        .ok_or_else(|| format!("row is not a closed table row: {line:?}"))?;
    let cells: Vec<&str> = inner.split('|').map(str::trim).collect();
    let [id, page, rev, scope, mode] = cells.as_slice() else {
        return Err(format!(
            "row has {n} columns, not the 5 the artifact attests (id, page, rev, scope, mode): \
             {line:?}",
            n = cells.len()
        ));
    };

    let unquote = |cell: &str| cell.trim_matches('`').to_string();

    let id = RuleId::parse(&unquote(id))
        .map_err(|fault| format!("row carries an illegal id: {fault} — {line:?}"))?;
    let page = unquote(page);
    validate_workspace_path(&page)
        .map_err(|fault| format!("row carries an unusable page path: {fault} — {line:?}"))?;
    let rev = unquote(rev);
    if rev.len() != REV_LEN
        || !rev
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    {
        return Err(format!(
            "row's rev `{rev}` is not a page rev ({REV_LEN} lowercase hex) — {line:?}"
        ));
    }
    let scope = ArmRoot::parse(&unquote(scope))
        .map_err(|fault| format!("row carries an unusable arm root: {fault} — {line:?}"))?;
    let mode = Mode::parse(&unquote(mode)).ok_or_else(|| {
        format!(
            "row's mode `{mode}` is outside the closed vocabulary (a check arms \
             `{check}`, a hook arms `{hook}`) — {line:?}",
            mode = unquote(mode),
            check = Mode::vocabulary(RuleKind::Check),
            hook = Mode::vocabulary(RuleKind::Hook),
        )
    })?;

    Ok(ArmedRow {
        id,
        page,
        rev,
        scope,
        mode,
    })
}

/// The page rev's rendered width — `blake3(bytes)[:16]`.
const REV_LEN: usize = 16;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registration::PageRef;

    /// A rule page carrying one registration tag and an id.
    fn rule_page(tag: &str, id: &str) -> String {
        format!("---\ntags: [type/rule, {tag}]\nid: {id}\n---\n\n# rule\n")
    }

    fn hook_page(id: &str) -> String {
        rule_page("rules/hook", id)
    }

    fn check_page(id: &str) -> String {
        rule_page("rules/check", id)
    }

    /// An in-memory workspace: the pages a walk would offer, and the bytes a
    /// verification would re-read. One fixture backs both so a test can EDIT a page
    /// and see the armed row react.
    #[derive(Default, Clone)]
    struct Workspace {
        pages: Vec<(ScopeLayer, String, String)>,
    }

    impl Workspace {
        fn page(mut self, layer: ScopeLayer, path: &str, body: &str) -> Self {
            self.pages.push((layer, path.to_string(), body.to_string()));
            self
        }

        fn hook(self, path: &str, id: &str) -> Self {
            let body = hook_page(id);
            self.page(ScopeLayer::Workspace, path, &body)
        }

        fn check(self, path: &str, id: &str) -> Self {
            let body = check_page(id);
            self.page(ScopeLayer::Workspace, path, &body)
        }

        fn index(&self) -> RuleIndex {
            RuleIndex::discover(self.pages.iter().map(|(layer, page, bytes)| PageRef {
                layer: *layer,
                page,
                bytes,
            }))
        }

        /// The live rev of one page — what a reviewer reads and attests.
        fn rev(&self, path: &str) -> String {
            page_rev(self.bytes(path))
        }

        fn bytes(&self, path: &str) -> &str {
            self.pages
                .iter()
                .find(|(_, page, _)| page == path)
                .map(|(_, _, bytes)| bytes.as_str())
                .expect("the fixture has that page")
        }

        /// Edit a page in place — the act that reddens an armed row.
        fn edit(&mut self, path: &str, body: &str) {
            let slot = self
                .pages
                .iter_mut()
                .find(|(_, page, _)| page == path)
                .expect("the fixture has that page");
            slot.2 = body.to_string();
        }

        fn remove(&mut self, path: &str) {
            self.pages.retain(|(_, page, _)| page != path);
        }
    }

    impl PageSource for Workspace {
        fn read(&self, page: &str) -> std::io::Result<String> {
            self.pages
                .iter()
                .find(|(_, path, _)| path == page)
                .map(|(_, _, bytes)| bytes.clone())
                .ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::NotFound, format!("no {page}"))
                })
        }
    }

    fn id(text: &str) -> RuleId {
        RuleId::parse(text).expect("a legal test id")
    }

    fn request(text: &str, mode: Mode, rev: &str) -> ArmRequest {
        ArmRequest {
            id: id(text),
            mode,
            attested_rev: rev.to_string(),
        }
    }

    /// Arm one id at the workspace root, at its live rev — the common path.
    fn arm_one(ws: &Workspace, text: &str, mode: Mode) -> Result<ArmedArtifact, Vec<ArmFault>> {
        let page = ws
            .pages
            .iter()
            .find(|(_, _, bytes)| bytes.contains(&format!("id: {text}\n")))
            .expect("the fixture has a page for that id");
        let rev = page_rev(&page.2);
        arm(
            &ws.index(),
            &ArmRoot::workspace(),
            [request(text, mode, &rev)],
        )
    }

    // ── the kind↔mode binding (F3) ────────────────────────────────────────────

    /// A hand-edited hook row (`armed` → `block`) parses clean — no page edit,
    /// so no rev change catches it. The mode is verified against the page's
    /// registration tag, so the row reddens and fails closed instead of buying
    /// itself a veto.
    #[test]
    fn a_hand_edited_hook_row_reaching_for_a_veto_reddens() {
        let ws = Workspace::default().hook("notify.md", "task.review-notify");
        let artifact = arm_one(&ws, "task.review-notify", Mode::Armed).expect("arms");

        // The hand edit: one word in the rendered artifact, `armed` → `block`.
        let tampered = artifact.render().replace("| `armed` |", "| `block` |");
        let parsed =
            parse_artifact(&tampered).expect("a tampered row still PARSES — that is the gap");
        assert_eq!(parsed.rows()[0].mode(), Mode::Block, "the edit took");

        let verdict = parsed.verify(&ws);
        assert!(
            verdict.firing().is_empty(),
            "a hook armed `block` must not fire: {verdict:?}"
        );
        let red = &verdict.red()[0];
        let Redness::ModeOutsideKind { kinds, .. } = red.why() else {
            panic!("expected a kind/mode refusal, got {:?}", red.why());
        };
        assert_eq!(kinds, &[RuleKind::Hook]);
        let rendered = red.to_string();
        assert!(
            rendered.contains("rules/hook") && rendered.contains("block"),
            "the refusal names the kind and the mode it reached for: {rendered}"
        );
        assert!(
            verdict.refusing().next().is_some(),
            "and it fails CLOSED rather than falling silent"
        );
    }

    /// The mirror case: a CHECK row hand-edited to a hook's vocabulary reddens
    /// too — the defect is "mode outside its kind", not "hooks are special".
    #[test]
    fn a_hand_edited_check_row_in_hook_vocabulary_reddens() {
        let ws = Workspace::default().check("law.md", "reviewer-not-owner");
        let artifact = arm_one(&ws, "reviewer-not-owner", Mode::Block).expect("arms");
        let tampered = artifact.render().replace("| `block` |", "| `armed` |");
        let verdict = parse_artifact(&tampered).expect("parses").verify(&ws);
        assert!(verdict.firing().is_empty(), "{verdict:?}");
        assert!(matches!(
            verdict.red()[0].why(),
            Redness::ModeOutsideKind { .. }
        ));
    }

    /// An honest row is untouched by the check: the page's tag admits its mode, so
    /// re-deriving the kind changes nothing about what fires.
    #[test]
    fn a_row_whose_mode_matches_its_page_kind_still_fires() {
        let ws = Workspace::default()
            .hook("notify.md", "task.review-notify")
            .check("law.md", "reviewer-not-owner");
        let mut artifact = arm_one(&ws, "task.review-notify", Mode::Armed).expect("arms");
        artifact
            .merge(arm_one(&ws, "reviewer-not-owner", Mode::Block).expect("arms"))
            .expect("two ids, one root");

        let verdict = artifact.verify(&ws);
        assert_eq!(verdict.firing().len(), 2, "both stand: {verdict:?}");
        assert!(verdict.red().is_empty());
    }

    // ── the § 4 shape ─────────────────────────────────────────────────────────

    #[test]
    fn the_artifact_carries_exactly_the_section_4_columns() {
        let ws = Workspace::default().hook("notify.md", "task.review-notify");
        let artifact = arm_one(&ws, "task.review-notify", Mode::Armed).expect("arms");
        let page = artifact.render();

        assert!(page.starts_with(ARTIFACT_TITLE), "titled: {page}");
        assert!(
            page.contains("| id | page | rev | scope | mode |"),
            "the header IS the § 4 column list: {page}"
        );

        let row = page
            .lines()
            .find(|l| l.contains("task.review-notify"))
            .expect("the row renders");
        let cells: Vec<&str> = row
            .strip_prefix('|')
            .and_then(|l| l.strip_suffix('|'))
            .expect("a closed table row")
            .split('|')
            .map(str::trim)
            .collect();
        assert_eq!(cells.len(), 5, "exactly five columns, no sixth: {row}");
        assert_eq!(
            cells,
            vec![
                "`task.review-notify`",
                "`notify.md`",
                &format!("`{}`", ws.rev("notify.md")),
                "`.`",
                "`armed`",
            ],
            "id · page · rev · scope · mode"
        );
        assert!(page.ends_with('\n'), "trailing newline");
    }

    #[test]
    fn one_row_per_armed_id_at_one_arm_root() {
        let ws = Workspace::default()
            .hook("a.md", "one")
            .check("b.md", "two");
        let index = ws.index();
        let artifact = arm(
            &index,
            &ArmRoot::workspace(),
            [
                request("one", Mode::Armed, &ws.rev("a.md")),
                request("two", Mode::Block, &ws.rev("b.md")),
            ],
        )
        .expect("both arm");
        assert_eq!(artifact.rows().len(), 2);
        let rows = artifact.render();
        assert_eq!(
            rows.lines().filter(|l| l.starts_with("| `")).count(),
            2,
            "two data rows"
        );
    }

    // ── one fingerprint law ───────────────────────────────────────────────────

    #[test]
    fn check_pages_and_hook_pages_are_attested_by_one_fingerprint_law() {
        // Byte-identical bodies but for the tag: one page-rev law pins both kinds.
        let ws = Workspace::default().hook("h.md", "h").check("c.md", "c");
        let hook = arm_one(&ws, "h", Mode::Armed).expect("hook arms");
        let check = arm_one(&ws, "c", Mode::Block).expect("check arms");

        assert_eq!(hook.rows()[0].rev(), page_rev(ws.bytes("h.md")));
        assert_eq!(check.rows()[0].rev(), page_rev(ws.bytes("c.md")));
        assert_eq!(
            hook.rows()[0].rev().len(),
            REV_LEN,
            "the same 16-hex page rev for both kinds"
        );
        assert_eq!(
            check.rows()[0].rev(),
            page_rev(ws.bytes("c.md")),
            "the check page is pinned by the SAME function as the hook page — there is no \
             `blake3(CHECK.md)` alias left to disagree with it"
        );
    }

    // ── the mode vocabulary splits by kind ────────────────────────────────────

    #[test]
    fn a_hook_arms_only_off_or_armed() {
        let ws = Workspace::default().hook("h.md", "h");
        for legal in [Mode::Off, Mode::Armed] {
            assert!(arm_one(&ws, "h", legal).is_ok(), "a hook arms {legal}");
        }
        for illegal in [Mode::Warn, Mode::Block] {
            let faults = arm_one(&ws, "h", illegal).expect_err("check vocabulary on a hook");
            assert!(matches!(faults[0], ArmFault::ModeKind { .. }), "{faults:?}");
        }
    }

    #[test]
    fn a_check_arms_only_off_warn_or_block() {
        let ws = Workspace::default().check("c.md", "c");
        for legal in [Mode::Off, Mode::Warn, Mode::Block] {
            assert!(arm_one(&ws, "c", legal).is_ok(), "a check arms {legal}");
        }
        let faults = arm_one(&ws, "c", Mode::Armed).expect_err("hook vocabulary on a check");
        assert!(matches!(faults[0], ArmFault::ModeKind { .. }), "{faults:?}");
    }

    #[test]
    fn a_cross_kind_mode_is_refused_loudly_naming_the_vocabulary() {
        let ws = Workspace::default().hook("h.md", "h");
        let faults = arm_one(&ws, "h", Mode::Block).expect_err("a hook cannot block");
        assert_eq!(faults.len(), 1);
        let rendered = faults[0].to_string();
        assert!(rendered.contains('h'), "names the id: {rendered}");
        assert!(rendered.contains("h.md"), "names the page: {rendered}");
        assert!(
            rendered.contains("off|armed"),
            "teaches the legal vocabulary: {rendered}"
        );
        assert!(
            rendered.contains("never veto"),
            "teaches WHY a hook has no severity axis: {rendered}"
        );
    }

    #[test]
    fn a_hook_row_carrying_a_check_mode_is_unrepresentable_not_tolerated() {
        // Enforced at the ACT: no rendered artifact carries a hook row at `block`.
        assert!(!Mode::Block.admits(RuleKind::Hook));
        assert!(!Mode::Warn.admits(RuleKind::Hook));
        assert!(!Mode::Armed.admits(RuleKind::Check));
        assert!(Mode::Off.admits(RuleKind::Hook) && Mode::Off.admits(RuleKind::Check));
    }

    // ── drift reddens and fails closed ────────────────────────────────────────

    #[test]
    fn editing_a_pinned_page_after_arming_reddens_its_row_and_it_does_not_fire() {
        let mut ws = Workspace::default().check("c.md", "c");
        let artifact = arm_one(&ws, "c", Mode::Block).expect("arms");
        assert_eq!(artifact.verify(&ws).firing().len(), 1, "fresh, it fires");

        let pinned = artifact.rows()[0].rev().to_string();
        ws.edit("c.md", &format!("{}\n<!-- edited -->\n", check_page("c")));

        let verdict = artifact.verify(&ws);
        assert!(
            verdict.firing().is_empty(),
            "the edited page does NOT fire on its new bytes"
        );
        assert_eq!(verdict.red().len(), 1);
        let red = &verdict.red()[0];
        let Redness::Drifted { report_rev } = red.why() else {
            panic!("expected drift, got {:?}", red.why());
        };
        assert_ne!(report_rev, &pinned, "the rev moved");
        assert_eq!(*report_rev, ws.rev("c.md"));
        let rendered = red.to_string();
        assert!(
            rendered.contains(&pinned) && rendered.contains(report_rev),
            "the refusal names both revs: {rendered}"
        );
    }

    #[test]
    fn a_red_check_row_refuses_the_write_but_a_red_hook_row_falls_silent() {
        let mut ws = Workspace::default().check("c.md", "c").hook("h.md", "h");
        let index = ws.index();
        let artifact = arm(
            &index,
            &ArmRoot::workspace(),
            [
                request("c", Mode::Block, &ws.rev("c.md")),
                request("h", Mode::Armed, &ws.rev("h.md")),
            ],
        )
        .expect("both arm");

        // BOTH pinned pages drift.
        ws.edit("c.md", &format!("{}\n<!-- x -->\n", check_page("c")));
        ws.edit("h.md", &format!("{}\n<!-- x -->\n", hook_page("h")));

        let verdict = artifact.verify(&ws);
        assert_eq!(verdict.red().len(), 2, "both reddened");
        assert!(verdict.firing().is_empty(), "neither fires");

        let refusing: Vec<&str> = verdict
            .refusing()
            .map(|red| red.row().id().as_str())
            .collect();
        assert_eq!(
            refusing,
            vec!["c"],
            "a drifted CHECK refuses the write; a drifted HOOK stays silent, because \
             refusing on a reaction's behalf would hand a hook the veto the ruling denies it"
        );
    }

    #[test]
    fn a_pinned_page_that_vanishes_reddens_rather_than_silently_passing() {
        let mut ws = Workspace::default().check("c.md", "c");
        let artifact = arm_one(&ws, "c", Mode::Block).expect("arms");
        ws.remove("c.md");

        let verdict = artifact.verify(&ws);
        assert!(verdict.firing().is_empty());
        assert!(matches!(verdict.red()[0].why(), Redness::Missing { .. }));
        assert_eq!(
            verdict.refusing().count(),
            1,
            "a vanished LAW fails closed, never open"
        );
    }

    #[test]
    fn an_attested_off_row_is_inert_but_still_pins_what_was_read() {
        let ws = Workspace::default().check("c.md", "c");
        let artifact = arm_one(&ws, "c", Mode::Off).expect("attested-off arms");
        assert_eq!(artifact.rows()[0].rev(), ws.rev("c.md"), "it pins");
        assert!(
            artifact.verify(&ws).firing().is_empty(),
            "and it never fires"
        );
    }

    // ── the composed law: select at P, then verify exactly those rows ─────────

    /// One id armed at two roots on one chain; the inner page drifts. The red
    /// inner row must still shadow the outer row and fail closed at its path —
    /// verify-then-select would instead hand the inner path to the stale outer
    /// row, a cap escape by page edit.
    #[test]
    fn a_drifted_inner_row_fails_closed_instead_of_handing_its_path_to_the_outer_row() {
        let mut ws = Workspace::default()
            .hook("notify.md", "shared")
            .hook("sessions/s1/notify.md", "shared");
        let index = ws.index();
        let mut artifact = arm(
            &index,
            &ArmRoot::workspace(),
            [request("shared", Mode::Armed, &ws.rev("notify.md"))],
        )
        .expect("the outer arm");
        artifact
            .merge(
                arm(
                    &index,
                    &ArmRoot::parse("sessions/s1").unwrap(),
                    [request(
                        "shared",
                        Mode::Armed,
                        &ws.rev("sessions/s1/notify.md"),
                    )],
                )
                .expect("the inner arm"),
            )
            .expect("two roots, one id — a legal artifact");

        // Only the INNER page drifts.
        ws.edit(
            "sessions/s1/notify.md",
            &format!("{}\n<!-- edited -->\n", hook_page("shared")),
        );

        // The bait: whole-artifact verification says the outer row is firing.
        let whole = artifact.verify(&ws);
        assert_eq!(
            whole
                .firing()
                .iter()
                .map(ArmedRow::page)
                .collect::<Vec<_>>(),
            vec!["notify.md"],
            "the outer row is green — selecting over THIS set is the failure"
        );

        // The law: at the inner path, the red inner row shadows and fails closed.
        let at = artifact.verify_at("sessions/s1/task.md", &ws);
        assert!(
            at.firing().is_empty(),
            "the outer row must NOT govern the inner path just because the inner row \
             reddened — that would be a cap escape by page edit"
        );
        assert_eq!(at.red().len(), 1);
        assert_eq!(at.red()[0].row().page(), "sessions/s1/notify.md");

        // And the outer row still governs its OWN paths, undisturbed.
        let outside = artifact.verify_at("task.md", &ws);
        assert_eq!(outside.firing().len(), 1);
        assert_eq!(outside.firing()[0].page(), "notify.md");
    }

    /// A drifted CHECK armed at `sessions/a` must not refuse a write under the
    /// sibling `sessions/b` — § 3 rules sibling subtrees independent.
    #[test]
    fn a_sibling_scopes_drift_does_not_refuse_this_write() {
        let mut ws = Workspace::default().check("sessions/a/law.md", "law");
        let index = ws.index();
        let artifact = arm(
            &index,
            &ArmRoot::parse("sessions/a").unwrap(),
            [request("law", Mode::Block, &ws.rev("sessions/a/law.md"))],
        )
        .expect("arms");

        ws.edit(
            "sessions/a/law.md",
            &format!("{}\n<!-- edited -->\n", check_page("law")),
        );

        // The bait: whole-artifact verification refuses.
        assert_eq!(
            artifact.verify(&ws).refusing().count(),
            1,
            "un-narrowed, the drift refuses — this is the set that must NOT gate a write"
        );

        // The law: it refuses inside its own scope, and nowhere else.
        assert_eq!(
            artifact
                .verify_at("sessions/a/task.md", &ws)
                .refusing()
                .count(),
            1,
            "inside the arm root, a drifted CHECK still refuses"
        );
        assert_eq!(
            artifact
                .verify_at("sessions/b/task.md", &ws)
                .refusing()
                .count(),
            0,
            "a sibling subtree is not coupled to this drift"
        );
    }

    // ── arming freezes resolution (THE CAP-ESCAPE GUARDRAIL) ──────────────────

    /// A page that appears after arming — a deeper override candidate live
    /// resolution would now hand the id to — does not enter the armed set until
    /// a re-arm. Otherwise any writer with put access could drop a deeper page
    /// and silently take over an armed id without any reviewer act.
    #[test]
    fn a_deeper_override_candidate_appearing_after_arming_is_the_cap_escape_guardrail() {
        let mut ws = Workspace::default().hook("rules.md", "shared");
        let artifact = arm_one(&ws, "shared", Mode::Armed).expect("arms the shallow page");
        assert_eq!(artifact.rows()[0].page(), "rules.md");

        // An attacker (or an innocent author) drops a DEEPER page with the same id.
        ws = ws.hook("sessions/s1/rules.md", "shared");

        // Live resolution now hands the id to the deeper page…
        let live = ws.index().narrowed_to("sessions/s1/x.md").resolve();
        assert_eq!(
            live.get("shared").unwrap().winner().page(),
            "sessions/s1/rules.md",
            "discovery DOES see the deeper page"
        );

        // …and the armed artifact does not care. It still pins the old page.
        assert_eq!(
            artifact.rows()[0].page(),
            "rules.md",
            "arming froze resolution — the new page governs nothing"
        );
        let verdict = artifact.verify(&ws);
        assert_eq!(verdict.firing().len(), 1);
        assert_eq!(
            verdict.firing()[0].page(),
            "rules.md",
            "the frozen winner is what fires, not the newly-appeared override"
        );
        assert!(
            !artifact.render().contains("sessions/s1/rules.md"),
            "the deeper page is nowhere in the attestation"
        );

        // Only a RE-ARM moves the id. That act is explicit and attested.
        let rearmed = arm(
            &ws.index(),
            &ArmRoot::parse("sessions/s1").unwrap(),
            [request(
                "shared",
                Mode::Armed,
                &ws.rev("sessions/s1/rules.md"),
            )],
        )
        .expect("a re-arm at the inner root pins the deeper page");
        assert_eq!(rearmed.rows()[0].page(), "sessions/s1/rules.md");
    }

    #[test]
    fn nothing_arms_by_tag_alone_a_discovered_page_never_fires() {
        let ws = Workspace::default()
            .hook("armed.md", "yes")
            .hook("discovered.md", "no");
        // Both pages are DISCOVERED…
        let index = ws.index();
        assert_eq!(index.registered().len(), 2);
        assert!(index.resolve().get("no").is_some(), "and both resolve");

        // …but only one is armed, and only that one is in the artifact or fires.
        let artifact = arm_one(&ws, "yes", Mode::Armed).expect("arms");
        assert_eq!(artifact.rows().len(), 1);
        assert_eq!(artifact.select_at("anywhere.md").len(), 1);
        let verdict = artifact.verify(&ws);
        let firing: Vec<&str> = verdict.firing().iter().map(|r| r.id().as_str()).collect();
        assert_eq!(
            firing,
            vec!["yes"],
            "the tag registered; only ARM activated"
        );
    }

    // ── narrowing + the selection law ─────────────────────────────────────────

    #[test]
    fn sibling_scopes_may_both_arm_the_same_id_and_never_interact() {
        let ws = Workspace::default()
            .hook("sessions/a/rules.md", "shared")
            .hook("sessions/b/rules.md", "shared");

        let mut artifact = arm(
            &ws.index(),
            &ArmRoot::parse("sessions/a").unwrap(),
            [request(
                "shared",
                Mode::Armed,
                &ws.rev("sessions/a/rules.md"),
            )],
        )
        .expect("the first sibling arms");
        let second = arm(
            &ws.index(),
            &ArmRoot::parse("sessions/b").unwrap(),
            [request(
                "shared",
                Mode::Armed,
                &ws.rev("sessions/b/rules.md"),
            )],
        )
        .expect("and so does the second — siblings are NOT a collision");
        artifact.merge(second).expect("both live in one artifact");

        assert_eq!(artifact.rows().len(), 2, "same id, two rows, two arm roots");
        let at_a = artifact.select_at("sessions/a/task.md");
        assert_eq!(at_a.len(), 1);
        assert_eq!(at_a[0].page(), "sessions/a/rules.md");
        let at_b = artifact.select_at("sessions/b/task.md");
        assert_eq!(at_b.len(), 1);
        assert_eq!(
            at_b[0].page(),
            "sessions/b/rules.md",
            "neither sibling reaches the other"
        );
        assert!(
            artifact.select_at("elsewhere/task.md").is_empty(),
            "and neither reaches outside its own subtree"
        );
    }

    #[test]
    fn an_inner_arm_shadows_an_outer_arm_on_one_chain() {
        let ws = Workspace::default()
            .hook("rules.md", "shared")
            .hook("sessions/s1/rules.md", "shared");

        let mut artifact = arm(
            &ws.index(),
            &ArmRoot::workspace(),
            [request("shared", Mode::Armed, &ws.rev("rules.md"))],
        )
        .expect("the workspace root arms the shallow page");
        let inner = arm(
            &ws.index(),
            &ArmRoot::parse("sessions/s1").unwrap(),
            [request(
                "shared",
                Mode::Armed,
                &ws.rev("sessions/s1/rules.md"),
            )],
        )
        .expect("the inner root arms the deeper page");
        artifact.merge(inner).expect("one artifact, two arm roots");

        let inside = artifact.select_at("sessions/s1/task.md");
        assert_eq!(inside.len(), 1, "per id, ONE row governs");
        assert_eq!(
            inside[0].page(),
            "sessions/s1/rules.md",
            "the deepest arm root containing the path wins"
        );

        let outside = artifact.select_at("other/task.md");
        assert_eq!(outside.len(), 1);
        assert_eq!(
            outside[0].page(),
            "rules.md",
            "outside the inner root, the outer arm still governs"
        );
    }

    #[test]
    fn narrowing_excludes_pages_below_the_arm_root() {
        let ws = Workspace::default().hook("sessions/s1/rules.md", "deep");
        // The id is registered in the workspace, but not on the ROOT's chain…
        assert!(ws.index().resolve().get("deep").is_some());
        let faults = arm(
            &ws.index(),
            &ArmRoot::workspace(),
            [request(
                "deep",
                Mode::Armed,
                &ws.rev("sessions/s1/rules.md"),
            )],
        )
        .expect_err("a page BELOW the arm root is not a candidate at it");
        assert!(
            matches!(faults[0], ArmFault::Unresolved { .. }),
            "{faults:?}"
        );
    }

    #[test]
    fn an_outer_page_is_a_candidate_at_an_inner_root() {
        let ws = Workspace::default().hook("rules.md", "shared");
        let artifact = arm(
            &ws.index(),
            &ArmRoot::parse("sessions/s1").unwrap(),
            [request("shared", Mode::Armed, &ws.rev("rules.md"))],
        )
        .expect("the chain reaches UP to the workspace root");
        assert_eq!(artifact.rows()[0].page(), "rules.md");
        assert_eq!(artifact.rows()[0].scope().as_str(), "sessions/s1");
    }

    #[test]
    fn a_root_never_reads_a_prefix_sibling_as_its_own_subtree() {
        let root = ArmRoot::parse("a/b").unwrap();
        assert!(root.contains("a/b"), "itself");
        assert!(root.contains("a/b/c.md"), "its child");
        assert!(
            !root.contains("a/bc.md"),
            "`a/bc.md` does NOT live under `a/b` — the separator is explicit"
        );
        assert!(ArmRoot::workspace().contains("anything/at/all.md"));
    }

    #[test]
    fn two_rows_sharing_id_and_arm_root_are_refused() {
        let ws = Workspace::default().hook("rules.md", "shared");
        let first = arm_one(&ws, "shared", Mode::Armed).expect("arms");
        let again = arm_one(&ws, "shared", Mode::Off).expect("the act itself is legal");
        let mut artifact = first;
        let faults = artifact
            .merge(again)
            .expect_err("(id, arm root) is the row key — a repeat is a defect");
        assert!(
            matches!(faults[0], ArmFault::Duplicate { .. }),
            "{faults:?}"
        );
    }

    // ── the ARM act's refusals ────────────────────────────────────────────────

    #[test]
    fn arming_an_id_that_resolves_to_nothing_is_refused() {
        let ws = Workspace::default().hook("rules.md", "known");
        let faults = arm(
            &ws.index(),
            &ArmRoot::workspace(),
            [request("unknown", Mode::Armed, "0000000000000000")],
        )
        .expect_err("arming cannot invent an effective set");
        assert!(
            matches!(faults[0], ArmFault::Unresolved { .. }),
            "{faults:?}"
        );
        assert!(faults[0].to_string().contains("unknown"));
    }

    #[test]
    fn a_collided_id_cannot_be_armed() {
        // Two pages, same id, same scope, ONE chain — the § 3 collision.
        let ws = Workspace::default()
            .hook("a.md", "shared")
            .hook("b.md", "shared");
        let set = ws.index().resolve();
        assert_eq!(set.collisions().len(), 1, "the resolver refuses the id");
        let faults = arm(
            &ws.index(),
            &ArmRoot::workspace(),
            [request("shared", Mode::Armed, "0000000000000000")],
        )
        .expect_err("an id that resolves to nothing arms nothing");
        assert!(
            matches!(faults[0], ArmFault::Unresolved { .. }),
            "{faults:?}"
        );
    }

    /// A page carrying BOTH registration tags is refused at ARM: the mode
    /// column is typed by kind, so a dual-kind id has no single vocabulary.
    /// Discovery admits such a page; arming answers the ambiguity fail-closed.
    #[test]
    fn a_dual_kind_page_is_refused_at_arm_and_told_how_to_split() {
        let body = "---\ntags: [rules/check, rules/hook]\nid: dual\n---\n";
        let ws = Workspace::default().page(ScopeLayer::Workspace, "dual.md", body);
        assert_eq!(
            ws.index().registered()[0].kinds(),
            &[RuleKind::Check, RuleKind::Hook],
            "discovery admits it"
        );

        for mode in [Mode::Off, Mode::Warn, Mode::Block, Mode::Armed] {
            let faults = arm(
                &ws.index(),
                &ArmRoot::workspace(),
                [request("dual", mode, &ws.rev("dual.md"))],
            )
            .expect_err("but arming it does not");
            assert!(matches!(faults[0], ArmFault::DualKind { .. }), "{faults:?}");
            let rendered = faults[0].to_string();
            assert!(
                rendered.contains("dual.md") && rendered.contains("Split it into two pages"),
                "the refusal names the page and the remedy: {rendered}"
            );
        }
    }

    #[test]
    fn drift_between_approval_and_arming_is_refused() {
        let ws = Workspace::default().check("c.md", "c");
        let faults = arm(
            &ws.index(),
            &ArmRoot::workspace(),
            [request("c", Mode::Block, "deadbeefdeadbeef")],
        )
        .expect_err("a stale approval never silently re-pins");
        let ArmFault::Drift {
            attested_rev,
            resolved_rev,
            ..
        } = &faults[0]
        else {
            panic!("expected Drift, got {faults:?}");
        };
        assert_eq!(attested_rev, "deadbeefdeadbeef");
        assert_eq!(*resolved_rev, ws.rev("c.md"));
    }

    #[test]
    fn a_duplicate_request_in_one_act_is_refused() {
        let ws = Workspace::default().hook("rules.md", "h");
        let rev = ws.rev("rules.md");
        let faults = arm(
            &ws.index(),
            &ArmRoot::workspace(),
            [
                request("h", Mode::Armed, &rev),
                request("h", Mode::Off, &rev),
            ],
        )
        .expect_err("one act attests one mode per id");
        assert!(
            matches!(faults[0], ArmFault::Duplicate { .. }),
            "{faults:?}"
        );
    }

    #[test]
    fn the_act_is_all_or_nothing_and_names_every_fault() {
        let ws = Workspace::default().hook("h.md", "h").check("c.md", "c");
        let faults = arm(
            &ws.index(),
            &ArmRoot::workspace(),
            [
                request("h", Mode::Block, &ws.rev("h.md")), // cross-kind
                request("c", Mode::Block, "deadbeefdeadbeef"), // drifted
                request("ghost", Mode::Off, "0000000000000000"), // unresolved
            ],
        )
        .expect_err("nothing lands");
        assert_eq!(faults.len(), 3, "every fault in ONE round-trip: {faults:?}");
        let ids: Vec<&str> = faults.iter().map(|f| f.id().as_str()).collect();
        assert_eq!(ids, vec!["h", "c", "ghost"], "in request order");
    }

    #[test]
    fn a_good_request_beside_a_bad_one_does_not_partially_land() {
        let ws = Workspace::default().hook("h.md", "h").check("c.md", "c");
        let outcome = arm(
            &ws.index(),
            &ArmRoot::workspace(),
            [
                request("h", Mode::Armed, &ws.rev("h.md")), // fine
                request("c", Mode::Armed, &ws.rev("c.md")), // cross-kind
            ],
        );
        assert!(
            outcome.is_err(),
            "a partial artifact would silently drop a rule the reviewer meant to arm"
        );
    }

    /// § 4 spells the `page` column as a workspace path, so a user-space winner
    /// has no unambiguous spelling here — refused by name, not silently dropped.
    #[test]
    fn a_user_space_winner_is_a_named_deferral_not_a_silent_drop() {
        let body = hook_page("u");
        let ws = Workspace::default().page(ScopeLayer::User, "rules.md", &body);
        let faults = arm(
            &ws.index(),
            &ArmRoot::workspace(),
            [request("u", Mode::Armed, &page_rev(&body))],
        )
        .expect_err("a user-space page does not arm into a workspace artifact");
        assert!(
            matches!(faults[0], ArmFault::UserLayerDeferred { .. }),
            "{faults:?}"
        );
    }

    #[test]
    fn a_workspace_page_shadows_a_user_page_and_arms_normally() {
        let user = hook_page("shared");
        let ws = Workspace::default()
            .page(ScopeLayer::User, "rules.md", &user)
            .hook("rules.md", "shared");
        let artifact = arm(
            &ws.index(),
            &ArmRoot::workspace(),
            [request("shared", Mode::Armed, &ws.rev("rules.md"))],
        )
        .expect("the workspace page wins and arms");
        assert_eq!(artifact.rows()[0].page(), "rules.md");
    }

    // ── the row grammar cannot be forged ──────────────────────────────────────

    #[test]
    fn a_page_path_that_would_forge_a_row_is_refused_at_intake() {
        for (hostile, forges) in [
            ("a|x|y|z|block.md", "extra columns"),
            ("a`.md", "the cell's own quoting"),
            ("a\nb.md", "a whole forged row"),
        ] {
            let body = hook_page("h");
            let ws = Workspace::default().page(ScopeLayer::Workspace, hostile, &body);
            let Err(faults) = arm(
                &ws.index(),
                &ArmRoot::workspace(),
                [request("h", Mode::Armed, &page_rev(&body))],
            ) else {
                panic!("a page path forging {forges} must not arm: {hostile:?}");
            };
            assert!(
                matches!(faults[0], ArmFault::Unrenderable { .. }),
                "{hostile:?} forges {forges}, so it is refused at INTAKE — the hostile bytes \
                 are unrepresentable, not escaped: {faults:?}"
            );
        }
    }

    #[test]
    fn an_arm_root_is_one_directory_with_one_spelling() {
        assert_eq!(ArmRoot::parse(".").unwrap(), ArmRoot::workspace());
        assert_eq!(ArmRoot::parse("").unwrap(), ArmRoot::workspace());
        assert_eq!(ArmRoot::parse("a/b").unwrap().depth(), 2);
        assert_eq!(ArmRoot::workspace().depth(), 0);

        assert_eq!(ArmRoot::parse("/abs"), Err(PathFault::Absolute));
        assert_eq!(ArmRoot::parse("a/../b"), Err(PathFault::Escapes));
        assert_eq!(ArmRoot::parse("a/./b"), Err(PathFault::DotSegment));
        assert_eq!(ArmRoot::parse("a/"), Err(PathFault::EmptySegment));
        assert_eq!(ArmRoot::parse("a//b"), Err(PathFault::EmptySegment));
        assert!(matches!(
            ArmRoot::parse("a|b"),
            Err(PathFault::Unrenderable { found: '|' })
        ));
    }

    /// `workspace:0` is the RESOLVER's layer:depth scope spelling (the chain
    /// lines of `mrd rules`), not a directory. Under the address grammar a
    /// head segment carrying `:` is a `root:` qualifier (§ 4.2 D11) — an
    /// address, never a workspace path — so an arm root spelled that way is
    /// refused with a teaching that names both vocabularies. Measured before
    /// this guard: the pasted cell parsed clean and governed nothing.
    #[test]
    fn the_resolvers_layer_depth_spelling_is_refused_as_an_arm_root() {
        for copied in ["workspace:0", "workspace:2", "user:1"] {
            let fault = ArmRoot::parse(copied).expect_err("resolver vocabulary is not a directory");
            assert!(
                matches!(fault, PathFault::RootSeparator { .. }),
                "{copied}: {fault:?}"
            );
            let teaching = fault.to_string();
            assert!(
                teaching.contains("layer:depth"),
                "names the colliding vocabulary: {teaching}"
            );
            assert!(
                teaching.contains("ARM ROOT") && teaching.contains("workspace root"),
                "teaches what the column takes instead: {teaching}"
            );
        }
        // D11 exactly: a `:` after the first `/` is an ordinary path byte.
        assert!(
            ArmRoot::parse("sessions/a:b").is_ok(),
            "a colon past the head segment stays a legal directory"
        );
    }

    /// The copy-paste row itself: a § 4 scope cell hand-filled with the winner
    /// line's `workspace:0`. It used to parse clean while governing nothing —
    /// the config twin of the face-lies family. Now the artifact refuses as
    /// corrupt, and the refusal teaches the collision at the cell that carries
    /// it.
    #[test]
    fn a_scope_cell_pasted_from_the_winner_line_is_corrupt_not_inert() {
        let ws = Workspace::default().check("rules/c.md", "c");
        let page = arm_one(&ws, "c", Mode::Block).expect("arms").render();
        let pasted = page.replace("| `.` |", "| `workspace:0` |");
        assert_ne!(
            pasted, page,
            "the fixture's scope cell was the workspace root"
        );
        let err = parse_artifact(&pasted).expect_err("the pasted resolver spelling refuses");
        assert!(
            err.detail.contains("arm root") && err.detail.contains("layer:depth"),
            "the refusal names both vocabularies: {}",
            err.detail
        );
    }

    // ── reading an attested page back ─────────────────────────────────────────

    #[test]
    fn an_artifact_round_trips_through_render_and_parse() {
        let ws = Workspace::default()
            .hook("h.md", "h")
            .check("sessions/s1/c.md", "c");
        let mut artifact = arm(
            &ws.index(),
            &ArmRoot::workspace(),
            [request("h", Mode::Armed, &ws.rev("h.md"))],
        )
        .expect("arms");
        artifact
            .merge(
                arm(
                    &ws.index(),
                    &ArmRoot::parse("sessions/s1").unwrap(),
                    [request("c", Mode::Warn, &ws.rev("sessions/s1/c.md"))],
                )
                .expect("arms"),
            )
            .expect("merges");

        let page = artifact.render();
        let read_back = parse_artifact(&page).expect("a rendered artifact parses");
        assert_eq!(read_back, artifact, "byte-for-byte the same rows");
        assert_eq!(read_back.render(), page, "and renders identically");
    }

    #[test]
    fn a_corrupt_artifact_never_reads_as_nothing_armed() {
        let ws = Workspace::default().hook("h.md", "h");
        let page = arm_one(&ws, "h", Mode::Armed).expect("arms").render();

        // Title stripped — the classic gate-disabling edit.
        let headless = page.replace(ARTIFACT_TITLE, "# something else");
        assert!(parse_artifact(&headless).is_err(), "titleless is corrupt");

        // Header stripped — the columns can no longer be trusted.
        let unheaded = page.replace(ARTIFACT_HEADER, "| a | b | c | d | e |");
        assert!(parse_artifact(&unheaded).is_err(), "re-columned is corrupt");

        // A row truncated to four columns.
        let truncated = page.replace(" | `armed` |", " |");
        assert!(parse_artifact(&truncated).is_err(), "short row is corrupt");
    }

    #[test]
    fn a_tampered_mode_word_is_refused_rather_than_coerced() {
        let ws = Workspace::default().hook("h.md", "h");
        let page = arm_one(&ws, "h", Mode::Armed).expect("arms").render();
        for forged in ["`enabled`", "`BLOCK`", "`on`", "``"] {
            let tampered = page.replace("`armed`", forged);
            assert!(
                parse_artifact(&tampered).is_err(),
                "{forged} is outside the closed vocabulary"
            );
        }
    }

    #[test]
    fn a_tampered_rev_is_refused() {
        let ws = Workspace::default().hook("h.md", "h");
        let page = arm_one(&ws, "h", Mode::Armed).expect("arms").render();
        let pinned = ws.rev("h.md");
        for forged in ["short", "DEADBEEFDEADBEEF", "zzzzzzzzzzzzzzzz"] {
            let tampered = page.replace(&pinned, forged);
            assert!(
                parse_artifact(&tampered).is_err(),
                "{forged} is not a page rev"
            );
        }
    }

    #[test]
    fn a_page_that_arms_the_same_id_twice_at_one_root_is_corrupt() {
        let ws = Workspace::default().hook("h.md", "h");
        let page = arm_one(&ws, "h", Mode::Armed).expect("arms").render();
        let row = page
            .lines()
            .find(|l| l.starts_with("| `h`"))
            .expect("the row")
            .to_string();
        let doubled = format!("{page}{row}\n");
        let err = parse_artifact(&doubled).expect_err("(id, arm root) is the row key");
        assert!(err.detail.contains("armed twice"), "{}", err.detail);
    }

    #[test]
    fn prose_around_the_table_never_parses_as_a_row() {
        let ws = Workspace::default().hook("h.md", "h");
        let artifact = arm_one(&ws, "h", Mode::Armed).expect("arms");
        let page = artifact.render();
        assert!(
            page.contains("Arming freezes resolution"),
            "the preamble is prose"
        );
        assert_eq!(
            parse_artifact(&page).expect("parses").rows().len(),
            1,
            "one row, and the title/preamble contributed none"
        );
    }

    #[test]
    fn an_empty_artifact_is_legal_and_arms_nothing() {
        let artifact = ArmedArtifact::default();
        let page = artifact.render();
        assert_eq!(parse_artifact(&page).expect("parses"), artifact);
        assert!(artifact.select_at("anything.md").is_empty());
    }
}
