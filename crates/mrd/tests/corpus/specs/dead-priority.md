---
corpus_test: reviewer-and-priority-dead-priority
rule: ../rules/reviewer-and-priority.md
corpus: ../tree
---

# dead-priority (two-citation rule page, exit 1)

The two-citation `reviewer-and-priority` rule page, loaded by path.
The LIVE `reviewer-close` rule fires where the manifest declares;
the `lower-priority` citation is a real predicate that only refuses a
`priority: high` close — a condition no governed-tree doc carries — so it never
fires and is reported DEAD. The `@2` twin of the effect kernel's `dead_priority`
replay rule: a present-but-never-fired citation, caught over the corpus. Exit 1.

```rules
reviewer-close
lower-priority
```

```case
{ "name": "r3a-self-close", "doc": "tasks/r3a-impl-plan.md", "actor": "agent:alice", "set": {"owner": "agent:alice", "status": "closed"}, "expect": "reviewer-close" }
```

```case
{ "name": "b3-reviewer-close", "doc": "tasks/b3-impl-plan.md", "actor": "agent:bob", "set": {"owner": "agent:alice", "status": "closed"}, "expect": "pass" }
```

```case
{ "name": "c-self-close", "doc": "tasks/c-impl-plan.md", "actor": "agent:erin", "set": {"owner": "agent:erin", "status": "closed"}, "expect": "reviewer-close" }
```
