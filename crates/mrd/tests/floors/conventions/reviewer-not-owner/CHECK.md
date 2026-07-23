---
paths:
  - tasks/**
---

# reviewer-not-owner (floor convention U4.4)

The reviewer who CLOSES a task must not be its `owner` — a close is an
attestation by a distinct reviewer, never a self-grade. When the acting writer
equals the task's declared `owner` on a close (`status: closed`), the write is
refused (taxonomy row 19, `reviewer_owner{owner}`, recovery `fix`). The legal
path is the reviewer-close scenario, where a distinct reviewer closes.

This is a FLOOR convention (plan U4.4) — the real armed law, not the throwaway
seed (`crates/policy/seed/reviewer-not-owner`, `never arm it`). Arm it through
the ladder in `docs/arming-from-zero.md`.

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
            passing = "scenarios/reviewer-close.md",
        )
```
