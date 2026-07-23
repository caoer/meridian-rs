---
paths:
  - tasks/**
---

# reviewer-and-priority (corpus-tier fixture)

A two-rule CHECK for the `test --corpus` dead-rule gate: the LIVE
`reviewer-not-owner` rule (the acting writer must not equal the task `owner`)
plus a DEAD `priority-guard` rule that refuses a `priority: high` close — a
condition no document in the governed tree carries, so no synthetic change ever
fires it. The corpus tier reports `scenarios/lower-priority.md` under "Dead
rules (declared, never fired)", the `@2` twin of the effect kernel's
`dead_priority` replay rule.

This is a THROWAWAY test fixture, not a floor convention — never arm it.

```starlark
def check_change(change):
    owner = change.doc.frontmatter.get("owner")
    actor = change.actor
    if actor != None and owner != None and actor == owner:
        refuse(
            message = "reviewer must not be the owner: " + actor + " cannot close their own task",
            passing = "scenarios/reviewer-close.md",
        )
    if change.doc.frontmatter.get("priority") == "high":
        refuse(
            message = "a high-priority task closes through the priority reviewer",
            passing = "scenarios/lower-priority.md",
        )
```
