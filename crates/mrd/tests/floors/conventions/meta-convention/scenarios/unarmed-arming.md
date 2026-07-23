---
actor: reviewer-r
force: false
---

# unarmed-arming (fires — precondition P@R)

An arming proposal (`arm: block`) with NO pinned `armed_rev` — unarmed evidence.
The convention fires (`arming_precondition`): you cannot arm without attested
evidence pinned at the reviewed rev. The legal path is [[attested-arming]].

```json ^put
{
  "path": "conventions/candidate/CHECK.md",
  "edits": [],
  "properties": { "arm": "block" },
  "remove": ["armed_rev"]
}
```

```starlark ^expect
def expect(t):
    if not t.result.refused:
        fail("arming without attested evidence must be refused")
    if "arming_precondition" not in t.result.message:
        fail("the refusal must teach the arming_precondition rule")
```
