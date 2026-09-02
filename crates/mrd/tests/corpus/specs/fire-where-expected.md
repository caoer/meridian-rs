---
corpus_test: reviewer-not-owner-fire-where-expected
rule: ../rules/reviewer-not-owner.md
corpus: ../governed
---

# fire-where-expected (fixture rule page, exit 0)

Synthetic close changes over the governed fixture tree (`../governed/tasks/*` —
task cards shaped like a session board). Each firing case assigns an `owner` and closes the
task AS that owner (reviewer == owner → the rule refuses); each passing case
closes as a distinct reviewer, an external (actor-less) edit, or a doc outside
the `tasks/**` scope. The `reviewer-close` citation fires exactly where the
manifest declares, so it is not dead: this run exits 0.

```rules
reviewer-close
```

```case
{ "name": "r3a-self-close", "doc": "tasks/plan-index.md", "actor": "agent:alice", "set": {"owner": "agent:alice", "status": "closed"}, "expect": "reviewer-close" }
```

```case
{ "name": "r3a-reviewer-close", "doc": "tasks/plan-index.md", "actor": "agent:bob", "set": {"owner": "agent:alice", "status": "closed"}, "expect": "pass" }
```

```case
{ "name": "b3-self-close", "doc": "tasks/plan-dialect.md", "actor": "agent:carol", "set": {"owner": "agent:carol", "status": "closed"}, "expect": "reviewer-close" }
```

```case
{ "name": "b3-reviewer-close", "doc": "tasks/plan-dialect.md", "actor": "agent:dave", "set": {"owner": "agent:carol", "status": "closed"}, "expect": "pass" }
```

```case
{ "name": "c-self-close", "doc": "tasks/plan-walker.md", "actor": "agent:erin", "set": {"owner": "agent:erin", "status": "closed"}, "expect": "reviewer-close" }
```

```case
{ "name": "gatecheck-external-edit", "doc": "tasks/gate-check.md", "set": {"owner": "agent:frank", "status": "closed"}, "expect": "pass" }
```

```case
{ "name": "decision-out-of-scope", "doc": "decisions/001-crate-cut.md", "actor": "agent:zt", "set": {"owner": "agent:zt", "status": "closed"}, "expect": "pass" }
```
