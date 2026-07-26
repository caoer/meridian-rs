//! The `MERIDIAN.md` config plane — the one entry point, parsed as content.
//!
//! # Charter
//! **Owns:** the bootstrap chain (`MERIDIAN_CONFIG` → `$HOME/MERIDIAN.md`), the
//! four resolution states, the in-file schema's strict parse (frontmatter +
//! the `meridian-mount` / `meridian-tool` block grammars), the closed refusal
//! set with its 1-based FILE lines, and the config's own rev and fingerprint —
//! minted by the shipped laws, never a new one (`docs/meridian-md-schema.md`
//! §7.1).
//!
//! **Never does:** bind a mount *in this module*. Canonicalization at bind, the
//! `workspace::deny_reason` ceiling, the equal-or-nested refusal, the
//! declared-vs-bound check and the grey classes live in [`mount`] —
//! `docs/address-grammar.md` §8. This module produces the parsed config; that
//! one gives it meaning. It also does not do project-local walk-up discovery: the
//! ratifying decision defers it, so the chain in [`resolve`] has exactly two
//! rungs and no third. The bridge period's env-var check is [`bridge`]'s.
//!
//! # Two env axes, one nil-vs-empty rule
//! [`Env`] carries the BOOTSTRAP variables (`MERIDIAN_CONFIG`, `HOME`) — where
//! the config file is. [`bridge::BridgeEnv`] carries the two llm-wiki variables
//! the entry-point ruling inverts into mount entries — what the file should
//! already say. Both treat an empty or whitespace-only value as unset, so the
//! two axes cannot come to disagree about what "stated" means.
//!
//! # The precedent this EXTENDS
//! `conventions/INDEX.md` is markdown-as-config shipping today
//! (`crates/policy/src/index.rs`): strictness scoped to a machine surface,
//! malformed fails closed, and the pinned rev is `blake3(bytes)[:16]`. Schema
//! §1 walks the parallel row by row and names the five places this schema
//! deliberately differs.
//!
//! # No partial mount table, by construction
//! [`parse`] returns `Result<Config, ConfigError>` and [`Config`]'s fields are
//! private with no public constructor — the only way to hold one is to have
//! parsed a whole file cleanly. A malformed config therefore leaves no
//! partially-populated state observable to any caller, because there is no
//! value to observe. That is the ratified no-partial-load made a property of
//! the type rather than of today's call sites.

use std::path::{Path, PathBuf};

use model::NodeKind;

pub mod bridge;
pub mod mount;

/// The reserved config filename (schema §2.4). Rung 2 of the chain.
pub const CONFIG_FILENAME: &str = "MERIDIAN.md";

/// The reserved override env var (schema §2.4). Rung 1 of the chain.
pub const CONFIG_ENV_VAR: &str = "MERIDIAN_CONFIG";

/// The required `type:` discriminator value (schema §4).
pub const CONFIG_TYPE: &str = "meridian-config";

/// The one schema version this build implements (schema §4). A future format
/// bumps it; this reader refuses a version it does not implement and never
/// guesses — `lock::LockError::UnsupportedVersion`'s own law.
pub const VERSION: u64 = 1;

/// The `meridian-mount` block language (schema §3.1).
pub const MOUNT_LANG: &str = "meridian-mount";

/// The `meridian-tool` block language (schema §3.1).
pub const TOOL_LANG: &str = "meridian-tool";

/// The mount block's fields, in canonical order (schema §5.1). The refusal
/// teaches this order verbatim, so it has one spelling.
pub const MOUNT_FIELDS: [&str; 5] = ["name", "path", "kind", "vault", "pin"];

/// The tool block's fields, in canonical order (schema §6).
pub const TOOL_FIELDS: [&str; 3] = ["name", "kind", "config"];

/// The canonical root-name charset, as the refusal spells it (schema §5.2).
pub const NAME_CHARSET: &str = "[a-z0-9-]";

/// Maximum canonical root-name length, in bytes (schema §5.2).
pub const NAME_MAX_BYTES: usize = 64;

/// The no-partial-load clause every refusal carries (schema §8.3, clause 2).
/// Ratified fail-loud is only visible if the refusal SAYS nothing loaded — a
/// reader who is not told assumes a partial config took effect.
pub const NO_PARTIAL_LOAD_CLAUSE: &str =
    "No mount table was loaded; the config is not partially applied.";

/// The teaching refusal for an unknown mount field, pinned VERBATIM as the
/// exemplar of the shape schema §8.3 fixes — the house pinned-`const` pattern
/// (`model::selector::D1_TEACHING_REFUSAL_EXEMPLAR`). Its three mandatory
/// clauses each close a specific failure: the LINE (the ratified "and where"),
/// [`NO_PARTIAL_LOAD_CLAUSE`], and a `Fix:` naming the legal form.
///
/// `refusal_exemplar_is_produced_not_asserted` reproduces this string from a
/// real parse, so a drift in the wording is a visible test failure rather than
/// a comment that went stale.
pub const UNKNOWN_FIELD_REFUSAL_EXEMPLAR: &str = "refused: ~/MERIDIAN.md line 14: unknown field `paths` in a meridian-mount block — legal fields are name, path, kind, vault, pin (in that order). No mount table was loaded; the config is not partially applied. Fix: remove the line or spell the field you meant.";

/// Why a config refused. The closed reason set of schema §8.2 — a reason word
/// is a `&'static str` from [`Reason::word`], never free text, which is what
/// keeps a refusal's spelling from drifting and makes it testable.
///
/// The set is an ENUM rather than a string convention so the compiler
/// enumerates it: a new reason cannot be minted at a call site, and
/// [`Reason::ALL`] cannot fall behind the variants without failing to compile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reason {
    /// State C: `MERIDIAN_CONFIG` names something that is not a readable
    /// regular file.
    ConfigPathUnusable,
    /// Rung 2 cannot be built: `$HOME` unset or empty.
    HomeUnresolvable,
    /// The file does not open with `---\n`, or the frontmatter fence never
    /// closes.
    NoFrontmatter,
    /// The frontmatter is not parseable YAML.
    FrontmatterUnparseable,
    /// A required *frontmatter* key is absent.
    MissingRequiredKey,
    /// `type:` is present and is not [`CONFIG_TYPE`].
    WrongTypeValue,
    /// `version:` is an integer this build does not implement.
    UnsupportedVersion,
    /// A required *block* field is absent, including a kind-conditional one.
    MissingRequiredField,
    /// A block line's key is not in that block's legal set.
    UnknownField,
    /// A key appears twice in one block.
    DuplicateField,
    /// A block's fields are not in canonical order.
    FieldOutOfOrder,
    /// A field is present that its block's `kind` forbids.
    FieldNotPermittedForKind,
    /// A value violates its field's type or charset.
    BadValue,
    /// A block body line is not `key: value`, or a `config:` payload line is
    /// not indented.
    MalformedLine,
    /// An engine block's fence never closes.
    UnterminatedBlock,
    /// Two `meridian-mount` blocks declare the same `name`.
    DuplicateMountName,
    /// Two `meridian-tool` blocks declare the same `name`.
    DuplicateToolName,
}

impl Reason {
    /// Every reason word, in schema §8.2's table order. The pack's refusal
    /// cases are one per class; this array is what lets a test assert the
    /// closed set has not silently grown or shrunk.
    pub const ALL: [Reason; 17] = [
        Reason::ConfigPathUnusable,
        Reason::HomeUnresolvable,
        Reason::NoFrontmatter,
        Reason::FrontmatterUnparseable,
        Reason::MissingRequiredKey,
        Reason::WrongTypeValue,
        Reason::UnsupportedVersion,
        Reason::MissingRequiredField,
        Reason::UnknownField,
        Reason::DuplicateField,
        Reason::FieldOutOfOrder,
        Reason::FieldNotPermittedForKind,
        Reason::BadValue,
        Reason::MalformedLine,
        Reason::UnterminatedBlock,
        Reason::DuplicateMountName,
        Reason::DuplicateToolName,
    ];

    /// The reason word — the closed-set spelling schema §8.2 fixes.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Reason::ConfigPathUnusable => "config-path-unusable",
            Reason::HomeUnresolvable => "home-unresolvable",
            Reason::NoFrontmatter => "no-frontmatter",
            Reason::FrontmatterUnparseable => "frontmatter-unparseable",
            Reason::MissingRequiredKey => "missing-required-key",
            Reason::WrongTypeValue => "wrong-type-value",
            Reason::UnsupportedVersion => "unsupported-version",
            Reason::MissingRequiredField => "missing-required-field",
            Reason::UnknownField => "unknown-field",
            Reason::DuplicateField => "duplicate-field",
            Reason::FieldOutOfOrder => "field-out-of-order",
            Reason::FieldNotPermittedForKind => "field-not-permitted-for-kind",
            Reason::BadValue => "bad-value",
            Reason::MalformedLine => "malformed-line",
            Reason::UnterminatedBlock => "unterminated-block",
            Reason::DuplicateMountName => "duplicate-mount-name",
            Reason::DuplicateToolName => "duplicate-tool-name",
        }
    }
}

/// A config refusal: the reason word, the config path, the 1-based FILE line,
/// what was found, and what is legal (schema §8.1, §8.3).
///
/// `line` is 1-based **in the file**, not within a block — a human editing
/// `MERIDIAN.md` is looking at file lines. `None` only where there are no bytes
/// to point at: state C and `home-unresolvable` (schema §8.1a).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    /// The closed-set reason word's variant.
    pub reason: Reason,
    /// The config path the refusal is about — `MERIDIAN_CONFIG` means the file
    /// may be anywhere, so the path is never implicit.
    pub path: PathBuf,
    /// The 1-based FILE line, per schema §8.1a's three exhaustive cases.
    pub line: Option<usize>,
    /// What was found and what is legal — the teaching middle of §8.3.
    /// Ends with `.`; it is prose about a closed-set reason, never the reason.
    pub detail: String,
    /// The `Fix:` clause — §8.3's third mandatory clause. Ends with `.`.
    pub fix: String,
}

impl ConfigError {
    fn new(
        reason: Reason,
        path: &Path,
        line: Option<usize>,
        detail: impl Into<String>,
        fix: impl Into<String>,
    ) -> ConfigError {
        ConfigError {
            reason,
            path: path.to_path_buf(),
            line,
            detail: detail.into(),
            fix: fix.into(),
        }
    }
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "refused: {}", self.path.display())?;
        if let Some(line) = self.line {
            write!(f, " line {line}")?;
        }
        write!(
            f,
            ": {} {NO_PARTIAL_LOAD_CLAUSE} Fix: {}",
            self.detail, self.fix
        )
    }
}

impl std::error::Error for ConfigError {}

/// A root's kind (schema §5.1 field 3). Two values, and a third needs a
/// ruling rather than a config line: kind selects the pin grain (a parsed span
/// versus the whole file) and whether `vault:` is required, so an unknown kind
/// has no defined behaviour to fall back to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MountKind {
    /// An Obsidian vault: parsed, section-grain pins, carries a vault name.
    Vault,
    /// A plain git folder: no parse, no sections, file-grain pins.
    GitFolder,
}

impl MountKind {
    /// The kind's spelling in the file.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            MountKind::Vault => "vault",
            MountKind::GitFolder => "git-folder",
        }
    }
}

/// One declared mount entry — the bytes of one `meridian-mount` block, parsed.
///
/// `path` is carried **verbatim**: canonicalization happens at BIND, which is
/// the mount table's (U7). Storing a canonicalized path here would mean two
/// crates canonicalize and a symlink could be spelled two ways.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountEntry {
    /// The canonical root name — the mount-table key.
    pub name: String,
    /// The local path, verbatim as written.
    pub path: String,
    /// `vault` or `git-folder`.
    pub kind: MountKind,
    /// The Obsidian vault name; `Some` iff `kind` is [`MountKind::Vault`].
    pub vault: Option<String>,
    /// The mount-as-claim pin (schema §5.3) — a well-formed fingerprint
    /// CID-token, carried VERBATIM. Normalizing, truncating, or re-minting it
    /// would break the claim it exists to carry.
    pub pin: Option<String>,
    /// The 1-based FILE line of this block's opening fence.
    ///
    /// Carried because a **bind** refusal is about one mount and must point at
    /// it, and by then the raw bytes are gone: [`Config`] holds the parse, not
    /// the file. §8.1a's absent-case rule already names the opening fence as
    /// the line an author can always see, so this is that same line kept rather
    /// than recomputed.
    pub fence_line: usize,
}

/// One declared tool — the engine-read half of a `meridian-tool` block.
///
/// The `config:` payload is **engine-opaque** (schema §6.1): the engine
/// validates that it is present and indented and never interprets a byte of
/// it. A tool this machine has not installed is not a broken config; it is a
/// statement addressed to someone else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDecl {
    /// The tool's name — the same charset as a root name.
    pub name: String,
    /// The tool's kind. Deliberately an open set: v1 owns zero kinds, so a
    /// closed set would admit nothing.
    pub kind: String,
    /// The payload, verbatim, indentation included — including its trailing
    /// newline. `None` when the block declares no `config:`.
    pub config: Option<String>,
}

/// A parsed `MERIDIAN.md`.
///
/// Fields are private and there is no public constructor: the only way to hold
/// a `Config` is to have parsed a whole file cleanly ([`parse`]). That is what
/// makes "no partial mount table" a property of the type — a caller cannot be
/// handed a half-populated one, because a half-populated one cannot exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    path: PathBuf,
    file_rev: String,
    fingerprint: Option<String>,
    mounts: Vec<MountEntry>,
    tools: Vec<ToolDecl>,
}

impl Config {
    /// The file this config was parsed from.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The config's own rev: `blake3(raw file bytes)[:16]`, the document root
    /// node's `node_rev` (schema §7.1). Spelled `file_rev` because that is
    /// what the wire already calls a whole-page rev — one name per thing, no
    /// new rev noun for the config.
    ///
    /// It is a **reported number, never a verdict** (schema §7.3): editing the
    /// file moves it, and there is no attestation baseline for `~/MERIDIAN.md`
    /// to compare it against (§9). Drift that IS a verdict is a mount's `pin`.
    #[must_use]
    pub fn file_rev(&self) -> &str {
        &self.file_rev
    }

    /// The config's content fingerprint — a `fp1.…` CID-token, like any page.
    /// `None` only under R31's one condition: a span whose norm-v2
    /// canonicalization is empty, which cannot be minted.
    #[must_use]
    pub fn fingerprint(&self) -> Option<&str> {
        self.fingerprint.as_deref()
    }

    /// The declared mounts, in document order.
    #[must_use]
    pub fn mounts(&self) -> &[MountEntry] {
        &self.mounts
    }

    /// The declared tools, in document order.
    #[must_use]
    pub fn tools(&self) -> &[ToolDecl] {
        &self.tools
    }
}

/// What the bootstrap chain resolved to.
///
/// State D (a clean parse declaring zero mounts) is **not** a variant: it is
/// `Loaded` with an empty mount table, and [`Resolution::mounts`] returns the
/// same empty slice state A returns. That identity is deliberate — "empty" and
/// "absent" reaching different code paths is how a nil-vs-empty bug is born,
/// so they reach the same one. The config's own rev is the single permitted
/// difference, and it is an observation, never a branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// State A: the chain resolved a path and no file is there. Current
    /// single-root behaviour, unchanged. Not an error and not a warning —
    /// every machine starts here.
    Absent {
        /// The path the chain resolved to and found nothing at.
        path: PathBuf,
    },
    /// A file was found and parsed clean. States D and "loaded" alike.
    Loaded(Config),
}

impl Resolution {
    /// The path the chain resolved to, present or absent.
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Resolution::Absent { path } => path,
            Resolution::Loaded(config) => config.path(),
        }
    }

    /// The mount table. The empty one in state A and state D alike — ONE code
    /// path, which is the whole point of naming state D.
    #[must_use]
    pub fn mounts(&self) -> &[MountEntry] {
        match self {
            Resolution::Absent { .. } => &[],
            Resolution::Loaded(config) => config.mounts(),
        }
    }

    /// The declared tools. Empty in state A.
    #[must_use]
    pub fn tools(&self) -> &[ToolDecl] {
        match self {
            Resolution::Absent { .. } => &[],
            Resolution::Loaded(config) => config.tools(),
        }
    }

    /// The config's own rev — `Some` iff a file was parsed. This is the ONLY
    /// permitted observable difference between state A and state D (schema
    /// §2.2).
    #[must_use]
    pub fn file_rev(&self) -> Option<&str> {
        match self {
            Resolution::Absent { .. } => None,
            Resolution::Loaded(config) => Some(config.file_rev()),
        }
    }

    /// The parsed config, when one was loaded.
    #[must_use]
    pub fn config(&self) -> Option<&Config> {
        match self {
            Resolution::Absent { .. } => None,
            Resolution::Loaded(config) => Some(config),
        }
    }
}

/// The two environment values the bootstrap chain reads, taken as data rather
/// than from the process.
///
/// Injected rather than read globally because the four resolution states are
/// distinguished BY these values: a test that had to mutate the process
/// environment could not assert them in parallel, and an engine that reads the
/// process environment deep inside a parse cannot be driven by a caller that
/// holds different ones (a daemon serving two workspaces, say).
/// [`Env::from_process`] is the one place the process is read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Env {
    /// `MERIDIAN_CONFIG` — rung 1. `None` = unset.
    pub meridian_config: Option<String>,
    /// `HOME` — rung 2's base. `None` = unset.
    pub home: Option<String>,
}

impl Env {
    /// Read [`CONFIG_ENV_VAR`] and `HOME` from the process environment.
    #[must_use]
    pub fn from_process() -> Env {
        Env {
            meridian_config: std::env::var(CONFIG_ENV_VAR).ok(),
            home: std::env::var("HOME").ok(),
        }
    }
}

/// Which rung of the chain supplied the path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rung {
    /// Rung 1 — `MERIDIAN_CONFIG` stated it. An unusable path here is state C.
    Override,
    /// Rung 2 — `$HOME/MERIDIAN.md`. An absent file here is state A.
    Default,
}

/// Resolve the bootstrap chain to a path, without touching the filesystem.
///
/// Exactly two rungs (schema §2.1) and there is no third: project-local
/// walk-up discovery is deferred by the ratifying decision. An empty or
/// whitespace-only `MERIDIAN_CONFIG` states no path and is treated as unset,
/// so the chain falls to rung 2 — the same nil-vs-empty distinction state D
/// draws for the mount table, applied on the env axis so the two cannot
/// diverge.
///
/// # Errors
/// [`Reason::HomeUnresolvable`] when rung 1 states nothing and `$HOME` is unset
/// or empty. That is not the absent case: the absent case means the default
/// path was resolved and nothing is there.
pub fn resolve_path(env: &Env) -> Result<PathBuf, ConfigError> {
    resolve_rung(env).map(|(path, _)| path)
}

fn resolve_rung(env: &Env) -> Result<(PathBuf, Rung), ConfigError> {
    if let Some(stated) = env.meridian_config.as_deref()
        && !stated.trim().is_empty()
    {
        return Ok((PathBuf::from(stated), Rung::Override));
    }
    let Some(home) = env.home.as_deref().filter(|h| !h.trim().is_empty()) else {
        return Err(ConfigError::new(
            Reason::HomeUnresolvable,
            Path::new("$HOME/MERIDIAN.md"),
            None,
            format!(
                "$HOME is unset or empty, so the default config path $HOME/{CONFIG_FILENAME} cannot be built."
            ),
            format!("export HOME, or state the config path explicitly with {CONFIG_ENV_VAR}."),
        ));
    };
    Ok((Path::new(home).join(CONFIG_FILENAME), Rung::Default))
}

/// Resolve the chain and load whatever it finds — the whole config plane in
/// one call, covering all four states of schema §2.2.
///
/// # Errors
/// [`ConfigError`]. State B (present but malformed) carries the parse's own
/// reason word and file line; state C ([`Reason::ConfigPathUnusable`]) fires
/// when `MERIDIAN_CONFIG` names something that is not a readable regular file
/// — never a silent fallback to `~/MERIDIAN.md`, which would mask an operator
/// intent that cannot be honoured.
pub fn resolve(env: &Env) -> Result<Resolution, ConfigError> {
    let (path, rung) = resolve_rung(env)?;
    match std::fs::metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && rung == Rung::Default => {
            // State A. Every machine starts here; failing would brick the CLI
            // on first run.
            Ok(Resolution::Absent { path })
        }
        Err(e) => Err(unusable(&path, rung, &e.to_string())),
        Ok(meta) if !meta.is_file() => Err(unusable(
            &path,
            rung,
            "it is not a regular file (a directory, a dangling symlink, or a special file)",
        )),
        Ok(_) => {
            let raw = std::fs::read_to_string(&path)
                .map_err(|e| unusable(&path, rung, &e.to_string()))?;
            parse(&raw, &path).map(Resolution::Loaded)
        }
    }
}

/// The state-C refusal. Rung 1 and rung 2 share the reason word because the
/// operator's next action is identical (schema §2.3); the detail names which
/// rung stated the path, because the fix differs.
fn unusable(path: &Path, rung: Rung, why: &str) -> ConfigError {
    let stated = match rung {
        Rung::Override => format!("{CONFIG_ENV_VAR} names"),
        Rung::Default => "the default config path is".to_string(),
    };
    let fix = match rung {
        Rung::Override => {
            format!(
                "point {CONFIG_ENV_VAR} at a readable {CONFIG_FILENAME}, or unset it to fall back to $HOME/{CONFIG_FILENAME}."
            )
        }
        Rung::Default => format!("replace it with a readable {CONFIG_FILENAME}, or remove it."),
    };
    ConfigError::new(
        Reason::ConfigPathUnusable,
        path,
        None,
        format!("{stated} {} but {why}.", path.display()),
        fix,
    )
}

/// Parse `MERIDIAN.md` bytes. `path` is carried into every refusal, since
/// [`CONFIG_ENV_VAR`] means the file may be anywhere.
///
/// The machine surface is the frontmatter plus every fenced block whose
/// info-string names an engine block-language; everything else is prose the
/// engine never parses and never refuses because of (schema §3). That scoping
/// is the law that makes markdown-as-config safe — without it, adding a
/// sentence of documentation to your own config could break your system.
///
/// A malformed config produces **exactly one** refusal — the first, in file
/// order (schema §8.4): the file does not half-load, so no second fault is
/// meaningful, and a cascade would bury the one the operator must fix.
///
/// # Errors
/// [`ConfigError`] — one of the closed reason set, naming what is broken and
/// where.
pub fn parse(raw: &str, path: &Path) -> Result<Config, ConfigError> {
    let doc = model::build(raw.to_string(), syntax::parse(raw));

    let fm = find_frontmatter(&doc.root).ok_or_else(|| {
        ConfigError::new(
            Reason::NoFrontmatter,
            path,
            Some(1),
            format!(
                "the file does not open with a closed `---` frontmatter block, so it cannot say that it is a {CONFIG_TYPE} or which schema version it speaks."
            ),
            format!("open the file with a `---` line, then `type: {CONFIG_TYPE}` and `version: {VERSION}`, then a closing `---` line."),
        )
    })?;
    check_frontmatter(raw, fm, path)?;

    let mut mounts: Vec<MountEntry> = Vec::new();
    let mut tools: Vec<ToolDecl> = Vec::new();
    let mut mount_name_lines: Vec<(String, usize)> = Vec::new();
    let mut tool_name_lines: Vec<(String, usize)> = Vec::new();

    for block in engine_blocks(raw, &doc.root) {
        match block.lang.as_str() {
            MOUNT_LANG => {
                let (entry, name_line) = parse_mount(&block, path)?;
                if let Some((_, first)) = mount_name_lines.iter().find(|(n, _)| *n == entry.name) {
                    return Err(duplicate_name(
                        Reason::DuplicateMountName,
                        path,
                        &entry.name,
                        name_line,
                        *first,
                        MOUNT_LANG,
                    ));
                }
                mount_name_lines.push((entry.name.clone(), name_line));
                mounts.push(entry);
            }
            TOOL_LANG => {
                let (decl, name_line) = parse_tool(&block, path)?;
                if let Some((_, first)) = tool_name_lines.iter().find(|(n, _)| *n == decl.name) {
                    return Err(duplicate_name(
                        Reason::DuplicateToolName,
                        path,
                        &decl.name,
                        name_line,
                        *first,
                        TOOL_LANG,
                    ));
                }
                tool_name_lines.push((decl.name.clone(), name_line));
                tools.push(decl);
            }
            // Another engine block-language in the reserved namespace. SKIPPED,
            // following the shipped precedent for exactly this case — "an
            // engine block is form-3's (or a later engine reader's), never a
            // form-2 chain block" (`crates/view/src/read_face.rs:622-648`). The
            // schema's §3.1 table names two languages and rules nothing about a
            // third; reported to the stage-3 leader as a spec gap rather than
            // ruled here.
            _ => {}
        }
    }

    Ok(Config {
        path: path.to_path_buf(),
        file_rev: doc.root.node_rev.0.clone(),
        fingerprint: model::fingerprint::fingerprint(&doc, &doc.root)
            .ok()
            .map(model::fingerprint::Fingerprint::into_string),
        mounts,
        tools,
    })
}

fn duplicate_name(
    reason: Reason,
    path: &Path,
    name: &str,
    second: usize,
    first: usize,
    lang: &str,
) -> ConfigError {
    ConfigError::new(
        reason,
        path,
        Some(second),
        format!(
            "a second {lang} block declares the name `{name}`, already declared at line {first} — a name is a key, and a map with two values for one key is not a map."
        ),
        format!("rename one of the two blocks, or delete the duplicate declared at line {second}."),
    )
}

// ---------------------------------------------------------------------------
// Frontmatter (schema §4)
// ---------------------------------------------------------------------------

fn find_frontmatter(node: &model::Node) -> Option<&model::Node> {
    if matches!(node.kind, NodeKind::Frontmatter { .. }) {
        return Some(node);
    }
    node.children.iter().find_map(find_frontmatter)
}

/// The frontmatter node's span is fence-to-fence, terminator-inclusive
/// (`crates/model/src/lib.rs`, `frontmatter_node_is_fence_to_fence_terminator_inclusive`).
/// The YAML the engine reads is the block between the two `---` lines; this
/// returns it with the FILE line its first line sits on.
fn frontmatter_inner(raw: &str, fm: &model::Node) -> (String, usize) {
    let slice = raw.get(fm.span.clone()).unwrap_or_default();
    let open_line = line_at(raw, fm.span.start);
    let mut lines: Vec<&str> = slice.lines().collect();
    if !lines.is_empty() {
        lines.remove(0); // the opening `---`
    }
    if lines.last().is_some_and(|l| l.trim_end() == "---") {
        lines.pop(); // the closing `---`
    }
    (lines.join("\n"), open_line + 1)
}

fn check_frontmatter(raw: &str, fm: &model::Node, path: &Path) -> Result<(), ConfigError> {
    let (inner, inner_first_line) = frontmatter_inner(raw, fm);

    let inner_line_count = inner.lines().count().max(1);
    let value: serde_yaml::Value = serde_yaml::from_str(&inner).map_err(|e| {
        // serde_yaml locations are 1-based over the slice it was handed, and an
        // unclosed construct is reported where the parser gave up — one line
        // PAST the block, since the block is all it was handed. Clamping into
        // the block is what keeps §8.1a's "its own line" a line the author can
        // actually see; the parser's own message, carried verbatim below, is
        // what names the construct.
        let line = e.location().map_or(1, |loc| {
            inner_first_line + loc.line().clamp(1, inner_line_count) - 1
        });
        ConfigError::new(
            Reason::FrontmatterUnparseable,
            path,
            Some(line),
            format!("the frontmatter is not parseable YAML — the parser says: {e}."),
            "fix the YAML the parser named, then re-run.".to_string(),
        )
    })?;

    // Unknown keys are permitted and ignored — the shipped posture for
    // markdown-as-config frontmatter (`crates/policy/src/convention.rs`), and
    // what lets a user carry Obsidian properties on their own entry page. Safe
    // in v1 only because v1 defines NO optional frontmatter key the engine
    // reads: both are required, so a typo of either fails loud.
    let map = value.as_mapping();

    let type_value = map.and_then(|m| m.get(serde_yaml::Value::from("type")));
    let Some(type_value) = type_value else {
        return Err(missing_key(path, "type"));
    };
    if type_value.as_str() != Some(CONFIG_TYPE) {
        let found = scalar_text(type_value);
        return Err(ConfigError::new(
            Reason::WrongTypeValue,
            path,
            Some(key_line(&inner, inner_first_line, "type")),
            format!("`type:` is `{found}`, but a config must declare `type: {CONFIG_TYPE}`."),
            format!("set `type: {CONFIG_TYPE}`, or point {CONFIG_ENV_VAR} at the file you meant."),
        ));
    }

    let version_value = map.and_then(|m| m.get(serde_yaml::Value::from("version")));
    let Some(version_value) = version_value else {
        return Err(missing_key(path, "version"));
    };
    let version_line = key_line(&inner, inner_first_line, "version");
    let Some(version) = version_value.as_u64() else {
        return Err(ConfigError::new(
            Reason::BadValue,
            path,
            Some(version_line),
            format!(
                "`version:` is `{}`, which is not an integer — a non-integer version is a typo, not an unsupported version, and the two have different fixes.",
                scalar_text(version_value)
            ),
            format!("set `version: {VERSION}`."),
        ));
    };
    if version != VERSION {
        return Err(ConfigError::new(
            Reason::UnsupportedVersion,
            path,
            Some(version_line),
            format!(
                "`version: {version}` is a schema version this build does not implement — this build implements version {VERSION}, and it will not guess a future format."
            ),
            "upgrade the engine, or set `version: 1` and write the v1 grammar.".to_string(),
        ));
    }
    Ok(())
}

fn missing_key(path: &Path, key: &str) -> ConfigError {
    ConfigError::new(
        Reason::MissingRequiredKey,
        path,
        Some(1),
        format!(
            "the frontmatter declares no `{key}:` — both `type` and `version` are required, which is what makes unknown frontmatter keys safe to ignore."
        ),
        match key {
            "type" => format!("add `type: {CONFIG_TYPE}` to the frontmatter."),
            _ => format!("add `version: {VERSION}` to the frontmatter."),
        },
    )
}

/// The FILE line a top-level frontmatter key sits on. Top-level keys sit at
/// column 0 — the same rule `syntax::frontmatter_keys` and
/// `model::parse_frontmatter` already apply, reused rather than re-spelled.
fn key_line(inner: &str, inner_first_line: usize, key: &str) -> usize {
    inner
        .lines()
        .enumerate()
        .find(|(_, l)| {
            !l.starts_with([' ', '\t']) && l.split_once(':').is_some_and(|(k, _)| k.trim() == key)
        })
        .map_or(1, |(i, _)| inner_first_line + i)
}

/// A YAML value as the refusal should name it: a scalar verbatim, anything
/// else by its shape.
fn scalar_text(value: &serde_yaml::Value) -> String {
    match value {
        serde_yaml::Value::Null => "null".to_string(),
        serde_yaml::Value::Bool(b) => b.to_string(),
        serde_yaml::Value::Number(n) => n.to_string(),
        serde_yaml::Value::String(s) => s.clone(),
        serde_yaml::Value::Sequence(_) => "a sequence".to_string(),
        serde_yaml::Value::Mapping(_) => "a mapping".to_string(),
        serde_yaml::Value::Tagged(t) => format!("!{}", t.tag),
    }
}

// ---------------------------------------------------------------------------
// The engine block surface (schema §3, §5, §6)
// ---------------------------------------------------------------------------

/// One engine block, located: its language, its opening-fence FILE line, its
/// body lines paired with their FILE lines, and whether its fence closed.
struct Block {
    lang: String,
    fence_line: usize,
    unterminated: bool,
    body: Vec<(usize, String)>,
}

/// Every `meridian-*` fenced block in the document, in document order.
///
/// The namespace predicate is [`lock::is_meridian_lang`] — the sole owner of
/// the reserved prefix (decision #8 §1), so this crate grows no second
/// spelling of it. Blocks are located through the MODEL TREE, not by scanning
/// lines: that is what makes a ` ```yaml ` block with mount-shaped keys, a
/// ` ```meridian-mount ` nested inside a four-backtick fence, an indented
/// snippet, and inline code all inert without a single special case
/// (`corpus/prose-decoys.md`).
fn engine_blocks(raw: &str, root: &model::Node) -> Vec<Block> {
    let mut spans: Vec<(model::ByteSpan, String, bool)> = Vec::new();
    collect_engine_blocks(root, &mut spans);
    spans.sort_by_key(|(span, _, _)| span.start);
    spans
        .into_iter()
        .map(|(span, lang, unterminated)| {
            let fence_line = line_at(raw, span.start);
            let slice = raw.get(span).unwrap_or_default();
            let mut lines: Vec<&str> = slice.lines().collect();
            if !lines.is_empty() {
                lines.remove(0); // the opening fence
            }
            if !unterminated && lines.last().is_some_and(|l| l.trim_end() == "```") {
                lines.pop(); // the closing fence
            }
            Block {
                // The FIRST whitespace token decides the language, matching
                // every existing reader; a trailing string is ignored.
                lang: lang.split_whitespace().next().unwrap_or("").to_string(),
                fence_line,
                unterminated,
                body: lines
                    .into_iter()
                    .enumerate()
                    .map(|(i, l)| (fence_line + 1 + i, l.to_string()))
                    .collect(),
            }
        })
        .collect()
}

fn collect_engine_blocks(node: &model::Node, out: &mut Vec<(model::ByteSpan, String, bool)>) {
    if let NodeKind::CodeBlock { lang, unterminated } = &node.kind
        && lock::is_meridian_lang(lang)
    {
        out.push((node.span.clone(), lang.clone(), *unterminated));
    }
    for child in &node.children {
        collect_engine_blocks(child, out);
    }
}

fn unterminated(block: &Block, path: &Path) -> ConfigError {
    ConfigError::new(
        Reason::UnterminatedBlock,
        path,
        Some(block.fence_line),
        format!(
            "the {} block opened here never closes, so where the machine surface ends is unknown — the engine will not guess it by parsing to end-of-file.",
            block.lang
        ),
        "add a closing ``` fence to the block.".to_string(),
    )
}

/// The per-block field cursor: canonical order, one spelling per fact.
struct Fields<'a> {
    legal: &'a [&'a str],
    lang: &'a str,
    seen: Vec<Option<(usize, String)>>,
    max_index: Option<usize>,
    max_line: usize,
}

impl<'a> Fields<'a> {
    fn new(legal: &'a [&'a str], lang: &'a str) -> Fields<'a> {
        Fields {
            legal,
            lang,
            seen: vec![None; legal.len()],
            max_index: None,
            max_line: 0,
        }
    }

    fn legal_list(&self) -> String {
        self.legal.join(", ")
    }

    /// Record one `key: value` line, refusing every structural deviation
    /// schema §5.1 names.
    fn record(
        &mut self,
        path: &Path,
        line: usize,
        key: &str,
        value: &str,
    ) -> Result<(), ConfigError> {
        let Some(index) = self.legal.iter().position(|f| *f == key) else {
            return Err(ConfigError::new(
                Reason::UnknownField,
                path,
                Some(line),
                format!(
                    "unknown field `{key}` in a {} block — legal fields are {} (in that order).",
                    self.lang,
                    self.legal_list()
                ),
                "remove the line or spell the field you meant.".to_string(),
            ));
        };
        if let Some((first, _)) = &self.seen[index] {
            return Err(ConfigError::new(
                Reason::DuplicateField,
                path,
                Some(line),
                format!(
                    "`{key}` appears twice in this {} block — first at line {first}, again here; last-wins and first-wins are both silent choices about which of two stated intents you meant.",
                    self.lang
                ),
                format!("delete one of the two `{key}:` lines."),
            ));
        }
        if self.max_index.is_some_and(|max| index < max) {
            return Err(ConfigError::new(
                Reason::FieldOutOfOrder,
                path,
                Some(self.max_line),
                format!(
                    "this {} block's fields are out of canonical order — the order is {}.",
                    self.lang,
                    self.legal_list()
                ),
                format!("reorder the block's lines to {}.", self.legal_list()),
            ));
        }
        self.seen[index] = Some((line, value.to_string()));
        if self.max_index.is_none_or(|max| index > max) {
            self.max_index = Some(index);
            self.max_line = line;
        }
        Ok(())
    }

    fn get(&self, key: &str) -> Option<(usize, &str)> {
        let index = self.legal.iter().position(|f| *f == key)?;
        self.seen[index]
            .as_ref()
            .map(|(line, value)| (*line, value.as_str()))
    }

    /// A required field that is absent. Per schema §8.1a the line is the
    /// construct's OPENING — a line the author can see, and always present.
    fn require(
        &self,
        path: &Path,
        fence_line: usize,
        key: &str,
    ) -> Result<(usize, &str), ConfigError> {
        self.get(key).ok_or_else(|| {
            ConfigError::new(
                Reason::MissingRequiredField,
                path,
                Some(fence_line),
                format!(
                    "the {} block opened here declares no `{key}:`, which is required — the block's fields are {} (in that order).",
                    self.lang,
                    self.legal_list()
                ),
                format!("add a `{key}:` line to the block."),
            )
        })
    }
}

/// Split one block body line into `key`, `:`, one space, value — the grammar
/// schema §5.1 fixes, modelled on `lock::parse`. Anything else, including a
/// blank line and a bare key, is [`Reason::MalformedLine`]: prose about a
/// mount belongs BESIDE the block, which is what §3's scoping law buys.
fn split_field<'a>(
    line: &'a str,
    path: &Path,
    n: usize,
    lang: &str,
) -> Result<(&'a str, &'a str), ConfigError> {
    let malformed = || {
        ConfigError::new(
            Reason::MalformedLine,
            path,
            Some(n),
            format!(
                "this line in a {lang} block is not `key: value` — blank lines, comments, and indented lines are not part of the block grammar."
            ),
            "write the line as `key: value`, or move the text outside the block.".to_string(),
        )
    };
    let (key, value) = line.split_once(": ").ok_or_else(malformed)?;
    if key.is_empty() || key.contains(char::is_whitespace) {
        return Err(malformed());
    }
    Ok((key, value.trim_end()))
}

// ---------------------------------------------------------------------------
// meridian-mount (schema §5)
// ---------------------------------------------------------------------------

fn parse_mount(block: &Block, path: &Path) -> Result<(MountEntry, usize), ConfigError> {
    if block.unterminated {
        return Err(unterminated(block, path));
    }
    let mut fields = Fields::new(&MOUNT_FIELDS, MOUNT_LANG);
    for (n, line) in &block.body {
        let (key, value) = split_field(line, path, *n, MOUNT_LANG)?;
        fields.record(path, *n, key, value)?;
    }

    // Presence first: an absent field points at the fence, which is earlier in
    // file order than any value fault inside the block (schema §8.4).
    let (name_line, name) = fields.require(path, block.fence_line, "name")?;
    let (path_line, mount_path) = fields.require(path, block.fence_line, "path")?;
    let (kind_line, kind_text) = fields.require(path, block.fence_line, "kind")?;

    check_name(name, path, name_line, "a canonical root name")?;
    if mount_path.trim().is_empty() {
        return Err(ConfigError::new(
            Reason::BadValue,
            path,
            Some(path_line),
            "`path:` is empty — a mount must name a filesystem path.".to_string(),
            "write the root's local path after `path: `.".to_string(),
        ));
    }
    let kind = match kind_text {
        "vault" => MountKind::Vault,
        "git-folder" => MountKind::GitFolder,
        other => {
            return Err(ConfigError::new(
                Reason::BadValue,
                path,
                Some(kind_line),
                format!("`kind: {other}` is not a root kind — the two legal kinds are vault and git-folder."),
                "set `kind: vault` for an Obsidian vault, or `kind: git-folder` for a plain git folder.".to_string(),
            ));
        }
    };

    // The kind-conditional half: the mount table is a THREE-way map, so a
    // vault root without its vault name leaves the map with two legs; a
    // git-folder root has no Obsidian vault, so `vault:` states something that
    // cannot be true.
    let vault = match (kind, fields.get("vault")) {
        (MountKind::Vault, Some((vault_line, vault_name))) => {
            if vault_name.trim().is_empty() {
                return Err(ConfigError::new(
                    Reason::BadValue,
                    path,
                    Some(vault_line),
                    "`vault:` is empty — a vault root must name its Obsidian vault.".to_string(),
                    "write the Obsidian vault name after `vault: `.".to_string(),
                ));
            }
            Some(vault_name.to_string())
        }
        (MountKind::Vault, None) => {
            return Err(ConfigError::new(
                Reason::MissingRequiredField,
                path,
                Some(block.fence_line),
                format!(
                    "the {MOUNT_LANG} block opened here declares `kind: vault` but no `vault:` — the mount table is a three-way map (canonical name, Obsidian vault name, local path), and without the vault name it has two legs."
                ),
                "add a `vault:` line naming the Obsidian vault.".to_string(),
            ));
        }
        (MountKind::GitFolder, Some((vault_line, _))) => {
            return Err(ConfigError::new(
                Reason::FieldNotPermittedForKind,
                path,
                Some(vault_line),
                "`vault:` is not permitted on a `kind: git-folder` mount — a git-folder root has no Obsidian vault, so the field states something that cannot be true.".to_string(),
                "remove the `vault:` line, or set `kind: vault` if this root really is one.".to_string(),
            ));
        }
        (MountKind::GitFolder, None) => None,
    };

    // The pin is a well-formed fingerprint CID-token and NOTHING more: parse is
    // codec-agnostic on purpose, because a git-folder root's pin grain is the
    // file and a vault root's is a parsed span — different codecs. What the
    // pin's target is, and how the claim is checked, is U7's.
    let pin = match fields.get("pin") {
        Some((pin_line, token)) => {
            if model::fingerprint::parse_fingerprint(token).is_none() {
                return Err(ConfigError::new(
                    Reason::BadValue,
                    path,
                    Some(pin_line),
                    format!(
                        "`pin: {token}` is not a well-formed fingerprint token — a token is four `.`-separated fields, version.codec.hashfn.digest, with a hex digest."
                    ),
                    "copy the whole fp token the engine minted, or remove the `pin:` line."
                        .to_string(),
                ));
            }
            Some(token.to_string())
        }
        None => None,
    };

    Ok((
        MountEntry {
            name: name.to_string(),
            path: mount_path.to_string(),
            kind,
            vault,
            pin,
            fence_line: block.fence_line,
        },
        name_line,
    ))
}

// ---------------------------------------------------------------------------
// meridian-tool (schema §6)
// ---------------------------------------------------------------------------

fn parse_tool(block: &Block, path: &Path) -> Result<(ToolDecl, usize), ConfigError> {
    if block.unterminated {
        return Err(unterminated(block, path));
    }
    let mut fields = Fields::new(&TOOL_FIELDS, TOOL_LANG);
    let mut payload: Option<Vec<String>> = None;

    let mut lines = block.body.iter();
    for (n, line) in lines.by_ref() {
        // `config:` is a BARE MARKER line, not a `key: value` line: everything
        // after it is the opaque payload.
        if line.trim_end() == "config:" {
            fields.record(path, *n, "config", "")?;
            payload = Some(Vec::new());
            break;
        }
        let (key, value) = split_field(line, path, *n, TOOL_LANG)?;
        fields.record(path, *n, key, value)?;
    }
    if let Some(collected) = payload.as_mut() {
        for (n, line) in lines {
            // Indentation is the only structural fact the engine needs from the
            // payload: it is what makes the payload's extent unambiguous. A
            // column-0 line is exactly the shape of a field the engine WOULD
            // read, so the ambiguity is real.
            if !line.starts_with([' ', '\t']) {
                return Err(ConfigError::new(
                    Reason::MalformedLine,
                    path,
                    Some(*n),
                    "this line follows `config:` but is not indented — every payload line must be indented by at least one space, which is what makes the payload's extent unambiguous.".to_string(),
                    "indent the line, or move it above `config:` if it is a field.".to_string(),
                ));
            }
            collected.push(line.clone());
        }
    }

    let (name_line, name) = fields.require(path, block.fence_line, "name")?;
    let (kind_line, kind) = fields.require(path, block.fence_line, "kind")?;
    check_name(name, path, name_line, "a tool name")?;
    check_name(kind, path, kind_line, "a tool kind")?;

    // The payload's MEANING is opaque; the block's SHAPE is not. A tool this
    // machine has not installed is a statement addressed to someone else, so
    // the `kind` is an open set — but a malformed DECLARATION still fails loud.
    let config = payload.map(|lines| {
        let mut text = lines.join("\n");
        text.push('\n');
        text
    });

    Ok((
        ToolDecl {
            name: name.to_string(),
            kind: kind.to_string(),
            config,
        },
        name_line,
    ))
}

// ---------------------------------------------------------------------------
// The canonical name charset (schema §5.2)
// ---------------------------------------------------------------------------

/// The charset is DERIVED, not chosen: it is the complement of the address
/// grammar's operator set (`:` `#` `@` `/` `.` `%` and whitespace), so no legal
/// name can ever collide with an address operator. Case is folded out because
/// names travel through URIs and case-insensitive filesystems, and `_` is
/// excluded to match the CHARSET-GUARD ruling.
///
/// This is a FLOOR for `addr::MountName`, not a ceiling: U3 may narrow what a
/// `root:` prefix accepts, never widen it, because a name outside this charset
/// cannot be bound and so could never resolve.
fn check_name(name: &str, path: &Path, line: usize, what: &str) -> Result<(), ConfigError> {
    let bad = |detail: String, fix: String| {
        Err(ConfigError::new(
            Reason::BadValue,
            path,
            Some(line),
            detail,
            fix,
        ))
    };
    if name.is_empty() {
        return bad(
            format!(
                "this value is empty, but {what} must be one or more characters from {NAME_CHARSET}."
            ),
            format!("write a name in {NAME_CHARSET}, for example `field-notes`."),
        );
    }
    if name.len() > NAME_MAX_BYTES {
        return bad(
            format!(
                "`{name}` is {} bytes, but {what} is at most {NAME_MAX_BYTES}.",
                name.len()
            ),
            "shorten the name.".to_string(),
        );
    }
    if let Some(offender) = name
        .chars()
        .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-'))
    {
        return bad(
            format!(
                "`{name}` contains `{offender}`, which is outside the charset for {what} — the legal charset is {NAME_CHARSET}."
            ),
            format!("rewrite the name using only {NAME_CHARSET}, for example `field-notes`."),
        );
    }
    if name.starts_with('-') || name.ends_with('-') {
        return bad(
            format!(
                "`{name}` starts or ends with `-`, which {what} may not — the charset is {NAME_CHARSET} with no leading or trailing `-`."
            ),
            "remove the leading or trailing `-`.".to_string(),
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------

/// The 1-based FILE line a byte offset sits on.
///
/// Offsets come from model node spans, which are char-aligned by construction;
/// a non-boundary offset would be a caller bug, so this walks back to the
/// nearest boundary rather than panicking on a slice.
fn line_at(raw: &str, byte: usize) -> usize {
    let mut end = byte.min(raw.len());
    while end > 0 && !raw.is_char_boundary(end) {
        end -= 1;
    }
    raw[..end].split('\n').count()
}

#[cfg(test)]
mod tests;
