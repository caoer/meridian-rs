---
actor: carol
force: false
---

# bound-verdict (passes)

`carol` lands a Verdict whose `reviewer` is `carol` — reviewer == closing actor.
The convention passes: the Verdict binds its attribution to its writer. This is
the legal path the firing scenario cites.

```json ^put
{
  "path": "verdicts/close-1.md",
  "edits": [],
  "properties": { "outcome": "approve" }
}
```

```starlark ^expect
def expect(t):
    if t.result.refused:
        fail("a Verdict whose reviewer equals the closing actor must land")
```
