---
tags: [type/rule, rules/check]
id: no-smuggled-heading
paths:
  - tasks/**
---

# no-smuggled-heading (corpus-tier fixture — the CONTENT law)

A write that edits a section must not grow the document's heading structure.
Adding a heading restructures the page, and restructuring is its own write with
its own review — the legal path is `structural-write`.

This fixture exists to pin the tier's CONTENT reach. Its predicate reads three
`rulepack-api@2` facts that no frontmatter mutation can move: the body-state
diff (`change.sections_changed`) and the world-model node kinds on BOTH sides of
the write (`change.before.nodes`, `change.doc.nodes`). A corpus tier that cannot
drive it cannot test the half of the change surface a markdown engine exists for.
Never arm it.

```starlark
def headings(facts):
    n = 0
    for node in facts.nodes:
        if node.kind == "heading":
            n = n + 1
    return n

def check_change(change):
    if len(change.sections_changed) == 0:
        return
    if headings(change.doc) > headings(change.before):
        refuse(
            message = "a section write must not add a heading; restructure in its own write",
            passing = "structural-write",
        )
```
