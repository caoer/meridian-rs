//! Frontmatter typed values with Go yaml.v3 OUTCOME parity — the closed
//! surface `CheckShape`/`IsNested`/the biconditional actually consult.
//!
//! Go decodes the whole frontmatter block with yaml.v3 into `map[string]any`
//! (meridian `internal/frontmatter.ParseBytes`), then iterates SORTED keys.
//! The load-bearing typing facts (each pinned by a fixture):
//!
//! - quoted scalars are strings, always
//! - unquoted: null forms → nil · true/false forms → bool · integers → int ·
//!   decimal floats → float · yaml-timestamp shapes (incl. DATE-ONLY) →
//!   time.Time · everything else → string
//! - flow/block sequences → []any (items typed by the same scalar rules)
//! - flow/block mappings anywhere, or a list inside a list → the NESTED class
//!   (substrate-law error)
//! - duplicate top-level keys → yaml.v3 decode ERROR → "unreadable
//!   frontmatter" (def/frontmatter finding), not last-wins
//!
//! The parser is a YAML SUBSET tuned to those outcomes, affordable because the
//! substrate law bounds depth at one level; it never guesses beyond what a
//! fixture pins — an unsupported construct classifies as Nested (loud), never
//! silently as string.

use std::collections::BTreeMap;

/// A decoded frontmatter value, Go `map[string]any` outcome classes.
#[derive(Debug, Clone, PartialEq)]
pub enum FmValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(String),
    /// Unquoted yaml-timestamp scalar; raw text kept for rendering/iso checks.
    Timestamp(String),
    Str(String),
    List(Vec<FmValue>),
    /// A mapping anywhere, or a list inside a list — the substrate-law class.
    Nested,
}

/// Parsed frontmatter: sorted key → typed value (Go iterates sorted keys).
pub type FmMeta = BTreeMap<String, FmValue>;

/// Extract + decode the frontmatter block of `raw` (whole-file bytes).
/// `Ok(None)` = no frontmatter block. `Err` = unreadable (the def/frontmatter
/// finding path): bad delimiters or a duplicate top-level key.
pub fn parse_meta(raw: &str) -> Result<Option<FmMeta>, String> {
    let block = match fm_block(raw) {
        Some(b) => b,
        None => return Ok(None),
    };
    let mut meta = FmMeta::new();
    let lines: Vec<&str> = block.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            i += 1;
            continue;
        }
        if line.starts_with([' ', '\t']) {
            // continuation of a value we didn't consume → structure this subset
            // doesn't model; classify the PREVIOUS key as nested already handled,
            // otherwise it is stray indentation yaml would reject.
            return Err(format!("unexpected indented line: {line:?}"));
        }
        let Some(colon) = find_key_colon(line) else {
            return Err(format!("not a mapping line: {line:?}"));
        };
        let key = line[..colon].trim().to_string();
        let inline = line[colon + 1..].trim();
        // Gather the indented continuation block (block sequences / nested maps).
        let mut cont: Vec<&str> = Vec::new();
        let mut j = i + 1;
        while j < lines.len() {
            let l = lines[j];
            if l.trim().is_empty() {
                cont.push(l);
                j += 1;
                continue;
            }
            // Continuations: indented lines, or column-0 "- " sequence items
            // under a value-less key (both yaml.v3-legal block sequences).
            if l.starts_with([' ', '\t']) || (inline.is_empty() && l.starts_with("- ")) {
                cont.push(l);
                j += 1;
                continue;
            }
            break;
        }
        // Trim trailing blank continuation lines (they belong to the next key).
        while cont.last().is_some_and(|l| l.trim().is_empty()) {
            cont.pop();
        }
        let value = if !inline.is_empty() && strip_comment(inline).is_empty() && cont.is_empty() {
            FmValue::Null
        } else if !strip_comment(inline).is_empty() {
            if cont.is_empty() {
                classify_scalar_or_flow(strip_comment(inline))
            } else {
                // inline value + continuation = multi-line scalar or nested — nested class
                FmValue::Nested
            }
        } else if cont.is_empty() {
            FmValue::Null
        } else {
            classify_block(&cont)
        };
        if meta.insert(key.clone(), value).is_some() {
            return Err(format!("mapping key {key:?} already defined"));
        }
        i = j;
    }
    Ok(Some(meta))
}

/// The frontmatter block between the leading `---` line and its closing `---`.
fn fm_block(raw: &str) -> Option<&str> {
    let rest = raw.strip_prefix("---")?;
    let rest = rest.strip_prefix('\r').unwrap_or(rest);
    let rest = rest.strip_prefix('\n')?;
    let mut offset = 0usize;
    for line in rest.split_inclusive('\n') {
        let t = line.trim_end_matches(['\r', '\n']);
        if t == "---" {
            return Some(&rest[..offset]);
        }
        offset += line.len();
    }
    None
}

/// Locate the key/value colon on a top-level mapping line (quoted keys are not
/// in the corpus surface; the first colon is the yaml.v3 outcome for plain keys).
fn find_key_colon(line: &str) -> Option<usize> {
    line.find(':')
}

/// Strip a trailing ` # comment` from an unquoted inline scalar (yaml comment
/// rule: '#' preceded by whitespace). Quoted scalars keep their bytes.
fn strip_comment(v: &str) -> &str {
    if v.starts_with('\'') || v.starts_with('"') {
        return v;
    }
    let bytes = v.as_bytes();
    for i in 1..bytes.len() {
        if bytes[i] == b'#' && (bytes[i - 1] == b' ' || bytes[i - 1] == b'\t') {
            return v[..i].trim_end();
        }
    }
    v
}

/// Classify an inline value: flow collection or scalar.
fn classify_scalar_or_flow(v: &str) -> FmValue {
    if v.starts_with('{') {
        return FmValue::Nested;
    }
    if v.starts_with('[') {
        return classify_flow_seq(v);
    }
    classify_scalar(v)
}

/// A flow sequence `[a, 'b', [c]]` — items split quote- and bracket-aware
/// (one-level-deep law: an inner `[` or `{` is already the nested class).
fn classify_flow_seq(v: &str) -> FmValue {
    let inner = match v.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        Some(s) => s,
        None => return FmValue::Nested, // unterminated flow — not our subset
    };
    if inner.trim().is_empty() {
        return FmValue::List(Vec::new());
    }
    let mut raw_items: Vec<&str> = Vec::new();
    let mut depth = 0i32;
    let mut in_quote: Option<char> = None;
    let mut start = 0usize;
    for (bi, c) in inner.char_indices() {
        match in_quote {
            Some(q) => {
                if c == q {
                    in_quote = None;
                }
            }
            None => match c {
                '\'' | '"' => in_quote = Some(c),
                '[' | '{' => depth += 1,
                ']' | '}' => depth -= 1,
                ',' if depth == 0 => {
                    raw_items.push(inner[start..bi].trim());
                    start = bi + 1;
                }
                _ => {}
            },
        }
    }
    raw_items.push(inner[start..].trim());
    let mut typed = Vec::with_capacity(raw_items.len());
    for it in raw_items {
        if it.starts_with('[') || it.starts_with('{') {
            return FmValue::Nested;
        }
        typed.push(classify_scalar(it));
    }
    FmValue::List(typed)
}

/// A block continuation: `- item` sequence lines, or nested mapping lines.
fn classify_block(cont: &[&str]) -> FmValue {
    let mut items = Vec::new();
    for l in cont {
        let t = l.trim();
        if t.is_empty() {
            continue;
        }
        let Some(item) = t
            .strip_prefix("- ")
            .or(t.strip_prefix("-").filter(|s| s.is_empty()))
        else {
            return FmValue::Nested; // an indented mapping line → nested class
        };
        let item = strip_comment(item.trim());
        if item.starts_with('[') || item.starts_with('{') || item.contains(": ") {
            return FmValue::Nested;
        }
        items.push(classify_scalar(item));
    }
    FmValue::List(items)
}

/// yaml.v3 core-schema scalar outcomes for one unwrapped scalar token.
fn classify_scalar(v: &str) -> FmValue {
    if let Some(q) = v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
        return FmValue::Str(q.replace("''", "'"));
    }
    if let Some(q) = v.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        return FmValue::Str(unescape_double(q));
    }
    match v {
        "" | "~" | "null" | "Null" | "NULL" => return FmValue::Null,
        "true" | "True" | "TRUE" => return FmValue::Bool(true),
        "false" | "False" | "FALSE" => return FmValue::Bool(false),
        _ => {}
    }
    if is_int(v)
        && let Ok(i) = v.parse::<i64>()
    {
        return FmValue::Int(i);
    }
    if is_float(v) {
        return FmValue::Float(v.to_string());
    }
    if is_yaml_timestamp(v) {
        return FmValue::Timestamp(v.to_string());
    }
    FmValue::Str(v.to_string())
}

fn unescape_double(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn is_int(v: &str) -> bool {
    let s = v.strip_prefix(['+', '-']).unwrap_or(v);
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

fn is_float(v: &str) -> bool {
    let s = v.strip_prefix(['+', '-']).unwrap_or(v);
    let Some((a, b)) = s.split_once('.') else {
        return false;
    };
    !a.is_empty()
        && !b.is_empty()
        && a.bytes().all(|c| c.is_ascii_digit())
        && b.bytes().all(|c| c.is_ascii_digit())
}

/// yaml.v3 timestamp shapes: RFC3339, the spaced form, T-form with optional
/// seconds/fraction/zone, and DATE-ONLY (all unquoted).
fn is_yaml_timestamp(v: &str) -> bool {
    let b = v.as_bytes();
    let date_ok = |d: &[u8]| {
        d.len() == 10
            && d[..4].iter().all(u8::is_ascii_digit)
            && d[4] == b'-'
            && d[5..7].iter().all(u8::is_ascii_digit)
            && d[7] == b'-'
            && d[8..10].iter().all(u8::is_ascii_digit)
    };
    if b.len() == 10 {
        return date_ok(b);
    }
    if b.len() < 16 || !date_ok(&b[..10]) || (b[10] != b'T' && b[10] != b' ') {
        return false;
    }
    let rest = &v[11..];
    let time_part = rest
        .trim_end_matches('Z')
        .split(['+'])
        .next()
        .unwrap_or(rest);
    let hm_ok = |t: &str| {
        let seg: Vec<&str> = t.split(':').collect();
        (2..=3).contains(&seg.len())
            && seg.iter().enumerate().all(|(i, s)| {
                let s = if i == 2 {
                    s.split('.').next().unwrap_or(s)
                } else {
                    s
                };
                s.len() == 2 && s.bytes().all(|c| c.is_ascii_digit())
            })
    };
    hm_ok(time_part)
}

/// Go `Doc.StringField`: the value if it decoded as a string, else "".
#[must_use]
pub fn string_field(meta: &FmMeta, key: &str) -> String {
    match meta.get(key) {
        Some(FmValue::Str(s)) => s.clone(),
        _ => String::new(),
    }
}
