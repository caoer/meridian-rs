---
tags: [type/rule, rules/middleware]
id: 010-fixture-middleware-wrong-entry
paths:
  - "**/tasks/*.md"
---

# 010-fixture-middleware-wrong-entry — a card records who bore it

**A FIXTURE, not a rule.** The shape of the sessions root's
`rules/010-middleware-spawned-by.md` as it stood on 2026-08-23: the tag names
the `rules/middleware` leg, and the fenced block defines `check_change` — the
CHECK leg's entry point. `def middleware` is absent, so the page REGISTERS and
cannot LOAD, and arming it in a firing mode would pin law that can never fire.

The shape is copied, never read from the live root: that page is being
rewritten (session `22-18-hook-support-design`, card
`rules-010-middleware-wrong-entry-point`), and a test bound to the live bytes
would flip with the rewrite instead of holding the wiring it exists to hold.
Only the `id:` differs from the live page — a `fixture-` segment so this copy
can never collide with the rule it models.

```starlark
def check_change(change):
    path = change.doc.path
    if not path.endswith(".md"):
        return
    who = change.doc.frontmatter.get("spawned-by")
    if who != None and who != "":
        return
    refuse(
        message = "spawned-by: " + path + " is a card with no `spawned-by`.",
        passing = "stamp-spawned-by-at-birth",
    )
```
