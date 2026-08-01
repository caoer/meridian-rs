---
corpus_test: decoy-close-floor
rule: ../../rules/decoy-close.md
corpus: ../tree
---

# decoy-close floor — fire-where-expected

A closed-looking marker without the canonical `status: closed` fires
`decoy_close`; a real close (canonical `status`) passes. The one declared rule
fires: exit 0.

```rules
real-close
```

```case
{ "name": "decoy-resolution", "doc": "tasks/plain-open.md", "actor": "worker-a", "set": {"resolution": "closed"}, "expect": "real-close" }
```

```case
{ "name": "decoy-done", "doc": "tasks/plain-open.md", "actor": "worker-a", "set": {"done": "true"}, "expect": "real-close" }
```

```case
{ "name": "real-close", "doc": "tasks/plain-open.md", "actor": "worker-a", "set": {"status": "closed"}, "expect": "pass" }
```
