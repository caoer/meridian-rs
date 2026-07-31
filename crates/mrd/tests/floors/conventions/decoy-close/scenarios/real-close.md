---
actor: worker-a
force: false
---

# real-close (passes)

The canonical `status: closed` carries the close. The convention passes: a real
close satisfies the close law, decoy markers notwithstanding. This is the legal
path the firing scenario cites.

```json ^put
{
  "op": "splice",
  "path": "tasks/ship-cache.md",
  "actor": "worker-a",
  "force": false,
  "edits": [
    {"target": {"fm_key": "status"}, "edit": {"put": {"at": "upsert", "text": "closed"}}}
  ]
}
```

```starlark ^expect
def expect(t):
    if t.result.refused:
        fail("a canonical status close must land")
```
