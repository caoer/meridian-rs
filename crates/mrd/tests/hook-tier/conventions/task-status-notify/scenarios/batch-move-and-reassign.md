---
scenario: hook-batch-move-and-reassign
convention_rev: hook@1
---

The exact production request changes two frontmatter nodes in one `edits[]` batch.
The old flattened harness could not represent this operation.

```json ^put
{
  "op": "splice",
  "path": "tasks/card.md",
  "actor": "mrd-test",
  "edits": [
    {
      "target": {"fm_key": "status"},
      "edit": {"put": {"at": "upsert", "text": "review"}}
    },
    {
      "target": {"fm_key": "reviewer"},
      "edit": {"put": {"at": "upsert", "text": "batch-reviewer"}}
    }
  ]
}
```

```starlark ^expect
def expect(t):
    want(t.result.ok, "the production batch splice must commit")
    want(len(t.result.effects) == 1, "the matching HOOK must arm one effect")
    effect = t.result.effects[0]
    want(effect.target == "batch-reviewer", "the HOOK reads the post-batch reviewer")
    doc = t.doc("tasks/card.md")
    want("status: review" in doc, "the status edit landed")
    want("reviewer: batch-reviewer" in doc, "the reviewer edit landed atomically")
```
