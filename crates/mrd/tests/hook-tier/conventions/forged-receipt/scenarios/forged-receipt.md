---
scenario: hook-forged-receipt
convention_rev: hook@1
---

The predicate emits a non-canonical receipt. Policy must reject the descriptor.

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
    fail("a forged receipt must fail before this expectation runs")
```
