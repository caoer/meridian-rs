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
  "path": "conventions/candidate/CHECK.md",
  "edits": [],
  "properties": { "arm": "block" }
}
```

```starlark ^expect
def expect(t):
    if t.result.refused:
        fail("an arming by a reviewer distinct from the author must land")
```
