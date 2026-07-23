---
actor: reviewer-r
force: false
---

# attested-arming (passes — the legal arming)

An arming proposal (`arm: block`) that pins attested evidence (`armed_rev`),
cites its evidence join (`cites`), and is armed by a reviewer (`reviewer-r`)
distinct from the `author` (`author-x`). The convention passes: all three
preconditions are met. This is the legal path the P@R firing scenario cites.

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
        fail("an arming that meets all three preconditions must land")
```
