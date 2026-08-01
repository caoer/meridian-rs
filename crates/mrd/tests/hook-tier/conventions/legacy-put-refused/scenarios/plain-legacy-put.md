---
scenario: hook-plain-legacy-put
convention_rev: hook@1
---

A HOOK convention with a LEGACY unanchored write fence. The legacy grammar is a
translated lookalike that the strict wire decoder never sees, so a HOOK must never
qualify through it.

```put
{
  "op": "splice",
  "path": "tasks/card.md",
  "target": {"fm_key": "status"},
  "at": "upsert",
  "text": "review"
}
```

```starlark ^expect
def expect(t):
    fail("the legacy write must be refused before this expectation runs")
```
