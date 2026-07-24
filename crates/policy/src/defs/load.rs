//! Def-file loading — Go `internal/defs/load.go` ported: the `^properties`
//! yaml block via the block index, the `## section: <Name>` rules via the
//! section table, the `^template` block via the block index. Decoding is
//! STRICT (unknown/nested params are `INVALID_PARAMS`) and a malformed def fails
//! CLOSED: the caller validates nothing against a half-read schema.

use serde::Deserialize;

use super::fm;
use super::shape;

/// One property declaration of a `^properties` block (schema-v2 §1.2), strict.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PropSpec {
    #[serde(default)]
    pub shape: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub default: Option<serde_yaml::Value>,
    #[serde(default)]
    pub suggest: Vec<String>,
    #[serde(default)]
    pub terminal: Vec<String>,
    #[serde(default)]
    pub stamp: String,
    #[serde(default)]
    pub guard: Vec<String>,
}

/// One `## section: <Name>` rule block (schema-v2 §1.3), strict.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SectionRule {
    #[serde(default)]
    pub write: String,
    #[serde(default)]
    pub entry: String,
    #[serde(default)]
    pub sync: String,
    #[serde(default)]
    pub merge: String,
    #[serde(default)]
    pub collision: String,
    #[serde(default, rename = "required-before-terminal")]
    pub required_before_terminal: bool,
    #[serde(default, rename = "promote-at")]
    pub promote_at: String,
    #[serde(default, rename = "legacy-mark")]
    pub legacy_mark: String,
    #[serde(default, rename = "on-violation")]
    pub on_violation: String,
}

/// One heading line of the `^template` block, compiled for recognition.
#[derive(Debug, Clone)]
pub struct TemplatePattern {
    pub depth: u8,
    pub text: String,
    pub literal: bool,
    matcher: PatternMatch,
}

impl TemplatePattern {
    pub fn matches(&self, depth: u8, title: &str) -> bool {
        depth == self.depth && self.matcher.matches(title)
    }
}

/// One loaded (or cascade-merged) definition.
#[derive(Debug, Clone, Default)]
pub struct Def {
    pub kind: String,
    pub preset: String,
    pub version: i64,
    /// Sorted by key (findings iterate sorted prop keys — `BTreeMap` by law).
    pub props: std::collections::BTreeMap<String, PropSpec>,
    /// Declared section rules, in def order.
    pub sections: Vec<(String, SectionRule)>,
    pub template: Vec<TemplatePattern>,
    pub sources: Vec<String>,
}

impl Def {
    #[must_use]
    pub fn section(&self, name: &str) -> Option<&SectionRule> {
        self.sections
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, r)| r)
    }
}

/// A def that failed to load (fail closed). The message is the Go error string
/// BYTE-EXACT (it lands verbatim inside the def/malformed refusal).
#[derive(Debug, Clone, PartialEq)]
pub struct DefError(pub String);

impl std::fmt::Display for DefError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// `ParseDef`: one definition from a pre-built document; `path` labels errors.
/// The caller supplies the parse (policy's charter takes `&Document`s — the
/// `syntax` edge stays out of the production dependency graph).
pub fn parse_def(doc: &model::Document, path: &str) -> Result<Def, DefError> {
    let meta = match fm::parse_meta(&doc.raw) {
        Ok(Some(m)) => m,
        Ok(None) => {
            return Err(DefError(format!(
                "def {path}: unreadable frontmatter: <nil>"
            )));
        }
        Err(e) => {
            return Err(DefError(format!("def {path}: unreadable frontmatter: {e}")));
        }
    };
    if fm::string_field(&meta, "type") != "def" {
        return Err(DefError(format!(
            "def {}: type is {}, want \"def\"",
            path,
            super::go_fmt::go_quote(&fm::string_field(&meta, "type"))
        )));
    }
    let mut def = Def {
        kind: fm::string_field(&meta, "defines"),
        preset: fm::string_field(&meta, "preset"),
        sources: vec![path.to_string()],
        ..Def::default()
    };
    if def.kind.is_empty() {
        return Err(DefError(format!("def {path}: missing `defines:` kind")));
    }
    if let Some(fm::FmValue::Int(v)) = meta.get("version") {
        def.version = *v;
    }

    // ^properties — strict decode; every param top-level, never nested.
    // Go's block index binds a standalone `^id` line to the block it FOLLOWS
    // (the fence); the engine model's anchor grain is the line itself, so this
    // narrow Go convention is resolved here, over the def bytes.
    let prop_src = anchored_fence(&doc.raw, "properties").ok_or_else(|| {
        DefError(format!(
            "def {path}: no ^properties block: E_NO_MATCH: no block ^properties — block ids inside fences are not addressable; check the id"
        ))
    })?;
    let props: std::collections::BTreeMap<String, PropSpec> = strict_yaml(&prop_src)
        .map_err(|e| DefError(format!("def {path}: INVALID_PARAMS in ^properties: {e}")))?;
    def.props = props;
    for (key, spec) in &def.props {
        if !shape::valid_shape(&spec.shape) {
            return Err(DefError(format!(
                "def {}: INVALID_PARAMS: {}: unknown shape {} (closed set: line text iso int bool ref list(line) list(ref) list(iso))",
                path,
                key,
                super::go_fmt::go_quote(&spec.shape)
            )));
        }
        if !shape::valid_stamp(&spec.stamp) {
            return Err(DefError(format!(
                "def {}: INVALID_PARAMS: {}: unknown stamp {} (closed set: create touch close)",
                path,
                key,
                super::go_fmt::go_quote(&spec.stamp)
            )));
        }
        for g in &spec.guard {
            if shape::parse_guard(g).is_none() {
                return Err(DefError(format!(
                    "def {}: INVALID_PARAMS: {}: unknown guard {} (closed set: actor-not-owner section-non-empty(<S>) requires(<key>) append-only)",
                    path,
                    key,
                    super::go_fmt::go_quote(g)
                )));
            }
        }
    }

    // `## section: <Name>` rule blocks, in def order, via the section table.
    for sec in super::check::doc_sections(doc) {
        let Some(name) = sec.title.strip_prefix("section: ") else {
            continue;
        };
        let mut rule = SectionRule::default();
        if let Some(block) = first_fence(&doc.raw[sec.content_span.clone()]) {
            rule = strict_yaml(&block).map_err(|e| {
                DefError(format!(
                    "def {}: INVALID_PARAMS in section {} rule: {}",
                    path,
                    super::go_fmt::go_quote(name),
                    e
                ))
            })?;
        }
        def.sections.push((name.to_string(), rule));
    }

    // ^template — heading lines become recognition patterns.
    if let Some(tmpl) = anchored_fence(&doc.raw, "template") {
        def.template = template_patterns(&tmpl);
    }
    Ok(def)
}

/// Strict YAML decode (unknown fields rejected — serde `deny_unknown_fields`).
fn strict_yaml<T: serde::de::DeserializeOwned + Default>(src: &str) -> Result<T, String> {
    if src.trim().is_empty() {
        return Ok(T::default());
    }
    serde_yaml::from_str(src).map_err(|e| e.to_string())
}

/// The inner lines of the fenced block a standalone `^id` line FOLLOWS —
/// Go's block-index convention for the def machine blocks (`^properties`,
/// `^template`): the anchor line binds to the block immediately above it.
fn anchored_fence(raw: &str, id: &str) -> Option<String> {
    let marker = format!("^{id}");
    let lines: Vec<&str> = raw.split('\n').collect();
    let anchor_at = lines
        .iter()
        .position(|l| l.trim_end_matches('\r').trim() == marker)?;
    // The line above the anchor must close a fence; scan back to its opener.
    let close = anchor_at.checked_sub(1)?;
    if !lines[close].trim_start().starts_with("```") {
        return None;
    }
    let open = lines[..close]
        .iter()
        .rposition(|l| l.trim_start().starts_with("```"))?;
    Some(lines[open + 1..close].join("\n"))
}

/// Go `firstFence`: the inner bytes of the first fenced block in a span.
fn first_fence(b: &str) -> Option<String> {
    let lines: Vec<&str> = b.split('\n').collect();
    let mut start: Option<usize> = None;
    for (i, ln) in lines.iter().enumerate() {
        if ln.trim_start().starts_with("```") {
            match start {
                None => start = Some(i + 1),
                Some(s) => return Some(lines[s..i].join("\n")),
            }
        }
    }
    None
}

/// Template heading lines outside fences → patterns.
fn template_patterns(tmpl: &str) -> Vec<TemplatePattern> {
    let mut out = Vec::new();
    let mut in_fence = false;
    for ln in tmpl.split('\n') {
        let trimmed = ln.trim();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let Some((depth, text)) = heading_line(ln) else {
            continue;
        };
        out.push(TemplatePattern {
            depth,
            literal: !text.contains("{{"),
            matcher: PatternMatch::compile(&text, "{{", "}}"),
            text,
        });
    }
    out
}

/// Go `reHeadingLine`: `^(#{1,6})\s+(.*?)\s*$`.
fn heading_line(ln: &str) -> Option<(u8, String)> {
    let hashes = ln.bytes().take_while(|b| *b == b'#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &ln[hashes..];
    if !rest.starts_with([' ', '\t']) {
        return None;
    }
    Some((hashes as u8, rest.trim().to_string()))
}

/// Boolean matcher for the anchored placeholder patterns Go compiles as
/// `^`+`QuoteMeta`+`(.+?)`+`$`. Go only ever calls `MatchString`, and lazy-vs-
/// greedy is irrelevant to the boolean result, so literal-segment scanning is
/// behavior-identical without a regex dependency.
///
/// Encoding: `lits[0] W lits[1] W … W lits[k]` where W = one wildcard
/// consuming ≥1 char. A pattern starting/ending with a placeholder has an
/// empty edge literal.
#[derive(Debug, Clone)]
pub(super) struct PatternMatch {
    lits: Vec<String>,
}

impl PatternMatch {
    pub(super) fn compile(text: &str, open: &str, close: &str) -> Self {
        let mut lits = Vec::new();
        let mut rest = text;
        loop {
            if let Some((s, e)) = find_placeholder(rest, open, close) {
                lits.push(rest[..s].to_string());
                rest = &rest[e..];
            } else {
                lits.push(rest.to_string());
                break;
            }
        }
        PatternMatch { lits }
    }

    pub(super) fn matches(&self, s: &str) -> bool {
        if !s.starts_with(self.lits[0].as_str()) {
            return false;
        }
        let mut pos = self.lits[0].len();
        for (i, lit) in self.lits.iter().enumerate().skip(1) {
            // A wildcard (≥1 char) precedes every literal after the first.
            let from = match s[pos..].chars().next() {
                Some(c) => pos + c.len_utf8(),
                None => return false,
            };
            if i == self.lits.len() - 1 {
                if lit.is_empty() {
                    return true; // trailing placeholder already consumed ≥1
                }
                return s.len() >= from + lit.len() && s.ends_with(lit.as_str());
            }
            match s[from..].find(lit.as_str()) {
                Some(off) => pos = from + off + lit.len(),
                None => return false,
            }
        }
        pos == s.len() // no placeholders: exact equality
    }
}

fn find_placeholder(s: &str, open: &str, close: &str) -> Option<(usize, usize)> {
    let start = s.find(open)?;
    let end_rel = s[start + open.len()..].find(close)?;
    let inner = &s[start + open.len()..start + open.len() + end_rel];
    if inner.is_empty() || inner.contains(open) {
        return None;
    }
    Some((start, start + open.len() + end_rel + close.len()))
}

/// Entry grammars use single-brace `{var}` placeholders.
pub(super) fn compile_entry(text: &str) -> PatternMatch {
    PatternMatch::compile(text, "{", "}")
}
