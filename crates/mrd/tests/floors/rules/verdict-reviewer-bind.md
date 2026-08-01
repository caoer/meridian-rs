---
tags: [type/rule, rules/check]
id: verdict-reviewer-bind
paths:
  - verdicts/**
---

# verdict-reviewer-bind (floor rule U4.4)

A close Verdict binds its `reviewer` to the writer who lands it: the Verdict's
declared `reviewer` MUST equal `change.actor`, the closing actor. A Verdict that
names a reviewer OTHER than the closing actor is refused — you cannot attribute
a review to someone else (taxonomy row 20, `reviewer_bind{reviewer}`, recovery
`fix`, ATTACK-024). This is distinct from claim-CAS (who may claim) and
reviewer≠owner (who may close): it binds the Verdict's attribution to its
writer. The legal path is the `bound-verdict` case.

FLOOR rule (plan U4.4). The `passing =` citation names a corpus-tier CASE — the
scenario page it used to name retired with its tier, and a refusal may not cite
a page that is not in the tree.

```starlark
def check_change(change):
    reviewer = change.doc.frontmatter.get("reviewer")
    actor = change.actor
    if actor != None and reviewer != None and reviewer != actor:
        refuse(
            message = "reviewer_bind: the Verdict names reviewer `" + reviewer + "` but the closing actor is `" + actor + "` — the Verdict must bind reviewer == actor (ATTACK-024)",
            passing = "bound-verdict",
        )
```
