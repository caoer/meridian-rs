---
scenario: put-escape-abs
convention_rev: seed@1
pairs: "[[put-append-pass]]"
---

Mount confinement (load-bearing, taxonomy row 22): a `^put` at an ABSOLUTE path
escapes the mount and is REFUSED `bad_path` — the bytes never land outside the
tmpdir. Fires; pairs the passing sibling that writes a legal in-mount path.

```base notes/todo.md
# Todo
```

```put
{ "op": "splice", "path": "/etc/meridian-evil.md", "target": {"hpath": [{"h": "Todo"}]}, "at": "end", "text": "pwned\n" }
```

```expect
def expect(t):
    want(t.result.refused, "an absolute path must be refused")
    want(t.result.code == "bad_path", "the refusal is bad_path (mount escape)")
```
