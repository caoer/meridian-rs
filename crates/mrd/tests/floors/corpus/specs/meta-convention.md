---
corpus_test: meta-convention-floor
rule: ../../rules/meta-convention.md
corpus: ../tree
---

# meta-convention floor — fire-where-expected (arming preconditions)

An arming proposal (`arm: block`) must pin attested evidence (`armed_rev`), cite
its evidence join (`cites`), and be armed by a reviewer distinct from the
`author`. Each precondition has its own rule; a case violating exactly one fires
exactly that rule. All three declared rules fire, so none is dead: exit 0.

The three legal-path cases (`attested-arming`, `cited-arming`,
`external-arming`) are the three passing scenarios this floor cited before the
scenario tier retired, carried here 1:1. They land the same legal write and
differ in which precondition they are the answer to — which is exactly why the
rule keeps three citations rather than collapsing onto one: the citation IS the
liveness key, so one shared name would stop the run from reporting which
precondition a case violated, and would make two of the three unreportable as
dead.

```rules
attested-arming
cited-arming
external-arming
```

```case
{ "name": "unarmed-evidence", "doc": "conventions/candidate/CHECK.md", "actor": "reviewer-r", "set": {"arm": "block"}, "remove": ["armed_rev"], "expect": "attested-arming" }
```

```case
{ "name": "uncited-arming", "doc": "conventions/candidate/CHECK.md", "actor": "reviewer-r", "set": {"arm": "block"}, "remove": ["cites"], "expect": "cited-arming" }
```

```case
{ "name": "self-arming", "doc": "conventions/candidate/CHECK.md", "actor": "author-x", "set": {"arm": "block"}, "expect": "external-arming" }
```

```case
{ "name": "attested-arming", "doc": "conventions/candidate/CHECK.md", "actor": "reviewer-r", "set": {"arm": "block"}, "expect": "pass" }
```

```case
{ "name": "cited-arming", "doc": "conventions/candidate/CHECK.md", "actor": "reviewer-r", "set": {"arm": "warn"}, "expect": "pass" }
```

```case
{ "name": "external-arming", "doc": "conventions/candidate/CHECK.md", "actor": "reviewer-q", "set": {"arm": "block"}, "expect": "pass" }
```
