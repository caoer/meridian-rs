---
actor: agent:bob
force: false
---

# reviewer-close (passes)

A reviewer distinct from the owner (`agent:bob`, while the owner is
`agent:alice`) closes the task. The convention passes: reviewer ≠ owner, so the
close lands. This is the legal path the firing scenario cites.

```json ^put
{
  "path": "tasks/fix-parser.md",
  "edits": [],
  "properties": { "status": "closed" }
}
```

```starlark ^expect
def expect(t):
    if t.result.refused:
        fail("a reviewer distinct from the owner may close the task")
```
