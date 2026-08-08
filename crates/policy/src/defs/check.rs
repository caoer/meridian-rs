//! The def-driven validator — Go `internal/defs/check.go` ported: strata 1–3
//! (property legality, cross-field, section shape + entry grammar) + the
//! census. Verdicts are tri-state valid · legacy-useful · invalid; a section
//! absent from the def's declared shape scores legacy-useful, NEVER invalid
//! (decision 7's "# Todo untouched"). Finding ORDER is load-bearing (the
//! refusal names `findings[0]`): sorted meta keys → sorted def prop keys →
//! biconditional → requires guards → owner/claims → sections in document order.

use super::Finding;
use super::fm::{self, FmValue};
use super::go_fmt;
use super::load::{Def, SectionRule, compile_entry};
use super::shape;

pub const VERDICT_VALID: &str = "valid";
pub const VERDICT_LEGACY: &str = "legacy-useful";
pub const VERDICT_INVALID: &str = "invalid";

#[derive(Debug, Clone)]
pub struct SectionVerdict {
    pub title: String,
    pub verdict: &'static str,
    pub note: String,
}

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub verdict: String,
    pub sections: Vec<SectionVerdict>,
    pub findings: Vec<Finding>,
}

/// A flattened section view (Go `body.Section` surface this module needs):
/// content span (heading line excluded, subtree-INCLUSIVE end), the written
/// title with any trailing ` ^id` marker stripped, depth, and the 1-based
/// whole-file line number of the first content line.
#[derive(Debug, Clone)]
pub struct SecView {
    pub title: String,
    pub depth: u8,
    pub content_span: std::ops::Range<usize>,
    pub start_line: usize,
}

/// Flatten a document's sections in document order.
#[must_use]
pub fn doc_sections(doc: &model::Document) -> Vec<SecView> {
    let mut out = Vec::new();
    collect_sections(&doc.root, &doc.raw, &mut out);
    out
}

fn collect_sections(node: &model::Node, raw: &str, out: &mut Vec<SecView>) {
    if let model::NodeKind::Section {
        heading_text,
        level,
    } = &node.kind
    {
        let content_start = heading_terminator_end(raw.as_bytes(), &node.span);
        let content_span = content_start..node.span.end;
        out.push(SecView {
            title: strip_title_marker(heading_text),
            depth: *level,
            start_line: line_of(raw, content_start),
            content_span,
        });
    }
    for child in &node.children {
        collect_sections(child, raw, out);
    }
}

/// Byte offset just past the heading line's terminator (the content start).
fn heading_terminator_end(bytes: &[u8], span: &std::ops::Range<usize>) -> usize {
    let mut i = span.start;
    while i < span.end && bytes[i] != b'\n' {
        i += 1;
    }
    (i + 1).min(span.end)
}

/// 1-based physical line number of a byte offset.
fn line_of(raw: &str, offset: usize) -> usize {
    raw.as_bytes()[..offset.min(raw.len())]
        .iter()
        .filter(|b| **b == b'\n')
        .count()
        + 1
}

/// Go `Section.Title`: heading text with a trailing ` ^id` marker stripped.
fn strip_title_marker(text: &str) -> String {
    let trimmed = text.trim_end_matches([' ', '\t']);
    if let Some(pos) = trimmed.rfind(" ^") {
        let id = &trimmed[pos + 2..];
        if !id.is_empty()
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return trimmed[..pos].trim_end_matches([' ', '\t']).to_string();
        }
    }
    trimmed.to_string()
}

/// Check: validate `doc` against `def` (never mutates). `path` labels findings.
#[must_use]
pub fn check(doc: &model::Document, def: &Def, path: &str) -> Report {
    let mut rep = Report::default();

    let meta = if let Some(m) = record_meta(doc, path, &mut rep) {
        m
    } else {
        rep.verdict = VERDICT_INVALID.to_string();
        return rep;
    };

    check_properties(&meta, def, path, &mut rep); // stratum 1
    let terminal = check_cross_field(&meta, def, path, &mut rep); // stratum 2
    check_sections(doc, def, terminal, path, &mut rep); // stratum 3
    census(&meta, def, path, &mut rep);

    rep.verdict = VERDICT_VALID.to_string();
    if rep.findings.iter().any(|f| f.severity == "error") {
        rep.verdict = VERDICT_INVALID.to_string();
    } else if rep.sections.iter().any(|s| s.verdict == VERDICT_LEGACY) {
        rep.verdict = VERDICT_LEGACY.to_string();
    }
    rep
}

/// `ScanNested`: the stratum-1 nested-frontmatter errors alone — runs ALWAYS,
/// even when no def resolves.
#[must_use]
pub fn scan_nested(doc: &model::Document, path: &str) -> Vec<Finding> {
    let mut rep = Report::default();
    if let Some(meta) = record_meta(doc, path, &mut rep) {
        for (key, v) in &meta {
            if shape::is_nested(v) {
                rep.findings.push(finding(
                    "def/nested-frontmatter",
                    "error",
                    path,
                    format!("{key}: nested frontmatter is an ERROR always, from any writer (substrate law); flatten to a scalar or one-level list"),
                ));
            }
        }
    }
    rep.findings
}

fn record_meta(doc: &model::Document, path: &str, rep: &mut Report) -> Option<fm::FmMeta> {
    match fm::parse_meta(&doc.raw) {
        Ok(Some(m)) => Some(m),
        Ok(None) => {
            rep.findings.push(finding(
                "def/frontmatter",
                "error",
                path,
                "unreadable frontmatter: <nil>".to_string(),
            ));
            None
        }
        Err(e) => {
            rep.findings.push(finding(
                "def/frontmatter",
                "error",
                path,
                format!("unreadable frontmatter: {e}"),
            ));
            None
        }
    }
}

/// Stratum 1: flat YAML, known key, stable shape, required present.
fn check_properties(meta: &fm::FmMeta, def: &Def, path: &str, rep: &mut Report) {
    for (key, v) in meta {
        if shape::is_nested(v) {
            rep.findings.push(finding(
                "def/nested-frontmatter",
                "error",
                path,
                format!("{key}: nested frontmatter is an ERROR always, from any writer (substrate law); flatten to a scalar or one-level list"),
            ));
            continue;
        }
        let Some(spec) = def.props.get(key) else {
            rep.findings.push(finding(
                "def/unknown-key",
                "warn",
                path,
                format!(
                    "{}: not declared by the {} def (forward-compat: kept, reported)",
                    key, def.kind
                ),
            ));
            continue;
        };
        if fm::is_empty(Some(v)) {
            continue; // present-but-empty = absent; required-ness below
        }
        if !shape::check_shape(&spec.shape, v) {
            rep.findings.push(finding(
                "def/shape",
                "error",
                path,
                format!(
                    "{}: value {} does not satisfy shape {}",
                    key,
                    go_fmt::go_value(v),
                    spec.shape
                ),
            ));
        }
    }
    for (key, spec) in &def.props {
        let absent = fm::is_empty(meta.get(key));
        if spec.required && absent {
            rep.findings.push(finding(
                "def/required",
                "error",
                path,
                format!(
                    "{}: required by the {} def, missing or empty",
                    key, def.kind
                ),
            ));
        }
    }
}

/// Stratum 2: the terminal biconditional, requires(<key>) guards, owner/claims.
/// Returns whether the record sits at a terminal status.
fn check_cross_field(meta: &fm::FmMeta, def: &Def, path: &str, rep: &mut Report) -> bool {
    let mut terminal = false;
    if let Some(spec) = def.props.get("status")
        && !spec.terminal.is_empty()
    {
        let status = fm::string_field(meta, "status");
        terminal = spec.terminal.contains(&status);
        let closed = !fm::is_empty(meta.get("closed_at"));
        if terminal && !closed {
            rep.findings.push(finding(
                    "def/biconditional",
                    "error",
                    path,
                    format!("status {} ∈ terminal {} but closed_at is empty (status ∈ terminal ⟺ closed_at set)", go_fmt::go_quote(&status), go_fmt::go_slice(&spec.terminal)),
                ));
        }
        if !terminal && closed {
            rep.findings.push(finding(
                    "def/biconditional",
                    "error",
                    path,
                    format!("closed_at is set but status {} ∉ terminal {} (status ∈ terminal ⟺ closed_at set)", go_fmt::go_quote(&status), go_fmt::go_slice(&spec.terminal)),
                ));
        }
    }

    for (key, spec) in &def.props {
        for g in &spec.guard {
            if let Some((kind, arg)) = shape::parse_guard(g) {
                let key_set = !fm::is_empty(meta.get(key));
                let arg_empty = fm::is_empty(meta.get(arg.as_str()));
                if kind == "requires" && key_set && arg_empty {
                    rep.findings.push(finding(
                        "def/requires",
                        "error",
                        path,
                        format!("{key} is set but requires({arg}): {arg} is empty"),
                    ));
                }
                // actor-not-owner / append-only are write-time guards.
            }
        }
    }

    // owner/claims consistency (§4.2): flagged, never absorbed.
    if def.props.contains_key("owner") {
        let owner = fm::string_field(meta, "owner");
        if !owner.is_empty() {
            let last = match meta.get("claims") {
                Some(FmValue::List(items)) => items
                    .last()
                    .and_then(|v| match v {
                        FmValue::Str(s) => Some(s.clone()),
                        _ => None,
                    })
                    .unwrap_or_default(),
                _ => String::new(),
            };
            if !last.contains(&owner) {
                rep.findings.push(finding(
                    "def/owner-claims",
                    "warn",
                    path,
                    format!(
                        "owner {} does not match the last claims entry {} (hand-edited owner?)",
                        owner,
                        go_fmt::go_quote(&last)
                    ),
                ));
            }
        }
    }
    terminal
}

/// Stratum 3, walked over the section map in document order.
fn check_sections(doc: &model::Document, def: &Def, terminal: bool, path: &str, rep: &mut Report) {
    let toc = doc_sections(doc);
    let mut covered: Vec<std::ops::Range<usize>> = Vec::new();
    let in_covered = |covered: &[std::ops::Range<usize>], s: &SecView| {
        covered
            .iter()
            .any(|c| s.content_span.start >= c.start && s.content_span.end <= c.end)
    };

    let mut present: std::collections::HashSet<String> = std::collections::HashSet::new();
    for sec in &toc {
        present.insert(sec.title.clone());
        if in_covered(&covered, sec) {
            continue;
        }
        if let Some(rule) = def.section(&sec.title) {
            covered.push(sec.content_span.clone());
            let sv = score_declared(doc, &toc, sec, rule, terminal, path, rep);
            rep.sections.push(sv);
            continue;
        }
        if def
            .template
            .iter()
            .any(|p| p.matches(sec.depth, &sec.title))
        {
            rep.sections.push(SectionVerdict {
                title: sec.title.clone(),
                verdict: VERDICT_VALID,
                note: "template scaffold".to_string(),
            });
            continue;
        }
        rep.sections.push(SectionVerdict {
            title: sec.title.clone(),
            verdict: VERDICT_LEGACY,
            note: "not in the def's declared shape — kept untouched, never invalid".to_string(),
        });
        rep.findings.push(finding_at(
            "def/legacy-section",
            "warn",
            path,
            sec.start_line as i64 - 1,
            format!("# {}: section absent from the {} def's shape → legacy-useful (decision 7: left untouched)", sec.title, def.kind),
        ));
    }

    for p in &def.template {
        if p.literal && !present.contains(&p.text) {
            rep.findings.push(finding(
                "def/section-missing",
                "warn",
                path,
                format!("# {}: template scaffold heading missing (repairable: `md def fix` inserts it empty)", p.text),
            ));
        }
    }
}

/// One declared section: emptiness at terminal, then the entry grammar.
fn score_declared(
    doc: &model::Document,
    toc: &[SecView],
    sec: &SecView,
    rule: &SectionRule,
    terminal: bool,
    path: &str,
    rep: &mut Report,
) -> SectionVerdict {
    let mut sv = SectionVerdict {
        title: sec.title.clone(),
        verdict: VERDICT_VALID,
        note: String::new(),
    };

    if rule.required_before_terminal
        && terminal
        && doc.raw[sec.content_span.clone()].trim().is_empty()
    {
        rep.findings.push(finding_at(
            "def/required-before-terminal",
            "error",
            path,
            sec.start_line as i64 - 1,
            format!(
                "# {}: must be non-empty before a terminal status (guard: section-non-empty)",
                sec.title
            ),
        ));
        sv.verdict = VERDICT_INVALID;
        sv.note = "empty at terminal status".to_string();
        return sv;
    }
    if rule.entry.is_empty() {
        return sv;
    }

    let grammar = compile_entry(&entry_text(&rule.entry));
    let mut legacy = 0usize;
    let mut miss = 0usize;
    let depth = heading_grammar_depth(&rule.entry);
    if depth > 0 {
        for sub in toc {
            if sub.content_span.start < sec.content_span.start
                || sub.content_span.end > sec.content_span.end
                || sub.depth != depth
            {
                continue;
            }
            if !rule.legacy_mark.is_empty() && sub.title.contains(&rule.legacy_mark) {
                legacy += 1;
                rep.findings.push(finding_at(
                    "def/legacy-entry",
                    "warn",
                    path,
                    sub.start_line as i64 - 1,
                    format!("# {}: entry {} carries {} — kept for content, excluded from strict harvest", sec.title, go_fmt::go_quote(&sub.title), rule.legacy_mark),
                ));
                continue;
            }
            if !grammar.matches(&sub.title) {
                miss += 1;
                rep.findings.push(finding_at(
                    "def/entry-grammar",
                    "warn",
                    path,
                    sub.start_line as i64 - 1,
                    format!(
                        "# {}: entry {} does not parse as {} → would be tagged {}",
                        sec.title,
                        go_fmt::go_quote(&sub.title),
                        go_fmt::go_quote(&rule.entry),
                        legacy_mark_or(rule)
                    ),
                ));
            }
        }
    } else {
        for (text, num) in section_lines(&doc.raw, sec) {
            let text = text.trim_end_matches([' ', '\t']);
            if !text.starts_with("- ") {
                continue; // continuation prose, comments, blanks
            }
            if !rule.sync.is_empty() && text.ends_with("<!-- manual -->") {
                continue;
            }
            if !rule.legacy_mark.is_empty() && text.contains(&rule.legacy_mark) {
                legacy += 1;
                rep.findings.push(finding_at(
                    "def/legacy-entry",
                    "warn",
                    path,
                    num as i64,
                    format!("# {}: entry {} carries {} — kept for content, excluded from strict harvest", sec.title, go_fmt::go_quote(text), rule.legacy_mark),
                ));
                continue;
            }
            if !grammar.matches(text) {
                miss += 1;
                rep.findings.push(finding_at(
                    "def/entry-grammar",
                    "warn",
                    path,
                    num as i64,
                    format!(
                        "# {}: entry {} does not parse as {} → would be tagged {}",
                        sec.title,
                        go_fmt::go_quote(text),
                        go_fmt::go_quote(&rule.entry),
                        legacy_mark_or(rule)
                    ),
                ));
            }
        }
    }
    if legacy + miss > 0 {
        sv.verdict = VERDICT_LEGACY;
        sv.note = format!("{legacy} legacy / {miss} unparsed entries");
    }
    sv
}

/// Off-suggest census: information for vocabulary accretion — never rejects.
fn census(meta: &fm::FmMeta, def: &Def, path: &str, rep: &mut Report) {
    for (key, spec) in &def.props {
        if spec.suggest.is_empty() {
            continue;
        }
        let v = fm::string_field(meta, key);
        if v.is_empty() || spec.suggest.contains(&v) || spec.terminal.contains(&v) {
            continue;
        }
        rep.findings.push(finding(
            "def/census",
            "info",
            path,
            format!(
                "{}: {} is off-suggest {} (census: reported, never rejected)",
                key,
                go_fmt::go_quote(&v),
                go_fmt::go_slice(&spec.suggest)
            ),
        ));
    }
}

// --- helpers ---

fn entry_text(entry: &str) -> String {
    let d = heading_grammar_depth(entry);
    if d > 0 {
        return entry[d as usize..].trim().to_string();
    }
    entry.to_string()
}

fn heading_grammar_depth(entry: &str) -> u8 {
    let d = entry.bytes().take_while(|b| *b == b'#').count();
    if d > 0 && entry.as_bytes().get(d) == Some(&b' ') {
        return d as u8;
    }
    0
}

fn legacy_mark_or(rule: &SectionRule) -> &str {
    if rule.legacy_mark.is_empty() {
        return "#legacy";
    }
    &rule.legacy_mark
}

/// A section's content lines with whole-file line numbers, fence-aware.
fn section_lines<'a>(raw: &'a str, sec: &SecView) -> Vec<(&'a str, usize)> {
    let mut out = Vec::new();
    let mut in_fence = false;
    let mut num = sec.start_line;
    for ln in raw[sec.content_span.clone()].split('\n') {
        let trimmed = ln.trim();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            num += 1;
            continue;
        }
        if !in_fence {
            out.push((ln, num));
        }
        num += 1;
    }
    out
}

pub(super) fn finding(rule: &str, severity: &'static str, path: &str, msg: String) -> Finding {
    Finding {
        rule_id: rule.to_string(),
        severity,
        file_path: path.to_string(),
        line: 0,
        message: msg,
    }
}

pub(super) fn finding_at(
    rule: &str,
    severity: &'static str,
    path: &str,
    line: i64,
    msg: String,
) -> Finding {
    Finding {
        rule_id: rule.to_string(),
        severity,
        file_path: path.to_string(),
        line,
        message: msg,
    }
}
