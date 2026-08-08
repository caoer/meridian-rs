//! The frontmatter scalar law (wire-contract § A.6) — the read half.
//!
//! A frontmatter value is the DECODED scalar: quote bytes stripped,
//! double-quoted escapes unescaped. The value-serving surfaces (the composed
//! read's `props` plane, the script plane's `card.fm`) serve this form; the
//! byte plane (`cat` on an `fm_key` node) stays raw, because "what you read
//! is exactly what is hashed" is the rev law (§4.2).
//!
//! The decode is the string-level twin of the policy fm parser's
//! `classify_scalar` string handling — the two must agree on every quoted
//! spelling, so the typed parse and the served value never diverge.

/// Decode one frontmatter scalar at the read seam: strip YAML quoting and
/// unescape double-quoted escapes. Unquoted values pass verbatim; a value
/// whose leading quote has no matching close is not a quoted scalar and
/// passes verbatim (the policy parser classifies it as a plain string too).
#[must_use]
pub fn decode(v: &str) -> String {
    if let Some(q) = v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
        return q.replace("''", "'");
    }
    if let Some(q) = v.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        return unescape_double(q);
    }
    v.to_string()
}

/// Unescape a double-quoted YAML scalar body: the four escapes the policy fm
/// parser knows (`\n`, `\t`, `\"`, `\\`); any other `\x` keeps both bytes.
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
            // A trailing backslash and an escaped backslash both keep one.
            Some('\\') | None => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::decode;

    #[test]
    fn unquoted_values_pass_verbatim() {
        assert_eq!(decode("doing"), "doing");
        assert_eq!(decode("3f9a1c07"), "3f9a1c07");
        assert_eq!(decode("[a, b]"), "[a, b]");
        assert_eq!(decode("2026-08-07"), "2026-08-07");
    }

    #[test]
    fn double_quoted_values_strip_quotes_and_unescape() {
        assert_eq!(decode("\"3f9a1c07\""), "3f9a1c07");
        assert_eq!(decode("\"[[1ed98864]]\""), "[[1ed98864]]");
        assert_eq!(decode("\"\""), "");
        assert_eq!(decode("\"a\\nb\""), "a\nb");
        assert_eq!(decode("\"a\\tb\""), "a\tb");
        assert_eq!(decode("\"say \\\"hi\\\"\""), "say \"hi\"");
        assert_eq!(decode("\"a\\\\b\""), "a\\b");
        assert_eq!(decode("\"a\\xb\""), "a\\xb", "unknown escapes keep both bytes");
    }

    #[test]
    fn single_quoted_values_strip_quotes_and_unescape_doubled_quotes() {
        assert_eq!(decode("'it''s mine'"), "it's mine");
        assert_eq!(decode("'[[id]]'"), "[[id]]");
        assert_eq!(decode("''"), "");
    }

    #[test]
    fn unterminated_quotes_pass_verbatim() {
        assert_eq!(decode("\"3f9a1c07"), "\"3f9a1c07");
        assert_eq!(decode("'3f9a1c07"), "'3f9a1c07");
    }
}
