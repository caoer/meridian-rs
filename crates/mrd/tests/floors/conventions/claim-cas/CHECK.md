---
paths:
  - tasks/**
---

# claim-cas (floor convention U4.4)

A claim is a compare-and-set on the task `owner`: it lands only against an
UNCLAIMED slot. When a task already carries a non-empty `owner` (the winner) and
a change reassigns `owner` to a different writer, the claim is contested — the
LOSER is refused and the refusal NAMES the winner (taxonomy row 18,
`claim_cas{winner}`, recovery `refresh`: re-read, someone holds it). The legal
path is the uncontested-claim scenario, where the slot was empty.

FLOOR convention (plan U4.4). Arm through `docs/arming-from-zero.md`.

```starlark
def check_change(change):
    if "owner" not in change.fields_changed:
        return
    winner = change.before.frontmatter.get("owner")
    after_owner = change.doc.frontmatter.get("owner")
    # A contested claim: the slot already held a winner, and the change moves it
    # to a different owner. An unclaimed slot (empty/None winner) is the legal path.
    if winner != None and winner != "" and after_owner != winner:
        refuse(
            message = "claim_cas: task already claimed by " + winner + " — the claim is contested; " + winner + " holds it, re-read",
            passing = "scenarios/uncontested-claim.md",
        )
```
