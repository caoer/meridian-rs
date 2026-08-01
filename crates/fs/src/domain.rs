//! The §12 hash domain — which markdown files' bytes enter the merkle root.
//!
//! ZT verbatim (contract §12.1): *"`mdfs_config.yaml` define the custom ignore
//! list. mdfs only work for md file. it does not hash any other file."* The
//! domain is a three-layer filter, applied in order:
//!
//! 1. **md-only floor** — only `*.md` files hash. `mdfs_config.yaml` is not md,
//!    so it is structurally outside its own domain.
//! 2. **default ignore, one rule** — any path with a dot-prefixed segment is
//!    ignored (`.github/README.md`, `.obsidian/**`, `.trash/**`). Zero
//!    enumeration; `.git`/lock/`.obsidian` artifacts are dissolved by
//!    construction and can never self-invalidate a guard. This floor is
//!    structural: custom `!` re-includes cannot lift it.
//! 3. **custom ignore** — `mdfs_config.yaml` carries a gitignore-style list
//!    (patterns, `!` re-includes) layered over the default (§12.3's `drafts/**`
//!    is such a rule). Last matching rule wins.
//!
//! The filter gates HASHING, not `load`: an ignored md file stays fully
//! addressable — `toc`/`cat`/`splice` reach it by explicit path — its bytes
//! simply never enter the root (hash ⊂ addressable, E-E8). See [`Domain`] for
//! the predicate and [`crate::hash_domain`] for the walk that consumes it.

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
/// Markdown because the workspace convention is that configuration a human
/// maintains is a page they can read: the ignore list rides the FRONTMATTER
/// (frontmatter filters, body reads) and the body carries the rationale for
/// each entry, which a bare YAML file has nowhere to put. It joins the reserved
/// path family already here — [`RESERVED_JOURNAL_PATH`], [`ARMED_RULES_PATH`],
/// [`ATTESTED_MARKER_PATH`].
///
/// # This file is inside its own hash domain, deliberately
/// Unlike [`CONFIG_FILE_NAME`] (non-md, structurally outside), a `.md` config
/// is hashed like any other page. That is the correct behaviour, not a wrinkle
/// to paper over: the file that DEFINES the attested surface should itself be
/// attested, so a change to the ignore list moves the root and is visible as a
/// fact rather than as a silent reshaping of what gets checked. It pairs with
/// the `version` field, which exists so an ignore-list change is a deliberate,
/// root-advancing act. Bootstrap is not circular — read the config, compute the
/// domain, then hash the domain including the config.
pub const DOMAIN_CONFIG_PATH: &str = "meridian/domain.md";

/// The ONE reserved receipt-journal page (d2 §2.1 A3/A9; node-rev-merkle-spec
/// §10 open-question 3). The receipt engine appends one row per guarded write
/// here; it is in-vault markdown, git-tracked, and **root-EXCLUDED** — the
/// workspace tree merkle never covers it. That exclusion is the whole point:
/// without it every guarded write would move the very root it just guarded (a
/// receipt records `root_after`, but writing that receipt would change the
/// root again — "a root that self-invalidates on every splice is useless as a
/// commit guard"). Excluding the journal lets a row carry BOTH `root_before`
/// and `root_after`, which is what makes the chain-continuity detector
/// (`receipt::journal::check_chain`) possible.
///
/// A NON-dot path on purpose: the dot-segment default-ignore would exclude a
/// `.`-prefixed page incidentally, but the journal must be git-TRACKED (the
/// outer git witness is one of its two integrity sides), and its exclusion is
/// a NAMED law, not a side effect of the dot rule.
///
/// # Integrity residuals — both named, stated not hidden (d2 §2.1)
/// 1. **Pre-push offline rewrite** — a full offline rewrite of the journal
///    before the first push is undetectable from inside the engine. This is
///    git's own trust floor; cryptographic closure is a deferred door.
/// 2. **Root-preserving online forged-row insertion** — inserting a forged row
///    whose `root_before`/`root_after` already chain is NOT caught by
///    `check_chain` (the chain stays continuous). Detection rests on the
///    receipt-engine-only write restriction (an ordinary `^put`/splice at this
///    path refuses — `wire-serve`) plus the git witness, never on chain
///    continuity alone.
///
///    **WIDENED, and stated because a silently-grown residual is worse than a
///    named one (F1's staged interval):** such a row now buys MORE than a
///    continuous-looking chain. `check::staged_trace` dates the git INDEX's tree
///    against **any** row in this journal — a legitimately staged INTERMEDIATE
///    governed state matches an earlier receipt, and refusing it was a measured
///    false red on the commonest path there is (`git add`, then any further
///    governed write). So a chain-continuous forged row whose `root_after` names
///    a tree the attacker also stages will VOUCH FOR THAT STAGED TREE at the
///    pre-commit fence.
///
///    **The capability is unchanged and so are its defences** — the attacker must
///    still be able to write this path out of band, which the receipt-engine-only
///    restriction refuses and the git witness records; what changed is what one
///    such row is worth. Two things bound it: the staged journal must be a true
///    PREFIX of this one (a spliced row is not), and the pin plane is assessed
///    over the staged bytes independently, so a forged tree still has to satisfy
///    every lock it carries. Carded as an s4 rider rather than left here.
pub const RESERVED_JOURNAL_PATH: &str = "meridian/journal.md";

/// The attested armed-set artifact (registration ruling § 4) — the ONE page the
/// door reads to learn a workspace's armed set, one row per armed id. It is
/// ordinary in-tree markdown and STAYS in the hash domain: the attestation IS the
/// page, so its rev matters.
///
/// Mirrors `policy::armed::ARMED_RULES_PATH` — `policy` is I/O-free and `fs`
/// knows nothing of rules, so neither crate can name the other's constant.
/// `crates/testsuite/tests/reserved_paths.rs` holds the two spellings together.
pub const ARMED_RULES_PATH: &str = "meridian/armed-rules.md";

/// The once-armed sentinel (U4.2 read contract). Its PRESENCE — not its bytes —
/// records that a workspace has EVER been armed. The gate needs this to tell a
/// never-armed workspace (no artifact, no marker ⇒ a bit-for-bit no-op) from a
/// once-armed one whose artifact went missing (⇒ fail CLOSED): the artifact alone
/// cannot make that distinction, because deleting it is exactly the attack the
/// marker defeats.
///
/// A NON-markdown path on purpose: the md-only hash-domain floor keeps it out
/// of the merkle root by construction (no carve-out needed, unlike the
/// journal), so writing the marker never perturbs the very root a write
/// guards. Arming (U4.4) creates it on the first arm and never removes it;
/// U4.3's integrity floor refuses its deletion/rename at the door.
pub const ATTESTED_MARKER_PATH: &str = "meridian/attested";

/// The §12 hash-domain filter: the md-only floor + dot-segment default ignore
/// (both fixed and structural) plus the custom ignore rules and the domain
/// `version` parsed from `mdfs_config.yaml`.
///
/// `version` rides the domain config so an ignore-list change can advance the
/// merkle prefix (`b3:` → `b3a:`, §12.3); the prefix *token* is minted by
/// `model::merkle_root` (M3-MERKLE), which reads [`Domain::version`].
#[derive(Debug, Clone, Default)]
pub struct Domain {
    version: u32,
    rules: Vec<Rule>,
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
    /// the workspace's single YAML decision is reserved for `policy`,
    /// P6-COMPILE): an optional top-level `version:` integer and an `ignore:`
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
    /// **Both present is an error, not a precedence rule.** Two live ignore
    /// lists are two answers to "what is attested here", and silently picking
    /// one means the file a reader is looking at may not be the file in force —
    /// the ambiguity is reported rather than resolved.
    pub fn load(root: &WorkspaceRoot) -> io::Result<Domain> {
        let md = read_optional(&root.0.join(DOMAIN_CONFIG_PATH))?;
        let yaml = read_optional(&root.0.join(CONFIG_FILE_NAME))?;
        match (md, yaml) {
            (Some(_), Some(_)) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "two domain configs are present: {DOMAIN_CONFIG_PATH} and {CONFIG_FILE_NAME}. \
                     They may declare different ignore lists, so which files are attested would \
                     depend on a precedence rule no reader of either file can see. \
                     Remedy: keep {DOMAIN_CONFIG_PATH} and delete {CONFIG_FILE_NAME}."
                ),
            )),
            (Some(text), None) => Ok(Domain::from_markdown(&text)),
            (None, Some(text)) => Ok(Domain::from_config(&text)),
            (None, None) => Ok(Domain::new()),
        }
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
    #[must_use]
    pub fn contains(&self, rel: &Path) -> bool {
        // 1. md-only floor.
        if !is_markdown(rel) {
            return false;
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
        if segments.iter().any(|s| s.starts_with('.')) {
            return false;
        }
        // 2b. the reserved receipt journal — root-EXCLUDED by NAMED law (d2
        //     §2.1 A3/A9), structural like the dot rule (a `!` re-include
        //     cannot lift it). Excluded so guarded writes do not self-
        //     invalidate the root and so a journal row may carry root_after.
        if is_reserved_journal(rel) {
            return false;
        }
        // 3. custom ignore — gitignore last-match-wins.
        let mut ignored = false;
        for rule in &self.rules {
            if rule.matches(&segments) {
                ignored = !rule.negate;
            }
        }
        !ignored
    }

    /// May a traversal skip `rel_dir` and everything beneath it WITHOUT
    /// changing the hash domain?
    ///
    /// This is the difference between filtering and pruning, and it is the
    /// whole cost story: [`contains`](Self::contains) answers per FILE, so a
    /// walk that filters afterwards has already paid the `stat` for every
    /// entry it then discards. Pruning declines to descend at all.
    ///
    /// Sound, not merely fast — it answers `true` only when BOTH hold:
    ///
    /// 1. the directory itself is ignored by last-match-wins, so every path
    ///    beneath it inherits the ignore; and
    /// 2. no `!` rule could re-include anything beneath it. gitignore
    ///    semantics let `!archive/index.md` survive `archive/**`, and a
    ///    pruned directory would silently drop that file out of the domain —
    ///    a WRONG ROOT, not a slow one. When re-inclusion cannot be ruled
    ///    out the answer is `false` and the walk pays for the descent.
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

/// The reserved-path family: every page that is engine SUBSTRATE rather than
/// content, and whose parent directories must therefore stay walkable whatever the
/// ignore list says.
///
/// Public so a cross-crate test can assert the family's MEMBERSHIP, not just each
/// member's spelling. A path that outlives its subject is the "renamed remnant"
/// failure — it would keep the door refusing writes to what is now an ordinary
/// file, and keep the walk carving a hole in the hash domain for nothing.
pub const RESERVED_PATHS: &[&str] = &[
    RESERVED_JOURNAL_PATH,
    ARMED_RULES_PATH,
    ATTESTED_MARKER_PATH,
];

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

/// Is `rel` the ONE reserved receipt-journal page? The single source of truth
/// for that identity, shared by the hash-domain exclusion (above) and the
/// write-choke-point restriction (`wire-serve`) so the two can never drift.
/// Matches on normalized path segments (a leading `./` or empty segments do
/// not defeat it), so a splice cannot dodge the restriction with a
/// non-canonical spelling of the reserved path.
#[must_use]
pub fn is_reserved_journal(rel: &Path) -> bool {
    normalized(rel) == RESERVED_JOURNAL_PATH
}

/// Is `rel` the attested armed-rules artifact ([`ARMED_RULES_PATH`])? Normalized
/// like [`is_reserved_journal`], so a non-canonical spelling
/// (`./meridian/armed-rules.md`) cannot dodge the INDEX-integrity floor at the
/// write door — deleting the artifact must never read as disarming.
#[must_use]
pub fn is_armed_rules(rel: &Path) -> bool {
    normalized(rel) == ARMED_RULES_PATH
}

/// Is `rel` the once-armed marker ([`ATTESTED_MARKER_PATH`])? Normalized like
/// [`is_reserved_journal`]. The U4.3 INDEX-integrity floor refuses its
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

/// One parsed gitignore-style rule. Matching operates on path *segments*.
#[derive(Debug, Clone)]
struct Rule {
    /// A `!`-prefixed re-include: when it matches it clears the ignore.
    negate: bool,
    /// Rooted at the workspace (leading `/` or an internal `/`); otherwise the
    /// pattern matches at any depth (basename semantics).
    anchored: bool,
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
            segs,
        })
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
    /// Deliberately conservative — it answers the question that makes pruning
    /// safe, so every uncertain case answers `true` (walk it) rather than
    /// risk dropping a re-included file out of the hash domain. Two cases
    /// admit no cheap proof and are therefore assumed reachable:
    /// an unanchored rule (basename semantics — it matches at any depth), and
    /// any rule containing `**` (which absorbs arbitrary segments).
    /// Otherwise the rule names a fixed-depth path, and it can only reach
    /// under `dir` when `dir` is a proper prefix of that path.
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

    // ---- §12.1 fixture bytes, verbatim from the contract §0.3 fixtures (docs/wire-contract-v2.md) ----
    const PLAN_V0: &str = "---\ntitle: Plan\n---\n# Goals\n\nShip the contract.\n\n## Q3\n\nship by August\n\n## Q4\n\n- item one\n- see [[2026-07-18]]\n- blocked on [[roadmap]]\n";
    const RECEIPTS_V0: &str = "# Receipts \u{2014} 2026-07-18\n"; // em dash = 3-byte UTF-8
    const GH_README: &str = "# CI notes\n";

    // §12.2 merkle encoding, ported from `root_of` — a TEST ORACLE only. The
    // production merkle_root lands in M3-MERKLE (`model`); this exists solely to
    // give the §12.1 counterfactual pair real, contract-matching hex roots.
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

    /// d2 §2.1 A3/A9: the reserved receipt journal is root-EXCLUDED by named
    /// law — never in the hash domain, and a custom `!` re-include cannot lift
    /// it (structural, above custom rules). Addressability is a separate
    /// concern (`fs::load` reaches it by path); this only gates HASHING.
    #[test]
    fn reserved_journal_is_root_excluded() {
        let d = Domain::new();
        assert!(!d.contains(Path::new(RESERVED_JOURNAL_PATH)));
        assert!(!d.contains(Path::new("meridian/journal.md")));
        // a sibling page under the same dir stays IN the domain — only the one
        // reserved page is carved out.
        assert!(d.contains(Path::new("meridian/notes.md")));
        // a `!` re-include cannot pull the journal back into the root.
        let re = Domain::from_config("ignore:\n  - \"!meridian/journal.md\"\n");
        assert!(!re.contains(Path::new(RESERVED_JOURNAL_PATH)));
    }

    #[test]
    fn default_version_is_zero() {
        assert_eq!(Domain::new().version(), 0);
        assert_eq!(Domain::from_config("ignore:\n  - \"x/**\"\n").version(), 0);
        assert_eq!(Domain::from_config("ignore: []\n").version(), 0);
    }
}
