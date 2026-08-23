//! Page-wide block enumeration — what `mode:"load"` addresses (hook-support
//! design § 2.2 *Evaluation — load*, step 1).
//!
//! The shipped run plane never needed this. Its discovery is driven entirely
//! by frontmatter: [`crate::address::declared`] reads the `task.<name>` keys,
//! [`crate::address::resolve_task`] resolves ONE of them to its anchor, and
//! [`crate::fence::classify`] classifies the span someone else already found.
//! A load asks the opposite question — *what does this page carry?* — so the
//! walk starts at the anchors and ends at the fences.
//!
//! Nothing here re-implements addressing. The anchor list is the model's
//! ([`model::selector::live_anchors`]), the anchor→node resolution is the mint
//! plane's ([`model::resolve`]), and the language classification is
//! `fence`'s. What is new is only the direction of travel.

use model::{ByteSpan, Document, Node, NodeKind, NodeRev, Ref, ResolveError};

use crate::address::{self, TASK_PREFIX, frontmatter};
use crate::fence::{self, FenceError, TaskLanguage};

/// One anchored fenced block on a page, as a load sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchoredBlock {
    /// The `^id` anchor that keys it — the `block` a fire addresses.
    pub anchor: String,
    /// The block's own `node_rev`: the module cache's key, and the `rev.block`
    /// a row publishes so a caller can print WHICH BYTES ran.
    pub rev: String,
    /// The classified language, or why classification refused. A refusal is
    /// carried rather than dropped: a block with an unknown info string is a
    /// fact about the page an author needs told, not an absence.
    pub lang: Result<TaskLanguage, FenceError>,
    /// The bytes between the fence lines, verbatim.
    pub source: String,
    /// The `task.<name>` this block is bound to, when the page binds one.
    /// Its presence makes the block the RUN plane's — reported with
    /// `entry_kind: task` and no declarations, untouched (§ 2.2 step 2).
    pub task: Option<String>,
}

impl AnchoredBlock {
    /// Whether this block is a starlark module — the only kind a load
    /// evaluates. Everything else is an exec target or a `bash(block=)`
    /// helper, and is never a module (§ 2.2 step 1).
    #[must_use]
    pub fn is_starlark(&self) -> bool {
        matches!(self.lang, Ok(TaskLanguage::Starlark))
    }
}

/// Why one anchor could not be resolved to a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockError {
    /// No anchor of that name on the page — the `no_block` fault.
    NoBlock { anchor: String },
    /// The id is minted more than once, so it addresses nothing — the typed
    /// `ambiguous_anchor` fault. The mint plane never silently picks.
    AmbiguousAnchor { anchor: String, count: usize },
    /// The anchor resolves, but not to a fenced code block.
    NotACodeBlock { anchor: String },
}

impl std::fmt::Display for BlockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlockError::NoBlock { anchor } => {
                write!(f, "no such block: ^{anchor}")
            }
            BlockError::AmbiguousAnchor { anchor, count } => write!(
                f,
                "^{anchor} is minted {count} times on this page, so it addresses none of them"
            ),
            BlockError::NotACodeBlock { anchor } => {
                write!(f, "^{anchor} does not key a fenced code block")
            }
        }
    }
}

impl std::error::Error for BlockError {}

impl BlockError {
    /// The wire fault class this refusal answers as (§ 2.2 Response).
    #[must_use]
    pub fn class(&self) -> &'static str {
        match self {
            BlockError::NoBlock { .. } | BlockError::NotACodeBlock { .. } => "no_block",
            BlockError::AmbiguousAnchor { .. } => "ambiguous_anchor",
        }
    }
}

/// Every anchored fenced block on the page, in document order.
///
/// An anchor that keys something other than a fence is simply not a block and
/// does not appear — a page's prose anchors are not the load's business. A
/// DUPLICATE anchor is different: it is an authoring fault the author needs
/// told about, so it appears as an [`Err`] rather than vanishing.
#[must_use]
pub fn anchored_blocks(doc: &Document) -> Vec<Result<AnchoredBlock, BlockError>> {
    let bindings = task_bindings(doc);
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for anchor in model::selector::live_anchors(doc) {
        // `live_anchors` lists every anchor NODE, so a duplicated id appears
        // twice; reporting it twice would be one authoring fault told as two.
        if seen.contains(&anchor) {
            continue;
        }
        match block_at(doc, &anchor, &bindings) {
            // Not a fence at all: prose anchors are not blocks, and a load
            // that reported them would answer with noise.
            Err(BlockError::NotACodeBlock { .. } | BlockError::NoBlock { .. }) => {}
            other => out.push(other),
        }
        seen.push(anchor);
    }
    out
}

/// Resolve ONE anchor to its block — the fire path's addressing, which asks
/// about a named block rather than enumerating the page.
///
/// # Errors
/// [`BlockError`] — dangling, duplicated, or not a fence.
pub fn block(doc: &Document, anchor: &str) -> Result<AnchoredBlock, BlockError> {
    block_at(doc, anchor, &task_bindings(doc))
}

/// One anchor → its block, against an already-read binding table.
fn block_at(
    doc: &Document,
    anchor: &str,
    bindings: &[(String, String)],
) -> Result<AnchoredBlock, BlockError> {
    let r#ref = Ref::anchor(anchor.to_owned()).map_err(|_| BlockError::NoBlock {
        anchor: anchor.to_owned(),
    })?;
    let target = model::resolve(doc, &r#ref).map_err(|e| match e {
        ResolveError::NotFound => BlockError::NoBlock {
            anchor: anchor.to_owned(),
        },
        ResolveError::Ambiguous(candidates) => BlockError::AmbiguousAnchor {
            anchor: anchor.to_owned(),
            count: candidates.len(),
        },
    })?;
    let (span, rev) =
        host_code_block(doc, &target.span).ok_or_else(|| BlockError::NotACodeBlock {
            anchor: anchor.to_owned(),
        })?;
    // A classification refusal is DATA here, not an error: the block exists,
    // and "its info string names no language I dispatch on" is what the
    // author has to be told about it.
    let (lang, source) = match fence::classify(doc, &span) {
        Ok(block) => (Ok(block.lang), block.source),
        Err(e) => (Err(e), inner_source(&doc.raw[span.clone()])),
    };
    Ok(AnchoredBlock {
        anchor: anchor.to_owned(),
        rev: rev.0,
        lang,
        source,
        task: bindings
            .iter()
            .find(|(_, bound)| bound == anchor)
            .map(|(task, _)| task.clone()),
    })
}

/// The page's `task.<name>` → anchor table, faults dropped: a sibling row's
/// broken binding must not hide a block that is fine (the shipped row-scoped
/// posture, [`crate::address::declared`]).
fn task_bindings(doc: &Document) -> Vec<(String, String)> {
    address::declared(doc)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|declared| {
            declared
                .binding
                .ok()
                .map(|binding| (declared.name, binding.anchor))
        })
        .collect()
}

/// The fenced code block an anchor keys — the same recognition
/// [`crate::address`] performs, over the same model widening: an own-line
/// anchor below a fence resolves TO the fence, so a block-keying anchor's
/// host span IS the `CodeBlock`'s span.
fn host_code_block(doc: &Document, anchor_span: &ByteSpan) -> Option<(ByteSpan, NodeRev)> {
    fn find(node: &Node, span: &ByteSpan) -> Option<(ByteSpan, NodeRev)> {
        if matches!(node.kind, NodeKind::CodeBlock { .. }) && node.span == *span {
            return Some((node.span.clone(), node.node_rev.clone()));
        }
        node.children.iter().find_map(|c| find(c, span))
    }
    find(&doc.root, anchor_span)
}

/// The bytes between the fence lines. Needed for the one case
/// [`fence::classify`] cannot answer — an unknown language still has a body,
/// and a caller reporting the refusal wants to show what it refused.
fn inner_source(fence_text: &str) -> String {
    let after_open = fence_text.find('\n').map_or("", |p| &fence_text[p + 1..]);
    match after_open.rfind('\n') {
        Some(p) => after_open[..p].to_owned(),
        None => String::new(),
    }
}

/// Whether the page declares ANY `task.<name>` binding — the cheap check the
/// fire path uses to tell a task page from a declaring one without resolving.
#[must_use]
pub fn declares_tasks(doc: &Document) -> bool {
    frontmatter(doc).is_some_and(|map| {
        map.0
            .iter()
            .any(|(key, _)| key.starts_with(TASK_PREFIX) && key.len() > TASK_PREFIX.len())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(raw: &str) -> Document {
        let raw = raw.to_owned();
        let nodes = syntax::parse(&raw);
        model::build(raw, nodes)
    }

    #[test]
    fn every_anchored_fence_is_enumerated_in_document_order() {
        let page = "\
# Page

```starlark
declare(on = \"Stop\")
```
^first

Some prose.
^just-prose

```bash
echo hi
```
^second
";
        let blocks: Vec<AnchoredBlock> = anchored_blocks(&doc(page))
            .into_iter()
            .map(|b| b.expect("no refusals in this page"))
            .collect();
        assert_eq!(
            blocks.iter().map(|b| b.anchor.as_str()).collect::<Vec<_>>(),
            ["first", "second"],
            "a prose anchor is not a block and does not appear"
        );
        assert!(blocks[0].is_starlark());
        assert_eq!(blocks[0].source, "declare(on = \"Stop\")");
        assert_eq!(blocks[1].lang, Ok(TaskLanguage::Bash));
        assert!(
            !blocks[0].rev.is_empty() && blocks[0].rev != blocks[1].rev,
            "each block carries its OWN rev — that is the cache key"
        );
    }

    #[test]
    fn a_task_bound_block_says_which_task_binds_it() {
        let page = "\
---
task.arm: \"[[#^armer]]\"
---

# Page

```starlark
def run(ctx):
    pass
```
^armer

```starlark
declare(on = \"Stop\")
```
^hook
";
        let blocks: Vec<AnchoredBlock> = anchored_blocks(&doc(page))
            .into_iter()
            .map(|b| b.expect("no refusals"))
            .collect();
        assert_eq!(blocks[0].task.as_deref(), Some("arm"));
        assert_eq!(
            blocks[1].task, None,
            "a declaring block is not bound to a task"
        );
        assert!(declares_tasks(&doc(page)));
    }

    #[test]
    fn a_duplicated_anchor_refuses_once_and_says_how_many() {
        let page = "\
# Page

```starlark
x = 1
```
^dup

```starlark
y = 2
```
^dup
";
        let refusals = anchored_blocks(&doc(page));
        assert_eq!(refusals.len(), 1, "one authoring fault, told once");
        let error = refusals[0].clone().expect_err("a duplicate addresses none");
        assert_eq!(
            error,
            BlockError::AmbiguousAnchor {
                anchor: "dup".to_owned(),
                count: 2
            }
        );
        assert_eq!(error.class(), "ambiguous_anchor");
    }

    #[test]
    fn an_unknown_language_is_carried_not_dropped() {
        let page = "\
# Page

```python
print(1)
```
^py
";
        let blocks = anchored_blocks(&doc(page));
        let block = blocks[0].clone().expect("the block exists");
        assert_eq!(
            block.lang,
            Err(FenceError::UnknownLanguage {
                lang: "python".to_owned()
            })
        );
        assert!(!block.is_starlark());
        assert_eq!(
            block.source, "print(1)",
            "the body is carried so a refusal can show what it refused"
        );
    }

    #[test]
    fn addressing_one_block_answers_the_typed_refusals() {
        let page = "\
# Page

```starlark
x = 1
```
^here
";
        let d = doc(page);
        assert_eq!(block(&d, "here").expect("resolves").anchor, "here");
        assert_eq!(
            block(&d, "gone").expect_err("dangling"),
            BlockError::NoBlock {
                anchor: "gone".to_owned()
            }
        );
        assert_eq!(block(&d, "gone").expect_err("dangling").class(), "no_block");
    }
}
