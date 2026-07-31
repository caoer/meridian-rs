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
  "op": "splice",
  "path": "tasks/unclaimed.md",
  "actor": "worker-b",
  "force": false,
  "edits": [
    {"target": {"fm_key": "owner"}, "edit": {"put": {"at": "upsert", "text": "worker-b"}}}
  ]
}
```

```starlark ^expect
def expect(t):
    if t.result.refused:
        fail("claiming an unclaimed slot must land")
```
