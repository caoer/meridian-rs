//! Frontmatter scalar codec — the value-plane normalization law
//! (wire-contract § A.3, docs-first 2026-08-07; dogfood-season-1 findings
//! 1–2, R43).
//!
//! The fleet quotes frontmatter scalars by convention (`owner: "[[x]]"`), and
//! quoting was not part of the value contract at either seam: reads served
//! quote bytes as value bytes, and the unquoted emit turned `[[x]]` into a
//! nested flow sequence the substrate law then refused. This module is the
//! ONE owner of both directions:
//!
//! - [`decode`] — what a value plane serves for a stored scalar. Callers:
//!   the composed read's `props[].value` (`wire-serve::read`) and the script
//!   plane's `fm` composition (`mrd::script::wire_host`).
//! - [`encode`] — what the server composes after `{key}: ` for a caller
//!   value. Callers: `plan_fm_upsert` (the native `put{at:"upsert"}` door)
//!   and `policy::defs::yaml_safe_value` (the `set_property` lowering and
//!   its `check_write` candidate).
//!
//! The byte planes are NOT this module's business: `cat` grain bytes, spans,
//! `node_rev`/`prop_rev`, and the `props1` fingerprint canonical bytes all
//! stay on STORED bytes (`fingerprint::classify_property` — "wire form
//! verbatim"). Normalization is a value-plane law, never a re-hash.
//!
//! No YAML library (no-serde crate law): the decoder is the same closed
//! hand-parsed yaml.v3-outcome subset the def checker decodes
//! (`policy::defs::fm::classify_scalar`), so the value plane and the typed
//! plane cannot disagree about what a quoted scalar says. The def checker
//! cannot be called from here (policy depends on model), so the quote
//! grammar is mirrored; `policy`'s `defs_rebuild` suite pins the two against
//! drift.

/// Decode one stored scalar into its value: whitespace-trimmed, then a
/// well-formed single-quoted wrapper decodes `''` → `'` and a well-formed
/// double-quoted wrapper decodes the `\n` `\t` `\"` `\\` escapes (an unknown
/// escape keeps its backslash). A multi-line grain (block value) is not a
/// scalar and passes through unchanged.
#[must_use]
pub fn decode(raw: &str) -> String {
    if raw.contains('\n') {
        return raw.to_string();
    }
    let v = raw.trim();
    if let Some(q) = v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
        return q.replace("''", "'");
    }
    if let Some(q) = v.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        return unescape_double(q);
    }
    v.to_string()
}

/// Encode one caller value into the bytes the server composes after
/// `{key}: ` — the fleet-canonical form. A plain round-trippable scalar
/// emits verbatim (typed scalars, plain strings, one-level flow lists); a
/// value whose plain emit would misread or refuse double-quotes with
/// `\"`/`\\` escaping ([`needs_quote`]).
///
/// A `\n`/`\r`-bearing value passes verbatim: the single-line refusal
/// belongs to the `set_property` doors (`MultiLineValue`), and the raw
/// splice door keeps its own reparse/def gates — both unchanged by this law.
#[must_use]
pub fn encode(value: &str) -> String {
    if !needs_quote(value) {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Whether the plain emit of `value` would misread or refuse on this
/// engine's own readers (wire-contract § A.3, the encode classes):
///
/// - the empty string — bare `key:` is the null spelling, not the R4
///   explicit-empty the fleet writes (`""`);
/// - a whitespace-led or -trailed value — the read trim would eat it;
/// - a quote-led value — the decoder would strip bytes the caller sent;
/// - a `{`-led value — the map class;
/// - a `[`-led value whose one-level flow parse is the NESTED class
///   (`[[x]]`) — the substrate law would refuse the plain emit;
/// - a value carrying `": "` — a mapping in value position;
/// - a `#`-led value, or one carrying a ` #`/`\t#` comment introducer — the
///   typed plane's comment rule would truncate it.
///
/// A `\n`/`\r`-bearing value is not scalar-lawed input and reports `false`
/// (see [`encode`]).
#[must_use]
pub fn needs_quote(value: &str) -> bool {
    if value.contains(['\n', '\r']) {
        return false;
    }
    if value.is_empty() || value != value.trim() {
        return true;
    }
    match value.as_bytes()[0] {
        b'\'' | b'"' | b'{' | b'#' => return true,
        b'[' => return flow_seq_is_nested(value),
        _ => {}
    }
    value.contains(": ") || value.contains(" #") || value.contains("\t#")
}

/// Whether a `[`-led value is the NESTED class under the def checker's
/// one-level-deep law: unterminated flow, or any top-level item itself
/// `[`/`{`-led. The emit-side mirror of `policy::defs::fm`'s
/// `classify_flow_seq` Nested outcomes (items split quote- and
/// bracket-aware), pinned against it by `policy`'s `defs_rebuild` suite.
fn flow_seq_is_nested(value: &str) -> bool {
    let Some(inner) = value.strip_prefix('[').and_then(|s| s.strip_suffix(']')) else {
        return true; // unterminated flow — not the subset
    };
    if inner.trim().is_empty() {
        return false; // the empty list
    }
    let mut items: Vec<&str> = Vec::new();
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
                    items.push(inner[start..bi].trim());
                    start = bi + 1;
                }
                _ => {}
            },
        }
    }
    items.push(inner[start..].trim());
    items
        .iter()
        .any(|it| it.starts_with('[') || it.starts_with('{'))
}

/// The double-quoted escape subset, byte-for-byte the def checker's
/// (`policy::defs::fm::unescape_double`): `\n` `\t` `\"` `\\`; an unknown
/// escape keeps its backslash; a trailing backslash stays.
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
    use super::{decode, encode, needs_quote};

    /// The season-1 finding-1 table verbatim, at codec grain: every stored
    /// form the dogfood measured, decoded to its value.
    #[test]
    fn decode_serves_the_season_1_table() {
        let table = [
            ("\"\"", ""),
            ("\"3f9a1c07\"", "3f9a1c07"),
            ("\"[[1ed98864]]\"", "[[1ed98864]]"),
            ("doing", "doing"),
        ];
        for (stored, value) in table {
            assert_eq!(decode(stored), value, "stored {stored:?}");
        }
    }

    #[test]
    fn decode_handles_the_closed_quote_grammar() {
        assert_eq!(decode("'it''s'"), "it's", "single-quote escape");
        assert_eq!(decode("\"a\\\"b\\\\c\""), "a\"b\\c", "double-quote escapes");
        assert_eq!(decode("\"tab\\there\""), "tab\there", r"\t decodes");
        assert_eq!(decode("\"\\q\""), "\\q", "unknown escape keeps its backslash");
        assert_eq!(decode("'"), "'", "a lone quote is not a wrapper");
        assert_eq!(decode("\"unterminated"), "\"unterminated", "no pair, no strip");
        assert_eq!(decode("  padded  "), "padded", "whitespace-trimmed");
        assert_eq!(decode("[a, b]"), "[a, b]", "flow lists pass through");
        assert_eq!(
            decode("\nfoo: bar"),
            "\nfoo: bar",
            "a multi-line grain is not a scalar and passes through unchanged"
        );
    }

    /// The encode classes (wire-contract § A.3): what quotes, what stays.
    #[test]
    fn encode_emits_the_fleet_canonical_form() {
        // Quotes — the classes whose plain emit misreads or refuses.
        assert_eq!(encode(""), "\"\"", "explicit empty, R4 three-states");
        assert_eq!(encode("[[1ed98864]]"), "\"[[1ed98864]]\"", "nested-shaped");
        assert_eq!(encode("review: pending"), "\"review: pending\"", "colon-space");
        assert_eq!(encode("\"pre\""), "\"\\\"pre\\\"\"", "quote-led, escaped");
        assert_eq!(encode("'led"), "\"'led\"", "single-quote-led");
        assert_eq!(encode("{a: b}"), "\"{a: b}\"", "map-shaped");
        assert_eq!(encode(" edge"), "\" edge\"", "whitespace-led");
        assert_eq!(encode("edge "), "\"edge \"", "whitespace-trailed");
        assert_eq!(encode("#tag"), "\"#tag\"", "#-led");
        assert_eq!(encode("v #c"), "\"v #c\"", "comment introducer");
        assert_eq!(encode("[a, [b]]"), "\"[a, [b]]\"", "list-of-list");
        assert_eq!(encode("[unterminated"), "\"[unterminated\"", "bad flow");
        // Verbatim — plain round-trippable scalars keep today's bytes.
        for plain in ["doing", "3f9a1c07", "2026-08-07", "42", "true", "null", "~",
                      "it's", "[a, b]", "[]", "a:b", "path/to#frag"] {
            assert_eq!(encode(plain), plain, "plain scalar {plain:?}");
        }
        // Multi-line input is not scalar-lawed here (the doors refuse first).
        assert_eq!(encode("a\nb"), "a\nb");
        assert!(!needs_quote("a\nb"));
    }

    /// The § A.3 round-trip law: for every value the encoder accepts,
    /// decoding the emitted line yields the caller's string exactly.
    #[test]
    fn encode_then_decode_is_identity() {
        let values = [
            "", "doing", "3f9a1c07", "[[1ed98864]]", "[[b1892b5a]]",
            "review: pending", "\"pre\"", "'led", "it's", "a\"b\\c",
            " edge", "edge ", "#tag", "v #c", "{a: b}", "[a, [b]]",
            "2026-08-07", "42", "true", "[a, b]", "~", "null",
            "path with spaces", "emoji ✓ and — dashes", "\\back\\slash",
        ];
        for v in values {
            assert_eq!(
                decode(&encode(v)),
                v,
                "decode(encode({v:?})) is the caller's string"
            );
        }
    }
}
