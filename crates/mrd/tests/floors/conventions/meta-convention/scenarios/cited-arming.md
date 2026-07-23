---
actor: reviewer-r
force: false
---

# cited-arming (passes — the `cites:` join legal path)

The legal path for the `cites:` precondition: an arming that declares its
structural `cites:` evidence join (alongside a pinned `armed_rev` and an author
distinct from the actor) lands. The firing counterpart drops `cites`.

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
        fail("an arming that cites its evidence join must land")
```
