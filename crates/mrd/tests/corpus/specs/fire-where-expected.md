---
corpus_test: reviewer-not-owner-fire-where-expected
convention: seed
corpus: ../tree
---

# fire-where-expected (seed convention, exit 0)

Synthetic close changes over the 18-02 governed tree (`../tree/tasks/*` — real
18-02 session task pages). Each firing case assigns an `owner` and closes the
task AS that owner (reviewer == owner → the seed refuses); each passing case
closes as a distinct reviewer, an external (actor-less) edit, or a doc outside
the `tasks/**` scope. The `reviewer-close.md` rule fires exactly where the
manifest declares, so it is not dead: this run exits 0.

```rules
scenarios/reviewer-close.md
```

```case
{ "name": "r3a-self-close", "doc": "tasks/r3a-impl-plan.md", "actor": "agent:alice", "set": {"owner": "agent:alice", "status": "closed"}, "expect": "scenarios/reviewer-close.md" }
```

```case
{ "name": "r3a-reviewer-close", "doc": "tasks/r3a-impl-plan.md", "actor": "agent:bob", "set": {"owner": "agent:alice", "status": "closed"}, "expect": "pass" }
```

```case
{ "name": "b3-self-close", "doc": "tasks/b3-impl-plan.md", "actor": "agent:carol", "set": {"owner": "agent:carol", "status": "closed"}, "expect": "scenarios/reviewer-close.md" }
```

```case
{ "name": "b3-reviewer-close", "doc": "tasks/b3-impl-plan.md", "actor": "agent:dave", "set": {"owner": "agent:carol", "status": "closed"}, "expect": "pass" }
```

```case
{ "name": "c-self-close", "doc": "tasks/c-impl-plan.md", "actor": "agent:erin", "set": {"owner": "agent:erin", "status": "closed"}, "expect": "scenarios/reviewer-close.md" }
```

```case
{ "name": "gatecheck-external-edit", "doc": "tasks/b3-gatecheck.md", "set": {"owner": "agent:frank", "status": "closed"}, "expect": "pass" }
```

```case
{ "name": "decision-out-of-scope", "doc": "decisions/001-package-cut.md", "actor": "agent:zt", "set": {"owner": "agent:zt", "status": "closed"}, "expect": "pass" }
```
