---
scenario: cas-stale-fires
convention_rev: seed@1
cas: true
pairs: "[[cas-declared]]"
---

Under `cas: true`, an explicit stale `if_root` in the `^put` proves the world
guard is REAL: the write is refused `root_mismatch` because the pinned root does
not match the live world. Fires; pairs the passing CAS sibling.

```base notes/plan.md
# Plan

body
```

```put
{ "op": "splice", "path": "notes/plan.md", "target": {"hpath": [{"h": "Plan"}]}, "at": "end", "text": "- more\n", "if_root": "b3:0000000000000000000000000000000000000000000000000000000000000000" }
```

```expect
def expect(t):
    want(t.result.refused, "a stale world guard must refuse")
    want(t.result.code == "root_mismatch", "the refusal is root_mismatch")
```
