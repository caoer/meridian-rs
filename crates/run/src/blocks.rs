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
//! The answers are addressing's own: an anchor resolves as the mint plane
//! resolves it ([`model::resolve`]), and the language classification is
//! `fence`'s. [`PageBlocks`] gathers them for every anchor from one walk of
//! the tree, so a page with many anchors costs one walk, not one per anchor.

use std::collections::{BTreeMap, HashMap};

use model::{ByteSpan, Document, Node, NodeKind, Ref};

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

/// A page's anchored blocks, resolved once and held without the page. A `run`
/// mode row copies this out of its page's lease and runs on it (merkle-spec
/// §6.8): each anchor's resolution, which for a fence is its bytes and for
/// anything else is the anchor's name and why it is not a block. Never the
/// parse tree.
///
/// [`PageBlocks::of`] walks the tree once. The answers are the mint plane's
/// ([`model::resolve`]): an anchor id resolves to the one anchor node minted
/// with it, whose span the model widens to the fence it keys, so an anchor
/// keys a block when a `CodeBlock` node has exactly that span. Two nodes with
/// one id are ambiguous, and an id no node carries is no block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageBlocks {
    /// The anchors [`PageBlocks::listed`] lists, in its order.
    listed: Vec<String>,
    /// Every live anchor's resolution, as [`PageBlocks::block`] answers it.
    named: BTreeMap<String, Result<AnchoredBlock, BlockError>>,
}

impl PageBlocks {
    /// Resolve every live anchor on `doc`, from one walk of its tree.
    #[must_use]
    pub fn of(doc: &Document) -> Self {
        let (anchors, fences) = index(doc);
        let bindings = task_bindings(doc);
        let mut listed = Vec::new();
        let mut named = BTreeMap::new();
        for (anchor, spans) in anchors {
            let resolved = resolve(doc, &anchor, &spans, &fences, &bindings);
            let lists = match &resolved {
                Ok(_) => true,
                // Not a fence at all: prose anchors are not blocks, and a load
                // that reported them would answer with noise.
                Err(BlockError::NotACodeBlock { .. } | BlockError::NoBlock { .. }) => false,
                // A duplicated id refuses before anyone asks whether it keys a
                // fence. With no block among its nodes it is the same noise,
                // and duplicate prose ids are ordinary on real pages — a load
                // that kept them would refuse on prose fleet-wide.
                Err(BlockError::AmbiguousAnchor { .. }) => {
                    spans.iter().any(|span| fences.contains_key(span))
                }
            };
            if lists {
                listed.push(anchor.clone());
            }
            named.insert(anchor, resolved);
        }
        Self { listed, named }
    }

    /// Every anchored fenced block on the page, in document order — what a
    /// load enumerates.
    ///
    /// An anchor that keys something other than a fence is simply not a block
    /// and does not appear — a page's prose anchors are not the load's
    /// business. A DUPLICATE anchor is that same question asked twice: it is an
    /// authoring fault the author needs told about, and appears once as an
    /// [`Err`] rather than vanishing, WHEN A BLOCK IS AMONG WHAT IT DUPLICATES —
    /// there the load cannot say which block it would run. A duplicated id that
    /// keys only prose is still prose, and leaves with the rest of the prose.
    ///
    /// That is a statement about ENUMERATING, and only about enumerating. A
    /// page that NAMES such an id still meets it: an `exec(block = ...)`
    /// declaration resolves through [`PageBlocks::block`], which does not
    /// narrow, so a LOAD of that page still faults `ambiguous_anchor` —
    /// reported against the declaration that named the id (§ 3.4 step 3,
    /// [`crate::modes`]). Naming is not enumerating, and that difference is the
    /// whole seam.
    #[must_use]
    pub fn listed(&self) -> Vec<Result<AnchoredBlock, BlockError>> {
        self.listed
            .iter()
            .filter_map(|anchor| self.named.get(anchor).cloned())
            .collect()
    }

    /// ONE anchor's block — the fire path's addressing, which asks about a
    /// named block rather than enumerating the page.
    ///
    /// # Errors
    /// [`BlockError`] — dangling, duplicated, or not a fence.
    pub fn block(&self, anchor: &str) -> Result<AnchoredBlock, BlockError> {
        self.named.get(anchor).cloned().unwrap_or_else(|| {
            Err(BlockError::NoBlock {
                anchor: anchor.to_owned(),
            })
        })
    }
}

/// One walk of the tree: each anchor id with its nodes' spans, ids in the order
/// [`model::selector::live_anchors`] first lists them, and each `CodeBlock`
/// node by its span (the first in document order, as a search would find).
#[allow(clippy::type_complexity)]
fn index(doc: &Document) -> (Vec<(String, Vec<ByteSpan>)>, HashMap<ByteSpan, &Node>) {
    fn walk<'a>(
        node: &'a Node,
        anchors: &mut Vec<(String, Vec<ByteSpan>)>,
        at: &mut HashMap<String, usize>,
        fences: &mut HashMap<ByteSpan, &'a Node>,
    ) {
        match &node.kind {
            NodeKind::Anchor { name } => {
                if let Some(&slot) = at.get(name) {
                    anchors[slot].1.push(node.span.clone());
                } else {
                    at.insert(name.clone(), anchors.len());
                    anchors.push((name.clone(), vec![node.span.clone()]));
                }
            }
            NodeKind::CodeBlock { .. } => {
                fences.entry(node.span.clone()).or_insert(node);
            }
            _ => {}
        }
        for child in &node.children {
            walk(child, anchors, at, fences);
        }
    }
    let mut anchors = Vec::new();
    let mut fences = HashMap::new();
    walk(&doc.root, &mut anchors, &mut HashMap::new(), &mut fences);
    (anchors, fences)
}

/// One anchor id → its block, from the page's index.
fn resolve(
    doc: &Document,
    anchor: &str,
    spans: &[ByteSpan],
    fences: &HashMap<ByteSpan, &Node>,
    bindings: &[(String, String)],
) -> Result<AnchoredBlock, BlockError> {
    if Ref::anchor(anchor.to_owned()).is_err() {
        return Err(BlockError::NoBlock {
            anchor: anchor.to_owned(),
        });
    }
    let span = match spans {
        [] => {
            return Err(BlockError::NoBlock {
                anchor: anchor.to_owned(),
            });
        }
        [only] => only,
        many => {
            return Err(BlockError::AmbiguousAnchor {
                anchor: anchor.to_owned(),
                count: many.len(),
            });
        }
    };
    let fence = fences.get(span).ok_or_else(|| BlockError::NotACodeBlock {
        anchor: anchor.to_owned(),
    })?;
    // A classification refusal is DATA here, not an error: the block exists,
    // and "its info string names no language I dispatch on" is what the
    // author has to be told about it.
    let (lang, source) = match fence::classify_node(doc, fence) {
        Ok(block) => (Ok(block.lang), block.source),
        Err(e) => (Err(e), inner_source(&doc.raw[span.clone()])),
    };
    Ok(AnchoredBlock {
        anchor: anchor.to_owned(),
        rev: fence.node_rev.0.clone(),
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
^zeta

Some prose.
^just-prose

```bash
echo hi
```
^alpha
";
        let blocks: Vec<AnchoredBlock> = PageBlocks::of(&doc(page))
            .listed()
            .into_iter()
            .map(|b| b.expect("no refusals in this page"))
            .collect();
        assert_eq!(
            blocks.iter().map(|b| b.anchor.as_str()).collect::<Vec<_>>(),
            ["zeta", "alpha"],
            "document order, not id order; a prose anchor is not a block and does not appear"
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
        let blocks: Vec<AnchoredBlock> = PageBlocks::of(&doc(page))
            .listed()
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
        let refusals = PageBlocks::of(&doc(page)).listed();
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
    fn a_duplicated_prose_anchor_is_not_the_loads_business() {
        let page = "\
# Page

Some prose.
^dup

More prose.
^dup
";
        assert_eq!(
            PageBlocks::of(&doc(page)).listed(),
            Vec::new(),
            "no candidate under ^dup hosts a code block, so the duplication \
             is not a block fault and a load must not price it"
        );
    }

    #[test]
    fn a_duplicate_with_one_block_candidate_still_refuses() {
        let page = "\
# Page

Some prose.
^mixed

```starlark
x = 1
```
^mixed
";
        let refusals = PageBlocks::of(&doc(page)).listed();
        assert_eq!(refusals.len(), 1, "one authoring fault, told once");
        let error = refusals[0]
            .clone()
            .expect_err("a block IS among the candidates, so the ambiguity is real");
        assert_eq!(
            error,
            BlockError::AmbiguousAnchor {
                anchor: "mixed".to_owned(),
                count: 2
            }
        );
        assert_eq!(error.class(), "ambiguous_anchor");
    }

    #[test]
    fn the_fire_path_still_names_a_prose_duplicate_as_ambiguous() {
        let page = "\
# Page

Some prose.
^dup

More prose.
^dup
";
        // The load drops it as noise because a load never asked about ^dup in
        // particular. A fire DID ask, so it keeps the more diagnostic refusal:
        // narrowing the enumerator must not narrow the addressing.
        assert_eq!(
            PageBlocks::of(&doc(page))
                .block("dup")
                .expect_err("a duplicate addresses none"),
            BlockError::AmbiguousAnchor {
                anchor: "dup".to_owned(),
                count: 2
            }
        );
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
        let blocks = PageBlocks::of(&doc(page)).listed();
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

Some prose.
^note

```starlark
x = 1
```
^here
";
        let d = PageBlocks::of(&doc(page));
        assert_eq!(d.block("here").expect("resolves").anchor, "here");
        assert_eq!(
            d.block("gone").expect_err("dangling"),
            BlockError::NoBlock {
                anchor: "gone".to_owned()
            }
        );
        assert_eq!(d.block("gone").expect_err("dangling").class(), "no_block");
        // A prose anchor resolves, to something that is not a fence: the
        // fire path says so, not that no such anchor exists.
        assert_eq!(
            d.block("note").expect_err("prose"),
            BlockError::NotACodeBlock {
                anchor: "note".to_owned()
            }
        );
    }
}
