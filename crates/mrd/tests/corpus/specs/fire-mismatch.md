---
corpus_test: reviewer-not-owner-fire-mismatch
convention: seed
corpus: ../tree
---

# fire-mismatch (load-bearing negative, exit 1)

The load-bearing negative that proves fire-where-expected is ENFORCED, not
vacuous. The first case closes a task as its own owner (which the seed refuses)
but declares `expect: pass` — a WRONG expectation. The tier observes the fire,
sees it disagree with the manifest, reports the mismatch, and exits 1. The
second case fires as declared, so `reviewer-close.md` is not dead: the only
finding is the mismatch.

```rules
scenarios/reviewer-close.md
```

```case
{ "name": "wrong-expect-pass", "doc": "tasks/r3a-impl-plan.md", "actor": "agent:alice", "set": {"owner": "agent:alice", "status": "closed"}, "expect": "pass" }
```

```case
{ "name": "correct-self-close", "doc": "tasks/b3-impl-plan.md", "actor": "agent:carol", "set": {"owner": "agent:carol", "status": "closed"}, "expect": "scenarios/reviewer-close.md" }
```
