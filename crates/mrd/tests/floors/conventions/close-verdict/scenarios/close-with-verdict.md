---
actor: worker-a
force: false
---

# close-with-verdict (passes)

A close flips `status` to `closed` AND records a `verdict` (create-OR-replace).
The convention passes: the close carries its Verdict. This is the legal path the
firing scenario cites; a re-decision (bounce) re-upserts the `verdict` key and
lands.

```json ^put
{
  "op": "splice",
  "path": "tasks/ship-cache.md",
  "actor": "worker-a",
  "force": false,
  "edits": [
    {"target": {"fm_key": "status"}, "edit": {"put": {"at": "upsert", "text": "closed"}}},
    {"target": {"fm_key": "verdict"}, "edit": {"put": {"at": "upsert", "text": "approve"}}}
  ]
}
```

```starlark ^expect
def expect(t):
    if t.result.refused:
        fail("a close carrying a Verdict must land")
```
