---
corpus_test: meta-convention-floor
convention: ../../conventions/meta-convention
corpus: ../tree
---

# meta-convention floor — fire-where-expected (arming preconditions)

An arming proposal (`arm: block`) must pin attested evidence (`armed_rev`), cite
its evidence join (`cites`), and be armed by a reviewer distinct from the
`author`. Each precondition has its own rule; a case violating exactly one fires
exactly that rule. The legal path meets all three. All three declared rules
fire, so none is dead: exit 0.

```rules
scenarios/attested-arming.md
scenarios/cited-arming.md
scenarios/external-arming.md
```

```case
{ "name": "unarmed-evidence", "doc": "conventions/candidate/CHECK.md", "actor": "reviewer-r", "set": {"arm": "block"}, "remove": ["armed_rev"], "expect": "scenarios/attested-arming.md" }
```

```case
{ "name": "uncited-arming", "doc": "conventions/candidate/CHECK.md", "actor": "reviewer-r", "set": {"arm": "block"}, "remove": ["cites"], "expect": "scenarios/cited-arming.md" }
```

```case
{ "name": "self-arming", "doc": "conventions/candidate/CHECK.md", "actor": "author-x", "set": {"arm": "block"}, "expect": "scenarios/external-arming.md" }
```

```case
{ "name": "attested-arming", "doc": "conventions/candidate/CHECK.md", "actor": "reviewer-r", "set": {"arm": "block"}, "expect": "pass" }
```
