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
  "op": "splice",
  "path": "conventions/candidate/CHECK.md",
  "actor": "reviewer-r",
  "force": false,
  "edits": [
    {"target": {"fm_key": "arm"}, "edit": {"put": {"at": "upsert", "text": "block"}}},
    {"target": {"fm_key": "armed_rev"}, "edit": {"put": {"at": "all", "text": ""}}}
  ]
}
```

```starlark ^expect
def expect(t):
    if not t.result.refused:
        fail("arming without attested evidence must be refused")
    if "arming_precondition" not in t.result.message:
        fail("the refusal must teach the arming_precondition rule")
```
