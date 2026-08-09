//! Go-exact text semantics for host-face addressing facts.
//!
//! Primitives replicate Go stdlib semantics (byte-identity with
//! `readsidecar.go`), never idiomatic-Rust near-equivalents:
//!
//! - [`is_go_space`] = Go `unicode.IsSpace` (explicit `White_Space` table).
//! - [`fields_count`] = `len(strings.Fields(s))`.
//! - [`sanitize_heading`] = `sanitizeHeadingHost` (`readsidecar.go:350`):
//!   `TrimSpace`, `/` → `-`, ASCII space → `-`, empty → `"untitled"`.
//! - [`DeweyCounter`] = `buildTocEntries` ordinal stack — including
//!   malformed-hierarchy behavior; reproduced, not repaired.
//!
//! [`is_go_space`] / [`sanitize_heading`] re-export [`model::gotext`] (one
//! owner). Word count and dewey stay here (projection facts, law 3).
//!
//! Parity target: the golden corpus. Unit tests pin divergence classes
//! (NBSP/NEL space, ZWSP not, level jumps, empty headings).

pub use model::gotext::{is_go_space, sanitize_heading};

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

    /// The re-export resolves to the owner. Identity is a compile-time fact —
    /// `pub use model::gotext::…` is one item, not a copy, which is why the
    /// projection here and `policy`'s defs rebuild cannot drift. Membership
    /// and the full drift-guard corpus are pinned at the owner.
    #[test]
    fn heading_predicate_reexports_the_model_owner() {
        assert_eq!(sanitize_heading("Slash/Title Here"), "Slash-Title-Here");
        assert!(is_go_space('\u{00A0}') && !is_go_space('\u{200B}'));
    }

    /// The Go ordinal stack on a well-formed hierarchy.
    #[test]
    fn dewey_well_formed_sequence() {
        let mut d = DeweyCounter::new();
        let got: Vec<String> = [1, 2, 2, 3, 2, 1].iter().map(|&l| d.next(l)).collect();
        assert_eq!(got, ["1", "1.1", "1.2", "1.2.1", "1.3", "2"]);
    }

    /// Malformed classes: the H1→H3 jump pushes one rung (no phantom
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
