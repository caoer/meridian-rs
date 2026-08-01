---
scenario: hook-move-to-review
convention_rev: hook@1
---

The exact production splice moves the task to review and arms one intent.

```json ^put
{
  "op": "splice",
  "path": "tasks/card.md",
  "actor": "mrd-test",
  "edits": [
    {
      "target": {"fm_key": "status"},
      "edit": {"put": {"at": "upsert", "text": "review"}}
    }
  ]
}
```

```starlark ^expect
def expect(t):
    want(t.result.ok, "the production splice must commit")
    want(len(t.result.effects) == 1, "the matching HOOK must arm one effect")
    effect = t.result.effects[0]
    want(effect.rule_id == "task-status-notify", "the emitting HOOK is named")
    want(effect.action == "notify", "the panel alias is carried")
    want(effect.target == "e4201e72", "the target comes from the card")
    want(effect.severity == "info", "the classification is data")
    want(effect.payload == "task tasks/card.md → review", "the payload is exact")
    want(effect.receipt.startswith("tasks/card.md#^r-"), "the pre-delivery receipt is exposed")
    want("status: review" in t.doc("tasks/card.md"), "the landed state is visible")
```
