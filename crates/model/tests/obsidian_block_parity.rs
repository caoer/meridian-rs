//! Obsidian block-reference parity for anchor HOST spans (F-R4 ruling,
//! 2026-08-13): `^id` addresses the block Obsidian's own metadata cache
//! assigns it — any block kind, not the marker's line alone.
//!
//! Ground truth: Obsidian 1.13.6 `app.metadataCache.getCache(..).blocks`,
//! probed live 2026-08-13 (session 12-04-f2-mrd-integration, worker
//! ad0d748f). The observed law:
//!
//! - **Tail id** (content precedes the marker on its line): the host is the
//!   enclosing leaf block — the whole multi-line paragraph, the whole callout,
//!   the whole table (a tail in the last row), the single list/task ITEM line
//!   (item grain, never the list), the heading line.
//! - **Own-line id**: attaches to the nearest preceding block, skipping blank
//!   lines and other marker-only lines; the marker line stays OUTSIDE the
//!   host span. Directly below a paragraph line or a list item (no blank
//!   between) the marker instead JOINS that block, marker line included —
//!   lazy continuation. A whole contiguous list is one attachment target
//!   (blank-separated `^id` below a list hosts the LIST); headings are
//!   attachable (`^id` below a heading hosts the heading line).
//! - **Frontmatter** never hosts an anchor (a caret there is literal YAML).
//!
//! **Embed parity for HEADING-hosted ids (probed 2026-08-15, Obsidian
//! 1.12.7, session 12-04-f2-mrd-integration worker 2feb1670, card
//! anchors-line-host-hint):** `![[note#^id]]` where `^id` attaches to a
//! heading renders the heading LINE alone — not the heading's section
//! content. Control in the same render pass: the heading embed of the same
//! section served heading + full content. So a face serving the heading
//! line for a block read of such an id matches the app's own embed
//! rendering; the section content stays the HEADING selector's answer.
//! (Burr worth knowing: Obsidian's heading-link lane keeps the raw heading
//! text marker included — `#Core instincts ^hostid` resolves, `#Core
//! instincts` does not. The engine's heading titles exclude the trailing
//! marker; that divergence is the link-lane's, not the block plane's.)
//!
//! Documented DELIBERATE supersets (engine serves where Obsidian vanishes the
//! id; nothing Obsidian serves is refused):
//! - a mid-paragraph tail id hosts its paragraph run (Obsidian: no id),
//! - an own-line id with no preceding block keeps its own line (Obsidian: no
//!   id),
//! - every id on a block resolves (Obsidian keeps only the LAST id of a
//!   block; earlier ones vanish).
//!
//! Rationale: the anchor inventory is the scanner's (norm-v2 anchor grammar
//! untouched); vanishing ids from the model would break the lossless node
//! inventory. The FACE therefore refuses no ref Obsidian resolves.

use model::{Ref, resolve};

fn doc(raw: &str) -> model::Document {
    model::build(raw.to_string(), syntax::parse(raw))
}

/// The resolved host span for `^id` — the block-leaf the anchor keys.
fn host(raw: &str, id: &str) -> std::ops::Range<usize> {
    let d = doc(raw);
    resolve(&d, &Ref::anchor(id).unwrap())
        .unwrap_or_else(|e| panic!("^{id} must resolve: {e:?}"))
        .span
}

/// Byte range of `needle`'s single occurrence in `raw`.
fn span_of(raw: &str, needle: &str) -> std::ops::Range<usize> {
    let start = raw.find(needle).expect("needle present");
    assert_eq!(raw.rfind(needle), Some(start), "needle must be unique");
    start..start + needle.len()
}

// --- tail ids ---

/// Single-line paragraph tail: the paragraph's one line (unchanged grain —
/// the F-R4 evidence case `LLM_WIKI.md ^load-skill` is this shape).
#[test]
fn tail_on_single_line_paragraph_hosts_the_line() {
    let raw = "# T\n\npara tail ^p1\n\nnext para\n";
    assert_eq!(host(raw, "p1"), span_of(raw, "para tail ^p1"));
}

/// Tail on the LAST line of a multi-line paragraph: the WHOLE paragraph
/// (Obsidian `ptail` probe: block spans both lines).
#[test]
fn tail_on_multiline_paragraph_hosts_the_whole_paragraph() {
    let raw = "# T\n\nfirst line of para\nsecond line ^ptail\n\nafter\n";
    assert_eq!(
        host(raw, "ptail"),
        span_of(raw, "first line of para\nsecond line ^ptail")
    );
}

/// Mid-paragraph tail: hosts the paragraph run — documented superset
/// (Obsidian drops the id entirely; the engine's inventory is the scanner's).
#[test]
fn tail_mid_paragraph_hosts_the_run_documented_superset() {
    let raw = "# T\n\nfirst line ^midtail\ncontinuation line\n\nafter\n";
    assert_eq!(
        host(raw, "midtail"),
        span_of(raw, "first line ^midtail\ncontinuation line")
    );
}

/// List-item tail: the ITEM line alone — item grain, never the list
/// (Obsidian `lastitem` probe; §6.3 worked `^r-000042` unchanged).
#[test]
fn tail_on_list_item_hosts_the_item_line() {
    let raw = "# T\n\n- item a\n- item b ^lastitem\n- item c\n";
    assert_eq!(host(raw, "lastitem"), span_of(raw, "- item b ^lastitem"));
}

/// Task tail: the task's item line (task grain = item grain).
#[test]
fn tail_on_task_hosts_the_item_line() {
    let raw = "# T\n\n- [ ] do the thing ^t1\n- [ ] other\n";
    assert_eq!(host(raw, "t1"), span_of(raw, "- [ ] do the thing ^t1"));
}

/// Tail on the last callout line: the WHOLE callout block (Obsidian `cotail`
/// probe: both `>` lines).
#[test]
fn tail_on_last_callout_line_hosts_the_whole_callout() {
    let raw = "# T\n\n> [!note] callout first\n> callout last ^cotail\n\nafter\n";
    assert_eq!(
        host(raw, "cotail"),
        span_of(raw, "> [!note] callout first\n> callout last ^cotail")
    );
}

/// Tail in the last table row: the WHOLE table (Obsidian `rowtail` probe).
#[test]
fn tail_in_last_table_row_hosts_the_whole_table() {
    let raw = "# T\n\n| x |\n|---|\n| 1 | ^rowtail\n\nafter\n";
    assert_eq!(
        host(raw, "rowtail"),
        span_of(raw, "| x |\n|---|\n| 1 | ^rowtail")
    );
}

/// Heading-line tail: the heading line (Obsidian `hh` probe).
#[test]
fn tail_on_heading_line_hosts_the_heading_line() {
    let raw = "# T\n\n## H2 ^hh\n\nbody\n";
    assert_eq!(host(raw, "hh"), span_of(raw, "## H2 ^hh"));
}

// --- own-line ids ---

/// Own-line directly below a table (no blank): the TABLE. One line of
/// engine grain: pulldown's table block swallows the directly-adjacent
/// marker line as a row, so the host span (= the engine's own Table node)
/// carries it — Obsidian's cache stops at the last real row. The served
/// content strips the marker either way; the blank-separated form below
/// matches Obsidian byte-exactly.
#[test]
fn own_line_below_table_hosts_the_table() {
    let raw = "# T\n\n| a | b |\n|---|---|\n| 1 | 2 |\n^tbl1\n\nafter\n";
    assert_eq!(
        host(raw, "tbl1"),
        span_of(raw, "| a | b |\n|---|---|\n| 1 | 2 |\n^tbl1")
    );
}

/// Own-line after a table with a BLANK line between: still the table
/// (attachment skips blanks; Obsidian `solo`-on-table probe).
#[test]
fn own_line_after_table_blank_separated_hosts_the_table() {
    let raw = "# T\n\n| c | d |\n|---|---|\n| 3 | 4 |\n\n^tbl2\n\nafter\n";
    assert_eq!(
        host(raw, "tbl2"),
        span_of(raw, "| c | d |\n|---|---|\n| 3 | 4 |")
    );
}

/// Own-line after a fence with a blank between: the FENCE (Obsidian
/// `fence1` probe — and the W-2 fixture-B `^check` shape, now addressable
/// under the F-R4 ruling).
#[test]
fn own_line_after_fence_hosts_the_fence() {
    let raw = "# T\n\n```bash\ncode line\n```\n\n^fence1\n\nafter\n";
    assert_eq!(host(raw, "fence1"), span_of(raw, "```bash\ncode line\n```"));
}

/// Own-line directly below a paragraph line (no blank): lazy continuation —
/// the paragraph INCLUDING the marker line (Obsidian `directbelow` probe).
#[test]
fn own_line_directly_below_paragraph_joins_it() {
    let raw = "# T\n\npara one\n^directbelow\n\npara two\n";
    assert_eq!(
        host(raw, "directbelow"),
        span_of(raw, "para one\n^directbelow")
    );
}

/// Own-line after a paragraph with a blank between: the paragraph, marker
/// EXCLUDED (Obsidian `gap2` / `afterpara` probes).
#[test]
fn own_line_blank_separated_after_paragraph_hosts_it() {
    let raw = "# T\n\nbody after heading\n\n^afterpara\n\nlast para\n";
    assert_eq!(host(raw, "afterpara"), span_of(raw, "body after heading"));
}

/// Own-line directly below a heading: the heading LINE (Obsidian
/// `afterheading` probe — a heading cannot lazily continue).
#[test]
fn own_line_directly_below_heading_hosts_the_heading_line() {
    let raw = "# T\n\n### C\n^afterheading\n\npara\n";
    assert_eq!(host(raw, "afterheading"), span_of(raw, "### C"));
}

/// Own-line after a heading with a blank between: the heading line
/// (Obsidian `orphan1` probe — `## A`, blank, `^orphan1`).
#[test]
fn own_line_after_heading_blank_separated_hosts_the_heading_line() {
    let raw = "## A\n\n^orphan1\n\npara one\n";
    assert_eq!(host(raw, "orphan1"), span_of(raw, "## A"));
}

/// Own-line after a whole list, blank-separated: the WHOLE contiguous list
/// (Obsidian `afterlist` probe: both items).
#[test]
fn own_line_after_list_hosts_the_whole_list() {
    let raw = "# T\n\n- alpha\n- beta\n\n^afterlist\n\npara\n";
    assert_eq!(host(raw, "afterlist"), span_of(raw, "- alpha\n- beta"));
}

/// Own-line directly below the last list item (no blank): joins THAT ITEM —
/// item line plus marker line, never the whole list (Obsidian `itemjoin`
/// probe).
#[test]
fn own_line_directly_below_list_item_joins_the_item() {
    let raw = "# T\n\n- gamma\n- delta\n^itemjoin\n\npara\n";
    assert_eq!(host(raw, "itemjoin"), span_of(raw, "- delta\n^itemjoin"));
}

/// Own-line after a plain (non-callout) blockquote: the quote lines — the
/// paragraph-run mechanics cover quotes, matching Obsidian's `quoteattach`
/// span even though the model mints no quote node.
#[test]
fn own_line_after_plain_blockquote_hosts_the_quote_lines() {
    let raw = "# T\n\n> multi quote a\n> multi quote b\n\n^quoteattach\n\npara\n";
    assert_eq!(
        host(raw, "quoteattach"),
        span_of(raw, "> multi quote a\n> multi quote b")
    );
}

/// Nested-item tail: the child item's line alone (Obsidian `nested` probe).
#[test]
fn tail_on_nested_item_hosts_the_child_line() {
    let raw = "# T\n\n- parent item\n  - child item ^nested\n\npara\n";
    assert_eq!(host(raw, "nested"), span_of(raw, "  - child item ^nested"));
}

// --- documented supersets & exclusions ---

/// Own-line at document start (no preceding block): keeps its own line —
/// documented superset (Obsidian drops the id; `atstart` probe).
#[test]
fn own_line_at_document_start_keeps_its_line() {
    let raw = "^atstart\n\npara\n";
    assert_eq!(host(raw, "atstart"), span_of(raw, "^atstart"));
}

/// Two own-line ids after one block: BOTH resolve to that block — documented
/// superset (Obsidian keeps only the last; `tbl2`/`solo` probe). Marker-only
/// lines are transparent to attachment.
#[test]
fn stacked_own_line_ids_all_host_the_block_documented_superset() {
    let raw = "# T\n\n| c | d |\n|---|---|\n| 3 | 4 |\n\n^first\n\n^second\n\npara\n";
    let table = span_of(raw, "| c | d |\n|---|---|\n| 3 | 4 |");
    assert_eq!(host(raw, "first"), table.clone());
    assert_eq!(host(raw, "second"), table);
}

/// A frontmatter caret is literal YAML: the anchor keeps line grain and its
/// host stays the frontmatter (off every face plane; Obsidian mints no id).
#[test]
fn frontmatter_caret_keeps_line_grain() {
    let raw = "---\nfmkey: x ^fm1\n---\n# T\n\npara\n";
    assert_eq!(host(raw, "fm1"), span_of(raw, "fmkey: x ^fm1"));
}
