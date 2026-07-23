---
paths:
  - tasks/**
---

# reviewer-not-owner

A task's own owner must not close their own task — a reviewer distinct from the
owner attests the close. When the acting writer equals the task's declared
`owner`, the close is refused; the legal path is the reviewer-close scenario,
where a different writer approves the work.

This is a throwaway SEED convention (plan U1.3): it exists so the harness
(`mrd test`, U1.2) and the door (`gate()`, U4.2) can pre-test against a real
`check_change` before the U4.4 floor conventions land. Do not arm it.

```starlark
def check_change(change):
    owner = change.doc.frontmatter.get("owner")
    actor = change.actor
    if actor != None and owner != None and actor == owner:
        refuse(
            message = "reviewer must not be the owner: " + actor + " cannot close their own task",
            passing = "scenarios/reviewer-close.md",
        )
```
