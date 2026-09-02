---
corpus_test: malformed-case-json
rule: ../rules/reviewer-not-owner.md
corpus: ../governed
---

# malformed (exit 2)

A `case` block whose JSON will not parse. A malformed spec is a tool failure
(exit 2) — the tier cannot render a trustworthy verdict, mirroring the tier-1
scenario runner's malformed-scenario exit.

```case
{ "doc": "tasks/plan-index.md", "actor": "agent:alice", "expect": }
```
