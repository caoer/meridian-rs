//! The host-face read facts: the addressing/render table, computed
//! engine-side ONCE — dewey ordinal, hpath address, raw title, word count,
//! CAS token — from the toc row projection ([`crate::project_toc`]) plus the
//! raw file bytes.
//!
//! Mirrored, not repaired (the authoritative target is the captured golden
//! corpus). One deliberate departure from the Go switch (F-R4 ruling,
//! 2026-08-13 — the old face under-implemented the format it speaks):
//!
//! - Heading rows and EVERY body-hosted anchor row become facts — list_item,
//!   task, callout, fence, table, paragraph and heading hosts alike, the
//!   full set Obsidian's own block references address. The one exclusion is
//!   a `frontmatter`-hosted anchor (truth-told since dogfood P2-c): a caret
//!   there is literal YAML, no block exists, and the keys are already served
//!   on the `props` plane. An anchor row is an anchor-plane row whatever
//!   host kind it echoes: it never enters the heading plane, so a
//!   `heading`-kinded ANCHOR row is not a section fact.
//!
//! One deliberate departure from the Go face:
//!
//! - Selector resolution returns ALL matches in document order (headings and
//!   block anchors alike): a bare duplicate yields >1 and the caller refuses
//!   `ambiguous_ref` naming the candidates — the strict plane never silently
//!   picks (§2.1; wire-contract A.3, door symmetry over duplicate ids). The
//!   dewey arm stays first-match (≤1 entry): a dewey ordinal is a positional
//!   row handle whose duplicates are a numbering artifact, not an ambiguity
//!   ([`selector_matches`]).

use crate::gotext::{DeweyCounter, fields_count};

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
    /// The address as raw segments — the same grammar `put` takes in
    /// `target.hpath`, carried verbatim so this face publishes an address the
    /// write plane accepts. Empty on anchor rows (their put grammar is
    /// `{"anchor":id}`). Per-segment `n` rides only where the raw text is
    /// ambiguous among its same-parent siblings ([`raw_addresses`]).
    pub hpath: Vec<wire::HpathSeg>,
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
/// lineage: dewey from the heading level sequence, sanitized hpath, raw
/// title, `strings.Fields` word count over each heading's content span;
/// every body-hosted anchor row as a `^id` fact (F-R4 — the set Obsidian
/// addresses); `frontmatter`-hosted anchors dropped (literal YAML, served on
/// the `props` plane).
#[must_use]
pub fn read_facts(rows: &[wire::TocNode], raw: &[u8]) -> Vec<ReadFact> {
    let mut out = Vec::new();
    let mut dewey = DeweyCounter::new();
    let raw_addrs = raw_addresses(rows);
    for (i, row) in rows.iter().enumerate() {
        // An anchor row never enters the heading plane, whatever host kind
        // it echoes (its `hpath` is None — it addresses nothing there).
        match row.anchor.as_deref() {
            None if row.kind == "heading" => {
                let level = row.level.unwrap_or(0);
                let segs = row.hpath.as_deref().unwrap_or_default();
                let title = segs.last().map(|s| s.h.clone()).unwrap_or_default();
                let words = row
                    .content_span
                    .as_ref()
                    .map_or(0, |cs| count_span_words(raw, cs));
                out.push(ReadFact {
                    n: dewey.next(level),
                    depth: level,
                    title,
                    hpath: raw_addrs[i].clone(),
                    words,
                    sec_rev: row.node_rev.0.clone(),
                    span: row.span,
                    content_span: row.content_span,
                    anchor: None,
                });
            }
            Some(anchor) if row.kind != "frontmatter" => {
                out.push(ReadFact {
                    n: format!("^{anchor}"),
                    depth: 0,
                    title: anchor.to_string(),
                    // an anchor row has no HEADING address and is not given a
                    // fabricated one; it is addressed through
                    // `ReadSel::Anchor`, off the `anchor` field below
                    hpath: Vec::new(),
                    words: 0,
                    sec_rev: row.node_rev.0.clone(),
                    span: row.span,
                    content_span: None,
                    anchor: Some(anchor.to_string()),
                });
            }
            // frontmatter-hosted anchors (no block to serve — the caret is
            // literal YAML) and anchor-less non-heading rows: no read row.
            _ => {}
        }
    }
    out
}

/// The raw put-grammar address for every row, indexed by row (non-heading rows
/// get an empty address) — the seam that closes the read→put loop.
///
/// `n` rides a segment only where the raw text is ambiguous among its
/// same-parent siblings: `n: None` demands uniqueness in
/// `model::resolve_hpath_node`, so a minimal address starts refusing once a
/// duplicate appears rather than silently resolving to one of them. The
/// occurrence counted is the one `resolve_hpath_node` counts — position among
/// siblings sharing the raw text under the same parent, never position among
/// all siblings and never document order.
///
/// Two passes, because ambiguity is a whole-document fact: pass 1 assigns each
/// heading row its containment parent, raw text, and occurrence; pass 2 turns
/// the totals into `n` and clones each parent's finished address (parents
/// precede children in document order, so the prefix is always ready).
fn raw_addresses(rows: &[wire::TocNode]) -> Vec<Vec<wire::HpathSeg>> {
    struct Seg {
        /// Row index of the containment parent; `None` at the top level.
        parent: Option<usize>,
        text: String,
        /// 1-based occurrence among same-parent siblings sharing `text`.
        occ: u32,
    }

    let mut segs: Vec<Option<Seg>> = (0..rows.len()).map(|_| None).collect();
    let mut counts: std::collections::HashMap<(Option<usize>, String), u32> =
        std::collections::HashMap::new();
    // Ancestor row indices by containment depth: `stack[d - 1]` is the row of
    // the depth-`d` ancestor. `hpath.len()` IS the containment depth (a section
    // carries the chain of governing headings including its own), and
    // `project_toc` emits rows in span order, so a row's ancestors are exactly
    // the most recent rows at the shallower depths — heading level skips
    // (`#` then `###`) do not disturb this.
    let mut stack: Vec<usize> = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        // Anchor rows never carry a heading address, whatever host kind they
        // echo — one entering here would corrupt the ancestor stack.
        if row.kind != "heading" || row.anchor.is_some() {
            continue;
        }
        let Some(text) = row.hpath.as_deref().and_then(<[_]>::last) else {
            continue; // a heading row with no chain addresses nothing
        };
        let depth = row.hpath.as_deref().map_or(0, <[_]>::len);
        stack.truncate(depth.saturating_sub(1));
        let parent = stack.last().copied();
        let occ = counts
            .entry((parent, text.h.clone()))
            .and_modify(|c| *c += 1)
            .or_insert(1);
        segs[i] = Some(Seg {
            parent,
            text: text.h.clone(),
            occ: *occ,
        });
        stack.push(i);
    }

    let mut out: Vec<Vec<wire::HpathSeg>> = (0..rows.len()).map(|_| Vec::new()).collect();
    for i in 0..rows.len() {
        let Some(seg) = &segs[i] else { continue };
        let mut addr = seg.parent.map(|p| out[p].clone()).unwrap_or_default();
        let ambiguous = counts
            .get(&(seg.parent, seg.text.clone()))
            .is_some_and(|&c| c > 1);
        addr.push(wire::HpathSeg {
            h: seg.text.clone(),
            n: ambiguous.then_some(seg.occ),
        });
        out[i] = addr;
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

/// Every fact a TAGGED selector addresses, in document order — the read op's
/// resolution primitive (wire-contract A.3). [`wire::ReadSel`] states the
/// plane, so nothing is inferred from spelling.
///
/// The heading arm compares SEGMENTS, per segment, on raw text, and returns
/// ALL matching rows: a selector segment with `n: None` matches any occurrence,
/// so a bare duplicate yields >1 and the caller refuses `ambiguous_ref` naming
/// the candidates — the strict plane never silently picks (§2.1). `n: Some(k)`
/// demands that occurrence exactly, so the addresses this face publishes —
/// minimal, carrying `n` only where ambiguous ([`raw_addresses`]) — resolve
/// back to the one row they were published from.
///
/// The anchor arm returns ALL blocks carrying the id, in document order: a
/// duplicate block id is an ambiguity like a duplicate heading, and the caller
/// refuses `ambiguous_ref` rather than silently serving the first carrier
/// (§2.1; wire-contract A.3, door symmetry over duplicate block ids). The
/// dewey arm stays first-match (≤1 entry): a dewey ordinal is a positional row
/// handle whose duplicates are a numbering artifact, not an ambiguity (see
/// `write::canonical_selector`).
#[must_use]
pub fn selector_matches<'a>(facts: &'a [ReadFact], sel: &wire::ReadSel) -> Vec<&'a ReadFact> {
    match sel {
        wire::ReadSel::Hpath { hpath } => facts
            .iter()
            .filter(|f| hpath_matches(hpath, &f.hpath))
            .collect(),
        wire::ReadSel::Dewey { n } => facts.iter().find(|f| &f.n == n).into_iter().collect(),
        wire::ReadSel::Anchor { anchor } => facts
            .iter()
            .filter(|f| f.anchor.as_deref() == Some(anchor))
            .collect(),
    }
}

/// [`selector_matches`] collapsed to the unique case: the fact when exactly
/// one row matches, `None` on a miss — and `None` on an ambiguous selector
/// (duplicate heading or duplicate block id), which a caller must instead
/// refuse with the ambiguity's own message, never report as a miss.
#[must_use]
pub fn resolve_selector<'a>(facts: &'a [ReadFact], sel: &wire::ReadSel) -> Option<&'a ReadFact> {
    let matches = selector_matches(facts, sel);
    if matches.len() == 1 {
        Some(matches[0])
    } else {
        None
    }
}

/// Per-segment address equality: same length, same raw text, and a selector
/// occurrence index that either abstains (`None` — first match wins) or names
/// the row's own. Byte-exact on `h`; nothing is sanitized on either side.
fn hpath_matches(sel: &[wire::HpathSeg], row: &[wire::HpathSeg]) -> bool {
    sel.len() == row.len()
        && sel
            .iter()
            .zip(row)
            .all(|(s, r)| s.h == r.h && (s.n.is_none() || s.n == r.n))
}

/// The sections-mode content bytes for one resolved fact
/// (`renderSectionsSidecar`, `readsidecar.go:295`): a heading's
/// heading-excluded content span, or a block leaf's full span with the
/// trailing `^id` marker stripped. RAW face — the render plane never elides
/// here (`meridian-*` blocks ride the raw read face verbatim).
#[must_use]
pub fn section_content(fact: &ReadFact, raw: &[u8]) -> Vec<u8> {
    match (&fact.content_span, &fact.anchor) {
        (Some(cs), _) => slice_span(raw, cs).to_vec(),
        (None, Some(anchor)) => strip_anchor_marker(slice_span(raw, &fact.span), anchor),
        (None, None) => Vec::new(),
    }
}

/// The ONE section-grain word count: fields over the section's own raw
/// content bytes ([`section_content`]) — the same bytes a `put` is built
/// from. Every face that publishes a section's `words` derives it here, so
/// the structured plane and the rendered projection cannot answer one
/// question with two numbers (F-S4): elision and decoration change what the
/// reader is SHOWN, never what the section HOLDS.
#[must_use]
pub fn section_words(fact: &ReadFact, raw: &[u8]) -> u64 {
    fields_count(&String::from_utf8_lossy(&section_content(fact, raw))) as u64
}

/// The ONE whole-file word count: fields over the document's raw bytes —
/// `wc -w` of the file, the number a reader cross-checks against.
///
/// Never a sum of toc rows. A row counts its heading-excluded but
/// SUBTREE-INCLUSIVE content span, so summing rows counts every descendant
/// once per ancestor level — a ~2x lie on any nested document, and the
/// banner is what readers budget from (D-USER r2 F3).
#[must_use]
pub fn words_total(raw: &[u8]) -> u64 {
    fields_count(&String::from_utf8_lossy(raw)) as u64
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
///
/// This is the heading plane, whole: `toc_text` and the composed read's `toc`
/// array carry exactly these rows, so no `toc` consumer can meet a second row
/// class. The `^id` anchor plane is [`anchor_rows`], served in its own array.
#[must_use]
pub fn toc_rows<'a>(facts: &'a [ReadFact], frag: &[wire::HpathSeg]) -> Vec<&'a ReadFact> {
    facts
        .iter()
        .filter(|f| f.depth > 0)
        .filter(|f| in_frag(f, frag))
        .collect()
}

/// The `^id` anchor plane: the block-anchor facts alone, in document order.
/// Put derives a write's governing sections by byte containment (every heading
/// whose span contains the anchor block's start byte), which is absolute-byte
/// arithmetic — so the two planes stay independent, and serving anchors out of
/// the [`toc_rows`] array is what keeps a `toc` consumer from meeting a row
/// class it does not expect.
///
/// Under a non-empty `frag` the anchor rows are scoped by that same byte
/// containment, so a scoped read never leaks a row from outside the requested
/// subtree. An empty `frag` is the whole document, including anchors above the
/// first heading.
#[must_use]
pub fn anchor_rows<'a>(facts: &'a [ReadFact], frag: &[wire::HpathSeg]) -> Vec<&'a ReadFact> {
    let scope: Vec<wire::Span> = toc_rows(facts, frag).iter().map(|f| f.span).collect();
    facts
        .iter()
        .filter(|f| f.depth == 0)
        .filter(|f| frag.is_empty() || scope.iter().any(|s| s.0 <= f.span.0 && f.span.0 < s.1))
        .collect()
}

/// The `frag` subtree predicate for a heading row: the section itself plus
/// descendants by SEGMENT prefix; an empty `frag` is the whole document.
fn in_frag(f: &ReadFact, frag: &[wire::HpathSeg]) -> bool {
    frag.is_empty() || (f.hpath.len() >= frag.len() && hpath_matches(frag, &f.hpath[..frag.len()]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gotext::sanitize_heading;

    fn facts(raw: &str) -> Vec<ReadFact> {
        let doc = model::build(raw.to_string(), syntax::parse(raw));
        read_facts(&crate::project_toc(&doc), raw.as_bytes())
    }

    /// Render one fact's address compactly for assertions: `A#2/B` — `#n` only
    /// where an occurrence index rides. A TEST spelling of a machine address;
    /// nothing in the engine joins one.
    fn addr(f: &ReadFact) -> String {
        wire::ReadSel::Hpath {
            hpath: f.hpath.clone(),
        }
        .display()
    }

    /// The whole document — the empty `frag` scope, spelled once.
    const ALL: &[wire::HpathSeg] = &[];

    /// A heading selector from a `/`-joined test spelling, through the ONE
    /// ingress door (so the tests exercise the door callers use).
    fn sel(s: &str) -> wire::ReadSel {
        wire::ReadSel::parse(s)
    }

    /// A `frag` scope from a `/`-joined test spelling.
    fn frag(s: &str) -> Vec<wire::HpathSeg> {
        match wire::ReadSel::parse(s) {
            wire::ReadSel::Hpath { hpath } => hpath,
            other => panic!("a frag scopes headings, got {other:?}"),
        }
    }

    /// The worked S0 plan doc: dewey, sanitized addresses, subtree-inclusive
    /// word counts (Goals counts its whole subtree), raw titles.
    #[test]
    fn s0_plan_read_facts() {
        let raw = "---\ntitle: Plan\n---\n# Goals\n\nShip the contract.\n\n## Q3\n\nship by August\n\n## Q4\n\n- item one\n- see [[2026-07-18]]\n- blocked on [[roadmap]]\n";
        let got = facts(raw);
        let view: Vec<(&str, u32, &str, String, u64)> = got
            .iter()
            .map(|f| (f.n.as_str(), f.depth, f.title.as_str(), addr(f), f.words))
            .collect();
        assert_eq!(
            view,
            vec![
                // Goals' content span is SUBTREE-inclusive (## child heading
                // tokens count as words: 3 + 2 + 3 + 2 + 10)
                ("1", 1, "Goals", "Goals".to_owned(), 20),
                ("1.1", 2, "Q3", "Goals/Q3".to_owned(), 3),
                ("1.2", 2, "Q4", "Goals/Q4".to_owned(), 10),
            ]
        );
        // frontmatter projects no read row
        assert_eq!(got.len(), 3);
    }

    /// Every body-hosted anchor projects a `^id` row since F-R4 — a
    /// task-hosted anchor (`- [ ] x ^t1`) is as addressable as a plain
    /// `list_item` one (the set Obsidian's own block references cover).
    #[test]
    fn task_and_list_item_anchors_both_project() {
        let raw = "# H\n\n- [ ] boxed ^t1\n- plain item ^p1\n";
        let got = facts(raw);
        let anchors: Vec<&str> = got.iter().filter_map(|f| f.anchor.as_deref()).collect();
        assert_eq!(anchors, vec!["t1", "p1"], "both body-hosted anchors project");
        let t1 = resolve_selector(&got, &sel("^t1")).expect("task anchor resolves (F-R4)");
        assert_eq!((t1.n.as_str(), t1.depth, t1.words), ("^t1", 0, 0));
        let p1 = resolve_selector(&got, &sel("^p1")).expect("list_item anchor resolves");
        assert_eq!((p1.n.as_str(), p1.depth, p1.words), ("^p1", 0, 0));
    }

    /// Grammar guard (dogfood P2-c lineage, F-R4 shape): a heading-hosted
    /// anchor enters the ANCHOR plane — never the heading plane, which
    /// carries exactly the real headings and no re-kinded anchor row. A
    /// frontmatter caret stays off both planes (literal YAML, no block —
    /// served on the `props` plane).
    #[test]
    fn heading_anchor_rides_the_anchor_plane_fm_caret_stays_off() {
        let raw = "---\ntitle: x ^fm-anchor\n---\n## Has anchor ^anch-head\n\n- item ^li\n";
        let got = facts(raw);
        let anchors: Vec<&str> = got.iter().filter_map(|f| f.anchor.as_deref()).collect();
        assert_eq!(
            anchors,
            vec!["anch-head", "li"],
            "the heading-hosted id projects; the frontmatter caret never does"
        );
        resolve_selector(&got, &sel("^anch-head")).expect("heading-hosted id resolves (F-R4)");
        assert!(resolve_selector(&got, &sel("^fm-anchor")).is_none());
        let headings: Vec<&str> = got
            .iter()
            .filter(|f| f.anchor.is_none())
            .map(|f| f.n.as_str())
            .collect();
        assert_eq!(
            headings,
            vec!["1"],
            "one real heading, no re-kinded anchor row"
        );
    }

    /// A duplicated block id matches EVERY carrier, in document order — the
    /// caller refuses `ambiguous_ref`; first-match was the silent-pick defect
    /// (dogfood-p1-read-ambiguous-ref; wire-contract A.3, door symmetry over
    /// duplicate block ids).
    #[test]
    fn a_duplicated_block_id_matches_every_carrier_in_document_order() {
        let raw = "# Tasks\n\n- first ^same-id\n\n- second ^same-id\n";
        let got = facts(raw);
        let matches = selector_matches(&got, &sel("^same-id"));
        assert_eq!(matches.len(), 2, "both carriers match");
        assert!(
            matches[0].span.0 < matches[1].span.0,
            "document order, never re-sorted"
        );
        assert!(
            resolve_selector(&got, &sel("^same-id")).is_none(),
            "the collapsed resolver refuses to pick one"
        );
        // A unique id still resolves to its one carrier.
        let unique = "# Tasks\n\n- only ^one-id\n";
        let got = facts(unique);
        assert_eq!(selector_matches(&got, &sel("^one-id")).len(), 1);
    }

    /// Duplicate headings: both occurrences get rows (dewey disambiguates),
    /// and selector resolution returns the FIRST — never last-wins.
    #[test]
    fn duplicate_headings_are_ambiguous_until_pinned() {
        let raw = "# Notes\n\nfirst\n\n# Notes\n\nsecond\n\n## Child\n\nchild body\n";
        let got = facts(raw);
        let ns: Vec<&str> = got.iter().map(|f| f.n.as_str()).collect();
        assert_eq!(ns, vec!["1", "2", "2.1"]);
        // A bare duplicate matches BOTH rows — the caller refuses with
        // candidates, never a silent first pick (wire-contract A.3).
        let matches = selector_matches(&got, &sel("Notes"));
        assert_eq!(
            matches.iter().map(|f| f.n.as_str()).collect::<Vec<_>>(),
            vec!["1", "2"],
            "both occurrences are candidates"
        );
        assert!(
            resolve_selector(&got, &sel("Notes")).is_none(),
            "an ambiguous selector does not resolve"
        );
        // subtree words: second + ## + Child + child + body
        assert_eq!(resolve_selector(&got, &sel("2")).map(|f| f.words), Some(5));
        // and the occurrence index addresses the second one exactly
        assert_eq!(
            resolve_selector(
                &got,
                &wire::ReadSel::Hpath {
                    hpath: vec![wire::HpathSeg {
                        h: "Notes".to_owned(),
                        n: Some(2)
                    }]
                }
            )
            .map(|f| f.n.as_str()),
            Some("2"),
            "`n` names the occurrence exactly"
        );
    }

    /// The two planes are disjoint — `toc_rows` is heading-only,
    /// `anchor_rows` is anchor-only — and containment still resolves across
    /// them, because it is absolute-byte arithmetic and never needed the
    /// interleaving.
    #[test]
    fn the_two_planes_are_disjoint_and_containment_still_crosses_them() {
        let raw = "# Tasks\n\n- top item ^t1\n\n## Sub\n\n- nested item ^n1\n\n# Notes\n\nbody\n";
        let got = facts(raw);
        let headings: Vec<String> = toc_rows(&got, ALL).iter().map(|f| addr(f)).collect();
        assert_eq!(
            headings,
            vec!["Tasks", "Tasks/Sub", "Notes"],
            "the heading plane carries every heading and NOTHING else"
        );
        assert!(
            toc_rows(&got, ALL).iter().all(|f| f.anchor.is_none()),
            "no anchor fact can reach a `toc` consumer"
        );
        let anchors: Vec<&str> = anchor_rows(&got, ALL)
            .iter()
            .filter_map(|f| f.anchor.as_deref())
            .collect();
        assert!(
            anchor_rows(&got, ALL).iter().all(|f| f.hpath.is_empty()),
            "an anchor row publishes NO heading address (U14) — it is addressed \
             through ReadSel::Anchor, never by disguising `^id` as one"
        );
        assert_eq!(
            anchors,
            vec!["t1", "n1"],
            "the anchor plane carries every addressable block anchor, in document order"
        );

        // `containingSectionTitles` (puttoc.go:86) across the two planes.
        let governing = |anchor: &str| -> Vec<String> {
            let block = anchor_rows(&got, ALL)
                .into_iter()
                .find(|f| f.anchor.as_deref() == Some(anchor))
                .expect("anchor row");
            toc_rows(&got, ALL)
                .into_iter()
                .filter(|h| h.span.0 <= block.span.0 && block.span.0 < h.span.1)
                .map(|h| h.title.clone())
                .collect()
        };
        assert_eq!(governing("t1"), vec!["Tasks".to_owned()]);
        assert_eq!(
            governing("n1"),
            vec!["Tasks".to_owned(), "Sub".to_owned()],
            "a nested block answers BOTH its ancestor sections"
        );
    }

    /// Frag scoping of anchor rows is the host's own byte containment: the
    /// scoped subtree's anchors ride, a sibling subtree's never do.
    #[test]
    fn anchor_rows_frag_scopes_anchors_by_byte_containment() {
        let raw = "# Tasks\n\n- item ^t1\n\n# Notes\n\n- other ^o1\n";
        let got = facts(raw);
        let scoped: Vec<&str> = anchor_rows(&got, &frag("Tasks"))
            .iter()
            .filter_map(|f| f.anchor.as_deref())
            .collect();
        assert_eq!(scoped, vec!["t1"]);
        let other: Vec<&str> = anchor_rows(&got, &frag("Notes"))
            .iter()
            .filter_map(|f| f.anchor.as_deref())
            .collect();
        assert_eq!(other, vec!["o1"]);
        assert!(
            anchor_rows(&got, &frag("Ghost")).is_empty(),
            "a frag naming nothing scopes nothing — the caller's refusal"
        );
    }

    /// An anchor above the first heading has no governing section; the
    /// whole-document anchor plane still carries it (the host must see the
    /// block to authorize a write to it).
    #[test]
    fn anchor_rows_carry_anchors_above_the_first_heading() {
        let raw = "- orphan item ^o1\n\n# Tasks\n\nbody\n";
        let got = facts(raw);
        let anchors: Vec<&str> = anchor_rows(&got, ALL)
            .iter()
            .filter_map(|f| f.anchor.as_deref())
            .collect();
        assert_eq!(anchors, vec!["o1"]);
        assert_eq!(
            toc_rows(&got, ALL)
                .iter()
                .map(|f| addr(f))
                .collect::<Vec<_>>(),
            vec!["Tasks"],
            "and the heading plane is unaffected by it"
        );
    }

    /// Three headings that `sanitize_heading` collapses onto one spelling keep
    /// three distinct published addresses, none carrying `n` — each raw text is
    /// unique among its siblings, so the collision existed only in the
    /// sanitized projection — and each resolves back to its own row.
    #[test]
    fn headings_that_sanitize_alike_keep_distinct_addresses_and_each_resolves_to_its_own_row() {
        let got = facts("# Scratch notes\n\na\n\n# Scratch-notes\n\nb\n\n# Scratch/notes\n\nc\n");
        assert_eq!(
            got.iter()
                .map(|f| sanitize_heading(&f.title))
                .collect::<Vec<_>>(),
            vec!["Scratch-notes", "Scratch-notes", "Scratch-notes"],
            "sanitize_heading is many-to-one — the projection that used to BE the address"
        );
        assert_eq!(
            got.iter().map(addr).collect::<Vec<_>>(),
            vec!["Scratch notes", "Scratch-notes", "Scratch/notes"],
            "and the published address distinguishes them byte-exactly, with no n invented"
        );
        for (i, f) in got.iter().enumerate() {
            let hit = resolve_selector(
                &got,
                &wire::ReadSel::Hpath {
                    hpath: f.hpath.clone(),
                },
            )
            .expect("a published address resolves");
            assert_eq!(
                hit.n, got[i].n,
                "row {i}: its own published address resolves to ITSELF, not to whichever \
                 row sanitizes the same way first"
            );
        }
    }

    /// A heading whose raw text contains `/` is addressable without ambiguity,
    /// because the address is an array and `/` is not a delimiter in it:
    /// `["Scratch", "notes"]` is a different address from `["Scratch/notes"]`,
    /// and any implementation joining or re-splitting on `/` merges them.
    #[test]
    fn a_slash_bearing_heading_is_addressable_and_never_collides_with_a_two_segment_path() {
        let got = facts("# Scratch/notes\n\nflat\n\n# Scratch\n\nx\n\n## notes\n\nnested\n");
        let flat = wire::ReadSel::Hpath {
            hpath: vec![wire::HpathSeg {
                h: "Scratch/notes".to_owned(),
                n: None,
            }],
        };
        let nested = wire::ReadSel::Hpath {
            hpath: vec![
                wire::HpathSeg {
                    h: "Scratch".to_owned(),
                    n: None,
                },
                wire::HpathSeg {
                    h: "notes".to_owned(),
                    n: None,
                },
            ],
        };
        let flat_hit = resolve_selector(&got, &flat).expect("the `/`-bearing heading resolves");
        let nested_hit = resolve_selector(&got, &nested).expect("the two-segment path resolves");
        assert_eq!(
            (flat_hit.n.as_str(), nested_hit.n.as_str()),
            ("1", "2.1"),
            "one segment containing `/` and two segments are DIFFERENT addresses"
        );
        assert_eq!(
            section_content(
                flat_hit,
                "# Scratch/notes\n\nflat\n\n# Scratch\n\nx\n\n## notes\n\nnested\n".as_bytes()
            ),
            b"\nflat\n\n".to_vec(),
            "and it serves its own bytes"
        );
    }

    /// `n` counts occurrences among same-text siblings under the same parent,
    /// the occurrence `model::resolve_hpath_node` counts. Two duplicate `# A`
    /// sections each hold a `## B`: the A segments are numbered and neither B
    /// is. An implementation counting identical full paths would number both Bs
    /// and send `A#2/B#2` to a parent holding one B.
    #[test]
    fn occurrence_index_counts_same_text_siblings_never_identical_full_paths() {
        let got = facts("# A\n\nx\n\n## B\n\ny\n\n# A\n\nz\n\n## B\n\nw\n");
        assert_eq!(
            got.iter().map(addr).collect::<Vec<_>>(),
            vec!["A#1", "A#1/B", "A#2", "A#2/B"],
            "the ambiguous ancestor is numbered; the unique child never is"
        );
    }

    /// A unique section publishes NO occurrence index, so the address starts
    /// refusing if a duplicate ever appears — rather than silently retargeting.
    /// A non-duplicate sibling sitting BEFORE the duplicates proves `n` is not
    /// position among all sibling sections (which would be 2 and 3 here).
    #[test]
    fn occurrence_index_rides_only_where_ambiguous_and_ignores_sibling_position() {
        let got = facts("# P\n\n## Alpha\n\na\n\n## N\n\nb\n\n## N\n\nc\n");
        assert_eq!(
            got.iter().map(addr).collect::<Vec<_>>(),
            vec!["P", "P/Alpha", "P/N#1", "P/N#2"]
        );
    }

    /// Anchor rows carry no heading address: their put grammar is
    /// `{"anchor":id}`, and the id rides the anchor plane un-sanitized.
    #[test]
    fn anchor_rows_carry_no_heading_address() {
        let got = facts("# H\n\n- plain item ^p1\n");
        let anchor = resolve_selector(&got, &sel("^p1")).expect("anchor row");
        assert!(anchor.hpath.is_empty());
        assert_eq!(
            addr(resolve_selector(&got, &sel("H")).expect("heading")),
            "H"
        );
    }

    /// Each grammar stays in its own plane: a heading whose raw text spells a
    /// dewey ordinal, or opens with `^`, is reachable only through the heading
    /// arm and shadows neither the ordinal nor the anchor plane.
    #[test]
    fn the_planes_do_not_shadow_each_other_across_grammars() {
        let got = facts("# 1.2\n\na\n\n# Real\n\nb\n\n- item ^1.2\n");
        let heading = resolve_selector(
            &got,
            &wire::ReadSel::Hpath {
                hpath: vec![wire::HpathSeg {
                    h: "1.2".to_owned(),
                    n: None,
                }],
            },
        )
        .expect("the heading named `1.2` is addressable AS a heading");
        assert_eq!(heading.title, "1.2");
        let dewey = resolve_selector(&got, &wire::ReadSel::Dewey { n: "2".to_owned() })
            .expect("the dewey ordinal addresses the SECOND heading row");
        assert_eq!(
            dewey.title, "Real",
            "the ordinal plane is positional and the heading named `1.2` never shadows it"
        );
    }

    /// Frag scoping: the section itself + descendants by segment prefix;
    /// sibling prefixes ("Note" vs "Notes") never match.
    #[test]
    fn toc_rows_frag_subtree() {
        let raw = "# Notes\n\nx\n\n## Deep\n\ny\n\n# Notes2\n\nz\n";
        let got = facts(raw);
        let scoped: Vec<String> = toc_rows(&got, &frag("Notes"))
            .iter()
            .map(|f| addr(f))
            .collect();
        assert_eq!(scoped, vec!["Notes", "Notes/Deep"]);
        assert!(toc_rows(&got, &frag("Ghost")).is_empty());
    }
}
