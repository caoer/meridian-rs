---
actor: reviewer-b
force: false
---

# reviewer-close (passes)

A reviewer distinct from the owner (`reviewer-b`, while the owner is `worker-a`)
closes the task. The convention passes: reviewer ≠ owner, so the close lands.
This is the legal path the firing scenario cites.

```json ^put
{
  "op": "splice",
  "path": "tasks/fix-parser.md",
  "actor": "reviewer-b",
  "force": false,
  "edits": [
    {"target": {"fm_key": "status"}, "edit": {"put": {"at": "upsert", "text": "closed"}}}
  ]
}
```

```starlark ^expect
def expect(t):
    if t.result.refused:
        fail("a reviewer distinct from the owner may close the task")
```
