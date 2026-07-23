---
paths:
  - tasks/**
---

# decoy-close (floor convention U4.4)

A decoy close signals "closed" through a NON-canonical marker while the
canonical `status` stays open — a change that looks like a close but does not
satisfy the real close law. A `resolution: closed`, `done: true`, or `state:
closed` marker WITHOUT `status: closed` fires (taxonomy row 21,
`decoy_close{rule}`, recovery `fix`). The legal path is the real-close scenario,
where the canonical `status` carries the close.

FLOOR convention (plan U4.4). Arm through `docs/arming-from-zero.md`.

```starlark
def check_change(change):
    fm = change.doc.frontmatter
    # The canonical close is `status: closed`. Anything else claiming closed is a decoy.
    if fm.get("status") == "closed":
        return
    decoy = fm.get("resolution") == "closed" or fm.get("done") == "true" or fm.get("state") == "closed"
    if decoy:
        refuse(
            message = "decoy_close: a closed-looking marker (resolution/done/state) without the canonical `status: closed` — a decoy close does not satisfy the real close law",
            passing = "scenarios/real-close.md",
        )
```
