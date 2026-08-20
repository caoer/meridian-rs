//! Shared test support: in-memory page building (no disk).

#![allow(dead_code, unreachable_pub)]

use model::Document;

/// Parse a page from raw markdown.
pub fn doc(raw: &str) -> Document {
    model::build(raw.to_owned(), syntax::parse(raw))
}

/// The standard fixture page: a starlark check task, a bash fix task with
/// caps/args/env declarations, a dangling binding, and a non-code-block
/// binding.
pub const PAGE: &str = "\
---
task.check-links: \"[[#^chk-1]]\"
task.fix-drift: \"[[#^fix-1]]\"
task.fix-drift.caps: md.edit, md.create:tasks/*.md
task.fix-drift.args: page
task.fix-drift.env: HOME_WIKI
task.dangling: \"[[#^gone]]\"
task.bad-block: \"[[#^para-1]]\"
---

# Tasks

```starlark
def run(ctx):
    pass
```
^chk-1

```bash
echo hi
```
^fix-1

A paragraph block.
^para-1
";

/// The span of the first `CodeBlock` node whose fence text contains `needle`.
pub fn code_block_span(doc: &Document, needle: &str) -> model::ByteSpan {
    fn collect<'a>(node: &'a model::Node, out: &mut Vec<&'a model::Node>) {
        if matches!(node.kind, model::NodeKind::CodeBlock { .. }) {
            out.push(node);
        }
        for c in &node.children {
            collect(c, out);
        }
    }
    let mut blocks = Vec::new();
    collect(&doc.root, &mut blocks);
    blocks
        .into_iter()
        .find(|n| doc.raw[n.span.clone()].contains(needle))
        .map(|n| n.span.clone())
        .expect("fixture code block")
}
