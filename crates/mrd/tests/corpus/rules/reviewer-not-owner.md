---
tags: [type/rule, rules/check]
id: reviewer-not-owner
paths:
  - tasks/**
---

# reviewer-not-owner (corpus-tier fixture)

A task's own owner must not close their own task — a reviewer distinct from the
owner attests the close. When the acting writer equals the task's declared
`owner`, the close is refused; the legal path is the `reviewer-close` case,
where a different writer approves the work.

This is a THROWAWAY corpus-tier fixture, not a floor rule: it exists so
`mrd test --corpus` has a one-rule law whose fire / dead / surprise behaviour
over the 18-02 governed tree can be pinned. It replaces the embedded seed
convention (`policy::seed`), which died with the folder loader — a rule is a
PAGE now, so a fixture rule is a page in the fixture tree rather than bytes
compiled into the engine. Never arm it.

```starlark
def check_change(change):
    owner = change.doc.frontmatter.get("owner")
    actor = change.actor
    if actor != None and owner != None and actor == owner:
        refuse(
            message = "reviewer must not be the owner: " + actor + " cannot close their own task",
            passing = "reviewer-close",
        )
```
