//! The §12 hash domain — which markdown files' bytes enter the merkle root.
//! A three-layer filter, applied in order:
//!
//! 1. **md-only floor** — only `*.md` files hash. `mdfs_config.yaml` is not md, so it is
//!    structurally outside its own domain.
//! 2. **default ignore, one rule** — any path with a dot-prefixed segment is ignored
//!    (`.github/README.md`, `.obsidian/**`, `.trash/**`). Zero enumeration;
//!    `.git`/lock/`.obsidian` artifacts are dissolved by construction and can never
//!    self-invalidate a guard. This floor is structural: custom `!` re-includes cannot
//!    lift it.
//! 3. **custom ignore** — `mdfs_config.yaml` carries a gitignore-style list (patterns,
//!    `!` re-includes) layered over the default. Last matching rule wins.
//!
//! The filter gates HASHING, not `load`: an ignored md file stays fully addressable —
//! `toc`/`cat`/`splice` reach it by explicit path — its bytes never enter the root (hash
//! ⊂ addressable). See [`Domain`] for the predicate and [`crate::hash_domain`] for
//! the walk that consumes it.

use std::cell::OnceCell;
use std::collections::BTreeMap;
use std::io;
use std::path::{Component, Path};

use crate::WorkspaceRoot;

/// The legacy custom-ignore config file. Not markdown ⇒ never in its own
/// domain. Superseded by [`DOMAIN_CONFIG_PATH`]; still read when it is the only
/// config present, so existing workspaces keep working.
pub const CONFIG_FILE_NAME: &str = "mdfs_config.yaml";

/// The custom-ignore config, as markdown — the declaration surface a workspace
/// should use.
///
/// Markdown so the ignore list rides the frontmatter while the body carries
/// each entry's rationale.
///
/// Unlike [`CONFIG_FILE_NAME`] (non-md, structurally outside), a `.md` config
/// is inside its own hash domain, deliberately: the file that defines the
/// attested surface is itself attested, so an ignore-list change moves the root
/// instead of silently reshaping what gets checked. Bootstrap is not circular —
/// read the config, compute the domain, then hash the domain including it.
pub const DOMAIN_CONFIG_PATH: &str = "meridian/domain.md";

/// The attested armed-set artifact (registration ruling § 4) — the one page the
/// door reads to learn a workspace's armed set, one row per armed id. Ordinary
/// in-tree markdown, inside the hash domain: the attestation IS the page, so
/// its rev matters.
///
/// Mirrors `policy::armed::ARMED_RULES_PATH` — `policy` is I/O-free and `fs`
/// knows nothing of rules, so neither crate can name the other's constant.
/// `crates/testsuite/tests/reserved_paths.rs` holds the two spellings together.
pub const ARMED_RULES_PATH: &str = "meridian/armed-rules.md";

/// The once-armed sentinel: its presence, not its bytes, records that a
/// workspace has ever been armed. The gate needs it to tell a never-armed
/// workspace (no artifact, no marker ⇒ a no-op) from a once-armed one whose
/// artifact went missing (⇒ fail closed) — deleting the artifact is exactly the
/// attack the marker defeats.
///
/// Non-markdown on purpose: the md-only hash-domain floor keeps it out of the
/// merkle root, so writing the marker never perturbs the root a write guards.
/// Arming creates it on the first arm and never removes it; the integrity floor
/// refuses its deletion/rename at the door.
pub const ATTESTED_MARKER_PATH: &str = "meridian/attested";

/// The §12 hash-domain filter: the md-only floor + dot-segment default ignore
/// (both fixed and structural) plus the custom ignore rules and the domain
/// `version` parsed from `mdfs_config.yaml`.
///
/// `version` rides the domain config so an ignore-list change can advance the
/// merkle prefix (`b3:` → `b3a:`, §12.3); the prefix *token* is minted by
/// `model::merkle_root`, which reads [`Domain::version`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Domain {
    version: u32,
    rules: Vec<Rule>,
}

/// WHICH §12.1 rule keeps a path out of the hash domain.
///
/// The vocabulary is the contract's own: the md-only floor, the dot-segment
/// default ignore, and the custom rules on `meridian/domain.md`. A face
/// renders [`ExclusionReason::word`]; it never spells one of these itself,
/// because a second spelling is how two faces come to name one rule
/// differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ExclusionReason {
    /// The md-only floor (§12.1 rule 1): the path is not `*.md`.
    NonMarkdown,
    /// The dot-segment default ignore (§12.1 rule 2), structural — a custom
    /// `!` re-include cannot lift it.
    DotSegment,
    /// A custom ignore rule on `meridian/domain.md` (§12.1 rule 3).
    CustomIgnore,
}

impl ExclusionReason {
    /// The face-facing word. One mint, shared by every plane.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            ExclusionReason::NonMarkdown => "non-md",
            ExclusionReason::DotSegment => "dot-segment",
            ExclusionReason::CustomIgnore => "custom-ignore",
        }
    }
}

impl std::fmt::Display for ExclusionReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.word())
    }
}

/// §12.1 rule 2 in ONE spelling: is `name` — a single path segment —
/// dot-prefixed?
///
/// The structural floor is also a WALK law: every enumerator that feeds a
/// serving or caveat face skips a dot-prefixed segment without entering it —
/// the hash-domain walk, the link fallback index, and the declined-markdown
/// scan all call THIS predicate. It exists because two walks once disagreed
/// about the rule (dogfood F11, 2026-08-15): `mrd rules` walked a dot-named
/// snapshot directory the record projection holds zero records for, and
/// reddened for files the engine does not serve. One spelling, and the two
/// faces cannot diverge again.
#[must_use]
pub fn dot_segment(name: &str) -> bool {
    name.starts_with('.')
}

/// WHY a wikilink target is a real file the hash domain does not carry, or
/// `None`. THE ONE MINT for the edge-map planes: the `links` doors and the
/// `sql link` projection both ask through here, so a `dot-segment` in one face
/// can never be a `custom-ignore` in the other (session decision 0034).
///
/// TWO CONDITIONS, and the first is the discriminator the column exists for:
///
/// 1. **A file must really be there.** Without the disk probe a reason would
///    attach to `[[.private/typo]]` — a path with no file behind it — and a
///    GENUINE TYPO MUST CARRY NO REASON. That is what tells an author "your
///    link is broken" apart from "this file is deliberately unhashed".
/// 2. The domain must exclude it, per [`Domain::exclusion`].
///
/// TWO ARMS answer, in order (ruling 2026-08-14, design (a-1)):
///
/// - **Literal**: the target as written plus the `.md` append rule — the
///   spelling a caller writes for a page.
/// - **Bare-name fallback**: a target with no `/` resolves by basename over
///   the out-of-domain files (an excluded file is absent from the corpus
///   index by construction, so the ambient basename search cannot answer).
///   Two guards are ratified scope, not advice: the match is **case-EXACT**
///   (an APFS case-folding probe would stamp `abc.BASE` — a genuine typo —
///   as deliberate), and an ambiguous basename takes the **deterministic
///   tie-break** shortest path then lexicographic, so nothing downstream is
///   walk-order-sensitive.
///
/// A PATHED spelling never falls back, deliberately: `git/GIT.base` written
/// where only `sources/git/GIT.base` exists is genuine rot, and a suffix walk
/// would stamp it as deliberate — erasing exactly the typo-vs-deliberate
/// discriminator. Stated limit, not a bug: a pathed spelling only a suffix
/// walk could find (e.g. `attachments/….xlsx` on the measuring corpus) stays
/// bare; the raw `link` rows remain the escape hatch for censusing those.
///
/// The out-of-domain index walks once, lazily, on the first bare-name miss —
/// dot-segment names are never walked (invisible to the vault the way the
/// app's own index treats them, and the rule that keeps `.git` unwalked), and
/// walk I/O errors read as absence, the same posture as the literal arm's
/// `is_file`.
pub struct LinkTargetProbe<'a> {
    root: &'a WorkspaceRoot,
    domain: &'a Domain,
    /// Exact basename → out-of-domain rel paths, each list sorted by the
    /// ratified tie-break (byte length, then lexicographic) so the pick is
    /// `first()` and determinism holds by construction.
    index: OnceCell<BTreeMap<String, Vec<String>>>,
}

impl<'a> LinkTargetProbe<'a> {
    /// A probe over `root` under `domain`. Cheap: the fallback index is not
    /// built until a bare name misses the literal arm.
    #[must_use]
    pub fn new(root: &'a WorkspaceRoot, domain: &'a Domain) -> Self {
        LinkTargetProbe {
            root,
            domain,
            index: OnceCell::new(),
        }
    }

    /// The §12.1 word for `target`, or `None` — [`resolution`](Self::resolution)
    /// without the path, the shape the edge-map faces render.
    #[must_use]
    pub fn exclusion(&self, target: &str) -> Option<ExclusionReason> {
        self.resolution(target).map(|(_, why)| why)
    }

    /// The full finding: the workspace-relative file `target` resolved to and
    /// WHY the domain excludes it, or `None` when no arm answers.
    #[must_use]
    pub fn resolution(&self, target: &str) -> Option<(String, ExclusionReason)> {
        let rel = Path::new(target);
        let candidate = if rel.extension().is_some() {
            rel.to_path_buf()
        } else {
            rel.with_extension("md")
        };
        if self.root.0.join(&candidate).is_file() {
            let why = self.domain.exclusion(&candidate)?;
            return Some((candidate.to_string_lossy().into_owned(), why));
        }
        // Bare names only: a pathed spelling never falls back (see above).
        if target.contains('/') {
            return None;
        }
        let name = candidate.file_name()?.to_str()?.to_owned();
        let index = self
            .index
            .get_or_init(|| build_fallback_index(self.root, self.domain));
        let path = index.get(&name)?.first()?;
        let why = self.domain.exclusion(Path::new(path))?;
        Some((path.clone(), why))
    }
}

/// Walk `root` for the fallback index: every non-dot-segment file the domain
/// excludes, keyed by exact basename, candidate lists in tie-break order.
/// Per-entry I/O errors skip the entry (absence posture, as `is_file`).
fn build_fallback_index(root: &WorkspaceRoot, domain: &Domain) -> BTreeMap<String, Vec<String>> {
    fn walk(abs: &Path, rel_dir: &str, domain: &Domain, map: &mut BTreeMap<String, Vec<String>>) {
        let Ok(entries) = std::fs::read_dir(abs) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let name = entry.file_name();
            // A non-UTF-8 name can never match a UTF-8 target.
            let Some(name) = name.to_str() else { continue };
            if dot_segment(name) {
                continue;
            }
            let rel = if rel_dir.is_empty() {
                name.to_owned()
            } else {
                format!("{rel_dir}/{name}")
            };
            if file_type.is_dir() {
                // No pruning of custom-ignored directories: their files are
                // exactly what this index exists to find.
                walk(&entry.path(), &rel, domain, map);
            } else if file_type.is_file() && domain.exclusion(Path::new(&rel)).is_some() {
                map.entry(name.to_owned()).or_default().push(rel);
            }
        }
    }
    let mut map = BTreeMap::new();
    walk(&root.0, "", domain, &mut map);
    for paths in map.values_mut() {
        paths.sort_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
    }
    map
}

impl Domain {
    /// The default domain: md-only floor + dot-segment ignore, no custom rules,
    /// `version` 0.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse the domain from `mdfs_config.yaml` text.
    ///
    /// Accepted schema (deliberately minimal — no YAML dependency enters `fs`;
    /// the workspace's single YAML decision is reserved for `policy`): an
    /// optional top-level `version:` integer and an `ignore:`
    /// block sequence of gitignore-style pattern scalars, e.g.
    ///
    /// ```yaml
    /// version: 1
    /// ignore:
    ///   - "drafts/**"
    ///   - "archive/**"
    ///   - "!archive/index.md"
    /// ```
    ///
    /// Comments (`#`, honoured outside quotes) and blank lines are ignored;
    /// unknown keys are tolerated. Flow sequences (`ignore: [a, b]`) are not
    /// parsed — block form only.
    #[must_use]
    pub fn from_config(yaml: &str) -> Self {
        let cfg = parse_config(yaml);
        Domain {
            version: cfg.version,
            rules: cfg.ignore.iter().filter_map(|p| Rule::parse(p)).collect(),
        }
    }

    /// Parse the domain from the markdown config page ([`DOMAIN_CONFIG_PATH`]).
    ///
    /// The ignore list rides the frontmatter block and takes the same schema
    /// [`from_config`](Self::from_config) accepts — the SURFACE moved, the
    /// pattern semantics did not. A page with no frontmatter yields the default
    /// domain: a config that declares nothing ignores nothing.
    #[must_use]
    pub fn from_markdown(md: &str) -> Self {
        Self::from_config(frontmatter(md).unwrap_or(""))
    }

    /// Read the workspace's domain config, or the default domain when none is
    /// present.
    ///
    /// [`DOMAIN_CONFIG_PATH`] is the surface; [`CONFIG_FILE_NAME`] is still
    /// honoured when it is the ONLY config, so existing workspaces keep
    /// working.
    ///
    /// # Errors
    /// I/O failure reading an existing config file. An absent config is not an
    /// error — it yields [`Domain::new`].
    ///
    /// Both present is an error, not a precedence rule: two live ignore lists
    /// are two answers to "what is attested here", so the ambiguity is reported
    /// rather than resolved.
    ///
    /// The ambiguity is NOT `InvalidData`: the dispatch boundary reads that
    /// kind as "a corpus file is not UTF-8" and mints `invalid_utf8`, which
    /// would tell a caller their bytes are corrupt when both configs decode
    /// perfectly. It rides the residual kind, so the wire face is
    /// `io_error{cause}` — env class either way, and the remedy still travels.
    pub fn load(root: &WorkspaceRoot) -> io::Result<Domain> {
        Ok(Scope::load(root)?.domain)
    }

    /// The domain `version` (§12.3) — 0 unless `mdfs_config.yaml` declares one.
    #[must_use]
    pub fn version(&self) -> u32 {
        self.version
    }

    /// Is `rel` — a workspace-relative path — in the HASH domain?
    ///
    /// `false` for any non-md file, any dot-segment path, or a path the custom
    /// rules ignore. A `false` here means "not hashed", never "not
    /// addressable": the ignored file is still `load`-able by explicit path.
    ///
    /// One mint with [`Domain::exclusion`]: membership is the absence of a
    /// reason, so the predicate and the reason word can never disagree about
    /// one path. Two predicates that merely agree drift the moment either is
    /// edited.
    #[must_use]
    pub fn contains(&self, rel: &Path) -> bool {
        self.exclusion(rel).is_none()
    }

    /// WHY `rel` is outside the hash domain, or `None` when it is a member.
    ///
    /// The three §12.1 rules are checked in their published order and the
    /// FIRST one that fires is the answer — the same order [`contains`] has
    /// always evaluated, so this reports the rule that actually decided.
    ///
    /// This exists because four distinct facts — the three exclusion classes
    /// and a genuine typo — collapsed to one word at every face, leaving an
    /// excluded file indistinguishable from a broken link (session decision
    /// 0034). The reason is what ends that collapse; it never changes what is
    /// hashed.
    #[must_use]
    pub fn exclusion(&self, rel: &Path) -> Option<ExclusionReason> {
        // 1. md-only floor.
        if !is_markdown(rel) {
            return Some(ExclusionReason::NonMarkdown);
        }
        let segments: Vec<&str> = rel
            .components()
            .filter_map(|c| match c {
                Component::Normal(s) => s.to_str(),
                _ => None,
            })
            .collect();
        // 2. default ignore — any dot-prefixed segment (structural floor,
        //    above custom rules: a `!` re-include cannot lift a dot path).
        if segments.iter().any(|s| dot_segment(s)) {
            return Some(ExclusionReason::DotSegment);
        }
        // 3. custom ignore — gitignore last-match-wins. `rel` names a FILE
        //    here (the hash domain is a set of files), so dir-only rules match
        //    through `matches_file`; dir-context callers (`prunes_dir`) keep
        //    the raw form.
        let mut ignored = false;
        for rule in &self.rules {
            if rule.matches_file(&segments) {
                ignored = !rule.negate;
            }
        }
        ignored.then_some(ExclusionReason::CustomIgnore)
    }

    /// Is a symlink at `rel` OUTSIDE the detection domain — skippable by the
    /// guarded walk rather than refused?
    ///
    /// The ignored-directory carve-out, extended to the link itself: when the
    /// custom rules ignore `rel` (last-match-wins, so a `!` re-include lifts
    /// the ignore for the exact path it names) and no `!` rule could
    /// re-include anything beneath it, nothing at or behind the link can
    /// reach a hash, attest or receipt surface — the same soundness argument
    /// [`prunes_dir`](Self::prunes_dir) makes for a real directory, and the
    /// dir form matters here because a symlink may BE a directory.
    ///
    /// Reserved paths are engine substrate: neither a reserved path itself
    /// nor a directory on the way to one is ever skippable, whatever the
    /// ignore list says.
    #[must_use]
    pub fn skips_symlink(&self, rel: &Path) -> bool {
        let joined = normalized(rel);
        if RESERVED_PATHS.contains(&joined.as_str()) {
            return false;
        }
        self.prunes_dir(rel)
    }

    /// May a traversal skip `rel_dir` and everything beneath it WITHOUT
    /// changing the hash domain?
    ///
    /// [`contains`](Self::contains) answers per file, so a walk that filters
    /// afterwards has already paid the `stat`; pruning declines to descend at
    /// all. It answers `true` only when both hold:
    ///
    /// 1. the directory itself is ignored by last-match-wins, so every path
    ///    beneath it inherits the ignore; and
    /// 2. no `!` rule could re-include anything beneath it — gitignore
    ///    semantics let `!archive/index.md` survive `archive/**`, and pruning
    ///    would drop it out of the domain, a wrong root rather than a slow one.
    ///
    /// Reserved paths are never pruned: they must stay reachable regardless of
    /// what the ignore list says about the directory holding them.
    #[must_use]
    pub fn prunes_dir(&self, rel_dir: &Path) -> bool {
        let segments: Vec<&str> = rel_dir
            .components()
            .filter_map(|c| match c {
                Component::Normal(s) => s.to_str(),
                _ => None,
            })
            .collect();
        // Never prune the workspace root itself.
        if segments.is_empty() {
            return false;
        }
        // Never prune a directory on the way to a reserved path.
        if RESERVED_PATHS
            .iter()
            .any(|reserved| is_prefix_of_reserved(&segments, reserved))
        {
            return false;
        }
        let mut ignored = false;
        for rule in &self.rules {
            if rule.matches(&segments) {
                ignored = !rule.negate;
            }
        }
        if !ignored {
            return false;
        }
        !self
            .rules
            .iter()
            .any(|rule| rule.negate && rule.could_reinclude_under(&segments))
    }
}

/// The filter, as the question `model` asks — one predicate, one owner. The
/// colour plane needs it to tell *the engine did not look* from *the engine
/// looked and the file is gone* (`wire-contract.md` §12.1, verdict-plane
/// clause); `model` cannot name a filesystem, so it names this trait instead.
impl model::HashDomain for Domain {
    fn contains(&self, rel: &str) -> bool {
        Domain::contains(self, Path::new(rel))
    }
}

/// The reserved-path family: every page that is engine substrate rather than
/// content, and whose parent directories must therefore stay walkable whatever
/// the ignore list says. Public so a cross-crate test can assert the family's
/// membership, not just each member's spelling.
pub const RESERVED_PATHS: &[&str] = &[ARMED_RULES_PATH, ATTESTED_MARKER_PATH];

/// Is `dir` a directory on the path to `reserved`?
fn is_prefix_of_reserved(dir: &[&str], reserved: &str) -> bool {
    let rsegs: Vec<&str> = reserved.split('/').filter(|s| !s.is_empty()).collect();
    // The reserved file's own leaf is not a directory, so a prefix must be
    // strictly shorter than the full reserved path.
    dir.len() < rsegs.len() && dir.iter().zip(rsegs.iter()).all(|(d, r)| d == r)
}

fn is_markdown(p: &Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case("md"))
}

/// Read a file that may legitimately not exist. `None` is absence; every other
/// I/O failure stays an error rather than degrading into "no config", which
/// would silently widen the attested surface on an unreadable file.
fn read_optional(path: &Path) -> io::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// The frontmatter block of a markdown page: the text between a leading `---`
/// fence and the next `---` line. `None` when the page does not open with one.
fn frontmatter(md: &str) -> Option<&str> {
    let rest = md
        .strip_prefix("---\n")
        .or_else(|| md.strip_prefix("---\r\n"))?;
    let end = rest
        .lines()
        .scan(0usize, |offset, line| {
            let start = *offset;
            *offset += line.len() + 1;
            Some((start, line))
        })
        .find(|(_, line)| line.trim_end() == "---")
        .map(|(start, _)| start)?;
    Some(&rest[..end])
}

/// Is `rel` the attested armed-rules artifact ([`ARMED_RULES_PATH`])? Matches on
/// normalized path segments, so a non-canonical spelling
/// (`./meridian/armed-rules.md`) cannot dodge the INDEX-integrity floor at the
/// write door — deleting the artifact must never read as disarming.
#[must_use]
pub fn is_armed_rules(rel: &Path) -> bool {
    normalized(rel) == ARMED_RULES_PATH
}

/// Is `rel` the once-armed marker ([`ATTESTED_MARKER_PATH`])? Normalized like
/// [`is_armed_rules`]. The U4.3 INDEX-integrity floor refuses its
/// deletion/rename at the write door (security F2: deleting the marker is the
/// silent-disarm attack the fail-closed design defeats).
#[must_use]
pub fn is_attested_marker(rel: &Path) -> bool {
    normalized(rel) == ATTESTED_MARKER_PATH
}

/// A path's normalized workspace-relative spelling: `Normal` segments joined by
/// `/`, dropping `.`/`..`/empty components. The single normalizer the reserved-
/// path identity checks share, so a non-canonical spelling defeats none of them.
fn normalized(rel: &Path) -> String {
    rel.components()
        .filter_map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// The effective scope: which config file governs the attested set, and the
/// parsed rules. The watcher's re-scope detection (§ A.9) compares two of
/// these SEMANTICALLY — a prose-only edit to `meridian/domain.md` parses to
/// the same `Domain` and re-scopes nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    /// The config file in effect: [`DOMAIN_CONFIG_PATH`], [`CONFIG_FILE_NAME`],
    /// or `None` (built-in defaults only).
    pub config: Option<&'static str>,
    /// The parsed rules that file (or the default) defines.
    pub domain: Domain,
}

impl Scope {
    /// The effective scope of `root` — [`Domain::load`]'s dispatch with the
    /// governing config file kept beside the rules.
    ///
    /// # Errors
    /// I/O failure reading an existing config file, or both configs present
    /// (the dual-config ambiguity [`Domain::load`] documents).
    pub fn load(root: &WorkspaceRoot) -> io::Result<Scope> {
        let md = read_optional(&root.0.join(DOMAIN_CONFIG_PATH))?;
        let yaml = read_optional(&root.0.join(CONFIG_FILE_NAME))?;
        match (md, yaml) {
            (Some(_), Some(_)) => Err(io::Error::other(format!(
                "two domain configs are present: {DOMAIN_CONFIG_PATH} and {CONFIG_FILE_NAME}. \
                     They may declare different ignore lists, so which files are attested would \
                     depend on a precedence rule no reader of either file can see. \
                     Remedy: keep {DOMAIN_CONFIG_PATH} and delete {CONFIG_FILE_NAME}."
            ))),
            (Some(text), None) => Ok(Scope {
                config: Some(DOMAIN_CONFIG_PATH),
                domain: Domain::from_markdown(&text),
            }),
            (None, Some(text)) => Ok(Scope {
                config: Some(CONFIG_FILE_NAME),
                domain: Domain::from_config(&text),
            }),
            (None, None) => Ok(Scope {
                config: None,
                domain: Domain::new(),
            }),
        }
    }
}

/// One parsed gitignore-style rule. Matching operates on path *segments*.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Rule {
    /// A `!`-prefixed re-include: when it matches it clears the ignore.
    negate: bool,
    /// Rooted at the workspace (leading `/` or an internal `/`); otherwise the
    /// pattern matches at any depth (basename semantics).
    anchored: bool,
    /// A trailing `/` on the pattern: the rule names a DIRECTORY (§12.1 rule 3,
    /// trailing-slash law), so [`matches_file`](Self::matches_file) binds its
    /// segments to parent directories only — never to the file's own name.
    dir_only: bool,
    /// Pattern segments; `**` matches zero or more path segments.
    segs: Vec<String>,
}

impl Rule {
    fn parse(pattern: &str) -> Option<Rule> {
        let mut p = pattern.trim();
        if p.is_empty() || p.starts_with('#') {
            return None;
        }
        let negate = p.starts_with('!');
        if negate {
            p = &p[1..];
        }
        let dir_only = p.ends_with('/');
        let body = p.trim_end_matches('/');
        let anchored = body.starts_with('/') || body.trim_start_matches('/').contains('/');
        let mut segs: Vec<String> = body
            .split('/')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
        if segs.is_empty() {
            return None;
        }
        // A trailing slash means "this directory and everything under it".
        if dir_only {
            segs.push("**".to_string());
        }
        Some(Rule {
            negate,
            anchored,
            dir_only,
            segs,
        })
    }

    /// Does this rule match `path` when it names a FILE?
    ///
    /// A dir-only rule binds to directories, so the file's own — final —
    /// segment is off-limits: the rule matches only through the parents.
    /// Without this split, the `**` a trailing slash compiles to matches ZERO
    /// segments and the "directory" glob lands on the file itself — how
    /// `scratch*/` once dropped the FILE `tasks/scratch-snapS-cleanup.md` out
    /// of a live hash domain (§12.1 rule 3 trailing-slash law, dogfood
    /// 2026-08-15, card record-reproject-on-put).
    fn matches_file(&self, path: &[&str]) -> bool {
        match (self.dir_only, path.split_last()) {
            (true, Some((_, parents))) => self.matches(parents),
            (true, None) => false,
            (false, _) => self.matches(path),
        }
    }

    fn matches(&self, path: &[&str]) -> bool {
        if self.anchored {
            match_segs(&self.segs, path)
        } else {
            // Unanchored: match at any depth.
            (0..=path.len()).any(|i| match_segs(&self.segs, &path[i..]))
        }
    }

    /// Could this `!` rule re-include some path strictly BENEATH `dir`?
    ///
    /// Conservative: every uncertain case answers `true` (walk it) rather than
    /// risk dropping a re-included file out of the hash domain. Two cases admit
    /// no cheap proof and are assumed reachable — an unanchored rule (basename
    /// semantics, matching at any depth) and any rule containing `**`.
    /// Otherwise the rule names a fixed-depth path, and it can only reach under
    /// `dir` when `dir` is a proper prefix of that path.
    fn could_reinclude_under(&self, dir: &[&str]) -> bool {
        if !self.anchored || self.segs.iter().any(|s| s == "**") {
            return true;
        }
        dir.len() < self.segs.len()
            && dir
                .iter()
                .zip(self.segs.iter())
                .all(|(d, p)| seg_glob(p, d))
    }
}

/// Match pattern segments against path segments, with `**` = zero-or-more.
fn match_segs(pat: &[String], path: &[&str]) -> bool {
    match pat.split_first() {
        None => path.is_empty(),
        Some((head, rest)) => {
            if head == "**" {
                (0..=path.len()).any(|i| match_segs(rest, &path[i..]))
            } else if let Some((ph, ptail)) = path.split_first() {
                seg_glob(head, ph) && match_segs(rest, ptail)
            } else {
                false
            }
        }
    }
}

/// Glob one segment: `*` matches any run within the segment, `?` one char.
fn seg_glob(pat: &str, name: &str) -> bool {
    let pat: Vec<char> = pat.chars().collect();
    let name: Vec<char> = name.chars().collect();
    let (mut pi, mut ni) = (0usize, 0usize);
    // (pat index just after the last `*`, name index that `*` is anchored at).
    let mut star: Option<(usize, usize)> = None;
    while ni < name.len() {
        if pi < pat.len() && (pat[pi] == '?' || pat[pi] == name[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < pat.len() && pat[pi] == '*' {
            star = Some((pi + 1, ni));
            pi += 1;
        } else if let Some((resume_pi, anchor_ni)) = star {
            // Backtrack: let the `*` swallow one more name char.
            pi = resume_pi;
            ni = anchor_ni + 1;
            star = Some((resume_pi, anchor_ni + 1));
        } else {
            return false;
        }
    }
    while pi < pat.len() && pat[pi] == '*' {
        pi += 1;
    }
    pi == pat.len()
}

struct ParsedConfig {
    version: u32,
    ignore: Vec<String>,
}

/// Hand-parse the constrained `mdfs_config.yaml` schema (§ [`Domain::from_config`]).
/// Deliberately dependency-free — the YAML crate is reserved for `policy`.
fn parse_config(yaml: &str) -> ParsedConfig {
    let mut version = 0u32;
    let mut ignore = Vec::new();
    let mut in_ignore = false;
    for raw in yaml.lines() {
        let line = strip_comment(raw);
        if line.trim().is_empty() {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim();
        if indent == 0 {
            in_ignore = false;
            if let Some(rest) = trimmed.strip_prefix("version:") {
                if let Ok(v) = unquote(rest.trim()).parse::<u32>() {
                    version = v;
                }
            } else if trimmed == "ignore:" {
                in_ignore = true;
            } else if let Some(rest) = trimmed.strip_prefix("ignore:") {
                // Block form only; a same-line value (e.g. `ignore: []`) opens
                // no block and contributes no rules.
                if rest.trim().is_empty() {
                    in_ignore = true;
                }
            }
            continue;
        }
        if in_ignore && let Some(item) = trimmed.strip_prefix('-') {
            let val = unquote(item.trim());
            if !val.is_empty() {
                ignore.push(val);
            }
        }
    }
    ParsedConfig { version, ignore }
}

/// Return the slice of `line` before an unquoted `#`.
fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut quote = 0u8;
    for (i, &c) in bytes.iter().enumerate() {
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
        } else if c == b'"' || c == b'\'' {
            quote = c;
        } else if c == b'#' {
            return &line[..i];
        }
    }
    line
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    let bytes = s.as_bytes();
    if s.len() >= 2
        && ((bytes[0] == b'"' && bytes[s.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[s.len() - 1] == b'\''))
    {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    // ---- §12.1 fixture bytes, verbatim from the contract §0.3 fixtures (docs/wire-contract.md) ----
    const PLAN_V0: &str = "---\ntitle: Plan\n---\n# Goals\n\nShip the contract.\n\n## Q3\n\nship by August\n\n## Q4\n\n- item one\n- see [[2026-07-18]]\n- blocked on [[roadmap]]\n";
    const RECEIPTS_V0: &str = "# Receipts \u{2014} 2026-07-18\n"; // em dash = 3-byte UTF-8
    const GH_README: &str = "# CI notes\n";

    // §12.2 merkle encoding — a test oracle only; the production merkle_root
    // lives in `model`.
    enum Node {
        File(Vec<u8>),
        Dir(BTreeMap<String, Node>),
    }

    fn insert(tree: &mut BTreeMap<String, Node>, rel: &str, data: &[u8]) {
        let parts: Vec<&str> = rel.split('/').collect();
        let mut cur = tree;
        for seg in &parts[..parts.len() - 1] {
            let entry = cur
                .entry((*seg).to_string())
                .or_insert_with(|| Node::Dir(BTreeMap::new()));
            cur = match entry {
                Node::Dir(m) => m,
                Node::File(_) => panic!("file/dir name clash at {seg}"),
            };
        }
        cur.insert(
            parts[parts.len() - 1].to_string(),
            Node::File(data.to_vec()),
        );
    }

    fn fold(dir: &BTreeMap<String, Node>) -> [u8; 32] {
        let mut children: Vec<(String, bool, [u8; 32])> = Vec::new();
        for (name, node) in dir {
            match node {
                Node::File(data) => {
                    children.push((name.clone(), false, *blake3::hash(data).as_bytes()));
                }
                Node::Dir(m) => {
                    if !m.is_empty() {
                        children.push((name.clone(), true, fold(m)));
                    }
                }
            }
        }
        children.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        let mut enc: Vec<u8> = Vec::new();
        for (name, is_dir, h) in &children {
            let nb = name.as_bytes();
            assert!(
                nb.len() < 128,
                "single-byte varint suffices for the fixture"
            );
            enc.push(u8::try_from(nb.len()).unwrap());
            enc.extend_from_slice(nb);
            enc.push(u8::from(*is_dir));
            enc.extend_from_slice(h);
        }
        *blake3::hash(&enc).as_bytes()
    }

    fn root_of(files: &BTreeMap<&str, Vec<u8>>) -> String {
        let mut tree = BTreeMap::new();
        for (rel, data) in files {
            insert(&mut tree, rel, data);
        }
        blake3::Hash::from_bytes(fold(&tree)).to_hex().to_string()
    }

    fn fixture() -> BTreeMap<&'static str, Vec<u8>> {
        BTreeMap::from([
            ("notes/plan.md", PLAN_V0.as_bytes().to_vec()),
            ("receipts/2026-07-18.md", RECEIPTS_V0.as_bytes().to_vec()),
            (".github/README.md", GH_README.as_bytes().to_vec()),
        ])
    }

    /// Gate 1: the §12.1 counterfactual pair. A wrong ignore implementation —
    /// one that fails to drop `.github/README.md` — computes `75a61c88…` where
    /// the correct domain computes `74162a12…`; it cannot pass both.
    #[test]
    fn counterfactual_pair_12_1() {
        // fixture byte-lengths pin the transcription (contract §0.3 sizes).
        assert_eq!(PLAN_V0.len(), 136);
        assert_eq!(RECEIPTS_V0.len(), 26);
        assert_eq!(GH_README.len(), 11);

        let domain = Domain::new();
        let all = fixture();
        let correct: BTreeMap<&str, Vec<u8>> = all
            .iter()
            .filter(|(rel, _)| domain.contains(Path::new(rel)))
            .map(|(k, v)| (*k, v.clone()))
            .collect();

        // correct: `.github/` dropped by the default dot-segment rule.
        assert_eq!(
            root_of(&correct),
            "74162a12ff0b323b52be37359cf5144fcc254ecf8801958402514a763829b5e9",
        );
        // wrong: hashing all three md files (no ignore) yields the other root.
        assert_eq!(
            root_of(&all),
            "75a61c883e372102cfe7d75e94992b9be65e33fbe95956897a4cf2ea45bb8f1b",
        );
        // the ignore decision is load-bearing: the two roots differ.
        assert_ne!(root_of(&correct), root_of(&all));
        // survivors are exactly the two non-dot md files.
        let mut survivors: Vec<&&str> = correct.keys().collect();
        survivors.sort();
        assert_eq!(survivors, vec![&"notes/plan.md", &"receipts/2026-07-18.md"]);
    }

    /// Gate 3: `mdfs_config.yaml` is non-md — structurally outside its own
    /// domain; not even a bespoke `!` re-include can pull it in (md-only floor
    /// sits above custom rules).
    #[test]
    fn config_file_is_outside_its_own_domain() {
        assert!(!Domain::new().contains(Path::new("mdfs_config.yaml")));
        assert!(!Domain::new().contains(Path::new(CONFIG_FILE_NAME)));
        let d = Domain::from_config("ignore:\n  - \"!mdfs_config.yaml\"\n");
        assert!(!d.contains(Path::new("mdfs_config.yaml")));
    }

    #[test]
    fn md_only_floor() {
        let d = Domain::new();
        assert!(d.contains(Path::new("notes/plan.md")));
        assert!(!d.contains(Path::new("notes/data.json")));
        assert!(!d.contains(Path::new("image.png")));
        assert!(!d.contains(Path::new("README")));
        assert!(d.contains(Path::new("Deep/Nested/file.MD"))); // ext case-insensitive
    }

    #[test]
    fn dot_segment_default_ignore() {
        let d = Domain::new();
        assert!(!d.contains(Path::new(".github/README.md")));
        assert!(!d.contains(Path::new(".obsidian/workspace.md")));
        assert!(!d.contains(Path::new(".trash/old.md")));
        assert!(!d.contains(Path::new("notes/.hidden.md"))); // dot segment at any depth
        assert!(d.contains(Path::new("notes/visible.md")));
    }

    /// §12.3: v1 `mdfs_config.yaml` adds `drafts/**` and bumps `version`.
    #[test]
    fn custom_ignore_drafts_12_3() {
        let d = Domain::from_config("version: 1\nignore:\n  - \"drafts/**\"\n");
        assert_eq!(d.version(), 1);
        assert!(!d.contains(Path::new("drafts/tmp.md")));
        assert!(!d.contains(Path::new("drafts/deep/nested.md")));
        assert!(d.contains(Path::new("notes/plan.md")));
    }

    #[test]
    fn custom_ignore_negation_reincludes() {
        let d = Domain::from_config("ignore:\n  - \"archive/**\"\n  - \"!archive/index.md\"\n");
        assert!(!d.contains(Path::new("archive/old.md")));
        assert!(d.contains(Path::new("archive/index.md"))); // re-included by `!`
    }

    /// §12.1 rule 3, trailing-slash law: a dir-only pattern binds directories.
    /// Regression for the live defect: the sessions root's `scratch*/` rule
    /// (declared for per-seat scratch DIRECTORIES) silently dropped the FILE
    /// `tasks/scratch-snapS-cleanup.md` out of the hash domain — the sql
    /// projection census undercounted the board by one card while the
    /// named-path doors served the file (card record-reproject-on-put,
    /// 2026-08-15). `git check-ignore` rules the same pair: exit 1 on the
    /// file, exit 0 on the directory's contents.
    #[test]
    fn dir_only_pattern_never_matches_a_bare_file() {
        let d = Domain::from_config("version: 2\nignore:\n  - \"scratch*/\"\n");
        // the incident's shape: a FILE whose basename fits the pattern body.
        assert_eq!(
            d.exclusion(Path::new(
                "year=2026/month=08/15-00-mrd-next-board/tasks/scratch-snapS-cleanup.md"
            )),
            None,
        );
        assert!(d.contains(Path::new("scratch-notes.md"))); // root-level file, same law
        // directory semantics stand: files beneath a matching dir segment.
        assert_eq!(
            d.exclusion(Path::new("results/scratch-r4/venv.md")),
            Some(ExclusionReason::CustomIgnore),
        );
        assert!(!d.contains(Path::new("scratch-top/deep/nested.md")));
    }

    /// The same law, anchored: `archive/deep/` names one directory, never the
    /// sibling file `archive/deep.md`.
    #[test]
    fn anchored_dir_only_pattern_binds_the_directory_only() {
        let d = Domain::from_config("ignore:\n  - \"archive/deep/\"\n");
        assert!(d.contains(Path::new("archive/deep.md")));
        assert!(!d.contains(Path::new("archive/deep/x.md")));
    }

    /// A `!` re-include with a trailing slash follows the same law through the
    /// same loop: it lifts the ignore for the directory's contents.
    #[test]
    fn negated_dir_only_reincludes_directory_contents() {
        let d = Domain::from_config("ignore:\n  - \"vendor/\"\n  - \"!vendor/keep/\"\n");
        assert!(!d.contains(Path::new("vendor/x.md")));
        assert!(d.contains(Path::new("vendor/keep/x.md")));
    }

    /// Dir-context matching is untouched by the trailing-slash law: the
    /// directory itself, and anything beneath it, still prunes.
    #[test]
    fn dir_only_pattern_still_prunes_the_directory() {
        let d = Domain::from_config("ignore:\n  - \"scratch*/\"\n");
        assert!(d.prunes_dir(Path::new("results/scratch-r4")));
        assert!(d.prunes_dir(Path::new("results/scratch-r4/deep")));
        assert!(!d.prunes_dir(Path::new("results")));
    }

    #[test]
    fn config_parsing_tolerates_comments_and_blanks() {
        let yaml = "# custom ignore\n\nversion: 2   # bumped\nignore:\n  - 'drafts/**'  # scratch\n  # a comment\n  - build/\n";
        let d = Domain::from_config(yaml);
        assert_eq!(d.version(), 2);
        assert!(!d.contains(Path::new("drafts/tmp.md")));
        assert!(!d.contains(Path::new("build/out.md")));
        assert!(!d.contains(Path::new("a/build/out.md"))); // unanchored dir at depth
        assert!(d.contains(Path::new("notes/plan.md")));
    }

    /// The journal's root-exclusion carve-out is retired: `meridian/journal.md`
    /// is an ordinary in-domain page, carved out by nothing.
    #[test]
    fn retired_journal_page_is_an_ordinary_domain_member() {
        let d = Domain::new();
        assert!(d.contains(Path::new("meridian/journal.md")));
        assert!(d.contains(Path::new("meridian/notes.md")));
    }

    #[test]
    fn default_version_is_zero() {
        assert_eq!(Domain::new().version(), 0);
        assert_eq!(Domain::from_config("ignore:\n  - \"x/**\"\n").version(), 0);
        assert_eq!(Domain::from_config("ignore: []\n").version(), 0);
    }
}
