//! Read a page's DECLARED `inputs:` manifest — the frontmatter block that any
//! editor writes (d2 §2.5; the 23-07 `check:` ruling). Pin RESOLVES this and
//! writes the `^inputs` lock; it never edits the manifest.
//!
//! The manifest is a frontmatter block sequence, read through the model
//! `fm_key` grain (U2.11 — the whole block-sequence value, never the key line
//! alone). Each item is one of:
//!
//! - a block mapping — `- ref: "B#Anchor-law"` then indented `claim:` / `check:`
//!   continuation lines (the 23-07 ruling's declared form);
//! - a flow mapping — `- {ref: 'B#Anchor-law', claim: 'why', check: 'A#^c'}`;
//! - a scalar — `- "results/round2/design-1.md@rev"` / `- "[[substrate]]"` /
//!   `- path#Heading` (the ref alone; a `@rev` pin hint is stripped — pin
//!   recomputes the rev, never trusts a declared one).

use model::{Document, Ref, resolve};

/// One declared manifest item: the ref plus the optional `claim:` free-text and
/// the optional engine-read `check:` predicate selector (23-07 ruling).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredItem {
    /// The ref as declared, `@rev` pin-hint stripped (`B#Anchor-law`,
    /// `[[substrate]]`, `session#seq-N`, `notes/plan.md`).
    pub declared_ref: String,
    /// The free-text assertion of WHY the ref is drawn from (engine-ignored
    /// passenger, carried verbatim into the lock).
    pub claim: Option<String>,
    /// The selector of a `def check_claim(t)` predicate (engine-read; a false
    /// claim refuses the whole pin — 23-07 ruling).
    pub check: Option<String>,
}

/// Read the declared `inputs:` manifest from a page's frontmatter. `None` when
/// the page declares no `inputs:` key (nothing to pin); an empty block yields an
/// empty list.
#[must_use]
pub fn read(doc: &Document) -> Option<Vec<DeclaredItem>> {
    let target = resolve(doc, &Ref::FmKey("inputs".to_string())).ok()?;
    let value = doc.raw.get(target.span.clone())?;
    Some(parse_value(value))
}

/// Parse the resolved `inputs:` value (the whole block-sequence, per the U2.11
/// grain) into declared items. The first line is the `inputs:` key; item lines
/// begin with `- ` at the sequence indent, continuation lines are deeper.
fn parse_value(value: &str) -> Vec<DeclaredItem> {
    let mut items = Vec::new();
    let mut current: Option<DeclaredItem> = None;
    // The dash column of the FIRST item fixes the sequence indent; a deeper
    // line is a continuation of the current item, a shallower line ends it.
    let mut dash_col: Option<usize> = None;

    for line in value.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim_start();

        // The `inputs:` key line (or any bare column-0 key) is not an item.
        if indent == 0 && !trimmed.starts_with('-') {
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix('-') {
            // A new sequence item at (or establishing) the sequence indent.
            if dash_col.is_none_or(|c| indent <= c) {
                dash_col = Some(indent);
                if let Some(item) = current.take() {
                    items.push(item);
                }
                current = Some(parse_item_head(rest.trim()));
                continue;
            }
        }

        // A continuation line of the current block-mapping item (deeper indent).
        if let Some(item) = current.as_mut()
            && dash_col.is_some_and(|c| indent > c)
        {
            apply_field(item, trimmed);
        }
    }
    if let Some(item) = current.take() {
        items.push(item);
    }
    items
}

/// Parse the head of a sequence item — the text after `- `. A `{…}` flow
/// mapping, a `key: value` first field of a block mapping, or a scalar ref.
fn parse_item_head(head: &str) -> DeclaredItem {
    if let Some(inner) = head.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
        return parse_flow_mapping(inner);
    }
    // A block mapping's first field is `ref: …` (or occasionally `claim:`/
    // `check:` first); a scalar is anything else.
    if head.starts_with("ref:") || head.starts_with("claim:") || head.starts_with("check:") {
        let mut item = DeclaredItem {
            declared_ref: String::new(),
            claim: None,
            check: None,
        };
        apply_field(&mut item, head);
        item
    } else {
        DeclaredItem {
            declared_ref: normalize_ref(&unquote(head)),
            claim: None,
            check: None,
        }
    }
}

/// Apply one `key: value` field to a block-mapping item (`ref`/`claim`/`check`;
/// unknown keys are ignored — they are not engine-load-bearing here).
fn apply_field(item: &mut DeclaredItem, field: &str) {
    let Some((key, value)) = field.split_once(':') else {
        return;
    };
    let value = unquote(value.trim());
    match key.trim() {
        "ref" => item.declared_ref = normalize_ref(&value),
        "claim" => item.claim = Some(value),
        "check" => item.check = Some(value),
        _ => {}
    }
}

/// Parse a flow mapping body (`ref: 'X', claim: 'Y', check: 'Z'`) into an item.
fn parse_flow_mapping(inner: &str) -> DeclaredItem {
    let mut item = DeclaredItem {
        declared_ref: String::new(),
        claim: None,
        check: None,
    };
    for field in split_top_level_commas(inner) {
        apply_field(&mut item, field.trim());
    }
    item
}

/// Normalize a declared ref: strip a `[[wikilink]]` wrapper to its inner text
/// and drop a trailing `@<rev>` pin hint (pin recomputes the rev). A wiki ref
/// stays as its inner text — the resolver renders it grey when the corpus does
/// not hold it (never a refusal).
fn normalize_ref(raw: &str) -> String {
    let s = raw.trim();
    let s = s
        .strip_prefix("[[")
        .and_then(|s| s.strip_suffix("]]"))
        .unwrap_or(s);
    // Drop a trailing `@<hex>` (a go-world declared rev): the ref is the address.
    if let Some((head, tail)) = s.rsplit_once('@')
        && !tail.is_empty()
        && tail.chars().all(|c| c.is_ascii_hexdigit())
    {
        return head.to_string();
    }
    s.to_string()
}

/// Strip one layer of surrounding single or double quotes.
fn unquote(value: &str) -> String {
    let v = value.trim();
    let bytes = v.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        if (first == b'\'' || first == b'"') && bytes[bytes.len() - 1] == first {
            return v[1..v.len() - 1].to_string();
        }
    }
    v.to_string()
}

/// Split a flow-mapping body on TOP-LEVEL commas only — a comma inside quotes
/// stays with its field.
fn split_top_level_commas(inner: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for ch in inner.chars() {
        match quote {
            Some(q) => {
                cur.push(ch);
                if ch == q {
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

    #[test]
    fn reads_block_mapping_items_with_claim_and_check() {
        let d = doc(
            "---\ntitle: A\ninputs:\n  - ref: \"B#Anchor-law\"\n    claim: \"freshness law\"\n    check: \"A#^claim-anchor\"\n  - ref: \"C#Body\"\n---\n\n# A\n",
        );
        let items = read(&d).expect("inputs present");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].declared_ref, "B#Anchor-law");
        assert_eq!(items[0].claim.as_deref(), Some("freshness law"));
        assert_eq!(items[0].check.as_deref(), Some("A#^claim-anchor"));
        assert_eq!(items[1].declared_ref, "C#Body");
        assert!(items[1].claim.is_none());
    }

    #[test]
    fn reads_scalar_items_stripping_rev_and_wikilink() {
        let d = doc(
            "---\ninputs:\n  - \"results/round2/design-1.md@d8536666b42dc8fd\"\n  - \"[[substrate]]\"\n  - notes/plan.md#Goals\n---\n\n# A\n",
        );
        let items = read(&d).expect("inputs present");
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].declared_ref, "results/round2/design-1.md");
        assert_eq!(items[1].declared_ref, "substrate");
        assert_eq!(items[2].declared_ref, "notes/plan.md#Goals");
    }

    #[test]
    fn reads_flow_mapping_item() {
        let d = doc("---\ninputs:\n  - {ref: 'B#Law', claim: 'why', check: 'A#^c'}\n---\n\n# A\n");
        let items = read(&d).expect("inputs present");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].declared_ref, "B#Law");
        assert_eq!(items[0].claim.as_deref(), Some("why"));
        assert_eq!(items[0].check.as_deref(), Some("A#^c"));
    }

    #[test]
    fn no_inputs_key_is_none() {
        let d = doc("---\ntitle: A\n---\n\n# A\n");
        assert!(read(&d).is_none());
    }
}
