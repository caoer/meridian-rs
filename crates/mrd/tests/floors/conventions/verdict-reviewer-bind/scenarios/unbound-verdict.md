---
actor: dave
force: false
---

# unbound-verdict (fires)

`dave` lands a Verdict whose `reviewer` names `carol` — someone other than the
closing actor. The convention fires (`reviewer_bind`, ATTACK-024): a Verdict may
not attribute its review to another. The legal path is [[bound-verdict]].

```json ^put
{
  "path": "verdicts/close-1.md",
  "edits": [],
  "properties": { "outcome": "approve" }
}
```

```starlark ^expect
def expect(t):
    if not t.result.refused:
        fail("a Verdict naming a reviewer other than the actor must be refused")
    if "reviewer_bind" not in t.result.message:
        fail("the refusal must teach the reviewer_bind rule")
```
