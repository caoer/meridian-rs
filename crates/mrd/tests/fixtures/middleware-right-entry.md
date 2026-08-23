---
tags: [type/rule, rules/middleware]
id: 010-fixture-middleware-right-entry
paths:
  - "**/tasks/*.md"
---

# 010-fixture-middleware-right-entry — the twin that loads

**A FIXTURE, not a rule.** `middleware-wrong-entry.md`'s twin: same tag, same
`paths:`, same page shape — the ONE difference that matters is that the fenced
block defines `def middleware`, the entry point the `rules/middleware` leg is
evaluated through.

It is the control. Without it, a green "the wrong entry point is refused" could
mean the gate discriminates on the entry point, or that this whole shape never
arms through the binary for some unrelated reason.

```starlark
def middleware(ctx):
    pass
```
