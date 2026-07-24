//! Go formatting quirks the finding/refusal strings depend on. The parity gate
//! compares these bytes verbatim — do not "improve" them.

/// Go `strconv.Quote`: double-quoted, Go escape rules. The finding sites quote
/// ASCII-ish addresses (hpaths, selectors, entry lines); this covers the Go
/// escape table for printable + the C escapes, and falls back to \u escapes the
/// way strconv does for non-printables.
///
/// SHARED (U8b): the splice plan-lowering's host-face refusal strings (`%q`
/// spellings) mint through THIS quoter — neutral data, no wire types.
#[must_use]
pub fn go_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\x07' => out.push_str("\\a"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            '\x0b' => out.push_str("\\v"),
            c if is_go_printable(c) => out.push(c),
            c if (c as u32) < 0x10000 => {
                use std::fmt::Write as _;
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => {
                use std::fmt::Write as _;
                let _ = write!(out, "\\U{:08x}", c as u32);
            }
        }
    }
    out.push('"');
    out
}

/// Go treats a rune as printable per unicode.IsPrint. For the address strings
/// this module quotes, letters/digits/punctuation/symbols/spaces print; the
/// separator categories and controls do not. This mirrors `IsPrint` closely
/// enough for the corpus surface (ASCII + the exotic-whitespace corpus chars),
/// and the fixtures pin the exact bytes.
fn is_go_printable(c: char) -> bool {
    if c == ' ' {
        return true;
    }
    if c.is_control() || c.is_whitespace() {
        // NBSP/NEL/ZWSP-class: Go prints Zs? unicode.IsPrint includes only
        // ASCII space among spaces — every other separator escapes.
        return false;
    }
    true
}

/// Go `fmt.Sprintf("%v", []string{...})` → `[a b]` (space-joined, brackets).
pub(super) fn go_slice(items: &[String]) -> String {
    format!("[{}]", items.join(" "))
}

/// Go `fmt.Sprintf("%v", v)` for the decoded frontmatter scalar in shape
/// findings (`value %v does not satisfy shape %s`).
pub(super) fn go_value(v: &crate::defs::fm::FmValue) -> String {
    use crate::defs::fm::FmValue;
    match v {
        FmValue::Str(s) => s.clone(),
        FmValue::Int(i) => i.to_string(),
        FmValue::Bool(b) => b.to_string(),
        FmValue::Timestamp(raw) => go_time_render(raw),
        // Go %v of a yaml float64: raw decimal (corpus-bounded; fixture-pinned).
        FmValue::Float(raw) => raw.clone(),
        FmValue::List(items) => {
            let rendered: Vec<String> = items.iter().map(go_value).collect();
            go_slice(&rendered)
        }
        FmValue::Nested => "<nested>".to_string(),
        FmValue::Null => "<nil>".to_string(),
    }
}

/// Go renders a time.Time with `%v` as `2026-07-16 10:00:00 +0000 UTC`; the
/// yaml.v3 decoder parses the naive form as UTC. Reproduce that rendering from
/// the raw scalar (fixtures pin the exact bytes).
fn go_time_render(raw: &str) -> String {
    // RFC3339 or naive "2006-01-02T15:04:05[.frac]" / "2006-01-02T15:04".
    let (date, rest) = match raw.split_once('T') {
        Some((d, r)) => (d, r),
        None => return raw.to_string(),
    };
    let (time_part, zone) = split_zone(rest);
    let time_norm = if time_part.matches(':').count() == 1 {
        format!("{time_part}:00")
    } else {
        time_part.to_string()
    };
    match zone {
        // yaml.v3 decodes the naive form and Z as UTC; Go %v renders that as
        // "<date> <time> +0000 UTC". Offset forms never reach a finding in the
        // corpus — pass raw so an unexpected case fails the fixture loudly
        // instead of shipping invented formatting.
        None | Some("Z") => format!("{date} {time_norm} +0000 UTC"),
        Some(_) => raw.to_string(),
    }
}

fn split_zone(rest: &str) -> (&str, Option<&str>) {
    if let Some(stripped) = rest.strip_suffix('Z') {
        return (stripped, Some("Z"));
    }
    for (i, c) in rest.char_indices().skip(1) {
        if (c == '+' || c == '-') && rest[..i].contains(':') {
            return (&rest[..i], Some(&rest[i..]));
        }
    }
    (rest, None)
}
