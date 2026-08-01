---
tags: [type/rule, rules/check]
id: meta-convention
paths:
  - conventions/**
---

# meta-convention (floor rule U4.4)

The rule that guards ARMING itself. A page under `conventions/**` that proposes
to arm (`arm: warn` / `arm: block`) must satisfy three preconditions before the
door will honour it (taxonomy row 8, `arming_precondition{rule}`, recovery
`fix`):

1. **P@R pinned evidence** — the arming must pin an attested evidence rev
   (`armed_rev`). Arming with no pinned rev is unarmed evidence — refused.
2. **structural `cites:` join** — the arming must declare a `cites:` evidence
   join. A missing join is refused.
3. **actor ≠ author** — a page's `author` may not arm their own rule
   (self-arming). `change.actor == author` is refused.

Each precondition cites its OWN legal-path case, which is what keeps the three
distinguishable: the tier's fire signal is keyed on the citation, so collapsing
the three onto one citation would stop the run from saying WHICH precondition a
case violated, and would leave two of the three unable to be reported dead.
The three legal-path cases carry the same write and differ in which axis they
demonstrate — exactly the three passing scenarios this rule used to cite before
the scenario tier retired.

FLOOR rule (plan U4.4). Each `passing =` citation names a corpus-tier CASE — the
scenario page it used to name retired with its tier, and a refusal may not cite
a page that is not in the tree.

```starlark
def check_change(change):
    fm = change.doc.frontmatter
    arm = fm.get("arm")
    # Only an arming proposal (arm set to warn/block) is judged.
    if arm != "warn" and arm != "block":
        return
    armed_rev = fm.get("armed_rev")
    if armed_rev == None or armed_rev == "":
        refuse(
            message = "arming_precondition: cannot arm without attested evidence — no pinned `armed_rev` at the reviewed rev (P@R)",
            passing = "attested-arming",
        )
    cites = fm.get("cites")
    if cites == None or cites == "":
        refuse(
            message = "arming_precondition: arming requires a structural `cites:` evidence join",
            passing = "cited-arming",
        )
    author = fm.get("author")
    actor = change.actor
    if author != None and actor != None and author == actor:
        refuse(
            message = "arming_precondition: actor == author — a page's author may not arm their own rule",
            passing = "external-arming",
        )
```
