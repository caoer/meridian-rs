---
actor: worker-b
force: false
---

# uncontested-claim (passes)

`worker-b` claims a task whose `owner` slot is empty (unclaimed). The convention
passes: the compare-and-set lands against the empty slot. This is the legal path
the firing scenario cites.

```json ^put
{
  "path": "tasks/unclaimed.md",
  "edits": [],
  "properties": { "owner": "worker-b" }
}
```

```starlark ^expect
def expect(t):
    if t.result.refused:
        fail("claiming an unclaimed slot must land")
```
