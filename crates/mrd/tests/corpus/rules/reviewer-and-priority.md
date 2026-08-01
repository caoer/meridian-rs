---
tags: [type/rule, rules/check]
id: reviewer-and-priority
paths:
  - tasks/**
---

# reviewer-and-priority (corpus-tier fixture)

A two-citation CHECK for the `test --corpus` dead-rule gate: the LIVE
`reviewer-close` citation (the acting writer must not equal the task `owner`)
plus a DEAD `lower-priority` citation that refuses a `priority: high` close — a
condition no document in the governed tree carries, so no synthetic change ever
fires it. The corpus tier reports `lower-priority` under "Dead rules (declared,
never fired)", the `@2` twin of the effect kernel's `dead_priority` replay rule.

One page may carry more than one citation: a citation names the LEGAL PATH a
refusal teaches, and one law can have several. Liveness is per citation, which
is why a two-citation page can be half dead.

This is a THROWAWAY test fixture, not a floor rule — never arm it.

```starlark
def check_change(change):
    owner = change.doc.frontmatter.get("owner")
    actor = change.actor
    if actor != None and owner != None and actor == owner:
        refuse(
            message = "reviewer must not be the owner: " + actor + " cannot close their own task",
            passing = "reviewer-close",
        )
    if change.doc.frontmatter.get("priority") == "high":
        refuse(
            message = "a high-priority task closes through the priority reviewer",
            passing = "lower-priority",
        )
```
