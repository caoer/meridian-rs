---
tags: [type/rule, rules/check]
id: reviewer-not-owner
paths:
  - tasks/**
---

# reviewer-not-owner (floor rule U4.4)

The reviewer who CLOSES a task must not be its `owner` — a close is an
attestation by a distinct reviewer, never a self-grade. When the acting writer
equals the task's declared `owner` on a close (`status: closed`), the write is
refused (taxonomy row 19, `reviewer_owner{owner}`, recovery `fix`). The legal
path is the `reviewer-close` case, where a distinct reviewer closes.

This is a FLOOR rule (plan U4.4) — the real armed law. It registers by TAG and
is identified by its frontmatter `id:`; arming it is the explicit attested ARM
act, never a consequence of carrying the tag.

The `passing =` citation names a corpus-tier CASE, not a page. It used to name
`scenarios/reviewer-close.md`, a file the scenario tier held; that tier retired,
and a refusal whose legal path points at a deleted file teaches nothing. The
case id is what survives, so the case id is what the refusal cites.

```starlark
def check_change(change):
    # Judge only a CLOSE — the status transition to `closed`.
    if change.doc.frontmatter.get("status") != "closed":
        return
    owner = change.doc.frontmatter.get("owner")
    actor = change.actor
    if actor != None and owner != None and actor == owner:
        refuse(
            message = "reviewer_owner: " + actor + " must not close a task they own — a distinct reviewer closes",
            passing = "reviewer-close",
        )
```
