---
scenario: orphan-fire
convention_rev: seed@1
---

Load-bearing negative: a FIRING scenario (its `^put` is refused) that declares
NO passing sibling. The pairing lint refuses the suite with a HARD ERROR (exit
2) — a firing scenario must wikilink a passing sibling.

```base notes/todo.md
# Todo
```

```put
{ "op": "splice", "path": "/etc/orphan.md", "target": {"hpath": [{"h": "Todo"}]}, "at": "end", "text": "x\n" }
```

```expect
def expect(t):
    want(t.result.refused, "this scenario fires")
```
