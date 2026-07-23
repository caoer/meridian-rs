---
scenario: unknown-attr
convention_rev: seed@1
---

Load-bearing negative: an `^expect` that reaches for an attribute outside the
closed `t.` surface (`t.bogus`) FAILS LOUD — the `t` surface is `result` /
`journal` / `doc(path)` and nothing else (taxonomy row 23). Run in isolation:
its `^expect` is EXPECTED to fail, so the suite reports exit 1 with a message
naming the missing attribute.

```base notes/todo.md
# Todo
```

```put
{ "op": "splice", "path": "notes/todo.md", "target": {"hpath": [{"h": "Todo"}]}, "at": "end", "text": "- x\n" }
```

```expect
def expect(t):
    want(t.bogus, "reaching for an unknown t. attribute must fault loud")
```
