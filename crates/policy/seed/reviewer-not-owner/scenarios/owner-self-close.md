---
actor: agent:alice
force: false
---

# owner-self-close (fires)

The task's owner (`agent:alice`) closes their own task. The convention fires:
the close is refused because the reviewer is the owner. The legal path is the
sibling passing scenario, [[reviewer-close]].

```json ^put
{
  "path": "tasks/fix-parser.md",
  "edits": [],
  "properties": { "status": "closed" }
}
```

```starlark ^expect
def expect(t):
    if not t.result.refused:
        fail("owner self-close must be refused")
    if "reviewer must not be the owner" not in t.result.message:
        fail("refusal must teach the reviewer-not-owner rule")
```
