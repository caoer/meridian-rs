---
scenario: hook-put-anchored-put
convention_rev: hook@1
---

The precedence bypass: a fence spelling BOTH the legacy language and the `^put`
anchor. Read as legacy, a flattened lookalike would qualify past the strict decoder;
the harness refuses the ambiguity instead of picking a grammar for the author.

```put ^put
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
    fail("the ambiguous fence must be refused before this expectation runs")
```
