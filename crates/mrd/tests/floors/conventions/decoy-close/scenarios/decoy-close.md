---
actor: worker-a
force: false
---

# decoy-close (fires)

A `resolution: closed` marker is set while the canonical `status` stays open — a
decoy close. The convention fires (`decoy_close`): the change looks closed but
does not satisfy the real close law. The legal path is [[real-close]].

```json ^put
{
  "path": "tasks/ship-cache.md",
  "edits": [],
  "properties": { "resolution": "closed" }
}
```

```starlark ^expect
def expect(t):
    if not t.result.refused:
        fail("a decoy close must be refused")
    if "decoy_close" not in t.result.message:
        fail("the refusal must teach the decoy_close rule")
```
