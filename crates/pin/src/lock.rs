//! The `^inputs` lock — the fenced ` ```yaml ^inputs ` block a pin writes
//! (d2 §2.5, passenger registry amendment). Parse an existing block (byte-exact,
//! for the merge law), render a fresh one, and merge under the union-by-canonical
//! -ref / existing-items-byte-preserved law carried from Go.
//!
//! This is the SAME block U2.9's `view::read_face` projects into `input_lock`
//! (fence-info `^inputs`, `items:` flow mappings) — pin writes it, U2.9 reads
//! it, and pin's own edge projection composes over U2.9's parse (never a fork).
//!
//! Item grammar (amendment): engine core `{ref, to, rev, rev_class}` + the
//! `claim` passenger + the engine-read `check` / `check_rev` fields. A grey
//! (declared-only) item carries no `rev`.

use model::{ByteSpan, Document, Node, NodeKind};

/// The `hash-algo` header of an engine-written lock: the content-class rev is the
/// node-rev (`blake3-256(span bytes)[:16]`, contract §1) — the same rev U2.2
/// `classify_edge` and U2.9 `board_drift` compare against live nodes. One owner
/// for the label: [`model::NODE_REV_ALGO`], so the reader's `superseded-algo`
/// grey (U3.4) can never drift from what the writer stamps here.
pub const HASH_ALGO: &str = model::NODE_REV_ALGO;

/// One lock item's engine-visible fields (the merge + render surface). A grey
/// declared-only item has `rev: None` (and no `rev_class`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// The `ref` — the declared selector, verbatim.
    pub declared_ref: String,
    /// The `to` — the resolved address `path#(hpath | ^block-id)` (== `ref` for
    /// a grey declared-only item the engine could not resolve).
    pub to: String,
    /// The `rev` — the pinned node-rev; `None` = declared-only (grey).
    pub rev: Option<String>,
    /// The `rev_class` — `content` | `object`; `None` for grey.
    pub rev_class: Option<String>,
    /// The `claim` passenger — engine-ignored free text, carried verbatim.
    pub claim: Option<String>,
    /// The engine-read `check` predicate selector (23-07 ruling).
    pub check: Option<String>,
    /// The `check_rev` — the rev the predicate text was pinned at.
    pub check_rev: Option<String>,
}

/// The existing `^inputs` lock parsed from a page: the block's byte span (for the
/// splice) and its items each paired with the RAW line bytes (for byte
/// preservation under the merge law). `span` is `None` when the page has no lock.
#[derive(Debug, Clone, Default)]
pub struct ExistingLock {
    /// The fence-to-fence byte span of the `^inputs` block (`None` = absent).
    pub span: Option<ByteSpan>,
    /// Each existing item + its exact rendered line (indent included).
    pub items: Vec<(Item, String)>,
}

/// Find and parse the page's `^inputs` lock block (the fenced code block whose
/// info string carries the `^inputs` anchor — the same marker U2.9 reads).
#[must_use]
pub fn find_existing(doc: &Document) -> ExistingLock {
    let mut found: Option<(ByteSpan, Vec<(Item, String)>)> = None;
    collect_block(&doc.root, &doc.raw, &mut found);
    match found {
        Some((span, items)) => ExistingLock {
            span: Some(span),
            items,
        },
        None => ExistingLock::default(),
    }
}

/// Walk the tree for the first `^inputs` code block, recording its span + items.
fn collect_block(node: &Node, raw: &str, out: &mut Option<(ByteSpan, Vec<(Item, String)>)>) {
    if out.is_some() {
        return;
    }
    if let NodeKind::CodeBlock { lang, .. } = &node.kind
        && is_inputs_lang(lang)
        && let Some(body) = raw.get(node.span.clone())
    {
        *out = Some((node.span.clone(), parse_items(body)));
        return;
    }
    for child in &node.children {
        collect_block(child, raw, out);
    }
}

/// Whether a fence info string addresses the `^inputs` lock (its whitespace
/// tokens include `^inputs`) — the marker U2.9's `is_inputs_lang` also uses.
fn is_inputs_lang(lang: &str) -> bool {
    lang.split_whitespace().any(|tok| tok == "^inputs")
}

/// Parse each `- {…}` flow-mapping line of a lock block body into an item + its
/// raw line. Fence and header lines (`hash-algo:`, `items:`) are skipped.
fn parse_items(body: &str) -> Vec<(Item, String)> {
    let mut items = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix('-') else {
            continue;
        };
        let Some(inner) = rest
            .trim()
            .strip_prefix('{')
            .and_then(|s| s.strip_suffix('}'))
        else {
            continue;
        };
        if let Some(item) = item_from_flow(inner) {
            items.push((item, line.to_string()));
        }
    }
    items
}

/// Parse one flow-mapping body into an [`Item`] (a row without a `ref` is not an
/// item). `to` defaults to `ref`.
fn item_from_flow(inner: &str) -> Option<Item> {
    let mut declared_ref = None;
    let mut to = None;
    let mut rev = None;
    let mut rev_class = None;
    let mut claim = None;
    let mut check = None;
    let mut check_rev = None;
    for field in split_top_level_commas(inner) {
        let Some((key, value)) = field.split_once(':') else {
            continue;
        };
        let value = unquote(value.trim());
        match key.trim() {
            "ref" => declared_ref = Some(value),
            "to" => to = Some(value),
            "rev" => rev = Some(value),
            "rev_class" => rev_class = Some(value),
            "claim" => claim = Some(value),
            "check" => check = Some(value),
            "check_rev" => check_rev = Some(value),
            _ => {}
        }
    }
    let declared_ref = declared_ref?;
    let to = to.unwrap_or_else(|| declared_ref.clone());
    Some(Item {
        declared_ref,
        to,
        rev,
        rev_class,
        claim,
        check,
        check_rev,
    })
}

/// Render one lock item as its `  - {…}` line (two-space sequence indent, single
/// -quoted scalars). Absent fields are omitted, so a grey item carries no `rev`.
#[must_use]
pub fn item_line(item: &Item) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = write!(
        out,
        "  - {{ref: {}, to: {}",
        q(&item.declared_ref),
        q(&item.to)
    );
    if let Some(rev) = &item.rev {
        let _ = write!(out, ", rev: {}", q(rev));
    }
    if let Some(rc) = &item.rev_class {
        let _ = write!(out, ", rev_class: {rc}");
    }
    if let Some(claim) = &item.claim {
        let _ = write!(out, ", claim: {}", q(claim));
    }
    if let Some(check) = &item.check {
        let _ = write!(out, ", check: {}", q(check));
    }
    if let Some(check_rev) = &item.check_rev {
        let _ = write!(out, ", check_rev: {}", q(check_rev));
    }
    out.push('}');
    out
}

/// Render the FULL `^inputs` block from item lines (fence to fence, no trailing
/// newline — the splice caller owns terminators around it).
#[must_use]
pub fn render_block(item_lines: &[String]) -> String {
    let mut out = String::from("```yaml ^inputs\n");
    out.push_str("hash-algo: ");
    out.push_str(HASH_ALGO);
    out.push_str("\nitems:\n");
    for line in item_lines {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str("```");
    out
}

/// The merge law (carried from Go): union by canonical `ref`, existing items
/// byte-preserved. For each DECLARED fresh item (manifest order): if an existing
/// item with the same `ref` is byte-for-byte semantically equal, keep its EXACT
/// line (byte preservation → idempotency); otherwise render the fresh line (a
/// drift refresh, or a new pin). Existing items whose `ref` is NOT re-declared
/// are appended verbatim (union — pin never silently drops a lock item).
#[must_use]
pub fn merge(existing: &ExistingLock, fresh: &[Item]) -> Vec<String> {
    let mut lines = Vec::new();
    let mut used: Vec<bool> = vec![false; existing.items.len()];

    for item in fresh {
        // An existing item with the same ref + identical semantic fields is
        // preserved verbatim; a changed one is re-rendered fresh.
        let preserved = existing
            .items
            .iter()
            .enumerate()
            .find(|(i, (e, _))| !used[*i] && e.declared_ref == item.declared_ref && e == item);
        if let Some((i, (_, raw))) = preserved {
            used[i] = true;
            lines.push(raw.clone());
        } else {
            // Mark any same-ref existing item consumed (this fresh line replaces
            // it), so it is not also appended below.
            if let Some((i, _)) = existing
                .items
                .iter()
                .enumerate()
                .find(|(i, (e, _))| !used[*i] && e.declared_ref == item.declared_ref)
            {
                used[i] = true;
            }
            lines.push(item_line(item));
        }
    }
    // Existing items no longer declared: preserved (union).
    for (i, (_, raw)) in existing.items.iter().enumerate() {
        if !used[i] {
            lines.push(raw.clone());
        }
    }
    lines
}

/// The node-rev of a byte slice — `blake3-256[:16]`, contract §1 (matches
/// model's node-rev law; used for the journal block-rev transition).
#[must_use]
pub fn node_rev(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().as_str()[..16].to_string()
}

/// Single-quote a scalar for the flow mapping (a `'` inside is doubled, YAML's
/// single-quote escape).
fn q(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Strip one layer of surrounding single or double quotes.
fn unquote(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        if (first == b'\'' || first == b'"') && bytes[bytes.len() - 1] == first {
            return value[1..value.len() - 1].replace("''", "'");
        }
    }
    value.to_string()
}

/// Split a flow-mapping body on TOP-LEVEL commas only (a comma inside quotes
/// stays with its field).
fn split_top_level_commas(inner: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for ch in inner.chars() {
        match quote {
            Some(qc) => {
                cur.push(ch);
                if ch == qc {
                    quote = None;
                }
            }
            None => match ch {
                '\'' | '"' => {
                    quote = Some(ch);
                    cur.push(ch);
                }
                ',' => fields.push(std::mem::take(&mut cur)),
                _ => cur.push(ch),
            },
        }
    }
    if !cur.trim().is_empty() {
        fields.push(cur);
    }
    fields
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(raw: &str) -> Document {
        model::build(raw.to_string(), syntax::parse(raw))
    }

    fn content_item(r: &str, rev: &str) -> Item {
        Item {
            declared_ref: r.to_string(),
            to: r.to_string(),
            rev: Some(rev.to_string()),
            rev_class: Some("content".to_string()),
            claim: None,
            check: None,
            check_rev: None,
        }
    }

    #[test]
    fn render_then_find_round_trips() {
        let block = render_block(&[item_line(&content_item("B#Law", "deadbeefdeadbeef"))]);
        let raw = format!("# A\n\n{block}\n");
        let d = doc(&raw);
        let existing = find_existing(&d);
        assert!(existing.span.is_some(), "the block is found");
        assert_eq!(existing.items.len(), 1);
        let (item, _) = &existing.items[0];
        assert_eq!(item.declared_ref, "B#Law");
        assert_eq!(item.rev.as_deref(), Some("deadbeefdeadbeef"));
        assert_eq!(item.rev_class.as_deref(), Some("content"));
    }

    #[test]
    fn merge_preserves_unchanged_and_refreshes_drift() {
        let existing_block = render_block(&[
            item_line(&content_item("A#One", "1111111111111111")),
            item_line(&content_item("B#Two", "2222222222222222")),
        ]);
        let d = doc(&format!("# X\n\n{existing_block}\n"));
        let existing = find_existing(&d);

        // Re-pin: A unchanged, B drifted to a new rev.
        let fresh = vec![
            content_item("A#One", "1111111111111111"),
            content_item("B#Two", "9999999999999999"),
        ];
        let lines = merge(&existing, &fresh);
        assert_eq!(lines.len(), 2);
        // A preserved byte-for-byte (its exact existing line).
        assert_eq!(lines[0], existing.items[0].1);
        // B refreshed to the new rev.
        assert!(lines[1].contains("9999999999999999"));
    }

    #[test]
    fn merge_unchanged_is_byte_identical() {
        let block = render_block(&[item_line(&content_item("A#One", "1111111111111111"))]);
        let d = doc(&format!("# X\n\n{block}\n"));
        let existing = find_existing(&d);
        let fresh = vec![content_item("A#One", "1111111111111111")];
        let lines = merge(&existing, &fresh);
        let re = render_block(&lines);
        assert_eq!(
            re, block,
            "an unchanged re-pin renders byte-identical bytes"
        );
    }

    #[test]
    fn merge_keeps_undeclared_existing_items() {
        let block = render_block(&[
            item_line(&content_item("A#One", "1111111111111111")),
            item_line(&content_item("B#Two", "2222222222222222")),
        ]);
        let d = doc(&format!("# X\n\n{block}\n"));
        let existing = find_existing(&d);
        // Only A is re-declared; B stays (union).
        let fresh = vec![content_item("A#One", "1111111111111111")];
        let lines = merge(&existing, &fresh);
        assert_eq!(lines.len(), 2, "B is preserved even though undeclared");
        assert!(lines[1].contains("B#Two"));
    }
}
