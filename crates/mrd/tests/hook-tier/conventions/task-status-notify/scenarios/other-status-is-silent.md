---
scenario: hook-other-status-is-silent
convention_rev: hook@1
---

A different status still uses the production splice, but the HOOK stays silent.

```json ^put
{ "op": "splice", "path": "tasks/card.md", "target": {"fm_key": "status"}, "at": "upsert", "text": "blocked" }
```

```starlark ^expect
def expect(t):
    want(t.result.ok, "the production splice must commit")
    want(len(t.result.effects) == 0, "a non-review transition must arm nothing")
    want("status: blocked" in t.doc("tasks/card.md"), "the landed state is visible")
```
