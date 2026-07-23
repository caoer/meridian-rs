---
scenario: put-escape-dotdot
convention_rev: seed@1
pairs: "[[put-append-pass]]"
---

Mount confinement (load-bearing, taxonomy row 22): a `^put` whose path traverses
`..` escapes the mount and is REFUSED `bad_path`. Fires; pairs a passing sibling.

```base notes/todo.md
# Todo
```

```put
{ "op": "splice", "path": "../meridian-evil.md", "target": {"hpath": [{"h": "Todo"}]}, "at": "end", "text": "pwned\n" }
```

```expect
def expect(t):
    want(t.result.refused, "a `..` traversal must be refused")
    want(t.result.code == "bad_path", "the refusal is bad_path (mount escape)")
```
