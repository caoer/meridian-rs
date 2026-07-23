---
corpus_test: reviewer-not-owner-surprise-rule
convention: seed
corpus: ../tree
---

# surprise-rule (undeclared rule fires, exit 1)

The manifest declares NO rules (the `rules` block is empty), yet a synthetic
change fires `scenarios/reviewer-close.md`. Each case still matches its `expect`,
but the convention fired a rule the manifest never declared — an incomplete
expected-fire manifest. The tier reports it under "Surprise rules (fired, never
declared)" and exits 1, so an under-declared manifest cannot pass silently.

```rules
```

```case
{ "name": "r3a-self-close", "doc": "tasks/r3a-impl-plan.md", "actor": "agent:alice", "set": {"owner": "agent:alice", "status": "closed"}, "expect": "scenarios/reviewer-close.md" }
```

```case
{ "name": "b3-reviewer-close", "doc": "tasks/b3-impl-plan.md", "actor": "agent:bob", "set": {"owner": "agent:alice", "status": "closed"}, "expect": "pass" }
```
