---
corpus_test: reviewer-and-priority-dead-priority
convention: ../reviewer-and-priority
corpus: ../tree
---

# dead-priority (folder convention, exit 1)

The two-rule `reviewer-and-priority` convention loaded from a folder on disk.
The LIVE `scenarios/reviewer-close.md` rule fires where the manifest declares;
the `scenarios/lower-priority.md` rule is a real predicate that only refuses a
`priority: high` close — a condition no governed-tree doc carries — so it never
fires and is reported DEAD. The `@2` twin of the effect kernel's `dead_priority`
replay rule: a present-but-never-fired rule, caught over the corpus. Exit 1.

```rules
scenarios/reviewer-close.md
scenarios/lower-priority.md
```

```case
{ "name": "r3a-self-close", "doc": "tasks/r3a-impl-plan.md", "actor": "agent:alice", "set": {"owner": "agent:alice", "status": "closed"}, "expect": "scenarios/reviewer-close.md" }
```

```case
{ "name": "b3-reviewer-close", "doc": "tasks/b3-impl-plan.md", "actor": "agent:bob", "set": {"owner": "agent:alice", "status": "closed"}, "expect": "pass" }
```

```case
{ "name": "c-self-close", "doc": "tasks/c-impl-plan.md", "actor": "agent:erin", "set": {"owner": "agent:erin", "status": "closed"}, "expect": "scenarios/reviewer-close.md" }
```
