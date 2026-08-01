---
paths:
  - tasks/**
---

# citation-collision (corpus-tier fixture)

A CHECK whose refusal cites a passing scenario named EXACTLY like the sibling HOOK's
slug. Citation ids and HOOK slugs are different namespaces; merging them would let
this refusal vouch for a HOOK that never fired.

This is a THROWAWAY test fixture, not a floor convention — never arm it.

```starlark
def check_change(change):
    if change.doc.frontmatter.get("status") == "collide":
        refuse(
            message = "the citation id collides with a HOOK slug on purpose",
            passing = "task-status-notify",
        )
```
