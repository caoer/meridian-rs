//! The frontmatter quoting layer — wire-contract § A.6.1, and its only owner.
//!
//! Every property value plane on the wire is a plane of STRINGS. What the
//! corpus stores is YAML source, and the two differ by exactly one layer:
//! quoting. This module is that layer and nothing else — it decodes a quoted
//! scalar and never infers a type, so `true` and `2026-08-07` come out of it
//! verbatim for a caller that classifies (`policy::defs::fm`) and unchanged for
//! a caller that just wants the text (the read seams).
//!
//! **No YAML library, and none needed** (`model` is the no-serde crate): §A.6.1
//! is a closed rule over one line of bytes. The strictness is the safety — a
//! MALFORMED quoted scalar decodes to itself, because a reader that guesses at
//! `'a' and 'b'` invents a value the file does not carry.

/// One frontmatter scalar, split by the only distinction this layer makes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scalar<'a> {
    /// A well-formed quoted scalar: quotes removed, escapes resolved. Quoted is
    /// a STRING in every schema — a caller that classifies types must not
    /// re-classify this arm.
    Quoted(String),
    /// Everything else, verbatim after the whitespace trim: plain scalars, flow
    /// collections, and malformed quoting.
    Plain(&'a str),
}

impl Scalar<'_> {
    /// The decoded text, allocating only for the quoted arm.
    #[must_use]
    pub fn into_text(self) -> String {
        match self {
            Scalar::Quoted(s) => s,
            Scalar::Plain(p) => p.to_string(),
        }
    }
}

/// Decode one frontmatter scalar (§ A.6.1). The input is a key line's colon
/// remainder; the whitespace trim happens here so every seam trims once and the
/// same way.
#[must_use]
pub fn decode(value: &str) -> Scalar<'_> {
    let v = value.trim();
    if let Some(inner) = single_quoted_body(v) {
        return Scalar::Quoted(inner.replace("''", "'"));
    }
    if let Some(inner) = double_quoted_body(v) {
        return Scalar::Quoted(unescape_double(inner));
    }
    Scalar::Plain(v)
}

/// The value a reader sees — [`decode`] flattened to its text. The read seams'
/// door: `props[].value`, the script face's `fm`, the run plane's bindings.
#[must_use]
pub fn text(value: &str) -> String {
    decode(value).into_text()
}

/// Encode `value` as a double-quoted scalar — the fleet-canonical spelling, and
/// the inverse of [`decode`]'s double-quoted arm (§ A.6.3). Only `\` and `"`
/// need escaping: a value carrying a newline never reaches here (the write
/// seams refuse it, never sanitize it).
#[must_use]
pub fn double_quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The body of a well-formed `'…'` scalar: interior `'` only ever doubled.
fn single_quoted_body(v: &str) -> Option<&str> {
    // A lone `'` cannot strip twice: the suffix strip runs on what the prefix
    // strip left, so the one-byte case is refused by construction.
    let inner = v.strip_prefix('\'')?.strip_suffix('\'')?;
    let bytes = inner.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\'' {
            // A lone interior quote closes the scalar early: malformed.
            if i + 1 >= bytes.len() || bytes[i + 1] != b'\'' {
                return None;
            }
            i += 2;
            continue;
        }
        i += 1;
    }
    Some(inner)
}

/// The body of a well-formed `"…"` scalar: no unescaped interior `"`, and the
/// closing quote itself not escaped.
fn double_quoted_body(v: &str) -> Option<&str> {
    let inner = v.strip_prefix('"')?.strip_suffix('"')?;
    let bytes = inner.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2, // the escaped byte cannot close the scalar
            b'"' => return None,
            _ => i += 1,
        }
    }
    // A trailing `\` consumed the closing quote we stripped: `"a\"` is unterminated.
    if i > bytes.len() {
        return None;
    }
    Some(inner)
}

/// The double-quoted escape set this layer resolves. An unknown escape stays
/// verbatim (backslash included) rather than being invented away.
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
            Some('r') => out.push('\r'),
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

#[cfg(test)]
mod tests {
    use super::{Scalar, decode, double_quote, text};

    /// The fleet-canonical corpus values — the dogfood season-1 read table,
    /// which measured every one of these arriving WITH its quote bytes.
    #[test]
    fn fleet_canonical_values_decode() {
        assert_eq!(text(r#""""#), "");
        assert_eq!(text(r#""3f9a1c07""#), "3f9a1c07");
        assert_eq!(text(r#""[[1ed98864]]""#), "[[1ed98864]]");
        assert_eq!(text("doing"), "doing");
    }

    /// Quoting is the whole layer: a quoted typed scalar is a string, and a
    /// plain one comes back verbatim for the caller that classifies.
    #[test]
    fn decode_never_infers_a_type() {
        assert_eq!(decode("true"), Scalar::Plain("true"));
        assert_eq!(decode("7"), Scalar::Plain("7"));
        assert_eq!(decode(r#""true""#), Scalar::Quoted("true".into()));
    }

    /// A flow collection is not a quoted scalar and is served whole.
    #[test]
    fn flow_collections_are_plain() {
        assert_eq!(decode("[a, b]"), Scalar::Plain("[a, b]"));
        assert_eq!(decode("[[a, b]]"), Scalar::Plain("[[a, b]]"));
    }

    /// Malformed quoting decodes to ITSELF. Guessing invents a value the file
    /// does not carry.
    #[test]
    fn malformed_quoting_stays_verbatim() {
        assert_eq!(decode("'a' and 'b'"), Scalar::Plain("'a' and 'b'"));
        assert_eq!(decode(r#""a" and "b""#), Scalar::Plain(r#""a" and "b""#));
        assert_eq!(decode("'unterminated"), Scalar::Plain("'unterminated"));
        assert_eq!(decode(r#""a\""#), Scalar::Plain(r#""a\""#));
        assert_eq!(decode("'"), Scalar::Plain("'"));
        assert_eq!(decode("\""), Scalar::Plain("\""));
    }

    /// The escape sets, both quoting styles.
    #[test]
    fn escapes_resolve() {
        assert_eq!(text("'it''s'"), "it's");
        assert_eq!(text(r#""a\\b""#), r"a\b");
        assert_eq!(text(r#""say \"hi\"""#), r#"say "hi""#);
        assert_eq!(text(r#""a\tb""#), "a\tb");
        // An escape outside the set is not invented away.
        assert_eq!(text(r#""a\qb""#), r"a\qb");
    }

    /// Empty and null-ish spellings: the layer removes quotes, it does not
    /// classify null — that is § A.6.2's stored-form question.
    #[test]
    fn empty_and_null_spellings() {
        assert_eq!(decode(""), Scalar::Plain(""));
        assert_eq!(decode("~"), Scalar::Plain("~"));
        assert_eq!(decode("''"), Scalar::Quoted(String::new()));
        assert_eq!(decode(r#""~""#), Scalar::Quoted("~".into()));
    }

    /// The trim is this layer's, once, so no seam does its own.
    #[test]
    fn value_is_trimmed_once_here() {
        assert_eq!(text("  spaced  "), "spaced");
        assert_eq!(text(r#"  "spaced"  "#), "spaced");
    }

    /// § A.6.3's inverse property, stated as a round trip: whatever
    /// [`double_quote`] emits, [`decode`] gives back unchanged.
    #[test]
    fn double_quote_round_trips_through_decode() {
        for v in [
            "",
            "[[1ed98864]]",
            "{a: b}",
            r#""already quoted""#,
            r"C:\path\to",
            "null",
            "a: b",
            "# not a comment",
            "it's",
        ] {
            assert_eq!(text(&double_quote(v)), v, "round trip for {v:?}");
        }
    }
}
