---
paths:
  - conventions/**
---

# meta-convention (floor convention U4.4)

The convention that guards ARMING itself. A convention page under
`conventions/**` that proposes to arm (`arm: warn` / `arm: block`) must satisfy
three preconditions before the door will honour it (taxonomy row 8,
`arming_precondition{rule}`, recovery `fix`):

1. **P@R pinned evidence** — the arming must pin an attested evidence rev
   (`armed_rev`). Arming with no pinned rev is unarmed evidence — refused.
2. **structural `cites:` join** — the arming must declare a `cites:` evidence
   join. A missing join is refused.
3. **actor ≠ author** — a convention's `author` may not arm their own
   convention (self-arming). `change.actor == author` is refused.

Each precondition cites its own passing scenario, so the three rules are
distinct (dead-rule granularity). The legal path is an arming that pins attested
evidence, cites the join, and is armed by a reviewer distinct from the author.

FLOOR convention (plan U4.4). Arm through `docs/arming-from-zero.md`.

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
            passing = "scenarios/attested-arming.md",
        )
    cites = fm.get("cites")
    if cites == None or cites == "":
        refuse(
            message = "arming_precondition: arming requires a structural `cites:` evidence join",
            passing = "scenarios/cited-arming.md",
        )
    author = fm.get("author")
    actor = change.actor
    if author != None and actor != None and author == actor:
        refuse(
            message = "arming_precondition: actor == author — a convention's author may not arm their own convention",
            passing = "scenarios/external-arming.md",
        )
```
