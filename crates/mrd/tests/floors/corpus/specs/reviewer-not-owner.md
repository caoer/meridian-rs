---
corpus_test: reviewer-not-owner-floor
convention: ../../conventions/reviewer-not-owner
corpus: ../tree
---

# reviewer-not-owner floor — fire-where-expected

Owner-self-close fires (`reviewer_owner`); a distinct reviewer's close, and an
external actor-less close, pass. The one declared rule fires, so it is not dead:
this run exits 0.

```rules
scenarios/reviewer-close.md
```

```case
{ "name": "owner-self-close", "doc": "tasks/owned-open.md", "actor": "worker-a", "set": {"status": "closed"}, "expect": "scenarios/reviewer-close.md" }
```

```case
{ "name": "reviewer-close", "doc": "tasks/owned-open.md", "actor": "reviewer-b", "set": {"status": "closed"}, "expect": "pass" }
```

```case
{ "name": "external-close", "doc": "tasks/owned-open.md", "set": {"status": "closed"}, "expect": "pass" }
```
