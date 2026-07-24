//! The host-face read facts (M1 U2): the addressing/render table ccc-statusd's
//! `buildTocEntries` (`readsidecar.go:215`) re-derived host-side, computed
//! engine-side ONCE — dewey ordinal, sanitized hpath address, raw title, word
//! count, CAS token — from the SAME input space the Go code consumed: the toc
//! row projection ([`crate::project_toc`]) plus the raw file bytes.
//!
//! Mirrored, not repaired (A-S2: the authoritative target is the U0 captured
//! golden corpus). Two deliberate Go behaviors ride along:
//!
//! - Only rows of kind `"heading"` and `"list_item"` become facts. An anchor
//!   whose HOST block is a task/callout/fence/table/paragraph projects under
//!   that kind and is DROPPED — exactly as the Go `switch` drops it, so e.g.
//!   a `- [ ] item ^t1` task anchor is NOT addressable on the read face (the
//!   U0 `basic` golden pins `^task1` unresolved).
//! - Selector resolution returns the FIRST match in row order (duplicate
//!   headings: read resolves the first occurrence; refusing ambiguous writes
//!   is the write plane's job, never last-wins).

use crate::gotext::{DeweyCounter, fields_count, sanitize_heading};

/// One host-face read row: the complete addressing fact set for one heading
/// section or one `list_item` block anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadFact {
    /// Dewey ordinal ("1.2.1") for headings; `"^id"` for anchor rows.
    pub n: String,
    /// Heading level; 0 for anchor rows (the Go `Depth 0` sentinel — toc-mode
    /// rendering skips these rows).
    pub depth: u32,
    /// RAW title: the last hpath segment verbatim (anchor rows: the id).
    pub title: String,
    /// The sanitized joined address ("Notes/Slash-Title-Here"); `"^id"` for
    /// anchor rows.
    pub hpath: String,
    /// `strings.Fields` word count over the content-span bytes (0 for anchor
    /// rows and content-less headings).
    pub words: u64,
    /// The node's CAS token (`node_rev`).
    pub sec_rev: String,
    /// Full node span (heading-inclusive; anchor rows: the block-leaf span).
    pub span: wire::Span,
    /// Heading rows: the heading-excluded, SUBTREE-inclusive content span.
    pub content_span: Option<wire::Span>,
    /// Anchor rows: the block id (without `^`).
    pub anchor: Option<String>,
}

/// Lift the toc row projection into host-face read facts — `buildTocEntries`
/// verbatim: dewey from the heading level sequence, sanitized hpath, raw
/// title, `strings.Fields` word count over each heading's content span;
/// `list_item` anchor rows as `^id` facts; every other row kind dropped.
#[must_use]
pub fn read_facts(rows: &[wire::TocNode], raw: &[u8]) -> Vec<ReadFact> {
    let mut out = Vec::new();
    let mut dewey = DeweyCounter::new();
    for row in rows {
        match row.kind.as_str() {
            "heading" => {
                let level = row.level.unwrap_or(0);
                let segs = row.hpath.as_deref().unwrap_or_default();
                let hpath = segs
                    .iter()
                    .map(|s| sanitize_heading(&s.h))
                    .collect::<Vec<_>>()
                    .join("/");
                let title = segs.last().map(|s| s.h.clone()).unwrap_or_default();
                let words = row
                    .content_span
                    .as_ref()
                    .map_or(0, |cs| count_span_words(raw, cs));
                out.push(ReadFact {
                    n: dewey.next(level),
                    depth: level,
                    title,
                    hpath,
                    words,
                    sec_rev: row.node_rev.0.clone(),
                    span: row.span,
                    content_span: row.content_span,
                    anchor: None,
                });
            }
            "list_item" => {
                let Some(anchor) = row.anchor.as_deref() else {
                    continue; // a list_item row without an anchor never projects
                };
                out.push(ReadFact {
                    n: format!("^{anchor}"),
                    depth: 0,
                    title: anchor.to_string(),
                    hpath: format!("^{anchor}"),
                    words: 0,
                    sec_rev: row.node_rev.0.clone(),
                    span: row.span,
                    content_span: None,
                    anchor: Some(anchor.to_string()),
                });
            }
            // frontmatter and every other host kind: no read row (the Go
            // switch default — task/callout/fence/table/paragraph anchors
            // are NOT addressable on this face).
            _ => {}
        }
    }
    out
}

/// `strings.Fields` count over `raw[span]`, with the Go `sliceSpan` defensive
/// bounds guard (a malformed span counts as empty, never panics). A slice
/// that splits a multibyte char decodes lossily — U+FFFD is not
/// `White_Space`, matching Go's `RuneError` field behavior.
#[must_use]
pub fn count_span_words(raw: &[u8], span: &wire::Span) -> u64 {
    fields_count(&String::from_utf8_lossy(slice_span(raw, span))) as u64
}

/// `sliceSpan` (`readsidecar.go:374`): `raw[start..end]` with the bounds
/// guard — out-of-range or inverted spans yield the empty slice.
#[must_use]
pub fn slice_span<'a>(raw: &'a [u8], span: &wire::Span) -> &'a [u8] {
    let (start, end) = (
        usize::try_from(span.0).unwrap_or(usize::MAX),
        usize::try_from(span.1).unwrap_or(usize::MAX),
    );
    if end > raw.len() || end < start {
        return &[];
    }
    &raw[start..end]
}

/// `resolveSelector` (`readsidecar.go:329`): match a selector against the
/// facts by exact sanitized hpath OR dewey/anchor `n` — FIRST match wins
/// (duplicate headings resolve to the first occurrence on the read face).
#[must_use]
pub fn resolve_selector<'a>(facts: &'a [ReadFact], sel: &str) -> Option<&'a ReadFact> {
    facts.iter().find(|f| f.hpath == sel || f.n == sel)
}

/// The sections-mode content bytes for one resolved fact
/// (`renderSectionsSidecar`, `readsidecar.go:295`): a heading's
/// heading-excluded content span, or a block leaf's full span with the
/// trailing `^id` marker stripped. RAW face — the render plane never elides
/// here (D's U0 pin: `meridian-*` blocks ride the raw read face verbatim).
#[must_use]
pub fn section_content(fact: &ReadFact, raw: &[u8]) -> Vec<u8> {
    match (&fact.content_span, &fact.anchor) {
        (Some(cs), _) => slice_span(raw, cs).to_vec(),
        (None, Some(anchor)) => strip_anchor_marker(slice_span(raw, &fact.span), anchor),
        (None, None) => Vec::new(),
    }
}

/// `stripAnchorMarker` (`readsidecar.go:384`): remove a trailing `" ^id"` /
/// `"^id"` block marker (space/tab-padded) from an inline-anchor block span;
/// a span without the suffix returns UNCHANGED (not even right-trimmed).
#[must_use]
pub fn strip_anchor_marker(b: &[u8], anchor: &str) -> Vec<u8> {
    let s = String::from_utf8_lossy(b);
    let trimmed = s.trim_end_matches([' ', '\t']);
    let marker = format!("^{anchor}");
    if let Some(stripped) = trimmed.strip_suffix(&marker) {
        stripped.trim_end_matches([' ', '\t']).as_bytes().to_vec()
    } else {
        b.to_vec()
    }
}

/// The toc-mode row filter (`renderTocSidecar`): heading rows only (anchor
/// rows are Depth-0, shape-table-excluded), optionally scoped to the `frag`
/// subtree — the section itself plus descendants by hpath prefix. An empty
/// result under a non-empty `frag` is the caller's "no section at" refusal.
#[must_use]
pub fn toc_rows<'a>(facts: &'a [ReadFact], frag: &str) -> Vec<&'a ReadFact> {
    facts
        .iter()
        .filter(|f| f.depth > 0)
        .filter(|f| frag.is_empty() || f.hpath == frag || f.hpath.starts_with(&format!("{frag}/")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(raw: &str) -> Vec<ReadFact> {
        let doc = model::build(raw.to_string(), syntax::parse(raw));
        read_facts(&crate::project_toc(&doc), raw.as_bytes())
    }

    /// The worked S0 plan doc: dewey, sanitized addresses, subtree-inclusive
    /// word counts (Goals counts its whole subtree), raw titles.
    #[test]
    fn s0_plan_read_facts() {
        let raw = "---\ntitle: Plan\n---\n# Goals\n\nShip the contract.\n\n## Q3\n\nship by August\n\n## Q4\n\n- item one\n- see [[2026-07-18]]\n- blocked on [[roadmap]]\n";
        let got = facts(raw);
        let view: Vec<(&str, u32, &str, &str, u64)> = got
            .iter()
            .map(|f| {
                (
                    f.n.as_str(),
                    f.depth,
                    f.title.as_str(),
                    f.hpath.as_str(),
                    f.words,
                )
            })
            .collect();
        assert_eq!(
            view,
            vec![
                // Goals' content span is SUBTREE-inclusive (## child heading
                // tokens count as words: 3 + 2 + 3 + 2 + 10)
                ("1", 1, "Goals", "Goals", 20),
                ("1.1", 2, "Q3", "Goals/Q3", 3),
                ("1.2", 2, "Q4", "Goals/Q4", 10),
            ]
        );
        // frontmatter projects no read row
        assert_eq!(got.len(), 3);
    }

    /// A task-hosted anchor (`- [ ] x ^t1`) projects kind `task` → DROPPED
    /// (the Go switch default); a plain `list_item` anchor projects a `^id`
    /// row. The U0 `basic` golden pins the task case unresolved.
    #[test]
    fn task_anchor_dropped_list_item_anchor_kept() {
        let raw = "# H\n\n- [ ] boxed ^t1\n- plain item ^p1\n";
        let got = facts(raw);
        let anchors: Vec<&str> = got.iter().filter_map(|f| f.anchor.as_deref()).collect();
        assert_eq!(anchors, vec!["p1"], "only the list_item anchor survives");
        assert!(resolve_selector(&got, "^t1").is_none());
        let p1 = resolve_selector(&got, "^p1").expect("list_item anchor resolves");
        assert_eq!((p1.n.as_str(), p1.depth, p1.words), ("^p1", 0, 0));
    }

    /// Duplicate headings: both occurrences get rows (dewey disambiguates),
    /// and selector resolution returns the FIRST — never last-wins.
    #[test]
    fn duplicate_headings_first_occurrence_wins() {
        let raw = "# Notes\n\nfirst\n\n# Notes\n\nsecond\n\n## Child\n\nchild body\n";
        let got = facts(raw);
        let ns: Vec<&str> = got.iter().map(|f| f.n.as_str()).collect();
        assert_eq!(ns, vec!["1", "2", "2.1"]);
        let hit = resolve_selector(&got, "Notes").expect("resolves");
        assert_eq!(hit.n, "1", "first occurrence");
        // subtree words: second + ## + Child + child + body
        assert_eq!(resolve_selector(&got, "2").map(|f| f.words), Some(5));
    }

    /// Frag scoping: the section itself + descendants by hpath prefix;
    /// sibling prefixes ("Note" vs "Notes") never match.
    #[test]
    fn toc_rows_frag_subtree() {
        let raw = "# Notes\n\nx\n\n## Deep\n\ny\n\n# Notes2\n\nz\n";
        let got = facts(raw);
        let scoped: Vec<&str> = toc_rows(&got, "Notes")
            .iter()
            .map(|f| f.hpath.as_str())
            .collect();
        assert_eq!(scoped, vec!["Notes", "Notes/Deep"]);
        assert!(toc_rows(&got, "Ghost").is_empty());
    }
}
