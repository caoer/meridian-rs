//! FROZEN Go-text heading predicate — sole engine owner (stage-2 S0).
//!
//! `sanitize_heading` is the address law for every heading-derived name.
//! Byte-for-byte with meridian-go `sanitizeHeading` / `sanitizeHeadingHost`
//! (`map.go`, `readsidecar.go:350`): `TrimSpace`, `/`→`-`, ASCII space→`-`,
//! empty→`"untitled"`. Frozen — reproduced, not improved.
//!
//! Lives in `model` (not `wire-map`) so three consumers cannot drift:
//! `wire-map::facts`, `policy::defs::rebuild`, and (until S2) the Go host
//! mirror. Frozen character-class primitive only; projection (dewey, hpath
//! assembly) stays in `wire-map` (law 3).
//!
//! `is_go_space` pins the trim character class as an explicit table so stdlib
//! Unicode revisions cannot skew the address law.

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

/// `sanitizeHeading` (`map.go`) / `sanitizeHeadingHost`
/// (`readsidecar.go:350`), verbatim semantics: `strings.TrimSpace`
/// (`White_Space` rune trim), then `/` → `-`, then ASCII space (U+0020 ONLY —
/// an interior NBSP/tab survives into the address), then empty → `"untitled"`.
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

    /// Every heading of the Go-side drift-guard corpus
    /// (`ccc-statusd/internal/mcpserver/testdata/parity/corpus`, 61 headings
    /// over 14 roots), paired with the output of the REAL Go
    /// `sanitizeHeadingHost` (`puttoc.go:144`) over the same input. This is
    /// the engine-side successor of `sanitize_drift_guard_test.go`: when S2
    /// deletes the host mirror, this table is what keeps the address law
    /// pinned to Go's bytes.
    const DRIFT_GUARD_CORPUS: &[(&str, &str)] = &[
        ("Tasks", "Tasks"),                                 // authz/agents/other999.md:6
        ("Handoff", "Handoff"),                             // authz/agents/other999.md:10
        ("Notes", "Notes"),                                 // authz/agents/other999.md:14
        ("Tasks", "Tasks"),                                 // authz/agents/w1234567.md:6
        ("Handoff", "Handoff"),                             // authz/agents/w1234567.md:10
        ("Notes", "Notes"),                                 // authz/agents/w1234567.md:14
        ("Todo", "Todo"),                                   // basic/corpus/basic.md:6
        ("Notes", "Notes"),                                 // basic/corpus/basic.md:11
        ("Slash/Title Here", "Slash-Title-Here"),           // basic/corpus/basic.md:16
        ("Memo", "Memo"),                                   // basic/corpus/basic.md:20
        ("First", "First"),                                 // crlf/corpus/crlf.md:5
        ("Second", "Second"),                               // crlf/corpus/crlf.md:10
        ("Properties", "Properties"),                       // defs/defs/broken.md:7
        ("Properties", "Properties"),                       // defs/defs/probe-fast.md:7
        ("Properties", "Properties"),                       // defs/defs/probe.md:7
        ("Sections", "Sections"),                           // defs/defs/probe.md:18
        ("section: Log", "section:-Log"),                   // defs/defs/probe.md:20
        ("section: Answer", "section:-Answer"),             // defs/defs/probe.md:26
        ("Properties", "Properties"),                       // defs/home/defs/probe.md:7
        ("Notes", "Notes"),                                 // defs/records/broken-rec.md:6
        ("Log", "Log"),                                     // defs/records/fast.md:8
        ("Answer", "Answer"),                               // defs/records/fast.md:12
        ("Log", "Log"),                                     // defs/records/item.md:9
        ("Answer", "Answer"),                               // defs/records/item.md:13
        ("Scratch", "Scratch"),                             // defs/records/item.md:17
        ("Notes", "Notes"),                                 // defs/records/mystery.md:5
        ("Notes", "Notes"), // duplicate-headings/corpus/duplicate-headings.md:5
        ("Notes", "Notes"), // duplicate-headings/corpus/duplicate-headings.md:9
        ("Child", "Child"), // duplicate-headings/corpus/duplicate-headings.md:13
        ("Empty One", "Empty-One"), // empty-sections/corpus/empty-sections.md:5
        ("Empty Two", "Empty-Two"), // empty-sections/corpus/empty-sections.md:6
        ("", "untitled"),   // empty-sections/corpus/empty-sections.md:8
        ("After Blank", "After-Blank"), // empty-sections/corpus/empty-sections.md:10
        ("Content At Last", "Content-At-Last"), // empty-sections/corpus/empty-sections.md:13
        ("One", "One"),     // level-jumps/corpus/level-jumps.md:1
        ("Jump Three", "Jump-Three"), // level-jumps/corpus/level-jumps.md:5
        ("Four", "Four"),   // level-jumps/corpus/level-jumps.md:9
        ("Back Two", "Back-Two"), // level-jumps/corpus/level-jumps.md:13
        ("Six", "Six"),     // level-jumps/corpus/level-jumps.md:17
        ("Reset One", "Reset-One"), // level-jumps/corpus/level-jumps.md:21
        ("Two After Reset", "Two-After-Reset"), // level-jumps/corpus/level-jumps.md:25
        ("Config", "Config"), // meridian-block/corpus/meridian-block.md:5
        ("Code", "Code"),   // meridian-block/corpus/meridian-block.md:14
        ("Solo", "Solo"),   // no-frontmatter/corpus/no-frontmatter.md:3
        ("Todo", "Todo"),   // read-modes/corpus/read-modes.md:6
        ("Notes", "Notes"), // read-modes/corpus/read-modes.md:11
        ("Slash/Title Here", "Slash-Title-Here"), // read-modes/corpus/read-modes.md:16
        ("Memo", "Memo"),   // read-modes/corpus/read-modes.md:20
        ("Todo", "Todo"),   // rebuild/corpus/rebuild.md:6
        ("Notes", "Notes"), // rebuild/corpus/rebuild.md:11
        ("Slash/Title Here", "Slash-Title-Here"), // rebuild/corpus/rebuild.md:16
        ("Memo", "Memo"),   // rebuild/corpus/rebuild.md:20
        ("Padded Title   ", "Padded-Title"), // trailing-ws/corpus/trailing-ws.md:5
        ("Anchor Zone", "Anchor-Zone"), // trailing-ws/corpus/trailing-ws.md:10
        ("Tail \t", "Tail"), // trailing-ws/corpus/trailing-ws.md:17
        ("Alpha\u{00A0}Beta", "Alpha\u{00A0}Beta"), // unicode-ws/corpus/unicode-ws.md:5
        ("Gamma\u{200B}Delta", "Gamma\u{200B}Delta"), // unicode-ws/corpus/unicode-ws.md:9
        ("Trailing NBSP\u{00A0}", "Trailing-NBSP"), // unicode-ws/corpus/unicode-ws.md:13
        ("Trailing ZWSP\u{200B}", "Trailing-ZWSP\u{200B}"), // unicode-ws/corpus/unicode-ws.md:17
        ("NEL\u{0085}Inside", "NEL\u{0085}Inside"), // unicode-ws/corpus/unicode-ws.md:21
        ("Em\u{2003}Space", "Em\u{2003}Space"), // unicode-ws/corpus/unicode-ws.md:25
    ];

    #[test]
    fn sanitize_heading_matches_drift_guard_corpus() {
        // Vacuity guard: the corpus must be the WHOLE 61, not a truncation.
        assert_eq!(
            DRIFT_GUARD_CORPUS.len(),
            61,
            "drift-guard corpus must carry all 61 headings"
        );
        for (title, want) in DRIFT_GUARD_CORPUS {
            assert_eq!(
                &sanitize_heading(title),
                want,
                "sanitize_heading({title:?}) drifted from Go sanitizeHeadingHost"
            );
        }
    }
}
