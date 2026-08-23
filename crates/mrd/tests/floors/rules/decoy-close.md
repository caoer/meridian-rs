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
def _text(v):
    # The rules plane serves the STORED scalar, quotes included — wire-contract
    # § A.6.1's named residual (`DocFacts.frontmatter`), which the live armed
    # rules all strip the same way. Since card `all-digit-short-ids-read-as-int`
    # the write doors quote a bool- or number-shaped value, so a decoy written
    # through a door arrives as `done: "true"`, and a rule that compared the raw
    # bytes would stop seeing the marker it exists to catch.
    if type(v) != "string":
        return ""
    s = v.strip()
    if len(s) >= 2 and s[0] == s[-1] and s[0] in ("\"", "'"):
        s = s[1:-1].strip()
    return s

def check_change(change):
    fm = change.doc.frontmatter
    # The canonical close is `status: closed`. Anything else claiming closed is a decoy.
    if _text(fm.get("status")) == "closed":
        return
    decoy = (_text(fm.get("resolution")) == "closed" or _text(fm.get("done")) == "true" or
             _text(fm.get("state")) == "closed")
    if decoy:
        refuse(
            message = "decoy_close: a closed-looking marker (resolution/done/state) without the canonical `status: closed` — a decoy close does not satisfy the real close law",
            passing = "real-close",
        )
```
