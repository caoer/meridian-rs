---
actor: reviewer-r
force: false
---

# external-arming (passes — the actor ≠ author legal path)

The legal path for the actor ≠ author precondition: a reviewer (`reviewer-r`)
distinct from the convention's `author` (`author-x`) arms it. The firing
counterpart has the author arm their own convention.

```json ^put
{
  "op": "splice",
  "path": "conventions/candidate/CHECK.md",
  "actor": "reviewer-r",
  "force": false,
  "edits": [
    {"target": {"fm_key": "arm"}, "edit": {"put": {"at": "upsert", "text": "block"}}}
  ]
}
```

```starlark ^expect
def expect(t):
    if t.result.refused:
        fail("an arming by a reviewer distinct from the author must land")
```
