---
corpus_test: reviewer-not-owner-dead-rule
rule: ../rules/reviewer-not-owner.md
corpus: ../governed
---

# dead-rule (fixture rule page, exit 1)

The `reviewer-not-owner` fixture page's one citation, `reviewer-close`, is DECLARED but
this corpus never fires it: every synthetic change closes as a distinct
reviewer, as an external (actor-less) edit, or over a doc outside the `tasks/**`
scope. Every case still matches its `expect` (all pass), so fire-where-expected
holds — yet the declared rule spent the whole corpus without firing, so the tier
reports it under "Dead rules (declared, never fired)" and exits 1. This is the
literal "corpus run over a one-rule law — a dead rule is reported."

```rules
reviewer-close
```

```case
{ "name": "r3a-reviewer-close", "doc": "tasks/plan-index.md", "actor": "agent:bob", "set": {"owner": "agent:alice", "status": "closed"}, "expect": "pass" }
```

```case
{ "name": "b3-reviewer-close", "doc": "tasks/plan-dialect.md", "actor": "agent:dave", "set": {"owner": "agent:carol", "status": "closed"}, "expect": "pass" }
```

```case
{ "name": "c-external-edit", "doc": "tasks/plan-walker.md", "set": {"owner": "agent:erin", "status": "closed"}, "expect": "pass" }
```

```case
{ "name": "decision-out-of-scope", "doc": "decisions/001-crate-cut.md", "actor": "agent:zt", "set": {"owner": "agent:zt", "status": "closed"}, "expect": "pass" }
```
