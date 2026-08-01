---
scenario: hook-two-writes
convention_rev: hook@1
---

Two production writes, each arming one intent. `t.result` is the FINAL write's
outcome, whole — it must not carry the earlier write's effect, because a result
combining one write's code with another write's effects is the result of no exact
production operation.

```json ^put
{
  "op": "splice",
  "path": "tasks/card.md",
  "actor": "mrd-test",
  "edits": [
    {
      "target": {"fm_key": "status"},
      "edit": {"put": {"at": "upsert", "text": "first"}}
    }
  ]
}
```

```json ^put
{
  "op": "splice",
  "path": "tasks/card.md",
  "actor": "mrd-test",
  "edits": [
    {
      "target": {"fm_key": "status"},
      "edit": {"put": {"at": "upsert", "text": "second"}}
    }
  ]
}
```

```starlark ^expect
def expect(t):
    want(t.result.ok, "both production splices commit")
    want(len(t.result.effects) == 1, "t.result carries only the FINAL write's effects")
    want(t.result.effects[0].target == "second", "and that effect is the final write's")
    want("status: second" in t.doc("tasks/card.md"), "the landed state is the final one")
