---
actor: worker-a
force: false
---

# owner-self-close (fires)

The task owner (`worker-a`) closes their own task. The convention fires: the
close is refused (`reviewer_owner`) because the reviewer is the owner. The legal
path is the sibling passing scenario, [[reviewer-close]].

```json ^put
{
  "op": "splice",
  "path": "tasks/fix-parser.md",
  "actor": "worker-a",
  "force": false,
  "edits": [
    {"target": {"fm_key": "status"}, "edit": {"put": {"at": "upsert", "text": "closed"}}}
  ]
}
```

```starlark ^expect
def expect(t):
    if not t.result.refused:
        fail("owner self-close must be refused")
    if "reviewer_owner" not in t.result.message:
        fail("refusal must teach the reviewer_owner rule")
```
