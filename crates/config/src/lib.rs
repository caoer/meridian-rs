//! The `MERIDIAN.md` config plane — the one entry point, parsed as content.
//!
//! Owns: the bootstrap chain (`MERIDIAN_CONFIG` → `$HOME/MERIDIAN.md`), the four
//! resolution states, the in-file schema's strict parse (frontmatter + the
//! `meridian-mount` / `meridian-tool` block grammars), the closed refusal set with
//! 1-based FILE lines, and the config's own rev and fingerprint
//! (`docs/meridian-md-schema.md` §7.1).
//!
//! Never does: bind a mount. Canonicalization at bind, the
//! `workspace::deny_reason` ceiling, the equal-or-nested refusal, and the
//! declared-vs-bound check live in [`mount`] (`docs/address-grammar.md` §8). No
//! project-local walk-up discovery: the chain in [`resolve`] has exactly two
//! rungs. The bridge period's env-var check is [`bridge`]'s.
//!
//! [`Env`] carries the bootstrap variables; [`bridge::BridgeEnv`] carries the two
//! llm-wiki variables. Both treat an empty or whitespace-only value as unset.

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

/// The one schema version this build implements (schema §4). A version this
/// build does not implement is refused, never guessed.
pub const VERSION: u64 = 1;

/// The `meridian-mount` block language (schema §3.1).
pub const MOUNT_LANG: &str = "meridian-mount";

/// The `meridian-tool` block language (schema §3.1).
pub const TOOL_LANG: &str = "meridian-tool";

/// The mount block's fields, in canonical order (schema §5.1).
pub const MOUNT_FIELDS: [&str; 6] = ["name", "path", "kind", "primary", "vault", "pin"];

/// The tool block's fields, in canonical order (schema §6).
pub const TOOL_FIELDS: [&str; 3] = ["name", "kind", "config"];

/// The canonical root-name charset, as the refusal spells it (schema §5.2).
pub const NAME_CHARSET: &str = "[a-z0-9-]";

/// Maximum canonical root-name length, in bytes (schema §5.2).
pub const NAME_MAX_BYTES: usize = 64;

/// The no-partial-load clause every refusal carries (schema §8.3, clause 2).
pub const NO_PARTIAL_LOAD_CLAUSE: &str =
    "No mount table was loaded; the config is not partially applied.";

/// The teaching refusal for an unknown mount field, pinned verbatim as the
/// exemplar of the shape schema §8.3 fixes.
/// `refusal_exemplar_is_produced_not_asserted` reproduces it from a real
/// parse, so drift in the wording fails a test.
pub const UNKNOWN_FIELD_REFUSAL_EXEMPLAR: &str = "refused: ~/MERIDIAN.md line 14: unknown field `paths` in a meridian-mount block — legal fields are name, path, kind, primary, vault, pin (in that order). No mount table was loaded; the config is not partially applied. Fix: remove the line or spell the field you meant.";

/// Why a config refused — the closed reason set of schema §8.2. A reason word
/// comes from [`Reason::word`], never free text.
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
    /// Two `meridian-mount` blocks carry `primary: true` — the designation is
    /// a role exactly one mount may hold, so a second one is a table-level
    /// defect, not a tie to break.
    DuplicatePrimaryDesignation,
}

impl Reason {
    /// Every reason word, in schema §8.2's table order.
    pub const ALL: [Reason; 18] = [
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
        Reason::DuplicatePrimaryDesignation,
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
            Reason::DuplicatePrimaryDesignation => "duplicate-primary-designation",
        }
    }
}

/// A config refusal: the reason word, the config path, the 1-based FILE line,
/// what was found, and what is legal (schema §8.1, §8.3).
///
/// `line` is 1-based in the file, not within a block; `None` only where there
/// are no bytes to point at: state C and `home-unresolvable` (schema §8.1a).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    /// The closed-set reason word's variant.
    pub reason: Reason,
    /// The config path the refusal is about.
    pub path: PathBuf,
    /// The 1-based FILE line, per schema §8.1a's three exhaustive cases.
    pub line: Option<usize>,
    /// What was found and what is legal (§8.3). Ends with `.`.
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

/// A root's kind (schema §5.1 field 3). Closed: kind selects the pin grain
/// and whether `vault:` is required, so an unknown kind has no fallback.
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
/// `path` is carried verbatim: canonicalization happens once, at bind, in the
/// mount table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountEntry {
    /// The canonical root name — the mount-table key.
    pub name: String,
    /// The local path, verbatim as written.
    pub path: String,
    /// `vault` or `git-folder`.
    pub kind: MountKind,
    /// The declared-primary designation (schema §5.1 field 4): `true` iff this
    /// block carries `primary: true`. A binding ROLE for fleet hosts — the one
    /// tree their single-root consumers anchor — parsed and reported here,
    /// never acted on by the engine. Absence is the only "not primary"
    /// spelling; at most one block per file may carry it.
    pub primary: bool,
    /// The Obsidian vault name; `Some` iff `kind` is [`MountKind::Vault`].
    pub vault: Option<String>,
    /// The mount-as-claim pin (schema §5.3) — a well-formed fingerprint
    /// CID-token, carried verbatim.
    pub pin: Option<String>,
    /// The 1-based FILE line of this block's opening fence — kept so a bind
    /// refusal can point at the mount after the raw bytes are gone (§8.1a).
    pub fence_line: usize,
}

/// One declared tool — the engine-read half of a `meridian-tool` block.
///
/// The `config:` payload is engine-opaque (schema §6.1): validated present
/// and indented, never interpreted.
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
/// Fields are private and there is no public constructor: holding a `Config`
/// means a whole file parsed cleanly ([`parse`]) — no partial mount table.
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
    /// node's `node_rev` (schema §7.1). A reported number, never a verdict
    /// (schema §7.3).
    #[must_use]
    pub fn file_rev(&self) -> &str {
        &self.file_rev
    }

    /// The config's content fingerprint — a `fp1.…` CID-token, like any page.
    /// `None` only when the norm-v2 canonicalization is empty.
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
/// State D (a clean parse declaring zero mounts) is not a variant: it is
/// `Loaded` with an empty mount table, so "empty" and "absent" reach one code
/// path. The config's own rev is the single permitted difference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// State A: the chain resolved a path and no file is there. Not an error —
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

    /// The mount table. The empty one in state A and state D alike.
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

    /// The config's own rev — `Some` iff a file was parsed; the only
    /// observable difference between state A and state D (schema §2.2).
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
/// than from the process. [`Env::from_process`] is the one place the process
/// is read.
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

/// Which rung of the chain supplied the path — the resolution's origin.
/// Public because the result alone cannot answer it: an override naming the
/// default path resolves identically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rung {
    /// Rung 1 — `MERIDIAN_CONFIG` stated it. An unusable path here is state C.
    Override,
    /// Rung 2 — `$HOME/MERIDIAN.md`. An absent file here is state A.
    Default,
}

impl Rung {
    /// The operator-facing word — the same two spellings `mrd --help` uses.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Rung::Override => CONFIG_ENV_VAR,
            Rung::Default => "$HOME/MERIDIAN.md",
        }
    }
}

/// Which rung of the chain answers, without touching the filesystem — the
/// sibling of [`resolve_path`], reading the same one decision.
///
/// # Errors
/// [`Reason::HomeUnresolvable`], on exactly the condition [`resolve_path`]
/// raises it: rung 1 states nothing and `$HOME` is unset or empty.
pub fn rung(env: &Env) -> Result<Rung, ConfigError> {
    resolve_rung(env).map(|(_, rung)| rung)
}

/// Resolve the bootstrap chain to a path, without touching the filesystem.
///
/// Exactly two rungs (schema §2.1); project-local walk-up discovery is
/// deferred. An empty or whitespace-only `MERIDIAN_CONFIG` states no path and
/// is treated as unset, so the chain falls to rung 2.
///
/// # Errors
/// [`Reason::HomeUnresolvable`] when rung 1 states nothing and `$HOME` is unset
/// or empty. Not the absent case: absent means the default path was resolved
/// and nothing is there.
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
/// when `MERIDIAN_CONFIG` names something that is not a readable regular file,
/// never a silent fallback to `~/MERIDIAN.md`.
pub fn resolve(env: &Env) -> Result<Resolution, ConfigError> {
    let (path, rung) = resolve_rung(env)?;
    match std::fs::metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && rung == Rung::Default => {
            // State A: every machine starts here; failing would brick first run.
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
/// engine never parses and never refuses because of (schema §3).
///
/// A malformed config produces exactly one refusal — the first, in file order
/// (schema §8.4).
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
    let mut primary_designation: Option<(String, usize)> = None;

    for block in engine_blocks(raw, &doc.root) {
        match block.lang.as_str() {
            MOUNT_LANG => {
                let (entry, name_line, primary_line) = parse_mount(&block, path)?;
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
                // Table-level like duplicate-mount-name: the designation is a
                // role exactly one mount may hold (schema §5.1), and the
                // parser never picks one — the whole table refuses.
                if let Some(second) = primary_line {
                    if let Some((first_name, first)) = &primary_designation {
                        return Err(ConfigError::new(
                            Reason::DuplicatePrimaryDesignation,
                            path,
                            Some(second),
                            format!(
                                "a second {MOUNT_LANG} block (`{}`) declares `primary: true`, already declared by `{first_name}` at line {first} — the primary designation is a role exactly one mount may hold, and nothing picks between two.",
                                entry.name
                            ),
                            format!(
                                "remove the `primary:` line here or the one at line {first}, leaving exactly one designated mount."
                            ),
                        ));
                    }
                    primary_designation = Some((entry.name.clone(), second));
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
            // A third reserved language is skipped, never refused: a block
            // belonging to a later engine reader must not fail the whole
            // config. The render face still shows its bytes.
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

/// The YAML between the two `---` lines (the node's span is fence-to-fence,
/// terminator-inclusive), with the FILE line its first line sits on.
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
        // serde_yaml locations are 1-based over the handed slice, and an
        // unclosed construct is reported one line past it; clamp into the
        // block so the refusal points at a line the author can see.
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

    // Unknown keys are permitted and ignored. Safe in v1 only because v1
    // defines no optional frontmatter key the engine reads: both are
    // required, so a typo of either fails loud.
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

/// The FILE line a top-level (column-0) frontmatter key sits on.
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
/// The namespace predicate is [`lock::is_meridian_lang`], the sole owner of
/// the reserved prefix. Blocks are located through the model tree, not by
/// scanning lines, which keeps decoys (a yaml block with mount-shaped keys, a
/// nested fence, an indented snippet, inline code) inert.
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
                // The first whitespace token decides the language; a trailing
                // string is ignored.
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
    /// construct's opening fence.
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
/// schema §5.1 fixes. Anything else, including a blank line and a bare key,
/// is [`Reason::MalformedLine`].
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

fn parse_mount(
    block: &Block,
    path: &Path,
) -> Result<(MountEntry, usize, Option<usize>), ConfigError> {
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

    let primary = parse_mount_primary(&fields, kind, path)?;

    let vault = parse_mount_vault(&fields, kind, path, block.fence_line)?;

    // Parse checks only that the pin is a well-formed fingerprint token —
    // codec-agnostic, since the two kinds pin different grains. Checking the
    // claim is bind's.
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
            primary: primary.is_some(),
            vault,
            pin,
            fence_line: block.fence_line,
        },
        name_line,
        primary,
    ))
}

// Kind-conditional (schema §5.1 field 5): a vault root requires `vault:`; a
// git-folder root forbids it.
fn parse_mount_vault(
    fields: &Fields<'_>,
    kind: MountKind,
    path: &Path,
    fence_line: usize,
) -> Result<Option<String>, ConfigError> {
    match (kind, fields.get("vault")) {
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
            Ok(Some(vault_name.to_string()))
        }
        (MountKind::Vault, None) => Err(ConfigError::new(
            Reason::MissingRequiredField,
            path,
            Some(fence_line),
            format!(
                "the {MOUNT_LANG} block opened here declares `kind: vault` but no `vault:` — the mount table is a three-way map (canonical name, Obsidian vault name, local path), and without the vault name it has two legs."
            ),
            "add a `vault:` line naming the Obsidian vault.".to_string(),
        )),
        (MountKind::GitFolder, Some((vault_line, _))) => Err(ConfigError::new(
            Reason::FieldNotPermittedForKind,
            path,
            Some(vault_line),
            "`vault:` is not permitted on a `kind: git-folder` mount — a git-folder root has no Obsidian vault, so the field states something that cannot be true.".to_string(),
            "remove the `vault:` line, or set `kind: vault` if this root really is one.".to_string(),
        )),
        (MountKind::GitFolder, None) => Ok(None),
    }
}

// The primary designation (schema §5.1a): optional, literal `true` only —
// absence is the one "not primary" spelling, so `primary: false` refuses
// rather than becoming a second spelling for the same fact. Kind-conditional
// like `vault:`: the primary root is where a fleet daemon writes, so a
// `git-folder` (source repo) designation states something that cannot be
// honoured. Returns the designation's FILE line when present and legal.
fn parse_mount_primary(
    fields: &Fields<'_>,
    kind: MountKind,
    path: &Path,
) -> Result<Option<usize>, ConfigError> {
    let Some((primary_line, value)) = fields.get("primary") else {
        return Ok(None);
    };
    if kind == MountKind::GitFolder {
        return Err(ConfigError::new(
            Reason::FieldNotPermittedForKind,
            path,
            Some(primary_line),
            "`primary:` is not permitted on a `kind: git-folder` mount — the primary root is where the fleet daemon writes, and a git-folder root binds a source repo.".to_string(),
            "remove the `primary:` line, or set `kind: vault` if this root really is one.".to_string(),
        ));
    }
    if value != "true" {
        return Err(ConfigError::new(
            Reason::BadValue,
            path,
            Some(primary_line),
            format!(
                "`primary: {value}` is not a designation — the only legal value is `true`; a mount that is not primary says so by carrying no `primary:` line."
            ),
            "write `primary: true`, or remove the line.".to_string(),
        ));
    }
    Ok(Some(primary_line))
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
        // `config:` is a bare marker line: everything after it is the opaque
        // payload.
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
            // A column-0 line is exactly the shape of a field the engine would
            // read, so every payload line must be indented.
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

/// The charset is the complement of the address grammar's operator set, so no
/// legal name can collide with an address operator; case is folded and `_`
/// excluded. A floor for `addr::MountName`: it may narrow, never widen.
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

/// The 1-based FILE line a byte offset sits on. A non-boundary offset walks
/// back to the nearest boundary rather than panicking.
fn line_at(raw: &str, byte: usize) -> usize {
    let mut end = byte.min(raw.len());
    while end > 0 && !raw.is_char_boundary(end) {
        end -= 1;
    }
    raw[..end].split('\n').count()
}

#[cfg(test)]
mod tests;
