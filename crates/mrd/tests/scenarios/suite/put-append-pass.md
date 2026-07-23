---
scenario: put-append-pass
convention_rev: seed@1
---

A plain splice `put at:end` appends to a mounted file and commits.

```base notes/todo.md
# Todo

- first
```

```put
{ "op": "splice", "path": "notes/todo.md", "target": {"hpath": [{"h": "Todo"}]}, "at": "end", "text": "- second\n" }
```

```expect
def expect(t):
    want(t.result.ok, "the append must commit")
    want("- second" in t.doc("notes/todo.md"), "the appended line is present after the write")
    want("- first" in t.doc("notes/todo.md"), "the original content survives")
```
