---
corpus_test: decoy-close-floor
convention: ../../conventions/decoy-close
corpus: ../tree
---

# decoy-close floor — fire-where-expected

A closed-looking marker without the canonical `status: closed` fires
`decoy_close`; a real close (canonical `status`) passes. The one declared rule
fires: exit 0.

```rules
scenarios/real-close.md
```

```case
{ "name": "decoy-resolution", "doc": "tasks/plain-open.md", "actor": "worker-a", "set": {"resolution": "closed"}, "expect": "scenarios/real-close.md" }
```

```case
{ "name": "decoy-done", "doc": "tasks/plain-open.md", "actor": "worker-a", "set": {"done": "true"}, "expect": "scenarios/real-close.md" }
```

```case
{ "name": "real-close", "doc": "tasks/plain-open.md", "actor": "worker-a", "set": {"status": "closed"}, "expect": "pass" }
```
