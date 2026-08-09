---
tags: [type/rule, rules/check]
id: decoy-close
paths:
  - tasks/**
---

# decoy-close (floor rule U4.4)

A decoy close signals "closed" through a NON-canonical marker while the
canonical `status` stays open — a change that looks like a close but does not
satisfy the real close law. A `resolution: closed`, `done: true`, or `state:
closed` marker WITHOUT `status: closed` fires (taxonomy row 21,
`decoy_close{rule}`, recovery `fix`). The legal path is the `real-close` case,
where the canonical `status` carries the close.

FLOOR rule (plan U4.4). The `passing =` citation names a corpus-tier CASE — the
scenario page it used to name retired with its tier, and a refusal may not cite
a page that is not in the tree.

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
            passing = "real-close",
        )
```
