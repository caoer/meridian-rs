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

/// The custom-ignore config file. Not markdown ⇒ never in its own domain.
pub const CONFIG_FILE_NAME: &str = "mdfs_config.yaml";

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

    /// Read `mdfs_config.yaml` from the workspace root, or the default domain
    /// when the file is absent.
    ///
    /// # Errors
    /// I/O failure reading an existing config file. An absent file is not an
    /// error — it yields [`Domain::new`].
    pub fn load(root: &WorkspaceRoot) -> io::Result<Domain> {
        let path = root.0.join(CONFIG_FILE_NAME);
        match std::fs::read_to_string(&path) {
            Ok(text) => Ok(Domain::from_config(&text)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Domain::new()),
            Err(e) => Err(e),
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
        // 3. custom ignore — gitignore last-match-wins.
        let mut ignored = false;
        for rule in &self.rules {
            if rule.matches(&segments) {
                ignored = !rule.negate;
            }
        }
        !ignored
    }
}

fn is_markdown(p: &Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case("md"))
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

    // ---- §12.1 fixture bytes, verbatim from `wire-contract-v2-verify.py` §0.3 ----
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

    #[test]
    fn default_version_is_zero() {
        assert_eq!(Domain::new().version(), 0);
        assert_eq!(Domain::from_config("ignore:\n  - \"x/**\"\n").version(), 0);
        assert_eq!(Domain::from_config("ignore: []\n").version(), 0);
    }
}
