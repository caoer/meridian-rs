//! Go-EXACT text semantics for the host-face addressing facts (M1 U2).
//!
//! The read cutover (plan A-C1) demands byte-identity with the LIVE Go
//! `readsidecar.go` output, so these primitives replicate the Go stdlib
//! semantics the host face uses — never idiomatic-Rust near-equivalents:
//!
//! - [`is_go_space`] = Go `unicode.IsSpace` (the Unicode `White_Space`
//!   property), pinned as an EXPLICIT table so neither stdlib's Unicode
//!   revision can skew the set underneath us.
//! - [`fields_count`] = `len(strings.Fields(s))` — maximal runs of
//!   non-`White_Space` runes.
//! - [`sanitize_heading`] = `sanitizeHeadingHost` (`readsidecar.go:350`,
//!   itself "mirrors meridian internal/body map.go VERBATIM"): `TrimSpace`,
//!   then `/` → `-`, then ASCII space → `-`, empty → `"untitled"`.
//! - [`DeweyCounter`] = the `buildTocEntries` ordinal stack
//!   (`readsidecar.go:222`), including its malformed-hierarchy behavior
//!   (level jumps H1→H3, resets H6→H1) — reproduced, not repaired.
//!
//! The authoritative parity target is the U0 captured golden corpus (A-S2);
//! these unit tests pin the enumerated divergence classes the review named
//! (NBSP/NEL are spaces, ZWSP is not, level jumps, empty headings).

/// Go `unicode.IsSpace`: the Unicode `White_Space` property, as an explicit
/// pinned table (stable since Unicode 6.3 — U+180E left the property then,
/// and Go's and Rust's current tables both agree with this set).
#[must_use]
pub const fn is_go_space(c: char) -> bool {
    matches!(
        c,
        '\u{0009}'..='\u{000D}' // TAB LF VT FF CR
        | '\u{0020}' // SPACE
        | '\u{0085}' // NEL
        | '\u{00A0}' // NBSP
        | '\u{1680}' // OGHAM SPACE MARK
        | '\u{2000}'..='\u{200A}' // EN QUAD .. HAIR SPACE
        | '\u{2028}' // LINE SEPARATOR
        | '\u{2029}' // PARAGRAPH SEPARATOR
        | '\u{202F}' // NARROW NO-BREAK SPACE
        | '\u{205F}' // MEDIUM MATHEMATICAL SPACE
        | '\u{3000}' // IDEOGRAPHIC SPACE
    )
}

/// `len(strings.Fields(s))`: the count of maximal runs of non-`White_Space`
/// runes. The host word count (`wordCountBytes`, `readsidecar.go:370`) is
/// exactly this over the section's content-span bytes.
#[must_use]
pub fn fields_count(s: &str) -> usize {
    let mut count = 0;
    let mut in_field = false;
    for c in s.chars() {
        if is_go_space(c) {
            in_field = false;
        } else if !in_field {
            count += 1;
            in_field = true;
        }
    }
    count
}

/// `sanitizeHeadingHost` (`readsidecar.go:350`), verbatim semantics:
/// `strings.TrimSpace` (`White_Space` rune trim), then `/` → `-`, then ASCII
/// space (U+0020 ONLY — an interior NBSP/tab survives into the address),
/// then empty → `"untitled"`.
#[must_use]
pub fn sanitize_heading(title: &str) -> String {
    let trimmed = title.trim_matches(is_go_space);
    // Go runs two ReplaceAll passes; the char sets are disjoint, so one
    // combined pass is byte-equivalent.
    let replaced = trimmed.replace(['/', ' '], "-");
    if replaced.is_empty() {
        "untitled".to_string()
    } else {
        replaced
    }
}

/// The `buildTocEntries` dewey ordinal stack (`readsidecar.go:222`): feed
/// heading levels in document order; each call renders the heading's ordinal
/// ("1.2.1"). Level jumps and resets follow the Go algorithm exactly —
/// including its non-unique ordinals on malformed hierarchies (an H3 under
/// H1 and a later H2 under the same H1 both render "1.1").
#[derive(Debug, Default)]
pub struct DeweyCounter {
    ord: Vec<u32>,
    depth: Vec<u32>,
}

impl DeweyCounter {
    /// A fresh counter for one document walk.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Advance the stack for a heading at `level` and render its dewey
    /// ordinal.
    pub fn next(&mut self, level: u32) -> String {
        while self.depth.last().is_some_and(|&d| d > level) {
            self.ord.pop();
            self.depth.pop();
        }
        if self.depth.last().is_some_and(|&d| d == level) {
            if let Some(last) = self.ord.last_mut() {
                *last += 1;
            }
        } else {
            self.ord.push(1);
            self.depth.push(level);
        }
        self.ord
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(".")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pinned `White_Space` membership — the A-C1 divergence classes:
    /// NBSP and NEL ARE spaces; ZWSP, ZWNJ, BOM, and the Unicode-6.3-removed
    /// Mongolian vowel separator are NOT.
    #[test]
    fn go_space_table_membership() {
        let spaces = [
            '\t', '\n', '\u{000B}', '\u{000C}', '\r', ' ', '\u{0085}', '\u{00A0}', '\u{1680}',
            '\u{2000}', '\u{2005}', '\u{200A}', '\u{2028}', '\u{2029}', '\u{202F}', '\u{205F}',
            '\u{3000}',
        ];
        for c in spaces {
            assert!(is_go_space(c), "U+{:04X} is Go-space", c as u32);
        }
        let non_spaces = [
            '\u{200B}', '\u{200C}', '\u{FEFF}', '\u{180E}', 'a', '0', '-',
        ];
        for c in non_spaces {
            assert!(!is_go_space(c), "U+{:04X} is NOT Go-space", c as u32);
        }
    }

    /// `strings.Fields` semantics: NBSP separates words, ZWSP does not;
    /// leading/trailing runs and CRLF collapse; all-space is zero fields.
    #[test]
    fn fields_count_matches_strings_fields() {
        assert_eq!(fields_count(""), 0);
        assert_eq!(fields_count(" \t\n"), 0);
        assert_eq!(fields_count("one"), 1);
        assert_eq!(fields_count("  one  two "), 2);
        assert_eq!(fields_count("a\r\nb"), 2);
        // NBSP is `White_Space` → a separator
        assert_eq!(fields_count("a\u{00A0}b"), 2);
        // NEL is `White_Space` → a separator
        assert_eq!(fields_count("a\u{0085}b"), 2);
        // ZWSP is NOT `White_Space` → one word
        assert_eq!(fields_count("a\u{200B}b"), 1);
        // ideographic space separates
        assert_eq!(fields_count("你\u{3000}好"), 2);
    }

    /// `sanitizeHeadingHost` verbatim: trim is `White_Space`-wide, replacement
    /// is `/` and ASCII space ONLY — interior NBSP/tab survive.
    #[test]
    fn sanitize_heading_matches_go_host() {
        assert_eq!(sanitize_heading("Lab state"), "Lab-state");
        assert_eq!(sanitize_heading("a/b c"), "a-b-c");
        assert_eq!(sanitize_heading("  padded  "), "padded");
        // NBSP-padded: TrimSpace removes it (`White_Space`)
        assert_eq!(sanitize_heading("\u{00A0}x\u{00A0}"), "x");
        // interior NBSP is NOT the ASCII space — it survives verbatim
        assert_eq!(sanitize_heading("a\u{00A0}b"), "a\u{00A0}b");
        // interior tab likewise survives (Go replaces U+0020 only)
        assert_eq!(sanitize_heading("a\tb"), "a\tb");
        // empty and all-space collapse to the sentinel
        assert_eq!(sanitize_heading(""), "untitled");
        assert_eq!(sanitize_heading("   "), "untitled");
        assert_eq!(sanitize_heading("\u{2003}\u{2028}"), "untitled");
        // all-space AFTER replacement stays: "/" of spaces → dashes, not empty
        assert_eq!(sanitize_heading(" / "), "-");
    }

    /// The Go ordinal stack on a well-formed hierarchy.
    #[test]
    fn dewey_well_formed_sequence() {
        let mut d = DeweyCounter::new();
        let got: Vec<String> = [1, 2, 2, 3, 2, 1].iter().map(|&l| d.next(l)).collect();
        assert_eq!(got, ["1", "1.1", "1.2", "1.2.1", "1.3", "2"]);
    }

    /// A-C1 malformed classes: the H1→H3 jump pushes one rung (no phantom
    /// levels), the H6→H1 reset pops to the top, and the H3-then-H2 fold
    /// yields the Go stack's non-unique "1.1" — reproduced, not repaired.
    #[test]
    fn dewey_level_jumps_match_go() {
        let mut d = DeweyCounter::new();
        assert_eq!(d.next(1), "1");
        assert_eq!(d.next(3), "1.1"); // H1→H3 jump: one push
        assert_eq!(d.next(2), "1.1"); // H3→H2: pop the 3, push a 2 rung — same rendering
        assert_eq!(d.next(6), "1.1.1");
        assert_eq!(d.next(1), "2"); // H6→H1 reset pops everything

        // a document that OPENS at a deep level
        let mut d2 = DeweyCounter::new();
        assert_eq!(d2.next(3), "1");
        assert_eq!(d2.next(1), "1"); // pop-below-everything then push: renders "1" again
    }
}
